//! MCP knowledge.* 工具派发（agent-autonomy-loop W3 / Task 4.2）。
//!
//! Reply Agent 在 `decision_phase == "tool_calling"` 中间轮通过
//! [`crate::agent::types::ToolCallRequest`] 声明想要调用的工具，本模块的
//! [`dispatch_tool_call`] 负责按 R4.4 / R4.5 / R4.6 的契约去 `KnowledgeRuntime`
//! 上检索并返回 JSON 结果（或错误结构）。
//!
//! 三个工具的契约（与 requirements.md R4.4–R4.6 / design.md §4 严格对齐）：
//!
//! - `knowledge.list_catalog`：输入 `{ kind?: documents|items|chunks, limit?: 1..=200 }`，
//!   输出 `{ items: [...], truncated, kind }`，**不返回正文**；同 run 内同 `kind`
//!   调用次数上限 2，超出 → `{"error":"tool_call_repeated"}`。
//! - `knowledge.search`：输入 `{ query: 1..=200 chars, top_k?: 1..=32 }`，
//!   输出 `{ hits, query, hit_count }`；非 verified chunk 的 `snippet` SHALL 被
//!   空字符串占位且 `redacted=true`。
//! - `knowledge.open_slice`：输入 `{ chunk_ids: [...] }`，K 由
//!   `knowledge_open_slice_max_k` 控制；任一未知 chunk_id 全部 fail（
//!   不返回部分结果）；非 verified body 替换为 `<redacted_unverified_chunk>`，
//!   但 `integrity_status` 字段保留原值。
//!
//! 公共行为：
//!
//! - 每次 dispatch 在实际查询前先 [`RunBudget::record_tool_call`]，超额返回
//!   `{"error":"budget_exceeded"}`（R4.3 / R4.8）；
//! - 单次 dispatch 5s timeout（`tokio::time::timeout`），超时返回
//!   `{"error":"tool_timeout"}`（R4.8）；
//! - 失败结果用 `serde_json::Value` 表达，由 [`reply_with_tools_loop`]
//!   决定是否计入"失败连击"（连续 ≥3 次失败强制结束循环）。

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mongodb::bson::Document;
use serde::Deserialize;
use serde_json::{json, Value};

use super::budget::{BudgetError, RunBudget};
use super::runtime::UserRuntimeParameters;
use super::types::{KnowledgeRuntime, ToolCallRequest};
use crate::models::{OperationKnowledgeChunk, OperationKnowledgeItem};

/// 单次 dispatch 的硬超时（R4.8）。
pub(crate) const TOOL_DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

/// 同一 `kind` 在单 run 内最多调用 list_catalog 的次数（R4.4）。
pub(crate) const LIST_CATALOG_PER_KIND_LIMIT: i32 = 2;

/// list_catalog 默认 / 上限 limit（R4.4：1..=200，default 50）。
const LIST_CATALOG_DEFAULT_LIMIT: i32 = 50;
const LIST_CATALOG_MAX_LIMIT: i32 = 200;

/// search 输入 query 的字符长度上限（R4.5：1..=200）。
const SEARCH_QUERY_MAX_CHARS: usize = 200;

/// search snippet 字符上限（R4.5：≤ 200 chars）。
const SEARCH_SNIPPET_MAX_CHARS: usize = 200;

/// `knowledge.search` 输出的 `hits[i].snippet` 在 chunk integrity_status !=
/// verified 时被空字符串占位（R4.5 redacted 语义）。
const REDACTED_UNVERIFIED_BODY: &str = "<redacted_unverified_chunk>";

// ── Tool 名 ────────────────────────────────────────────────────────────

pub(crate) const TOOL_LIST_CATALOG: &str = "knowledge.list_catalog";
pub(crate) const TOOL_SEARCH: &str = "knowledge.search";
pub(crate) const TOOL_OPEN_SLICE: &str = "knowledge.open_slice";

/// 用于 R4.1 toolCalls schema 校验：合法 tool 名白名单。
pub(crate) const ALLOWED_TOOL_NAMES: &[&str] =
    &[TOOL_LIST_CATALOG, TOOL_SEARCH, TOOL_OPEN_SLICE];

/// 单 run 内的 tool dispatch 状态（在多轮 [`reply_with_tools_loop`] 之间共享）。
///
/// 由调用方在工具循环开始前 `Default::default()` 创建，每轮通过 mutable
/// borrow 传入 [`dispatch_tool_call`]；记录"同 kind list_catalog 调用次数"
/// 等需要跨轮判定的状态。
#[derive(Debug, Default)]
pub(crate) struct ToolDispatchState {
    /// 每个 `kind`（documents / items / chunks）已成功执行过的 list_catalog
    /// 次数。`tool_call_repeated` 判定基于此（R4.4）。
    pub list_catalog_calls_per_kind: HashMap<String, i32>,
}

impl ToolDispatchState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

// ── 入参 schema（serde 反序列化容器）────────────────────────────────────

/// `knowledge.list_catalog` 入参（R4.4）。
#[derive(Debug, Default, Deserialize)]
struct ListCatalogArgs {
    /// `documents` / `items` / `chunks`；缺省视为 `chunks`。
    #[serde(default)]
    kind: Option<String>,
    /// 1..=200，缺省 50；非法（≤0 或非整数）回退到 50。
    #[serde(default)]
    limit: Option<i32>,
}

/// `knowledge.search` 入参（R4.5）。
#[derive(Debug, Default, Deserialize)]
struct SearchArgs {
    #[serde(default)]
    query: Option<String>,
    /// 1..=32，缺省由 `runtime.knowledge_search_top_k` 注入。
    #[serde(default)]
    top_k: Option<i32>,
}

/// `knowledge.open_slice` 入参（R4.6）。
#[derive(Debug, Default, Deserialize)]
struct OpenSliceArgs {
    #[serde(default)]
    chunk_ids: Option<Vec<String>>,
}

// ── 主入口 ──────────────────────────────────────────────────────────────

/// 派发 Reply Agent 的一次工具调用。
///
/// 行为概览：
///
/// 1. 校验 `call.tool` 在白名单内；非法 → `{"error":"unknown_tool","detail":...}`。
/// 2. 通过 [`RunBudget::record_tool_call`] 占用 1 次 tool call 槽位 + 估算的
///    token 数；超额 → `{"error":"budget_exceeded","detail":...}`。
/// 3. 用 [`tokio::time::timeout`] 包裹具体 tool 实现，5s 内完成 → 返回
///    JSON；超时 → `{"error":"tool_timeout"}`。
/// 4. 具体 tool 实现走纯函数（基于内存中的 [`KnowledgeRuntime`]，无 IO）；
///    返回的 `Value` 由调用方注入下一轮 prompt 的 `[system tool result]` 段。
///
/// **注意**：本函数总是返回 `Value`，**不返回 Result**——错误也是合法的工具
/// 结果，以便 Reply Agent 在下一轮自我修正（R4.8 失败降级）。
pub(crate) async fn dispatch_tool_call(
    call: &ToolCallRequest,
    runtime: &UserRuntimeParameters,
    knowledge: &KnowledgeRuntime,
    budget: &Arc<RunBudget>,
    state: &mut ToolDispatchState,
) -> Value {
    let tool = call.tool.trim();
    if !ALLOWED_TOOL_NAMES.iter().any(|allowed| *allowed == tool) {
        return tool_error("unknown_tool", &format!("tool name '{tool}' not allowed"));
    }

    // R4.3：先占预算。tokens_consumed 这里统一用 0，由具体 tool 在成功路径
    // 上再追加（避免失败也扣 token）。
    if let Err(err) = budget.record_tool_call(0) {
        return budget_error_value(&err);
    }

    // R4.8：5s 单次 timeout。
    let fut = async move {
        match tool {
            TOOL_LIST_CATALOG => exec_list_catalog(&call.arguments, knowledge, state),
            TOOL_SEARCH => exec_search(&call.arguments, knowledge, runtime),
            TOOL_OPEN_SLICE => exec_open_slice(&call.arguments, knowledge, runtime),
            _ => unreachable!("tool whitelist enforced above"),
        }
    };
    match tokio::time::timeout(TOOL_DISPATCH_TIMEOUT, fut).await {
        Ok(value) => value,
        Err(_) => tool_error(
            "tool_timeout",
            &format!("tool '{tool}' exceeded 5s timeout"),
        ),
    }
}

// ── 错误辅助函数 ───────────────────────────────────────────────────────

fn tool_error(code: &str, detail: &str) -> Value {
    json!({ "error": code, "detail": detail })
}

fn budget_error_value(err: &BudgetError) -> Value {
    let detail = err.to_string();
    json!({ "error": "budget_exceeded", "detail": detail })
}

// ── knowledge.list_catalog ─────────────────────────────────────────────

fn exec_list_catalog(
    arguments: &Document,
    knowledge: &KnowledgeRuntime,
    state: &mut ToolDispatchState,
) -> Value {
    let args: ListCatalogArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };

    let kind = args.kind.as_deref().unwrap_or("chunks").trim().to_string();
    if !matches!(kind.as_str(), "documents" | "items" | "chunks") {
        return tool_error(
            "invalid_input",
            &format!("kind '{kind}' must be one of documents|items|chunks"),
        );
    }

    // R4.4：单 run 内同 kind 调用次数 ≤ 2。
    let already = state
        .list_catalog_calls_per_kind
        .get(&kind)
        .copied()
        .unwrap_or(0);
    if already >= LIST_CATALOG_PER_KIND_LIMIT {
        return tool_error(
            "tool_call_repeated",
            &format!(
                "kind '{kind}' already called {already} times (limit {LIST_CATALOG_PER_KIND_LIMIT})"
            ),
        );
    }

    let limit = args
        .limit
        .filter(|v| *v > 0)
        .unwrap_or(LIST_CATALOG_DEFAULT_LIMIT)
        .min(LIST_CATALOG_MAX_LIMIT)
        .max(1) as usize;

    let (items_json, total) = match kind.as_str() {
        "documents" => {
            let total = knowledge.documents.len();
            let items = knowledge
                .documents
                .iter()
                .take(limit)
                .map(|doc| {
                    json!({
                        "id": doc.id.map(|id| id.to_hex()).unwrap_or_default(),
                        "title": doc.title.clone(),
                        "category": doc.source_type.clone(),
                        "integrity_status": Value::Null,
                        "updated_at": doc.updated_at.timestamp_millis(),
                    })
                })
                .collect::<Vec<_>>();
            (items, total)
        }
        "items" => {
            let total = knowledge.items.len();
            let items = knowledge
                .items
                .iter()
                .take(limit)
                .map(|item| {
                    json!({
                        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
                        "title": item.title.clone(),
                        "category": item.knowledge_type.clone().unwrap_or_else(|| item.category.clone()),
                        "integrity_status": Value::Null,
                        "updated_at": item.updated_at.timestamp_millis(),
                    })
                })
                .collect::<Vec<_>>();
            (items, total)
        }
        _ => {
            // chunks
            let total = knowledge.chunks.len();
            let items = knowledge
                .chunks
                .iter()
                .take(limit)
                .map(|chunk| {
                    json!({
                        "id": chunk.id.map(|id| id.to_hex()).unwrap_or_default(),
                        "title": chunk.title.clone(),
                        "category": chunk.knowledge_type.clone().unwrap_or_default(),
                        "integrity_status": chunk.integrity_status.clone().unwrap_or_default(),
                        "updated_at": chunk.updated_at.timestamp_millis(),
                    })
                })
                .collect::<Vec<_>>();
            (items, total)
        }
    };

    let truncated = total > items_json.len();

    // 仅在返回成功时累加调用次数（错误路径不算"消耗"调用配额）。
    *state
        .list_catalog_calls_per_kind
        .entry(kind.clone())
        .or_insert(0) += 1;

    json!({
        "items": items_json,
        "truncated": truncated,
        "kind": kind,
        "total": total,
    })
}

// ── knowledge.search ───────────────────────────────────────────────────

fn exec_search(
    arguments: &Document,
    knowledge: &KnowledgeRuntime,
    runtime: &UserRuntimeParameters,
) -> Value {
    let args: SearchArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };

    let query_raw = args.query.unwrap_or_default();
    let query = query_raw.trim();
    if query.is_empty() {
        return tool_error("invalid_query", "query is empty");
    }
    if query.chars().count() > SEARCH_QUERY_MAX_CHARS {
        return tool_error(
            "invalid_query",
            &format!("query exceeds {SEARCH_QUERY_MAX_CHARS} chars"),
        );
    }

    let top_k = args
        .top_k
        .filter(|v| *v > 0)
        .unwrap_or(runtime.knowledge_search_top_k)
        .min(32)
        .max(1) as usize;

    // 简单评分：title / summary / routing_card 包含 query 的字数命中数；
    // verified chunks 优先排前。本期不做向量召回，留作 W6 之后的工作。
    let mut scored: Vec<(f64, &OperationKnowledgeChunk)> = knowledge
        .chunks
        .iter()
        .filter_map(|chunk| {
            let score = score_chunk_for_query(chunk, query);
            if score > 0.0 {
                Some((score, chunk))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let hits = scored
        .into_iter()
        .take(top_k)
        .map(|(score, chunk)| build_search_hit(chunk, score))
        .collect::<Vec<_>>();
    let hit_count = hits.len();

    json!({
        "hits": hits,
        "query": query.to_string(),
        "hit_count": hit_count,
    })
}

fn build_search_hit(chunk: &OperationKnowledgeChunk, score: f64) -> Value {
    let chunk_id = chunk.id.map(|id| id.to_hex()).unwrap_or_default();
    let integrity = chunk.integrity_status.clone().unwrap_or_default();
    let is_verified = integrity == "verified";
    let snippet_source = chunk
        .summary
        .clone()
        .or_else(|| chunk.routing_card.clone())
        .or_else(|| chunk.body.clone())
        .unwrap_or_default();
    let snippet = if is_verified {
        truncate_chars(&snippet_source, SEARCH_SNIPPET_MAX_CHARS)
    } else {
        String::new()
    };
    json!({
        "chunk_id": chunk_id,
        "score": score,
        "snippet": snippet,
        "integrity_status": integrity,
        "redacted": !is_verified,
        "title": chunk.title.clone(),
    })
}

fn score_chunk_for_query(chunk: &OperationKnowledgeChunk, query: &str) -> f64 {
    let q = query.to_lowercase();
    let mut score = 0.0;
    let title = chunk.title.to_lowercase();
    if title.contains(&q) {
        score += 3.0;
    }
    if let Some(summary) = chunk.summary.as_ref() {
        if summary.to_lowercase().contains(&q) {
            score += 2.0;
        }
    }
    if let Some(routing) = chunk.routing_card.as_ref() {
        if routing.to_lowercase().contains(&q) {
            score += 1.5;
        }
    }
    if let Some(body) = chunk.body.as_ref() {
        if body.to_lowercase().contains(&q) {
            score += 1.0;
        }
    }
    if score > 0.0
        && chunk
            .integrity_status
            .as_deref()
            .map(|s| s == "verified")
            .unwrap_or(false)
    {
        score += 0.5;
    }
    score
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

// ── knowledge.open_slice ───────────────────────────────────────────────

fn exec_open_slice(
    arguments: &Document,
    knowledge: &KnowledgeRuntime,
    runtime: &UserRuntimeParameters,
) -> Value {
    let args: OpenSliceArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };

    let chunk_ids = args.chunk_ids.unwrap_or_default();
    if chunk_ids.is_empty() {
        return tool_error("invalid_input", "chunk_ids is empty");
    }

    let cap = runtime.knowledge_open_slice_max_k.max(1) as usize;
    if chunk_ids.len() > cap {
        return tool_error(
            "over_limit",
            &format!("chunk_ids length {} exceeds K={cap}", chunk_ids.len()),
        );
    }

    // R4.6：未知 chunk_id 全部 fail（即使其它命中）。
    let mut missing = Vec::new();
    let mut found: Vec<&OperationKnowledgeChunk> = Vec::with_capacity(chunk_ids.len());
    for id in &chunk_ids {
        match knowledge
            .chunks
            .iter()
            .find(|c| c.id.map(|oid| oid.to_hex()).as_deref() == Some(id.as_str()))
        {
            Some(chunk) => found.push(chunk),
            None => missing.push(id.clone()),
        }
    }
    if !missing.is_empty() {
        return json!({
            "error": "unknown_chunk_id",
            "missing": missing,
        });
    }

    let slices = found
        .into_iter()
        .map(|chunk| {
            let integrity = chunk.integrity_status.clone().unwrap_or_default();
            let is_verified = integrity == "verified";
            let body = if is_verified {
                chunk.body.clone().unwrap_or_default()
            } else {
                REDACTED_UNVERIFIED_BODY.to_string()
            };
            json!({
                "chunk_id": chunk.id.map(|id| id.to_hex()).unwrap_or_default(),
                "body": body,
                "integrity_status": integrity,
                "source": chunk.source_quote.clone().unwrap_or_default(),
                "updated_at": chunk.updated_at.timestamp_millis(),
                "title": chunk.title.clone(),
                "redacted": !is_verified,
            })
        })
        .collect::<Vec<_>>();

    json!({ "slices": slices })
}

// ── 内部辅助 ────────────────────────────────────────────────────────────

fn parse_arguments<T: for<'de> Deserialize<'de> + Default>(arguments: &Document) -> Result<T, String> {
    if arguments.is_empty() {
        return Ok(T::default());
    }
    let value = mongodb::bson::Bson::Document(arguments.clone()).into_relaxed_extjson();
    serde_json::from_value::<T>(value).map_err(|e| format!("invalid arguments: {e}"))
}

#[allow(dead_code)]
fn item_summary(item: &OperationKnowledgeItem) -> Option<String> {
    item.summary.clone()
}

#[cfg(test)]
mod tests {
    //! agent-autonomy-loop W3 / Task 4.2 unit tests for `knowledge.*` 三工具派发。
    //!
    //! 覆盖契约（与 requirements.md R4.4 / R4.5 / R4.6 / R4.8 / R4.3 对齐）：
    //!
    //! - `knowledge.list_catalog`：同 `kind` 第三次调用返回
    //!   `tool_call_repeated`；非法 `kind` 返回 `invalid_input`；`limit` 截断生效。
    //! - `knowledge.search`：空 query → `invalid_query`；非 verified chunk 的
    //!   `snippet` 必为空字符串且 `redacted=true`；verified chunk 命中时返回正文截断。
    //! - `knowledge.open_slice`：未知 chunk_id 一票否决（即使其它命中）；非
    //!   verified chunk 的 `body` 替换为 `<redacted_unverified_chunk>` 但
    //!   `integrity_status` 保留原值。
    //! - `dispatch_tool_call`：未知工具名 → `unknown_tool`；预算耗尽 → `budget_exceeded`。
    use super::*;
    use crate::agent::budget::RunBudget;
    use crate::agent::runtime::UserRuntimeParameters;
    use crate::agent::types::{KnowledgeRuntime, ToolCallRequest};
    use crate::models::{
        OperationKnowledgeChunk, OperationKnowledgeDocument, OperationKnowledgeItem,
    };
    use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt, Document};

    fn build_chunk(
        title: &str,
        integrity: Option<&str>,
        body: Option<&str>,
        summary: Option<&str>,
    ) -> OperationKnowledgeChunk {
        OperationKnowledgeChunk {
            id: Some(ObjectId::new()),
            workspace_id: "default".into(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: "user_operations".into(),
            knowledge_type: Some("product".into()),
            business_context: None,
            title: title.into(),
            summary: summary.map(ToString::to_string),
            body: body.map(ToString::to_string),
            routing_card: None,
            applicable_scenes: vec![],
            not_applicable_scenes: vec![],
            safe_claims: vec![],
            forbidden_claims: vec![],
            evidence_items: vec![],
            source_quote: Some("doc#1".into()),
            source_anchors: vec![],
            integrity_status: integrity.map(ToString::to_string),
            confidence_score: None,
            distortion_risks: vec![],
            unsupported_claims: vec![],
            verified_claims: vec![],
            status: "active".into(),
            priority: 0,
            created_at: BsonDt::now(),
            updated_at: BsonDt::now(),
        }
    }

    fn build_doc(title: &str) -> OperationKnowledgeDocument {
        OperationKnowledgeDocument {
            id: Some(ObjectId::new()),
            workspace_id: "default".into(),
            account_id: None,
            domain: "user_operations".into(),
            source_type: "manual".into(),
            source_name: None,
            title: title.into(),
            summary: None,
            catalog_summary: None,
            routing_map: vec![],
            risk_notes: vec![],
            raw_content: None,
            content_hash: None,
            line_index: vec![],
            section_index: vec![],
            status: "active".into(),
            version: 1,
            created_at: BsonDt::now(),
            updated_at: BsonDt::now(),
        }
    }

    fn build_item(title: &str) -> OperationKnowledgeItem {
        OperationKnowledgeItem {
            id: Some(ObjectId::new()),
            workspace_id: "default".into(),
            account_id: None,
            domain: "user_operations".into(),
            category: "product".into(),
            business_type: "general".into(),
            knowledge_type: Some("product".into()),
            business_context: None,
            title: title.into(),
            summary: Some("s".into()),
            body: None,
            routing_card: None,
            applicable_scenes: vec![],
            not_applicable_scenes: vec![],
            suitable_for: vec![],
            not_suitable_for: vec![],
            customer_stages: vec![],
            operation_states: vec![],
            intent_levels: vec![],
            safe_claims: vec![],
            forbidden_claims: vec![],
            common_questions: vec![],
            common_objections: vec![],
            evidence_items: vec![],
            source_type: "manual".into(),
            source_name: None,
            status: "active".into(),
            priority: 0,
            version: 1,
            created_at: BsonDt::now(),
            updated_at: BsonDt::now(),
        }
    }

    fn make_runtime() -> UserRuntimeParameters {
        UserRuntimeParameters::default()
    }

    fn make_budget(token_budget: i64, tool_calls: i32) -> Arc<RunBudget> {
        Arc::new(RunBudget::new("test-run", token_budget, 32, tool_calls))
    }

    fn call_with_args(tool: &str, arguments: Document) -> ToolCallRequest {
        ToolCallRequest {
            tool: tool.into(),
            arguments,
        }
    }

    // ── list_catalog ──────────────────────────────────────────────

    #[test]
    fn list_catalog_third_call_same_kind_returns_repeated() {
        // R4.4：同 run 内同 kind 调用次数上限 2，第三次必须返回 tool_call_repeated。
        let knowledge = KnowledgeRuntime {
            documents: vec![],
            items: vec![],
            chunks: vec![build_chunk("c1", Some("verified"), None, None)],
        };
        let mut state = ToolDispatchState::new();
        let args = Document::new();
        let v1 = exec_list_catalog(&args, &knowledge, &mut state);
        assert!(v1.get("error").is_none(), "first call must succeed: {v1}");
        let v2 = exec_list_catalog(&args, &knowledge, &mut state);
        assert!(v2.get("error").is_none(), "second call must succeed: {v2}");
        let v3 = exec_list_catalog(&args, &knowledge, &mut state);
        assert_eq!(
            v3.get("error").and_then(|x| x.as_str()),
            Some("tool_call_repeated"),
            "third call should be rejected: {v3}"
        );
    }

    #[test]
    fn list_catalog_invalid_kind_rejected_without_consuming_quota() {
        // R4.4：非法 kind 返回 invalid_input；不应消耗调用配额（错误路径不计入）。
        let knowledge = KnowledgeRuntime::default();
        let mut state = ToolDispatchState::new();
        let bad = exec_list_catalog(&doc! {"kind": "messages"}, &knowledge, &mut state);
        assert_eq!(bad.get("error").and_then(|x| x.as_str()), Some("invalid_input"));
        assert!(state.list_catalog_calls_per_kind.is_empty());
    }

    #[test]
    fn list_catalog_limit_truncates_and_reports_total() {
        // R4.4：limit 截断后 truncated=true，total 反映原始集合大小。
        let chunks: Vec<_> = (0..5)
            .map(|i| build_chunk(&format!("c{i}"), Some("verified"), None, None))
            .collect();
        let knowledge = KnowledgeRuntime {
            documents: vec![build_doc("d")],
            items: vec![build_item("i")],
            chunks,
        };
        let mut state = ToolDispatchState::new();
        let v = exec_list_catalog(&doc! {"kind": "chunks", "limit": 2}, &knowledge, &mut state);
        assert_eq!(v["truncated"].as_bool(), Some(true));
        assert_eq!(v["total"].as_i64(), Some(5));
        assert_eq!(v["items"].as_array().map(|a| a.len()), Some(2));
        assert_eq!(v["kind"].as_str(), Some("chunks"));
    }

    // ── search ────────────────────────────────────────────────────

    #[test]
    fn search_empty_query_rejected() {
        // R4.5：query 必填且 1..=200 chars；空白裁剪后为空 → invalid_query。
        let knowledge = KnowledgeRuntime::default();
        let runtime = make_runtime();
        let v = exec_search(&doc! {"query": "   "}, &knowledge, &runtime);
        assert_eq!(v.get("error").and_then(|x| x.as_str()), Some("invalid_query"));
    }

    #[test]
    fn search_non_verified_snippet_is_redacted_and_flagged() {
        // R4.5：非 verified chunk 命中时 snippet 必须为空串且 redacted=true。
        let knowledge = KnowledgeRuntime {
            documents: vec![],
            items: vec![],
            chunks: vec![build_chunk(
                "alpha-product",
                Some("draft"),
                Some("alpha-product full body content"),
                Some("alpha-product summary"),
            )],
        };
        let runtime = make_runtime();
        let v = exec_search(&doc! {"query": "alpha"}, &knowledge, &runtime);
        let hits = v["hits"].as_array().expect("hits must be array");
        assert_eq!(hits.len(), 1, "should hit the single chunk: {v}");
        let hit = &hits[0];
        assert_eq!(hit["snippet"].as_str(), Some(""));
        assert_eq!(hit["redacted"].as_bool(), Some(true));
        assert_eq!(hit["integrity_status"].as_str(), Some("draft"));
    }

    #[test]
    fn search_verified_returns_snippet_with_redacted_false() {
        // R4.5 反向：verified chunk 必须返回 snippet 且 redacted=false。
        let knowledge = KnowledgeRuntime {
            documents: vec![],
            items: vec![],
            chunks: vec![build_chunk(
                "beta-feature",
                Some("verified"),
                None,
                Some("beta-feature highlights"),
            )],
        };
        let runtime = make_runtime();
        let v = exec_search(&doc! {"query": "beta"}, &knowledge, &runtime);
        let hit = &v["hits"].as_array().unwrap()[0];
        assert_eq!(hit["redacted"].as_bool(), Some(false));
        assert!(hit["snippet"].as_str().unwrap().contains("beta-feature"));
    }

    // ── open_slice ────────────────────────────────────────────────

    #[test]
    fn open_slice_unknown_id_fails_all_no_partial() {
        // R4.6：任一未知 chunk_id 即整体 fail，不返回部分结果。
        let chunk = build_chunk("c1", Some("verified"), Some("body-1"), None);
        let known_id = chunk.id.unwrap().to_hex();
        let knowledge = KnowledgeRuntime {
            documents: vec![],
            items: vec![],
            chunks: vec![chunk],
        };
        let runtime = make_runtime();
        let v = exec_open_slice(
            &doc! {"chunk_ids": [known_id, "ffffffffffffffffffffffff"]},
            &knowledge,
            &runtime,
        );
        assert_eq!(
            v.get("error").and_then(|x| x.as_str()),
            Some("unknown_chunk_id"),
            "must reject all when any id missing: {v}"
        );
        assert!(v.get("slices").is_none(), "no partial slices on failure");
    }

    #[test]
    fn open_slice_redacts_non_verified_body_but_keeps_integrity() {
        // R4.6：非 verified chunk body 占位为 <redacted_unverified_chunk>，
        // 但 integrity_status 字段保留原值，redacted=true。
        let chunk = build_chunk("c1", Some("draft"), Some("secret body"), None);
        let id = chunk.id.unwrap().to_hex();
        let knowledge = KnowledgeRuntime {
            documents: vec![],
            items: vec![],
            chunks: vec![chunk],
        };
        let runtime = make_runtime();
        let v = exec_open_slice(&doc! {"chunk_ids": [id]}, &knowledge, &runtime);
        let slice = &v["slices"].as_array().unwrap()[0];
        assert_eq!(slice["body"].as_str(), Some(REDACTED_UNVERIFIED_BODY));
        assert_eq!(slice["integrity_status"].as_str(), Some("draft"));
        assert_eq!(slice["redacted"].as_bool(), Some(true));
    }

    #[test]
    fn open_slice_over_k_cap_rejected() {
        // R4.6：chunk_ids 长度超过 K（runtime.knowledge_open_slice_max_k）→ over_limit。
        let knowledge = KnowledgeRuntime::default();
        let mut runtime = make_runtime();
        runtime.knowledge_open_slice_max_k = 2;
        let v = exec_open_slice(
            &doc! {"chunk_ids": ["a", "b", "c"]},
            &knowledge,
            &runtime,
        );
        assert_eq!(v.get("error").and_then(|x| x.as_str()), Some("over_limit"));
    }

    // ── dispatch_tool_call (async) ────────────────────────────────

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error_value() {
        // R4.1：未知工具名直接返回 unknown_tool 错误结构（不抛异常）。
        let knowledge = KnowledgeRuntime::default();
        let runtime = make_runtime();
        let budget = make_budget(10_000, 8);
        let mut state = ToolDispatchState::new();
        let call = call_with_args("knowledge.delete_all", Document::new());
        let v = dispatch_tool_call(&call, &runtime, &knowledge, &budget, &mut state).await;
        assert_eq!(v.get("error").and_then(|x| x.as_str()), Some("unknown_tool"));
    }

    #[tokio::test]
    async fn dispatch_returns_budget_exceeded_when_quota_zero() {
        // R4.3 / R4.8：tool_call_budget 已满 → record_tool_call 失败 → 返回
        // budget_exceeded；不进入具体 tool 实现。
        let knowledge = KnowledgeRuntime::default();
        let runtime = make_runtime();
        let budget = make_budget(10_000, 0);
        let mut state = ToolDispatchState::new();
        let call = call_with_args(TOOL_LIST_CATALOG, doc! {"kind": "chunks"});
        let v = dispatch_tool_call(&call, &runtime, &knowledge, &budget, &mut state).await;
        assert_eq!(
            v.get("error").and_then(|x| x.as_str()),
            Some("budget_exceeded")
        );
    }

    #[tokio::test]
    async fn dispatch_list_catalog_happy_path_consumes_budget() {
        // 端到端：合法 list_catalog 调用应成功返回，并消耗 1 次 tool_call 配额。
        let knowledge = KnowledgeRuntime {
            documents: vec![],
            items: vec![],
            chunks: vec![build_chunk("c1", Some("verified"), None, None)],
        };
        let runtime = make_runtime();
        let budget = make_budget(10_000, 4);
        let mut state = ToolDispatchState::new();
        let call = call_with_args(TOOL_LIST_CATALOG, doc! {"kind": "chunks"});
        let v = dispatch_tool_call(&call, &runtime, &knowledge, &budget, &mut state).await;
        assert!(v.get("error").is_none(), "expected success: {v}");
        assert_eq!(v["kind"].as_str(), Some("chunks"));
        let snap = budget.snapshot();
        assert_eq!(snap.tool_calls_used, 1);
    }
}

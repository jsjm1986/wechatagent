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

use chrono::{Duration as ChronoDuration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt, Document};
use mongodb::options::FindOptions;
use serde::Deserialize;
use serde_json::{json, Value};

use super::budget::{BudgetError, RunBudget};
use super::runtime::UserRuntimeParameters;
use super::types::{KnowledgeRuntime, ToolCallRequest};
use crate::db::Database;
use crate::models::{
    KnowledgeUsageLog, OperationKnowledgeChunk, OperationKnowledgeDocument,
};

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

// knowledge-digest-workstation Phase 5: chat-only async tools。
// 与 user-ops 三大工具物理隔离：仅在 chat tool loop 内派发，
// 永不进 user-ops `dispatch_tool_call`（保持 user-ops gateway 路径不变）。
pub(crate) const TOOL_AUDIT_COMPLETENESS: &str = "knowledge.audit_completeness";
pub(crate) const TOOL_SEARCH_CHUNKS: &str = "knowledge.search_chunks";
pub(crate) const TOOL_PROPOSE_REPAIR: &str = "knowledge.propose_repair";
pub(crate) const TOOL_ANALYZE_LOGS: &str = "knowledge.analyze_logs";
// 让 agent 拥有"对整个知识库的完整观察"能力的 3 个补充工具：
// - open_document：按 documentId 取父文档原文（截断）；
// - inspect_pack：按 itemId 取知识包完整元数据；
// - verify_anchor：传 chunkId + 候选 sourceQuote，立即返回是否能在父文档命中
//   （与 verify gate 同一套 source_anchor_for_quote 模糊 anchor 算法，由 chat
//   route 在 dispatch 时注入回调，避免 knowledge_tools 直接依赖 routes）。
pub(crate) const TOOL_OPEN_DOCUMENT: &str = "knowledge.open_document";
pub(crate) const TOOL_INSPECT_PACK: &str = "knowledge.inspect_pack";
pub(crate) const TOOL_VERIFY_ANCHOR: &str = "knowledge.verify_anchor";

/// 用于 R4.1 toolCalls schema 校验：合法 tool 名白名单。
pub(crate) const ALLOWED_TOOL_NAMES: &[&str] =
    &[TOOL_LIST_CATALOG, TOOL_SEARCH, TOOL_OPEN_SLICE];

/// chat tool loop 的合法 tool 白名单（user-ops 三件套 + 7 个 chat-only 工具）。
pub(crate) const ALLOWED_CHAT_TOOL_NAMES: &[&str] = &[
    TOOL_LIST_CATALOG,
    TOOL_SEARCH,
    TOOL_OPEN_SLICE,
    TOOL_AUDIT_COMPLETENESS,
    TOOL_SEARCH_CHUNKS,
    TOOL_PROPOSE_REPAIR,
    TOOL_ANALYZE_LOGS,
    TOOL_OPEN_DOCUMENT,
    TOOL_INSPECT_PACK,
    TOOL_VERIFY_ANCHOR,
];

/// chat 工具单 turn 调用次数硬上限（与设计保持一致：≤ 6）。
pub(crate) const CHAT_TOOL_CALLS_PER_TURN_CAP: usize = 6;

/// chat analyze_logs 的回看窗口（24h）。
pub(crate) const CHAT_ANALYZE_LOGS_WINDOW_HOURS: i64 = 24;
/// chat analyze_logs 单次返回的 chunk 上限。
pub(crate) const CHAT_ANALYZE_LOGS_MAX_CHUNKS: usize = 32;

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
            // operation_knowledge_items 已删除；items 维度永远空。
            (Vec::<Value>::new(), 0)
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

// ── chat-only tools (knowledge-digest-workstation Phase 5) ─────────────────
//
// 与 user-ops 三件套的关键差异：
// - 这些 tool 直接读 MongoDB（异步），因此走单独的 `dispatch_chat_tool_call`；
// - 仍然受 RunBudget tool_call 配额 + 5s 单 dispatch timeout 约束；
// - 仍然 fail-as-Value（错误也是合法的工具结果，让 LLM 在下一轮自我修正）；
// - 永不写入 outbox / 永不触达 mcp.* / 永不进 user-ops gateway。

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuditCompletenessArgs {
    #[serde(default)]
    chunk_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchChunksArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    top_k: Option<i32>,
    #[serde(default)]
    only_verified: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProposeRepairArgs {
    #[serde(default)]
    chunk_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeLogsArgs {
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    hours: Option<i64>,
    #[serde(default)]
    only_blocked_or_held: Option<bool>,
}

// knowledge-digest-workstation Phase 5 / 工具补完三件套。
//
// open_document：让 agent 直接读父文档原文（截断 4000 字），常见用法是
// update_chunk 之前先看一眼真正的原文段，避免凭 chunk 自身的 sourceQuote 推断；
//
// inspect_pack：取整个知识包的完整元数据（routingCard / commonObjections /
// safeClaims / forbiddenClaims / customerStages 等），让 agent 在 update_pack
// 前知道当前包到底长什么样、哪些字段还没填；
//
// verify_anchor：把 candidate sourceQuote 投到父文档跑一遍 verify gate 的
// 模糊 anchor 算法，返回 hit/miss + offset；让 agent 在生成 sourceQuote 之前
// 自己先校验，而不是把无锚草稿直接抛到 chat_apply（apply 也仍会强制 needs_review，
// 这只是给 agent 一次"主动自检"的能力，与红线"AI 永不自动 verify"不冲突）。

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenDocumentArgs {
    #[serde(default)]
    document_id: Option<String>,
    /// 截断字符数，1..=8000，缺省 4000。超出 → 截到 4000。
    #[serde(default)]
    max_chars: Option<i32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectPackArgs {
    #[serde(default)]
    item_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyAnchorArgs {
    #[serde(default)]
    chunk_id: Option<String>,
    /// 待校验的 sourceQuote 候选；缺省 → 用 chunk 当前已存的 source_quote。
    #[serde(default)]
    source_quote: Option<String>,
}

/// `verify_anchor` 模糊匹配回调签名：
/// `(raw_content, document_id_hex, source_quote) -> Option<doc!{ ... }>`。
/// 由 chat route 注入；为空时 verify_anchor 工具退化为 best-effort 子串匹配。
pub(crate) type AnchorMatchFn =
    fn(&str, Option<String>, &str) -> Option<mongodb::bson::Document>;

/// 异步派发 chat tool call。
///
/// 行为概览（与 sync 版本对齐）：
/// 1. 校验 `call.tool` 在 chat 白名单内；
/// 2. `RunBudget::record_tool_call` 占 1 槽 + 0 token；
/// 3. 5s 单次 timeout；
/// 4. 错误以 Value 返回（不抛异常）。
///
/// 与 sync `dispatch_tool_call` 隔离：本函数在 chat_tool_loop 内调用，永不与
/// user-ops gateway 共享路径。
pub(crate) async fn dispatch_chat_tool_call(
    call: &ToolCallRequest,
    runtime: &UserRuntimeParameters,
    knowledge: &KnowledgeRuntime,
    db: &Database,
    workspace_id: &str,
    budget: &Arc<RunBudget>,
    state: &mut ToolDispatchState,
    anchor_match: Option<AnchorMatchFn>,
) -> Value {
    let tool = call.tool.trim();
    if !ALLOWED_CHAT_TOOL_NAMES.iter().any(|allowed| *allowed == tool) {
        return tool_error("unknown_tool", &format!("tool name '{tool}' not allowed"));
    }
    if let Err(err) = budget.record_tool_call(0) {
        return budget_error_value(&err);
    }

    let arguments = call.arguments.clone();
    let tool_owned = tool.to_string();
    let workspace_id_owned = workspace_id.to_string();

    // 把所有 dispatch 用 timeout 包住——与 sync 版本对齐。
    let result = tokio::time::timeout(TOOL_DISPATCH_TIMEOUT, async {
        match tool_owned.as_str() {
            TOOL_LIST_CATALOG => exec_list_catalog(&arguments, knowledge, state),
            TOOL_SEARCH => exec_search(&arguments, knowledge, runtime),
            TOOL_OPEN_SLICE => exec_open_slice(&arguments, knowledge, runtime),
            TOOL_AUDIT_COMPLETENESS => {
                exec_audit_completeness(&arguments, db, &workspace_id_owned).await
            }
            TOOL_SEARCH_CHUNKS => {
                exec_search_chunks(&arguments, db, &workspace_id_owned).await
            }
            TOOL_PROPOSE_REPAIR => {
                exec_propose_repair(&arguments, db, &workspace_id_owned).await
            }
            TOOL_ANALYZE_LOGS => {
                exec_analyze_logs(&arguments, db, &workspace_id_owned).await
            }
            TOOL_OPEN_DOCUMENT => {
                exec_open_document(&arguments, db, &workspace_id_owned).await
            }
            TOOL_INSPECT_PACK => {
                exec_inspect_pack(&arguments, db, &workspace_id_owned).await
            }
            TOOL_VERIFY_ANCHOR => {
                exec_verify_anchor(&arguments, db, &workspace_id_owned, anchor_match).await
            }
            _ => unreachable!("chat tool whitelist enforced above"),
        }
    })
    .await;
    match result {
        Ok(v) => v,
        Err(_) => tool_error(
            "tool_timeout",
            &format!("tool '{tool}' exceeded 5s timeout"),
        ),
    }
}

// ── exec: knowledge.audit_completeness ─────────────────────────────────
//
// 输入：{ chunk_id }；
// 输出：{ chunk_id, integrity_status, missing_fields, has_source_quote,
//        verified_claim_count, evidence_count, completeness_score (0..=1) }；
// 错误：invalid_input / unknown_chunk_id / db_error。
async fn exec_audit_completeness(
    arguments: &Document,
    db: &Database,
    workspace_id: &str,
) -> Value {
    let args: AuditCompletenessArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };
    let chunk_id = match args.chunk_id.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => return tool_error("invalid_input", "chunk_id is required"),
    };
    let oid = match ObjectId::parse_str(&chunk_id) {
        Ok(o) => o,
        Err(_) => return tool_error("invalid_input", "chunk_id is not a valid ObjectId"),
    };
    let chunk = match db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": oid, "workspace_id": workspace_id },
            None,
        )
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            return json!({ "error": "unknown_chunk_id", "missing": [chunk_id] });
        }
        Err(e) => return tool_error("db_error", &e.to_string()),
    };

    let mut missing: Vec<&str> = Vec::new();
    if chunk.title.trim().is_empty() {
        missing.push("title");
    }
    if chunk.summary.as_deref().map(str::trim).unwrap_or("").is_empty() {
        missing.push("summary");
    }
    if chunk
        .source_quote
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        missing.push("sourceQuote");
    }
    if chunk.applicable_scenes.is_empty() {
        missing.push("applicableScenes");
    }

    let total_checked = 4.0_f64;
    let filled = (total_checked - missing.len() as f64).max(0.0);
    let completeness_score = (filled / total_checked * 1000.0).round() / 1000.0;

    json!({
        "chunk_id": chunk_id,
        "title": chunk.title,
        "integrity_status": chunk.integrity_status.clone().unwrap_or_default(),
        "status": chunk.status,
        "missing_fields": missing,
        "has_source_quote": chunk.source_quote.is_some(),
        "applicable_scene_count": chunk.applicable_scenes.len(),
        "completeness_score": completeness_score,
        "updated_at": chunk.updated_at.timestamp_millis(),
    })
}

// ── exec: knowledge.search_chunks ──────────────────────────────────────
//
// 输入：{ query, top_k?, only_verified? }
// 输出：{ hits: [{ chunk_id, title, integrity_status, score, snippet,
//                  redacted }], hit_count, query }
// 与 user-ops `knowledge.search` 行为相似但走 db query；snippet 仍按 verified
// 闸门 redact。
async fn exec_search_chunks(
    arguments: &Document,
    db: &Database,
    workspace_id: &str,
) -> Value {
    let args: SearchChunksArgs = match parse_arguments(arguments) {
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
    let top_k = args.top_k.filter(|v| *v > 0).unwrap_or(8).min(32).max(1) as usize;
    let only_verified = args.only_verified.unwrap_or(false);

    // 简单做法：拉前 200 条候选 in-memory 评分（避免引入 $text 索引依赖）。
    let mut filter = doc! { "workspace_id": workspace_id };
    if only_verified {
        filter.insert("integrity_status", "verified");
    }
    let cursor = match db
        .operation_knowledge_chunks()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1_i32 })
                .limit(200)
                .build(),
        )
        .await
    {
        Ok(c) => c,
        Err(e) => return tool_error("db_error", &e.to_string()),
    };
    let chunks: Vec<OperationKnowledgeChunk> = match cursor.try_collect().await {
        Ok(v) => v,
        Err(e) => return tool_error("db_error", &e.to_string()),
    };

    let mut scored: Vec<(f64, &OperationKnowledgeChunk)> = chunks
        .iter()
        .filter_map(|c| {
            let s = score_chunk_for_query(c, query);
            if s > 0.0 {
                Some((s, c))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let hits: Vec<Value> = scored
        .into_iter()
        .take(top_k)
        .map(|(score, c)| build_search_hit(c, score))
        .collect();
    let hit_count = hits.len();
    json!({
        "query": query.to_string(),
        "hits": hits,
        "hit_count": hit_count,
        "only_verified": only_verified,
    })
}

// ── exec: knowledge.propose_repair ─────────────────────────────────────
//
// 输入：{ chunk_id }
// 输出：{ chunk_id, suggestions: [...] }，每条 suggestion 形如
// { field, reason, hint }；不直接写库（与 AI 永不自动 verify 红线一致）。
async fn exec_propose_repair(
    arguments: &Document,
    db: &Database,
    workspace_id: &str,
) -> Value {
    let args: ProposeRepairArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };
    let chunk_id = match args.chunk_id.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => return tool_error("invalid_input", "chunk_id is required"),
    };
    let oid = match ObjectId::parse_str(&chunk_id) {
        Ok(o) => o,
        Err(_) => return tool_error("invalid_input", "chunk_id is not a valid ObjectId"),
    };
    let chunk = match db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": oid, "workspace_id": workspace_id },
            None,
        )
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            return json!({ "error": "unknown_chunk_id", "missing": [chunk_id] });
        }
        Err(e) => return tool_error("db_error", &e.to_string()),
    };

    let mut suggestions: Vec<Value> = Vec::new();
    if chunk
        .source_quote
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        suggestions.push(json!({
            "field": "sourceQuote",
            "reason": "缺少原文出处，无法走 verify 模糊 anchor 校验",
            "hint": "请粘贴一段父文档中支撑本切片结论的原文（≥10 字）",
            "severity": "high",
        }));
    }
    if chunk.applicable_scenes.is_empty() {
        suggestions.push(json!({
            "field": "applicableScenes",
            "reason": "未声明适用场景，路由器命中率低",
            "hint": "补 1-3 个场景标签（如『售前/异议处理/复购』）",
            "severity": "medium",
        }));
    }
    let integrity = chunk.integrity_status.clone().unwrap_or_default();
    if integrity != "verified" && integrity != "needs_review" {
        suggestions.push(json!({
            "field": "integrityStatus",
            "reason": format!("当前状态 '{integrity}' 非 verified/needs_review；运营对话起草后应回到 needs_review 等待复核"),
            "hint": "对话生成新内容后保持 status=draft + integrityStatus=needs_review",
            "severity": "low",
        }));
    }

    json!({
        "chunk_id": chunk_id,
        "title": chunk.title,
        "integrity_status": integrity,
        "suggestion_count": suggestions.len(),
        "suggestions": suggestions,
        "ai_will_not_auto_apply": true,
    })
}

// ── exec: knowledge.analyze_logs ───────────────────────────────────────
//
// 输入：{ account_id?, hours?, only_blocked_or_held? }
// 输出：{ window_hours, total_runs, blocked_or_held_runs, top_chunks, items }
// items[i]: { run_id, blocked_reason, knowledge_ids, created_at }
// 复用 KnowledgeUsageLog 的 blocked_reason 字段做 24h 块/hold 反查。
async fn exec_analyze_logs(
    arguments: &Document,
    db: &Database,
    workspace_id: &str,
) -> Value {
    let args: AnalyzeLogsArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };
    let hours = args
        .hours
        .filter(|v| *v > 0)
        .unwrap_or(CHAT_ANALYZE_LOGS_WINDOW_HOURS)
        .min(72);
    let only_blocked = args.only_blocked_or_held.unwrap_or(true);
    let cutoff = Utc::now() - ChronoDuration::hours(hours);
    let cutoff_bson = BsonDt::from_millis(cutoff.timestamp_millis());

    let mut filter = doc! {
        "workspace_id": workspace_id,
        "created_at": { "$gte": cutoff_bson },
    };
    if let Some(account_id) = args
        .account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        filter.insert("account_id", account_id);
    }
    if only_blocked {
        filter.insert(
            "$or",
            mongodb::bson::Bson::Array(vec![
                mongodb::bson::Bson::Document(doc! { "review_approved": false }),
                mongodb::bson::Bson::Document(doc! {
                    "blocked_reason": { "$exists": true, "$ne": null },
                }),
            ]),
        );
    }

    let cursor = match db
        .knowledge_usage_logs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1_i32 })
                .limit(CHAT_ANALYZE_LOGS_MAX_CHUNKS as i64)
                .build(),
        )
        .await
    {
        Ok(c) => c,
        Err(e) => return tool_error("db_error", &e.to_string()),
    };
    let logs: Vec<KnowledgeUsageLog> = match cursor.try_collect().await {
        Ok(v) => v,
        Err(e) => return tool_error("db_error", &e.to_string()),
    };

    let mut chunk_freq: HashMap<String, i32> = HashMap::new();
    let mut items: Vec<Value> = Vec::with_capacity(logs.len());
    let total_runs = logs.len();
    let mut blocked = 0_i32;
    for log in &logs {
        if log.blocked_reason.is_some() || !log.review_approved {
            blocked += 1;
        }
        for kid in &log.knowledge_ids {
            *chunk_freq.entry(kid.to_hex()).or_insert(0) += 1;
        }
        items.push(json!({
            "run_id": log.run_id,
            "account_id": log.account_id,
            "blocked_reason": log.blocked_reason,
            "review_approved": log.review_approved,
            "knowledge_ids": log.knowledge_ids.iter().map(|o| o.to_hex()).collect::<Vec<_>>(),
            "created_at": log.created_at.timestamp_millis(),
        }));
    }
    let mut top_chunks: Vec<(String, i32)> = chunk_freq.into_iter().collect();
    top_chunks.sort_by(|a, b| b.1.cmp(&a.1));
    let top_chunks_json: Vec<Value> = top_chunks
        .into_iter()
        .take(8)
        .map(|(id, count)| json!({ "chunk_id": id, "hit_count": count }))
        .collect();

    json!({
        "window_hours": hours,
        "only_blocked_or_held": only_blocked,
        "total_runs": total_runs,
        "blocked_or_held_runs": blocked,
        "top_chunks": top_chunks_json,
        "items": items,
    })
}

// ── exec: knowledge.open_document ──────────────────────────────────────
//
// 输入：{ document_id, max_chars? }；max_chars 缺省 4000，硬上限 8000。
// 输出：{ document_id, title, source_type, raw_content_excerpt, raw_content_truncated,
//        raw_content_total_chars, summary }；
// 错误：invalid_input / unknown_document_id / db_error。
async fn exec_open_document(
    arguments: &Document,
    db: &Database,
    workspace_id: &str,
) -> Value {
    let args: OpenDocumentArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };
    let document_id = match args
        .document_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(id) => id.to_string(),
        None => return tool_error("invalid_input", "document_id is required"),
    };
    let oid = match ObjectId::parse_str(&document_id) {
        Ok(o) => o,
        Err(_) => {
            return tool_error("invalid_input", "document_id is not a valid ObjectId")
        }
    };
    let max_chars = args
        .max_chars
        .filter(|v| *v > 0)
        .unwrap_or(4000)
        .min(8000) as usize;
    let doc_record: OperationKnowledgeDocument = match db
        .operation_knowledge_documents()
        .find_one(
            doc! { "_id": oid, "workspace_id": workspace_id },
            None,
        )
        .await
    {
        Ok(Some(d)) => d,
        Ok(None) => return json!({ "error": "unknown_document_id", "missing": [document_id] }),
        Err(e) => return tool_error("db_error", &e.to_string()),
    };
    let raw = doc_record.raw_content.clone().unwrap_or_default();
    let total_chars = raw.chars().count();
    let truncated = total_chars > max_chars;
    let excerpt: String = if truncated {
        raw.chars().take(max_chars).collect()
    } else {
        raw
    };
    json!({
        "document_id": document_id,
        "title": doc_record.title,
        "source_type": doc_record.source_type,
        "source_name": doc_record.source_name,
        "summary": doc_record.summary,
        "catalog_summary": doc_record.catalog_summary,
        "status": doc_record.status,
        "raw_content_excerpt": excerpt,
        "raw_content_truncated": truncated,
        "raw_content_total_chars": total_chars as i32,
        "max_chars": max_chars as i32,
        "updated_at": doc_record.updated_at.timestamp_millis(),
    })
}

// ── exec: knowledge.inspect_pack ───────────────────────────────────────
//
// 输入：{ item_id }
// 输出：{ item_id, title, routing_card, summary, customer_stages, intent_levels,
//        common_questions, common_objections, safe_claims, forbidden_claims,
//        evidence_items, applicable_scenes, not_applicable_scenes,
//        product_tags, business_topics, status, updated_at }
// 错误：invalid_input / unknown_item_id / db_error。
async fn exec_inspect_pack(
    arguments: &Document,
    _db: &Database,
    _workspace_id: &str,
) -> Value {
    // operation_knowledge_items 已删除；inspect_pack 永久返回 unknown_item_id。
    let args: InspectPackArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };
    let item_id = args.item_id.unwrap_or_default();
    json!({ "error": "unknown_item_id", "missing": [item_id] })
}

// ── exec: knowledge.verify_anchor ──────────────────────────────────────
//
// 输入：{ chunk_id, source_quote? }；缺省 source_quote 时用 chunk 当前已存的。
// 输出：{ chunk_id, document_id?, anchor_hit, anchor?, source_quote_used,
//        method: "exact"|"fuzzy"|"none", note? }
// 错误：invalid_input / missing_parent_document / unknown_chunk_id / db_error。
async fn exec_verify_anchor(
    arguments: &Document,
    db: &Database,
    workspace_id: &str,
    anchor_match: Option<AnchorMatchFn>,
) -> Value {
    let args: VerifyAnchorArgs = match parse_arguments(arguments) {
        Ok(args) => args,
        Err(detail) => return tool_error("invalid_input", &detail),
    };
    let chunk_id = match args
        .chunk_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(id) => id.to_string(),
        None => return tool_error("invalid_input", "chunk_id is required"),
    };
    let oid = match ObjectId::parse_str(&chunk_id) {
        Ok(o) => o,
        Err(_) => return tool_error("invalid_input", "chunk_id is not a valid ObjectId"),
    };
    let chunk: OperationKnowledgeChunk = match db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": oid, "workspace_id": workspace_id },
            None,
        )
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return json!({ "error": "unknown_chunk_id", "missing": [chunk_id] }),
        Err(e) => return tool_error("db_error", &e.to_string()),
    };
    let document_oid = match chunk.document_id {
        Some(d) => d,
        None => {
            return json!({
                "chunk_id": chunk_id,
                "anchor_hit": false,
                "method": "none",
                "note": "chunk has no parent document_id; cannot verify anchor",
            });
        }
    };
    let document_id_hex = document_oid.to_hex();
    let parent: OperationKnowledgeDocument = match db
        .operation_knowledge_documents()
        .find_one(
            doc! { "_id": document_oid, "workspace_id": workspace_id },
            None,
        )
        .await
    {
        Ok(Some(d)) => d,
        Ok(None) => {
            return tool_error(
                "missing_parent_document",
                &format!("document {document_id_hex} not found"),
            );
        }
        Err(e) => return tool_error("db_error", &e.to_string()),
    };
    let raw_content = parent.raw_content.clone().unwrap_or_default();
    let candidate = args
        .source_quote
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| chunk.source_quote.clone().unwrap_or_default());
    let candidate_trimmed = candidate.trim();
    if candidate_trimmed.is_empty() {
        return json!({
            "chunk_id": chunk_id,
            "document_id": document_id_hex,
            "anchor_hit": false,
            "method": "none",
            "note": "candidate source_quote is empty",
        });
    }
    if let Some(start) = raw_content.find(candidate_trimmed) {
        let end = start + candidate_trimmed.len();
        return json!({
            "chunk_id": chunk_id,
            "document_id": document_id_hex,
            "source_quote_used": candidate_trimmed,
            "anchor_hit": true,
            "method": "exact",
            "anchor": {
                "startOffset": start as i32,
                "endOffset": end as i32,
            },
        });
    }
    // 退回模糊：若调用方注入了 anchor_match，复用 verify gate 同算法。
    if let Some(matcher) = anchor_match {
        if let Some(anchor) = matcher(&raw_content, Some(document_id_hex.clone()), candidate_trimmed) {
            let value = mongodb::bson::Bson::Document(anchor).into_relaxed_extjson();
            return json!({
                "chunk_id": chunk_id,
                "document_id": document_id_hex,
                "source_quote_used": candidate_trimmed,
                "anchor_hit": true,
                "method": "fuzzy",
                "anchor": value,
            });
        }
    }
    json!({
        "chunk_id": chunk_id,
        "document_id": document_id_hex,
        "source_quote_used": candidate_trimmed,
        "anchor_hit": false,
        "method": "none",
        "note": "candidate quote did not match parent document (exact + fuzzy both miss)",
    })
}

// ── 内部辅助 ────────────────────────────────────────────────────────────

fn parse_arguments<T: for<'de> Deserialize<'de> + Default>(arguments: &Document) -> Result<T, String> {
    if arguments.is_empty() {
        return Ok(T::default());
    }
    let value = mongodb::bson::Bson::Document(arguments.clone()).into_relaxed_extjson();
    serde_json::from_value::<T>(value).map_err(|e| format!("invalid arguments: {e}"))
}

// item_summary removed: OperationKnowledgeItem 已随 sales 旧库删除。

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
    use crate::models::{OperationKnowledgeChunk, OperationKnowledgeDocument};
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
            applicable_scenes: vec![],
            not_applicable_scenes: vec![],
            source_quote: Some("doc#1".into()),
            source_anchors: vec![],
            integrity_status: integrity.map(ToString::to_string),
            confidence_score: None,
            status: "active".into(),
            priority: 0,
            product_tags: vec![],
            business_topics: vec![],
            created_at: BsonDt::now(),
            updated_at: BsonDt::now(),
            wiki_type: None,
            domain_attributes: None,
            provenance: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            previous_version_id: None,
            related_chunks: None,
            usage_stats: None,
            dynamic_confidence: None,
            integrity_score: None,
            locked_fields: None,
            chunk_type: "product_fact".to_string(),
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
            product_tags: vec![],
            business_topics: vec![],
            created_at: BsonDt::now(),
            updated_at: BsonDt::now(),
            catalog_summary_persisted: None,
            catalog_version: None,
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

    // P2-11：锁住 7 个 chat-only Args struct 的 camelCase 反序列化契约。
    // LLM 通过 tool_calling 把 JSON 入参传进来，字段是 camelCase（chunkId / topK / onlyVerified
    // / accountId / sourceQuote 等）；P0-1 修复后所有 chat Args 都打了 #[serde(rename_all =
    // "camelCase")]，本组测试防止后续误删 / 误改 attr 导致 LLM 入参全部反序列化为 None。
    //
    // 走 BSON Document → bson::from_document 路径，与生产路径 parse_arguments 一致。

    #[test]
    fn audit_completeness_args_accept_camel_case() {
        let d = doc! { "chunkId": "c-1" };
        let a: AuditCompletenessArgs = mongodb::bson::from_document(d).expect("camelCase 反序列化必须通过");
        assert_eq!(a.chunk_id.as_deref(), Some("c-1"));
    }

    #[test]
    fn search_chunks_args_accept_camel_case() {
        let d = doc! { "query": "宝妈", "topK": 8_i32, "onlyVerified": true };
        let a: SearchChunksArgs = mongodb::bson::from_document(d).expect("camelCase 反序列化必须通过");
        assert_eq!(a.query.as_deref(), Some("宝妈"));
        assert_eq!(a.top_k, Some(8));
        assert_eq!(a.only_verified, Some(true));
    }

    #[test]
    fn propose_repair_args_accept_camel_case() {
        let d = doc! { "chunkId": "c-9" };
        let a: ProposeRepairArgs = mongodb::bson::from_document(d).expect("camelCase 反序列化必须通过");
        assert_eq!(a.chunk_id.as_deref(), Some("c-9"));
    }

    #[test]
    fn analyze_logs_args_accept_camel_case() {
        let d = doc! { "accountId": "acc-1", "hours": 24_i64, "onlyBlockedOrHeld": true };
        let a: AnalyzeLogsArgs = mongodb::bson::from_document(d).expect("camelCase 反序列化必须通过");
        assert_eq!(a.account_id.as_deref(), Some("acc-1"));
        assert_eq!(a.hours, Some(24));
        assert_eq!(a.only_blocked_or_held, Some(true));
    }

    #[test]
    fn open_document_args_accept_camel_case() {
        let d = doc! { "documentId": "doc-x", "maxChars": 4000_i32 };
        let a: OpenDocumentArgs = mongodb::bson::from_document(d).expect("camelCase 反序列化必须通过");
        assert_eq!(a.document_id.as_deref(), Some("doc-x"));
        assert_eq!(a.max_chars, Some(4000));
    }

    #[test]
    fn inspect_pack_args_accept_camel_case() {
        let d = doc! { "itemId": "p-7" };
        let a: InspectPackArgs = mongodb::bson::from_document(d).expect("camelCase 反序列化必须通过");
        assert_eq!(a.item_id.as_deref(), Some("p-7"));
    }

    #[test]
    fn verify_anchor_args_accept_camel_case() {
        let d = doc! { "chunkId": "c-2", "sourceQuote": "原文片段≥10字" };
        let a: VerifyAnchorArgs = mongodb::bson::from_document(d).expect("camelCase 反序列化必须通过");
        assert_eq!(a.chunk_id.as_deref(), Some("c-2"));
        assert_eq!(a.source_quote.as_deref(), Some("原文片段≥10字"));
    }

    #[test]
    fn snake_case_keys_do_not_populate_chat_args_after_p0_1() {
        // 反向断言：snake_case 字段名应被忽略（serde rename_all 是单向 camelCase 接收）；
        // 防止后续有人把 #[serde(alias = "chunk_id")] 加进来又恢复为旧的双向 alias 路径。
        let d = doc! { "chunk_id": "c-1" };
        let a: AuditCompletenessArgs = mongodb::bson::from_document(d).unwrap_or_default();
        assert!(a.chunk_id.is_none(), "snake_case 不应被识别（rename_all=camelCase 单向）");
    }
}

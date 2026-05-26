//! Agent-first 渐进式披露知识库 agent（替代 BM25 / 向量召回路径）。
//!
//! 设计目标：让 LLM 自己驱动 wiki chunk 探索，按 skills 那样的"按需披露"模式：
//! - 工具集：`list_catalog`（只看目录摘要） / `open_chunk`（按需展开正文+引用） /
//!   `follow_relations`（沿 related_chunks 跳）。
//! - 多轮预算：≤ 3 轮 LLM 决策（catalog → open → follow / answer）；超出强制
//!   answer 当前已 opened chunks。
//! - 预算复用：所有 LLM 调用走 [`super::generate_agent_json`]，自动累计到当前
//!   run 的 [`super::RunBudget`]；上游 gateway 已用掉大半 budget 时，本 agent
//!   会被自然挤掉早 answer。
//! - 不写 chunk：本 agent 是**只读**面向 agent 的检索面，不触发 patch / verify。
//! - 隔离：本模块**不**引用 `gateway` / `outbox` / `mcp` / `agent_send_outbox`，
//!   保持 agent 子模块对运营网关零耦合，可独立给 `/api/knowledge/ask` 用。

use std::collections::HashSet;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, to_bson, Document};
use mongodb::options::FindOptions;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::AppResult;
use crate::models::OperationKnowledgeChunk;
use crate::routes::AppState;

use super::budget::current_run_budget;
use super::generate_agent_json;

/// 单轮探索的硬上限：3 轮 LLM 决策（catalog → open / follow → answer）。
/// 第 4 轮直接强制 answer；与 [`super::RunBudget::max_llm_calls`] 互不替代——
/// budget 用尽更早跳出循环。
const MAX_ROUNDS: i32 = 3;

/// `list_catalog` 一次返回 chunk 摘要的硬上限，控制 prompt size。
const CATALOG_PAGE_SIZE: usize = 30;

/// 一次 `open_chunk` 最多展开几条 chunk，避免 prompt 爆炸。
const OPEN_CHUNK_BATCH: usize = 8;

/// `follow_relations` 单跳/双跳最大展开数量。
const FOLLOW_RELATIONS_LIMIT: usize = 16;

/// `summary` 在 catalog 中的截断长度（按 char 数算，CJK 友好）。
const CATALOG_SUMMARY_CHARS: usize = 120;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AnswerRequest {
    pub workspace_id: String,
    /// account_id 为 None 时只看 workspace 共享 chunk。
    pub account_id: Option<String>,
    pub query: String,
    /// 初始过滤；为空时返回整个 workspace 的 catalog 头部。
    #[serde(default)]
    pub filter: CatalogFilter,
    /// 客户端可选提示：希望 ≤ 多少轮 answer。clamp 到 `[1, MAX_ROUNDS]`。
    #[serde(default)]
    pub max_rounds: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFilter {
    #[serde(default)]
    pub wiki_types: Vec<String>,
    #[serde(default)]
    pub business_topics: Vec<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// 默认 false：catalog / open_chunk 仅暴露 `integrity_status="verified"` chunk，
    /// 与 [`super::knowledge_router::load_operation_knowledge`] 的 verified-only
    /// 加载对齐。设为 true 时上层（如内部审阅工具）可越权拉取 needs_review / draft
    /// chunk，但 router 路径永远走 false。
    #[serde(default)]
    pub include_unverified: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub chunk_id: String,
    pub wiki_type: String,
    pub chunk_type: String,
    pub title: String,
    pub summary: String,
    pub business_topics: Vec<String>,
    pub verified: bool,
    pub has_source_quote: bool,
    pub dynamic_confidence: f64,
    pub related_count: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkFull {
    pub chunk_id: String,
    pub wiki_type: String,
    pub chunk_type: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub source_quote: Option<String>,
    pub source_anchors: Vec<Document>,
    pub related_chunks: Vec<RelatedRefView>,
    pub verified: bool,
    pub business_topics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedRefView {
    pub chunk_id: String,
    pub kind: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceQuoteCitation {
    pub chunk_id: String,
    pub quote: String,
    /// 在 chunk.source_anchors 中对应的下标；越界视为 None。
    pub source_anchor_index: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerResult {
    pub answer: String,
    pub cited_chunk_ids: Vec<String>,
    pub source_quotes: Vec<SourceQuoteCitation>,
    pub tool_trace: Vec<Document>,
    pub rounds_used: i32,
    /// true 表示 3 轮内未给出 answer，由兜底逻辑强制返回。
    pub truncated: bool,
}

/// LLM 在每一轮回包必须遵守的 action 协议。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum AgentAction {
    ListCatalog {
        #[serde(default)]
        filter: CatalogFilter,
    },
    OpenChunk {
        #[serde(default)]
        ids: Vec<String>,
    },
    FollowRelations {
        #[serde(default, alias = "chunkId")]
        chunk_id: String,
        #[serde(default)]
        depth: Option<u32>,
    },
    Answer {
        #[serde(default, alias = "citedChunkIds")]
        cited_chunk_ids: Vec<String>,
        #[serde(default, alias = "sourceQuotes")]
        source_quotes: Vec<RawSourceQuote>,
        #[serde(default)]
        answer: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSourceQuote {
    #[serde(default, alias = "chunkId")]
    pub chunk_id: String,
    #[serde(default)]
    pub quote: String,
    #[serde(default, alias = "sourceAnchorIndex")]
    pub source_anchor_index: Option<i32>,
}

const SYSTEM_PROMPT: &str = "你是运营知识库的 wiki 研究员。\n\
你必须按 skills 的渐进式披露模式工作：先看 catalog 摘要，再选择性地 open_chunk 完整正文，最后给出带引用的 answer。\n\
你不能凭空回答；任何回答都必须来自被你 open 过的 chunk。\n\
你只输出严格 JSON。每轮只能输出 4 个 action 之一：list_catalog / open_chunk / follow_relations / answer。\n\
最多 3 轮工具调用。最后一轮必须 answer。";

/// 知识库 agent 主循环。
///
/// 流程：
/// 1. 先取一份 catalog（30 条摘要）；
/// 2. 进入 ≤ 3 轮 LLM 循环；每轮把"已 open 的 chunk + 当前 catalog + 已收集的 follow"
///    一起喂给 LLM，让它输出下一步 action；
/// 3. 收到 `answer` action 立即返回；
/// 4. 超过 3 轮或 budget 用尽 → 用当前 opened 强制 answer，标 `truncated=true`。
pub async fn answer(state: &AppState, req: AnswerRequest) -> AppResult<AnswerResult> {
    let max_rounds = req
        .max_rounds
        .unwrap_or(MAX_ROUNDS)
        .clamp(1, MAX_ROUNDS);

    let mut tool_trace: Vec<Document> = Vec::new();
    let mut opened: Vec<ChunkFull> = Vec::new();
    let mut opened_seen: HashSet<String> = HashSet::new();
    let mut catalog =
        list_catalog(state, &req.workspace_id, req.account_id.as_deref(), &req.filter).await?;
    tool_trace.push(doc! {
        "tool": "list_catalog",
        "filter": filter_to_doc(&req.filter),
        "returned": catalog.len() as i32,
    });

    if catalog.is_empty() {
        return Ok(AnswerResult {
            answer: "知识库无相关内容。".to_string(),
            cited_chunk_ids: Vec::new(),
            source_quotes: Vec::new(),
            tool_trace,
            rounds_used: 0,
            truncated: false,
        });
    }

    // last_completed_round：跟踪实际跑完了多少轮（含 budget_exceeded 提前 break /
    // invalid_action continue 的情况）。供兜底 / budget 提前退出时上报真实
    // rounds_used，避免前端误以为"用了 max_rounds 才放弃"。
    let mut last_completed_round: i32 = 0;
    for round in 1..=max_rounds {
        if let Some(budget) = current_run_budget() {
            if budget.is_exceeded() {
                tool_trace.push(doc! {
                    "tool": "budget_exceeded",
                    "round": round,
                });
                break;
            }
        }
        last_completed_round = round;

        let user_prompt = build_prompt(&req.query, &opened, &catalog, round, max_rounds);
        let value = generate_agent_json(
            state,
            req.account_id.as_deref(),
            None,
            None,
            "knowledge.agent",
            SYSTEM_PROMPT,
            &user_prompt,
        )
        .await?;

        let action = match serde_json::from_value::<AgentAction>(value.clone()) {
            Ok(action) => action,
            Err(err) => {
                tool_trace.push(doc! {
                    "tool": "error",
                    "round": round,
                    "reason": format!("invalid_action:{err}"),
                    "raw": value.to_string(),
                });
                continue;
            }
        };

        match action {
            AgentAction::ListCatalog { filter } => {
                catalog = list_catalog(
                    state,
                    &req.workspace_id,
                    req.account_id.as_deref(),
                    &filter,
                )
                .await?;
                tool_trace.push(doc! {
                    "tool": "list_catalog",
                    "round": round,
                    "filter": filter_to_doc(&filter),
                    "returned": catalog.len() as i32,
                });
            }
            AgentAction::OpenChunk { ids } => {
                let mut opened_now: Vec<String> = Vec::new();
                let mut not_found: Vec<String> = Vec::new();
                for id in ids.into_iter().take(OPEN_CHUNK_BATCH) {
                    if opened_seen.contains(&id) {
                        continue;
                    }
                    match open_chunk(state, &req.workspace_id, &id).await? {
                        Some(full) => {
                            opened_seen.insert(id.clone());
                            opened_now.push(id);
                            opened.push(full);
                        }
                        None => {
                            not_found.push(id);
                        }
                    }
                }
                let mut entry = doc! {
                    "tool": "open_chunk",
                    "round": round,
                    "opened": opened_now.clone(),
                };
                if !not_found.is_empty() {
                    entry.insert("notFound", not_found);
                }
                tool_trace.push(entry);
            }
            AgentAction::FollowRelations { chunk_id, depth } => {
                let depth = depth.unwrap_or(1).clamp(1, 2);
                let entries = follow_relations(
                    state,
                    &req.workspace_id,
                    &chunk_id,
                    depth,
                    &opened_seen,
                )
                .await?;
                let appended = entries.len() as i32;
                merge_catalog(&mut catalog, entries);
                tool_trace.push(doc! {
                    "tool": "follow_relations",
                    "round": round,
                    "chunkId": chunk_id,
                    "depth": depth as i32,
                    "appended": appended,
                });
            }
            AgentAction::Answer {
                cited_chunk_ids,
                source_quotes,
                answer,
            } => {
                let (cited, quotes) =
                    filter_answer_against_opened(&opened_seen, cited_chunk_ids, source_quotes);
                tool_trace.push(doc! {
                    "tool": "answer",
                    "round": round,
                    "citedCount": cited.len() as i32,
                    "quoteCount": quotes.len() as i32,
                });
                return Ok(AnswerResult {
                    answer,
                    cited_chunk_ids: cited,
                    source_quotes: quotes,
                    tool_trace,
                    rounds_used: round,
                    truncated: false,
                });
            }
        }
    }

    // 兜底：未在循环内 answer。可能原因：跑完 max_rounds、budget 提前 break、
    // 多次 invalid_action 把轮数耗光。rounds_used 上报真实跑过的轮数（最低 0），
    // 而不是 max_rounds，避免前端误读。
    let cited_chunk_ids: Vec<String> = opened
        .iter()
        .map(|c| c.chunk_id.clone())
        .collect();
    tool_trace.push(doc! {
        "tool": "answer",
        "rounds": last_completed_round,
        "truncated": true,
        "citedCount": cited_chunk_ids.len() as i32,
    });
    Ok(AnswerResult {
        answer: "知识库未在限定轮数内得出结论；已返回当前打开的 chunk 摘要供运营人员判断。".to_string(),
        cited_chunk_ids,
        source_quotes: Vec::new(),
        tool_trace,
        rounds_used: last_completed_round,
        truncated: true,
    })
}

/// 列出 chunk 摘要（不含 body）。按 `dynamic_confidence` × `wiki_type` 优先级
/// 倒排，限制 30 条。account_id=None 时只看 workspace 共享 chunk；带 account_id
/// 时合并查（共享 + 私有）。
pub async fn list_catalog(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    filter: &CatalogFilter,
) -> AppResult<Vec<CatalogEntry>> {
    let mut query = doc! {
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "status": filter.status.clone().unwrap_or_else(|| "active".to_string()),
    };
    // 默认仅暴露 verified chunk（与 router corpus 对齐）。include_unverified=true
    // 由上层显式开启（例如知识库后台审阅 UI 想看 needs_review）。
    if !filter.include_unverified {
        query.insert("integrity_status", "verified");
    }
    let account_or = match account_id {
        Some(id) => vec![doc! { "account_id": null }, doc! { "account_id": id }],
        None => vec![doc! { "account_id": null }],
    };
    query.insert("$or", account_or);
    if !filter.wiki_types.is_empty() {
        query.insert("wiki_type", doc! { "$in": filter.wiki_types.clone() });
    }
    if !filter.business_topics.is_empty() {
        query.insert(
            "business_topics",
            doc! { "$in": filter.business_topics.clone() },
        );
    }

    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            query,
            FindOptions::builder()
                .sort(doc! {
                    "dynamic_confidence": -1,
                    "priority": -1,
                    "updated_at": -1,
                })
                .limit(CATALOG_PAGE_SIZE as i64 * 4)
                .build(),
        )
        .await?;
    let mut chunks: Vec<OperationKnowledgeChunk> = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        chunks.push(item);
    }

    chunks.sort_by(|a, b| {
        let pa = wiki_type_priority(a.wiki_type.as_deref());
        let pb = wiki_type_priority(b.wiki_type.as_deref());
        let ca = a.dynamic_confidence.unwrap_or(0.0);
        let cb = b.dynamic_confidence.unwrap_or(0.0);
        pb.cmp(&pa)
            .then(cb.partial_cmp(&ca).unwrap_or(std::cmp::Ordering::Equal))
            .then(b.priority.cmp(&a.priority))
    });
    chunks.truncate(CATALOG_PAGE_SIZE);

    Ok(chunks.into_iter().map(chunk_to_catalog_entry).collect())
}

/// 打开单条 chunk 的完整正文 + 引用 + relations。
///
/// 默认只返回 `integrity_status="verified"` 的 chunk；非 verified（draft /
/// needs_review）静默返回 `None`，避免 agent cite 到未 verify 的内容。
pub async fn open_chunk(
    state: &AppState,
    workspace_id: &str,
    chunk_id: &str,
) -> AppResult<Option<ChunkFull>> {
    let oid = match ObjectId::parse_str(chunk_id) {
        Ok(oid) => oid,
        Err(_) => return Ok(None),
    };
    let result = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": oid,
                "workspace_id": workspace_id,
                "integrity_status": "verified",
            },
            None,
        )
        .await?;
    Ok(result.map(chunk_to_full))
}

/// 沿 `related_chunks` 跳一跳或两跳；返回的 chunk 摘要去除已 opened 的，避免
/// 重复给 agent。
pub async fn follow_relations(
    state: &AppState,
    workspace_id: &str,
    chunk_id: &str,
    depth: u32,
    opened_seen: &HashSet<String>,
) -> AppResult<Vec<CatalogEntry>> {
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(chunk_id.to_string());
    let mut frontier: Vec<String> = vec![chunk_id.to_string()];
    let mut output: Vec<CatalogEntry> = Vec::new();

    for _ in 0..depth {
        let mut next: Vec<String> = Vec::new();
        for src in frontier.drain(..) {
            let oid = match ObjectId::parse_str(&src) {
                Ok(oid) => oid,
                Err(_) => continue,
            };
            let chunk = state
                .db
                .operation_knowledge_chunks()
                .find_one(
                    doc! { "_id": oid, "workspace_id": workspace_id },
                    None,
                )
                .await?;
            let Some(chunk) = chunk else { continue };
            let related = chunk.related_chunks.unwrap_or_default();
            for rel in related {
                if !visited.insert(rel.chunk_id.clone()) {
                    continue;
                }
                if opened_seen.contains(&rel.chunk_id) {
                    continue;
                }
                let target_oid = match ObjectId::parse_str(&rel.chunk_id) {
                    Ok(oid) => oid,
                    Err(_) => continue,
                };
                if let Some(target) = state
                    .db
                    .operation_knowledge_chunks()
                    .find_one(
                        doc! {
                            "_id": target_oid,
                            "workspace_id": workspace_id,
                            "status": "active",
                            "integrity_status": "verified",
                        },
                        None,
                    )
                    .await?
                {
                    output.push(chunk_to_catalog_entry(target));
                    next.push(rel.chunk_id);
                    if output.len() >= FOLLOW_RELATIONS_LIMIT {
                        return Ok(output);
                    }
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(output)
}

fn chunk_to_catalog_entry(chunk: OperationKnowledgeChunk) -> CatalogEntry {
    let chunk_id = chunk.id.map(|oid| oid.to_hex()).unwrap_or_default();
    let summary = chunk
        .summary
        .clone()
        .or_else(|| chunk.body.clone())
        .map(|s| truncate_chars(&s, CATALOG_SUMMARY_CHARS))
        .unwrap_or_default();
    let related_count = chunk
        .related_chunks
        .as_ref()
        .map(|v| v.len())
        .unwrap_or(0) as i32;
    let verified = chunk
        .integrity_status
        .as_deref()
        .map(|s| s == "verified")
        .unwrap_or(false);
    CatalogEntry {
        chunk_id,
        wiki_type: chunk.wiki_type.clone().unwrap_or_default(),
        chunk_type: chunk.chunk_type.clone(),
        title: chunk.title.clone(),
        summary,
        business_topics: chunk.business_topics.clone(),
        verified,
        has_source_quote: chunk
            .source_quote
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false),
        dynamic_confidence: chunk.dynamic_confidence.unwrap_or(0.0),
        related_count,
    }
}

fn chunk_to_full(chunk: OperationKnowledgeChunk) -> ChunkFull {
    let chunk_id = chunk.id.map(|oid| oid.to_hex()).unwrap_or_default();
    let related_chunks = chunk
        .related_chunks
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|rel| RelatedRefView {
            chunk_id: rel.chunk_id,
            kind: rel.kind,
            note: rel.note,
        })
        .collect();
    let verified = chunk
        .integrity_status
        .as_deref()
        .map(|s| s == "verified")
        .unwrap_or(false);
    ChunkFull {
        chunk_id,
        wiki_type: chunk.wiki_type.unwrap_or_default(),
        chunk_type: chunk.chunk_type,
        title: chunk.title,
        summary: chunk.summary.unwrap_or_default(),
        body: chunk.body.unwrap_or_default(),
        source_quote: chunk.source_quote,
        source_anchors: chunk.source_anchors,
        related_chunks,
        verified,
        business_topics: chunk.business_topics,
    }
}

fn merge_catalog(target: &mut Vec<CatalogEntry>, incoming: Vec<CatalogEntry>) {
    let mut seen: HashSet<String> = target.iter().map(|e| e.chunk_id.clone()).collect();
    for entry in incoming {
        if seen.insert(entry.chunk_id.clone()) {
            target.push(entry);
        }
    }
    if target.len() > CATALOG_PAGE_SIZE * 2 {
        target.truncate(CATALOG_PAGE_SIZE * 2);
    }
}

/// PBT 入口：merge_catalog 的纯版本，便于在 tests/knowledge_agent_pbt.rs
/// 验证幂等性 / 去重 / 顺序保留。
pub fn merge_catalog_pure(target: &mut Vec<CatalogEntry>, incoming: Vec<CatalogEntry>) {
    merge_catalog(target, incoming)
}

/// 从 LLM 输出的 cited_chunk_ids / source_quotes 过滤出仅包含已 open 过的
/// chunk 的子集。这是 plan v3 P0 的"answer subset of opened"硬不变量：
/// LLM 不许凭空创造没有打开过的 chunk_id。
///
/// **公开供 PBT 使用**——`tests/knowledge_agent_pbt.rs` 把这个函数作为
/// 纯逻辑测试入口，验证：
/// 1. cited 永远是 opened_seen 的子集；
/// 2. quote.chunk_id 必须命中 opened_seen 才保留；
/// 3. 空 quote chunk_id / 不在 opened_seen 中的全部丢掉。
pub fn filter_answer_against_opened(
    opened_seen: &HashSet<String>,
    cited_chunk_ids: Vec<String>,
    raw_quotes: Vec<RawSourceQuote>,
) -> (Vec<String>, Vec<SourceQuoteCitation>) {
    let cited: Vec<String> = cited_chunk_ids
        .into_iter()
        .filter(|id| opened_seen.contains(id))
        .collect();
    let quotes: Vec<SourceQuoteCitation> = raw_quotes
        .into_iter()
        .filter(|q| !q.chunk_id.is_empty() && opened_seen.contains(&q.chunk_id))
        .map(|q| SourceQuoteCitation {
            chunk_id: q.chunk_id,
            quote: q.quote,
            source_anchor_index: q.source_anchor_index,
        })
        .collect();
    (cited, quotes)
}

fn build_prompt(
    query: &str,
    opened: &[ChunkFull],
    catalog: &[CatalogEntry],
    round: i32,
    max_rounds: i32,
) -> String {
    let opened_json = serde_json::to_string_pretty(
        &opened
            .iter()
            .map(|c| {
                json!({
                    "chunkId": c.chunk_id,
                    "title": c.title,
                    "wikiType": c.wiki_type,
                    "summary": c.summary,
                    "body": c.body,
                    "sourceQuote": c.source_quote,
                    "sourceAnchors": c.source_anchors.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                    "relatedChunks": c.related_chunks,
                    "verified": c.verified,
                    "businessTopics": c.business_topics,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let catalog_json = serde_json::to_string_pretty(
        &catalog
            .iter()
            .map(|c| json!(c))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let last_round = round >= max_rounds;
    let force_answer_hint = if last_round {
        "\n这是最后一轮，必须输出 action=answer。"
    } else {
        ""
    };
    format!(
        r#"用户查询：
{query}

当前轮次：{round}/{max_rounds}{force_answer_hint}

已 open 的 chunks（含正文）：
{opened_json}

候选 catalog（仅摘要，不含正文）：
{catalog_json}

下一步只输出以下 4 种 action 之一的严格 JSON：
{{"action":"list_catalog","filter":{{"wikiTypes":["..."],"businessTopics":["..."]}}}}
{{"action":"open_chunk","ids":["chunk_id_1","chunk_id_2"]}}
{{"action":"follow_relations","chunkId":"...","depth":1}}
{{"action":"answer","citedChunkIds":["..."],"sourceQuotes":[{{"chunkId":"...","quote":"...","sourceAnchorIndex":0}}],"answer":"..."}}

规则：
- citedChunkIds 必须是上面"已 open 的 chunks"中的 chunkId 子集；不能凭空创造。
- 每个 cited 必须配 sourceQuote；如某 chunk 没有可引用原文，可省略 sourceQuote 但仍可 cite。
- 候选 catalog 中所有 chunk 都已 integrity_status=verified；遇到 verified=false 是异常，不要 cite。
- 当用户查询无相关知识时，answer 直接说"知识库无相关内容"，cited 留空。
- 不要复述 catalog 中的整段 summary；用自然语言总结答复。"#,
        query = query,
        round = round,
        max_rounds = max_rounds,
        force_answer_hint = force_answer_hint,
        opened_json = opened_json,
        catalog_json = catalog_json,
    )
}

fn filter_to_doc(filter: &CatalogFilter) -> Document {
    to_bson(filter)
        .ok()
        .and_then(|b| b.as_document().cloned())
        .unwrap_or_default()
}

/// 9 类 wiki_type 的排序权重（数值越大越优先）。与
/// `knowledge_router::format_operation_knowledge_for_prompt` 的输出顺序一致。
pub fn wiki_type_priority(wiki_type: Option<&str>) -> i32 {
    match wiki_type.unwrap_or("entity") {
        "thesis" => 90,
        "synthesis" => 80,
        "methodology" => 70,
        "finding" => 60,
        "comparison" => 50,
        "concept" => 40,
        "entity" => 30,
        "source" => 20,
        "query" => 10,
        _ => 0,
    }
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

/// PBT 入口：knowledge_agent 多轮预算上限。
pub const PBT_MAX_ROUNDS: i32 = MAX_ROUNDS;
/// PBT 入口：catalog 摘要截断长度（CJK char 数）。
pub const PBT_CATALOG_SUMMARY_CHARS: usize = CATALOG_SUMMARY_CHARS;
/// PBT 入口：catalog 单页上限。
pub const PBT_CATALOG_PAGE_SIZE: usize = CATALOG_PAGE_SIZE;

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_helpers {
    use super::*;

    /// 给 PBT 用：把任意字符串当作 chunk_id 在内存里造一条 ChunkFull。
    pub fn synthetic_chunk(chunk_id: &str, title: &str, body: &str) -> ChunkFull {
        ChunkFull {
            chunk_id: chunk_id.to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: title.to_string(),
            summary: title.to_string(),
            body: body.to_string(),
            source_quote: Some(body.to_string()),
            source_anchors: Vec::new(),
            related_chunks: Vec::new(),
            verified: true,
            business_topics: Vec::new(),
        }
    }

    pub fn synthetic_catalog_entry(chunk_id: &str, title: &str) -> CatalogEntry {
        CatalogEntry {
            chunk_id: chunk_id.to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: title.to_string(),
            summary: title.to_string(),
            business_topics: Vec::new(),
            verified: true,
            has_source_quote: true,
            dynamic_confidence: 0.9,
            related_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_handles_cjk_boundary() {
        let input = "三步价格异议处理方法论：先共情、后说价值、最后给方案。";
        let out = truncate_chars(input, 5);
        assert_eq!(out.chars().count(), 6); // 5 chars + 省略号
        assert!(out.ends_with('…'));
    }

    #[test]
    fn wiki_type_priority_orders_thesis_above_source() {
        assert!(wiki_type_priority(Some("thesis")) > wiki_type_priority(Some("source")));
        assert!(wiki_type_priority(Some("methodology")) > wiki_type_priority(Some("entity")));
        assert_eq!(wiki_type_priority(None), wiki_type_priority(Some("entity")));
    }

    #[test]
    fn merge_catalog_dedups_by_chunk_id() {
        let mut base = vec![CatalogEntry {
            chunk_id: "a".to_string(),
            wiki_type: "entity".to_string(),
            chunk_type: "product_fact".to_string(),
            title: String::new(),
            summary: String::new(),
            business_topics: Vec::new(),
            verified: false,
            has_source_quote: false,
            dynamic_confidence: 0.0,
            related_count: 0,
        }];
        let incoming = vec![
            CatalogEntry {
                chunk_id: "a".to_string(),
                wiki_type: "entity".to_string(),
                chunk_type: "product_fact".to_string(),
                title: String::new(),
                summary: String::new(),
                business_topics: Vec::new(),
                verified: false,
                has_source_quote: false,
                dynamic_confidence: 0.0,
                related_count: 0,
            },
            CatalogEntry {
                chunk_id: "b".to_string(),
                wiki_type: "entity".to_string(),
                chunk_type: "product_fact".to_string(),
                title: String::new(),
                summary: String::new(),
                business_topics: Vec::new(),
                verified: false,
                has_source_quote: false,
                dynamic_confidence: 0.0,
                related_count: 0,
            },
        ];
        merge_catalog(&mut base, incoming);
        assert_eq!(base.len(), 2);
        assert_eq!(base[0].chunk_id, "a");
        assert_eq!(base[1].chunk_id, "b");
    }

    /// `Answer` action 的 cited_chunk_ids 子集断言由 PBT 覆盖；这里只验证
    /// 最朴素的"action 反序列化"路径。
    #[test]
    fn parse_answer_action_with_camel_alias() {
        let raw: serde_json::Value = serde_json::from_str(
            r#"{"action":"answer","citedChunkIds":["c1"],"sourceQuotes":[{"chunkId":"c1","quote":"q","sourceAnchorIndex":0}],"answer":"hello"}"#,
        )
        .unwrap();
        let action: AgentAction = serde_json::from_value(raw).unwrap();
        match action {
            AgentAction::Answer {
                cited_chunk_ids,
                source_quotes,
                answer,
            } => {
                assert_eq!(cited_chunk_ids, vec!["c1".to_string()]);
                assert_eq!(source_quotes.len(), 1);
                assert_eq!(answer, "hello");
            }
            _ => panic!("expected answer"),
        }
    }
}

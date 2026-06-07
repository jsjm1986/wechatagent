//! 运营知识库杂项：知识问答检索 + gap 信号治理 + 摄取源管理 + 元数据/用量统计 + 算子记忆。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, Bson, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent;
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};

use super::super::AppState;
use super::*;

pub(in crate::routes) async fn list_knowledge_usage(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let mut cursor = state
        .db
        .knowledge_usage_logs()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": account_id
            },
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(knowledge_usage_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct AnalyzeLogsQuery {
    account_id: Option<String>,
    /// 回看窗口（小时），缺省 24，硬上限 72。
    hours: Option<i64>,
    /// 仅统计被拦截 / 暂缓的 run，缺省 true。
    only_blocked_or_held: Option<bool>,
}

/// `GET /api/operation-knowledge/logs/analyze`
///
/// 只读：按窗口聚合 `knowledge_usage_logs`，输出 `{window_hours, total_runs,
/// blocked_or_held_runs, top_chunks, items}`。语义与 chat tool
/// `knowledge.analyze_logs` 完全一致，前端 / 运营审查时直接 HTTP 取，不用走
/// LLM。
pub(in crate::routes) async fn analyze_operation_knowledge_logs(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AnalyzeLogsQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = admin.current_workspace.clone();
    let hours = query.hours.filter(|v| *v > 0).unwrap_or(24).min(72);
    let only_blocked = query.only_blocked_or_held.unwrap_or(true);
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours);
    let cutoff_bson = DateTime::from_millis(cutoff.timestamp_millis());

    let mut filter = doc! {
        "workspace_id": &workspace_id,
        "created_at": { "$gte": cutoff_bson },
    };
    if let Some(account_id) = query
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
            Bson::Array(vec![
                Bson::Document(doc! { "review_approved": false }),
                Bson::Document(doc! {
                    "blocked_reason": { "$exists": true, "$ne": Bson::Null },
                }),
            ]),
        );
    }

    let mut cursor = state
        .db
        .knowledge_usage_logs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1_i32 })
                .limit(50)
                .build(),
        )
        .await?;

    let mut chunk_freq: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut items: Vec<Value> = Vec::new();
    let mut blocked: i32 = 0;
    while let Some(log) = cursor.try_next().await? {
        if log.blocked_reason.is_some() || !log.review_approved {
            blocked += 1;
        }
        for kid in &log.knowledge_ids {
            *chunk_freq.entry(kid.to_hex()).or_insert(0) += 1;
        }
        items.push(json!({
            "runId": log.run_id,
            "accountId": log.account_id,
            "blockedReason": log.blocked_reason,
            "reviewApproved": log.review_approved,
            "knowledgeIds": log.knowledge_ids.iter().map(|o| o.to_hex()).collect::<Vec<_>>(),
            "createdAt": crate::models::dt_to_string(log.created_at),
        }));
    }

    let total_runs = items.len() as i32;
    let mut top_chunks: Vec<(String, i32)> = chunk_freq.into_iter().collect();
    top_chunks.sort_by(|a, b| b.1.cmp(&a.1));
    let top_chunks_json: Vec<Value> = top_chunks
        .into_iter()
        .take(8)
        .map(|(id, count)| json!({ "chunkId": id, "hitCount": count }))
        .collect();

    Ok(Json(json!({
        "windowHours": hours,
        "onlyBlockedOrHeld": only_blocked,
        "totalRuns": total_runs,
        "blockedOrHeldRuns": blocked,
        "topChunks": top_chunks_json,
        "items": items,
    })))
}

// ── G5 · 元信息聚合：单次 $facet 拉 4 维 ─────────────────────────────
//
// 返回：
//   - wikiTypeCounts:        Vec<{ wikiType, count }>
//   - verifiedRatioByType:   Vec<{ wikiType, total, verified, ratio }>
//   - topEditors:            Vec<{ author, count }>      (top 10)
//   - recentActivity7d:      Vec<{ date, op, count }>     (最近 7 天)
//
// **不写库 / 不修 schema / 不引外部缓存**。一次 aggregate 命中 4 个维度。
pub async fn knowledge_aggregate_metadata(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    use futures::StreamExt;
    let ws = &admin.current_workspace;
    let cutoff = mongodb::bson::DateTime::from_millis(
        (chrono::Utc::now() - chrono::Duration::days(7)).timestamp_millis(),
    );

    // 1) wikiTypeCounts + verifiedRatioByType 在 chunks 上做。
    let chunks_pipe = vec![
        doc! { "$match": { "workspace_id": ws } },
        doc! {
            "$facet": {
                "wikiTypeCounts": [
                    { "$group": {
                        "_id": { "$ifNull": ["$wiki_type", "unknown"] },
                        "count": { "$sum": 1 },
                    } },
                    { "$sort": { "count": -1 } },
                ],
                "verifiedRatio": [
                    { "$group": {
                        "_id": { "$ifNull": ["$wiki_type", "unknown"] },
                        "total": { "$sum": 1 },
                        "verified": { "$sum": {
                            "$cond": [{ "$eq": ["$integrity_status", "verified"] }, 1, 0]
                        } },
                    } },
                    { "$sort": { "_id": 1 } },
                ],
            }
        },
    ];
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .aggregate(chunks_pipe, None)
        .await?;
    let chunks_facet = match cursor.next().await {
        Some(Ok(d)) => d,
        Some(Err(e)) => return Err(AppError::from(e)),
        None => Document::new(),
    };

    let wiki_type_counts: Vec<Value> = chunks_facet
        .get_array("wikiTypeCounts")
        .ok()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_document().cloned())
                .map(|d| {
                    json!({
                        "wikiType": d.get_str("_id").unwrap_or("unknown"),
                        "count": d.get_i32("count").unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let verified_ratio_by_type: Vec<Value> = chunks_facet
        .get_array("verifiedRatio")
        .ok()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_document().cloned())
                .map(|d| {
                    let total = d.get_i32("total").unwrap_or(0);
                    let verified = d.get_i32("verified").unwrap_or(0);
                    let ratio = if total > 0 {
                        verified as f64 / total as f64
                    } else {
                        0.0
                    };
                    json!({
                        "wikiType": d.get_str("_id").unwrap_or("unknown"),
                        "total": total,
                        "verified": verified,
                        "ratio": ratio,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // 2) topEditors + recentActivity7d 在 chunk_revisions 上做。
    // chunk_revisions 没有 workspace_id 字段（绑定 chunk_id），单租户部署下无影响；
    // 多租户场景需要后续 $lookup 关联 chunks 集合，超出本波范围。
    let revisions_pipe = vec![
        doc! {
            "$facet": {
                "topEditors": [
                    { "$match": { "created_by": { "$exists": true, "$ne": null } } },
                    { "$group": {
                        "_id": "$created_by",
                        "count": { "$sum": 1 },
                    } },
                    { "$sort": { "count": -1 } },
                    { "$limit": 10 },
                ],
                "recentActivity": [
                    { "$match": { "created_at": { "$gte": cutoff } } },
                    { "$group": {
                        "_id": {
                            "date": { "$dateToString": { "format": "%Y-%m-%d", "date": "$created_at" } },
                            "op": { "$ifNull": ["$op", "unknown"] },
                        },
                        "count": { "$sum": 1 },
                    } },
                    { "$sort": { "_id.date": 1 } },
                ],
            }
        },
    ];
    let mut rcursor = state
        .db
        .chunk_revisions()
        .aggregate(revisions_pipe, None)
        .await?;
    let rev_facet = match rcursor.next().await {
        Some(Ok(d)) => d,
        Some(Err(e)) => return Err(AppError::from(e)),
        None => Document::new(),
    };

    let top_editors: Vec<Value> = rev_facet
        .get_array("topEditors")
        .ok()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_document().cloned())
                .map(|d| {
                    json!({
                        "author": d.get_str("_id").unwrap_or("unknown"),
                        "count": d.get_i32("count").unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let recent_activity_7d: Vec<Value> = rev_facet
        .get_array("recentActivity")
        .ok()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_document().cloned())
                .map(|d| {
                    let key = d.get_document("_id").cloned().unwrap_or_default();
                    json!({
                        "date": key.get_str("date").unwrap_or(""),
                        "op": key.get_str("op").unwrap_or("unknown"),
                        "count": d.get_i32("count").unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(json!({
        "wikiTypeCounts": wiki_type_counts,
        "verifiedRatioByType": verified_ratio_by_type,
        "topEditors": top_editors,
        "recentActivity7d": recent_activity_7d,
    })))
}

// ── knowledge-wiki Phase F：gap-signal 路由 ───────────────────────────────────

/// 列出 gap signal。默认返回 `pending` 状态；`status` 查询参数可选。
pub(in crate::routes) async fn list_knowledge_gap_signals(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<GapSignalListQuery>,
) -> AppResult<Json<Value>> {
    use futures::TryStreamExt;
    let status = query.status.as_deref().unwrap_or("pending");
    let mut filter = doc! {
        "workspace_id": &admin.current_workspace,
        "status": status,
    };
    if let Some(kind) = query.kind.as_deref() {
        filter.insert("kind", kind);
    }
    let cursor = state
        .db
        .knowledge_gap_signals()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(query.limit.unwrap_or(100))
                .build(),
        )
        .await?;
    let signals: Vec<crate::models::KnowledgeGapSignal> = cursor.try_collect().await?;
    let items: Vec<Value> = signals
        .iter()
        .map(|s| {
            json!({
                "signalId": s.signal_id,
                "kind": s.kind,
                "title": s.title,
                "description": s.description,
                "severity": s.severity,
                "source": s.source,
                "status": s.status,
                "affectedChunkIds": s.affected_chunk_ids,
                "searchQueries": s.search_queries,
                "resolutionNote": s.resolution_note,
                "createdAt": crate::models::dt_to_string(s.created_at).unwrap_or_default(),
                "resolvedAt": s.resolved_at
                    .and_then(|t| crate::models::dt_to_string(t)),
            })
        })
        .collect();
    Ok(Json(json!({ "signals": items })))
}

#[derive(Debug, Deserialize, Default)]
pub struct GapSignalListQuery {
    pub status: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GapSignalResolutionRequest {
    #[serde(default)]
    pub note: Option<String>,
}

/// 手动 dismiss 一条 signal（运营确认本条不需要处理）。
pub(in crate::routes) async fn dismiss_knowledge_gap_signal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(signal_id): Path<String>,
    Json(payload): Json<GapSignalResolutionRequest>,
) -> AppResult<Json<Value>> {
    let note = payload
        .note
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "human:dismissed".into());
    let now = mongodb::bson::DateTime::now();
    let result = state
        .db
        .knowledge_gap_signals()
        .update_one(
            doc! {
                "signal_id": &signal_id,
                "workspace_id": &admin.current_workspace,
                "status": "pending"
            },
            doc! { "$set": {
                "status": "dismissed",
                "resolution_note": note,
                "resolved_at": now,
            }},
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("knowledge_gap_signal".into()));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 标记一条 signal 为 applied（运营已按建议改了 chunk）。
pub(in crate::routes) async fn apply_knowledge_gap_signal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(signal_id): Path<String>,
    Json(payload): Json<GapSignalResolutionRequest>,
) -> AppResult<Json<Value>> {
    let note = payload
        .note
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "human:applied".into());
    let now = mongodb::bson::DateTime::now();
    let result = state
        .db
        .knowledge_gap_signals()
        .update_one(
            doc! {
                "signal_id": &signal_id,
                "workspace_id": &admin.current_workspace,
                "status": "pending"
            },
            doc! { "$set": {
                "status": "applied",
                "resolution_note": note,
                "resolved_at": now,
            }},
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("knowledge_gap_signal".into()));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 手动触发一次 structural lint + stage 1 sweep。
pub(in crate::routes) async fn sweep_knowledge_gap_signals(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    use crate::knowledge_wiki::gap_signals;
    let workspace = &admin.current_workspace;
    let lint = gap_signals::run_structural_lint(&state.db, workspace).await?;
    let sweep = gap_signals::sweep_stale_signals(&state.db, workspace).await?;
    Ok(Json(json!({
        "structuralLint": {
            "newSignals": lint.new_signals,
            "existingPending": lint.existing_pending,
            "stage1AutoResolved": lint.stage1_auto_resolved,
        },
        "sweep": {
            "stage1AutoResolved": sweep.stage1_auto_resolved,
            "stage2LlmResolved": sweep.stage2_llm_resolved,
        }
    })))
}

// ── /api/knowledge/ask: Agent-first 渐进式披露问答入口 ─────────────────
//
// 让前端 AskView 与运营 agent 共享同一条 knowledge_agent 主循环；不走 BM25 / 向量
// 召回，由 LLM 自己 list_catalog → open_chunk → follow_relations → answer。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct KnowledgeAskRequest {
    /// 已废弃：服务端忽略此字段，一律用 session 的 current_workspace（防跨租户读取）。
    #[allow(dead_code)]
    workspace_id: Option<String>,
    account_id: Option<String>,
    query: String,
    /// 1..=3；为 None 时由 knowledge_agent 默认走 3 轮。
    max_rounds: Option<i32>,
    #[serde(default)]
    filter: KnowledgeAskFilter,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct KnowledgeAskFilter {
    #[serde(default)]
    wiki_types: Vec<String>,
    #[serde(default)]
    business_topics: Vec<String>,
    #[serde(default)]
    status: Option<String>,
}

/// `POST /api/knowledge/ask`：调用 [`crate::agent::knowledge_agent::answer`] 主循环。
///
/// 返回 schema：`{ answer, citedChunkIds, sourceQuotes, toolTrace, roundsUsed,
/// truncated, tookMs }`。`tookMs` 为后端测得的端到端耗时（含 LLM 与 mongo I/O）。
pub(in crate::routes) async fn ask_knowledge(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(req): Json<KnowledgeAskRequest>,
) -> AppResult<Json<Value>> {
    let started_at = std::time::Instant::now();
    if req.query.trim().is_empty() {
        return Err(AppError::BadRequest("query 不能为空".into()));
    }
    // 多租户隔离：一律用 session 注入的 current_workspace，忽略 body 里 client 传的
    // workspaceId（AuthenticatedAdmin 不携带可访问 workspace 列表，无法做 ACL 校验，
    // 信任 client 值会导致跨租户读取）。切换 workspace 走 POST /api/auth/workspace。
    let workspace_id = admin.current_workspace.clone();
    let account_id = req
        .account_id
        .clone()
        .filter(|s| !s.trim().is_empty());
    let agent_req = agent::knowledge_agent::AnswerRequest {
        workspace_id,
        account_id,
        query: req.query.clone(),
        filter: agent::knowledge_agent::CatalogFilter {
            wiki_types: req.filter.wiki_types,
            business_topics: req.filter.business_topics,
            status: req.filter.status,
            // /api/knowledge/ask 是用户/agent 主入口，沿用 router 路径的 verified-only
            // 语义：未审核 chunk 不上 prompt（[`CatalogFilter::include_unverified`]）。
            include_unverified: false,
        },
        max_rounds: req.max_rounds,
    };
    let result = agent::knowledge_agent::answer(&state, agent_req).await?;
    let took_ms = started_at.elapsed().as_millis() as u64;
    // tool_trace 是 Vec<bson::Document>；直接 serde_json 序列化会暴露 BSON Extended
    // JSON（如 `{"$numberInt":"3"}`），前端时间线需要纯 JSON，故走
    // `.into_relaxed_extjson()` 桥接（与 src/agent/tool_loop.rs:359 / chat_tool_loop.rs:316
    // / knowledge_tools.rs:1252 / routes/domain_schemas.rs:150 一致）。
    let tool_trace_json: Vec<Value> = result
        .tool_trace
        .into_iter()
        .map(|d| mongodb::bson::Bson::Document(d).into_relaxed_extjson())
        .collect();
    Ok(Json(json!({
        "answer": result.answer,
        "citedChunkIds": result.cited_chunk_ids,
        "sourceQuotes": result.source_quotes.iter().map(|q| json!({
            "chunkId": q.chunk_id,
            "quote": q.quote,
            "sourceAnchorIndex": q.source_anchor_index,
        })).collect::<Vec<_>>(),
        "toolTrace": tool_trace_json,
        "roundsUsed": result.rounds_used,
        "truncated": result.truncated,
        "tookMs": took_ms,
    })))
}

// ── /api/knowledge/ask/stream: SSE 流式版 /api/knowledge/ask ──────────
//
// 浏览器 EventSource 仅支持 GET，所以参数走 query string；filter 用逗号分隔字符串。
// 每个 tool_trace 步同步推 `event:trace`，跑完推 `event:answer`，最后 `event:close`。
// 与 chat_session_stream:5562 同模式（`futures::stream::unfold` 包 receiver、零新依赖）。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct KnowledgeAskStreamQuery {
    query: String,
    /// 已废弃：服务端忽略此字段，一律用 session 的 current_workspace（防跨租户读取）。
    #[allow(dead_code)]
    workspace_id: Option<String>,
    account_id: Option<String>,
    max_rounds: Option<i32>,
    /// 逗号分隔，例如 `wikiTypes=methodology,thesis`。
    wiki_types: Option<String>,
    /// 同上：`businessTopics=价格异议,客户分级`。
    business_topics: Option<String>,
    status: Option<String>,
}

fn split_csv(raw: Option<&str>) -> Vec<String> {
    raw.map(|s| {
        s.split(',')
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// `GET /api/knowledge/ask/stream`：SSE 推送 [`agent::knowledge_agent::answer_streaming`]
/// 的实时事件。事件类型：
///   - `trace` —— 每一步工具调用（与 `tool_trace` 一一对应，纯 JSON）
///   - `answer` —— 终态 `AnswerResult`（同 `/api/knowledge/ask` JSON 形态）
///   - `close` —— 流结束信号；前端收到后应主动 `es.close()` 不再重连
pub(in crate::routes) async fn ask_knowledge_stream(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(req): Query<KnowledgeAskStreamQuery>,
) -> AppResult<
    axum::response::Sse<
        impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};

    if req.query.trim().is_empty() {
        return Err(AppError::BadRequest("query 不能为空".into()));
    }
    // 多租户隔离：一律用 session 注入的 current_workspace，忽略 body 里 client 传的
    // workspaceId（AuthenticatedAdmin 不携带可访问 workspace 列表，无法做 ACL 校验，
    // 信任 client 值会导致跨租户读取）。切换 workspace 走 POST /api/auth/workspace。
    let workspace_id = admin.current_workspace.clone();
    let account_id = req
        .account_id
        .clone()
        .filter(|s| !s.trim().is_empty());
    let agent_req = agent::knowledge_agent::AnswerRequest {
        workspace_id,
        account_id,
        query: req.query.clone(),
        filter: agent::knowledge_agent::CatalogFilter {
            wiki_types: split_csv(req.wiki_types.as_deref()),
            business_topics: split_csv(req.business_topics.as_deref()),
            status: req.status,
            include_unverified: false,
        },
        max_rounds: req.max_rounds,
    };

    // tx/rx 跨任务推 TraceEvent；tx 在 spawn 任务里 drop，rx 端走完就发 close。
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<agent::knowledge_agent::TraceEvent>();
    // 取消句柄：客户端断开 → unfold state drop → CancelOnDrop::drop 翻 true →
    // spawn 任务在下次轮询前检测到 → 兜底返回 cancelled=true。
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_for_agent = cancel.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        // answer_streaming 末尾会发 TraceEvent::Final；error 路径只能 drop tx，
        // 所以这里把 Err 转成一条 error 事件再发出去再退出。
        if let Err(err) = agent::knowledge_agent::answer_streaming(
            &state_clone,
            agent_req,
            tx.clone(),
            Some(cancel_for_agent),
        )
        .await
        {
            let _ = tx.send(agent::knowledge_agent::TraceEvent::Step {
                payload: json!({
                    "tool": "error",
                    "reason": format!("agent_error:{err}"),
                }),
            });
        }
        // tx 在此 drop（仅剩 spawn 任务持有；drop 后 rx.recv 会拿到 None）。
    });

    /// `unfold` 的 state 类型；Drop 时翻 cancel。axum 在 client 断开时 drop body
    /// 流，body 流的 state 跟着 drop → 这里顺手把取消标志位 set 住。spawn 任务
    /// 看到后会主动早退出。
    struct CancelOnDrop {
        rx: tokio::sync::mpsc::UnboundedReceiver<agent::knowledge_agent::TraceEvent>,
        closed: bool,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            self.cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let initial = CancelOnDrop {
        rx,
        closed: false,
        cancel: cancel.clone(),
    };
    let stream = futures::stream::unfold(initial, |mut st| async move {
        if st.closed {
            return None;
        }
        match st.rx.recv().await {
            Some(agent::knowledge_agent::TraceEvent::Step { payload }) => {
                let data = payload.to_string();
                Some((
                    Ok::<_, std::convert::Infallible>(Event::default().event("trace").data(data)),
                    st,
                ))
            }
            Some(agent::knowledge_agent::TraceEvent::Token { delta }) => {
                let data = json!({ "delta": delta }).to_string();
                Some((
                    Ok::<_, std::convert::Infallible>(Event::default().event("token").data(data)),
                    st,
                ))
            }
            Some(agent::knowledge_agent::TraceEvent::Final { answer }) => {
                // 与 /api/knowledge/ask 的 JSON 形态对齐：tool_trace 走 relaxed extjson。
                let tool_trace_json: Vec<Value> = answer
                    .tool_trace
                    .iter()
                    .cloned()
                    .map(|d| mongodb::bson::Bson::Document(d).into_relaxed_extjson())
                    .collect();
                let payload = json!({
                    "answer": answer.answer,
                    "citedChunkIds": answer.cited_chunk_ids,
                    "sourceQuotes": answer.source_quotes.iter().map(|q| json!({
                        "chunkId": q.chunk_id,
                        "quote": q.quote,
                        "sourceAnchorIndex": q.source_anchor_index,
                    })).collect::<Vec<_>>(),
                    "toolTrace": tool_trace_json,
                    "roundsUsed": answer.rounds_used,
                    "truncated": answer.truncated,
                    "cancelled": answer.cancelled,
                });
                Some((Ok(Event::default().event("answer").data(payload.to_string())), st))
            }
            None => {
                st.closed = true;
                Some((Ok(Event::default().event("close").data("done")), st))
            }
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Phase E / E5：knowledge agent 进程级指标。
///
/// 当前只透出 [`agent::knowledge_agent::cache_stats`]（answer cache 命中率 + TTL 配置）。
/// 后续可在此聚合 budget 用尽次数 / cancel 比率等。返回 200 + JSON。
pub(in crate::routes) async fn knowledge_metrics(
    State(_state): State<AppState>,
) -> AppResult<axum::Json<serde_json::Value>> {
    let cache = agent::knowledge_agent::cache_stats();
    Ok(axum::Json(serde_json::json!({
        "answerCache": cache,
    })))
}

/// `GET /api/knowledge/operator-memory`：列出运营长期偏好记忆。
///
/// Phase F：Atlas 视图需要展示运营自己写过的偏好/拒绝/上下文记忆，
/// 以便核对哪些会被注入到 reply prompt。**只读**，不 bump `last_used_at`
/// （bump 仅在真正被 reply Agent 复用时发生，UI 浏览不算复用）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct OperatorMemoryQuery {
    pub account_id: Option<String>,
    pub operator_id: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<i64>,
}

pub(in crate::routes) async fn list_operator_memory(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<OperatorMemoryQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = admin.current_workspace.clone();
    let account_id = query
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let operator_id = query
        .operator_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let now = DateTime::now();
    let mut filter = doc! {
        "workspace_id": &workspace_id,
        "account_id": &account_id,
        "operator_id": &operator_id,
        "$or": [
            { "expires_at": { "$exists": false } },
            { "expires_at": null },
            { "expires_at": { "$gt": now } },
        ],
    };
    if let Some(kind) = query.kind.as_deref() {
        let kind_trim = kind.trim();
        if !kind_trim.is_empty() {
            if !["preference", "rejection", "context"].contains(&kind_trim) {
                return Err(AppError::BadRequest(format!(
                    "kind 非法：{kind_trim}（必须在 [preference, rejection, context]）"
                )));
            }
            filter.insert("kind", kind_trim);
        }
    }

    let opts = FindOptions::builder()
        .sort(doc! { "last_used_at": -1_i32 })
        .limit(limit)
        .build();

    let mut cursor = state
        .db
        .knowledge_operator_memory()
        .find(filter, opts)
        .await
        .map_err(|e| AppError::External(format!("查询运营记忆失败：{e}")))?;

    let mut items: Vec<Value> = Vec::new();
    while let Some(m) = cursor
        .try_next()
        .await
        .map_err(|e| AppError::External(format!("迭代运营记忆失败：{e}")))?
    {
        items.push(json!({
            "id": m.id.map(|i| i.to_hex()),
            "workspaceId": m.workspace_id,
            "accountId": m.account_id,
            "operatorId": m.operator_id,
            "kind": m.kind,
            "content": m.content,
            "createdAt": m.created_at.try_to_rfc3339_string().ok(),
            "lastUsedAt": m.last_used_at.try_to_rfc3339_string().ok(),
            "expiresAt": m.expires_at.and_then(|d| d.try_to_rfc3339_string().ok()),
        }));
    }

    Ok(Json(json!({
        "workspaceId": workspace_id,
        "accountId": account_id,
        "operatorId": operator_id,
        "items": items,
    })))
}

// ── Phase G P1-6：ingest sources CRUD ────────────────────────────────────
//
// 写路径只接受 status="active"；failing/disabled 是 worker 自行迁移的，admin
// 通过此接口可重置（active），但不能直接写 failing/disabled（违反闭集语义）。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestSourceCreateRequest {
    pub kind: String,
    pub url: String,
    pub schedule_minutes: i64,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestSourceUpdateRequest {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub schedule_minutes: Option<i64>,
    #[serde(default)]
    pub label: Option<String>,
    /// 仅允许写 "active"——把 failing 重置回 active；其他值 400。
    #[serde(default)]
    pub status: Option<String>,
}

pub async fn list_ingest_sources(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let workspace_id = admin.current_workspace.clone();
    let mut cursor = state
        .db
        .ingest_sources()
        .find(doc! { "workspace_id": &workspace_id }, None)
        .await
        .map_err(AppError::from)?;
    let mut items: Vec<Value> = Vec::new();
    while let Some(src) = cursor.try_next().await.map_err(AppError::from)? {
        items.push(serde_json::to_value(src).map_err(|e| AppError::External(e.to_string()))?);
    }
    Ok(Json(json!({ "workspaceId": workspace_id, "items": items })))
}

pub async fn create_ingest_source(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<IngestSourceCreateRequest>,
) -> AppResult<Json<Value>> {
    if !matches!(payload.kind.as_str(), "rss" | "html") {
        return Err(AppError::BadRequest(
            "kind must be 'rss' or 'html'".to_string(),
        ));
    }
    if payload.url.trim().is_empty() {
        return Err(AppError::BadRequest("url required".to_string()));
    }
    if payload.schedule_minutes < 1 {
        return Err(AppError::BadRequest(
            "schedule_minutes must be >= 1".to_string(),
        ));
    }
    let now = DateTime::now();
    let source_id = format!("ing_{}", ObjectId::new().to_hex());
    let row = crate::models::IngestSource {
        id: None,
        source_id: source_id.clone(),
        workspace_id: admin.current_workspace.clone(),
        kind: payload.kind,
        url: payload.url,
        schedule_minutes: payload.schedule_minutes,
        label: payload.label,
        last_fetched_at: None,
        last_etag: None,
        last_error: None,
        status: "active".to_string(),
        failure_streak: 0,
        ingest_count: 0,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .ingest_sources()
        .insert_one(&row, None)
        .await
        .map_err(AppError::from)?;
    Ok(Json(json!({ "sourceId": source_id })))
}

pub async fn update_ingest_source(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(source_id): Path<String>,
    Json(payload): Json<IngestSourceUpdateRequest>,
) -> AppResult<Json<Value>> {
    let mut set_doc = doc! { "updated_at": DateTime::now() };
    if let Some(url) = payload.url {
        if url.trim().is_empty() {
            return Err(AppError::BadRequest("url cannot be empty".to_string()));
        }
        set_doc.insert("url", url);
    }
    if let Some(m) = payload.schedule_minutes {
        if m < 1 {
            return Err(AppError::BadRequest(
                "schedule_minutes must be >= 1".to_string(),
            ));
        }
        set_doc.insert("schedule_minutes", m);
    }
    if let Some(label) = payload.label {
        set_doc.insert("label", label);
    }
    if let Some(s) = payload.status {
        if s != "active" {
            return Err(AppError::BadRequest(
                "status only accepts 'active' (failing/disabled is worker-managed)".to_string(),
            ));
        }
        set_doc.insert("status", "active");
        set_doc.insert("failure_streak", 0);
        set_doc.insert("last_error", Bson::Null);
    }
    let result = state
        .db
        .ingest_sources()
        .update_one(
            doc! {
                "source_id": &source_id,
                "workspace_id": &admin.current_workspace,
            },
            doc! { "$set": set_doc },
            None,
        )
        .await
        .map_err(AppError::from)?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("ingest source not found".to_string()));
    }
    Ok(Json(json!({ "sourceId": source_id, "updated": true })))
}

pub async fn delete_ingest_source(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(source_id): Path<String>,
) -> AppResult<Json<Value>> {
    let result = state
        .db
        .ingest_sources()
        .delete_one(
            doc! {
                "source_id": &source_id,
                "workspace_id": &admin.current_workspace,
            },
            None,
        )
        .await
        .map_err(AppError::from)?;
    if result.deleted_count == 0 {
        return Err(AppError::NotFound("ingest source not found".to_string()));
    }
    Ok(Json(json!({ "sourceId": source_id, "deleted": true })))
}

//! 知识库日报工作站 worker 入口（knowledge-digest-workstation）。
//!
//! 设计见 `.kiro/specs/knowledge-digest-workstation/{requirements,design,tasks}.md`
//! 与 `docs/agent-policy.md` 知识库日报工作站章节。
//!
//! **隔离红线**：本模块严禁引用 `crate::agent::gateway / outbox`、
//! `crate::mcp::*`、`agent_send_outbox` 写入路径或 `run_user_operation_gateway`
//! 等生产链路入口。日报合成是离线分析任务，与对话发送链路彻底隔离。
//!
//! `worker_loop` 在 `KNOWLEDGE_DIGEST_ENABLED=false` 时 early-return；启用时按
//! `KNOWLEDGE_DIGEST_RUN_HOUR` 整点跑 `generate_today_digest` 合成当日日报。
//! 路由 `GET /api/knowledge/digest/today` 未命中时按需同步合成（见 routes/knowledge/digest_inbox.rs）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, NaiveTime, TimeZone};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use serde_json::{json, Value};
use tokio::time::sleep;

use crate::agent::{generate_agent_json, RunBudget, RUN_BUDGET};
use crate::error::{AppError, AppResult};
use crate::models::{KnowledgeDailyReport, KnowledgeDigestCard, KnowledgeUsageLog};
use crate::prompts::load_prompt;
use crate::routes::AppState;

mod labels;

/// Stable content identity for one card inside a visible digest snapshot.
/// The hash covers every field that can affect what an operator is approving;
/// callers must not treat the ObjectId alone as a version token.
pub(crate) fn digest_card_snapshot_hash(card: &KnowledgeDigestCard) -> String {
    use sha2::{Digest, Sha256};

    let canonical = json!({
        "cardId": card.card_id.to_hex(),
        "kind": card.kind,
        "title": card.title,
        "summary": card.summary,
        "targetRefs": card.target_refs,
        "suggestedAction": card.suggested_action,
        "severity": card.severity,
        "metric": card.metric,
    });
    let bytes = serde_json::to_vec(&canonical).expect("digest card canonical JSON is serializable");
    hex::encode(Sha256::digest(bytes))
}

/// Stable identity of the currently visible digest snapshot. Attempt audit
/// fields are intentionally excluded: a failed regeneration may update those
/// while the last successful cards and `current_generation` remain current.
pub(crate) fn digest_report_snapshot_hash(report: &KnowledgeDailyReport) -> String {
    use sha2::{Digest, Sha256};

    let card_hashes = report
        .cards
        .iter()
        .map(digest_card_snapshot_hash)
        .collect::<Vec<_>>();
    let canonical = json!({
        "reportId": report.id.map(|id| id.to_hex()),
        "workspaceId": report.workspace_id,
        "accountId": report.account_id,
        "reportDate": report.report_date,
        "currentGeneration": report.current_generation,
        "cardHashes": card_hashes,
        "dismissedCardIds": report
            .dismissed_card_ids
            .iter()
            .map(|id| id.to_hex())
            .collect::<Vec<_>>(),
    });
    let bytes =
        serde_json::to_vec(&canonical).expect("digest report canonical JSON is serializable");
    hex::encode(Sha256::digest(bytes))
}

/// 主循环：`KNOWLEDGE_DIGEST_ENABLED=false` 时立即 return，等价于功能未启用。
///
/// 启用时按 `KNOWLEDGE_DIGEST_RUN_HOUR`（运营时区，默认 9）计算到下一次本地
/// 时间该小时整点的 sleep 时长，醒来跑一次 [`generate_today_digest`]，再 sleep
/// 到次日同一时刻。日内手动重算走 `POST /api/knowledge/digest/regenerate`，
/// 不依赖此 loop。
pub async fn worker_loop(state: AppState) {
    if !state.config.knowledge_digest_enabled {
        tracing::info!(
            "knowledge digest worker disabled (KNOWLEDGE_DIGEST_ENABLED=false); skip spawn"
        );
        return;
    }
    let run_hour = state.config.knowledge_digest_run_hour.min(23);
    tracing::info!(run_hour, "knowledge digest worker starting");
    loop {
        let wait = duration_until_next_run(run_hour);
        tracing::debug!(?wait, "knowledge digest worker sleeping until next run");
        sleep(wait).await;
        if let Err(err) = generate_all_account_digests(&state).await {
            tracing::warn!(
                ?err,
                "knowledge digest account enumeration failed; continuing"
            );
        }
    }
}

/// Run one scheduled digest for every persisted account scope. A failure in one account is
/// isolated and does not prevent later accounts from receiving their report.
async fn generate_all_account_digests(state: &AppState) -> AppResult<usize> {
    let mut cursor = state.db.accounts().find(doc! {}, None).await?;
    let mut attempted = 0usize;
    while let Some(account) = cursor.try_next().await? {
        attempted += 1;
        if let Err(err) =
            generate_today_digest(state, &account.workspace_id, &account.account_id).await
        {
            tracing::warn!(
                workspace_id = %account.workspace_id,
                account_id = %account.account_id,
                ?err,
                "knowledge digest account tick failed; continuing"
            );
        }
    }
    Ok(attempted)
}

/// Narrow production-protocol harness for the scheduled account enumeration.
/// It executes the same helper called by [`worker_loop`] without sleeping until
/// the configured wall-clock hour.
#[doc(hidden)]
pub async fn generate_all_account_digests_for_redline(state: &AppState) -> AppResult<usize> {
    generate_all_account_digests(state).await
}

/// 计算从现在到下一次 `run_hour:00` 的本地时间间隔。今天 `run_hour` 还没到则等到今天，
/// 否则等到次日。
fn duration_until_next_run(run_hour: u32) -> Duration {
    let now = Local::now();
    let target_today = Local
        .from_local_datetime(
            &now.date_naive()
                .and_time(NaiveTime::from_hms_opt(run_hour, 0, 0).unwrap_or_default()),
        )
        .single();
    let target = match target_today {
        Some(t) if t > now => t,
        _ => {
            // 今天已过 → 次日
            let next_day = now.date_naive().succ_opt().unwrap_or(now.date_naive());
            Local
                .from_local_datetime(
                    &next_day.and_time(NaiveTime::from_hms_opt(run_hour, 0, 0).unwrap_or_default()),
                )
                .single()
                .unwrap_or(now + chrono::Duration::hours(24))
        }
    };
    let delta = (target - now).to_std().unwrap_or(Duration::from_secs(60));
    // 至少 sleep 60s，避免边界条件死循环（now == target 时 delta=0）。
    if delta < Duration::from_secs(60) {
        Duration::from_secs(60)
    } else {
        delta
    }
}

// ── Phase 2：4 路只读分析 + LLM 合成 + upsert ──────────────────────────────
//
// 设计准则（与 mod.rs 顶部「隔离红线」配套）：
// 1. 全部分析函数**只读**，不写 `operation_knowledge_*` / `agent_run_logs` /
//    `agent_send_outbox` / `proposals`，更不调 MCP；
// 2. 每个分析函数返回**结构化中间信号**，由 [`compose_cards`] 喂给
//    `knowledge.digest.compose` LLM 合成最终 `KnowledgeDigestCard[]`；
// 3. LLM 调用统一走 [`crate::agent::generate_agent_json`]，挂 `RUN_BUDGET`
//    task-local（24000 token / 8 LLM calls / tool=i32::MAX 因为不走 tool-loop）。

#[derive(Debug, Clone)]
struct ChunkHealthSignal {
    chunk_id: String,
    title: String,
    missing_fields: Vec<String>,
    status: String,
    age_days: i64,
}

#[derive(Debug, Clone, Default)]
struct UsageDigest {
    total: i64,
    hits: i64,
    misses: i64,
    /// `chunk_id -> (used_count, blocked_count)`
    per_chunk: HashMap<String, (i64, i64)>,
    /// 落空 query 的 reply_text 摘要（前 5 条），用作 LLM 输入。
    top_miss_samples: Vec<String>,
}

#[derive(Debug, Clone)]
struct BlockSignal {
    chunk_id: String,
    block_count: i64,
    top_block_reason: String,
    summary: String,
    sample_run_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct EvolutionSignal {
    proposal_id: String,
    status: String,
    proposal_kind: String,
    summary: String,
}

/// `(workspace_id, account_id)` 范围过滤的复用 helper。
fn ws_filter(workspace_id: &str, account_id: &str) -> Document {
    doc! { "workspace_id": workspace_id, "account_id": account_id }
}

/// 24h 时间窗口下界（BSON DateTime）。
fn since_24h() -> BsonDateTime {
    let now = chrono::Utc::now();
    let lower = now - chrono::Duration::hours(24);
    BsonDateTime::from_millis(lower.timestamp_millis())
}

/// **只读**扫描当前账号可见的 `operation_knowledge_chunks`：账号私有行加
/// `account_id=null` 的 workspace 共享行，与生产召回/Catalog 可见域一致。
/// 1. `integrity_status ∈ {needs_review, missing_evidence}` 或非空 `missing_fields`；
/// 2. `status="draft"` 且 `created_at` ≥ 7 天。
async fn analyze_chunks_health(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Vec<ChunkHealthSignal>> {
    let now = chrono::Utc::now();
    let seven_days_ago = now - chrono::Duration::days(7);
    let filter = doc! {
        "workspace_id": workspace_id,
        "$and": [
            {
                "$or": [
                    { "account_id": null },
                    { "account_id": account_id },
                ]
            },
            {
                "$or": [
                    { "integrity_status": { "$in": ["needs_review", "missing_evidence"] } },
                    { "source_quote": { "$in": [null, ""] } },
                    {
                        "status": "draft",
                        "created_at": { "$lte": BsonDateTime::from_millis(seven_days_ago.timestamp_millis()) }
                    },
                ]
            },
        ]
    };
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(filter, None)
        .await?;
    let mut out: Vec<ChunkHealthSignal> = Vec::new();
    while let Some(chunk) = cursor.try_next().await? {
        let chunk_id = chunk.id.map(|oid| oid.to_hex()).unwrap_or_default();
        if chunk_id.is_empty() {
            continue;
        }
        let mut missing_fields: Vec<String> = Vec::new();
        if chunk
            .source_quote
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            missing_fields.push("sourceQuote".to_string());
        }
        if chunk
            .integrity_status
            .as_deref()
            .map(|s| s == "needs_review" || s == "missing_evidence")
            .unwrap_or(false)
        {
            missing_fields.push("integrityStatus".to_string());
        }
        // 跳过：status=active 且 missing_fields 为空 且 age < 7 天的 chunk。
        let created_ms = chunk.created_at.timestamp_millis();
        let age_days = ((now.timestamp_millis() - created_ms) / 86_400_000).max(0);
        if missing_fields.is_empty() && chunk.status != "draft" {
            continue;
        }
        out.push(ChunkHealthSignal {
            chunk_id,
            title: chunk.title.clone(),
            missing_fields,
            status: chunk.status.clone(),
            age_days,
        });
        if out.len() >= 200 {
            break; // 防御：单日最多 200 条 health signal 喂给 LLM
        }
    }
    Ok(out)
}

/// **只读**聚合 `knowledge_usage_logs` 24h：命中率 + per-chunk 频次 + 落空样本。
async fn analyze_usage_logs(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<UsageDigest> {
    let mut filter = ws_filter(workspace_id, account_id);
    filter.insert("created_at", doc! { "$gte": since_24h() });
    let mut cursor = state.db.knowledge_usage_logs().find(filter, None).await?;
    let mut digest = UsageDigest::default();
    while let Some(log) = cursor.try_next().await? {
        digest.total += 1;
        if log.review_approved && log.blocked_reason.is_none() {
            digest.hits += 1;
            for kid in log.knowledge_ids.iter() {
                let entry = digest
                    .per_chunk
                    .entry(kid.to_hex())
                    .or_insert((0_i64, 0_i64));
                entry.0 += 1;
            }
        } else {
            digest.misses += 1;
            if let Some(text) = log.reply_text.as_ref() {
                if digest.top_miss_samples.len() < 5 && !text.trim().is_empty() {
                    let mut snippet: String = text.chars().take(60).collect();
                    if text.chars().count() > 60 {
                        snippet.push('…');
                    }
                    digest.top_miss_samples.push(snippet);
                }
            }
            for kid in log.knowledge_ids.iter() {
                let entry = digest
                    .per_chunk
                    .entry(kid.to_hex())
                    .or_insert((0_i64, 0_i64));
                entry.1 += 1;
            }
        }
    }
    Ok(digest)
}

/// **只读**扫描 `agent_run_logs.final_review_status` 命中 4 个 block 状态值的 run，
/// 反查 `knowledge_route.selectedChunkIds`（camelCase BSON），按 chunk_id 分桶后
/// 调 `knowledge.digest.summarize_logs` 生成单句摘要。
async fn analyze_run_logs(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    run_id: &str,
) -> AppResult<Vec<BlockSignal>> {
    let block_states = vec![
        "blocked_by_required_field",
        "blocked_by_budget",
        "blocked_unverified_product_claim",
        "blocked_by_safety_guard",
    ];
    let mut filter = ws_filter(workspace_id, account_id);
    filter.insert("final_review_status", doc! { "$in": &block_states });
    filter.insert("created_at", doc! { "$gte": since_24h() });
    let mut cursor = state.db.agent_run_logs().find(filter, None).await?;

    /// per-chunk 累计 run id + 拦截原因。
    #[derive(Default)]
    struct Bucket {
        run_ids: Vec<String>,
        block_reasons: HashMap<String, i64>,
    }
    let mut buckets: HashMap<String, Bucket> = HashMap::new();

    while let Some(log) = cursor.try_next().await? {
        let block_reason = log.final_review_status.clone();
        let chunk_ids = log
            .knowledge_route
            .get_array("selectedChunkIds")
            .ok()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if chunk_ids.is_empty() {
            continue;
        }
        for cid in chunk_ids {
            let bucket = buckets.entry(cid).or_default();
            if bucket.run_ids.len() < 8 {
                bucket.run_ids.push(log.run_id.clone());
            }
            *bucket
                .block_reasons
                .entry(block_reason.clone())
                .or_insert(0) += 1;
        }
    }
    if buckets.is_empty() {
        return Ok(Vec::new());
    }

    // 限制最多 LLM call 次数：单 tick 至多 6 个 chunk 走 summarize_logs（其余直接给
    // fallback 文案）。
    let mut bucket_vec: Vec<(String, Bucket)> = buckets.into_iter().collect();
    bucket_vec.sort_by(|a, b| {
        b.1.run_ids
            .len()
            .cmp(&a.1.run_ids.len())
            .then_with(|| a.0.cmp(&b.0))
    });
    let mut out: Vec<BlockSignal> = Vec::new();
    for (idx, (chunk_id, bucket)) in bucket_vec.into_iter().enumerate() {
        let block_count = bucket.run_ids.len() as i64;
        let top_block_reason = bucket
            .block_reasons
            .iter()
            .max_by_key(|(_, c)| **c)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let summary = if idx < 6 {
            // 前 6 大 chunk 走 LLM summarize；超出走 fallback。
            match summarize_block_runs(
                state,
                workspace_id,
                account_id,
                run_id,
                &chunk_id,
                &bucket.run_ids,
                &top_block_reason,
            )
            .await
            {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!(?err, chunk_id = %chunk_id, "summarize_logs failed; using fallback");
                    format!(
                        "AI 观察：该切片在 {} 条 run 上被{}拦截",
                        block_count,
                        labels::block_reason_zh(&top_block_reason)
                    )
                }
            }
        } else {
            format!(
                "AI 观察：该切片在 {} 条 run 上被{}拦截",
                block_count,
                labels::block_reason_zh(&top_block_reason)
            )
        };
        out.push(BlockSignal {
            chunk_id,
            block_count,
            top_block_reason,
            summary,
            sample_run_ids: bucket.run_ids.into_iter().take(3).collect(),
        });
    }
    Ok(out)
}

async fn summarize_block_runs(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    run_id: &str,
    chunk_id: &str,
    run_ids: &[String],
    top_block_reason: &str,
) -> AppResult<String> {
    let system = load_prompt(&state.db, workspace_id, "knowledge.digest.summarize_logs").await?;
    let user = json!({
        "chunkId": chunk_id,
        "runs": run_ids.iter().take(8).map(|r| json!({
            "runId": r,
            "finalReviewStatus": top_block_reason,
            "blockReason": top_block_reason,
            "contactSummary": "(已脱敏)",
            "draftReplyHead": "(已脱敏)"
        })).collect::<Vec<_>>(),
    })
    .to_string();
    let value = generate_agent_json(
        state,
        workspace_id,
        Some(account_id),
        None,
        Some(run_id),
        "knowledge.digest.summarize_logs",
        &system,
        &user,
    )
    .await?;
    let summary = value
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if summary.is_empty() {
        return Err(AppError::LlmUnavailable {
            kind: "empty_summary".to_string(),
            retry_count: 0,
            detail: "summarize_logs 返回空 summary".to_string(),
            hint: "稍后重试或检查 prompt 版本".to_string(),
        });
    }
    Ok(summary)
}

/// **只读**扫 `proposals` 24h 内 `eligible_for_release | rolled_back`。
async fn analyze_evolution(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Vec<EvolutionSignal>> {
    let mut filter = ws_filter(workspace_id, account_id);
    filter.insert(
        "status",
        doc! { "$in": ["eligible_for_release", "rolled_back"] },
    );
    filter.insert(
        "$or",
        mongodb::bson::Bson::Array(vec![
            mongodb::bson::Bson::Document(doc! {
                "released_at": { "$gte": since_24h() }
            }),
            mongodb::bson::Bson::Document(doc! {
                "rolled_back_at": { "$gte": since_24h() }
            }),
            mongodb::bson::Bson::Document(doc! {
                "status": "eligible_for_release"
            }),
        ]),
    );
    let mut cursor = state.db.proposals().find(filter, None).await?;
    let mut out: Vec<EvolutionSignal> = Vec::new();
    while let Some(p) = cursor.try_next().await? {
        let proposal_id = p.id.map(|o| o.to_hex()).unwrap_or_default();
        if proposal_id.is_empty() {
            continue;
        }
        let summary = match p.status.as_str() {
            "eligible_for_release" => format!(
                "AI 建议复核：演化提案 {} 已通过评测，等待运营确认发布",
                p.proposal_kind
            ),
            "rolled_back" => format!("AI 已回滚：演化提案 {} 在发布后指标退化", p.proposal_kind),
            other => format!("AI 演化状态：{}", other),
        };
        out.push(EvolutionSignal {
            proposal_id,
            status: p.status.clone(),
            proposal_kind: p.proposal_kind.clone(),
            summary,
        });
        if out.len() >= 50 {
            break;
        }
    }
    Ok(out)
}

/// 调 `knowledge.digest.compose` LLM 合成卡片数组；返回经过封闭枚举校验后的 `Vec<KnowledgeDigestCard>`。
async fn compose_cards(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    run_id: &str,
    report_date: &str,
    chunk_health: &[ChunkHealthSignal],
    usage: &UsageDigest,
    blocked: &[BlockSignal],
    evolution: &[EvolutionSignal],
) -> AppResult<Vec<KnowledgeDigestCard>> {
    let system = load_prompt(&state.db, workspace_id, "knowledge.digest.compose").await?;

    let chunk_health_json: Vec<Value> = chunk_health
        .iter()
        .take(80)
        .map(|c| {
            json!({
                "chunkId": c.chunk_id,
                "title": c.title,
                "missingFields": c.missing_fields,
                "status": c.status,
                "ageDays": c.age_days,
            })
        })
        .collect();

    let low_hit_rate_chunk_ids: Vec<String> = usage
        .per_chunk
        .iter()
        .filter(|(_, (used, blocked))| *used + *blocked >= 3 && *blocked * 2 > *used)
        .map(|(k, _)| k.clone())
        .collect();

    let usage_json = json!({
        "total": usage.total,
        "hits": usage.hits,
        "misses": usage.misses,
        "hitRate": if usage.total > 0 { (usage.hits as f64) / (usage.total as f64) } else { 0.0 },
        "lowHitRateChunkIds": low_hit_rate_chunk_ids,
        "topMissSamples": usage.top_miss_samples,
    });

    let blocked_json: Vec<Value> = blocked
        .iter()
        .map(|b| {
            json!({
                "chunkId": b.chunk_id,
                "blockReason": b.top_block_reason,
                "count": b.block_count,
                "sampleSummary": b.summary,
                "sampleRunIds": b.sample_run_ids,
            })
        })
        .collect();

    let evolution_json: Vec<Value> = evolution
        .iter()
        .map(|e| {
            json!({
                "proposalId": e.proposal_id,
                "status": e.status,
                "kind": e.proposal_kind,
                "summary": e.summary,
            })
        })
        .collect();

    let user = json!({
        "chunkHealth": chunk_health_json,
        "usageDigest": usage_json,
        "blockedRuns": blocked_json,
        "evolutionDigest": evolution_json,
    })
    .to_string();

    let value = generate_agent_json(
        state,
        workspace_id,
        Some(account_id),
        None,
        Some(run_id),
        "knowledge.digest.compose",
        &system,
        &user,
    )
    .await?;

    let raw_arr = digest_card_items(&value);

    Ok(parse_cards_from_llm_array(raw_arr, account_id, report_date))
}

/// Normalize the two documented Digest response envelopes plus the singleton-card shape
/// produced by the shared LLM parser.
///
/// `parse_json_content` intentionally unwraps a one-element object array (`[{...}]`) for
/// object-oriented agent protocols. Digest is array-oriented, so a valid one-card response can
/// arrive here as the card object itself. Only objects carrying all card discriminator fields are
/// restored to a singleton array; unrelated objects remain empty. The restored item still passes
/// through `parse_cards_from_llm_array`, which enforces every closed enum and field constraint.
fn digest_card_items(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::Object(object) => {
            if let Some(cards) = object.get("cards").and_then(Value::as_array) {
                return cards.clone();
            }
            if ["kind", "title", "suggestedAction", "severity"]
                .iter()
                .all(|key| object.contains_key(*key))
            {
                vec![value.clone()]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// knowledge-digest-workstation R5：dismiss 卡片必须在 regenerate 后仍然生效。
/// 老实现 `card_id = ObjectId::new()` 每次新随机 → regenerate 后 dismissed_card_ids
/// 全部成孤儿。改成由 `(account_id, report_date, kind, target_refs_signature, title)` 派生
/// sha256 前 12 字节 → ObjectId，让"同一天 + 同 kind + 同目标 + 同标题"的卡片
/// 在 regenerate 后保持稳定 cardId。新卡片（运营当日新增问题）天然得到不同 id。
fn stable_card_id(
    account_id: &str,
    report_date: &str,
    kind: &str,
    target_refs: &[Document],
    title: &str,
) -> ObjectId {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(account_id.as_bytes());
    hasher.update(b"|");
    hasher.update(report_date.as_bytes());
    hasher.update(b"|");
    hasher.update(kind.as_bytes());
    hasher.update(b"|");
    // target_refs 里只取 id+kind 拼成稳定签名（顺序敏感，prompt 已规定 LLM 按出现顺序输出）
    for tr in target_refs {
        hasher.update(tr.get_str("kind").unwrap_or("").as_bytes());
        hasher.update(b":");
        hasher.update(tr.get_str("id").unwrap_or("").as_bytes());
        hasher.update(b";");
    }
    hasher.update(b"|");
    hasher.update(title.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 12];
    bytes.copy_from_slice(&digest[..12]);
    ObjectId::from_bytes(bytes)
}

fn migrated_dismissed_card_ids(
    existing: Option<&KnowledgeDailyReport>,
    cards: &[KnowledgeDigestCard],
) -> Vec<ObjectId> {
    let Some(existing) = existing else {
        return Vec::new();
    };
    existing
        .cards
        .iter()
        .filter(|old| existing.dismissed_card_ids.contains(&old.card_id))
        .filter_map(|old| {
            cards
                .iter()
                .find(|new| {
                    new.kind == old.kind
                        && new.title == old.title
                        && new.target_refs == old.target_refs
                })
                .map(|new| new.card_id)
        })
        .collect()
}

/// severity 档位：critical > warn > info；未知值排在最后。
fn digest_severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 0,
        "warn" => 1,
        "info" => 2,
        _ => 3,
    }
}

/// 卡片的 `metric.value`，取不到时为 [`f64::MIN`]（排在同级有指标的卡片之后
/// ——没有量化依据的卡片不该抢占注意力）。
///
/// `value` 可能是 i64 或 f64：落库时按 JSON 数值的实际类型分别 insert
/// （见 `parse_cards_from_llm_array` 里的 metric 解析），两种都要认。
///
/// NaN 与「没有指标」等价处理，同样落到 `f64::MIN`。这一步不能省：`total_cmp` 的全序
/// 里正 NaN **大于** `f64::INFINITY`，若原样返回，一张 NaN 指标的卡片会在降序中霸占
/// 榜首——恰好与上面那句「没有量化依据的卡片不该抢占注意力」相反。
fn digest_metric_value(card: &KnowledgeDigestCard) -> f64 {
    let raw = card
        .metric
        .as_ref()
        .and_then(|m| m.get("value"))
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    match raw {
        Some(value) if !value.is_nan() => value,
        _ => f64::MIN,
    }
}

/// 日报卡片排序：severity 优先，同级按 `metric.value` 降序。
///
/// 第二排序键此前只写在注释里、没进代码：排序闭包只比较了 severity rank。
/// `sort_by` 是稳定排序，于是同级卡片的先后完全取决于 LLM 的输出顺序——运营看到的
/// 「今日最该处理什么」是随机的。
///
/// 用 `total_cmp` 而非 `partial_cmp().unwrap()`：后者遇 NaN 返回 `None`、unwrap 当场
/// panic，而 NaN 能从 LLM 的 f64 值一路进到这里。提成命名函数是为了让测试直接验证
/// 生产比较器本身（而不是在测试里复制一份等价闭包）。
fn compare_digest_cards(a: &KnowledgeDigestCard, b: &KnowledgeDigestCard) -> std::cmp::Ordering {
    digest_severity_rank(&a.severity)
        .cmp(&digest_severity_rank(&b.severity))
        .then_with(|| digest_metric_value(b).total_cmp(&digest_metric_value(a)))
}

/// 从 LLM 返回的 raw JSON 数组校验/裁剪/排序成 [`KnowledgeDigestCard`]。
/// 抽出此 helper 是为了让 smoke 测试覆盖封闭枚举 + 字段裁剪 + severity 排序，
/// 而不需要真正起 LLM。
///
/// `account_id`、`report_date` 与 (kind, target_refs, title) 一起派生稳定 cardId，让 regenerate
/// 后用户已 dismiss 的卡片不会重新冒出来（R5）。
fn parse_cards_from_llm_array(
    raw_arr: Vec<Value>,
    account_id: &str,
    report_date: &str,
) -> Vec<KnowledgeDigestCard> {
    let allowed_kinds = [
        "chunk_missing_field",
        "chunk_low_hit_rate",
        "chunk_caused_block",
        "pack_outdated",
        "evolution_pending",
        "evolution_released",
        "freeform",
    ];
    let allowed_severities = ["info", "warn", "critical"];
    let allowed_actions = [
        "fix_chunk",
        "add_chunk",
        "retag",
        "review_evolution",
        "dismiss",
        "freeform",
    ];

    let mut cards: Vec<KnowledgeDigestCard> = Vec::new();
    for item in raw_arr.into_iter() {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let kind = obj.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let severity = obj.get("severity").and_then(|v| v.as_str()).unwrap_or("");
        let action = obj
            .get("suggestedAction")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !allowed_kinds.contains(&kind)
            || !allowed_severities.contains(&severity)
            || !allowed_actions.contains(&action)
        {
            continue;
        }
        let mut title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        if title.chars().count() > 60 {
            title = title.chars().take(60).collect();
        }
        let mut summary = obj
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if summary.chars().count() > 200 {
            summary = summary.chars().take(200).collect();
        }
        let target_refs = obj
            .get("targetRefs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tr| {
                        let o = tr.as_object()?;
                        let kind = o.get("kind").and_then(|v| v.as_str())?.to_string();
                        let id = o.get("id").and_then(|v| v.as_str())?.to_string();
                        if id.is_empty() {
                            return None;
                        }
                        Some(doc! { "kind": kind, "id": id })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let metric = obj.get("metric").and_then(|v| v.as_object()).map(|m| {
            let mut d = Document::new();
            if let Some(name) = m.get("name").and_then(|v| v.as_str()) {
                d.insert("name", name);
            }
            if let Some(val) = m.get("value") {
                if let Some(i) = val.as_i64() {
                    d.insert("value", i);
                } else if let Some(f) = val.as_f64() {
                    d.insert("value", f);
                }
            }
            if let Some(threshold) = m.get("threshold") {
                if let Some(i) = threshold.as_i64() {
                    d.insert("threshold", i);
                } else if let Some(f) = threshold.as_f64() {
                    d.insert("threshold", f);
                }
            }
            d
        });
        cards.push(KnowledgeDigestCard {
            card_id: stable_card_id(account_id, report_date, kind, &target_refs, &title),
            kind: kind.to_string(),
            title,
            summary,
            target_refs,
            suggested_action: action.to_string(),
            severity: severity.to_string(),
            metric,
        });
        if cards.len() >= 50 {
            break;
        }
    }

    cards.sort_by(compare_digest_cards);

    cards
}

/// 生成当日 `knowledge_daily_reports` 记录。
///
/// Phase 2 落地：扫描 4 数据源（chunks 完整度 / hit-rate / blocked runs / evolution
/// proposals）→ `knowledge.digest.compose` LLM → 卡片数组 → upsert by
/// `(workspace_id, account_id, report_date)`。
///
/// 调用方：worker_loop（每日 09:00）+ digest_today / digest_regenerate sync 路径。
pub(crate) async fn generate_today_digest(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<KnowledgeDailyReport> {
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let attempt_generation =
        claim_digest_attempt(state, workspace_id, account_id, &report_date).await?;
    let run_id = format!(
        "digest_{}_{}_{}_g{}",
        workspace_id, account_id, report_date, attempt_generation
    );

    let budget = digest_run_budget(
        &run_id,
        state.config.knowledge_digest_run_token_budget,
        state.config.knowledge_digest_run_max_llm_calls,
    );
    generate_today_digest_inner(
        state,
        workspace_id,
        account_id,
        &report_date,
        &run_id,
        attempt_generation,
        budget,
    )
    .await
}

fn digest_run_budget(run_id: &str, token_budget: i64, max_llm_calls: i32) -> Arc<RunBudget> {
    Arc::new(RunBudget::new(
        run_id,
        token_budget,
        max_llm_calls,
        i32::MAX,
    ))
}

async fn claim_digest_attempt(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
) -> AppResult<i64> {
    let filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "report_date": report_date,
    };
    let now = BsonDateTime::now();
    let update = doc! {
        "$setOnInsert": {
            "workspace_id": workspace_id,
            "account_id": account_id,
            "report_date": report_date,
            "generated_at": now,
            "generated_by": "worker",
            "status": "failed",
            "error_kind": null,
            "budget_snapshot": {},
            "cards": [],
            "dismissed_card_ids": [],
            "prompt_versions": {},
            "current_generation": 0i64,
            "latest_attempt_budget_snapshot": {},
        },
        "$set": {
            "latest_attempt_status": "running",
            "latest_attempt_error_kind": null,
            "latest_attempt_at": now,
        },
        "$inc": { "attempt_generation": 1i64 },
    };
    let upsert = FindOneAndUpdateOptions::builder()
        .upsert(true)
        .return_document(ReturnDocument::After)
        .build();

    match state
        .db
        .knowledge_daily_reports()
        .find_one_and_update(filter.clone(), update.clone(), upsert)
        .await
    {
        Ok(Some(report)) => Ok(report.attempt_generation),
        Ok(None) => Err(AppError::Conflict(
            "digest_attempt_claim_missing".to_string(),
        )),
        Err(error) if is_duplicate_key_error(&error) => {
            // Concurrent first attempts can race on the unique report key.
            // The loser retries as an update after the winner inserted it.
            let retry = FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build();
            state
                .db
                .knowledge_daily_reports()
                .find_one_and_update(
                    filter,
                    doc! {
                        "$set": {
                            "latest_attempt_status": "running",
                            "latest_attempt_error_kind": null,
                            "latest_attempt_at": BsonDateTime::now(),
                        },
                        "$inc": { "attempt_generation": 1i64 },
                    },
                    retry,
                )
                .await?
                .map(|report| report.attempt_generation)
                .ok_or_else(|| AppError::Conflict("digest_attempt_claim_race".to_string()))
        }
        Err(error) => Err(error.into()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn finalize_digest_attempt(
    state: &AppState,
    mut report_filter: Document,
    attempt_generation: i64,
    status: &str,
    error_kind: Option<String>,
    budget_snapshot: Document,
    prompt_versions: Document,
    serialized_cards: mongodb::bson::Bson,
    migrated_dismissed: Vec<ObjectId>,
    now: BsonDateTime,
) -> AppResult<KnowledgeDailyReport> {
    let workspace_id = report_filter
        .get_str("workspace_id")
        .map(str::to_owned)
        .map_err(|_| AppError::Conflict("digest_attempt_identity_missing".to_string()))?;
    let account_id = report_filter
        .get_str("account_id")
        .map(str::to_owned)
        .map_err(|_| AppError::Conflict("digest_attempt_identity_missing".to_string()))?;
    let report_date = report_filter
        .get_str("report_date")
        .map(str::to_owned)
        .map_err(|_| AppError::Conflict("digest_attempt_identity_missing".to_string()))?;
    report_filter.insert("attempt_generation", attempt_generation);
    let current = state
        .db
        .knowledge_daily_reports()
        .find_one(report_filter.clone(), None)
        .await?;
    let has_success = current
        .as_ref()
        .is_some_and(|report| report.last_success_at.is_some() || report.status == "ok");

    let mut set = doc! {
        "latest_attempt_status": status,
        "latest_attempt_error_kind": error_kind.clone(),
        "latest_attempt_at": now,
        "latest_attempt_budget_snapshot": budget_snapshot.clone(),
    };
    if status == "ok" {
        set.extend(doc! {
            "generated_at": now,
            "generated_by": "worker",
            "status": "ok",
            "error_kind": null,
            "budget_snapshot": budget_snapshot,
            "cards": serialized_cards,
            "prompt_versions": prompt_versions,
            "current_generation": attempt_generation,
            "last_success_at": now,
        });
    } else if !has_success {
        // First-ever failure remains visible, but later failures cannot erase
        // a previously committed successful snapshot.
        set.extend(doc! {
            "generated_at": now,
            "generated_by": "worker",
            "status": status,
            "error_kind": error_kind,
            "budget_snapshot": budget_snapshot,
            "cards": serialized_cards,
            "prompt_versions": prompt_versions,
        });
    }

    let mut update = doc! { "$set": set };
    if status == "ok" && !migrated_dismissed.is_empty() {
        update.insert(
            "$addToSet",
            doc! { "dismissed_card_ids": { "$each": migrated_dismissed } },
        );
    }
    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();
    if let Some(saved) = state
        .db
        .knowledge_daily_reports()
        .find_one_and_update(report_filter, update, options)
        .await?
    {
        return Ok(saved);
    }

    // A newer attempt owns the row. Return its authoritative visible snapshot
    // instead of exposing this late result to a synchronous caller.
    state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "report_date": report_date,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::Conflict("digest_attempt_lost".to_string()))
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match &*error.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            matches!(write_error.code, 11000 | 11001)
        }
        ErrorKind::BulkWrite(bulk) => bulk.write_errors.as_ref().is_some_and(|errors| {
            errors
                .iter()
                .any(|write_error| matches!(write_error.code, 11000 | 11001))
        }),
        _ => false,
    }
}

/// Narrow read-only harness for SR-119 visibility redlines. It executes the
/// production health analyzer and exposes only stable chunk identities.
#[doc(hidden)]
pub async fn analyze_chunk_health_ids_for_redline(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Vec<String>> {
    Ok(analyze_chunks_health(state, workspace_id, account_id)
        .await?
        .into_iter()
        .map(|signal| signal.chunk_id)
        .collect())
}

/// Narrow production-protocol harness for SR-121 database redlines.
#[doc(hidden)]
pub async fn claim_digest_attempt_for_redline(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
) -> AppResult<i64> {
    claim_digest_attempt(state, workspace_id, account_id, report_date).await
}

/// Finalize a precomputed digest outcome through the production generation
/// fence. Tests supply cards directly so no LLM or prompt behavior is mocked.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub async fn finalize_digest_attempt_for_redline(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
    attempt_generation: i64,
    status: &str,
    error_kind: Option<String>,
    cards: Vec<KnowledgeDigestCard>,
) -> AppResult<KnowledgeDailyReport> {
    let report_filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "report_date": report_date,
    };
    let existing = state
        .db
        .knowledge_daily_reports()
        .find_one(report_filter.clone(), None)
        .await?;
    let migrated_dismissed = migrated_dismissed_card_ids(existing.as_ref(), &cards);
    finalize_digest_attempt(
        state,
        report_filter,
        attempt_generation,
        status,
        error_kind,
        doc! { "token_budget": 12345i64, "max_llm_calls": 3i64 },
        doc! { "knowledge.digest.compose": "redline" },
        mongodb::bson::to_bson(&cards)?,
        migrated_dismissed,
        BsonDateTime::now(),
    )
    .await
}

async fn generate_today_digest_inner(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
    run_id: &str,
    attempt_generation: i64,
    budget: Arc<RunBudget>,
) -> AppResult<KnowledgeDailyReport> {
    RUN_BUDGET
        .scope(Arc::clone(&budget), async move {
            do_generate(
                state,
                workspace_id,
                account_id,
                report_date,
                run_id,
                attempt_generation,
                Arc::clone(&budget),
            )
            .await
        })
        .await
}

async fn do_generate(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
    run_id: &str,
    attempt_generation: i64,
    budget: Arc<RunBudget>,
) -> AppResult<KnowledgeDailyReport> {
    // 1. 4 路只读分析（任一失败 → status=failed + 写空 cards 报告）。
    let result: AppResult<(
        Vec<ChunkHealthSignal>,
        UsageDigest,
        Vec<BlockSignal>,
        Vec<EvolutionSignal>,
        Vec<KnowledgeDigestCard>,
    )> = async {
        let chunk_health = analyze_chunks_health(state, workspace_id, account_id).await?;
        let usage = analyze_usage_logs(state, workspace_id, account_id).await?;
        let blocked = analyze_run_logs(state, workspace_id, account_id, run_id).await?;
        let evolution = analyze_evolution(state, workspace_id, account_id).await?;
        let cards = compose_cards(
            state,
            workspace_id,
            account_id,
            run_id,
            report_date,
            &chunk_health,
            &usage,
            &blocked,
            &evolution,
        )
        .await?;
        Ok((chunk_health, usage, blocked, evolution, cards))
    }
    .await;

    let snapshot = budget.snapshot();
    let budget_doc = doc! {
        "tokens_used": snapshot.tokens_used,
        "llm_calls_used": snapshot.llm_calls_used as i64,
        "token_budget": snapshot.token_budget,
        "max_llm_calls": snapshot.max_llm_calls as i64,
    };
    let prompt_versions = doc! {
        "knowledge.digest.compose": "v1",
        "knowledge.digest.summarize_logs": "v1",
    };

    let (status, error_kind, cards) = match result {
        Ok((_, _, _, _, cards)) => ("ok".to_string(), None, cards),
        Err(AppError::LlmUnavailable { kind, detail, .. }) => {
            tracing::warn!(%kind, %detail, "knowledge digest compose hit LLM error; saving failed report");
            ("failed".to_string(), Some(kind), Vec::new())
        }
        Err(AppError::BudgetExceeded { reason, .. }) => {
            tracing::warn!(%reason, "knowledge digest compose hit budget; saving partial report");
            (
                "partial".to_string(),
                Some("budget_exceeded".to_string()),
                Vec::new(),
            )
        }
        Err(err) => {
            tracing::warn!(
                ?err,
                "knowledge digest compose failed; saving failed report"
            );
            (
                "failed".to_string(),
                Some("internal".to_string()),
                Vec::new(),
            )
        }
    };

    // 2. Finalize only the latest claimed generation. A failed regeneration
    // updates latest-attempt metadata but never replaces a successful snapshot.
    // SR-124 rollout compatibility: account-scoped card ids differ from ids produced before
    // this fix. Preserve dismissals by mapping dismissed cards from the existing report to
    // semantically identical cards in this regeneration. $addToSet keeps concurrent dismisses.
    let report_filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "report_date": report_date,
    };
    let existing = state
        .db
        .knowledge_daily_reports()
        .find_one(report_filter.clone(), None)
        .await?;
    let migrated_dismissed = migrated_dismissed_card_ids(existing.as_ref(), &cards);
    let now = BsonDateTime::now();
    let serialized_cards = mongodb::bson::to_bson(&cards)?;
    let saved = finalize_digest_attempt(
        state,
        report_filter,
        attempt_generation,
        &status,
        error_kind.clone(),
        budget_doc,
        prompt_versions,
        serialized_cards,
        migrated_dismissed,
        now,
    )
    .await?;

    let attempt_committed = saved.attempt_generation == attempt_generation;
    let audit_status = if attempt_committed {
        status.as_str()
    } else {
        "superseded"
    };
    let audit_error_kind = if attempt_committed {
        error_kind.clone()
    } else {
        Some("digest_attempt_superseded".to_string())
    };

    // 3. 旁路审计：knowledge_usage_logs（route_result.kind="digest_compose"）+ AgentEvent。
    let card_count = if attempt_committed {
        cards.len() as i64
    } else {
        0
    };
    let usage_log = KnowledgeUsageLog {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        run_id: run_id.to_string(),
        knowledge_ids: Vec::new(),
        route_result: doc! {
            "kind": "digest_compose",
            "status": audit_status,
            "cardCount": card_count,
            "reportDate": report_date,
            "tokensUsed": snapshot.tokens_used,
            "llmCallsUsed": snapshot.llm_calls_used as i64,
        },
        reply_text: None,
        review_approved: attempt_committed && status == "ok",
        blocked_reason: audit_error_kind.clone(),
        tool_trace: Vec::new(),
        created_at: now,
    };
    if let Err(err) = state
        .db
        .knowledge_usage_logs()
        .insert_one(&usage_log, None)
        .await
    {
        tracing::warn!(
            ?err,
            "knowledge_usage_logs insert failed (digest); ignoring"
        );
    }

    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "knowledge_digest_generated".to_string(),
        status: audit_status.to_string(),
        summary: format!(
            "AI 知识库日报合成完成：{} 张卡片（{}）",
            card_count, audit_status
        ),
        details: Some(doc! {
            "reportDate": report_date,
            "cardCount": card_count,
            "errorKind": audit_error_kind,
            "tokensUsed": snapshot.tokens_used,
            "llmCallsUsed": snapshot.llm_calls_used as i64,
        }),
        created_at: now,
        dedupe_key: None,
    };
    if let Err(err) = state.db.events().insert_one(&event, None).await {
        tracing::warn!(?err, "agent_events insert failed (digest); ignoring");
    }

    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    // 仅测试用：构造 NaN metric 需要直接写 Bson（NaN 不是合法 JSON 值）。
    use mongodb::bson::Bson;

    #[test]
    fn duration_until_next_run_is_positive() {
        let d = duration_until_next_run(9);
        assert!(
            d.as_secs() >= 60,
            "duration must be at least 60s, got {:?}",
            d
        );
        assert!(
            d.as_secs() <= 24 * 3600,
            "duration must be ≤ 24h, got {:?}",
            d
        );
    }

    #[test]
    fn duration_until_next_run_clamps_invalid_hour() {
        // 超过 23 的 hour 在 worker_loop 里会先 .min(23)，但本函数本身收 u32，
        // 给一个 23 的边界值确保不 panic。
        let d = duration_until_next_run(23);
        assert!(d.as_secs() >= 60);
    }

    #[test]
    fn digest_run_budget_uses_effective_config_values() {
        let budget = digest_run_budget("digest-config-test", 12_345, 3);
        let snapshot = budget.snapshot();
        assert_eq!(snapshot.token_budget, 12_345);
        assert_eq!(snapshot.max_llm_calls, 3);
        assert_eq!(snapshot.tool_call_budget, i32::MAX);
    }

    #[test]
    fn digest_card_items_restores_singleton_card_object_from_shared_llm_parser() {
        let singleton = json!({
            "kind": "chunk_missing_field",
            "title": "缺 sourceQuote",
            "summary": "AI 建议补完原文出处",
            "targetRefs": [{"kind": "chunk", "id": "chunk-a"}],
            "suggestedAction": "fix_chunk",
            "severity": "warn"
        });

        let items = digest_card_items(&singleton);
        assert_eq!(items, vec![singleton]);
        let cards = parse_cards_from_llm_array(items, "acc-a", "2026-07-27");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, "chunk_missing_field");
        assert_eq!(cards[0].suggested_action, "fix_chunk");
        assert_eq!(cards[0].target_refs[0].get_str("id").unwrap(), "chunk-a");
    }

    #[test]
    fn digest_card_items_rejects_unrelated_or_invalid_singleton_objects() {
        assert!(digest_card_items(&json!({
            "naturalReply": "not a digest card",
            "plannedSteps": []
        }))
        .is_empty());

        let invalid_enum = json!({
            "kind": "unknown_card_kind",
            "title": "invalid",
            "summary": "must still pass the closed-enum parser",
            "targetRefs": [{"kind": "chunk", "id": "chunk-a"}],
            "suggestedAction": "fix_chunk",
            "severity": "warn"
        });
        let items = digest_card_items(&invalid_enum);
        assert_eq!(items.len(), 1, "shape normalization is not enum validation");
        assert!(parse_cards_from_llm_array(items, "acc-a", "2026-07-27").is_empty());
    }

    /// Phase 2 smoke：LLM 返回**未知 kind / severity / suggestedAction** 时，
    /// `parse_cards_from_llm_array` 必须**整张丢弃**，不允许污染封闭枚举。
    #[test]
    fn parse_cards_drops_items_with_unknown_enum_values() {
        let raw = vec![
            // 合法
            json!({
                "kind": "chunk_missing_field",
                "title": "缺 sourceQuote",
                "summary": "AI 建议补完 1 条切片的原文出处",
                "targetRefs": [{"kind": "chunk", "id": "abc"}],
                "suggestedAction": "fix_chunk",
                "severity": "warn"
            }),
            // 非法 kind（测试用占位，避免使用产品红线词）
            json!({
                "kind": "unknown_card_kind",
                "title": "x", "summary": "y",
                "suggestedAction": "fix_chunk", "severity": "warn"
            }),
            // 非法 severity
            json!({
                "kind": "chunk_missing_field",
                "title": "x", "summary": "y",
                "suggestedAction": "fix_chunk", "severity": "fatal"
            }),
            // 非法 action
            json!({
                "kind": "chunk_missing_field",
                "title": "x", "summary": "y",
                "suggestedAction": "delete", "severity": "info"
            }),
            // title 空
            json!({
                "kind": "freeform",
                "title": "",
                "summary": "y",
                "suggestedAction": "freeform", "severity": "info"
            }),
        ];
        let cards = parse_cards_from_llm_array(raw, "acc-a", "2026-05-24");
        assert_eq!(cards.len(), 1, "只有第一张合法卡片可入库");
        assert_eq!(cards[0].kind, "chunk_missing_field");
        assert_eq!(cards[0].severity, "warn");
        assert_eq!(cards[0].suggested_action, "fix_chunk");
    }

    /// Phase 2 smoke：severity 排序为 critical > warn > info；
    /// 同时 title > 60 字 / summary > 200 字必须**截断**而不丢卡片。
    #[test]
    fn parse_cards_sorts_by_severity_and_truncates_long_text() {
        let long_title: String = "标".repeat(80);
        let long_summary: String = "述".repeat(220);
        let raw = vec![
            json!({
                "kind": "freeform",
                "title": "info 卡",
                "summary": "summary",
                "suggestedAction": "freeform",
                "severity": "info"
            }),
            json!({
                "kind": "evolution_pending",
                "title": long_title.clone(),
                "summary": long_summary.clone(),
                "targetRefs": [{"kind": "proposal", "id": "p1"}],
                "suggestedAction": "review_evolution",
                "severity": "critical"
            }),
            json!({
                "kind": "chunk_low_hit_rate",
                "title": "warn 卡",
                "summary": "summary",
                "suggestedAction": "retag",
                "severity": "warn"
            }),
        ];
        let cards = parse_cards_from_llm_array(raw, "acc-a", "2026-05-24");
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[0].severity, "critical", "critical 必须排第一");
        assert_eq!(cards[1].severity, "warn", "warn 第二");
        assert_eq!(cards[2].severity, "info", "info 第三");
        // 长文本截断
        assert!(
            cards[0].title.chars().count() <= 60,
            "title 超长必须截断，实际 {} 字符",
            cards[0].title.chars().count()
        );
        assert!(
            cards[0].summary.chars().count() <= 200,
            "summary 超长必须截断，实际 {} 字符",
            cards[0].summary.chars().count()
        );
    }

    /// 同 severity 内必须按 `metric.value` 降序，缺指标的排最后。
    ///
    /// 回归点：注释一直写着「同级按 metric.value desc」，而 `sort_by` 的闭包只比较了
    /// severity rank。`sort_by` 是稳定排序，于是同级顺序完全由 LLM 的输出顺序决定，
    /// 运营看到的「今日最该先处理什么」是随机的。本例把最大值故意放在输入末尾——
    /// 只有第二排序键真正参与比较，它才能冒到最前。
    #[test]
    fn parse_cards_sorts_same_severity_by_metric_value_desc() {
        let card = |title: &str, metric: Value| {
            let mut obj = json!({
                "kind": "chunk_missing_field",
                "title": title,
                "summary": "同 severity，靠 metric.value 分先后",
                "suggestedAction": "fix_chunk",
                "severity": "warn"
            });
            if !metric.is_null() {
                obj["metric"] = metric;
            }
            obj
        };
        let raw = vec![
            card("小值", json!({"name": "missing_fields", "value": 2})),
            card("无指标", Value::Null),
            // i64 与 f64 混用：落库时按类型分别 insert，比较器必须两种都认。
            card("中值", json!({"name": "hit_rate", "value": 7.5})),
            card("大值", json!({"name": "missing_fields", "value": 42})),
        ];
        let cards = parse_cards_from_llm_array(raw, "acc-a", "2026-05-24");
        let titles: Vec<&str> = cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["大值", "中值", "小值", "无指标"],
            "同 severity 必须按 metric.value 降序，缺指标的排最后"
        );
    }

    /// severity 仍是第一排序键：高 severity 的小指标必须压过低 severity 的大指标。
    #[test]
    fn parse_cards_severity_outranks_metric_value() {
        let raw = vec![
            json!({
                "kind": "chunk_missing_field",
                "title": "warn 大指标",
                "summary": "s",
                "suggestedAction": "fix_chunk",
                "severity": "warn",
                "metric": {"name": "missing_fields", "value": 999}
            }),
            json!({
                "kind": "chunk_caused_block",
                "title": "critical 小指标",
                "summary": "s",
                "suggestedAction": "fix_chunk",
                "severity": "critical",
                "metric": {"name": "block_count", "value": 1}
            }),
        ];
        let cards = parse_cards_from_llm_array(raw, "acc-a", "2026-05-24");
        assert_eq!(
            cards[0].title, "critical 小指标",
            "severity 必须优先于 metric.value"
        );
    }

    /// NaN 不得让排序 panic。
    ///
    /// LLM 的 `value` 经 `as_f64` 落库，NaN 能一路进到比较函数；
    /// `partial_cmp().unwrap()` 会当场 panic，故生产实现用 `total_cmp`。
    /// 这里直接调生产比较器 `compare_digest_cards`（不复制一份闭包，否则测的是副本
    /// 而非真实代码）：NaN 不是合法 JSON 值，无法经 `parse_cards_from_llm_array`
    /// 的 serde 路径构造，只能从结构体入手。
    #[test]
    fn compare_digest_cards_survives_nan_metric_value() {
        let card = |title: &str, value: Bson| KnowledgeDigestCard {
            card_id: ObjectId::new(),
            kind: "chunk_missing_field".to_string(),
            title: title.to_string(),
            summary: String::new(),
            target_refs: vec![],
            suggested_action: "fix_chunk".to_string(),
            severity: "warn".to_string(),
            metric: Some(doc! { "name": "x", "value": value }),
        };
        let mut cards = vec![
            card("nan", Bson::Double(f64::NAN)),
            card("normal", Bson::Int64(5)),
        ];
        cards.sort_by(compare_digest_cards);
        assert_eq!(cards.len(), 2, "排序不得 panic，也不得丢卡片");
        assert_eq!(
            cards[0].title, "normal",
            "NaN 按 f64::MIN 处理，必须排在有效指标之后"
        );
    }

    /// Phase 2 smoke：单批超过 50 张卡片必须**裁剪到 ≤ 50**，防止前端画布炸开。
    #[test]
    fn parse_cards_caps_batch_at_50() {
        let raw: Vec<Value> = (0..80)
            .map(|i| {
                json!({
                    "kind": "freeform",
                    "title": format!("卡片{}", i),
                    "summary": "ok",
                    "suggestedAction": "freeform",
                    "severity": "info"
                })
            })
            .collect();
        let cards = parse_cards_from_llm_array(raw, "acc-a", "2026-05-24");
        assert_eq!(cards.len(), 50, "单批必须裁剪到 ≤ 50");
    }

    /// Phase 2 smoke：targetRefs 中非法 / 缺 id 的 ref 必须被 drop，但卡片本身保留。
    #[test]
    fn parse_cards_filters_invalid_target_refs_but_keeps_card() {
        let raw = vec![json!({
            "kind": "chunk_caused_block",
            "title": "切片 abc 被 fact_risk 拦截",
            "summary": "AI 建议复核",
            "targetRefs": [
                {"kind": "chunk", "id": "abc"},
                {"kind": "chunk"},                  // 缺 id
                {"kind": "chunk", "id": ""},         // 空 id
                "not-an-object",
            ],
            "suggestedAction": "fix_chunk",
            "severity": "critical"
        })];
        let cards = parse_cards_from_llm_array(raw, "acc-a", "2026-05-24");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].target_refs.len(), 1);
        assert_eq!(cards[0].target_refs[0].get_str("id").unwrap_or(""), "abc");
    }

    /// R5：dismiss 卡片必须在 regenerate 后仍然生效。同一 (report_date, kind,
    /// target_refs, title) 组合必须派生相同 cardId；不同 title / target / 日期
    /// 必须派生不同 cardId。否则用户 dismiss 后 regenerate 卡片又冒出来。
    #[test]
    fn parse_cards_card_id_is_stable_across_regenerations() {
        let raw = || {
            vec![json!({
                "kind": "chunk_caused_block",
                "title": "切片 abc 被 fact_risk 拦截",
                "summary": "AI 建议复核",
                "targetRefs": [{"kind": "chunk", "id": "abc"}],
                "suggestedAction": "fix_chunk",
                "severity": "critical"
            })]
        };
        // 同一天同一卡片：两次 parse 必须得到相同 cardId（regenerate 不破坏 dismiss）。
        let first = parse_cards_from_llm_array(raw(), "acc-a", "2026-05-24");
        let second = parse_cards_from_llm_array(raw(), "acc-a", "2026-05-24");
        assert_eq!(
            first[0].card_id, second[0].card_id,
            "同 (date,kind,refs,title) 必须派生相同 cardId"
        );
        // 不同日期 → 不同 cardId（昨日 dismiss 不影响今日）。
        let other_day = parse_cards_from_llm_array(raw(), "acc-a", "2026-05-25");
        assert_ne!(
            first[0].card_id, other_day[0].card_id,
            "不同 report_date 必须派生不同 cardId"
        );
        // 同日不同 title → 不同 cardId（避免不同问题被误合并）。
        let mut diff_title = raw();
        diff_title[0]["title"] = json!("切片 abc 被 pressure_risk 拦截");
        let diff = parse_cards_from_llm_array(diff_title, "acc-a", "2026-05-24");
        assert_ne!(
            first[0].card_id, diff[0].card_id,
            "不同 title 必须派生不同 cardId"
        );
        let other_account = parse_cards_from_llm_array(raw(), "acc-b", "2026-05-24");
        assert_ne!(
            first[0].card_id, other_account[0].card_id,
            "不同 account_id 必须派生不同 cardId"
        );
    }

    #[test]
    fn migrated_dismissals_map_only_semantically_identical_cards() {
        let old_dismissed_id = ObjectId::new();
        let old_visible_id = ObjectId::new();
        let new_dismissed_id = ObjectId::new();
        let new_visible_id = ObjectId::new();
        let card = |card_id, title: &str| KnowledgeDigestCard {
            card_id,
            kind: "chunk_missing_field".to_string(),
            title: title.to_string(),
            summary: "summary may change without changing card identity".to_string(),
            target_refs: vec![doc! { "kind": "chunk", "id": "chunk-a" }],
            suggested_action: "fix_chunk".to_string(),
            severity: "warn".to_string(),
            metric: None,
        };
        let existing = KnowledgeDailyReport {
            id: Some(ObjectId::new()),
            workspace_id: "ws-a".to_string(),
            account_id: "acc-a".to_string(),
            report_date: "2026-05-24".to_string(),
            generated_at: BsonDateTime::now(),
            generated_by: "worker".to_string(),
            status: "ok".to_string(),
            error_kind: None,
            budget_snapshot: Document::new(),
            cards: vec![
                card(old_dismissed_id, "same card"),
                card(old_visible_id, "visible card"),
            ],
            dismissed_card_ids: vec![old_dismissed_id],
            prompt_versions: Document::new(),
            attempt_generation: 1,
            current_generation: 1,
            latest_attempt_status: Some("ok".to_string()),
            latest_attempt_error_kind: None,
            latest_attempt_at: Some(BsonDateTime::now()),
            latest_attempt_budget_snapshot: Document::new(),
            last_success_at: Some(BsonDateTime::now()),
        };
        let regenerated = vec![
            card(new_dismissed_id, "same card"),
            card(new_visible_id, "visible card"),
            card(ObjectId::new(), "new card"),
        ];

        assert_eq!(
            migrated_dismissed_card_ids(Some(&existing), &regenerated),
            vec![new_dismissed_id]
        );
        assert!(migrated_dismissed_card_ids(None, &regenerated).is_empty());
    }
}

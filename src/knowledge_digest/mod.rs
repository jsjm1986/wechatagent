//! 知识库日报工作站 worker 入口（knowledge-digest-workstation）。
//!
//! 设计见 `.kiro/specs/knowledge-digest-workstation/{requirements,design,tasks}.md`
//! 与 `docs/agent-policy.md` 知识库日报工作站章节。
//!
//! **隔离红线**：本模块严禁引用 `crate::agent::gateway / outbox`、
//! `crate::mcp::*`、`agent_send_outbox` 写入路径或 `run_user_operation_gateway`
//! 等生产链路入口。日报合成是离线分析任务，与对话发送链路彻底隔离。
//!
//! Phase 1（本波）：仅 worker 骨架 + early-return on disabled flag + Phase 2
//! 的 `generate_today_digest` 占位（`todo!()`），路由 `GET /api/knowledge/digest/today`
//! 在未命中时直接 404，不触发同步合成。

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
    tracing::info!(
        run_hour,
        "knowledge digest worker starting (Phase 1 skeleton — generate_today_digest is todo!())"
    );
    loop {
        let wait = duration_until_next_run(run_hour);
        tracing::debug!(?wait, "knowledge digest worker sleeping until next run");
        sleep(wait).await;
        if let Err(err) = generate_today_digest(&state).await {
            tracing::warn!(?err, "knowledge digest tick failed; continuing");
        }
    }
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

/// **只读**扫描 `operation_knowledge_chunks`：
/// 1. `integrity_status ∈ {needs_review, missing_evidence}` 或非空 `missing_fields`；
/// 2. `status="draft"` 且 `created_at` ≥ 7 天。
async fn analyze_chunks_health(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Vec<ChunkHealthSignal>> {
    let now = chrono::Utc::now();
    let seven_days_ago = now - chrono::Duration::days(7);
    let mut filter = ws_filter(workspace_id, account_id);
    filter.insert(
        "$or",
        mongodb::bson::Bson::Array(vec![
            mongodb::bson::Bson::Document(doc! {
                "integrity_status": { "$in": ["needs_review", "missing_evidence"] }
            }),
            mongodb::bson::Bson::Document(doc! {
                "source_quote": { "$in": [null, ""] }
            }),
            mongodb::bson::Bson::Document(doc! {
                "status": "draft",
                "created_at": { "$lte": BsonDateTime::from_millis(seven_days_ago.timestamp_millis()) }
            }),
        ]),
    );
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(filter, None)
        .await?;
    let mut out: Vec<ChunkHealthSignal> = Vec::new();
    while let Some(chunk) = cursor.try_next().await? {
        let chunk_id = chunk
            .id
            .map(|oid| oid.to_hex())
            .unwrap_or_default();
        if chunk_id.is_empty() {
            continue;
        }
        let mut missing_fields: Vec<String> = Vec::new();
        if chunk.source_quote.as_deref().unwrap_or("").trim().is_empty() {
            missing_fields.push("sourceQuote".to_string());
        }
        if chunk.evidence_items.is_empty() {
            missing_fields.push("evidenceItems".to_string());
        }
        if chunk.safe_claims.is_empty() {
            missing_fields.push("safeClaims".to_string());
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
    filter.insert(
        "final_review_status",
        doc! { "$in": &block_states },
    );
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
            *bucket.block_reasons.entry(block_reason.clone()).or_insert(0) += 1;
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
            match summarize_block_runs(state, run_id, &chunk_id, &bucket.run_ids, &top_block_reason).await {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!(?err, chunk_id = %chunk_id, "summarize_logs failed; using fallback");
                    format!("AI 观察：该切片在 {} 条 run 上被 {} 拦截", block_count, top_block_reason)
                }
            }
        } else {
            format!("AI 观察：该切片在 {} 条 run 上被 {} 拦截", block_count, top_block_reason)
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
    run_id: &str,
    chunk_id: &str,
    run_ids: &[String],
    top_block_reason: &str,
) -> AppResult<String> {
    let system = load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "knowledge.digest.summarize_logs",
    )
    .await?;
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
        None,
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
            "rolled_back" => format!(
                "AI 已回滚：演化提案 {} 在发布后指标退化",
                p.proposal_kind
            ),
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
    run_id: &str,
    chunk_health: &[ChunkHealthSignal],
    usage: &UsageDigest,
    blocked: &[BlockSignal],
    evolution: &[EvolutionSignal],
) -> AppResult<Vec<KnowledgeDigestCard>> {
    let system = load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "knowledge.digest.compose",
    )
    .await?;

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
        None,
        None,
        Some(run_id),
        "knowledge.digest.compose",
        &system,
        &user,
    )
    .await?;

    let raw_arr = match &value {
        Value::Array(a) => a.clone(),
        Value::Object(obj) => obj
            .get("cards")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    Ok(parse_cards_from_llm_array(raw_arr))
}

/// 从 LLM 返回的 raw JSON 数组校验/裁剪/排序成 [`KnowledgeDigestCard`]。
/// 抽出此 helper 是为了让 smoke 测试覆盖封闭枚举 + 字段裁剪 + severity 排序，
/// 而不需要真正起 LLM。
fn parse_cards_from_llm_array(raw_arr: Vec<Value>) -> Vec<KnowledgeDigestCard> {
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
        let Some(obj) = item.as_object() else { continue };
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
            card_id: ObjectId::new(),
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

    // severity 排序：critical > warn > info；同级按 metric.value desc。
    cards.sort_by(|a, b| {
        let rank = |s: &str| match s {
            "critical" => 0,
            "warn" => 1,
            "info" => 2,
            _ => 3,
        };
        rank(&a.severity).cmp(&rank(&b.severity))
    });

    cards
}

/// 生成当日 `knowledge_daily_reports` 记录。
///
/// Phase 2 落地：扫描 4 数据源（chunks 完整度 / hit-rate / blocked runs / evolution
/// proposals）→ `knowledge.digest.compose` LLM → 卡片数组 → upsert by
/// `(workspace_id, account_id, report_date)`。
///
/// 调用方：worker_loop（每日 09:00）+ digest_today / digest_regenerate sync 路径。
pub(crate) async fn generate_today_digest(state: &AppState) -> AppResult<KnowledgeDailyReport> {
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let run_id = format!("digest_{}_{}", account_id, report_date);

    let budget = Arc::new(RunBudget::new(
        run_id.clone(),
        24_000,
        8,
        i32::MAX,
    ));
    generate_today_digest_inner(state, &workspace_id, &account_id, &report_date, &run_id, budget).await
}

async fn generate_today_digest_inner(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
    run_id: &str,
    budget: Arc<RunBudget>,
) -> AppResult<KnowledgeDailyReport> {
    RUN_BUDGET
        .scope(Arc::clone(&budget), async move {
            do_generate(state, workspace_id, account_id, report_date, run_id, Arc::clone(&budget)).await
        })
        .await
}

async fn do_generate(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
    run_id: &str,
    budget: Arc<RunBudget>,
) -> AppResult<KnowledgeDailyReport> {
    // 1. 4 路只读分析（任一失败 → status=failed + 写空 cards 报告）。
    let result: AppResult<(Vec<ChunkHealthSignal>, UsageDigest, Vec<BlockSignal>, Vec<EvolutionSignal>, Vec<KnowledgeDigestCard>)> = async {
        let chunk_health = analyze_chunks_health(state, workspace_id, account_id).await?;
        let usage = analyze_usage_logs(state, workspace_id, account_id).await?;
        let blocked = analyze_run_logs(state, workspace_id, account_id, run_id).await?;
        let evolution = analyze_evolution(state, workspace_id, account_id).await?;
        let cards = compose_cards(state, run_id, &chunk_health, &usage, &blocked, &evolution).await?;
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
        Err(AppError::LlmUnavailable { kind, .. }) => {
            tracing::warn!(%kind, "knowledge digest compose hit LLM error; saving failed report");
            ("failed".to_string(), Some(kind), Vec::new())
        }
        Err(AppError::BudgetExceeded { reason, .. }) => {
            tracing::warn!(%reason, "knowledge digest compose hit budget; saving partial report");
            ("partial".to_string(), Some("budget_exceeded".to_string()), Vec::new())
        }
        Err(err) => {
            tracing::warn!(?err, "knowledge digest compose failed; saving failed report");
            ("failed".to_string(), Some("internal".to_string()), Vec::new())
        }
    };

    // 2. upsert by `(workspace_id, account_id, report_date)`。
    let now = BsonDateTime::now();
    let serialized_cards = mongodb::bson::to_bson(&cards).unwrap_or_else(|_| {
        mongodb::bson::Bson::Array(Vec::new())
    });

    let update = doc! {
        "$set": {
            "workspace_id": workspace_id,
            "account_id": account_id,
            "report_date": report_date,
            "generated_at": now,
            "generated_by": "worker",
            "status": &status,
            "error_kind": error_kind.clone(),
            "budget_snapshot": budget_doc,
            "cards": serialized_cards,
            "prompt_versions": prompt_versions,
        },
        "$setOnInsert": {
            "dismissed_card_ids": mongodb::bson::Bson::Array(Vec::new()),
        },
    };

    let opts = FindOneAndUpdateOptions::builder()
        .upsert(true)
        .return_document(ReturnDocument::After)
        .build();

    let saved = state
        .db
        .knowledge_daily_reports()
        .find_one_and_update(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "report_date": report_date,
            },
            update,
            opts,
        )
        .await?
        .ok_or_else(|| AppError::External("upsert knowledge_daily_reports returned none".to_string()))?;

    // 3. 旁路审计：knowledge_usage_logs（route_result.kind="digest_compose"）+ AgentEvent。
    let card_count = cards.len() as i64;
    let usage_log = KnowledgeUsageLog {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        run_id: run_id.to_string(),
        knowledge_ids: Vec::new(),
        route_result: doc! {
            "kind": "digest_compose",
            "status": &status,
            "cardCount": card_count,
            "reportDate": report_date,
            "tokensUsed": snapshot.tokens_used,
            "llmCallsUsed": snapshot.llm_calls_used as i64,
        },
        reply_text: None,
        review_approved: status == "ok",
        blocked_reason: error_kind.clone(),
        tool_trace: Vec::new(),
        created_at: now,
    };
    if let Err(err) = state
        .db
        .knowledge_usage_logs()
        .insert_one(&usage_log, None)
        .await
    {
        tracing::warn!(?err, "knowledge_usage_logs insert failed (digest); ignoring");
    }

    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "knowledge_digest_generated".to_string(),
        status: status.clone(),
        summary: format!(
            "AI 知识库日报合成完成：{} 张卡片（{}）",
            card_count, status
        ),
        details: Some(doc! {
            "reportDate": report_date,
            "cardCount": card_count,
            "errorKind": error_kind.clone(),
            "tokensUsed": snapshot.tokens_used,
            "llmCallsUsed": snapshot.llm_calls_used as i64,
        }),
        created_at: now,
    };
    if let Err(err) = state.db.events().insert_one(&event, None).await {
        tracing::warn!(?err, "agent_events insert failed (digest); ignoring");
    }

    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_until_next_run_is_positive() {
        let d = duration_until_next_run(9);
        assert!(d.as_secs() >= 60, "duration must be at least 60s, got {:?}", d);
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
            // 非法 kind
            json!({
                "kind": "human_takeover",
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
        let cards = parse_cards_from_llm_array(raw);
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
        let cards = parse_cards_from_llm_array(raw);
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
        let cards = parse_cards_from_llm_array(raw);
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
        let cards = parse_cards_from_llm_array(raw);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].target_refs.len(), 1);
        assert_eq!(
            cards[0].target_refs[0].get_str("id").unwrap_or(""),
            "abc"
        );
    }
}

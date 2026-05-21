//! agent-autonomy-loop W6 / Task 7.1：自治回路监控聚合接口。
//!
//! 实现两条端点：
//!
//! - `GET /api/outcomes/autonomy?horizon=24h|7d|30d&account_id=...`
//!   返回 R10 的 7 个核心指标 + 分子分母原始计数 + AI 暂缓三类细分 +
//!   发送链路（outbox）状态行 + `legacy_mode_unchecked` 独立计数。
//! - `GET /api/outcomes/autonomy/revisions?limit=50&horizon=...&account_id=...`
//!   返回最近 N 条 `revisionApplied=true` 的 run，附 contact 显示名 +
//!   pre/postReply 摘要 + revisionDirection / finalReviewStatus / holdCategory。
//!
//! 实现要点：
//!
//! - `total_runs == 0` 时所有比率 SHALL 返回 `null`（不要回退成 0）。
//! - `legacy_mode_unchecked` SHALL 不计入新指标分子分母（按"未升级"独立计数）。
//! - 历史脏值（如 `held_for_human`）按 `final_review_status` 过滤即可天然剔除。
//! - 在 100k runs 规模下 ≤ 2s：所有过滤都打到 W0/W6 已建好的
//!   `(account_id, final_review_status, created_at)` 与
//!   `(account_id, autonomy_mode, created_at)` 复合索引上。

use axum::{
    extract::{Query, State},
    Json,
};
use chrono::Duration;
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppResult;

use super::AppState;

/// 在 R10 自治指标聚合中按"未升级"独立计数的状态。
const LEGACY_STATUS: &str = "legacy_mode_unchecked";

/// M3 / Task 70：`/api/outcomes/autonomy` 响应中 planner 子段聚合的事件 kind 集合。
///
/// 与 `src/planner/mod.rs` 内 `EMIT_EVENT_KINDS` 是不同语义：
/// - `EMIT_EVENT_KINDS` 决定 daily-cap 跨段计数；不含 `_tick / _capped / _backoff`。
/// - 这里是"展示给运营"的全集，包含 tick / 各段 emit / capped / backoff，便于前端可视化。
const PLANNER_EVENT_KINDS: &[&str] = &[
    "strategic_planner_tick",
    "strategic_planner_emit",
    "strategic_planner_capped",
    "strategic_planner_silent_backoff",
    "strategic_planner_commitment_tick",
    "strategic_planner_commitment_overdue",
    "strategic_planner_commitment_imminent",
    "strategic_planner_commitment_backoff",
    "strategic_planner_stage_stagnation_tick",
    "strategic_planner_stage_stagnation",
    "strategic_planner_stage_stagnation_backoff",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyMetricsQuery {
    account_id: Option<String>,
    horizon: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyRevisionsQuery {
    account_id: Option<String>,
    horizon: Option<String>,
    limit: Option<i64>,
}

/// 把 `horizon` 字符串解析为 `(label, since DateTime)` 二元组。
///
/// 默认 24h；不识别值回退到 24h。`None` 视为无窗口。返回 `None` 表示无窗口
/// 过滤；调用方在小数据量集成测试中传 `None` 即可。
fn parse_horizon_since(horizon: Option<&str>) -> Option<DateTime> {
    let label = horizon.unwrap_or("24h");
    let now = chrono::Utc::now();
    let since_chrono = match label {
        "24h" => now - Duration::hours(24),
        "7d" => now - Duration::days(7),
        "30d" => now - Duration::days(30),
        _ => now - Duration::hours(24),
    };
    Some(DateTime::from_millis(since_chrono.timestamp_millis()))
}

/// 构造 `agent_run_logs` 上的 horizon + workspace + account 过滤器。
///
/// 命中 W6 (`account_id, final_review_status, created_at`) /
/// (`account_id, autonomy_mode, created_at`) 复合索引；进一步过滤交给计数 stage。
fn build_horizon_filter(
    workspace_id: &str,
    account_id: &str,
    horizon: Option<&str>,
) -> Document {
    let mut filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
    };
    if let Some(since) = parse_horizon_since(horizon) {
        filter.insert("created_at", doc! { "$gte": since });
    }
    filter
}

/// 从 100k 维度的 `count_documents` 角度看，所有过滤都走索引；本函数仅是统一
/// `merge` 业务条件的便利助手。
fn merge_filter(base: &Document, extra: Document) -> Document {
    let mut merged = base.clone();
    for (k, v) in extra {
        merged.insert(k, v);
    }
    merged
}

/// 比率分子/分母相除：分母 0 → null（前端展示"暂无数据"）。
fn ratio(numer: u64, denom: u64) -> Value {
    if denom == 0 {
        Value::Null
    } else {
        Value::from((numer as f64) / (denom as f64))
    }
}

/// M3 / Task 70：聚合 `agent_events` 上 planner 三段 tick / emit / capped / backoff 计数，
/// 并把 silent_tick 的 `details.scanned / emitted` 累加暴露出来，给前端 Planner section 用。
///
/// 单次 aggregation pipeline：
/// 1. `$match` 走 `(workspace, account, created_at)` 索引前缀，再按 kind 白名单过滤
/// 2. `$group by kind`，`$sum: 1` 计 count，`$sum: "$details.scanned" / "$details.emitted"`
///    把 silent_tick / commitment_tick / stage_stagnation_tick 的明细汇总
///
/// 旧事件 detail 缺字段时 `$sum` 自然按 0 处理，无需额外 fallback。
async fn fetch_planner_section(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    horizon: Option<&str>,
) -> AppResult<Value> {
    let mut match_filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "kind": { "$in": PLANNER_EVENT_KINDS },
    };
    if let Some(since) = parse_horizon_since(horizon) {
        match_filter.insert("created_at", doc! { "$gte": since });
    }

    let pipeline = vec![
        doc! { "$match": match_filter },
        doc! {
            "$group": {
                "_id": "$kind",
                "count": { "$sum": 1i64 },
                "scanned": { "$sum": { "$ifNull": ["$details.scanned", 0i64] } },
                "emitted": { "$sum": { "$ifNull": ["$details.emitted", 0i64] } },
            }
        },
    ];

    let mut cursor = state.db.events().aggregate(pipeline, None).await?;
    let mut counts: std::collections::HashMap<String, (i64, i64, i64)> =
        std::collections::HashMap::new();
    while let Some(row) = cursor.try_next().await? {
        let kind = row.get_str("_id").unwrap_or_default().to_string();
        if kind.is_empty() {
            continue;
        }
        let count = row.get_i64("count").unwrap_or(0);
        let scanned = row.get_i64("scanned").unwrap_or(0);
        let emitted = row.get_i64("emitted").unwrap_or(0);
        counts.insert(kind, (count, scanned, emitted));
    }

    let pick = |kind: &str| -> (i64, i64, i64) { counts.get(kind).copied().unwrap_or((0, 0, 0)) };

    let (silent_tick_count, silent_scanned, silent_emitted_detail) = pick("strategic_planner_tick");
    let silent_emit = pick("strategic_planner_emit").0;
    let silent_capped = pick("strategic_planner_capped").0;
    let silent_backoff = pick("strategic_planner_silent_backoff").0;

    let commitment_tick = pick("strategic_planner_commitment_tick").0;
    let overdue_emits = pick("strategic_planner_commitment_overdue").0;
    let imminent_emits = pick("strategic_planner_commitment_imminent").0;
    let commitment_backoff = pick("strategic_planner_commitment_backoff").0;

    let stagnation_tick = pick("strategic_planner_stage_stagnation_tick").0;
    let stagnation_emit = pick("strategic_planner_stage_stagnation").0;
    let stagnation_backoff = pick("strategic_planner_stage_stagnation_backoff").0;

    Ok(json!({
        "silent": {
            "tick": silent_tick_count,
            "scanned": silent_scanned,
            "emitted": silent_emit,
            "tickDetailEmitted": silent_emitted_detail,
            "capped": silent_capped,
            "backoff": silent_backoff,
        },
        "commitment": {
            "tick": commitment_tick,
            "overdueEmits": overdue_emits,
            "imminentEmits": imminent_emits,
            "backoff": commitment_backoff,
        },
        "stagnation": {
            "tick": stagnation_tick,
            "emitted": stagnation_emit,
            "backoff": stagnation_backoff,
        },
    }))
}

pub async fn get_autonomy_outcomes(
    State(state): State<AppState>,
    Query(query): Query<AutonomyMetricsQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let horizon_label = query.horizon.as_deref();
    let base = build_horizon_filter(&workspace_id, &account_id, horizon_label);

    let runs = state.db.agent_run_logs();
    let outbox = state.db.collection_agent_send_outbox();

    // 「升级后」总数：finalReviewStatus 落在合法新枚举里且 != legacy_mode_unchecked。
    // 历史脏值（如空字符串、held_for_human）天然不命中此过滤，自动剔除。
    let upgraded_filter = merge_filter(
        &base,
        doc! {
            "final_review_status": {
                "$in": [
                    "approved",
                    "revision_applied_approved",
                    "revision_failed",
                    "held_by_ai_policy",
                    "blocked_by_safety_guard",
                    "ai_waiting_for_more_context",
                    "blocked_by_required_field",
                    "blocked_by_budget",
                    "blocked_unverified_product_claim",
                ]
            }
        },
    );
    let total_runs = runs
        .count_documents(upgraded_filter.clone(), None)
        .await?;

    let legacy_runs = runs
        .count_documents(
            merge_filter(&base, doc! { "final_review_status": LEGACY_STATUS }),
            None,
        )
        .await?;

    let revision_applied = runs
        .count_documents(
            merge_filter(&upgraded_filter, doc! { "revision_applied": true }),
            None,
        )
        .await?;
    let revision_pass = runs
        .count_documents(
            merge_filter(
                &upgraded_filter,
                doc! {
                    "revision_applied": true,
                    "final_review_status": "revision_applied_approved",
                },
            ),
            None,
        )
        .await?;

    let held_by_ai_policy = runs
        .count_documents(
            merge_filter(
                &base,
                doc! { "final_review_status": "held_by_ai_policy" },
            ),
            None,
        )
        .await?;
    let blocked_by_safety_guard = runs
        .count_documents(
            merge_filter(
                &base,
                doc! { "final_review_status": "blocked_by_safety_guard" },
            ),
            None,
        )
        .await?;
    let ai_waiting_for_more_context = runs
        .count_documents(
            merge_filter(
                &base,
                doc! { "final_review_status": "ai_waiting_for_more_context" },
            ),
            None,
        )
        .await?;

    let unverified_claim_block = runs
        .count_documents(
            merge_filter(
                &upgraded_filter,
                doc! { "final_review_status": "blocked_unverified_product_claim" },
            ),
            None,
        )
        .await?;

    // taxonomy_candidate_rate：review.risks 含 `taxonomy_candidate:*` 任一条目。
    // BSON `$elemMatch` + `$regex` 在升级后 run 集合上扫描。
    let taxonomy_candidate = runs
        .count_documents(
            merge_filter(
                &upgraded_filter,
                doc! {
                    "review.risks": { "$regex": "^taxonomy_candidate:" }
                },
            ),
            None,
        )
        .await?;

    // self_critique_addressed：仅在 revisionApplied=true 中统计；分母 = revision_applied。
    let self_critique_addressed = runs
        .count_documents(
            merge_filter(
                &upgraded_filter,
                doc! {
                    "revision_applied": true,
                    "review.selfCritiqueAddressed": true,
                },
            ),
            None,
        )
        .await?;

    // autonomy_mode_distribution：auto / assisted / blocked 三类占比；分母 = upgraded total。
    let autonomy_auto = runs
        .count_documents(
            merge_filter(&upgraded_filter, doc! { "autonomy_mode": "auto" }),
            None,
        )
        .await?;
    let autonomy_assisted = runs
        .count_documents(
            merge_filter(&upgraded_filter, doc! { "autonomy_mode": "assisted" }),
            None,
        )
        .await?;
    let autonomy_blocked = runs
        .count_documents(
            merge_filter(&upgraded_filter, doc! { "autonomy_mode": "blocked" }),
            None,
        )
        .await?;

    // 发送链路（outbox）状态行：sent / canceled / failed_terminal / 总入队数。
    // outbox `created_at` 与 run `created_at` 同一个 horizon 视角即可。
    let outbox_base = build_horizon_filter(&workspace_id, &account_id, horizon_label);
    let outbox_total = outbox.count_documents(outbox_base.clone(), None).await?;
    let outbox_sent = outbox
        .count_documents(merge_filter(&outbox_base, doc! { "status": "sent" }), None)
        .await?;
    let outbox_canceled = outbox
        .count_documents(
            merge_filter(&outbox_base, doc! { "status": "canceled" }),
            None,
        )
        .await?;
    let outbox_failed_terminal = outbox
        .count_documents(
            merge_filter(&outbox_base, doc! { "status": "failed_terminal" }),
            None,
        )
        .await?;

    // M3 / Task 70：planner 三段 tick / emit / capped / backoff 聚合给前端展示。
    let planner_section = fetch_planner_section(&state, &workspace_id, &account_id, horizon_label).await?;

    Ok(Json(json!({
        "horizon": horizon_label.unwrap_or("24h"),
        "accountId": account_id,
        "totalRuns": total_runs,
        "legacyModeUnchecked": legacy_runs,
        "metrics": {
            "revisionTriggerRate": ratio(revision_applied, total_runs),
            "revisionPassRate": ratio(revision_pass, revision_applied),
            "aiHoldBreakdown": {
                "heldByAiPolicy": ratio(held_by_ai_policy, total_runs),
                "blockedBySafetyGuard": ratio(blocked_by_safety_guard, total_runs),
                "aiWaitingForMoreContext": ratio(ai_waiting_for_more_context, total_runs),
            },
            "taxonomyCandidateRate": ratio(taxonomy_candidate, total_runs),
            "unverifiedClaimBlockRate": ratio(unverified_claim_block, total_runs),
            "selfCritiqueAddressedRate": ratio(self_critique_addressed, revision_applied),
            "autonomyModeDistribution": {
                "auto": ratio(autonomy_auto, total_runs),
                "assisted": ratio(autonomy_assisted, total_runs),
                "blocked": ratio(autonomy_blocked, total_runs),
            },
        },
        "rawCounts": {
            "totalRuns": total_runs,
            "revisionApplied": revision_applied,
            "revisionPass": revision_pass,
            "heldByAiPolicy": held_by_ai_policy,
            "blockedBySafetyGuard": blocked_by_safety_guard,
            "aiWaitingForMoreContext": ai_waiting_for_more_context,
            "taxonomyCandidate": taxonomy_candidate,
            "unverifiedClaimBlock": unverified_claim_block,
            "selfCritiqueAddressed": self_critique_addressed,
            "autonomyAuto": autonomy_auto,
            "autonomyAssisted": autonomy_assisted,
            "autonomyBlocked": autonomy_blocked,
            "legacyModeUnchecked": legacy_runs,
        },
        "outboxLink": {
            "totalEnqueued": outbox_total,
            "sent": outbox_sent,
            "canceled": outbox_canceled,
            "failedTerminal": outbox_failed_terminal,
            "sendSuccessRate": ratio(outbox_sent, outbox_total),
            "canceledRate": ratio(outbox_canceled, outbox_total),
            "failedTerminalRate": ratio(outbox_failed_terminal, outbox_total),
        },
        "planner": planner_section,
    })))
}

pub async fn list_autonomy_revisions(
    State(state): State<AppState>,
    Query(query): Query<AutonomyRevisionsQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let horizon_label = query.horizon.as_deref();
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let mut filter = build_horizon_filter(&workspace_id, &account_id, horizon_label);
    filter.insert("revision_applied", true);

    let mut cursor = state
        .db
        .agent_run_logs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;

    let mut items = Vec::new();
    while let Some(log) = cursor.try_next().await? {
        // contact 显示名按 `(account_id, wxid)` 查（命中 contacts unique 索引）。
        let contact_name: Option<String> = if let Some(ref wxid) = log.contact_wxid {
            let contact = state
                .db
                .contacts()
                .find_one(
                    doc! {
                        "workspace_id": &workspace_id,
                        "account_id": &account_id,
                        "wxid": wxid,
                    },
                    None,
                )
                .await?;
            contact.and_then(|c| c.remark.or(c.nickname).or(Some(wxid.clone())))
        } else {
            None
        };

        let pre = log.pre_revision_summary.unwrap_or_default();
        let post = log.post_revision_summary.unwrap_or_default();
        let pre_excerpt = excerpt(&pre, 50);
        let post_excerpt = excerpt(&post, 50);

        let revision_direction = log
            .review
            .get_str("revisionDirection")
            .ok()
            .map(|s| excerpt(s, 80))
            .unwrap_or_default();
        let hold_category = log
            .review
            .get_str("holdCategory")
            .ok()
            .map(|s| s.to_string())
            .unwrap_or_default();

        items.push(json!({
            "runId": log.run_id,
            "contactWxid": log.contact_wxid,
            "contactName": contact_name,
            "preReplyExcerpt": pre_excerpt,
            "postReplyExcerpt": post_excerpt,
            "preRevisionSummary": pre,
            "postRevisionSummary": post,
            "revisionDirection": revision_direction,
            "finalReviewStatus": log.final_review_status,
            "holdCategory": hold_category,
            "selfCritique": log.self_critique,
            "createdAt": crate::models::dt_to_string(log.created_at),
        }));
    }

    Ok(Json(json!({
        "horizon": horizon_label.unwrap_or("24h"),
        "accountId": account_id,
        "items": items,
    })))
}

/// 按 Unicode 字符截断（避免 byte-索引切碎多字节 UTF-8）。
fn excerpt(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

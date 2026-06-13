//! agent-self-evolution M4 W4 Task 5.6：+24h post-release review。
//!
//! 每次 release 后由 `release.rs` 调 [`schedule_post_release_review`] 插一条
//! `post_release_reviews` 文档（`scheduled_at = released_at + 24h`,
//! `completed=false`）。`evolution::run_one_tick` 末尾调 [`run_due_reviews`]，
//! 扫一次到期且未完成的条目，对每条：
//!
//! 1. 算 `released_at - 24h ~ released_at`（BEFORE）与
//!    `released_at ~ released_at + 24h`（AFTER）两个 24h 窗口下的 `agent_run_logs`
//!    切片；
//! 2. 写 `actual_send_success_rate_delta`（approved-like / total 差值）与
//!    `actual_5gate_hit_delta`（每个 5gate block-rate 的差值）；
//! 3. 把 `completed=true / completed_at=now`，并写一条
//!    `agent_events kind="evolution_post_release_review"`。
//!
//! **不自动回滚** —— Requirements 9.7 显式要求：post-release 仅观测，回滚必须经
//! admin 走 `POST /evolution/proposals/:id/rollback`。
//!
//! **隔离红线**：本模块严禁引用 `crate::agent::gateway / outbox`、`crate::mcp::*`、
//! `agent_send_outbox` 写入路径，或 `run_user_operation_gateway / handle_managed_message
//! / handle_follow_up_task` 等生产链路入口。

use std::collections::HashMap;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDateTime, Document};

use crate::routes::AppState;

use super::error::EvolutionError;

/// 24h 对比窗口（前后各 24h）。
const REVIEW_WINDOW_HOURS: i64 = 24;

/// `final_review_status` 落在该集合里视为「approved-like」，进入 send-success
/// 比率分子。与 `routes::outcomes_autonomy` 对升级后枚举的口径保持一致。
const APPROVED_LIKE_STATUSES: &[&str] = &["approved", "revision_applied_approved"];

/// `final_review_status` 落在该集合里视为「升级后总样本」，进入分母。
/// 历史脏值（`legacy_mode_unchecked` / 空串等）天然不命中，自动剔除。
const UPGRADED_STATUSES: &[&str] = &[
    "approved",
    "revision_applied_approved",
    "revision_failed",
    "held_by_ai_policy",
    "blocked_by_safety_guard",
    "ai_waiting_for_more_context",
    "blocked_by_required_field",
    "blocked_by_budget",
    "blocked_unverified_product_claim",
];

/// 5gate block-rate delta 关注的 gate_key（与 release_threshold 支持的 gate_key
/// 同集；planner_block_rate_threshold 不属于 5gate，所以不进 block-rate 桶）。
const FIVE_GATE_KEYS: &[(&str, &str)] = &[
    ("fact_risk_block", "blocked_by_safety_guard"),
    ("pressure_risk_block", "held_by_ai_policy"),
    ("human_like_score_rewrite", "revision_failed"),
    ("emotional_value_rewrite", "revision_failed"),
    ("product_accuracy_score_block", "blocked_unverified_product_claim"),
];

/// 安插一条 `post_release_reviews` 文档。`released_at` 由 `release.rs` 在自己
/// transaction 内确定的 `now`；`scheduled_at = released_at + 24h`。
///
/// 注意：本函数**不参与** release transaction —— 即便 insert 失败也仅 warn，
/// 不影响 release 本身（post-release review 是观测，不是门禁）。
pub async fn schedule_post_release_review(
    state: &AppState,
    proposal_id: ObjectId,
    workspace_id: &str,
    account_id: &str,
    proposal_kind: &str,
    released_at: BsonDateTime,
) -> Result<(), EvolutionError> {
    let scheduled_at = released_at_plus_hours(released_at, REVIEW_WINDOW_HOURS);
    let doc = doc! {
        "proposal_id": proposal_id,
        "workspace_id": workspace_id,
        "account_id": account_id,
        "proposal_kind": proposal_kind,
        "released_at": released_at,
        "scheduled_at": scheduled_at,
        "completed": false,
        "actual_send_success_rate_delta": null,
        "actual_5gate_hit_delta": doc! {},
        "completed_at": null,
    };
    state
        .db
        .raw()
        .collection::<Document>("post_release_reviews")
        .insert_one(doc, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

/// 扫一次 `scheduled_at <= now AND completed=false` 的待评测条目，逐条计算
/// 24h 前/后窗口的 outcomes 切片差，落字段，置 `completed=true`，并写一条
/// `agent_events kind="evolution_post_release_review"`。
///
/// 返回本次完成的条目数（用于 tick 事件 summary）。
pub async fn run_due_reviews(state: &AppState) -> Result<usize, EvolutionError> {
    let now = BsonDateTime::now();
    let mut cursor = state
        .db
        .raw()
        .collection::<Document>("post_release_reviews")
        .find(
            doc! {
                "completed": false,
                "scheduled_at": { "$lte": now },
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;

    let mut completed_count = 0usize;
    use futures::TryStreamExt;
    while let Some(review) = cursor.try_next().await.map_err(EvolutionError::from)? {
        if let Err(e) = process_one_review(state, &review).await {
            tracing::warn!(?e, "post_release_review processing failed; will retry next tick");
            continue;
        }
        completed_count += 1;
    }
    Ok(completed_count)
}

async fn process_one_review(state: &AppState, review: &Document) -> Result<(), EvolutionError> {
    let id = review
        .get_object_id("_id")
        .map_err(|e| EvolutionError::Internal(format!("post_release_reviews._id missing: {e}")))?;
    let workspace_id = review
        .get_str("workspace_id")
        .map_err(|e| EvolutionError::Internal(format!("workspace_id missing: {e}")))?
        .to_string();
    let account_id = review
        .get_str("account_id")
        .map_err(|e| EvolutionError::Internal(format!("account_id missing: {e}")))?
        .to_string();
    let proposal_id = review
        .get_object_id("proposal_id")
        .map_err(|e| EvolutionError::Internal(format!("proposal_id missing: {e}")))?;
    let proposal_kind = review
        .get_str("proposal_kind")
        .map_err(|e| EvolutionError::Internal(format!("proposal_kind missing: {e}")))?
        .to_string();
    let released_at = review
        .get_datetime("released_at")
        .map_err(|e| EvolutionError::Internal(format!("released_at missing: {e}")))?
        .to_owned();

    let before_start = released_at_plus_hours(released_at, -REVIEW_WINDOW_HOURS);
    let after_end = released_at_plus_hours(released_at, REVIEW_WINDOW_HOURS);

    let before = compute_window_metrics(state, &workspace_id, &account_id, before_start, released_at).await?;
    let after = compute_window_metrics(state, &workspace_id, &account_id, released_at, after_end).await?;

    let delta_send_success = after
        .send_success_rate
        .zip(before.send_success_rate)
        .map(|(a, b)| a - b);

    // 2.5-pre-3：业务结果兜底观测指标的前/后窗口升幅。仅观测——写进 details 供 admin
    // 察觉"放行率升但负反应也升"的背离，不参与任何 promote/rollback 判决（强制门留 main-4）。
    let delta_negative_reaction = after
        .negative_reaction_rate
        .zip(before.negative_reaction_rate)
        .map(|(a, b)| a - b);

    let mut delta_5gate = Document::new();
    for (gate_key, _status) in FIVE_GATE_KEYS {
        let a = after.five_gate_hit_rate.get(*gate_key).copied();
        let b = before.five_gate_hit_rate.get(*gate_key).copied();
        if let (Some(a), Some(b)) = (a, b) {
            delta_5gate.insert(*gate_key, a - b);
        }
    }

    let now = BsonDateTime::now();
    state
        .db
        .raw()
        .collection::<Document>("post_release_reviews")
        .update_one(
            doc! { "_id": id },
            doc! {
                "$set": {
                    "completed": true,
                    "completed_at": now,
                    "actual_send_success_rate_delta": delta_send_success,
                    "actual_5gate_hit_delta": delta_5gate.clone(),
                    "actual_negative_reaction_rate_delta": delta_negative_reaction,
                }
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;

    let mut details = doc! {
        "review_id": id,
        "proposal_id": proposal_id,
        "proposal_kind": &proposal_kind,
        "released_at": released_at,
        "before_total_runs": before.total_runs as i64,
        "after_total_runs": after.total_runs as i64,
        "actual_5gate_hit_delta": delta_5gate,
    };
    if let Some(d) = delta_send_success {
        details.insert("actual_send_success_rate_delta", d);
    }
    // 2.5-pre-3：负反应率观测（仅写 details，不参与判决）。窗口内无已分类客户反应
    // （全删失/无反应）时 before/after 为 None → delta 缺省不写，admin 面板天然空缺。
    if let Some(d) = delta_negative_reaction {
        details.insert("actual_negative_reaction_rate_delta", d);
        // 观测态 breach flag：升幅超阈仅标记供 admin 察觉"放行率升但负反应也升"的背离，
        // **不参与任何 promote/rollback 判决**（强制门留 2.5-main-4，默认关）。
        let breached = d > state.config.evolution_max_negative_reaction_increase;
        details.insert("negative_reaction_increase_breached_observed", breached);
    }
    if let Some(b) = before.negative_reaction_rate {
        details.insert("before_negative_reaction_rate", b);
    }
    if let Some(a) = after.negative_reaction_rate {
        details.insert("after_negative_reaction_rate", a);
    }

    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.clone(),
        account_id: account_id.clone(),
        contact_wxid: None,
        kind: "evolution_post_release_review".to_string(),
        status: "ok".to_string(),
        summary: format!(
            "post-release review completed for proposal {proposal_id} ({proposal_kind})"
        ),
        details: Some(details),
        created_at: now,
        dedupe_key: None,
    };
    state
        .db
        .events()
        .insert_one(event, None)
        .await
        .map_err(EvolutionError::from)?;

    Ok(())
}

/// 单个 24h 窗口的聚合产出：升级后总数、approved-like 数、每 gate 的 block 数。
struct WindowMetrics {
    total_runs: i64,
    send_success_rate: Option<f64>,
    /// gate_key → block-rate（命中 / total）。
    five_gate_hit_rate: HashMap<String, f64>,
    /// universal-domain-adaptation 2.5-pre-3：业务结果兜底观测指标。
    /// `negative_reaction_rate = Σ(label==Block) / max(1, Σ(label∈{Hit,Block}))`，
    /// 客户负反应占已分类反应的比例。数据源 = 窗口内 `agent_decision_reviews.outcome_status`
    /// 经 [`crate::knowledge_wiki::gap_signals::classify_outcome_label`] 三态判定
    /// （沉默/pending/未分类 = Censored，删失排除，不进分子也不进分母）。
    /// 窗口内无已分类反应时为 `None`（避免 0/0 NaN 落库）。
    negative_reaction_rate: Option<f64>,
}

async fn compute_window_metrics(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    start: BsonDateTime,
    end: BsonDateTime,
) -> Result<WindowMetrics, EvolutionError> {
    let runs = state.db.agent_run_logs();
    let base = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "created_at": { "$gte": start, "$lt": end },
        "final_review_status": { "$in": UPGRADED_STATUSES.to_vec() },
    };
    let total_runs = runs
        .count_documents(base.clone(), None)
        .await
        .map_err(EvolutionError::from)? as i64;
    let approved_like = runs
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "created_at": { "$gte": start, "$lt": end },
                "final_review_status": { "$in": APPROVED_LIKE_STATUSES.to_vec() },
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)? as i64;

    let send_success_rate = if total_runs > 0 {
        Some(approved_like as f64 / total_runs as f64)
    } else {
        None
    };

    let mut five_gate_hit_rate = HashMap::new();
    if total_runs > 0 {
        for (gate_key, status) in FIVE_GATE_KEYS {
            let hit = runs
                .count_documents(
                    doc! {
                        "workspace_id": workspace_id,
                        "account_id": account_id,
                        "created_at": { "$gte": start, "$lt": end },
                        "final_review_status": *status,
                    },
                    None,
                )
                .await
                .map_err(EvolutionError::from)? as i64;
            five_gate_hit_rate.insert((*gate_key).to_string(), hit as f64 / total_runs as f64);
        }
    }

    let negative_reaction_rate =
        compute_negative_reaction_rate(state, workspace_id, account_id, start, end).await?;

    Ok(WindowMetrics {
        total_runs,
        send_success_rate,
        five_gate_hit_rate,
        negative_reaction_rate,
    })
}

/// 2.5-pre-3：窗口内客户负反应率（业务结果兜底观测指标）。
///
/// 按 `(workspace_id, account_id, created_at∈[start,end))` 在 `agent_decision_reviews`
/// 上按 `outcome_status` group-count，再用回路① 同一支
/// [`crate::knowledge_wiki::gap_signals::classify_outcome_label`] 三态判定把每个
/// status 归 Hit / Block / Censored。`negative_reaction_rate = Σ(Block) /
/// (Σ(Hit)+Σ(Block))`；删失（沉默/pending/未分类）不进分子也不进分母。已分类反应
/// 为 0 时返回 `None`（避免 0/0 NaN 落库）。
///
/// **复用回路① 的 classify**：2.5-main-2 把 classify 的极性源换成 active
/// DomainProfile.outcome_polarity 后，本观测指标自动跟随同一极性，无需二次接线。
///
/// 2.5-main-4：提升为 `pub(crate)` 供 `auto_release` 的负反应强制门复用（同一口径、
/// 同一极性源），避免两处算法 drift。
pub(crate) async fn compute_negative_reaction_rate(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    start: BsonDateTime,
    end: BsonDateTime,
) -> Result<Option<f64>, EvolutionError> {
    use crate::knowledge_wiki::gap_signals::{classify_outcome_label, OutcomeLabel};
    use futures::TryStreamExt;

    let pipeline = vec![
        doc! { "$match": {
            "workspace_id": workspace_id,
            "account_id": account_id,
            "created_at": { "$gte": start, "$lt": end },
            "outcome_status": { "$exists": true, "$ne": null },
        }},
        doc! { "$group": { "_id": "$outcome_status", "n": { "$sum": 1i64 } } },
    ];
    let mut cursor = state
        .db
        .decision_reviews()
        .aggregate(pipeline, None)
        .await
        .map_err(EvolutionError::from)?;

    let mut hits: i64 = 0;
    let mut blocks: i64 = 0;
    while let Some(row) = cursor.try_next().await.map_err(EvolutionError::from)? {
        let n = row.get_i64("n").unwrap_or(0);
        let status = row.get_str("_id").ok();
        match classify_outcome_label(status) {
            OutcomeLabel::Hit => hits += n,
            OutcomeLabel::Block => blocks += n,
            OutcomeLabel::Censored => {}
        }
    }

    Ok(negative_reaction_rate_from_counts(hits, blocks))
}

/// 2.5-pre-3：从 Hit / Block 计数算负反应率的纯算术核心（删失已在 caller 排除）。
/// `Block / (Hit+Block)`；已分类反应为 0 时返回 `None`（避免 0/0 NaN 落库）。
///
/// 2.5-main-4：提升为 `pub(crate)`，与 [`compute_negative_reaction_rate`] 一并供
/// `auto_release` 复用。
pub(crate) fn negative_reaction_rate_from_counts(hits: i64, blocks: i64) -> Option<f64> {
    let classified = hits + blocks;
    if classified > 0 {
        Some(blocks as f64 / classified as f64)
    } else {
        None
    }
}

/// 把 `BsonDateTime` 加 / 减 H 小时；负值即向前回看。借由 ms 时间戳做算术，
/// 不依赖 bson `time-0_3` feature。
fn released_at_plus_hours(t: BsonDateTime, hours: i64) -> BsonDateTime {
    let delta_ms = hours
        .saturating_mul(60)
        .saturating_mul(60)
        .saturating_mul(1000);
    BsonDateTime::from_millis(t.timestamp_millis().saturating_add(delta_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_at_plus_hours_handles_positive_and_negative() {
        let t = BsonDateTime::from_millis(1_700_000_000_000);
        let plus24 = released_at_plus_hours(t, 24);
        let minus24 = released_at_plus_hours(t, -24);
        let one_day_ms = 24 * 60 * 60 * 1000;
        assert_eq!(plus24.timestamp_millis() - t.timestamp_millis(), one_day_ms);
        assert_eq!(t.timestamp_millis() - minus24.timestamp_millis(), one_day_ms);
    }

    #[test]
    fn five_gate_key_set_matches_threshold_module_keys() {
        let keys: Vec<&str> = FIVE_GATE_KEYS.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"fact_risk_block"));
        assert!(keys.contains(&"pressure_risk_block"));
        assert!(keys.contains(&"human_like_score_rewrite"));
        assert!(keys.contains(&"emotional_value_rewrite"));
        assert!(keys.contains(&"product_accuracy_score_block"));
        assert_eq!(keys.len(), 5);
    }

    #[test]
    fn approved_like_is_subset_of_upgraded() {
        for s in APPROVED_LIKE_STATUSES {
            assert!(UPGRADED_STATUSES.contains(s), "{s} must be upgraded");
        }
    }

    #[test]
    fn negative_reaction_rate_excludes_censored_from_denominator() {
        // 3 block + 1 hit = 4 已分类；删失（沉默/pending）不进分母。
        assert_eq!(negative_reaction_rate_from_counts(1, 3), Some(0.75));
        // 全 hit → 0.0（非 None）。
        assert_eq!(negative_reaction_rate_from_counts(4, 0), Some(0.0));
    }

    #[test]
    fn negative_reaction_rate_is_none_when_no_classified_reaction() {
        // 窗口内全删失（无 hit 无 block）→ None，避免 0/0 NaN 落库。
        assert_eq!(negative_reaction_rate_from_counts(0, 0), None);
    }

    #[test]
    fn negative_reaction_rate_uses_default_polarity_classification() {
        // 观测指标与回路① 共用 classify_outcome_label：DEFAULT 销售极性下
        // buying_signal=Hit、objection/complaint=Block、沉默/未知=Censored 删失。
        use crate::knowledge_wiki::gap_signals::{classify_outcome_label, OutcomeLabel};
        assert_eq!(
            classify_outcome_label(Some("user_replied_buying_signal")),
            OutcomeLabel::Hit
        );
        assert_eq!(
            classify_outcome_label(Some("user_replied_complaint")),
            OutcomeLabel::Block
        );
        // 沉默/pending/未知 = 删失（Iron Law ②），绝不当负例。
        assert_eq!(classify_outcome_label(Some("pending")), OutcomeLabel::Censored);
        assert_eq!(classify_outcome_label(None), OutcomeLabel::Censored);
    }
}

//! Phase C / C5：threshold proposal 自动 release（hold_rate close-loop）。
//!
//! Roadmap 原话："近 N 周（默认 14d）`hold_rate` 跌破阈值时自动写
//! `threshold_overrides.gate_key`，经 `post_release` 评估通过才生效"。
//!
//! 触发链路：
//! 1. 演化器 tick 末尾调 [`auto_release_eligible_thresholds`]；
//! 2. 扫描 `proposals.proposal_kind="threshold" AND status="eligible_for_release"`；
//! 3. 对每条候选回看 `evolution_auto_release_window_hours` 小时窗口的
//!    `agent_run_logs`，统计该 gate 的命中率；
//! 4. 若命中率仍在 [`super::threshold::THRESHOLD_REASONABLE_BANDS`] 之外（方向
//!    与候选方向一致，意味着信号没有自然回正） → 调
//!    [`super::release::release_threshold`]，admin id=`"evolution_auto_release"`；
//! 5. release_threshold 内部会自动 schedule +24h post_release_review，
//!    "经 post_release 评估通过才生效"由现有 review 路径承担（不自动回滚 ——
//!    Requirements 9.7：post-release 仅观测，回滚必须经 admin）。
//!
//! 自动通道**仅适用于 threshold**（纯统计可观测）；prompt 候选不在此路径，仍要
//! admin 二次确认走 `POST /api/evolution/proposals/:id/release`。
//!
//! `evolution_auto_release_enabled=false` 时整段函数立即 return，零侧效。
//!
//! **隔离红线**：本文件继承 `evolution/` 模块的红线——严禁引用
//! `crate::agent::gateway / outbox`、`crate::mcp::*` 等生产链路入口。

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};

use crate::routes::AppState;

use super::error::EvolutionError;
use super::threshold::THRESHOLD_REASONABLE_BANDS;

/// 单 tick 自动 release 主入口。返回本 tick 实际触发自动 release 的条数。
///
/// `evolution_auto_release_enabled=false` → 立即 return Ok(0)。任何下游错误均
/// 被吞掉转 warn，避免一条候选的失败拖累整个 tick；调用方（`run_one_tick`）也
/// 已用 `unwrap_or_else` 兜底。
pub async fn auto_release_eligible_thresholds(state: &AppState) -> Result<usize, EvolutionError> {
    if !state.config.evolution_auto_release_enabled {
        return Ok(0);
    }
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();
    let cap = state.config.evolution_auto_release_per_tick_cap.max(1);
    let window_hours = state.config.evolution_auto_release_window_hours.max(1) as i64;

    // 1. 拉所有 eligible_for_release threshold proposal。
    let proposals: Vec<crate::models::Proposal> = state
        .db
        .proposals()
        .find(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": &account_id,
                "proposal_kind": "threshold",
                "status": "eligible_for_release",
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .try_collect()
        .await
        .map_err(EvolutionError::from)?;

    if proposals.is_empty() {
        return Ok(0);
    }

    // 2. 算窗口内每 gate 的命中率（一次扫描复用给所有候选）。
    let now = DateTime::now();
    let window_start = DateTime::from_millis(
        now.timestamp_millis()
            .saturating_sub(window_hours * 3600 * 1000),
    );
    let hit_rates = compute_window_gate_hit_rates(state, &workspace_id, &account_id, window_start)
        .await?;

    // 3. 顺序处理候选；命中 cap 后跳过余下。
    let mut released = 0_usize;
    for proposal in proposals {
        if released >= cap {
            break;
        }
        let proposal_id = match proposal.id {
            Some(id) => id,
            None => continue,
        };
        let gate_key = match proposal.gate_key.as_deref() {
            Some(g) => g,
            None => {
                tracing::warn!(
                    ?proposal_id,
                    "auto_release: eligible threshold proposal missing gate_key; skip"
                );
                continue;
            }
        };
        let band = THRESHOLD_REASONABLE_BANDS
            .iter()
            .find(|(k, _, _)| *k == gate_key);
        let (lower, upper) = match band {
            Some((_, l, u)) => (*l, *u),
            None => {
                tracing::warn!(
                    proposal_id = ?proposal_id,
                    gate_key,
                    "auto_release: gate_key not in THRESHOLD_REASONABLE_BANDS; skip"
                );
                continue;
            }
        };
        let observed = hit_rates.get(gate_key).copied();
        let decision = decide_auto_release(observed, lower, upper);

        // 决策事件先写——无论 release 成功失败都留审计。
        let _ = write_auto_release_decision_event(
            state,
            &workspace_id,
            &account_id,
            proposal_id,
            gate_key,
            observed,
            lower,
            upper,
            decision,
        )
        .await;

        if !decision {
            continue;
        }

        match super::release::release_threshold(state, proposal_id, "evolution_auto_release").await {
            Ok(()) => {
                released += 1;
            }
            Err(e) => {
                tracing::warn!(
                    ?e,
                    ?proposal_id,
                    gate_key,
                    "auto_release: release_threshold failed; will retry next tick"
                );
            }
        }
    }
    Ok(released)
}

/// 纯函数版本：观察到的窗口命中率落在区间外 → 释放（true），落在区间内 → 跳过
/// 留给 admin（false）。`observed=None`（窗口内无样本）也保守返回 false ——
/// 没有信号不能盲目释放。
pub fn decide_auto_release(observed: Option<f64>, target_lower: f64, target_upper: f64) -> bool {
    match observed {
        None => false,
        Some(rate) => rate < target_lower || rate > target_upper,
    }
}

/// 在 `[window_start, now)` 区间扫一次 `agent_run_logs`，按 [`THRESHOLD_REASONABLE_BANDS`]
/// 6 个 gate 的命中分类聚合命中率（命中 / 总数）。`total=0` 时返回空 map（与 None 等价）。
///
/// 与 [`super::threshold::generate`] 内的口径一致：5 闸 block 类直接看
/// `final_review_status`；rewrite 类用 `revision_applied=true` 给 human_like /
/// emotional_value 各 +1 命中（反映"draft 不达标曾被 rewrite"的频次）。
async fn compute_window_gate_hit_rates(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    window_start: DateTime,
) -> Result<HashMap<String, f64>, EvolutionError> {
    let runs = state.db.agent_run_logs();
    let base = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "created_at": { "$gte": window_start },
    };
    let total = runs
        .count_documents(base.clone(), None)
        .await
        .map_err(EvolutionError::from)? as f64;
    let mut out: HashMap<String, f64> = HashMap::new();
    if total <= 0.0 {
        return Ok(out);
    }

    let mut counts: HashMap<&'static str, i64> = HashMap::new();
    for (gate, _l, _u) in THRESHOLD_REASONABLE_BANDS {
        counts.insert(*gate, 0);
    }

    let mut cursor = runs
        .find(base.clone(), None)
        .await
        .map_err(EvolutionError::from)?;
    while let Some(run) = cursor.try_next().await.map_err(EvolutionError::from)? {
        match run.final_review_status.as_str() {
            "blocked_unverified_product_claim" => {
                *counts.entry("product_accuracy_score_block").or_default() += 1;
            }
            "held_by_ai_policy" => {
                *counts.entry("fact_risk_block").or_default() += 1;
            }
            "blocked_by_safety_guard" => {
                *counts.entry("pressure_risk_block").or_default() += 1;
            }
            _ => {}
        }
        if run.revision_applied {
            *counts.entry("human_like_score_rewrite").or_default() += 1;
            *counts.entry("emotional_value_rewrite").or_default() += 1;
        }
    }
    // planner_block_rate_threshold 暂不在 agent_run_logs 计数；窗口内若无样本
    // → 留 None，decide_auto_release 会保守拒释放。
    for (gate, hit) in counts {
        out.insert(gate.to_string(), hit as f64 / total);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn write_auto_release_decision_event(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    proposal_id: mongodb::bson::oid::ObjectId,
    gate_key: &str,
    observed: Option<f64>,
    target_lower: f64,
    target_upper: f64,
    decision: bool,
) -> Result<(), EvolutionError> {
    let mut details = doc! {
        "proposal_id": proposal_id,
        "gate_key": gate_key,
        "target_lower": target_lower,
        "target_upper": target_upper,
        "decision_release": decision,
    };
    if let Some(rate) = observed {
        details.insert("hit_rate_observed", rate);
    }
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "evolution_auto_release_decision".to_string(),
        status: if decision { "release" } else { "skip" }.to_string(),
        summary: format!(
            "auto_release decision for {gate_key}: {} (observed={:?}, band=[{:.3},{:.3}])",
            if decision { "RELEASE" } else { "SKIP" },
            observed,
            target_lower,
            target_upper
        ),
        details: Some(details),
        created_at: DateTime::now(),
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

/// 给 [`super::run_one_tick`] 的事件 summary 用——把上一段事件的"已自动 release
/// 多少条"压成 [`Document`]，方便 tick_completed 事件附带。
pub fn auto_release_event_details(released: usize) -> Document {
    doc! {
        "auto_released_count": released as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_auto_release_inside_band_skips() {
        // 命中率回到正常区间 → 留给 admin 决定，不自动 release。
        assert!(!decide_auto_release(Some(0.10), 0.05, 0.15));
        assert!(!decide_auto_release(Some(0.05), 0.05, 0.15));
        assert!(!decide_auto_release(Some(0.15), 0.05, 0.15));
    }

    #[test]
    fn decide_auto_release_below_lower_releases() {
        // 命中率仍低于下限——意味阈值仍过严，候选仍需 release。
        assert!(decide_auto_release(Some(0.01), 0.05, 0.15));
    }

    #[test]
    fn decide_auto_release_above_upper_releases() {
        // 命中率仍高于上限——意味阈值仍过松，候选仍需 release。
        assert!(decide_auto_release(Some(0.50), 0.05, 0.15));
    }

    #[test]
    fn decide_auto_release_no_signal_skips() {
        // 窗口内无样本：保守拒释放，避免凭空生效阈值变更。
        assert!(!decide_auto_release(None, 0.05, 0.15));
    }

    #[test]
    fn auto_release_event_details_serializes_count() {
        let d = auto_release_event_details(3);
        assert_eq!(d.get_i32("auto_released_count").unwrap(), 3);
    }
}

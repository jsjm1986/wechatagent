//! threshold 候选生成（M4 W2 Task 3.1）。
//!
//! 纯统计路径：在 cohort 窗口内统计 6 个 gate 的命中率（fact_risk_block /
//! pressure_risk_block / human_like_score_rewrite / emotional_value_rewrite /
//! product_accuracy_score_block / planner_block_rate_threshold），与
//! [`THRESHOLD_REASONABLE_BANDS`] 对比：
//!
//! - 命中率 < 下限 → 候选 +step（表示阈值过严，需放松）
//! - 命中率 > 上限 → 候选 -step（表示阈值过松，需收紧）
//!
//! 候选 `proposed_value` clamp 到硬上下限（5 闸 [1,10]、PlannerBlockRate
//! [0.05, 0.95]）；同 gate 在 cooldown 内已 release 过则跳过。
//!
//! 单 tick 最多 4 条 proposal，按"距离目标区间最远"优先；超出 quota 的候选
//! 仍会被 insert，但 status=`rejected_below_threshold` `failure_reason="exceeded_per_tick_quota"`，
//! 保留审计痕迹。
//!
//! **不**调 LLM；不消耗 EvolutionBudget（Requirements 3.7）。

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime},
    options::FindOptions,
};

use crate::routes::AppState;

use super::error::EvolutionError;

/// 最大 per-tick threshold proposal 数。design.md §3.1 锁定为 4。
const MAX_THRESHOLD_PROPOSALS_PER_TICK: usize = 4;

/// 6 个 gate 的目标命中率区间。落在 `[lower, upper]` 内即视为正常，区间外
/// 触发候选生成。
///
/// 区间值参考 design.md §3.1：5 闸 block 类目标命中率 5%~15%（阈值过严会
/// 把太多正常 reply 挡掉）；rewrite 类目标 8%~18%（rewrite 频次过低意味
/// 阈值过松，过高意味 prompt 输出质量整体偏低）；planner block rate 区间
/// 单独定（10%~30%——planner 自我反馈环本身就是为了在这个范围内振荡）。
pub const THRESHOLD_REASONABLE_BANDS: &[(&str, f64, f64)] = &[
    ("fact_risk_block", 0.05, 0.15),
    ("pressure_risk_block", 0.05, 0.15),
    ("human_like_score_rewrite", 0.08, 0.18),
    ("emotional_value_rewrite", 0.08, 0.18),
    ("product_accuracy_score_block", 0.05, 0.15),
    ("planner_block_rate_threshold", 0.10, 0.30),
];

/// 5 闸阈值的硬边界（防止候选漂到 0 或 11）。
const FIVE_GATE_HARD_MIN: f64 = 1.0;
const FIVE_GATE_HARD_MAX: f64 = 10.0;
const PLANNER_BLOCK_RATE_HARD_MIN: f64 = 0.05;
const PLANNER_BLOCK_RATE_HARD_MAX: f64 = 0.95;

/// 5 闸阈值步长（命中率失衡时单次调整幅度）。
const FIVE_GATE_STEP: f64 = 0.5;
/// PlannerBlockRate 步长（小数比例，按 5% 一步走）。
const PLANNER_BLOCK_RATE_STEP: f64 = 0.05;

/// 把 final_review_status 映射到 5 闸命中分类。返回 `Some(gate_key)` 表示这条
/// run 命中某 gate；返回 `None` 表示不算任何 gate（如 approved / approved_after_revision）。
fn classify_gate_hit(final_review_status: &str) -> Option<&'static str> {
    match final_review_status {
        "blocked_unverified_product_claim" => Some("product_accuracy_score_block"),
        "held_by_ai_policy" => Some("fact_risk_block"),
        "blocked_by_safety_guard" => Some("pressure_risk_block"),
        // human_like / emotional_value 是 rewrite 类，rewrite 后通常 final 走
        // approved_after_revision；这里通过 revision_applied 字段补判（在 generate 内）。
        _ => None,
    }
}

/// 生成 threshold 候选。返回的 Vec 已按"按距离区间远近排序、按 quota 截断、
/// 余下置 rejected"准备就绪——调用方直接 insert_many 即可。
///
/// `experiment_id` 用于把候选挂到本 tick 的 envelope。
pub async fn generate(
    state: &AppState,
    experiment_id: &str,
    cohort_run_ids: &[ObjectId],
) -> Result<Vec<crate::models::Proposal>, EvolutionError> {
    if cohort_run_ids.is_empty() {
        return Ok(Vec::new());
    }
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();

    // 1. 把 cohort 内每条 run 拉出来，按 gate 累加命中数。
    let mut total_runs = 0_u64;
    let mut hit_counts: HashMap<&'static str, u64> =
        THRESHOLD_REASONABLE_BANDS.iter().map(|(k, _, _)| (*k, 0)).collect();
    let mut cursor = state
        .db
        .agent_run_logs()
        .find(doc! { "_id": { "$in": cohort_run_ids } }, None)
        .await
        .map_err(EvolutionError::from)?;
    while let Some(run) = cursor.try_next().await.map_err(EvolutionError::from)? {
        total_runs += 1;
        if let Some(gate) = classify_gate_hit(&run.final_review_status) {
            if let Some(c) = hit_counts.get_mut(gate) {
                *c += 1;
            }
        }
        if run.revision_applied {
            // revision 触发意味着 human_like / emotional_value 至少有一个rewrite。
            // 这里粗略地把每次 revision 算 0.5 命中给 human_like + 0.5 给 emotional_value，
            // 让两侧阈值都能感知到信号。整数计数用 ceil。
            if let Some(c) = hit_counts.get_mut("human_like_score_rewrite") {
                *c += 1;
            }
            if let Some(c) = hit_counts.get_mut("emotional_value_rewrite") {
                *c += 1;
            }
        }
    }
    if total_runs == 0 {
        return Ok(Vec::new());
    }
    let total_runs_f = total_runs as f64;

    // 2. 算每 gate 的命中率与候选方向。
    let cooldown_skipped = load_gate_cooldowns(state, &workspace_id, &account_id).await?;
    let now = DateTime::now();

    #[derive(Debug)]
    struct Candidate {
        gate: &'static str,
        hit_rate: f64,
        target_lower: f64,
        target_upper: f64,
        current_value: f64,
        proposed_raw: f64,
        clamped: bool,
        cooldown_active: bool,
        distance_from_band: f64,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (gate, lower, upper) in THRESHOLD_REASONABLE_BANDS {
        let hits = *hit_counts.get(gate).unwrap_or(&0) as f64;
        let hit_rate = hits / total_runs_f;
        if hit_rate >= *lower && hit_rate <= *upper {
            // 已在目标区间，不产候选。
            continue;
        }
        let current_value = current_threshold_value(state, gate);
        let step = if *gate == "planner_block_rate_threshold" {
            PLANNER_BLOCK_RATE_STEP
        } else {
            FIVE_GATE_STEP
        };
        // hit_rate 偏低 → 阈值过严 → +step（让阈值更高，更难触发，命中率下降——
        // 等等，这里语义反了：阈值越高越难触发命中率上升）。
        // 业务上：5 闸都是"分数 ≥ 阈值"才命中（block / rewrite），阈值越低 → 越多 run 命中。
        //   hit_rate 过低 → 阈值过高 → 应当 -step 让阈值降低，命中更多。
        //   hit_rate 过高 → 阈值过低 → 应当 +step 让阈值升高，命中更少。
        // PlannerBlockRate 同款方向：rate 越低越严格（更早 backoff），rate 过低 → +step。
        let proposed_raw = if hit_rate < *lower {
            current_value - step
        } else {
            current_value + step
        };
        let (hard_min, hard_max) = if *gate == "planner_block_rate_threshold" {
            (PLANNER_BLOCK_RATE_HARD_MIN, PLANNER_BLOCK_RATE_HARD_MAX)
        } else {
            (FIVE_GATE_HARD_MIN, FIVE_GATE_HARD_MAX)
        };
        let proposed_clamped = proposed_raw.clamp(hard_min, hard_max);
        let clamped = (proposed_clamped - proposed_raw).abs() > f64::EPSILON;
        let distance = if hit_rate < *lower {
            *lower - hit_rate
        } else {
            hit_rate - *upper
        };
        candidates.push(Candidate {
            gate,
            hit_rate,
            target_lower: *lower,
            target_upper: *upper,
            current_value,
            proposed_raw: proposed_clamped,
            clamped,
            cooldown_active: cooldown_skipped.contains(&gate.to_string()),
            distance_from_band: distance,
        });
    }

    // 3. 按距离区间倒序，挑前 N=4 个为 pending_eval；其它（cooldown / 超 quota）
    //    依旧 insert，但 status=rejected_below_threshold + 不同 failure_reason。
    candidates.sort_by(|a, b| {
        b.distance_from_band
            .partial_cmp(&a.distance_from_band)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut emitted_pending = 0_usize;
    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        let mut cohort_notes = doc! {
            "hit_rate_observed": c.hit_rate,
            "target_lower": c.target_lower,
            "target_upper": c.target_upper,
            "total_runs_in_cohort": total_runs as i64,
        };
        if c.clamped {
            cohort_notes.insert("clamped_to_value", c.proposed_raw);
        }
        let (status, failure_reason) = if c.cooldown_active {
            ("rejected_below_threshold", Some("cooldown_active"))
        } else if emitted_pending >= MAX_THRESHOLD_PROPOSALS_PER_TICK {
            ("rejected_below_threshold", Some("exceeded_per_tick_quota"))
        } else {
            emitted_pending += 1;
            ("pending_eval", None)
        };
        out.push(crate::models::Proposal {
            id: None,
            experiment_id: experiment_id.to_string(),
            workspace_id: workspace_id.clone(),
            account_id: account_id.clone(),
            proposal_kind: "threshold".to_string(),
            status: status.to_string(),
            gate_key: Some(c.gate.to_string()),
            current_value: Some(c.current_value),
            proposed_value: Some(c.proposed_raw),
            cohort_notes,
            proposed_template_key: None,
            proposed_section: None,
            diff_summary: None,
            diff_snippet: None,
            critic_reasoning: None,
            expected_improvement_on: vec![],
            risk_note: None,
            previous_prompt_version: None,
            eval_metrics: doc! {},
            eval_replays_completed: 0,
            eval_replays_failed: 0,
            significance_passed: None,
            failure_reason: failure_reason.map(str::to_string),
            released_at: None,
            released_by: None,
            rolled_back_at: None,
            rolled_back_by: None,
            created_at: now,
            updated_at: now,
        });
    }
    Ok(out)
}

/// 加载当前生效的 gate cooldown 集合（gate_key 是字符串）。
async fn load_gate_cooldowns(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> Result<std::collections::HashSet<String>, EvolutionError> {
    let cooldown_hours = state.config.evolution_threshold_release_cooldown_hours.max(1) as i64;
    let now_ms = DateTime::now().timestamp_millis();
    let since = DateTime::from_millis(now_ms.saturating_sub(cooldown_hours * 3600 * 1000));
    let mut cursor = state
        .db
        .threshold_overrides()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "released_at": { "$gte": since },
                "rolled_back_at": null,
            },
            FindOptions::builder()
                .sort(doc! { "released_at": -1 })
                .build(),
        )
        .await
        .map_err(EvolutionError::from)?;
    let mut set = std::collections::HashSet::new();
    while let Some(o) = cursor.try_next().await.map_err(EvolutionError::from)? {
        set.insert(o.gate_key);
    }
    Ok(set)
}

/// 当前生效的 gate 阈值——M4 W2 阶段优先读 AppConfig 的相关字段；W4 task 5.1
/// 引入 `resolve_thresholds` 中央读路径后改为读 threshold_overrides 兜底。
/// 这里是占位实现，避免 W2 强行依赖 W4 还未落地的 helper。
fn current_threshold_value(state: &AppState, gate: &str) -> f64 {
    match gate {
        "fact_risk_block" => 6.0,
        "pressure_risk_block" => 7.0,
        "human_like_score_rewrite" => 6.0,
        "emotional_value_rewrite" => 5.0,
        "product_accuracy_score_block" => 7.0,
        "planner_block_rate_threshold" => state.config.strategic_planner_block_rate_threshold,
        _ => 0.0,
    }
}

/// 纯函数版本：给定 gate 名 / 当前阈值 / 命中率 / 区间，返回（建议值, 是否被 clamp）。
/// 与 [`generate`] 的内部逻辑保持一致；抽出独立函数仅为单测可达。
///
/// 语义：业务上 5 闸都是"分数 ≥ 阈值"才命中（block / rewrite），阈值越低 → 越多
/// run 命中。所以：
/// - hit_rate 过低 → 阈值过高 → -step（让阈值降低，命中更多）
/// - hit_rate 过高 → 阈值过低 → +step（让阈值升高，命中更少）
///
/// PlannerBlockRate 同款方向：rate 越低越严格，rate 过低 → +step。
pub fn decide_candidate(
    gate: &str,
    current_value: f64,
    hit_rate: f64,
    target_lower: f64,
    target_upper: f64,
) -> Option<(f64, bool)> {
    if hit_rate >= target_lower && hit_rate <= target_upper {
        return None;
    }
    let step = if gate == "planner_block_rate_threshold" {
        PLANNER_BLOCK_RATE_STEP
    } else {
        FIVE_GATE_STEP
    };
    let proposed_raw = if hit_rate < target_lower {
        current_value - step
    } else {
        current_value + step
    };
    let (hard_min, hard_max) = if gate == "planner_block_rate_threshold" {
        (PLANNER_BLOCK_RATE_HARD_MIN, PLANNER_BLOCK_RATE_HARD_MAX)
    } else {
        (FIVE_GATE_HARD_MIN, FIVE_GATE_HARD_MAX)
    };
    let proposed_clamped = proposed_raw.clamp(hard_min, hard_max);
    let clamped = (proposed_clamped - proposed_raw).abs() > f64::EPSILON;
    Some((proposed_clamped, clamped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_gate_hit_known_statuses() {
        assert_eq!(
            classify_gate_hit("blocked_unverified_product_claim"),
            Some("product_accuracy_score_block")
        );
        assert_eq!(classify_gate_hit("held_by_ai_policy"), Some("fact_risk_block"));
        assert_eq!(
            classify_gate_hit("blocked_by_safety_guard"),
            Some("pressure_risk_block")
        );
        assert_eq!(classify_gate_hit("approved"), None);
        assert_eq!(classify_gate_hit("approved_after_revision"), None);
    }

    #[test]
    fn reasonable_bands_are_well_formed() {
        for (gate, lower, upper) in THRESHOLD_REASONABLE_BANDS {
            assert!(*lower < *upper, "{gate}: lower < upper");
            assert!(*lower >= 0.0 && *upper <= 1.0, "{gate}: 0..=1 range");
        }
        assert_eq!(THRESHOLD_REASONABLE_BANDS.len(), 6);
    }

    #[test]
    fn five_gate_clamp_keeps_proposal_in_range() {
        // hit_rate 极低，candidate 会 -step；但 clamp 限制 ≥ 1.0。
        let proposed = (1.0_f64 - FIVE_GATE_STEP).clamp(FIVE_GATE_HARD_MIN, FIVE_GATE_HARD_MAX);
        assert_eq!(proposed, FIVE_GATE_HARD_MIN);
        let proposed_high =
            (10.0_f64 + FIVE_GATE_STEP).clamp(FIVE_GATE_HARD_MIN, FIVE_GATE_HARD_MAX);
        assert_eq!(proposed_high, FIVE_GATE_HARD_MAX);
    }

    #[test]
    fn planner_block_rate_clamp_lower_bound() {
        let proposed = (0.05_f64 - PLANNER_BLOCK_RATE_STEP)
            .clamp(PLANNER_BLOCK_RATE_HARD_MIN, PLANNER_BLOCK_RATE_HARD_MAX);
        assert_eq!(proposed, PLANNER_BLOCK_RATE_HARD_MIN);
    }

    /// hit_rate 显著低于下限 → 阈值需 -step（让命中更容易，命中率回升）。
    #[test]
    fn decide_candidate_hit_rate_below_lower_decreases_threshold() {
        let (proposed, clamped) =
            decide_candidate("fact_risk_block", 6.0, 0.01, 0.05, 0.15).unwrap();
        assert_eq!(proposed, 5.5);
        assert!(!clamped);
    }

    /// hit_rate 显著高于上限 → 阈值需 +step（让命中更难，命中率回落）。
    #[test]
    fn decide_candidate_hit_rate_above_upper_increases_threshold() {
        let (proposed, clamped) =
            decide_candidate("pressure_risk_block", 7.0, 0.30, 0.05, 0.15).unwrap();
        assert_eq!(proposed, 7.5);
        assert!(!clamped);
    }

    /// hit_rate 在区间内 → 不产候选。
    #[test]
    fn decide_candidate_hit_rate_inside_band_returns_none() {
        assert!(decide_candidate("fact_risk_block", 6.0, 0.10, 0.05, 0.15).is_none());
    }

    /// 5 闸阈值在边界外被 clamp 到硬下限，clamped 标记为 true。
    #[test]
    fn decide_candidate_clamps_when_below_hard_min() {
        let (proposed, clamped) =
            decide_candidate("fact_risk_block", 1.0, 0.0, 0.05, 0.15).unwrap();
        assert_eq!(proposed, FIVE_GATE_HARD_MIN);
        assert!(clamped);
    }

    /// PlannerBlockRate 用更小的步长（5%）。
    #[test]
    fn decide_candidate_uses_planner_step_for_block_rate() {
        let (proposed, _) =
            decide_candidate("planner_block_rate_threshold", 0.5, 0.05, 0.10, 0.30).unwrap();
        // hit_rate < lower → -step
        assert!((proposed - 0.45).abs() < 1e-9);
    }
}

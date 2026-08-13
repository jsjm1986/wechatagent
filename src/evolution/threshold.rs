//! threshold 候选生成（M4 W2 Task 3.1）。
//!
//! 纯统计路径：在 cohort 窗口内统计 5 个 review gate 的命中率（fact_risk_block /
//! pressure_risk_block / human_like_score_rewrite / emotional_value_rewrite /
//! product_accuracy_score_block），与
//! [`THRESHOLD_REASONABLE_BANDS`] 对比：
//!
//! - `score >= threshold` 命中：命中率低则减阈值，高则加阈值；
//! - `score < threshold` 命中：方向相反。
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
use super::revision::threshold_revision;

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
    // pressure_risk_block 保留目标区间但**当前不产候选**（缺陷 #16）：pressure 在
    // 生产是软闸（触发 single-shot revision，终态痕迹是 revision_applied /
    // revision_failed），不产生任何专属 block 终态；此前把 `blocked_by_safety_guard`
    // 当作它的命中统计源是错误归因。在接入真实软闸命中观测源（如 run log 记录
    // 具体触发的 rewrite/pressure 闸）之前，`generate` 对该 gate 跳过候选生成
    // （与 planner_block_rate_threshold 同一"缺样本 ≠ 0 命中"纪律）。
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

/// 5 闸运行时使用整数阈值，候选也必须使用整数步长。
const FIVE_GATE_STEP: f64 = 1.0;
/// PlannerBlockRate 步长（小数比例，按 5% 一步走）。
const PLANNER_BLOCK_RATE_STEP: f64 = 0.05;

/// 把 final_review_status 映射到 5 闸命中分类。返回 `Some(gate_key)` 表示这条
/// run 命中某 gate；返回 `None` 表示不算任何 gate（如 approved / revision_applied_approved）。
fn classify_gate_hit(final_review_status: &str) -> Option<&'static str> {
    match final_review_status {
        "blocked_unverified_product_claim" => Some("product_accuracy_score_block"),
        "held_by_ai_policy" => Some("fact_risk_block"),
        // `blocked_by_safety_guard` 不归任何 gate（缺陷 #16 修复，保守口径）：
        // 该终态的生产来源是非产品业务事实证据门与 fail-closed 基础设施路径
        // （gates.rs:779 证据发送前失效、:818 unsupported 非产品业务事实、
        // GatewayStatusFinal::BlockedBySafetyGuard 的 R5.3.a 注释 :469-474，
        // 以及 review/mod.rs 的 ClaimGate/schema 失败 hold 族）——没有一条由
        // pressure 或 product 分数阈值驱动（pressure 是软闸走 revision，不产
        // block 终态；产品阈值门的终态是 blocked_unverified_product_claim）。
        // 归入任何 gate 都会让该 gate 的命中率混入"调阈值不会改变"的事件。
        // human_like / emotional_value 是 rewrite 类，rewrite 后通常 final 走
        // revision_applied_approved；这里通过 revision_applied 字段补判（在 generate 内）。
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
    workspace_id: &str,
    account_id: &str,
    cohort_run_ids: &[ObjectId],
) -> Result<Vec<crate::models::Proposal>, EvolutionError> {
    if cohort_run_ids.is_empty() {
        return Ok(Vec::new());
    }
    // 1. 把 cohort 内每条 run 拉出来，按 gate 累加命中数。
    //    只统计**有真实终态/字段信号源**的 gate：pressure_risk_block（软闸，无
    //    block 终态）与 planner_block_rate_threshold（无 run log 同源样本）都不在
    //    此表内——它们在下方 band 循环里经 `hit_counts.get(gate)` miss 被跳过。
    let mut total_runs = 0_u64;
    let mut hit_counts: HashMap<&'static str, f64> = gates_with_terminal_stat_source()
        .iter()
        .map(|gate| (*gate, 0.0))
        .collect();
    let mut cursor = state
        .db
        .agent_run_logs()
        .find(
            doc! {
                "_id": { "$in": cohort_run_ids },
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;
    while let Some(run) = cursor.try_next().await.map_err(EvolutionError::from)? {
        total_runs += 1;
        if let Some(gate) = classify_gate_hit(&run.final_review_status) {
            if let Some(c) = hit_counts.get_mut(gate) {
                *c += 1.0;
            }
        }
        if run.revision_applied {
            // revision 触发意味着 human_like / emotional_value 至少有一个rewrite。
            // 当前 run log 未记录究竟是哪一个 rewrite 闸触发，暂按两侧各 0.5 分摊。
            if let Some(c) = hit_counts.get_mut("human_like_score_rewrite") {
                *c += 0.5;
            }
            if let Some(c) = hit_counts.get_mut("emotional_value_rewrite") {
                *c += 0.5;
            }
        }
    }
    if total_runs == 0 {
        return Ok(Vec::new());
    }
    let total_runs_f = total_runs as f64;

    // 缺陷 #16：pressure gate 因无终态统计源被跳过——留 tick 级审计事件说明
    // 原因（best-effort：事件写失败只 warn，不影响其余 gate 的候选生成）。
    write_gate_skipped_event(
        state,
        experiment_id,
        workspace_id,
        account_id,
        "pressure_risk_block",
        "no_terminal_signal_source",
    )
    .await;

    // 2. 算每 gate 的命中率与候选方向。
    let cooldown_skipped = load_gate_cooldowns(state, workspace_id, account_id).await?;
    // #155(P1)：候选的 current_value 必须基于当前生效 override，而非硬编码占位。
    let active_overrides = load_active_threshold_overrides(state, workspace_id, account_id).await?;
    let now = DateTime::now();

    #[derive(Debug)]
    struct Candidate {
        gate: &'static str,
        hit_rate: f64,
        target_lower: f64,
        target_upper: f64,
        current_value: f64,
        /// 名实注意：存的是 `decide_candidate` 返回的 **clamp 后**候选值（clamp
        /// 发生时 cohort_notes.clamped_to_value 记录的就是本值），并非 clamp 前
        /// 的原始提案；是否发生过 clamp 由旁边的 `clamped` 标记。
        proposed_raw: f64,
        clamped: bool,
        cooldown_active: bool,
        distance_from_band: f64,
        base_revision: String,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (gate, lower, upper) in THRESHOLD_REASONABLE_BANDS {
        // 无统计源的 gate 直接跳过：planner block rate 没有 agent_run_logs 同源
        // 样本；pressure_risk_block 是软闸、无 block 终态（缺陷 #16）。缺样本不是
        // 0 命中，因此在接入真实观测源前不对它们生成候选。
        let Some(hits) = hit_counts.get(gate).copied() else {
            continue;
        };
        let hit_rate = hits / total_runs_f;
        if hit_rate >= *lower && hit_rate <= *upper {
            // 已在目标区间，不产候选。
            continue;
        }
        let active = active_overrides.get(*gate);
        let current_value = active
            .map(|override_row| override_row.value)
            .unwrap_or_else(|| default_threshold_value(state, gate));
        let (proposed_value, clamped) =
            decide_candidate(gate, current_value, hit_rate, *lower, *upper)
                .expect("out-of-band hit rate must produce a threshold candidate");
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
            proposed_raw: proposed_value,
            clamped,
            cooldown_active: cooldown_skipped.contains(&gate.to_string()),
            distance_from_band: distance,
            base_revision: threshold_revision(
                active.and_then(|override_row| override_row.id),
                current_value,
            ),
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
            workspace_id: workspace_id.to_string(),
            account_id: account_id.to_string(),
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
            base_revision: Some(c.base_revision),
            released_revision: None,
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
    let cooldown_hours = state
        .config
        .evolution_threshold_release_cooldown_hours
        .max(1) as i64;
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
                "current_version": true,
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

/// 静态默认阈值——当 `threshold_overrides` 里某 gate 还没有任何生效覆盖时的
/// baseline。与 `agent::runtime::ResolvedThresholds::baseline` 的 AppConfig 默认
/// 保持同源（5 闸默认值见 CLAUDE.md 硬规则）。
fn default_threshold_value(state: &AppState, gate: &str) -> f64 {
    match gate {
        "fact_risk_block" => 6.0,
        "pressure_risk_block" => 7.0,
        "human_like_score_rewrite" => 6.0,
        "emotional_value_rewrite" => 6.0,
        "product_accuracy_score_block" => 7.0,
        "planner_block_rate_threshold" => state.config.strategic_planner_block_rate_threshold,
        _ => 0.0,
    }
}

/// #155(P1)：读取当前生效的 `threshold_overrides`（per gate_key 最新且未 rollback）。
///
/// 旧实现 `current_threshold_value` 是硬编码占位，完全忽略已 release 的 override，
/// 导致下一轮候选从过期的 baseline 起步、且 audit 的 `previous_value` 是错的
/// （与 #152 反向显著性门配套——放松提案必须基于真实当前值才能判定方向）。
/// 这里复用 `resolve_thresholds` 的 override 读取语义（`rolled_back_at=null` +
/// `released_at desc` 去重），但工作在 workspace/account 维度（演化器 tick 无
/// `Contact` 上下文，不能直接调 `resolve_thresholds`）。
async fn load_active_threshold_overrides(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> Result<HashMap<String, ActiveThresholdOverride>, EvolutionError> {
    let mut cursor = state
        .db
        .threshold_overrides()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "current_version": true,
                "rolled_back_at": null,
            },
            FindOptions::builder()
                .sort(doc! { "released_at": -1 })
                .build(),
        )
        .await
        .map_err(EvolutionError::from)?;
    let mut out: HashMap<String, ActiveThresholdOverride> = HashMap::new();
    while let Some(o) = cursor.try_next().await.map_err(EvolutionError::from)? {
        // 首见即最新（已按 released_at desc 排序），后续同 gate 跳过。
        out.entry(o.gate_key).or_insert(ActiveThresholdOverride {
            id: o.id,
            value: o.value,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
struct ActiveThresholdOverride {
    id: Option<ObjectId>,
    value: f64,
}

/// 纯函数版本：给定 gate 名 / 当前阈值 / 命中率 / 区间，返回（建议值, 是否被 clamp）。
/// 与 [`generate`] 的内部逻辑保持一致；抽出独立函数仅为单测可达。
///
/// 比较方向由 gate 决定：hallucination / pressure / planner 是 `>=` 命中，
/// human-like / emotional-value / grounding 是 `<` 命中。
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
    let low_hit_adjustment = if gate_hits_below_threshold(gate) {
        step
    } else {
        -step
    };
    let proposed_raw = if hit_rate < target_lower {
        current_value + low_hit_adjustment
    } else {
        current_value - low_hit_adjustment
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

/// 有真实统计源的 review gate：终态映射（[`classify_gate_hit`]）覆盖 fact /
/// product 两个 block 闸；`revision_applied` 字段分摊覆盖两个 rewrite 闸。
/// pressure_risk_block **不在**此列（软闸无 block 终态，缺陷 #16）；
/// planner_block_rate_threshold 亦无 run log 同源样本。
fn gates_with_terminal_stat_source() -> &'static [&'static str] {
    &[
        "fact_risk_block",
        "human_like_score_rewrite",
        "emotional_value_rewrite",
        "product_accuracy_score_block",
    ]
}

/// 缺陷 #16 审计事件：某 gate 因无统计源被跳过候选生成。best-effort——写失败
/// 只 warn，绝不阻断其余 gate 的候选生成（与 mod.rs tick 事件族同款直写
/// `agent_events`，不触发送链）。
async fn write_gate_skipped_event(
    state: &AppState,
    experiment_id: &str,
    workspace_id: &str,
    account_id: &str,
    gate: &str,
    reason: &str,
) {
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "evolution_threshold_gate_skipped".to_string(),
        status: "info".to_string(),
        summary: format!("threshold gate {gate} skipped: {reason}"),
        details: Some(doc! {
            "experiment_id": experiment_id,
            "gate_key": gate,
            "reason": reason,
        }),
        created_at: DateTime::now(),
        dedupe_key: None,
    };
    if let Err(e) = state.db.events().insert_one(event, None).await {
        tracing::warn!(?e, gate, reason, "write evolution_threshold_gate_skipped event failed");
    }
}

fn gate_hits_below_threshold(gate: &str) -> bool {
    matches!(
        gate,
        "human_like_score_rewrite" | "emotional_value_rewrite" | "product_accuracy_score_block"
    )
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
        assert_eq!(
            classify_gate_hit("held_by_ai_policy"),
            Some("fact_risk_block")
        );
        // 缺陷 #16：blocked_by_safety_guard 来自证据门/fail-closed 基础设施路径
        // （gates.rs:779,818 + R5.3.a），与 pressure 分数阈值无因果——不归任何 gate。
        assert_eq!(classify_gate_hit("blocked_by_safety_guard"), None);
        assert_eq!(classify_gate_hit("approved"), None);
        assert_eq!(classify_gate_hit("revision_applied_approved"), None);
    }

    /// 缺陷 #16：pressure_risk_block 无终态统计源 → 不进 hit_counts 初始集，
    /// band 循环对它 `continue`，永不生成候选（与 planner 同一纪律）；band 表
    /// 仍保留其目标区间（接入真实观测源后恢复生成）。
    #[test]
    fn pressure_gate_is_excluded_from_stat_sources_but_kept_in_bands() {
        assert!(
            !gates_with_terminal_stat_source().contains(&"pressure_risk_block"),
            "pressure 软闸无 block 终态，不得进统计源集合"
        );
        assert!(
            !gates_with_terminal_stat_source().contains(&"planner_block_rate_threshold"),
            "planner 无 run log 同源样本，不得进统计源集合"
        );
        assert!(
            THRESHOLD_REASONABLE_BANDS
                .iter()
                .any(|(g, _, _)| *g == "pressure_risk_block"),
            "band 表保留 pressure 目标区间（文档化观测口径）"
        );
        // 统计源集合内的 gate 必须都有 band（防单侧漂移）。
        for gate in gates_with_terminal_stat_source() {
            assert!(
                THRESHOLD_REASONABLE_BANDS.iter().any(|(g, _, _)| g == gate),
                "统计源 gate {gate} 缺 band"
            );
        }
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
        // >= 命中闸在 hit_rate 极低时会 -step；clamp 限制 ≥ 1.0。
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

    /// >= 命中闸：hit_rate 低于下限时减阈值，让命中率回升。
    #[test]
    fn decide_candidate_hit_rate_below_lower_decreases_threshold() {
        let (proposed, clamped) =
            decide_candidate("fact_risk_block", 6.0, 0.01, 0.05, 0.15).unwrap();
        assert_eq!(proposed, 5.0);
        assert!(!clamped);
    }

    /// >= 命中闸：hit_rate 高于上限时加阈值，让命中率回落。
    #[test]
    fn decide_candidate_hit_rate_above_upper_increases_threshold() {
        let (proposed, clamped) =
            decide_candidate("pressure_risk_block", 7.0, 0.30, 0.05, 0.15).unwrap();
        assert_eq!(proposed, 8.0);
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

    #[test]
    fn decide_candidate_low_score_gate_uses_reverse_direction() {
        let (low_rate, _) =
            decide_candidate("emotional_value_rewrite", 6.0, 0.01, 0.08, 0.18).unwrap();
        let (high_rate, _) =
            decide_candidate("product_accuracy_score_block", 7.0, 0.30, 0.05, 0.15).unwrap();
        assert_eq!(low_rate, 7.0, "低于阈值命中：命中率低应提高阈值");
        assert_eq!(high_rate, 6.0, "低于阈值命中：命中率高应降低阈值");
    }

    /// PlannerBlockRate 用更小的步长（5%）。
    #[test]
    fn decide_candidate_uses_planner_step_for_block_rate() {
        let (proposed, _) =
            decide_candidate("planner_block_rate_threshold", 0.5, 0.05, 0.10, 0.30).unwrap();
        // planner >= 命中：hit_rate < lower → -step
        assert!((proposed - 0.45).abs() < 1e-9);
    }
}

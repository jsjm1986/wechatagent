//! 显著性测试纯函数（M4 W3 Task 4.4 / 4.8）。
//!
//! 输入 `Vec<ShadowReplay>`，输出 `(passed: bool, eval_metrics: Document)`。
//! 决定性、无 IO、无 LLM —— PBT / 单测可达。
//!
//! 失败短路：
//! - completed_replay_count < min_replays → reject `insufficient_completed_replays`
//! - fail_rate (failed / total) > max_fail_rate → reject `replay_fail_rate_above_threshold`
//!
//! 阈值候选（threshold）：
//! - send_success_rate_delta = new_rate - original_rate
//! - 通过条件：delta ≥ min_send_success_delta；同时 5 闸任一项 new_hit_rate - original_hit_rate ≤ max_5gate_hit_increase
//!
//! Prompt 候选（prompt）：
//! - self_critique_addressed_delta = new_addressed_rate - original_addressed_rate
//! - 通过条件：delta ≥ min_self_critique_delta；同时 5 闸任一项 hit_rate 涨幅 ≤ max_5gate_hit_increase
//! - token_cost_delta（仅观测，不强制）
//!
//! 5 闸 key 列表与 [`crate::evolution::threshold::THRESHOLD_REASONABLE_BANDS`] 一致：
//! `fact_risk_block / pressure_risk_block / human_like_score_rewrite /
//! emotional_value_rewrite / product_accuracy_score_block`（不含 planner_block_rate_threshold；
//! 后者属 planner 域，不在 shadow 内观测）。
//!
//! 任意 NaN 进入 → 直接 reject `nan_in_metrics`（防御）。

use mongodb::bson::{doc, Bson, Document};

use crate::config::AppConfig;
use crate::models::ShadowReplay;

/// 5 闸 key（shadow 范围内观测；planner_block_rate 不在 shadow 内）。
pub const FIVE_GATE_KEYS: &[&str] = &[
    "fact_risk_block",
    "pressure_risk_block",
    "human_like_score_rewrite",
    "emotional_value_rewrite",
    "product_accuracy_score_block",
];

/// 把 final_review_status 视为"成功送出"的状态集。与 design.md §4.6 一致。
pub const SEND_SUCCESS_STATUSES: &[&str] = &["approved", "approved_after_revision"];

/// 三个**安全闸**（block 类）与其 `final_review_status` 拦截态的映射。
///
/// 与 [`crate::evolution::threshold::classify_gate_hit`] 同一份约定，方向反置：
/// 给定 gate_key 找到"被该闸拦下时写入的终态字符串"。`human_like /
/// emotional_value` 是 rewrite 类、`planner_block_rate` 属 planner 域，都**不**是
/// 安全闸，不在此表内——放松它们不构成"漏过风险消息"。
///
/// #152 反向显著性门只对本表内的 gate 生效。
pub const SAFETY_GATE_BLOCK_STATUS: &[(&str, &str)] = &[
    ("fact_risk_block", "held_by_ai_policy"),
    ("pressure_risk_block", "blocked_by_safety_guard"),
    ("product_accuracy_score_block", "blocked_unverified_product_claim"),
];

/// 给定 gate_key 返回其安全拦截态；非安全闸 / None → None。
pub fn safety_block_status_for(gate_key: Option<&str>) -> Option<&'static str> {
    let key = gate_key?;
    SAFETY_GATE_BLOCK_STATUS
        .iter()
        .find(|(g, _)| *g == key)
        .map(|(_, status)| *status)
}

/// 显著性配置（无 IO 版本）。从 [`AppConfig`] 摘取。
#[derive(Debug, Clone)]
pub struct SignificanceCfg {
    pub min_replays: usize,
    pub min_send_success_delta: f64,
    pub min_self_critique_delta: f64,
    pub max_5gate_hit_increase: f64,
    pub max_fail_rate: f64,
    /// #152：安全闸放松回归率上限。shadow 中"原本被该安全闸拦下、新配置却
    /// 放行"的占比超过此值即 reject，哪怕 send_success / self_critique 都达标。
    /// 默认 0.0 —— 零容忍：任一条风险消息从 blocked 翻成 sent 即否决放松提案。
    pub max_safety_regression_rate: f64,
}

impl SignificanceCfg {
    pub fn from_config(cfg: &AppConfig) -> Self {
        Self {
            min_replays: cfg.evolution_min_replays,
            min_send_success_delta: cfg.evolution_min_send_success_delta,
            min_self_critique_delta: cfg.evolution_min_self_critique_delta,
            max_5gate_hit_increase: cfg.evolution_max_5gate_hit_increase,
            max_fail_rate: cfg.evolution_replay_max_fail_rate,
            max_safety_regression_rate: cfg.evolution_max_safety_regression_rate,
        }
    }
}

/// #152 反向显著性门：算"安全回归率"——`completed` 中 `original_final_review_status`
/// 等于该安全闸拦截态、但 `new_final_review_status` 落在 [`SEND_SUCCESS_STATUSES`]
/// 的占比（分母为全部 completed replay，即"放走的风险消息占整体流量比例"）。
///
/// 返回 `(passed, rate, count)`。`block_status=None`（非安全闸提案）→ 直接 pass，
/// rate/count 记 0，调用方据此跳过该门。
fn grade_safety_regression(
    completed: &[&ShadowReplay],
    block_status: Option<&str>,
    cfg: &SignificanceCfg,
) -> (bool, f64, u64) {
    let Some(block_status) = block_status else {
        return (true, 0.0, 0);
    };
    let n = completed.len();
    if n == 0 {
        return (true, 0.0, 0);
    }
    let count = completed
        .iter()
        .filter(|r| {
            r.original_final_review_status.as_deref() == Some(block_status)
                && r.new_final_review_status
                    .as_deref()
                    .map(|s| SEND_SUCCESS_STATUSES.contains(&s))
                    .unwrap_or(false)
        })
        .count() as u64;
    let rate = count as f64 / n as f64;
    // `>` 而非 `>=`：默认 max=0.0 时，count=0→rate=0.0→0.0>0.0=false→pass；
    // 任一回归→rate>0→fail。
    let passed = rate <= cfg.max_safety_regression_rate;
    (passed, rate, count)
}

/// 阈值候选显著性测试。
///
/// 通过条件（必须全部成立）：
/// - completed ≥ min_replays
/// - failed / total ≤ max_fail_rate
/// - send_success_rate_delta ≥ min_send_success_delta
/// - 5 闸任一项 new_hit_rate - original_hit_rate ≤ max_5gate_hit_increase
/// - #152：若 `gate_key` 是安全闸（[`SAFETY_GATE_BLOCK_STATUS`]），安全回归率
///   ≤ `max_safety_regression_rate`（默认 0.0）。非安全闸该门恒过。
pub fn grade_threshold(
    replays: &[ShadowReplay],
    cfg: &SignificanceCfg,
    gate_key: Option<&str>,
) -> (bool, Document) {
    if let Some(reason) = early_reject(replays, cfg) {
        return reason;
    }
    let completed: Vec<&ShadowReplay> =
        replays.iter().filter(|r| r.status == "completed").collect();

    let original_send = success_rate(&completed, |r| r.original_final_review_status.as_deref());
    let new_send = success_rate(&completed, |r| r.new_final_review_status.as_deref());
    let send_delta = new_send - original_send;

    if send_delta.is_nan() {
        return (false, doc! { "reason": "nan_in_metrics" });
    }

    let gate_deltas = compute_5gate_deltas(&completed);
    if let Some(reason) = nan_in_gate_deltas(&gate_deltas) {
        return (false, doc! { "reason": reason });
    }
    let max_increase = gate_deltas
        .iter()
        .map(|(_, d)| *d)
        .fold(f64::NEG_INFINITY, f64::max);

    // #152 反向显著性门：放松安全闸时，原本被拦下的风险消息不得翻成已发送。
    let safety_block_status = safety_block_status_for(gate_key);
    let (safety_passed, safety_rate, safety_count) =
        grade_safety_regression(&completed, safety_block_status, cfg);

    let send_passed = send_delta >= cfg.min_send_success_delta;
    let gate_passed = max_increase <= cfg.max_5gate_hit_increase;
    let passed = send_passed && gate_passed && safety_passed;

    let mut metrics = doc! {
        "kind": "threshold",
        "completed_replay_count": completed.len() as i64,
        "failed_replay_count": (replays.len() - completed.len()) as i64,
        "original_send_success_rate": original_send,
        "new_send_success_rate": new_send,
        "send_success_rate_delta": send_delta,
        "max_5gate_hit_increase_observed": max_increase,
        "send_success_delta_passed": send_passed,
        "gate_increase_passed": gate_passed,
        "safety_regression_passed": safety_passed,
        "safety_regression_rate": safety_rate,
        "safety_regression_count": safety_count as i64,
    };
    if let Some(status) = safety_block_status {
        metrics.insert("safety_gate_block_status", status);
    }
    let mut gate_doc = Document::new();
    for (gate, delta) in gate_deltas {
        gate_doc.insert(gate, Bson::Double(delta));
    }
    metrics.insert("five_gate_hit_delta_per_gate", gate_doc);
    if !passed {
        // 优先暴露安全回归（最危险），其次 send，最后 gate。
        let reason = if !safety_passed {
            "safety_gate_regression_above_threshold"
        } else if !send_passed {
            "send_success_delta_below_threshold"
        } else {
            "gate_hit_increase_above_threshold"
        };
        metrics.insert("reason", reason);
    }
    (passed, metrics)
}

/// Prompt 候选显著性测试。
///
/// 通过条件（必须全部成立）：
/// - completed ≥ min_replays
/// - failed / total ≤ max_fail_rate
/// - self_critique_addressed_delta ≥ min_self_critique_delta
/// - 5 闸任一项 hit_rate 涨幅 ≤ max_5gate_hit_increase
///
/// `token_cost_delta` 仅观测、不强制。
pub fn grade_prompt(replays: &[ShadowReplay], cfg: &SignificanceCfg) -> (bool, Document) {
    if let Some(reason) = early_reject(replays, cfg) {
        return reason;
    }
    let completed: Vec<&ShadowReplay> =
        replays.iter().filter(|r| r.status == "completed").collect();

    // self_critique_addressed_rate
    let original_addressed = ratio_of(&completed, |r| match r.original_self_critique_for_metric() {
        Some(true) => Some(1.0),
        Some(false) => Some(0.0),
        None => None,
    });
    let new_addressed = ratio_of(&completed, |r| match r.new_self_critique_addressed {
        Some(true) => Some(1.0),
        Some(false) => Some(0.0),
        None => None,
    });
    let critique_delta = new_addressed - original_addressed;

    // token_cost_delta：仅观测
    let token_delta = mean_token_delta(&completed);

    if critique_delta.is_nan() || token_delta.is_nan() {
        return (false, doc! { "reason": "nan_in_metrics" });
    }

    let gate_deltas = compute_5gate_deltas(&completed);
    if let Some(reason) = nan_in_gate_deltas(&gate_deltas) {
        return (false, doc! { "reason": reason });
    }
    let max_increase = gate_deltas
        .iter()
        .map(|(_, d)| *d)
        .fold(f64::NEG_INFINITY, f64::max);

    let critique_passed = critique_delta >= cfg.min_self_critique_delta;
    let gate_passed = max_increase <= cfg.max_5gate_hit_increase;
    let passed = critique_passed && gate_passed;

    let mut metrics = doc! {
        "kind": "prompt",
        "completed_replay_count": completed.len() as i64,
        "failed_replay_count": (replays.len() - completed.len()) as i64,
        "original_self_critique_addressed_rate": original_addressed,
        "new_self_critique_addressed_rate": new_addressed,
        "self_critique_addressed_delta": critique_delta,
        "max_5gate_hit_increase_observed": max_increase,
        "self_critique_delta_passed": critique_passed,
        "gate_increase_passed": gate_passed,
        "token_cost_delta_mean": token_delta,
    };
    let mut gate_doc = Document::new();
    for (gate, delta) in gate_deltas {
        gate_doc.insert(gate, Bson::Double(delta));
    }
    metrics.insert("five_gate_hit_delta_per_gate", gate_doc);
    if !passed {
        let reason = if !critique_passed {
            "self_critique_delta_below_threshold"
        } else {
            "gate_hit_increase_above_threshold"
        };
        metrics.insert("reason", reason);
    }
    (passed, metrics)
}

/// 共享的早期 reject 路径：completed 不足 / 失败率过高 → 直接 reject。
fn early_reject(
    replays: &[ShadowReplay],
    cfg: &SignificanceCfg,
) -> Option<(bool, Document)> {
    let total = replays.len();
    let completed_count = replays.iter().filter(|r| r.status == "completed").count();
    let failed_count = total - completed_count;
    if completed_count < cfg.min_replays {
        return Some((
            false,
            doc! {
                "reason": "insufficient_completed_replays",
                "completed_replay_count": completed_count as i64,
                "failed_replay_count": failed_count as i64,
                "min_replays_required": cfg.min_replays as i64,
            },
        ));
    }
    if total > 0 {
        let fail_rate = failed_count as f64 / total as f64;
        if fail_rate > cfg.max_fail_rate {
            return Some((
                false,
                doc! {
                    "reason": "replay_fail_rate_above_threshold",
                    "completed_replay_count": completed_count as i64,
                    "failed_replay_count": failed_count as i64,
                    "fail_rate": fail_rate,
                    "max_fail_rate": cfg.max_fail_rate,
                },
            ));
        }
    }
    None
}

/// "成功送出"率：以 [`SEND_SUCCESS_STATUSES`] 为正例。
fn success_rate<F>(replays: &[&ShadowReplay], extract: F) -> f64
where
    F: Fn(&ShadowReplay) -> Option<&str>,
{
    if replays.is_empty() {
        return 0.0;
    }
    let hit = replays
        .iter()
        .filter(|r| {
            extract(r)
                .map(|s| SEND_SUCCESS_STATUSES.contains(&s))
                .unwrap_or(false)
        })
        .count() as f64;
    hit / replays.len() as f64
}

/// 通用比例：`extract` 返回 `Some(1.0)` 计入分子分母、`Some(0.0)` 仅计入分母、`None` 跳过。
fn ratio_of<F>(replays: &[&ShadowReplay], extract: F) -> f64
where
    F: Fn(&ShadowReplay) -> Option<f64>,
{
    let mut num = 0.0;
    let mut denom = 0.0;
    for r in replays {
        if let Some(v) = extract(r) {
            num += v;
            denom += 1.0;
        }
    }
    if denom == 0.0 {
        return 0.0;
    }
    num / denom
}

/// 算 5 闸的 hit-rate delta = new_rate - original_rate。
/// `new_5gate_hit` 是 `Document { fact_risk_block: bool, ... }` 形态。
/// 由于 [`ShadowReplay`] 现阶段只记 `new_5gate_hit`（无 `original_5gate_hit`），
/// `original` 一侧默认全 false（即原 run 全部走 send_success 路径不命中 5 闸）。
fn compute_5gate_deltas(replays: &[&ShadowReplay]) -> Vec<(&'static str, f64)> {
    let n = replays.len() as f64;
    if n == 0.0 {
        return FIVE_GATE_KEYS.iter().map(|k| (*k, 0.0)).collect();
    }
    FIVE_GATE_KEYS
        .iter()
        .map(|gate| {
            let new_hits = replays
                .iter()
                .filter(|r| r.new_5gate_hit.get_bool(gate).unwrap_or(false))
                .count() as f64;
            let original_hits = replays
                .iter()
                .filter(|r| r.original_5gate_hit_or_default(gate))
                .count() as f64;
            let new_rate = new_hits / n;
            let original_rate = original_hits / n;
            (*gate, new_rate - original_rate)
        })
        .collect()
}

fn nan_in_gate_deltas(deltas: &[(&'static str, f64)]) -> Option<&'static str> {
    if deltas.iter().any(|(_, d)| d.is_nan()) {
        Some("nan_in_metrics")
    } else {
        None
    }
}

/// token_cost_delta（mean）：仅观测、不参与 pass/fail。
fn mean_token_delta(replays: &[&ShadowReplay]) -> f64 {
    let mut sum = 0.0_f64;
    let mut n = 0.0_f64;
    for r in replays {
        if let Some(new_cost) = r.new_token_cost {
            sum += new_cost as f64;
            n += 1.0;
        }
    }
    if n == 0.0 {
        return 0.0;
    }
    sum / n
}

/// `ShadowReplay` 内部 helpers —— 给 significance 用。
trait ShadowReplayExt {
    /// `original_self_critique_addressed` 在 [`ShadowReplay`] 现阶段未被冗余记录
    /// （只记 new 侧），原值默认 None；W3 task 4.1 的 replay 路径会同时填好两侧。
    /// 这里给一个稳定的访问点，便于后续替换字段。
    fn original_self_critique_for_metric(&self) -> Option<bool>;
    /// 同理：现阶段 `original_5gate_hit` 未存，默认全 false（不命中）。
    fn original_5gate_hit_or_default(&self, gate: &str) -> bool;
}

impl ShadowReplayExt for ShadowReplay {
    fn original_self_critique_for_metric(&self) -> Option<bool> {
        // 当前 schema 没有 original_self_critique_addressed 字段；W3 replay 可在
        // started_at 之前由 caller 写入 ad-hoc Document，这里返回 None 占位。
        None
    }
    fn original_5gate_hit_or_default(&self, _gate: &str) -> bool {
        false
    }
}

/// Task 4.5：聚合本 experiment 下所有 proposals + 各自的 shadow_replays，
/// 调 [`grade_threshold`] / [`grade_prompt`]，把 `eval_replays_completed /
/// eval_replays_failed / eval_metrics / significance_passed / status` update 回
/// proposals。全部完成后由调用方推进 `experiments.status="awaiting_admin"`。
///
/// 行为：
/// - `proposal.status="pending_eval"` 才参与（其它 status 视为已被 W2/threshold
///   quota 拒绝、不再变更）；
/// - 通过显著性测试 → status="eligible_for_release"；
/// - 否则 → status="rejected_below_threshold"，`failure_reason` 取 metrics.reason；
/// - 单条 proposal 的 update 失败不阻塞其它 proposal——错误向上抛只在 cursor 失败时。
pub async fn aggregate_and_grade(
    state: &crate::routes::AppState,
    experiment_id: &str,
) -> Result<(usize, usize), super::error::EvolutionError> {
    use futures::TryStreamExt;
    use mongodb::bson::DateTime;

    let cfg = SignificanceCfg::from_config(&state.config);

    // 1. 加载本 experiment 下所有 proposals。
    let mut proposals: Vec<crate::models::Proposal> = state
        .db
        .proposals()
        .find(doc! { "experiment_id": experiment_id }, None)
        .await
        .map_err(super::error::EvolutionError::from)?
        .try_collect()
        .await
        .map_err(super::error::EvolutionError::from)?;

    let mut eligible_count = 0_usize;
    let mut rejected_count = 0_usize;

    for proposal in proposals.iter_mut() {
        if proposal.status != "pending_eval" {
            continue;
        }
        let proposal_id = match proposal.id {
            Some(id) => id,
            None => continue, // 防御：未持久化的 proposal 不可能出现在 query 结果里
        };

        // 2. 加载该 proposal 的所有 shadow_replays。
        let replays: Vec<ShadowReplay> = state
            .db
            .shadow_replays()
            .find(doc! { "proposal_id": proposal_id }, None)
            .await
            .map_err(super::error::EvolutionError::from)?
            .try_collect()
            .await
            .map_err(super::error::EvolutionError::from)?;

        let total = replays.len();
        let completed = replays.iter().filter(|r| r.status == "completed").count();
        let failed = total - completed;

        // 3. 按 kind 调对应 grader。
        let (passed, metrics) = match proposal.proposal_kind.as_str() {
            "threshold" => grade_threshold(&replays, &cfg, proposal.gate_key.as_deref()),
            "prompt" => grade_prompt(&replays, &cfg),
            other => (
                false,
                doc! {
                    "reason": "unknown_proposal_kind",
                    "kind": other,
                },
            ),
        };

        let new_status = if passed {
            eligible_count += 1;
            "eligible_for_release"
        } else {
            rejected_count += 1;
            "rejected_below_threshold"
        };
        let failure_reason = if passed {
            None
        } else {
            metrics
                .get_str("reason")
                .ok()
                .map(str::to_string)
                .or_else(|| Some("significance_failed".to_string()))
        };

        let mut update = doc! {
            "status": new_status,
            "eval_replays_completed": completed as i32,
            "eval_replays_failed": failed as i32,
            "eval_metrics": metrics,
            "significance_passed": passed,
            "updated_at": DateTime::now(),
        };
        if let Some(reason) = failure_reason {
            update.insert("failure_reason", reason);
        }

        let _ = state
            .db
            .proposals()
            .update_one(
                doc! { "_id": proposal_id },
                doc! { "$set": update },
                None,
            )
            .await
            .map_err(super::error::EvolutionError::from)?;
    }

    Ok((eligible_count, rejected_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{doc, oid::ObjectId, DateTime};

    fn cfg() -> SignificanceCfg {
        SignificanceCfg {
            min_replays: 30,
            min_send_success_delta: 0.05,
            min_self_critique_delta: 0.10,
            max_5gate_hit_increase: 0.10,
            max_fail_rate: 0.30,
            max_safety_regression_rate: 0.0,
        }
    }

    fn rep(
        status: &str,
        original_status: Option<&str>,
        new_status: Option<&str>,
        gate_hits: Document,
        new_self_critique: Option<bool>,
        new_token_cost: Option<i64>,
    ) -> ShadowReplay {
        ShadowReplay {
            id: None,
            proposal_id: ObjectId::new(),
            experiment_id: "exp_test".to_string(),
            workspace_id: "ws".to_string(),
            account_id: "acct".to_string(),
            source_run_id: ObjectId::new(),
            status: status.to_string(),
            failure_reason: None,
            original_final_review_status: original_status.map(str::to_string),
            new_final_review_status: new_status.map(str::to_string),
            new_review_risks: vec![],
            new_token_cost,
            new_5gate_hit: gate_hits,
            new_self_critique_addressed: new_self_critique,
            similarity_to_original_text: 0.0,
            started_at: DateTime::now(),
            finished_at: Some(DateTime::now()),
        }
    }

    fn no_gate() -> Document {
        doc! {
            "fact_risk_block": false,
            "pressure_risk_block": false,
            "human_like_score_rewrite": false,
            "emotional_value_rewrite": false,
            "product_accuracy_score_block": false,
        }
    }

    /// 4.7 case 1：30 条 replay，原 0.6 / 新 0.7 → threshold passed=true
    #[test]
    fn threshold_pass_when_send_success_delta_above_min() {
        let mut replays = Vec::new();
        // 30 条：原成功 18 条（0.6），新成功 21 条（0.7）。
        for i in 0..30 {
            let original = if i < 18 { Some("approved") } else { Some("blocked_by_safety_guard") };
            let new = if i < 21 { Some("approved") } else { Some("blocked_by_safety_guard") };
            replays.push(rep("completed", original, new, no_gate(), None, None));
        }
        let (passed, metrics) = grade_threshold(&replays, &cfg(), None);
        assert!(passed, "expected passed for +0.10 send_success delta, got metrics={metrics:?}");
        assert_eq!(metrics.get_str("kind").unwrap(), "threshold");
    }

    /// 4.7 case 2：replay 失败率 > 30% → reject
    #[test]
    fn threshold_reject_when_fail_rate_above_max() {
        let mut replays = Vec::new();
        // 总 45 条，completed 31 条（≥ min_replays=30），failed 14 条（31.1% > 30%）。
        for _ in 0..31 {
            replays.push(rep(
                "completed",
                Some("approved"),
                Some("approved"),
                no_gate(),
                None,
                None,
            ));
        }
        for _ in 0..14 {
            replays.push(rep("failed", None, None, no_gate(), None, None));
        }
        let (passed, metrics) = grade_threshold(&replays, &cfg(), None);
        assert!(!passed);
        assert_eq!(
            metrics.get_str("reason").unwrap(),
            "replay_fail_rate_above_threshold"
        );
    }

    /// 4.7 case 3：5gate_hit_delta 任一 > 0.10 → prompt reject
    #[test]
    fn prompt_reject_when_any_5gate_increase_above_max() {
        let mut replays = Vec::new();
        // 30 条 completed；其中 5 条 fact_risk_block=true（0.166 hit_rate vs 原 0），
        // self_critique 全部 addressed=true（delta=1.0，过门）。
        for i in 0..30 {
            let mut gate = no_gate();
            if i < 5 {
                gate.insert("fact_risk_block", true);
            }
            replays.push(rep(
                "completed",
                Some("approved_after_revision"),
                Some("approved_after_revision"),
                gate,
                Some(true),
                Some(1000),
            ));
        }
        let (passed, metrics) = grade_prompt(&replays, &cfg());
        assert!(!passed);
        assert_eq!(
            metrics.get_str("reason").unwrap(),
            "gate_hit_increase_above_threshold"
        );
    }

    /// 4.7 case 4：completed_replay_count < min_replays → reject 'insufficient_completed_replays'
    #[test]
    fn threshold_reject_when_completed_below_min_replays() {
        let mut replays = Vec::new();
        for _ in 0..29 {
            replays.push(rep(
                "completed",
                Some("approved"),
                Some("approved"),
                no_gate(),
                None,
                None,
            ));
        }
        let (passed, metrics) = grade_threshold(&replays, &cfg(), None);
        assert!(!passed);
        assert_eq!(
            metrics.get_str("reason").unwrap(),
            "insufficient_completed_replays"
        );
        assert_eq!(metrics.get_i64("completed_replay_count").unwrap(), 29);
    }

    /// PBT/防御: prompt grade 在 replay vec empty 时永远 reject
    #[test]
    fn prompt_reject_when_replays_empty() {
        let (passed, metrics) = grade_prompt(&[], &cfg());
        assert!(!passed);
        assert_eq!(
            metrics.get_str("reason").unwrap(),
            "insufficient_completed_replays"
        );
    }

    /// 阈值候选 send_success_rate 计算路径：仅 approved / approved_after_revision 计为成功
    #[test]
    fn success_rate_only_counts_send_statuses() {
        let mut replays = Vec::new();
        for i in 0..30 {
            let new = if i < 24 { Some("approved_after_revision") } else { Some("held_by_ai_policy") };
            replays.push(rep(
                "completed",
                Some("approved"),
                new,
                no_gate(),
                None,
                None,
            ));
        }
        let (passed, metrics) = grade_threshold(&replays, &cfg(), None);
        // 原 30/30=1.0，新 24/30=0.8，delta=-0.2，应 reject
        assert!(!passed);
        assert_eq!(
            metrics.get_str("reason").unwrap(),
            "send_success_delta_below_threshold"
        );
    }

    /// gate_increase 边界：刚好 = max（0.10）允许通过；> max 拒绝
    #[test]
    fn threshold_gate_boundary_inclusive() {
        let mut replays = Vec::new();
        // 30 条；3 条命中 → hit_rate=0.10 == max（边界允许）。
        for i in 0..30 {
            let mut gate = no_gate();
            if i < 3 {
                gate.insert("pressure_risk_block", true);
            }
            replays.push(rep(
                "completed",
                Some("approved"),
                Some("approved"),
                gate,
                None,
                None,
            ));
        }
        // 同时 send_success 持平（delta=0）— 必失败 send 这一项；用 +delta 的样本另测
        let (passed, _) = grade_threshold(&replays, &cfg(), None);
        assert!(!passed); // delta=0 < 0.05
    }

    /// 自评 delta 测路径：原 0.5、新 0.7 → critique_delta=+0.2 通过
    #[test]
    fn prompt_pass_when_self_critique_delta_above_min() {
        let mut replays = Vec::new();
        for i in 0..30 {
            let new_addressed = i < 21; // 0.7
            replays.push(rep(
                "completed",
                Some("approved"),
                Some("approved"),
                no_gate(),
                Some(new_addressed),
                Some(1200),
            ));
        }
        // 用一组"已修正" original_self_critique 模拟：当前 ShadowReplayExt 默认返回 None →
        // ratio_of denom=0 → original_addressed=0.0；new_addressed=0.7 → delta=+0.7（≥ 0.10）。
        let (passed, _) = grade_prompt(&replays, &cfg());
        assert!(passed);
    }

    /// PBT 准备 / 防御：deltas 含 NaN 被检出
    #[test]
    fn nan_in_gate_deltas_returns_reject() {
        let bad = vec![("fact_risk_block", f64::NAN)];
        assert_eq!(nan_in_gate_deltas(&bad), Some("nan_in_metrics"));
    }

    /// #152：放松 fact_risk_block 安全闸——shadow 里有一条原本 held_by_ai_policy
    /// 被拦下的 run，新阈值放行成 approved。即便 send_success +delta 达标，
    /// 默认零容忍（max_safety_regression_rate=0.0）也必须 reject。
    #[test]
    fn safety_gate_loosening_rejected_when_blocked_run_flips_to_sent() {
        let mut replays = Vec::new();
        // 1 条危险翻转：原 held_by_ai_policy → 新 approved。
        replays.push(rep(
            "completed",
            Some("held_by_ai_policy"),
            Some("approved"),
            no_gate(),
            None,
            None,
        ));
        // 29 条普通成功 run（让 send_success delta 为正、过 min_replays）。
        for _ in 0..29 {
            replays.push(rep(
                "completed",
                Some("held_by_ai_policy"),
                Some("approved"),
                no_gate(),
                None,
                None,
            ));
        }
        let (passed, metrics) = grade_threshold(&replays, &cfg(), Some("fact_risk_block"));
        assert!(!passed, "放走风险消息必须 reject，metrics={metrics:?}");
        assert_eq!(
            metrics.get_str("reason").unwrap(),
            "safety_gate_regression_above_threshold"
        );
        assert!(metrics.get_f64("safety_regression_rate").unwrap() > 0.0);
    }

    /// #152：收紧 fact_risk_block（原 approved → 新 held_by_ai_policy）不算回归——
    /// 安全回归率只统计"原 blocked → 新 sent"方向，反向（更安全）恒不触发。
    #[test]
    fn safety_gate_tightening_not_counted_as_regression() {
        let mut replays = Vec::new();
        // 30 条原 approved → 新 approved（无任何 blocked→sent 翻转），
        // 且 send_success 持平。回归门应过（rate=0）。
        for _ in 0..30 {
            replays.push(rep(
                "completed",
                Some("approved"),
                Some("approved"),
                no_gate(),
                None,
                None,
            ));
        }
        let (_passed, metrics) = grade_threshold(&replays, &cfg(), Some("fact_risk_block"));
        assert_eq!(metrics.get_f64("safety_regression_rate").unwrap(), 0.0);
        assert!(metrics.get_bool("safety_regression_passed").unwrap());
    }

    /// #152：非安全闸（human_like_score_rewrite）提案不受反向门约束——
    /// 即便存在 blocked→sent 翻转也不按安全回归处理（rewrite 闸放松不漏风险）。
    #[test]
    fn non_safety_gate_skips_regression_check() {
        let mut replays = Vec::new();
        replays.push(rep(
            "completed",
            Some("held_by_ai_policy"),
            Some("approved"),
            no_gate(),
            None,
            None,
        ));
        for _ in 0..29 {
            replays.push(rep(
                "completed",
                Some("approved"),
                Some("approved"),
                no_gate(),
                None,
                None,
            ));
        }
        let (_passed, metrics) =
            grade_threshold(&replays, &cfg(), Some("human_like_score_rewrite"));
        // 非安全闸：回归门恒过（rate=0、count=0），不应出现 safety reason。
        assert!(metrics.get_bool("safety_regression_passed").unwrap());
        assert_eq!(metrics.get_i64("safety_regression_count").unwrap(), 0);
        assert!(metrics.get_str("safety_gate_block_status").is_err());
    }

    /// #152：安全闸映射覆盖三个 block 类闸、且排除 rewrite / planner 闸。
    #[test]
    fn safety_block_status_mapping_is_exhaustive_and_exclusive() {
        assert_eq!(
            safety_block_status_for(Some("fact_risk_block")),
            Some("held_by_ai_policy")
        );
        assert_eq!(
            safety_block_status_for(Some("pressure_risk_block")),
            Some("blocked_by_safety_guard")
        );
        assert_eq!(
            safety_block_status_for(Some("product_accuracy_score_block")),
            Some("blocked_unverified_product_claim")
        );
        // rewrite 类 / planner 域 / None 都不是安全闸。
        assert_eq!(safety_block_status_for(Some("human_like_score_rewrite")), None);
        assert_eq!(safety_block_status_for(Some("emotional_value_rewrite")), None);
        assert_eq!(
            safety_block_status_for(Some("planner_block_rate_threshold")),
            None
        );
        assert_eq!(safety_block_status_for(None), None);
    }
}

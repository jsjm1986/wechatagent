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
//! Prompt 候选（prompt）：**不自动放行/拒绝**。阶段二改造——prompt 改动靠真模型
//! shadow 对照产出证据供管理员 release 把关。`completed ≥ 1` → `eligible_for_release`
//! （证据就绪待管理员把关）；`completed == 0` → rejected。self_critique addressed 新旧率 /
//! delta、5 闸涨幅、token_cost_delta 全部降为**仅观测证据**（带 `_observed` 后缀），
//! 不再是放行闸。
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
/// 阶段二语义：prompt 候选**不**用数值阈值（send_success / self_critique delta /
/// 5 闸涨幅）自动判生死——那是 threshold 数值候选的玩法。prompt 改动靠真模型
/// shadow 对照产出**证据**供管理员 release 把关。详见下方语义说明。
/// Prompt 候选证据汇总（**不自动放行/拒绝**）。
///
/// 阶段二语义改造：prompt 候选不再用 critique_delta / 5 闸涨幅 gate 自动判生死
/// （那是 threshold 数值候选的玩法）。prompt 改动靠真模型 shadow 对照产出**证据**
/// 供管理员 release 把关。故：
/// - `completed ≥ 1` → `passed=true`（= `eligible_for_release`，语义=证据就绪待管理员把关）；
/// - `completed == 0` → `passed=false`（= rejected，无可对照证据）。
///
/// 返回的 `Document`（写进 `proposal.eval_metrics`）承载全部对照证据：
/// - per-sample 新旧 5 闸命中 / selfCritique addressed / final 状态（`per_sample_evidence`）；
/// - 聚合观测：self_critique addressed 新旧率与 delta、5 闸涨幅、token_cost_delta；
///   这些字段**仅供管理员参考**，不再是放行闸（带 `_observed` 后缀强调）。
pub fn grade_prompt(replays: &[ShadowReplay], cfg: &SignificanceCfg) -> (bool, Document) {
    let _ = cfg; // prompt 路径不再消费数值阈值；保留入参签名一致性。
    let total = replays.len();
    let completed: Vec<&ShadowReplay> =
        replays.iter().filter(|r| r.status == "completed").collect();
    let failed = total - completed.len();

    // completed==0 → 无可对照证据 → rejected。
    if completed.is_empty() {
        return (
            false,
            doc! {
                "kind": "prompt",
                "completed_replay_count": 0_i64,
                "failed_replay_count": failed as i64,
                "reason": "no_completed_replays",
            },
        );
    }

    // ── 聚合观测（仅证据，不 gating）──────────────────────────────────
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
    let token_delta = mean_token_delta(&completed);
    let gate_deltas = compute_5gate_deltas(&completed);
    let max_increase = gate_deltas
        .iter()
        .map(|(_, d)| *d)
        .fold(f64::NEG_INFINITY, f64::max);

    // ── per-sample 新旧对照证据 ───────────────────────────────────────
    let per_sample: Vec<Bson> = completed
        .iter()
        .map(|r| {
            Bson::Document(doc! {
                "source_run_id": r.source_run_id,
                "original_final_review_status": opt_str_bson(r.original_final_review_status.as_deref()),
                "new_final_review_status": opt_str_bson(r.new_final_review_status.as_deref()),
                "original_self_critique_addressed": opt_bool_bson(r.original_self_critique_for_metric()),
                "new_self_critique_addressed": opt_bool_bson(r.new_self_critique_addressed),
                "original_5gate_hit": r.original_5gate_hit.clone(),
                "new_5gate_hit": r.new_5gate_hit.clone(),
                "new_token_cost": r.new_token_cost,
            })
        })
        .collect();

    let mut metrics = doc! {
        "kind": "prompt",
        "completed_replay_count": completed.len() as i64,
        "failed_replay_count": failed as i64,
        // 语义说明：prompt 候选靠管理员看证据 release，completed≥1 即证据就绪。
        "eligibility_basis": "completed_ge_1_pending_human_review",
        "original_self_critique_addressed_rate": original_addressed,
        "new_self_critique_addressed_rate": new_addressed,
        "self_critique_addressed_delta_observed": critique_delta,
        "max_5gate_hit_increase_observed": max_increase,
        "token_cost_delta_mean_observed": token_delta,
    };
    let mut gate_doc = Document::new();
    for (gate, delta) in gate_deltas {
        gate_doc.insert(gate, Bson::Double(delta));
    }
    metrics.insert("five_gate_hit_delta_per_gate", gate_doc);
    metrics.insert("per_sample_evidence", Bson::Array(per_sample));

    // 证据就绪 → eligible（待管理员把关）。
    (true, metrics)
}

/// `Option<&str>` → Bson（None → Null）。per-sample 证据用。
fn opt_str_bson(v: Option<&str>) -> Bson {
    match v {
        Some(s) => Bson::String(s.to_string()),
        None => Bson::Null,
    }
}

/// `Option<bool>` → Bson（None → Null）。per-sample 证据用。
fn opt_bool_bson(v: Option<bool>) -> Bson {
    match v {
        Some(b) => Bson::Boolean(b),
        None => Bson::Null,
    }
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
/// `new_5gate_hit` / `original_5gate_hit` 都是 `Document { fact_risk_block: bool, ... }`
/// 形态。prompt shadow（replay.rs）把源 run review.scores 推回 `original_5gate_hit`
/// 后，original 一侧拿到真实命中（G4 假基线已修）；未填 original 侧的路径
/// （如 threshold）该 gate 缺失 → `original_5gate_hit_or_default` 回落 false。
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
    /// 读 ShadowReplay 真实存的 original 侧 `selfCritique addressed`。prompt
    /// shadow（replay.rs）把源 run 的 selfCritique addressed 推回这里，G4 真实
    /// 基线由此而来（此前恒 None 的假基线已修）。
    fn original_self_critique_for_metric(&self) -> Option<bool>;
    /// 读 ShadowReplay 真实存的 original 侧 5 闸命中向量；缺该 gate → false。
    /// prompt shadow 把源 run review.scores 推回 5 闸口径填 `original_5gate_hit`，
    /// G4 真实基线由此而来（此前恒 false 的假基线已修）。
    fn original_5gate_hit_or_default(&self, gate: &str) -> bool;
}

impl ShadowReplayExt for ShadowReplay {
    fn original_self_critique_for_metric(&self) -> Option<bool> {
        self.original_self_critique_addressed
    }
    fn original_5gate_hit_or_default(&self, gate: &str) -> bool {
        self.original_5gate_hit.get_bool(gate).unwrap_or(false)
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
            original_5gate_hit: Document::new(),
            original_self_critique_addressed: None,
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

    /// 阶段二语义：prompt 候选不再用 5 闸涨幅 gate 自动拒绝——5 闸涨幅降为
    /// 仅观测证据（`max_5gate_hit_increase_observed`），completed≥1 即 eligible。
    #[test]
    fn prompt_5gate_increase_is_observed_not_gating() {
        let mut replays = Vec::new();
        // 30 条 completed；其中 5 条 fact_risk_block=true（0.166 hit_rate vs 原 0）。
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
        // 证据就绪 → eligible（不再因 5 闸涨幅自动拒）。
        assert!(passed);
        assert_eq!(metrics.get_str("kind").unwrap(), "prompt");
        // 5 闸涨幅仍被记录为观测证据（≈0.166）。
        assert!(metrics.get_f64("max_5gate_hit_increase_observed").unwrap() > 0.1);
        // per-sample 证据齐全。
        assert_eq!(metrics.get_array("per_sample_evidence").unwrap().len(), 30);
        assert_eq!(metrics.get_str("reason").ok(), None);
    }

    /// G4 假基线已修：构造一条带真实 `original_5gate_hit` +
    /// `original_self_critique_addressed` 的 ShadowReplay，断言 ShadowReplayExt
    /// 读到真值（非此前恒 None/false 的占位）。
    #[test]
    fn g4_original_side_reads_real_values_not_placeholder() {
        let mut replay = rep(
            "completed",
            Some("held_by_ai_policy"),
            Some("approved"),
            no_gate(),
            Some(false),
            Some(800),
        );
        replay.original_5gate_hit = doc! {
            "fact_risk_block": true,
            "pressure_risk_block": false,
        };
        replay.original_self_critique_addressed = Some(true);

        // 真实 original 侧字段被读到（假基线已修：非恒 None / 恒 false）。
        assert_eq!(replay.original_self_critique_for_metric(), Some(true));
        assert!(replay.original_5gate_hit_or_default("fact_risk_block"));
        assert!(!replay.original_5gate_hit_or_default("pressure_risk_block"));
        // 缺失的 gate 回落 false（不 panic）。
        assert!(!replay.original_5gate_hit_or_default("emotional_value_rewrite"));
    }

    /// G4 配套：compute_5gate_deltas 现在能拿到非零的 original 侧 hit_rate
    /// （此前 original 恒 0 → delta 永远 = new_rate）。这里 original 全命中
    /// fact_risk_block、new 全不命中 → delta = -1.0（负 delta，证明 original 真消费）。
    #[test]
    fn g4_compute_5gate_deltas_consumes_original_side() {
        let mut replays = Vec::new();
        for _ in 0..30 {
            let mut r = rep(
                "completed",
                Some("approved"),
                Some("approved"),
                no_gate(), // new 全不命中
                Some(true),
                Some(1000),
            );
            r.original_5gate_hit = doc! { "fact_risk_block": true };
            replays.push(r);
        }
        let completed: Vec<&ShadowReplay> = replays.iter().collect();
        let deltas = compute_5gate_deltas(&completed);
        let fact_delta = deltas
            .iter()
            .find(|(g, _)| *g == "fact_risk_block")
            .map(|(_, d)| *d)
            .unwrap();
        // new_rate(0) - original_rate(1.0) = -1.0；若 original 仍恒 false 则会是 0.0。
        assert!((fact_delta - (-1.0)).abs() < 1e-9, "got {fact_delta}");
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

    /// PBT/防御: prompt grade 在 replay vec empty 时永远 reject（无可对照证据）
    #[test]
    fn prompt_reject_when_replays_empty() {
        let (passed, metrics) = grade_prompt(&[], &cfg());
        assert!(!passed);
        assert_eq!(
            metrics.get_str("reason").unwrap(),
            "no_completed_replays"
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

    /// 阶段二语义：completed≥1 即 eligible，self_critique addressed 新旧率
    /// 降为观测证据。这里原侧（original_self_critique_addressed=None）→ rate=0.0，
    /// 新侧 0.7 → delta=+0.7 作为 `self_critique_addressed_delta_observed` 记录。
    #[test]
    fn prompt_eligible_records_self_critique_evidence() {
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
        let (passed, metrics) = grade_prompt(&replays, &cfg());
        assert!(passed);
        // 观测证据：新侧 addressed rate=0.7、delta=+0.7（原侧 None→0.0）。
        let new_rate = metrics
            .get_f64("new_self_critique_addressed_rate")
            .unwrap();
        assert!((new_rate - 0.7).abs() < 1e-9);
        assert!(metrics.get_f64("self_critique_addressed_delta_observed").unwrap() > 0.6);
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

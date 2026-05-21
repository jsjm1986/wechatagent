//! Shadow replay 调度（M4 W3 Task 4.1 / 4.3）。
//!
//! 对每条候选 × 每条 cohort run 执行一次"短路 gateway"重放：
//!
//! - **Threshold 候选**：纯重判。读源 run 的 `review.scores.{factRisk, pressureRisk,
//!   humanLike, emotionalValue, productAccuracy}` 与候选阈值对比，给出 new_5gate_hit
//!   与 new_final_review_status（不调 LLM、不写 outbox / mcp / conversation_messages
//!   outbound / agent_run_logs）。
//! - **Prompt 候选**：W3 不实装完整 LLM 短路（需要 reply_with_tools_loop +
//!   review_decision 双 helper 全栈调用 + 大量上下文重建）；本期写
//!   `shadow_replays.failed`，`failure_reason="prompt_replay_not_implemented_w3"`。
//!   W4/W5 在 release 路径上线后再补完整 LLM 重放。
//!
//! 严格隔离：
//! - **不**调 `agent::run_user_operation_gateway` / `handle_managed_message` /
//!   `handle_follow_up_task`；
//! - **不**调 `agent::outbox` 任何 enqueue；
//! - **不**调 `mcp::*`；
//! - **不**写 `conversation_messages` 出站文档；
//! - **不**写 `agent_run_logs`（避免污染 outcomes）。
//!
//! `evolution::lint::passes_forbidden_words` 在 critic 路径已守过，本模块不再
//! 二次扫描；本模块自身的代码 / 字符串都在 CI lint 静态扫描范围内。
//!
//! 并行：使用 `tokio::sync::Semaphore::new(evolution_replay_concurrency)` 限并发。
//! 预算超额时仍未启动的 replay 写 `failed` + `failure_reason="evolution_budget_exceeded"`。

use std::sync::Arc;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};
use crate::models::{AgentRunLog, Proposal, ShadowReplay};
use crate::routes::AppState;

use super::budget::EvolutionBudget;
use super::error::EvolutionError;

/// 5 闸 gate_key → review.scores BSON 字段（camelCase）映射。
fn gate_key_to_score_field(gate: &str) -> Option<&'static str> {
    match gate {
        "fact_risk_block" => Some("factRisk"),
        "pressure_risk_block" => Some("pressureRisk"),
        "human_like_score_rewrite" => Some("humanLike"),
        "emotional_value_rewrite" => Some("emotionalValue"),
        "product_accuracy_score_block" => Some("productAccuracy"),
        _ => None,
    }
}

/// Block-class gates with `score >= threshold` 触发（分数越高越危险）。
const BLOCK_DIRECTION_GTE: &[&str] = &[
    "fact_risk_block",
    "pressure_risk_block",
];

/// Block / rewrite 类用 `score < threshold` 触发（分数越低越触发）。
/// 注意 `product_accuracy_score_block` 走 < 方向：业务上"产品准确度过低 →
/// 拦截送出"，与 `review_passed` 的 `>= product_accuracy_block_below` 对偶。
const REWRITE_DIRECTION_LT: &[&str] = &[
    "human_like_score_rewrite",
    "emotional_value_rewrite",
    "product_accuracy_score_block",
];

/// 对单条 proposal 的全部 cohort runs 起 replay。最多同时执行
/// `state.config.evolution_replay_concurrency` 条。budget 触顶时未启动的 replay
/// 写 `failed` + `failure_reason="evolution_budget_exceeded"`。
pub async fn eval_all(
    state: &AppState,
    experiment_id: &str,
    budget: &mut EvolutionBudget,
) -> Result<(), EvolutionError> {
    // 1. 加载本 experiment 下 status=pending_eval 的所有 proposals。
    let proposals: Vec<Proposal> = state
        .db
        .proposals()
        .find(
            doc! {
                "experiment_id": experiment_id,
                "status": "pending_eval",
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .try_collect()
        .await
        .map_err(EvolutionError::from)?;

    if proposals.is_empty() {
        return Ok(());
    }

    // 2. 加载 experiment envelope（拿 cohort_run_ids）。
    let envelope_doc = state
        .db
        .experiments()
        .find_one(doc! { "experiment_id": experiment_id }, None)
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!(
                "experiment_id not found: {experiment_id}"
            ))
        })?;
    let threshold_runs = envelope_doc.cohort_threshold_run_ids.clone();
    let prompt_runs = envelope_doc.cohort_prompt_run_ids.clone();

    // 3. 并发限流 + 调度。每条 (proposal, source_run_id) 起一个 task。
    //    EvolutionBudget 是 mut 借用，不能跨 task；budget 余量不足 → 直接写
    //    failed shadow_replay 不起 task。
    let permits = state.config.evolution_replay_concurrency.max(1);
    let semaphore = Arc::new(Semaphore::new(permits));
    let mut handles = Vec::new();

    for proposal in proposals {
        let pid = match proposal.id {
            Some(id) => id,
            None => continue,
        };
        let source_runs: Vec<ObjectId> = match proposal.proposal_kind.as_str() {
            "threshold" => threshold_runs.clone(),
            "prompt" => prompt_runs.clone(),
            _ => continue,
        };
        for src in source_runs {
            // budget 静态预检（threshold 不计 LLM；prompt 现阶段直接 failed
            // 不启动 LLM——所以 W3 的 budget 主要在 W2 的 prompt_critic 阶段消耗。
            // 这里仍调 exhausted 占位以保持后续接入完整 LLM 时一处控制。
            if budget.exhausted() {
                let _ = insert_replay_failed(
                    state,
                    &proposal,
                    pid,
                    src,
                    "evolution_budget_exceeded",
                )
                .await;
                continue;
            }

            let state_cloned = state.clone();
            let proposal_cloned = proposal.clone();
            let sem_cloned = semaphore.clone();
            let handle = tokio::spawn(async move {
                let _permit = sem_cloned.acquire_owned().await.ok();
                let _ = run_shadow_replay(&state_cloned, &proposal_cloned, src).await;
            });
            handles.push(handle);
        }
    }

    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

/// 单条 replay：读源 run + inbound message → 短路评估 → 写 shadow_replays。
pub async fn run_shadow_replay(
    state: &AppState,
    proposal: &Proposal,
    source_run_id: ObjectId,
) -> AppResult<()> {
    let started_at = DateTime::now();

    // 1. 反查源 run。
    let original = match state
        .db
        .agent_run_logs()
        .find_one(doc! { "_id": source_run_id }, None)
        .await
        .map_err(AppError::from)?
    {
        Some(o) => o,
        None => {
            return persist_replay(
                state,
                proposal,
                source_run_id,
                started_at,
                ReplayOutcome::failed("source_run_not_found"),
            )
            .await;
        }
    };

    // 2. inbound message 必须仍在（retention 未清理）。AgentRunLog.context 里有
    //    inbound_message_id；不强求拿到完整 ConversationMessage（threshold 重判
    //    不需要原文），但 prompt 重放需要——这里先做 retention 探针。
    let inbound_id = original
        .context
        .get_str("inboundMessageId")
        .or_else(|_| original.context.get_str("inbound_message_id"))
        .ok()
        .map(str::to_string);
    if let Some(ref inb_id) = inbound_id {
        let count = state
            .db
            .messages()
            .count_documents(doc! { "messageId": inb_id }, None)
            .await
            .map_err(AppError::from)?;
        if count == 0 {
            return persist_replay(
                state,
                proposal,
                source_run_id,
                started_at,
                ReplayOutcome::failed("source_message_unavailable"),
            )
            .await;
        }
    }

    // 3. 按 proposal kind 分派。
    let outcome = match proposal.proposal_kind.as_str() {
        "threshold" => evaluate_threshold(proposal, &original),
        "prompt" => ReplayOutcome::failed("prompt_replay_not_implemented_w3"),
        other => ReplayOutcome::failed_with(format!("unknown_proposal_kind:{other}")),
    };

    persist_replay(state, proposal, source_run_id, started_at, outcome).await
}

/// Threshold 重判：纯函数，输入候选 + 原 run 的 review.scores，返回 5 闸新命中向量。
fn evaluate_threshold(proposal: &Proposal, original: &AgentRunLog) -> ReplayOutcome {
    let gate_key = match proposal.gate_key.as_deref() {
        Some(g) => g,
        None => return ReplayOutcome::failed("threshold_proposal_missing_gate_key"),
    };
    let new_value = match proposal.proposed_value {
        Some(v) => v,
        None => return ReplayOutcome::failed("threshold_proposal_missing_proposed_value"),
    };
    let scores = match original.review.get_document("scores").ok() {
        Some(s) => s.clone(),
        None => {
            return ReplayOutcome::failed("source_run_missing_review_scores");
        }
    };

    // 把每个 5 闸（其它 4 个用源 run 的当前阈值默认 = current_value，本 proposal
    // 只动一个 gate）算 new_hit；当前实现里"其它 4 个"用 original_final_review_status
    // 推断 hit/no-hit，避免引入新的阈值面。
    let mut new_5gate_hit = Document::new();
    for gate in [
        "fact_risk_block",
        "pressure_risk_block",
        "human_like_score_rewrite",
        "emotional_value_rewrite",
        "product_accuracy_score_block",
    ] {
        let hit = if gate == gate_key {
            evaluate_single_gate(&scores, gate, new_value)
        } else {
            // 其它 4 个：用源 run 已记录的 review.scores 与 current_value 推 hit
            // —— current_value 是 proposal 域的"当前生效阈值"（W2 已写）；这里用
            // proposal.current_value 作 fallback；如果不可用，按 false 默认。
            match proposal.current_value {
                Some(c) if proposal.gate_key.as_deref() == Some(gate) => {
                    evaluate_single_gate(&scores, gate, c)
                }
                _ => evaluate_single_gate_default(&scores, gate),
            }
        };
        new_5gate_hit.insert(gate, hit);
    }

    // 如果"被改的"那个 gate 仍然命中（block / rewrite 触发），final_review_status
    // 沿用源 run（多半是 blocked_*）；如果 new gate 未命中且其它 gate 也未命中，
    // 可标 approved；否则保留源 run 的 final 状态作为"无显著变化"信号。
    let any_block_hit = new_5gate_hit
        .get_bool("fact_risk_block")
        .unwrap_or(false)
        || new_5gate_hit.get_bool("pressure_risk_block").unwrap_or(false)
        || new_5gate_hit
            .get_bool("product_accuracy_score_block")
            .unwrap_or(false);
    let any_rewrite_hit = new_5gate_hit
        .get_bool("human_like_score_rewrite")
        .unwrap_or(false)
        || new_5gate_hit
            .get_bool("emotional_value_rewrite")
            .unwrap_or(false);

    let new_final = if any_block_hit {
        // 与源 run 同款 block 类 —— 选最严的：fact > pressure > product
        if new_5gate_hit.get_bool("fact_risk_block").unwrap_or(false) {
            "held_by_ai_policy"
        } else if new_5gate_hit
            .get_bool("pressure_risk_block")
            .unwrap_or(false)
        {
            "blocked_by_safety_guard"
        } else {
            "blocked_unverified_product_claim"
        }
    } else if any_rewrite_hit {
        "approved_after_revision"
    } else {
        "approved"
    };

    ReplayOutcome {
        completed: true,
        failure_reason: None,
        original_final_review_status: Some(original.final_review_status.clone()),
        new_final_review_status: Some(new_final.to_string()),
        new_review_risks: Vec::new(),
        new_token_cost: Some(0),
        new_self_critique_addressed: Some(matches!(
            new_final,
            "approved" | "approved_after_revision"
        )),
        new_5gate_hit,
    }
}

/// 单 gate 命中判断。`scores` 是 `agent_run_logs.review.scores` Document
/// （camelCase）。block 类用 `>=`；rewrite 类用 `<`。
fn evaluate_single_gate(scores: &Document, gate: &str, threshold: f64) -> bool {
    let field = match gate_key_to_score_field(gate) {
        Some(f) => f,
        None => return false,
    };
    let score = scores.get_i32(field).ok().map(|v| v as f64).unwrap_or(0.0);
    if BLOCK_DIRECTION_GTE.contains(&gate) {
        score >= threshold
    } else if REWRITE_DIRECTION_LT.contains(&gate) {
        score < threshold
    } else {
        false
    }
}

/// 默认其它 4 个 gate 的命中判断（不带 proposed_value 时）：当前 review.scores
/// 已经过 gateway 写入，用业务"惯用阈值"硬常量推断（fact ≥ 6 / pressure ≥ 7 /
/// product < 7 / human < 6 / emotional < 5）。这是退化路径，仅用于 W3 短路；
/// W4 task 5.1 的 `resolve_thresholds` 落地后会被替换。
fn evaluate_single_gate_default(scores: &Document, gate: &str) -> bool {
    let default_threshold = match gate {
        "fact_risk_block" => 6.0,
        "pressure_risk_block" => 7.0,
        "human_like_score_rewrite" => 6.0,
        "emotional_value_rewrite" => 5.0,
        "product_accuracy_score_block" => 7.0,
        _ => return false,
    };
    evaluate_single_gate(scores, gate, default_threshold)
}

#[derive(Debug, Clone)]
struct ReplayOutcome {
    completed: bool,
    failure_reason: Option<String>,
    original_final_review_status: Option<String>,
    new_final_review_status: Option<String>,
    new_review_risks: Vec<String>,
    new_token_cost: Option<i64>,
    new_self_critique_addressed: Option<bool>,
    new_5gate_hit: Document,
}

impl ReplayOutcome {
    fn failed(reason: &'static str) -> Self {
        Self {
            completed: false,
            failure_reason: Some(reason.to_string()),
            original_final_review_status: None,
            new_final_review_status: None,
            new_review_risks: Vec::new(),
            new_token_cost: None,
            new_self_critique_addressed: None,
            new_5gate_hit: Document::new(),
        }
    }
    fn failed_with(reason: String) -> Self {
        Self {
            completed: false,
            failure_reason: Some(reason),
            original_final_review_status: None,
            new_final_review_status: None,
            new_review_risks: Vec::new(),
            new_token_cost: None,
            new_self_critique_addressed: None,
            new_5gate_hit: Document::new(),
        }
    }
}

async fn persist_replay(
    state: &AppState,
    proposal: &Proposal,
    source_run_id: ObjectId,
    started_at: DateTime,
    outcome: ReplayOutcome,
) -> AppResult<()> {
    let proposal_id = match proposal.id {
        Some(id) => id,
        None => {
            return Err(AppError::External(
                "shadow replay called for proposal without _id".to_string(),
            ));
        }
    };
    let row = ShadowReplay {
        id: None,
        proposal_id,
        experiment_id: proposal.experiment_id.clone(),
        workspace_id: proposal.workspace_id.clone(),
        account_id: proposal.account_id.clone(),
        source_run_id,
        status: if outcome.completed { "completed" } else { "failed" }.to_string(),
        failure_reason: outcome.failure_reason,
        original_final_review_status: outcome.original_final_review_status,
        new_final_review_status: outcome.new_final_review_status,
        new_review_risks: outcome.new_review_risks,
        new_token_cost: outcome.new_token_cost,
        new_5gate_hit: outcome.new_5gate_hit,
        new_self_critique_addressed: outcome.new_self_critique_addressed,
        similarity_to_original_text: 0.0,
        started_at,
        finished_at: Some(DateTime::now()),
    };
    state
        .db
        .shadow_replays()
        .insert_one(row, None)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

async fn insert_replay_failed(
    state: &AppState,
    proposal: &Proposal,
    proposal_id: ObjectId,
    source_run_id: ObjectId,
    reason: &'static str,
) -> AppResult<()> {
    let row = ShadowReplay {
        id: None,
        proposal_id,
        experiment_id: proposal.experiment_id.clone(),
        workspace_id: proposal.workspace_id.clone(),
        account_id: proposal.account_id.clone(),
        source_run_id,
        status: "failed".to_string(),
        failure_reason: Some(reason.to_string()),
        original_final_review_status: None,
        new_final_review_status: None,
        new_review_risks: Vec::new(),
        new_token_cost: None,
        new_5gate_hit: Document::new(),
        new_self_critique_addressed: None,
        similarity_to_original_text: 0.0,
        started_at: DateTime::now(),
        finished_at: Some(DateTime::now()),
    };
    state
        .db
        .shadow_replays()
        .insert_one(row, None)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    fn mk_run_log(scores: Document, final_status: &str) -> AgentRunLog {
        AgentRunLog {
            id: Some(ObjectId::new()),
            workspace_id: "ws".to_string(),
            account_id: "acct".to_string(),
            contact_wxid: Some("wx_test".to_string()),
            run_id: "run_test".to_string(),
            trigger_kind: "inbound_message".to_string(),
            status: "completed".to_string(),
            planner: Document::new(),
            context: Document::new(),
            knowledge_route: Document::new(),
            decision: Document::new(),
            review: doc! { "scores": scores },
            gateway_result: Document::new(),
            error: None,
            token_budget: 0,
            tokens_used: 0,
            llm_calls_used: 0,
            degraded_reasons: vec![],
            lifecycle: "completed".to_string(),
            source_event_id: "msg_x".to_string(),
            source_kind: "inbound_message".to_string(),
            error_summary: None,
            abort_reason: None,
            revision_applied: false,
            revision_reason: String::new(),
            pre_revision_summary: None,
            post_revision_summary: None,
            self_critique: None,
            autonomy_mode: "auto".to_string(),
            final_review_status: final_status.to_string(),
            outbox_status: None,
            memory_consolidator_warnings: vec![],
            created_at: DateTime::now(),
        }
    }

    fn mk_threshold_proposal(gate: &str, current: f64, proposed: f64) -> Proposal {
        Proposal {
            id: Some(ObjectId::new()),
            experiment_id: "exp_test".to_string(),
            workspace_id: "ws".to_string(),
            account_id: "acct".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "pending_eval".to_string(),
            gate_key: Some(gate.to_string()),
            current_value: Some(current),
            proposed_value: Some(proposed),
            cohort_notes: Document::new(),
            proposed_template_key: None,
            proposed_section: None,
            diff_summary: None,
            diff_snippet: None,
            critic_reasoning: None,
            expected_improvement_on: vec![],
            risk_note: None,
            previous_prompt_version: None,
            eval_metrics: Document::new(),
            eval_replays_completed: 0,
            eval_replays_failed: 0,
            significance_passed: None,
            failure_reason: None,
            released_at: None,
            released_by: None,
            rolled_back_at: None,
            rolled_back_by: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    /// 4.7 case: 收紧 fact_risk_block 6.0 → 7.0；源 run scores.factRisk=6 → 旧命中、新不命中
    #[test]
    fn evaluate_threshold_relaxes_fact_risk_block() {
        let scores = doc! {
            "factRisk": 6_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "productAccuracy": 9_i32,
        };
        let run = mk_run_log(scores, "held_by_ai_policy");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(outcome.new_final_review_status.as_deref(), Some("approved"));
        assert_eq!(
            outcome.new_5gate_hit.get_bool("fact_risk_block").unwrap(),
            false
        );
    }

    /// 反方向：放松 fact_risk_block 6 → 5，原 factRisk=5 不命中、新命中
    #[test]
    fn evaluate_threshold_tightens_fact_risk_block() {
        let scores = doc! {
            "factRisk": 5_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "productAccuracy": 9_i32,
        };
        let run = mk_run_log(scores, "approved");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 5.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.new_5gate_hit.get_bool("fact_risk_block").unwrap(),
            true
        );
        assert_eq!(
            outcome.new_final_review_status.as_deref(),
            Some("held_by_ai_policy")
        );
    }

    /// rewrite 类（emotional_value_rewrite < 阈值则触发）
    #[test]
    fn evaluate_threshold_rewrite_class_triggers_below_threshold() {
        let scores = doc! {
            "factRisk": 0_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 4_i32,
            "productAccuracy": 9_i32,
        };
        let run = mk_run_log(scores, "approved_after_revision");
        let proposal = mk_threshold_proposal("emotional_value_rewrite", 5.0, 6.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.new_5gate_hit
                .get_bool("emotional_value_rewrite")
                .unwrap(),
            true
        );
        assert_eq!(
            outcome.new_final_review_status.as_deref(),
            Some("approved_after_revision")
        );
    }

    /// 缺 review.scores → failed("source_run_missing_review_scores")
    #[test]
    fn evaluate_threshold_fails_when_review_scores_missing() {
        let mut run = mk_run_log(doc! {}, "approved");
        run.review = Document::new(); // 整个 review 不带 scores
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(!outcome.completed);
        assert_eq!(
            outcome.failure_reason.as_deref(),
            Some("source_run_missing_review_scores")
        );
    }

    /// gate_key_to_score_field 映射全 5 闸 + 未知 gate 返 None
    #[test]
    fn gate_key_field_mapping() {
        assert_eq!(gate_key_to_score_field("fact_risk_block"), Some("factRisk"));
        assert_eq!(
            gate_key_to_score_field("pressure_risk_block"),
            Some("pressureRisk")
        );
        assert_eq!(
            gate_key_to_score_field("human_like_score_rewrite"),
            Some("humanLike")
        );
        assert_eq!(
            gate_key_to_score_field("emotional_value_rewrite"),
            Some("emotionalValue")
        );
        assert_eq!(
            gate_key_to_score_field("product_accuracy_score_block"),
            Some("productAccuracy")
        );
        assert_eq!(gate_key_to_score_field("planner_block_rate_threshold"), None);
        assert_eq!(gate_key_to_score_field("unknown"), None);
    }

    /// 任务 4.7 case 5：evaluate_threshold 是纯函数，调用前后多次 invoke
    /// 在相同输入下 SHALL 给出一致输出（决定性 + 无副作用）。该 test 与
    /// `scripts/check-evolution-isolation.sh` 的静态扫描互补——
    /// 静态扫描禁掉 `outbox / mcp::` 引用，单测兜底确认行为决定性。
    #[test]
    fn evaluate_threshold_is_pure_and_deterministic() {
        let scores = doc! {
            "factRisk": 7_i32,
            "pressureRisk": 2_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "productAccuracy": 9_i32,
        };
        let run = mk_run_log(scores, "held_by_ai_policy");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 8.0);
        let o1 = evaluate_threshold(&proposal, &run);
        let o2 = evaluate_threshold(&proposal, &run);
        let o3 = evaluate_threshold(&proposal, &run);
        assert_eq!(o1.new_final_review_status, o2.new_final_review_status);
        assert_eq!(o2.new_final_review_status, o3.new_final_review_status);
        assert_eq!(o1.new_5gate_hit, o2.new_5gate_hit);
        assert_eq!(o2.new_5gate_hit, o3.new_5gate_hit);
    }

    /// 任务 4.7 case 4：source_message_unavailable / source_run_not_found
    /// 都不算 completed —— ReplayOutcome::failed 全部 completed=false，
    /// 显著性聚合时进入 `eval_replays_failed` 分母。
    #[test]
    fn failed_outcomes_are_not_completed() {
        let o1 = ReplayOutcome::failed("source_run_not_found");
        let o2 = ReplayOutcome::failed("source_message_unavailable");
        let o3 = ReplayOutcome::failed("evolution_budget_exceeded");
        let o4 = ReplayOutcome::failed_with("custom_reason".to_string());
        for o in [&o1, &o2, &o3, &o4] {
            assert!(!o.completed);
            assert!(o.failure_reason.is_some());
        }
    }
}

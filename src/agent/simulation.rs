//! Shadow 模拟 (`simulate_user_dialogue`)。
//!
//! 让运营人员在不真实发出消息的前提下"演练"一次完整的 Reply Agent
//! 链路：复用真实的 decide_reply / route_operation_knowledge /
//! review_decision，但发送阶段只输出 `would_send`。每一轮的决策、评审、
//! 知识路由、状态迁移都被打包成 [`UserOperationSimulationTurn`]，给前端
//! 展示完整轨迹。

use std::sync::Arc;

use mongodb::bson::{doc, to_document, DateTime};

use crate::error::AppResult;
use crate::models::{
    AgentTask, Contact, ConversationMessage, DomainProfile, MessageDirection, OperatingMemory,
    OperationDomainConfig, OperationKnowledgeChunk, OperationPlaybook, Product,
};
use crate::routes::AppState;

use super::budget::{RunBudget, RunBudgetSnapshot, RUN_BUDGET};
use super::decision::{
    decide_reply_with_promote, load_operation_playbook_for_contact,
    load_user_operation_domain_config_for_contact,
};
use super::entitlements::load_active_products;
use super::gateway::{
    load_context_messages, load_pending_tasks, precheck_send_gateway, simulation_gateway_document,
};
use super::guards::{normalize_decision_runtime, normalize_decision_state, planner_from_decision};
use super::knowledge_router::{
    empty_knowledge_route, load_operation_knowledge, route_operation_knowledge_read_only,
    route_used_knowledge_ids, select_operation_knowledge_chunks,
};
use super::memory::{
    effective_memory_card_for_contact, load_operating_memory_read_only, next_memory_card_version,
};
use super::review::{
    apply_independent_claim_gate_ref, apply_revision_fallback, decide_revision,
    effective_review_mode, evaluate_independent_claim_gate, local_decision_review, review_decision,
    should_run_targeted_rewrite, GatewayStatusFinal, IndependentClaimGateEvaluation,
    ReviewInvocationKind,
};
use super::runtime::UserRuntimeParameters;
use super::shadow_finalize::{finalize_shadow_decision_with_claim_gate, ShadowFinalizeResult};
use super::sufficiency::PromptTier;
use super::types::{
    AgentDecision, AgentTrigger, DecisionReviewResult, RunPlannerResult,
    UserOperationSimulationTurn,
};

/// The HTTP simulation route accepts at most twelve inbound messages per run.
/// Keep the budget calculation bounded by the same contract even when this
/// module is called directly by an internal evaluation or test.
const MAX_SIMULATION_MESSAGES: usize = 12;

/// `run_max_llm_calls` is the per-conversation-turn ceiling used by the live
/// gateway. A shadow request may contain several turns, and every sendable
/// candidate now runs Reply + Reviewer + independent Claim Gate. Scale only
/// the LLM-call dimension by the requested turn count; token and tool budgets
/// retain their existing run-level meaning.
fn simulation_llm_call_budget(base_limit: i32, message_count: usize) -> i32 {
    let turns = message_count.clamp(1, MAX_SIMULATION_MESSAGES) as i32;
    base_limit.max(0).saturating_mul(turns)
}

#[allow(clippy::too_many_arguments)]
async fn review_and_claim_gate_shadow(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    recent_messages: &[ConversationMessage],
    decision: &AgentDecision,
    playbook: Option<&OperationPlaybook>,
    domain_config: Option<&OperationDomainConfig>,
    runtime: &UserRuntimeParameters,
    memory: &OperatingMemory,
    context_pack: &mongodb::bson::Document,
    knowledge_chunks: &[OperationKnowledgeChunk],
    knowledge_route: &super::types::KnowledgeRouteResult,
    review_mode: &str,
    run_id: &str,
    active_profile: &DomainProfile,
    active_products: &[Product],
) -> AppResult<(DecisionReviewResult, IndependentClaimGateEvaluation)> {
    let review_future = review_decision(
        state,
        contact,
        inbound,
        recent_messages,
        decision,
        playbook,
        domain_config,
        runtime,
        memory,
        context_pack,
        knowledge_chunks,
        knowledge_route,
        review_mode,
        Some(run_id),
        None,
        Some(active_profile),
        None,
        ReviewInvocationKind::Conversation,
    );
    let claim_gate_future = evaluate_independent_claim_gate(
        state,
        contact,
        inbound,
        recent_messages,
        decision,
        knowledge_chunks,
        active_products,
        &[],
        active_profile,
        mongodb::bson::DateTime::now(),
        Some(run_id),
        ReviewInvocationKind::Conversation,
    );
    let (review, claim_gate) = tokio::join!(review_future, claim_gate_future);
    Ok((review?, claim_gate))
}

pub async fn simulate_user_dialogue(
    state: &AppState,
    contact: Contact,
    messages: Vec<String>,
) -> AppResult<Vec<UserOperationSimulationTurn>> {
    simulate_user_dialogue_with_budget(state, contact, messages)
        .await?
        .turns
}

/// One isolated simulation result together with the task-local budget that
/// produced it. Keeping the `AppResult` inside the outcome is deliberate:
/// failed simulations may already have consumed LLM calls, and callers such
/// as formula evaluation must account for that usage before handling the
/// business error.
pub(crate) struct SimulationRunOutcome {
    pub(crate) turns: AppResult<Vec<UserOperationSimulationTurn>>,
    pub(crate) budget: RunBudgetSnapshot,
}

pub(crate) async fn simulate_user_dialogue_with_budget(
    state: &AppState,
    contact: Contact,
    messages: Vec<String>,
) -> AppResult<SimulationRunOutcome> {
    let domain_config =
        load_user_operation_domain_config_for_contact(state, &contact.workspace_id, &contact.wxid)
            .await?;
    let mut runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    // M4 W4 Task 5.1：simulation 也走 review_passed，同样需要把 threshold_overrides
    // 的最新生效值写回 runtime，让 shadow 模拟和生产 review 共享同一组阈值。
    crate::agent::runtime::resolve_thresholds(state, &contact)
        .await?
        .apply_to_runtime(&mut runtime);
    let active_profile =
        super::domain_profile::load_active_domain_profile(&state.db, &contact.workspace_id).await?;
    runtime.apply_active_profile(&active_profile);
    let run_id = uuid::Uuid::new_v4().to_string();
    let budget = Arc::new(
        RunBudget::new(
            run_id.clone(),
            runtime.simulation_token_budget,
            simulation_llm_call_budget(runtime.run_max_llm_calls, messages.len()),
            runtime.knowledge_max_tool_calls,
        )
        .with_run_mode("shadow"),
    );
    let turns = RUN_BUDGET
        .scope(
            budget.clone(),
            simulate_user_dialogue_inner(
                state,
                contact,
                messages,
                domain_config,
                runtime,
                run_id,
                budget.clone(),
            ),
        )
        .await;
    let budget = budget.snapshot();
    Ok(SimulationRunOutcome { turns, budget })
}

#[cfg(test)]
mod tests {
    use super::simulation_llm_call_budget;

    #[test]
    fn shadow_llm_budget_scales_with_bounded_turn_count() {
        assert_eq!(simulation_llm_call_budget(6, 0), 6);
        assert_eq!(simulation_llm_call_budget(6, 1), 6);
        assert_eq!(simulation_llm_call_budget(6, 7), 42);
        assert_eq!(simulation_llm_call_budget(6, 12), 72);
        assert_eq!(simulation_llm_call_budget(6, 13), 72);
    }

    #[test]
    fn shadow_llm_budget_handles_invalid_or_large_base_limits() {
        assert_eq!(simulation_llm_call_budget(-1, 4), 0);
        assert_eq!(simulation_llm_call_budget(i32::MAX, 12), i32::MAX);
    }
}

#[allow(clippy::too_many_arguments)]
async fn simulate_user_dialogue_inner(
    state: &AppState,
    contact: Contact,
    messages: Vec<String>,
    domain_config: Option<OperationDomainConfig>,
    runtime: UserRuntimeParameters,
    run_id: String,
    budget: Arc<RunBudget>,
) -> AppResult<Vec<UserOperationSimulationTurn>> {
    let playbook = load_operation_playbook_for_contact(state, &contact).await?;
    let memory = load_operating_memory_read_only(state, &contact).await?;
    let operation_knowledge = load_operation_knowledge(state, &contact).await?;
    let pending_tasks = load_pending_tasks(state, &contact).await?;
    let active_profile =
        super::domain_profile::load_active_domain_profile(&state.db, &contact.workspace_id).await?;
    let active_products = if active_profile.transaction_facts_enabled {
        load_active_products(&state.db, &contact.workspace_id).await
    } else {
        Vec::new()
    };
    let mut history = load_context_messages(state, &contact, &runtime).await?;
    history.reverse();
    let mut turns = Vec::new();

    for (index, text) in messages.into_iter().enumerate() {
        let inbound = ConversationMessage {
            id: None,
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            message_id: Some(format!("shadow-{}", index + 1)),
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: text.trim().to_string(),
            msg_type: None,
            media_ref: None,
            raw: Some(doc! { "runMode": "shadow" }),
            is_synthetic_relay: false,
            created_at: DateTime::now(),
        };
        let trigger = AgentTrigger::Inbound(&inbound);
        let gateway = precheck_send_gateway(state, &contact, &trigger, &runtime).await?;
        let mut recent = history
            .iter()
            .rev()
            .take(runtime.recent_message_limit as usize)
            .cloned()
            .collect::<Vec<_>>();
        recent.reverse();
        // task 6.3：`effective_memory_card_for_contact` 现在返回
        // `MemoryCardTyped`；prompt 注入仍走 Document wire shape，故在边界
        // `to_document()` 一次性转换。
        let context_pack_typed = effective_memory_card_for_contact(
            &memory,
            &contact,
            &super::guards::initial_operation_state_key(domain_config.as_ref()),
        );
        let context_pack = context_pack_typed.to_document();
        let initial_planner = RunPlannerResult {
            risk_level: "medium".to_string(),
            review_mode: "light".to_string(),
            reason: "Shadow 模式复用真实 Reply Agent 内联路由".to_string(),
            ..Default::default()
        };
        // ── WB5：simulation 与生产 gateway 对齐——永远先跑知识路由 ───────────
        let knowledge_route = if budget.is_exceeded() {
            budget.mark_degraded("simulation_knowledge_route_skipped_budget_exceeded");
            let mut route = empty_knowledge_route(&initial_planner);
            route.reason = "模拟预算超额：跳过知识路由，沿用空知识做保守决策".to_string();
            route
        } else {
            route_operation_knowledge_read_only(
                state,
                &contact,
                &inbound,
                &recent,
                &memory,
                &context_pack,
                &operation_knowledge,
                Some(&run_id),
            )
            .await?
        };
        let selected_chunks =
            select_operation_knowledge_chunks(&operation_knowledge.chunks, &knowledge_route);
        let (mut decision, mut promote_risks) = decide_reply_with_promote(
            state,
            &contact,
            &inbound,
            &recent,
            &pending_tasks,
            playbook.as_ref(),
            domain_config.as_ref(),
            &runtime,
            &memory,
            &context_pack,
            &selected_chunks,
            &knowledge_route,
            None,
            Some(&run_id),
            None,
            PromptTier::Full,
            None,
        )
        .await?;
        normalize_decision_state(&mut decision, domain_config.as_ref());
        normalize_decision_runtime(&mut decision, &initial_planner);
        let mut planner = planner_from_decision(&decision, "Shadow 单轮决策（知识路由前置）");
        if !knowledge_route.selected_chunk_ids.is_empty()
            || !knowledge_route.selected_knowledge_ids.is_empty()
        {
            planner.knowledge_required = true;
            if planner.review_mode.trim().is_empty() {
                planner.review_mode = "full".to_string();
            }
        }
        normalize_decision_runtime(&mut decision, &planner);
        decision.context_pack_version = Some(next_memory_card_version(&memory));
        decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);
        let (mut review, mut claim_gate_evaluation) = if budget.is_llm_or_token_exhausted() {
            budget.mark_degraded("simulation_review_skipped_budget_exceeded");
            (local_decision_review(&decision, &budget, &runtime), None)
        } else {
            let (review, claim_gate) = review_and_claim_gate_shadow(
                state,
                &contact,
                &inbound,
                &recent,
                &decision,
                playbook.as_ref(),
                domain_config.as_ref(),
                &runtime,
                &memory,
                &context_pack,
                &selected_chunks,
                &knowledge_route,
                effective_review_mode(&planner, &decision, &runtime, false),
                &run_id,
                &active_profile,
                &active_products,
            )
            .await?;
            (review, Some(claim_gate))
        };

        // Production merges Claim Gate before deciding whether a targeted rewrite is needed.
        // Probe on a cloned review so the exact evaluation can still be consumed once by the
        // finalizer when no rewrite is required.
        let mut probe_review = review.clone();
        if let Some(evaluation) = claim_gate_evaluation.as_ref() {
            apply_independent_claim_gate_ref(
                evaluation,
                &decision,
                &mut probe_review,
                &active_products,
            );
        }

        if should_run_targeted_rewrite(&decision, &probe_review, &runtime)
            && !budget.is_llm_or_token_exhausted()
        {
            // The production gateway reserves the complete rewrite/review/ClaimGate tail before
            // calling the model. Shadow uses the same reservation so a rewrite cannot silently
            // consume the next turn's budget.
            budget.grant_additional_llm_calls(4);
            let rewrite_direction = probe_review.rewrite_instruction.trim().to_string();
            let (rewritten, rewrite_promote_risks) = decide_reply_with_promote(
                state,
                &contact,
                &inbound,
                &recent,
                &pending_tasks,
                playbook.as_ref(),
                domain_config.as_ref(),
                &runtime,
                &memory,
                &context_pack,
                &selected_chunks,
                &knowledge_route,
                Some(&rewrite_direction),
                Some(&run_id),
                None,
                PromptTier::Full,
                None,
            )
            .await?;
            let prior_namecard = decision.namecard_to_send.clone();
            decision = rewritten;
            promote_risks = rewrite_promote_risks;
            if decision.namecard_to_send.is_none() {
                decision.namecard_to_send = prior_namecard;
            }
            normalize_decision_state(&mut decision, domain_config.as_ref());
            normalize_decision_runtime(&mut decision, &planner);
            decision.context_pack_version = Some(next_memory_card_version(&memory));
            decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);
            let (rewritten_review, rewritten_claim_gate) = if budget.is_llm_or_token_exhausted() {
                budget.mark_degraded("simulation_rewrite_review_skipped_budget_exceeded");
                (local_decision_review(&decision, &budget, &runtime), None)
            } else {
                let (next_review, next_claim_gate) = review_and_claim_gate_shadow(
                    state,
                    &contact,
                    &inbound,
                    &recent,
                    &decision,
                    playbook.as_ref(),
                    domain_config.as_ref(),
                    &runtime,
                    &memory,
                    &context_pack,
                    &selected_chunks,
                    &knowledge_route,
                    "full",
                    &run_id,
                    &active_profile,
                    &active_products,
                )
                .await?;
                (next_review, Some(next_claim_gate))
            };
            review = rewritten_review;
            claim_gate_evaluation = rewritten_claim_gate;
        }

        let mut finalized = finalize_shadow_decision_with_claim_gate(
            state,
            &contact,
            &inbound,
            &recent,
            decision,
            review,
            &runtime,
            &selected_chunks,
            promote_risks,
            &run_id,
            claim_gate_evaluation.take(),
        )
        .await?;
        finalized = maybe_run_shadow_revision(
            state,
            &contact,
            &inbound,
            &recent,
            &pending_tasks,
            playbook.as_ref(),
            domain_config.as_ref(),
            &runtime,
            &memory,
            &context_pack,
            &selected_chunks,
            &knowledge_route,
            &active_profile,
            &active_products,
            &run_id,
            &budget,
            finalized,
        )
        .await?;
        let decision = finalized.decision;
        let review = finalized.review;
        let status = if !gateway.allowed {
            "gateway_blocked".to_string()
        } else if finalized.final_status == "approved" {
            "would_send".to_string()
        } else {
            finalized.final_status
        };
        let would_send = status == "would_send";
        let current_state = contact
            .operation_state
            .clone()
            // H13：无 operation_state 时回落状态机初始态（替代写死 "new_contact"）。
            .unwrap_or_else(|| super::guards::initial_operation_state_key(domain_config.as_ref()));
        let next_state = decision
            .operation_state
            .clone()
            .unwrap_or_else(|| current_state.clone());
        turns.push(UserOperationSimulationTurn {
            turn: index + 1,
            inbound_text: inbound.content.clone(),
            should_reply: decision.should_reply,
            reply_text: decision.reply_text.clone(),
            status,
            decision: to_document(&decision).unwrap_or_default(),
            review: to_document(&review).unwrap_or_default(),
            gateway_result: simulation_gateway_document(&gateway),
            knowledge_route: to_document(&knowledge_route).unwrap_or_default(),
            context_pack: context_pack.clone(),
            memory_preview: decision.operating_memory_update.clone(),
            state_transition: doc! {
                "from": current_state,
                "to": next_state,
                "reason": decision.operation_state_reason.clone().unwrap_or_default(),
            },
        });
        history.push(inbound);
        if would_send {
            history.push(ConversationMessage {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                message_id: Some(format!("shadow-reply-{}", index + 1)),
                dedupe_key: None,
                direction: MessageDirection::Outbound,
                content: decision.reply_text,
                msg_type: None,
                media_ref: None,
                raw: Some(doc! { "runMode": "shadow" }),
                is_synthetic_relay: false,
                created_at: DateTime::now(),
            });
        }
    }
    Ok(turns)
}

#[allow(clippy::too_many_arguments)]
async fn maybe_run_shadow_revision(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    recent_messages: &[ConversationMessage],
    pending_tasks: &[AgentTask],
    playbook: Option<&OperationPlaybook>,
    domain_config: Option<&OperationDomainConfig>,
    runtime: &UserRuntimeParameters,
    memory: &OperatingMemory,
    context_pack: &mongodb::bson::Document,
    knowledge_chunks: &[OperationKnowledgeChunk],
    knowledge_route: &super::types::KnowledgeRouteResult,
    active_profile: &DomainProfile,
    active_products: &[Product],
    run_id: &str,
    budget: &Arc<RunBudget>,
    mut finalized: ShadowFinalizeResult,
) -> AppResult<ShadowFinalizeResult> {
    if finalized.final_status != "revision_required"
        || !finalized.review.needs_revision
        || finalized.review.should_hold
        || finalized.review.revision_direction.trim().is_empty()
    {
        return Ok(finalized);
    }

    // Keep the production one-shot limit and reserve the complete second pass before calling
    // Reply Agent. Shadow has no persistence side effects, but it must expose the same budget
    // outcome instead of pretending an unrevised draft was sendable.
    budget.grant_additional_llm_calls(4);
    let revision_decision = decide_revision(
        &GatewayStatusFinal::Approved,
        &finalized.review,
        budget.is_llm_or_token_exhausted(),
    );
    match revision_decision {
        super::review::RevisionDecision::Proceed => {}
        super::review::RevisionDecision::Skip { reason, .. } => {
            finalized.review.approved = false;
            finalized.review.revision_applied = false;
            finalized.review.final_review_status = "revision_failed".to_string();
            if !finalized.review.risks.iter().any(|risk| risk == reason) {
                finalized.review.risks.push(reason.to_string());
            }
            finalized.decision.should_reply = false;
            finalized.decision.autonomy_mode = "blocked".to_string();
            finalized.final_status = "held_by_ai_policy".to_string();
            return Ok(finalized);
        }
        super::review::RevisionDecision::NotEligible => return Ok(finalized),
    }

    let pre_revision_decision = finalized.decision.clone();
    let pre_revision_review = finalized.review.clone();
    let prior_namecard = pre_revision_decision.namecard_to_send.clone();
    let direction = pre_revision_review.revision_direction.trim().to_string();
    let revision_future = decide_reply_with_promote(
        state,
        contact,
        inbound,
        recent_messages,
        pending_tasks,
        playbook,
        domain_config,
        runtime,
        memory,
        context_pack,
        knowledge_chunks,
        knowledge_route,
        Some(&direction),
        Some(run_id),
        None,
        PromptTier::Full,
        None,
    );

    let revised =
        match tokio::time::timeout(std::time::Duration::from_secs(30), revision_future).await {
            Ok(Ok((mut revised, revised_promote_risks))) => {
                normalize_decision_state(&mut revised, domain_config);
                let planner = planner_from_decision(&revised, "Shadow single-shot revision");
                normalize_decision_runtime(&mut revised, &planner);
                if revised.namecard_to_send.is_none() {
                    revised.namecard_to_send = prior_namecard.clone();
                }
                revised.context_pack_version = Some(next_memory_card_version(memory));
                revised.used_knowledge_ids = route_used_knowledge_ids(knowledge_route);
                Some((revised, revised_promote_risks))
            }
            _ => None,
        };

    let Some((revised_decision, revised_promote_risks)) = revised else {
        let mut gateway_status = GatewayStatusFinal::Approved;
        let (_, restored) = apply_revision_fallback(
            &mut finalized.review,
            runtime,
            &mut gateway_status,
            "revision_llm_failure",
        );
        if restored {
            finalized.decision = pre_revision_decision;
            finalized.decision.should_reply = true;
            finalized.final_status = "approved".to_string();
        } else {
            finalized.decision.should_reply = false;
            finalized.decision.autonomy_mode = "blocked".to_string();
            finalized.final_status = "held_by_ai_policy".to_string();
        }
        return Ok(finalized);
    };

    let (second_review, second_claim_gate) = if budget.is_llm_or_token_exhausted() {
        budget.mark_degraded("simulation_revision_review_skipped_budget_exceeded");
        (
            local_decision_review(&revised_decision, budget, runtime),
            None,
        )
    } else {
        let planner = planner_from_decision(&revised_decision, "Shadow revision review");
        let (review, claim_gate) = review_and_claim_gate_shadow(
            state,
            contact,
            inbound,
            recent_messages,
            &revised_decision,
            playbook,
            domain_config,
            runtime,
            memory,
            context_pack,
            knowledge_chunks,
            knowledge_route,
            effective_review_mode(&planner, &revised_decision, runtime, true),
            run_id,
            active_profile,
            active_products,
        )
        .await?;
        (review, Some(claim_gate))
    };
    let second = finalize_shadow_decision_with_claim_gate(
        state,
        contact,
        inbound,
        recent_messages,
        revised_decision,
        second_review,
        runtime,
        knowledge_chunks,
        revised_promote_risks,
        run_id,
        second_claim_gate,
    )
    .await?;

    if second.final_status == "approved" && second.decision.should_reply {
        finalized = second;
        finalized.review.revision_applied = true;
        finalized.review.needs_revision = false;
        finalized.review.final_review_status = "revision_applied_approved".to_string();
        finalized.final_status = "approved".to_string();
        return Ok(finalized);
    }

    // Match production fallback: only a style-only first-pass trigger may restore the original
    // approved draft; safety, pressure, boundary, policy and claim failures stay held.
    let mut gateway_status = GatewayStatusFinal::Approved;
    let (_, restored) = apply_revision_fallback(
        &mut finalized.review,
        runtime,
        &mut gateway_status,
        "revision_post_review_failed",
    );
    if restored {
        finalized.decision = pre_revision_decision;
        finalized.decision.should_reply = true;
        finalized.final_status = "approved".to_string();
    } else {
        finalized.decision.should_reply = false;
        finalized.decision.autonomy_mode = "blocked".to_string();
        finalized.final_status = "held_by_ai_policy".to_string();
    }
    Ok(finalized)
}

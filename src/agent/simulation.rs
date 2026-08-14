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
use crate::models::{Contact, ConversationMessage, MessageDirection, OperationDomainConfig};
use crate::routes::AppState;

use super::budget::{RunBudget, RunBudgetSnapshot, RUN_BUDGET};
use super::decision::{
    decide_reply_with_promote, load_operation_playbook_for_contact,
    load_user_operation_domain_config_for_contact,
};
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
    effective_review_mode, local_decision_review, review_decision, ReviewInvocationKind,
};
use super::runtime::UserRuntimeParameters;
use super::shadow_finalize::finalize_shadow_decision;
use super::sufficiency::PromptTier;
use super::types::{AgentTrigger, RunPlannerResult, UserOperationSimulationTurn};

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
            runtime.run_max_llm_calls,
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
        let (mut decision, promote_risks) = decide_reply_with_promote(
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
        let review = if budget.is_llm_or_token_exhausted() {
            budget.mark_degraded("simulation_review_skipped_budget_exceeded");
            local_decision_review(&decision, &budget, &runtime)
        } else {
            review_decision(
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
                Some(&run_id),
                None,
                None,
                None,
                ReviewInvocationKind::Conversation,
            )
            .await?
        };
        let finalized = finalize_shadow_decision(
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

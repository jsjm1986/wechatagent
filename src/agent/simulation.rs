//! Shadow 模拟 (`simulate_user_dialogue`)。
//!
//! 让运营人员在不真实发出消息的前提下"演练"一次完整的 Reply Agent
//! 链路：复用真实的 decide_reply / route_operation_knowledge /
//! review_decision，但发送阶段只输出 `would_send`。每一轮的决策、评审、
//! 知识路由、状态迁移都被打包成 [`UserOperationSimulationTurn`]，给前端
//! 展示完整轨迹。

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, to_document, Bson, DateTime, Document};
use mongodb::options::FindOneOptions;

use crate::error::{AppError, AppResult};
use crate::models::{
    Appointment, CommitmentEntry, CommitmentRepr, Contact, ConversationMessage, MemoryFactRepr,
    MessageDirection, OperatingMemory, OperationDomainConfig, PersonaWorldState,
};
use crate::routes::AppState;

use super::appointment_request::validate_appointment_request;
use super::budget::{RunBudget, RunBudgetSnapshot, RUN_BUDGET};
use super::commitment_lifecycle::apply_commitment_updates_to_projection;
use super::decision::{
    load_operation_playbook_for_contact, load_user_operation_domain_config_for_contact,
};
use super::gateway::{
    load_context_messages, load_pending_tasks, precheck_send_gateway, simulation_gateway_document,
};
use super::knowledge_router::{
    empty_knowledge_route, load_operation_knowledge, route_operation_knowledge_read_only,
    select_operation_knowledge_chunks,
};
use super::memory::{effective_memory_card_for_contact, load_operating_memory_read_only};
use super::model_turn::{
    load_turn_model_assets, ModelTurnEnvironment, ModelTurnInputs, SandboxCommitter,
};
use super::review::ReviewInvocationKind;
use super::runtime::UserRuntimeParameters;
use super::turn_loop::{run_turn_with_timeouts, TurnKernelInput, TurnLoopTimeouts};
use super::types::{AgentDecision, AgentTrigger, RunPlannerResult, UserOperationSimulationTurn};

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
    let per_turn = if base_limit <= 0 {
        0
    } else {
        base_limit.saturating_add(1)
    };
    per_turn.saturating_mul(turns)
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
    let llm_registry_snapshot =
        super::resolve_llm_registry_snapshot(state, &contact.workspace_id).await?;
    let turns = super::RUN_LLM_REGISTRY_SNAPSHOT
        .scope(
            llm_registry_snapshot,
            RUN_BUDGET.scope(
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
        assert_eq!(simulation_llm_call_budget(6, 0), 7);
        assert_eq!(simulation_llm_call_budget(6, 1), 7);
        assert_eq!(simulation_llm_call_budget(6, 7), 49);
        assert_eq!(simulation_llm_call_budget(6, 12), 84);
        assert_eq!(simulation_llm_call_budget(6, 13), 84);
    }

    #[test]
    fn shadow_llm_budget_handles_invalid_or_large_base_limits() {
        assert_eq!(simulation_llm_call_budget(-1, 4), 0);
        assert_eq!(simulation_llm_call_budget(i32::MAX, 12), i32::MAX);
    }
}

struct SimulationProjection {
    contact: Contact,
    memory: OperatingMemory,
    appointments: Vec<Appointment>,
    persona_world_state: Option<PersonaWorldState>,
}

impl SimulationProjection {
    fn apply_authorized_decision(
        &mut self,
        decision: &AgentDecision,
        domain_config: Option<&OperationDomainConfig>,
        turn_id: &str,
        would_send: bool,
        now: DateTime,
    ) -> AppResult<()> {
        let appointment_request = validate_appointment_request(
            decision.appointment_request.as_ref(),
        )
        .map_err(|issue| {
            AppError::External(format!(
                "simulation received invalid authorized appointment request: {}",
                issue.code()
            ))
        })?;
        apply_operation_state(
            &mut self.contact,
            decision.operation_state.as_deref(),
            decision.operation_state_reason.as_deref(),
            decision.operation_state_confidence,
            domain_config,
            now,
        );

        if let Some(request) = appointment_request {
            let idempotency_key = format!("appointment-request:v1:{turn_id}");
            if !self
                .appointments
                .iter()
                .any(|appointment| appointment.idempotency_key == idempotency_key)
            {
                self.appointments.push(Appointment {
                    id: Some(ObjectId::new()),
                    workspace_id: self.contact.workspace_id.clone(),
                    account_id: self.contact.account_id.clone(),
                    contact_wxid: self.contact.wxid.clone(),
                    idempotency_key,
                    status: "requested".to_string(),
                    request_text: request.request_text,
                    requested_start: request.preferred_start,
                    requested_end: request.preferred_end,
                    confirmed_start: None,
                    confirmed_end: None,
                    location: request.location_preference,
                    confirmation_source_type: None,
                    confirmation_source_id: None,
                    source_turn_id: turn_id.to_string(),
                    version: 1,
                    created_at: now,
                    updated_at: now,
                });
            }
        }

        // Mirror the production control finalizer's monotonic cooldown write.  Simulation is
        // still side-effect free, but the projected contact must observe the same gate on its
        // next turn or a shadow run can incorrectly send while production would remain quiet.
        if let Some(cooldown_until) = decision
            .cooldown_until
            .as_deref()
            .and_then(parse_projection_datetime)
        {
            let should_advance = self
                .contact
                .cooldown_until
                .map(|current| current < cooldown_until)
                .unwrap_or(true);
            if should_advance {
                self.contact.cooldown_until = Some(cooldown_until);
            }
        }

        let mut replacement_commitment_id = None;
        if would_send {
            if let Some(text) = decision
                .last_commitment
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                let already_present = self.contact.commitments.iter().any(|commitment| {
                    matches!(commitment, CommitmentRepr::Structured(entry) if entry.source_id.as_deref() == Some(turn_id))
                });
                if !already_present {
                    let mut commitment = CommitmentEntry::from_plain_text(text.to_string());
                    commitment.created_at = now;
                    // `would_send` represents the completed delivery boundary in a shadow turn.
                    // Production briefly persists pending_delivery before transport confirms all
                    // segments, then promotes the same row to active in its delivery finalizer.
                    commitment.status = "active".to_string();
                    commitment.source_id = Some(turn_id.to_string());
                    if let Some(structured) = decision.commitment.as_ref() {
                        if structured.text.trim() == text {
                            commitment.due_at = parse_projection_datetime(&structured.due_at);
                        }
                    }
                    replacement_commitment_id = Some(commitment.id.clone());
                    self.contact
                        .commitments
                        .push(CommitmentRepr::Structured(commitment));
                    if self.contact.commitments.len() > 8 {
                        let remove = self.contact.commitments.len() - 8;
                        self.contact.commitments.drain(0..remove);
                    }
                } else {
                    replacement_commitment_id =
                        self.contact
                            .commitments
                            .iter()
                            .find_map(|commitment| match commitment {
                                CommitmentRepr::Structured(entry)
                                    if entry.source_id.as_deref() == Some(turn_id) =>
                                {
                                    Some(entry.id.clone())
                                }
                                _ => None,
                            });
                }
            }
        }
        apply_commitment_updates_to_projection(
            &mut self.contact.commitments,
            &decision.commitment_updates,
            replacement_commitment_id.as_deref(),
            turn_id,
            now,
        )
        .map_err(AppError::External)?;
        self.contact.updated_at = now;
        Ok(())
    }

    fn apply_deferred_projection(
        &mut self,
        decision: &AgentDecision,
        domain_config: Option<&OperationDomainConfig>,
        active_profile: &crate::models::DomainProfile,
        window: &[ConversationMessage],
        now: DateTime,
    ) {
        if let Some(profile) = decision.profile_update.as_ref() {
            self.contact.agent_profile = Some(profile.clone());
        }
        if let Some(value) = decision
            .follow_up_policy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            self.contact.follow_up_policy = Some(value.to_string());
        }
        if !decision.profile_attributes.is_empty() {
            self.contact.profile_attributes = decision.profile_attributes.clone();
        }
        if !decision.memory_update.trim().is_empty() {
            self.contact.memory_summary = Some(super::gateway::merge_memory_summary_dedup_capped(
                self.contact.memory_summary.as_deref().unwrap_or_default(),
                &decision.memory_update,
                12,
                1_200,
            ));
        }

        let mut signals = decision.domain_signals.clone();
        let declared = super::domain_profile::decision_dimension_kinds(active_profile);
        super::domain_signals::retain_declared_dimensions(&mut signals, &declared);
        let stage_evidence =
            super::tag_evidence::resolve_evidence(window, &decision.stage_evidence_turns);
        let stage_realtime =
            super::gateway::stage_realtime_write_allowed(super::tag_evidence::evidence_strength(
                &stage_evidence,
                window,
                decision.stage_explicit_intent,
            ));
        let proposed_stage = signals
            .get_str("customer_stage")
            .ok()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        if proposed_stage.as_deref().is_some_and(|stage| {
            !stage_realtime
                || super::guards::check_state_transition(
                    domain_config,
                    self.contact.operation_state.as_deref(),
                    stage,
                )
                .is_some()
        }) {
            signals.remove("customer_stage");
        }
        if !signals.is_empty() {
            let attributes = self
                .contact
                .domain_attributes
                .get_or_insert_with(Document::new);
            for (key, value) in signals {
                if let Some(value) = value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    attributes.insert(key, value);
                }
            }
            self.contact.domain_attributes_updated_at = Some(now);
        }
        if let Some(stage) = proposed_stage.filter(|stage| {
            self.contact
                .domain_attributes
                .as_ref()
                .and_then(|attributes| attributes.get_str("customer_stage").ok())
                == Some(stage.as_str())
        }) {
            apply_operation_state(
                &mut self.contact,
                Some(&stage),
                None,
                None,
                domain_config,
                now,
            );
        }

        apply_operating_memory_patch(&mut self.memory, &decision.operating_memory_update);
        if !decision.next_best_action.is_empty() {
            merge_projection_document(&mut self.memory.next_action, &decision.next_best_action);
        }
        let valid_candidates = decision
            .memory_candidates
            .iter()
            .cloned()
            .filter_map(super::memory::validated_memory_candidate)
            .collect::<Vec<_>>();
        let max_importance = valid_candidates
            .iter()
            .filter_map(|candidate| candidate.get_i32("importance").ok())
            .max()
            .unwrap_or(0);
        if super::memory::decide_candidate_status(decision.memory_write_score, max_importance)
            == "pending"
        {
            apply_memory_candidates_preview(&mut self.memory, &valid_candidates);
        }
        self.memory.updated_at = now;
        self.contact.profile_updated_at = Some(now);
        self.contact.updated_at = now;
    }
}

fn parse_projection_datetime(value: &str) -> Option<DateTime> {
    let value = value.trim();
    (!value.is_empty())
        .then(|| DateTime::parse_rfc3339_str(value).ok())
        .flatten()
}

fn apply_operation_state(
    contact: &mut Contact,
    proposed: Option<&str>,
    reason: Option<&str>,
    confidence: Option<i32>,
    domain_config: Option<&OperationDomainConfig>,
    now: DateTime,
) {
    let Some(proposed) = proposed
        .map(str::trim)
        .filter(|proposed| !proposed.is_empty())
    else {
        return;
    };
    if contact.operation_state.as_deref() != Some(proposed)
        && super::guards::check_state_transition(
            domain_config,
            contact.operation_state.as_deref(),
            proposed,
        )
        .is_some()
    {
        return;
    }
    contact.operation_state = Some(proposed.to_string());
    contact.operation_state_reason = reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(ToString::to_string);
    contact.operation_state_confidence = confidence;
    contact.operation_state_updated_at = Some(now);
}

fn merge_projection_document(target: &mut Document, patch: &Document) {
    for (key, value) in patch {
        match (target.get_mut(key), value) {
            (Some(Bson::Document(existing)), Bson::Document(update)) => {
                merge_projection_document(existing, update);
            }
            _ => {
                target.insert(key, value.clone());
            }
        }
    }
}

fn apply_operating_memory_patch(memory: &mut OperatingMemory, patch: &Document) {
    if let Ok(value) = patch.get_document("userUnderstanding") {
        merge_projection_document(&mut memory.user_understanding, value);
    }
    if let Ok(value) = patch.get_document("relationshipState") {
        merge_projection_document(&mut memory.relationship_state, value);
    }
    if let Ok(value) = patch.get_document("productFit") {
        merge_projection_document(&mut memory.product_fit, value);
    }
    if let Ok(value) = patch.get_document("nextAction") {
        merge_projection_document(&mut memory.next_action, value);
    }
}

fn append_memory_card_text(
    card: &mut crate::models::MemoryCardTyped,
    key: &str,
    value: &str,
    cap: usize,
) {
    let mut values = card.extra.get_array(key).cloned().unwrap_or_default();
    if !values.iter().any(|item| item.as_str() == Some(value)) {
        values.push(Bson::String(value.to_string()));
    }
    if values.len() > cap {
        let remove = values.len() - cap;
        values.drain(0..remove);
    }
    card.extra.insert(key, values);
}

fn apply_memory_candidates_preview(memory: &mut OperatingMemory, candidates: &[Document]) {
    let mut changed = false;
    for candidate in candidates {
        let Ok(kind) = candidate.get_str("type") else {
            continue;
        };
        let Ok(content) = candidate.get_str("content") else {
            continue;
        };
        match kind {
            "preference" => {
                append_memory_card_text(&mut memory.memory_card, "preferences", content, 8)
            }
            "doNotDo" => append_memory_card_text(&mut memory.memory_card, "doNotDo", content, 10),
            "commitment" => {
                append_memory_card_text(&mut memory.memory_card, "commitments", content, 8)
            }
            "objection" => {
                append_memory_card_text(&mut memory.memory_card, "objections", content, 8)
            }
            "openLoop" => append_memory_card_text(&mut memory.memory_card, "openLoops", content, 8),
            "conflict" => {
                let mut conflicts = memory
                    .memory_card
                    .extra
                    .get_array("conflicts")
                    .cloned()
                    .unwrap_or_default();
                conflicts.push(Bson::Document(candidate.clone()));
                if conflicts.len() > 10 {
                    let remove = conflicts.len() - 10;
                    conflicts.drain(0..remove);
                }
                memory.memory_card.extra.insert("conflicts", conflicts);
            }
            _ => {
                if !memory
                    .memory_card
                    .recent_facts
                    .iter()
                    .any(|fact| fact.as_text() == content)
                {
                    memory
                        .memory_card
                        .recent_facts
                        .push(MemoryFactRepr::Plain(content.to_string()));
                    if memory.memory_card.recent_facts.len() > 10 {
                        let remove = memory.memory_card.recent_facts.len() - 10;
                        memory.memory_card.recent_facts.drain(0..remove);
                    }
                }
            }
        }
        changed = true;
    }
    if changed {
        memory.memory_card_version = memory.memory_card_version.saturating_add(1);
        memory.memory_card_updated_at = Some(DateTime::now());
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
    let assist_override = contact.domain_attributes.as_ref().and_then(|attributes| {
        attributes
            .get_str(crate::models::ASSIST_MODE_OVERRIDE_ATTR)
            .ok()
    });
    let assist_on = super::referral::assist_mode_active(
        domain_config
            .as_ref()
            .and_then(|config| config.assist_mode_enabled),
        assist_override,
    );
    let model_assets = load_turn_model_assets(state, &contact, &active_profile, assist_on).await?;
    let effective_soul = active_profile
        .soul_override
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(model_assets.published_soul.as_deref())
        .unwrap_or_default()
        .to_string();
    let mut history = load_context_messages(state, &contact, &runtime).await?;
    history.reverse();
    let persona_now = DateTime::now();
    let persona_world_state = state
        .db
        .persona_world_states()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "current": true,
                "effective_from": { "$lte": persona_now },
                "effective_until": { "$gt": persona_now },
            },
            FindOneOptions::builder()
                .sort(doc! { "version": -1 })
                .build(),
        )
        .await?;
    let mut projection = SimulationProjection {
        contact,
        memory,
        appointments: Vec::new(),
        persona_world_state,
    };
    let mut turns = Vec::new();

    for (index, text) in messages.into_iter().enumerate() {
        let turn_contact = projection.contact.clone();
        let turn_memory = projection.memory.clone();
        let inbound = ConversationMessage {
            id: Some(ObjectId::new()),
            workspace_id: turn_contact.workspace_id.clone(),
            account_id: turn_contact.account_id.clone(),
            contact_wxid: turn_contact.wxid.clone(),
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
        let gateway = precheck_send_gateway(state, &turn_contact, &trigger, &runtime).await?;
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
            &turn_memory,
            &turn_contact,
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
                &turn_contact,
                &inbound,
                &recent,
                &turn_memory,
                &context_pack,
                &operation_knowledge,
                Some(&run_id),
            )
            .await?
        };
        let selected_chunks =
            select_operation_knowledge_chunks(&operation_knowledge.chunks, &knowledge_route);
        let turn_id = inbound
            .message_id
            .clone()
            .unwrap_or_else(|| format!("shadow-turn-{}", index + 1));
        let authority = super::authority::compile(super::authority::AuthorityCompileInput {
            state,
            run_id: &run_id,
            turn_id: &turn_id,
            contact: &turn_contact,
            inbound: &inbound,
            recent_messages: &recent,
            memory: &turn_memory,
            active_products: &model_assets.active_products,
            referral_cards: &model_assets.referral_cards,
            effective_soul: &effective_soul,
            projected_appointments: &projection.appointments,
            projected_world_state: projection.persona_world_state.as_ref(),
            invocation: super::authority::AuthorityInvocation::Conversation,
            evaluated_at: DateTime::now(),
        })
        .await?;
        authority.append_verified_knowledge(
            &selected_chunks,
            &super::knowledge_router::route_used_knowledge_ids(&knowledge_route),
            DateTime::now(),
        )?;

        let mut environment = ModelTurnEnvironment::new(
            ModelTurnInputs {
                state,
                contact: &turn_contact,
                inbound: &inbound,
                recent_messages: &recent,
                pending_tasks: &pending_tasks,
                playbook: playbook.as_ref(),
                domain_config: domain_config.as_ref(),
                runtime: &runtime,
                memory: &turn_memory,
                context_pack: &context_pack,
                knowledge: &operation_knowledge,
                selected_chunks: &selected_chunks,
                knowledge_route: &knowledge_route,
                initial_planner: &initial_planner,
                active_profile: &active_profile,
                active_products: &model_assets.active_products,
                published_soul: model_assets.published_soul.as_deref(),
                sendable_assets: &model_assets.sendable_assets,
                referral_cards: &model_assets.referral_cards,
                reply_prompts: &model_assets.reply_prompts,
                reply_context: &model_assets.reply_context,
                reviewer_prompts: None,
                authority: &authority,
                budget: budget.clone(),
                run_id: &run_id,
                prompt_override: None,
                invocation_kind: ReviewInvocationKind::Conversation,
                first_generation: None,
                persist_runtime_snapshot: false,
            },
            SandboxCommitter::with_policy(
                "simulation",
                state,
                &turn_contact,
                domain_config.as_ref(),
                &runtime,
                &inbound,
            ),
        );
        let outcome = run_turn_with_timeouts(
            &TurnKernelInput {
                run_id: &run_id,
                turn_id: &turn_id,
                authority_bundle_hash: authority.bundle_hash(),
            },
            &mut environment,
            TurnLoopTimeouts::from_seconds(
                state.config.agent_turn_phase_timeout_seconds,
                state.config.agent_turn_repair_timeout_seconds,
                state.config.agent_turn_authorization_timeout_seconds,
                state.config.agent_turn_total_timeout_seconds,
            ),
        )
        .await?;
        drop(environment);
        let commit_receipt = outcome.commit_receipt.to_document();
        let authorized = outcome.authorization.disposition == "authorized";
        let final_status = outcome.authorization.final_status.clone();
        let mut decision = outcome.draft.decision;
        let review = outcome.authorization.review;
        if !authorized {
            decision.should_reply = false;
            decision.autonomy_mode = "blocked".to_string();
        }
        let status = if !gateway.allowed {
            "gateway_blocked".to_string()
        } else if authorized && decision.should_reply {
            "would_send".to_string()
        } else if authorized {
            "no_reply".to_string()
        } else {
            final_status
        };
        let would_send = status == "would_send";
        let current_state = turn_contact
            .operation_state
            .clone()
            // H13：无 operation_state 时回落状态机初始态（替代写死 "new_contact"）。
            .unwrap_or_else(|| super::guards::initial_operation_state_key(domain_config.as_ref()));
        let mut projection_window = history.clone();
        projection_window.push(inbound.clone());
        let mut memory_preview = doc! { "status": "not_authorized" };
        if gateway.allowed && authorized {
            let applied_at = DateTime::now();
            projection.apply_authorized_decision(
                &decision,
                domain_config.as_ref(),
                &turn_id,
                would_send,
                applied_at,
            )?;
            match super::post_decision::generate_projection_read_only(
                state,
                &decision,
                &projection.memory,
                &context_pack,
                domain_config.as_ref(),
                &active_profile,
                &model_assets.active_products,
                &projection_window,
                &projection.contact,
                &run_id,
            )
            .await
            {
                Ok(deferred) => match super::post_decision::normalize_projection_read_only(
                    state,
                    &projection.contact,
                    domain_config.as_ref(),
                    &active_profile,
                    deferred,
                )
                .await
                {
                    Ok(projected_decision) => {
                        projection.apply_deferred_projection(
                            &projected_decision,
                            domain_config.as_ref(),
                            &active_profile,
                            &projection_window,
                            DateTime::now(),
                        );
                        memory_preview = doc! {
                            "status": "applied",
                            "operatingMemoryUpdate": projected_decision.operating_memory_update,
                            "memoryCandidates": projected_decision.memory_candidates,
                            "memoryWriteScore": projected_decision.memory_write_score,
                            "memoryUpdate": projected_decision.memory_update,
                            "memoryCard": to_document(&projection.memory.memory_card).unwrap_or_default(),
                        };
                    }
                    Err(error) => {
                        tracing::warn!(%error, %run_id, %turn_id, "simulation projection normalization failed");
                        memory_preview = doc! {
                            "status": "failed",
                            "reason": "projection_normalization_failed",
                        };
                    }
                },
                Err(error) => {
                    tracing::warn!(%error, %run_id, %turn_id, "simulation deferred projection failed");
                    memory_preview = doc! {
                        "status": "failed",
                        "reason": "projection_generation_failed",
                    };
                }
            }
        } else if !gateway.allowed {
            memory_preview = doc! { "status": "gateway_blocked" };
        }
        let next_state = projection
            .contact
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
            commit_receipt,
            memory_preview,
            state_transition: doc! {
                "from": current_state,
                "to": next_state,
                "reason": decision.operation_state_reason.clone().unwrap_or_default(),
                "accepted": projection.contact.operation_state != turn_contact.operation_state,
            },
        });
        history.push(inbound);
        if would_send {
            history.push(ConversationMessage {
                id: Some(ObjectId::new()),
                workspace_id: projection.contact.workspace_id.clone(),
                account_id: projection.contact.account_id.clone(),
                contact_wxid: projection.contact.wxid.clone(),
                message_id: Some(format!("shadow-reply-{}", index + 1)),
                dedupe_key: None,
                direction: MessageDirection::Outbound,
                content: decision.reply_text.clone(),
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

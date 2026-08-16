//! Model-facing implementation of the shared turn Harness.
//!
//! This module owns no delivery policy. It gives the model one bounded way to generate,
//! inspect read-only knowledge, receive independent authorization feedback, and try again.
//! Provenance, lifecycle and side effects remain deterministic responsibilities of the
//! authority snapshot and the selected [`TurnCommitter`].

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use mongodb::bson::{doc, to_document, DateTime, Document};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::models::{
    AgentTask, Contact, ContentAsset, ConversationMessage, DomainProfile, OperatingMemory,
    OperationDomainConfig, OperationKnowledgeChunk, OperationPlaybook, Product, ReferralCard,
};
use crate::routes::AppState;

use super::appointment_request::validate_appointment_request;
use super::authority::AuthoritySnapshot;
use super::budget::RunBudget;
use super::decision::{
    decide_reply_with_promote_context, initial_operation_state_for_contact,
    load_operation_state_policy_for_contact, load_published_soul, load_referral_cards,
    load_reply_prompt_snapshot, load_sendable_assets, DecisionRunSnapshot, PromptOverride,
    ReplyContextCache, ReplyPromptSnapshot,
};
use super::gateway::precheck_send_gateway;
use super::guards::{
    action_policy_state_key, decision_operation_state_candidate, enforce_reviewed_decision_actions,
    normalize_decision_runtime, normalize_decision_state, planner_from_decision,
};
use super::knowledge_router::route_used_knowledge_ids;
use super::knowledge_tools::{
    dispatch_chat_tool_call, ToolDispatchState, TOOL_LIST_CATALOG, TOOL_OPEN_SLICE, TOOL_SEARCH,
};
use super::review::{
    apply_independent_claim_gate, contact_has_principal_product_exemption, effective_review_mode,
    evaluate_independent_claim_gate_with_authority, finalize_review_for_send,
    local_decision_review, review_decision, should_run_targeted_rewrite, GatewayStatusFinal,
    PendingFinalizeEvent, ReviewInvocationKind, ReviewerPromptCache,
};
use super::runtime::UserRuntimeParameters;
use super::sufficiency::PromptTier;
use super::turn_loop::{
    AuthorizationManifest, CommitPlan, CommitReceipt, CommitResult, DraftEnvelope,
    ToolDispatchBatch, TurnEnvironment, TurnGenerateRequest,
};
use super::types::{
    AgentDecision, AgentTrigger, KnowledgeRouteResult, KnowledgeRuntime, RunPlannerResult,
    ToolCallRequest, HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD,
};

const USER_TURN_TOOL_NAMES: &[&str] = &[TOOL_LIST_CATALOG, TOOL_SEARCH, TOOL_OPEN_SLICE];

pub(crate) struct TurnModelAssets {
    pub active_products: Vec<Product>,
    pub published_soul: Option<String>,
    pub sendable_assets: Vec<ContentAsset>,
    pub referral_cards: Vec<ReferralCard>,
    pub reply_prompts: ReplyPromptSnapshot,
    pub reply_context: ReplyContextCache,
}

pub(crate) async fn load_turn_model_assets(
    state: &AppState,
    contact: &Contact,
    active_profile: &DomainProfile,
    assist_on: bool,
) -> AppResult<TurnModelAssets> {
    let products_future = async {
        if active_profile.transaction_facts_enabled {
            super::entitlements::load_active_products(&state.db, &contact.workspace_id).await
        } else {
            Vec::new()
        }
    };
    let soul_future = async {
        let has_override = active_profile
            .soul_override
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        if has_override {
            Ok(None)
        } else {
            load_published_soul(state, &contact.workspace_id, "user")
                .await
                .map(Some)
        }
    };
    let sendable_assets_future = async {
        load_sendable_assets(state, &contact.workspace_id, &contact.account_id)
            .await
            .unwrap_or_default()
    };
    let referral_cards_future = async {
        if assist_on {
            load_referral_cards(state, &contact.workspace_id, &contact.account_id)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    };
    let prompt_future = load_reply_prompt_snapshot(state, contact);
    let (active_products, published_soul, sendable_assets, referral_cards, reply_prompts) = tokio::join!(
        products_future,
        soul_future,
        sendable_assets_future,
        referral_cards_future,
        prompt_future,
    );
    Ok(TurnModelAssets {
        active_products,
        published_soul: published_soul?,
        sendable_assets,
        referral_cards,
        reply_prompts: reply_prompts?,
        reply_context: ReplyContextCache::new(),
    })
}

#[async_trait]
pub(crate) trait TurnCommitter: Send {
    async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult>;
}

#[derive(Clone)]
pub(crate) struct SandboxPolicyContext {
    state: AppState,
    contact: Contact,
    inbound: ConversationMessage,
    domain_config: Option<OperationDomainConfig>,
    runtime: UserRuntimeParameters,
}

#[derive(Clone)]
pub(crate) struct SandboxCommitter {
    environment: String,
    policy_context: Option<SandboxPolicyContext>,
}

impl SandboxCommitter {
    #[allow(dead_code)]
    pub(crate) fn new(environment: impl Into<String>) -> Self {
        Self {
            environment: environment.into(),
            policy_context: None,
        }
    }

    pub(crate) fn with_policy(
        environment: impl Into<String>,
        state: &AppState,
        contact: &Contact,
        domain_config: Option<&OperationDomainConfig>,
        runtime: &UserRuntimeParameters,
        inbound: &ConversationMessage,
    ) -> Self {
        Self {
            environment: environment.into(),
            policy_context: Some(SandboxPolicyContext {
                state: state.clone(),
                contact: contact.clone(),
                inbound: inbound.clone(),
                domain_config: domain_config.cloned(),
                runtime: runtime.clone(),
            }),
        }
    }

    async fn tighten_policy(
        &self,
        plan: &mut CommitPlan,
        context: &SandboxPolicyContext,
    ) -> AppResult<SandboxPolicyOutcome> {
        let mut outcome = SandboxPolicyOutcome::default();
        if plan.authorization.disposition != "authorized" {
            return Ok(outcome);
        }

        let operation_state = if let Some(state) = action_policy_state_key(
            context.domain_config.as_ref(),
            context.contact.operation_state.as_deref(),
            decision_operation_state_candidate(&plan.draft.decision),
        ) {
            state
        } else {
            initial_operation_state_for_contact(&context.state, &context.contact).await?
        };
        let policy = load_operation_state_policy_for_contact(
            &context.state,
            &context.contact.workspace_id,
            &operation_state,
            &context.contact.wxid,
        )
        .await?;
        let actions = super::guards::reviewed_decision_actions(
            &plan.draft.decision,
            &plan.authorization.review,
        );
        if let Err((action, reason)) = enforce_reviewed_decision_actions(
            policy.as_ref(),
            &plan.draft.decision,
            &plan.authorization.review,
        ) {
            outcome.state_action_hold = Some(doc! {
                "actions": actions,
                "action": action,
                "operation_state": operation_state,
                "reason": &reason,
            });
            sandbox_hold_plan(
                plan,
                "held_by_ai_policy",
                &reason,
                "state_action_policy_blocked",
            );
            return Ok(outcome);
        }

        let gateway = precheck_send_gateway(
            &context.state,
            &context.contact,
            &AgentTrigger::Inbound(&context.inbound),
            &context.runtime,
        )
        .await?;
        if plan.draft.decision.should_reply && !gateway.allowed {
            let status = if gateway.status == "quiet_hours_deferred" {
                "quiet_hours_deferred"
            } else {
                "gateway_blocked"
            };
            outcome.gateway_block = Some(to_document(&gateway).unwrap_or_default());
            sandbox_hold_plan(plan, status, &gateway.reason, "final_send_precheck_blocked");
        }
        Ok(outcome)
    }
}

#[derive(Default)]
struct SandboxPolicyOutcome {
    state_action_hold: Option<Document>,
    gateway_block: Option<Document>,
}

fn sandbox_hold_plan(plan: &mut CommitPlan, status: &str, reason: &str, risk: &str) {
    let mut review = plan.authorization.review.clone();
    review.approved = false;
    review.should_hold = true;
    review.final_review_status = status.to_string();
    if !risk.is_empty() && !review.risks.iter().any(|item| item == risk) {
        review.risks.push(risk.to_string());
    }
    if review.review_summary.trim().is_empty() {
        review.review_summary = reason.to_string();
    }
    plan.draft.decision.should_reply = false;
    plan.draft.decision.autonomy_mode = "blocked".to_string();
    plan.authorization = AuthorizationManifest::held(status, reason, review);
}

#[async_trait]
impl TurnCommitter for SandboxCommitter {
    async fn commit(&mut self, mut plan: CommitPlan) -> AppResult<CommitResult> {
        let policy_outcome = if let Some(context) = self.policy_context.as_ref() {
            self.tighten_policy(&mut plan, context).await?
        } else {
            SandboxPolicyOutcome::default()
        };
        let mut details = doc! {
            "final_status": &plan.authorization.final_status,
            "draft_hash": &plan.draft.draft_hash,
            "would_reply": plan.draft.decision.should_reply,
            "would_create_appointment": plan
                .draft
                .decision
                .appointment_request
                .as_ref()
                .is_some_and(|request| request.requested),
        };
        if let Some(hold) = policy_outcome.state_action_hold {
            details.insert("state_action_hold", hold);
        }
        if let Some(block) = policy_outcome.gateway_block {
            details.insert("gateway_block", block);
        }
        Ok(CommitResult {
            receipt: CommitReceipt {
                status: if plan.authorization.disposition == "authorized" {
                    "simulated".to_string()
                } else {
                    "held".to_string()
                },
                environment: self.environment.clone(),
                committed_at: DateTime::now(),
                details,
                ..CommitReceipt::default()
            },
            plan,
        })
    }
}

/// Frozen model-visible inputs for one turn. Production, Simulation and Prompt Shadow construct
/// this same shape; only the committer and optional prompt override differ.
pub(crate) struct ModelTurnInputs<'a> {
    pub state: &'a AppState,
    pub contact: &'a Contact,
    pub inbound: &'a ConversationMessage,
    pub recent_messages: &'a [ConversationMessage],
    pub pending_tasks: &'a [AgentTask],
    pub playbook: Option<&'a OperationPlaybook>,
    pub domain_config: Option<&'a OperationDomainConfig>,
    pub runtime: &'a UserRuntimeParameters,
    pub memory: &'a OperatingMemory,
    pub context_pack: &'a Document,
    pub knowledge: &'a KnowledgeRuntime,
    pub selected_chunks: &'a [OperationKnowledgeChunk],
    pub knowledge_route: &'a KnowledgeRouteResult,
    pub initial_planner: &'a RunPlannerResult,
    pub active_profile: &'a DomainProfile,
    pub active_products: &'a [Product],
    pub published_soul: Option<&'a str>,
    pub sendable_assets: &'a [ContentAsset],
    pub referral_cards: &'a [ReferralCard],
    pub reply_prompts: &'a ReplyPromptSnapshot,
    pub reply_context: &'a ReplyContextCache,
    pub reviewer_prompts: Option<&'a ReviewerPromptCache>,
    pub authority: &'a AuthoritySnapshot,
    pub budget: Arc<RunBudget>,
    pub run_id: &'a str,
    pub prompt_override: Option<&'a PromptOverride>,
    pub invocation_kind: ReviewInvocationKind,
    /// Production may overlap routing with a first Lean pass and hand that exact result to the
    /// kernel. Every later tool/repair iteration is generated in Full.
    pub first_generation: Option<(AgentDecision, Vec<String>)>,
    /// Shadow environments deliberately avoid writing turn snapshots.
    pub persist_runtime_snapshot: bool,
}

pub(crate) struct ModelTurnEnvironment<'a, C> {
    inputs: ModelTurnInputs<'a>,
    committer: C,
    tool_state: ToolDispatchState,
    opened_chunk_ids: HashSet<String>,
    last_planner: RunPlannerResult,
    pending_finalize_events: Vec<PendingFinalizeEvent>,
}

impl<'a, C> ModelTurnEnvironment<'a, C> {
    pub(crate) fn new(inputs: ModelTurnInputs<'a>, committer: C) -> Self {
        let last_planner = inputs.initial_planner.clone();
        Self {
            inputs,
            committer,
            tool_state: ToolDispatchState::new(),
            opened_chunk_ids: HashSet::new(),
            last_planner,
            pending_finalize_events: Vec::new(),
        }
    }

    pub(crate) fn pending_finalize_events(&self) -> &[PendingFinalizeEvent] {
        &self.pending_finalize_events
    }

    pub(crate) fn committer(&self) -> &C {
        &self.committer
    }

    fn normalize_candidate(
        &mut self,
        mut decision: AgentDecision,
        full_context: bool,
    ) -> AgentDecision {
        normalize_decision_state(&mut decision, self.inputs.domain_config);
        normalize_decision_runtime(&mut decision, self.inputs.initial_planner);
        let mut planner = planner_from_decision(&decision, "shared Agent Harness turn");
        if !self.inputs.knowledge_route.selected_chunk_ids.is_empty()
            || !self
                .inputs
                .knowledge_route
                .selected_knowledge_ids
                .is_empty()
            || !self.opened_chunk_ids.is_empty()
        {
            planner.knowledge_required = true;
            if planner.review_mode.trim().is_empty() {
                planner.review_mode = "full".to_string();
            }
        }
        normalize_decision_runtime(&mut decision, &planner);
        decision.context_pack_version =
            Some(super::memory::next_memory_card_version(self.inputs.memory));
        if full_context {
            let mut used = route_used_knowledge_ids(self.inputs.knowledge_route);
            used.extend(self.opened_chunk_ids.iter().cloned());
            used.sort();
            used.dedup();
            decision.used_knowledge_ids = used;
        }
        self.last_planner = planner;
        decision
    }

    fn loop_context(request: &TurnGenerateRequest) -> String {
        serde_json::to_string(&json!({
            "harnessObservation": {
                "verifiedToolResults": request.tool_context,
                "authorizationFeedback": request.authorization_feedback,
                "repairAttempt": request.repair_attempt,
                "iteration": request.iteration,
            },
            "instruction": "Reassess the complete conversation and current authority bundle. Use authorization feedback to revise only what is unsupported or low quality. Tool errors are observations, not proof. Return another tool_calling step only when a specific read-only lookup is still necessary; otherwise return a complete final decision."
        }))
        .unwrap_or_default()
    }

    fn repair_instruction(review: &super::types::DecisionReviewResult) -> String {
        [
            review.rewrite_instruction.trim(),
            review.revision_direction.trim(),
            review.review_summary.trim(),
        ]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or("Remove or narrow the unsupported part and produce a complete final response.")
        .to_string()
    }

    fn hold_for_exhausted_repair(
        draft: &mut DraftEnvelope,
        mut review: super::types::DecisionReviewResult,
        risk: &str,
        reason: &str,
    ) -> AuthorizationManifest {
        draft.decision.should_reply = false;
        draft.decision.autonomy_mode = "blocked".to_string();
        review.approved = false;
        review.needs_revision = false;
        review.final_review_status = "blocked_by_budget".to_string();
        if !review.risks.iter().any(|existing| existing == risk) {
            review.risks.push(risk.to_string());
        }
        if review.review_summary.trim().is_empty() {
            review.review_summary = reason.to_string();
        }
        AuthorizationManifest::held("blocked_by_budget", reason, review)
    }

    fn authorization_chunks(&self) -> Vec<OperationKnowledgeChunk> {
        let mut chunks = self.inputs.selected_chunks.to_vec();
        let mut seen = chunks
            .iter()
            .filter_map(|chunk| chunk.id.map(|id| id.to_hex()))
            .collect::<HashSet<_>>();
        chunks.extend(self.inputs.knowledge.chunks.iter().filter_map(|chunk| {
            let id = chunk.id.map(|id| id.to_hex())?;
            (self.opened_chunk_ids.contains(&id) && seen.insert(id)).then(|| chunk.clone())
        }));
        chunks
    }
}

#[async_trait]
impl<C> TurnEnvironment for ModelTurnEnvironment<'_, C>
where
    C: TurnCommitter,
{
    async fn generate(
        &mut self,
        request: TurnGenerateRequest,
    ) -> AppResult<(AgentDecision, Vec<String>)> {
        if request.iteration == 0 {
            if let Some((decision, risks)) = self.inputs.first_generation.take() {
                let full_context = !self.inputs.knowledge_route.selected_chunk_ids.is_empty()
                    && !decision.used_knowledge_ids.is_empty();
                return Ok((self.normalize_candidate(decision, full_context), risks));
            }
        }

        let loop_context = Self::loop_context(&request);
        let rewrite_instruction = (!request.authorization_feedback.trim().is_empty())
            .then_some(request.authorization_feedback.as_str());
        let (decision, risks) = decide_reply_with_promote_context(
            self.inputs.state,
            self.inputs.contact,
            self.inputs.inbound,
            self.inputs.recent_messages,
            self.inputs.pending_tasks,
            self.inputs.playbook,
            self.inputs.domain_config,
            self.inputs.runtime,
            self.inputs.memory,
            self.inputs.context_pack,
            self.inputs.selected_chunks,
            self.inputs.knowledge_route,
            rewrite_instruction,
            Some(self.inputs.run_id),
            self.inputs.prompt_override,
            PromptTier::Full,
            Some(DecisionRunSnapshot {
                active_profile: self.inputs.active_profile,
                active_products: self.inputs.active_products,
                published_soul: self.inputs.published_soul,
                sendable_assets: self.inputs.sendable_assets,
                referral_cards: self.inputs.referral_cards,
                reply_prompts: self.inputs.reply_prompts,
                reply_context: self.inputs.reply_context,
                authority: self.inputs.authority,
            }),
            Some(&loop_context),
        )
        .await?;
        Ok((self.normalize_candidate(decision, true), risks))
    }

    async fn dispatch_tools(
        &mut self,
        calls: &[ToolCallRequest],
        deadline: tokio::time::Instant,
    ) -> AppResult<ToolDispatchBatch> {
        let mut results = Vec::with_capacity(calls.len());
        let mut trace = Vec::with_capacity(calls.len());
        let mut dispatched = 0usize;

        for call in calls {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::External("turn_loop_timeout".to_string()));
            }
            let result = if USER_TURN_TOOL_NAMES.contains(&call.tool.trim()) {
                dispatched += 1;
                dispatch_chat_tool_call(
                    call,
                    self.inputs.runtime,
                    self.inputs.knowledge,
                    &self.inputs.state.db,
                    &self.inputs.contact.workspace_id,
                    &self.inputs.contact.account_id,
                    &self.inputs.budget,
                    &mut self.tool_state,
                    None,
                )
                .await
            } else {
                json!({
                    "error": "tool_not_available_in_user_turn",
                    "detail": "Only knowledge.list_catalog, knowledge.search and knowledge.open_slice are available in this Harness."
                })
            };

            if call.tool.trim() == TOOL_OPEN_SLICE && result.get("error").is_none() {
                let ids = result
                    .get("slices")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|slice| {
                        slice.get("integrity_status").and_then(Value::as_str) == Some("verified")
                    })
                    .filter_map(|slice| slice.get("chunk_id").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                self.inputs.authority.append_verified_knowledge(
                    &self.inputs.knowledge.chunks,
                    &ids,
                    DateTime::now(),
                )?;
                self.opened_chunk_ids.extend(ids);
            }

            let trace_row = doc! {
                "phase": "tool_result",
                "tool": &call.tool,
                "arguments": call.arguments.clone(),
                "result": to_document(&result).unwrap_or_else(|_| doc! { "value": result.to_string() }),
            };
            trace.push(trace_row);
            results.push(json!({
                "tool": call.tool,
                "arguments": call.arguments,
                "result": result,
            }));
        }

        Ok(ToolDispatchBatch {
            context_fragment: serde_json::to_string(&json!({ "toolResults": results }))
                .unwrap_or_default(),
            trace,
            dispatched,
            evidence_fingerprint: self.inputs.authority.ledger().hash(),
        })
    }

    async fn authorize(&mut self, draft: &mut DraftEnvelope) -> AppResult<AuthorizationManifest> {
        self.pending_finalize_events.clear();
        if let Err(issue) =
            validate_appointment_request(draft.decision.appointment_request.as_ref())
        {
            let risk = format!("appointment_request_invalid:{}", issue.code());
            let mut review = super::types::DecisionReviewResult {
                approved: false,
                rewrite_instruction: issue.repair_instruction().to_string(),
                review_summary: format!(
                    "Structured appointment request failed validation: {}",
                    issue.code()
                ),
                risks: vec![risk],
                ..Default::default()
            };
            if self.inputs.budget.is_llm_or_token_exhausted() {
                review.should_hold = true;
                review.hold_category = HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD.to_string();
                review.final_review_status = "blocked_by_budget".to_string();
                review
                    .risks
                    .push("budget_exceeded_no_action_repair".to_string());
                return Ok(AuthorizationManifest::held(
                    "blocked_by_budget",
                    "Appointment action is invalid and the bounded repair budget is exhausted",
                    review,
                ));
            }
            return Ok(AuthorizationManifest::repairable(
                "repair_required",
                issue.repair_instruction(),
                review,
            ));
        }

        let decision = &draft.decision;
        let has_appointment_action = decision
            .appointment_request
            .as_ref()
            .is_some_and(|request| request.requested);
        let requires_semantic_authorization = decision.should_reply || has_appointment_action;
        let authorization_chunks = self.authorization_chunks();
        let (mut review, priced_from_catalog) = if !requires_semantic_authorization {
            (
                local_decision_review(decision, &self.inputs.budget, self.inputs.runtime),
                false,
            )
        } else if self.inputs.budget.is_llm_or_token_exhausted() {
            self.inputs
                .budget
                .mark_degraded("turn_authorization_skipped_budget_exceeded");
            let mut authorization_probe = decision.clone();
            authorization_probe.should_reply = true;
            let mut review = local_decision_review(
                &authorization_probe,
                &self.inputs.budget,
                self.inputs.runtime,
            );
            if has_appointment_action && !decision.should_reply {
                review.review_summary =
                    "Required appointment action authorization was unavailable because the run budget was exhausted"
                        .to_string();
                review
                    .risks
                    .push("budget_exceeded_no_action_authorization".to_string());
            }
            (review, false)
        } else {
            let claim_gate_future = evaluate_independent_claim_gate_with_authority(
                self.inputs.state,
                self.inputs.contact,
                self.inputs.inbound,
                self.inputs.recent_messages,
                decision,
                &authorization_chunks,
                self.inputs.active_products,
                self.inputs.referral_cards,
                self.inputs.active_profile,
                DateTime::now(),
                Some(self.inputs.run_id),
                self.inputs.invocation_kind,
                Some(self.inputs.authority),
            );
            let (mut review, claim_gate) = if decision.should_reply {
                let review_mode = effective_review_mode(
                    &self.last_planner,
                    decision,
                    self.inputs.runtime,
                    draft.repair_attempt > 0,
                );
                let review_future = review_decision(
                    self.inputs.state,
                    self.inputs.contact,
                    self.inputs.inbound,
                    self.inputs.recent_messages,
                    decision,
                    self.inputs.playbook,
                    self.inputs.domain_config,
                    self.inputs.runtime,
                    self.inputs.memory,
                    self.inputs.context_pack,
                    &authorization_chunks,
                    self.inputs.knowledge_route,
                    review_mode,
                    Some(self.inputs.run_id),
                    self.inputs.prompt_override,
                    Some(self.inputs.active_profile),
                    self.inputs.reviewer_prompts,
                    self.inputs.invocation_kind,
                );
                let (review, claim_gate) = tokio::join!(review_future, claim_gate_future);
                (review?, claim_gate)
            } else {
                (
                    local_decision_review(decision, &self.inputs.budget, self.inputs.runtime),
                    claim_gate_future.await,
                )
            };
            let priced_from_catalog = apply_independent_claim_gate(
                claim_gate,
                decision,
                &mut review,
                self.inputs.active_products,
            );
            (review, priced_from_catalog)
        };

        if should_run_targeted_rewrite(decision, &review, self.inputs.runtime) {
            if self.inputs.budget.is_llm_or_token_exhausted() {
                return Ok(Self::hold_for_exhausted_repair(
                    draft,
                    review,
                    "budget_exceeded_before_action_repair",
                    "The candidate requires another authorization repair, but the bounded run budget is exhausted",
                ));
            }
            return Ok(AuthorizationManifest::repairable(
                "repair_required",
                Self::repair_instruction(&review),
                review,
            ));
        }

        let finalized = finalize_review_for_send(
            review,
            &mut draft.decision,
            self.inputs.runtime,
            self.inputs.contact,
            &authorization_chunks,
            draft.promote_risks.clone(),
            priced_from_catalog,
            contact_has_principal_product_exemption(self.inputs.contact),
        );
        review = finalized.review;
        let pending_events = finalized.pending_events;

        if matches!(finalized.status, GatewayStatusFinal::Approved)
            && review.needs_revision
            && !review.should_hold
        {
            if self.inputs.budget.is_llm_or_token_exhausted() {
                return Ok(Self::hold_for_exhausted_repair(
                    draft,
                    review,
                    "budget_exceeded_before_revision",
                    "The approved candidate requires a quality revision, but the bounded run budget is exhausted",
                ));
            }
            return Ok(AuthorizationManifest::repairable(
                "revision_required",
                Self::repair_instruction(&review),
                review,
            ));
        }

        self.pending_finalize_events = pending_events;

        if matches!(finalized.status, GatewayStatusFinal::Approved) {
            if draft.repair_attempt > 0 {
                review.revision_applied = true;
                review.needs_revision = false;
                review.final_review_status = "revision_applied_approved".to_string();
            }
            Ok(AuthorizationManifest::authorized(
                review.final_review_status.clone(),
                review,
            ))
        } else {
            Ok(AuthorizationManifest::held(
                finalized.status.gateway_status_str(),
                review.review_summary.clone(),
                review,
            ))
        }
    }

    async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult> {
        self.committer.commit(plan).await
    }

    async fn persist_runtime_state(
        &mut self,
        loop_trace: &[Document],
        authorization: &AuthorizationManifest,
        commit_receipt: &CommitReceipt,
    ) -> AppResult<()> {
        if !self.inputs.persist_runtime_snapshot {
            return Ok(());
        }
        self.inputs
            .authority
            .persist_runtime_state(
                &self.inputs.state.db,
                loop_trace,
                Some(authorization.to_document()),
                Some(commit_receipt.to_document()),
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_turn_tool_surface_is_read_only_and_bounded() {
        assert_eq!(USER_TURN_TOOL_NAMES.len(), 3);
        assert!(USER_TURN_TOOL_NAMES.contains(&TOOL_LIST_CATALOG));
        assert!(USER_TURN_TOOL_NAMES.contains(&TOOL_SEARCH));
        assert!(USER_TURN_TOOL_NAMES.contains(&TOOL_OPEN_SLICE));
        assert!(!USER_TURN_TOOL_NAMES.iter().any(|name| {
            name.contains("write") || name.contains("repair") || name.contains("send")
        }));
    }

    #[test]
    fn repair_feedback_prefers_structured_reviewer_direction() {
        let review = super::super::types::DecisionReviewResult {
            rewrite_instruction: "remove unsupported schedule".to_string(),
            revision_direction: "tone only".to_string(),
            review_summary: "summary".to_string(),
            ..Default::default()
        };
        assert_eq!(
            ModelTurnEnvironment::<SandboxCommitter>::repair_instruction(&review),
            "remove unsupported schedule"
        );
    }
}

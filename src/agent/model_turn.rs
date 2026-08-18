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
use super::commitment_lifecycle::validate_commitment_updates;
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
use super::knowledge_router::{
    coherent_knowledge_handoff, route_requires_knowledge_review, route_used_knowledge_ids,
    KnowledgeHandoffKind,
};
use super::knowledge_tools::{
    dispatch_chat_tool_call, ToolDispatchState, TOOL_LIST_CATALOG, TOOL_OPEN_SLICE, TOOL_SEARCH,
};
use super::review::{
    apply_independent_claim_gate, contact_has_principal_product_exemption, effective_review_mode,
    embedded_light_claim_gate_evaluation, evaluate_independent_claim_gate_with_authority,
    finalize_review_for_send, local_decision_review, review_decision, should_run_targeted_rewrite,
    GatewayStatusFinal, PendingFinalizeEvent, ReviewInvocationKind, ReviewerPromptCache,
};
use super::runtime::UserRuntimeParameters;
use super::sufficiency::PromptTier;
use super::turn_loop::{
    AuthorizationManifest, CommitPlan, CommitReceipt, CommitResult, DraftEnvelope,
    ToolDispatchBatch, TurnEnvironment, TurnGenerateRequest,
};
use super::types::{
    reply_protocol_violations, AgentDecision, AgentTrigger, KnowledgeRouteResult, KnowledgeRuntime,
    RunPlannerResult, ToolCallRequest, HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD,
};

const USER_TURN_TOOL_NAMES: &[&str] = &[TOOL_LIST_CATALOG, TOOL_SEARCH, TOOL_OPEN_SLICE];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypedRouteConsistencyIssueKind {
    MissingCustomerReply,
    IncorrectNextStep,
    UnexpectedEscalationRequest,
    ConflictingDeliverySideEffects,
    MissingEscalationRequest,
    IncompleteEscalationRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TypedRouteConsistencyIssue {
    handoff: KnowledgeHandoffKind,
    kind: TypedRouteConsistencyIssueKind,
}

fn has_conflicting_delivery_side_effects(decision: &AgentDecision) -> bool {
    decision
        .appointment_request
        .as_ref()
        .is_some_and(|request| request.requested)
        || decision
            .commitment
            .as_ref()
            .is_some_and(|commitment| !commitment.text.trim().is_empty())
        || decision
            .last_commitment
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || decision
            .follow_up
            .as_ref()
            .is_some_and(|follow_up| follow_up.needed)
        || decision
            .cooldown_until
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || decision
            .assets_to_send
            .iter()
            .any(|directive| !directive.asset_id.trim().is_empty())
        || decision
            .namecard_to_send
            .as_ref()
            .is_some_and(|directive| !directive.card_id.trim().is_empty())
        || !decision.commitment_updates.is_empty()
}

/// Validate the structural hand-off between two independent AI decisions. The Knowledge Agent
/// owns the semantic route; this function checks only that the Reply Agent represented the
/// already-selected route as one complete customer-facing action. No customer text is inspected.
fn typed_route_consistency_issue(
    route: &KnowledgeRouteResult,
    decision: &AgentDecision,
    principal_channel_available: bool,
) -> Option<TypedRouteConsistencyIssue> {
    let handoff = coherent_knowledge_handoff(route)?;
    if handoff == KnowledgeHandoffKind::AskPrincipal && !principal_channel_available {
        return None;
    }
    if !decision.should_reply || decision.reply_text.trim().is_empty() {
        return Some(TypedRouteConsistencyIssue {
            handoff,
            kind: TypedRouteConsistencyIssueKind::MissingCustomerReply,
        });
    }

    if handoff != KnowledgeHandoffKind::AskPrincipal {
        let expected_next_step = match handoff {
            KnowledgeHandoffKind::ClarifyCustomer => "clarify",
            KnowledgeHandoffKind::DeferLicensedProfessional
            | KnowledgeHandoffKind::DeferExternalSystem => "defer",
            KnowledgeHandoffKind::AskPrincipal => unreachable!(),
        };
        if decision.next_step != expected_next_step {
            return Some(TypedRouteConsistencyIssue {
                handoff,
                kind: TypedRouteConsistencyIssueKind::IncorrectNextStep,
            });
        }
        if decision
            .escalation_request
            .as_ref()
            .is_some_and(|request| request.needed)
        {
            return Some(TypedRouteConsistencyIssue {
                handoff,
                kind: TypedRouteConsistencyIssueKind::UnexpectedEscalationRequest,
            });
        }
        if has_conflicting_delivery_side_effects(decision) {
            return Some(TypedRouteConsistencyIssue {
                handoff,
                kind: TypedRouteConsistencyIssueKind::ConflictingDeliverySideEffects,
            });
        }
        return None;
    }

    let Some(request) = decision
        .escalation_request
        .as_ref()
        .filter(|request| request.needed)
    else {
        return Some(TypedRouteConsistencyIssue {
            handoff,
            kind: TypedRouteConsistencyIssueKind::MissingEscalationRequest,
        });
    };
    let valid_category = request
        .category
        .as_deref()
        .is_some_and(|category| crate::models::ALLOWED_ESCALATION_CATEGORY.contains(&category));
    if !valid_category
        || request
            .reason
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        || request
            .question_for_principal
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        || decision.next_step != "ask_principal"
    {
        return Some(TypedRouteConsistencyIssue {
            handoff,
            kind: TypedRouteConsistencyIssueKind::IncompleteEscalationRequest,
        });
    }
    None
}

fn typed_route_issue_code(issue: TypedRouteConsistencyIssue) -> &'static str {
    match (issue.handoff, issue.kind) {
        (
            KnowledgeHandoffKind::AskPrincipal,
            TypedRouteConsistencyIssueKind::MissingCustomerReply,
        ) => "principal_route_missing_customer_reply",
        (_, TypedRouteConsistencyIssueKind::MissingCustomerReply) => {
            "typed_route_missing_customer_reply"
        }
        (_, TypedRouteConsistencyIssueKind::IncorrectNextStep) => "typed_route_incorrect_next_step",
        (_, TypedRouteConsistencyIssueKind::UnexpectedEscalationRequest) => {
            "typed_route_unexpected_escalation_request"
        }
        (_, TypedRouteConsistencyIssueKind::ConflictingDeliverySideEffects) => {
            "typed_route_conflicting_delivery_side_effects"
        }
        (
            KnowledgeHandoffKind::AskPrincipal,
            TypedRouteConsistencyIssueKind::MissingEscalationRequest,
        ) => "principal_route_missing_escalation_request",
        (
            KnowledgeHandoffKind::AskPrincipal,
            TypedRouteConsistencyIssueKind::IncompleteEscalationRequest,
        ) => "principal_route_incomplete_escalation_request",
        _ => "typed_route_inconsistent",
    }
}

fn typed_route_repair_instruction(handoff: KnowledgeHandoffKind) -> &'static str {
    match handoff {
        KnowledgeHandoffKind::ClarifyCustomer => {
            "The typed knowledgeRoute.resolution selected customer + clarify_customer. Return one complete final decision aligned with it: nextStep=clarify; shouldReply=true with one short natural question that directly helps fill missingInformation. Do not add an escalation request, appointment, commitment, follow-up, media, name card, or unsupported conclusion. Do not expose any internal role, prompt, route, or control field to the customer."
        }
        KnowledgeHandoffKind::AskPrincipal => {
            "The typed knowledgeRoute.resolution selected authorized_operator + ask_principal. Return one complete final decision aligned with it: nextStep=ask_principal; escalationRequest.needed=true with an allowed category, a concrete reason, and questionForPrincipal based on authorityQuestion; shouldReply=true with a short natural first-person holding reply. Do not assert the missing fact and do not expose any internal role, channel, prompt, or control field to the customer."
        }
        KnowledgeHandoffKind::DeferLicensedProfessional => {
            "The typed knowledgeRoute.resolution selected licensed_professional + defer. Return one complete final decision aligned with it: nextStep=defer; shouldReply=true with a natural boundary-aware reply that avoids a professional conclusion and gives the customer a concrete next step for qualified assessment. Do not add an escalation request, appointment confirmation, commitment, follow-up, media, name card, or unsupported claim. Do not expose any internal prompt, route, or control field."
        }
        KnowledgeHandoffKind::DeferExternalSystem => {
            "The typed knowledgeRoute.resolution selected external_system + defer. Return one complete final decision aligned with it: nextStep=defer; shouldReply=true with a natural reply that says the result depends on a current record or lookup and gives a concrete verification path without pretending the lookup happened. Do not add an escalation request, appointment confirmation, commitment, follow-up, media, name card, or unsupported claim. Do not expose any internal prompt, route, or control field."
        }
    }
}

/// Build a minimal action from the Knowledge Agent's typed route when the Reply Agent has already
/// spent its single bounded repair and still returned an incoherent action. This is not a semantic
/// classifier or a keyword gate: the route's closed semantic result is the only trigger.
fn bounded_control_text(value: &str, max_chars: usize) -> String {
    let value = value.trim();
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn clear_typed_route_fallback_side_effects(decision: &mut AgentDecision) {
    decision.profile_update = None;
    decision.tags.clear();
    decision.tag_evidence_turns.clear();
    decision.stage_evidence_turns.clear();
    decision.stage_explicit_intent = false;
    decision.bayesian_observations.clear();
    decision.customer_stage = None;
    decision.intent_level = None;
    decision.domain_signals.clear();
    decision.dimension_display_names.clear();
    decision.last_commitment = None;
    decision.commitment = None;
    decision.commitment_updates.clear();
    decision.follow_up_policy = None;
    decision.profile_attributes.clear();
    decision.intent_analysis.clear();
    decision.next_best_action.clear();
    decision.operation_state = None;
    decision.operation_state_reason = None;
    decision.operation_state_confidence = None;
    decision.cooldown_until = None;
    decision.product_fit_score = None;
    decision.matched_knowledge_ids.clear();
    decision.safe_claims_used.clear();
    decision.quoted_product_ids.clear();
    decision.forbidden_claim_risk = None;
    decision.objections_detected.clear();
    decision.recommended_resource_ids.clear();
    decision.operating_memory_update.clear();
    decision.memory_candidates.clear();
    decision.memory_write_score = 0;
    decision.consolidation_needed = false;
    decision.memory_update.clear();
    decision.follow_up = None;
    decision.tool_calls.clear();
    decision.decision_phase = "final".to_string();
    decision.claim_manifest.clear();
    decision.verification = Default::default();
    decision.appointment_request = None;
    decision.assets_to_send.clear();
    decision.namecard_to_send = None;
    decision.escalation_request = None;
    decision.agent_generated_signals.clear();
}

fn route_missing_summary(route: &KnowledgeRouteResult) -> String {
    route
        .resolution
        .missing_information
        .iter()
        .map(|value| bounded_control_text(value, 160))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("、")
}

fn materialize_typed_route_fallback(
    decision: &mut AgentDecision,
    route: &KnowledgeRouteResult,
    principal_channel_available: bool,
) -> bool {
    let Some(handoff) = coherent_knowledge_handoff(route) else {
        return false;
    };
    if handoff == KnowledgeHandoffKind::AskPrincipal && !principal_channel_available {
        return false;
    }

    let missing_summary = route_missing_summary(route);
    let reason = if missing_summary.is_empty() {
        "知识研判显示当前信息不足以形成可靠结论".to_string()
    } else {
        bounded_control_text(
            &format!("知识研判显示以下信息仍待补齐：{missing_summary}"),
            480,
        )
    };

    // A failed repair must not carry stale structured intents into the commit layer. The
    // fallback is one minimal customer-facing action; unrelated writes are discarded rather than
    // guessed from the rejected draft.
    clear_typed_route_fallback_side_effects(decision);

    decision.should_reply = true;
    decision.knowledge_need_reason = reason.clone();
    decision.why_skip_reply.clear();
    decision.risk_self_check =
        "只表达当前证据边界和下一步，不补写未经核实的事实或承诺。".to_string();

    match handoff {
        KnowledgeHandoffKind::ClarifyCustomer => {
            // `missingInformation` is an internal semantic artifact, not customer copy. If the
            // Reply Agent still cannot render the typed hand-off after its bounded repair, keep
            // the fallback neutral instead of exposing an internal checklist verbatim.
            decision.reply_text =
                "我想先把你的情况弄准一点：你现在最希望我先帮你确认哪一件事？".to_string();
            decision.next_step = "clarify".to_string();
            decision.autonomy_mode = "auto".to_string();
            decision.knowledge_need = "insufficient".to_string();
            decision.why_should_reply =
                "先向客户补齐结构化研判指出的关键信息，再继续给出可靠答复。".to_string();
            decision.sufficiency = "need_clarification".to_string();
            decision.missing_tier = "none".to_string();
            decision.clarification_intent = if missing_summary.is_empty() {
                "确认客户当前真正想了解的具体问题".to_string()
            } else {
                missing_summary
            };
        }
        KnowledgeHandoffKind::AskPrincipal => {
            let question = if route.resolution.authority_question.trim().is_empty() {
                if missing_summary.is_empty() {
                    "请确认当前业务安排和可对外回复口径。".to_string()
                } else {
                    bounded_control_text(
                        &format!("请确认以下信息后再回复客户：{missing_summary}"),
                        480,
                    )
                }
            } else {
                bounded_control_text(&route.resolution.authority_question, 480)
            };
            decision.reply_text = super::escalation::fallback_holding_reply().to_string();
            decision.next_step = "ask_principal".to_string();
            decision.autonomy_mode = "assisted".to_string();
            decision.knowledge_need = "required".to_string();
            decision.why_should_reply =
                "先自然承接客户并确认缺失事实，避免在没有依据时给出确定安排。".to_string();
            decision.sufficiency = "need_more_context".to_string();
            decision.missing_tier = "none".to_string();
            decision.clarification_intent.clear();
            decision.escalation_request = Some(crate::models::EscalationRequest {
                needed: true,
                category: Some(crate::models::ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string()),
                reason: Some(reason),
                question_for_principal: Some(question),
                self_serviceable_part: None,
                is_generalizable: false,
            });
        }
        KnowledgeHandoffKind::DeferLicensedProfessional => {
            decision.reply_text = "只凭现在这些信息，我不能替你判断当前的具体情况。最稳妥的是让具备相应资质的专业人员结合实际情况评估；你愿意的话，我可以先帮你把需要重点说明的情况理一下。".to_string();
            decision.next_step = "defer".to_string();
            decision.autonomy_mode = "assisted".to_string();
            decision.knowledge_need = "insufficient".to_string();
            decision.why_should_reply =
                "明确说明专业判断边界，并给出不会误导客户的下一步。".to_string();
            decision.sufficiency = "enough".to_string();
            decision.missing_tier = "none".to_string();
            decision.clarification_intent.clear();
        }
        KnowledgeHandoffKind::DeferExternalSystem => {
            decision.reply_text = "这件事要以当前记录或实时查询结果为准，我现在不能凭现有信息替你确认。你把最新查询结果发我，我再接着帮你看。".to_string();
            decision.next_step = "defer".to_string();
            decision.autonomy_mode = "assisted".to_string();
            decision.knowledge_need = "insufficient".to_string();
            decision.why_should_reply =
                "明确说明实时记录缺口，并给出不会伪造查询结果的下一步。".to_string();
            decision.sufficiency = "enough".to_string();
            decision.missing_tier = "none".to_string();
            decision.clarification_intent.clear();
        }
    }
    true
}

const REPAIR_GENERATION_CALLS: i32 = 1;
const REQUIRED_REAUTHORIZATION_CALLS: i32 = 2;

fn ensure_repair_and_reauthorization_capacity(budget: &RunBudget) -> bool {
    let snapshot = budget.snapshot();
    let token_ceiling = snapshot
        .token_budget
        .saturating_add(snapshot.escalation_bonus);
    if snapshot.tokens_used >= token_ceiling {
        return false;
    }

    // A repair candidate must be generated and then independently authorized by the primary
    // Reviewer plus ClaimGate. The optional second Reviewer is intentionally not repeated for a
    // repaired candidate: it already contributed epistemic diversity on the original candidate
    // and is not part of the mandatory send fence.
    let required_calls = REPAIR_GENERATION_CALLS + REQUIRED_REAUTHORIZATION_CALLS;
    let available_calls = budget.available_llm_calls_before_tail(0);
    if available_calls < required_calls {
        budget.grant_additional_llm_calls(required_calls - available_calls);
    }

    !budget.is_llm_or_token_exhausted()
        && budget.available_llm_calls_before_tail(REQUIRED_REAUTHORIZATION_CALLS) > 0
}

fn align_principal_claim_repair(
    route: &KnowledgeRouteResult,
    principal_channel_available: bool,
    review: &mut super::types::DecisionReviewResult,
) {
    if !principal_channel_available
        || coherent_knowledge_handoff(route) != Some(KnowledgeHandoffKind::AskPrincipal)
        || review.should_hold
        || review
            .claim_analysis
            .get_i64("unsupportedBusinessClaimCount")
            .unwrap_or(0)
            <= 0
    {
        return;
    }

    let claim_gate_detail = review.rewrite_instruction.trim().to_string();
    review.approved = false;
    review.needs_revision = false;
    review.rewrite_instruction = format!(
        "The Knowledge Agent selected ask_principal and the independent ClaimGate found an unsupported real-world assertion in the customer reply. Preserve the complete escalationRequest and a short first-person holding reply, but remove every attempted answer to the unresolved fact, including softened guesses or likely-sounding conclusions. Do not expose the principal, internal route, or authorization process.{}",
        if claim_gate_detail.is_empty() {
            String::new()
        } else {
            format!(" ClaimGate detail: {claim_gate_detail}")
        }
    );
}

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
    protocol_repair_issued: bool,
    commitment_update_repair_issued: bool,
    dropped_commitment_update_risk: Option<String>,
    typed_route_repair_issued: bool,
    typed_route_fallback_applied: bool,
}

fn requires_full_knowledge_review(
    route: &KnowledgeRouteResult,
    opened_chunk_ids: &HashSet<String>,
) -> bool {
    route_requires_knowledge_review(route) || !opened_chunk_ids.is_empty()
}

fn embedded_light_claim_gate_eligible(
    review_mode: &str,
    decision: &AgentDecision,
    route: &KnowledgeRouteResult,
    opened_chunk_ids: &HashSet<String>,
    invocation_kind: ReviewInvocationKind,
    inbound_is_synthetic_relay: bool,
) -> bool {
    review_mode == "light"
        && invocation_kind == ReviewInvocationKind::Conversation
        && !inbound_is_synthetic_relay
        && decision.should_reply
        && decision.risk_level == "low"
        && decision.knowledge_need == "not_required"
        && decision.autonomy_mode == "auto"
        && decision.next_step == "respond"
        && decision.used_knowledge_ids.is_empty()
        && !requires_full_knowledge_review(route, opened_chunk_ids)
        && !has_conflicting_delivery_side_effects(decision)
        && !decision.verification.needed
        && !decision
            .escalation_request
            .as_ref()
            .is_some_and(|request| request.needed)
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
            protocol_repair_issued: false,
            commitment_update_repair_issued: false,
            dropped_commitment_update_risk: None,
            typed_route_repair_issued: false,
            typed_route_fallback_applied: false,
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
        if requires_full_knowledge_review(self.inputs.knowledge_route, &self.opened_chunk_ids) {
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
        review.should_hold = true;
        review.hold_category = HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD.to_string();
        review.final_review_status = "blocked_by_budget".to_string();
        if !review.risks.iter().any(|existing| existing == risk) {
            review.risks.push(risk.to_string());
        }
        if review.review_summary.trim().is_empty() {
            review.review_summary = reason.to_string();
        }
        AuthorizationManifest::held("blocked_by_budget", reason, review)
    }

    fn protocol_repair_instruction(violations: &[String]) -> String {
        let violations = serde_json::to_string(violations).unwrap_or_else(|_| "[]".to_string());
        format!(
            "The previous Reply decision did not satisfy the structured final contract. Protocol violations: {violations}. Re-evaluate the conversation and return one complete decisionPhase=final JSON object. Include every required send-critical field with the correct JSON type and allowed enum value, including riskLevel, knowledgeNeed, runMode, autonomyMode, conversationMode, operationState, needsReview, shouldReply, riskSelfCheck, and the matching replyText/whyShouldReply or whySkipReply branch. Preserve only actions supported by the authority bundle. Do not explain the protocol to the customer and do not place internal field names in replyText."
        )
    }

    fn hold_for_protocol_failure(
        draft: &mut DraftEnvelope,
        violations: Vec<String>,
        reason: &str,
    ) -> AuthorizationManifest {
        draft.decision.should_reply = false;
        draft.decision.autonomy_mode = "blocked".to_string();
        let review = super::types::DecisionReviewResult {
            approved: false,
            should_hold: true,
            hold_reason: reason.to_string(),
            hold_category: HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD.to_string(),
            final_review_status: "blocked_by_required_field".to_string(),
            review_summary: reason.to_string(),
            risks: violations,
            ..Default::default()
        };
        AuthorizationManifest::held("blocked_by_required_field", reason, review)
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
        self.dropped_commitment_update_risk = None;
        let protocol_violations = reply_protocol_violations(&draft.promote_risks);
        if !protocol_violations.is_empty() {
            let instruction = Self::protocol_repair_instruction(&protocol_violations);
            let repair_capacity_available =
                ensure_repair_and_reauthorization_capacity(&self.inputs.budget);
            if !self.protocol_repair_issued && repair_capacity_available {
                self.protocol_repair_issued = true;
                let review = super::types::DecisionReviewResult {
                    approved: false,
                    needs_revision: true,
                    rewrite_instruction: instruction.clone(),
                    review_summary:
                        "Reply decision failed the structured protocol and requires one bounded regeneration"
                            .to_string(),
                    risks: protocol_violations,
                    ..Default::default()
                };
                return Ok(AuthorizationManifest::repairable(
                    "repair_required",
                    instruction,
                    review,
                ));
            }

            if !repair_capacity_available {
                self.inputs
                    .budget
                    .mark_degraded("reply_protocol_repair_skipped_required_authorization_tail");
            }
            return Ok(Self::hold_for_protocol_failure(
                draft,
                protocol_violations,
                "Reply decision remained structurally invalid after the bounded repair opportunity",
            ));
        }
        if let Err(issue) =
            validate_appointment_request(draft.decision.appointment_request.as_ref())
        {
            let risk = format!("appointment_request_invalid:{}", issue.code());
            let review = super::types::DecisionReviewResult {
                approved: false,
                rewrite_instruction: issue.repair_instruction().to_string(),
                review_summary: format!(
                    "Structured appointment request failed validation: {}",
                    issue.code()
                ),
                risks: vec![risk],
                ..Default::default()
            };
            if !ensure_repair_and_reauthorization_capacity(&self.inputs.budget) {
                self.inputs
                    .budget
                    .mark_degraded("appointment_repair_skipped_required_authorization_tail");
                return Ok(Self::hold_for_exhausted_repair(
                    draft,
                    review,
                    "budget_insufficient_for_action_repair_and_reauthorization",
                    "The appointment action requires repair, but the bounded run budget does not retain enough capacity for repair plus independent reauthorization",
                ));
            }
            return Ok(AuthorizationManifest::repairable(
                "repair_required",
                issue.repair_instruction(),
                review,
            ));
        }

        let creates_replacement_commitment = draft.decision.should_reply
            && !draft.decision.reply_text.trim().is_empty()
            && (draft
                .decision
                .commitment
                .as_ref()
                .is_some_and(|commitment| !commitment.text.trim().is_empty())
                || draft
                    .decision
                    .last_commitment
                    .as_deref()
                    .is_some_and(|commitment| !commitment.trim().is_empty()));
        if let Some(issue) = validate_commitment_updates(
            &self.inputs.contact.commitments,
            &draft.decision.commitment_updates,
            creates_replacement_commitment,
            DateTime::now(),
        ) {
            let risk = issue.code().to_string();
            let instruction = issue.repair_instruction();
            let review = super::types::DecisionReviewResult {
                approved: false,
                needs_revision: true,
                rewrite_instruction: instruction.to_string(),
                review_summary: format!(
                    "Structured commitment lifecycle update failed validation: {}",
                    issue.code()
                ),
                risks: vec![risk.clone()],
                ..Default::default()
            };
            let repair_capacity_available =
                ensure_repair_and_reauthorization_capacity(&self.inputs.budget);
            if !self.commitment_update_repair_issued && repair_capacity_available {
                self.commitment_update_repair_issued = true;
                return Ok(AuthorizationManifest::repairable(
                    "repair_required",
                    instruction,
                    review,
                ));
            }

            if !repair_capacity_available {
                self.inputs
                    .budget
                    .mark_degraded("commitment_update_repair_skipped_required_authorization_tail");
            }
            // Lifecycle metadata is optional to the customer-facing response. A malformed or
            // repeatedly unsupported update must not suppress an otherwise reviewable reply.
            draft.decision.commitment_updates.clear();
            self.dropped_commitment_update_risk = Some(risk);
            self.inputs
                .budget
                .mark_degraded("invalid_commitment_updates_dropped");
        }

        let principal_channel_available = self
            .inputs
            .domain_config
            .map(super::escalation::resolve_ask_human_policy)
            .is_some_and(|policy| !policy.decider_chain.is_empty());
        if let Some(issue) = typed_route_consistency_issue(
            self.inputs.knowledge_route,
            &draft.decision,
            principal_channel_available,
        ) {
            let issue_code = typed_route_issue_code(issue);
            let instruction = typed_route_repair_instruction(issue.handoff);
            let review = super::types::DecisionReviewResult {
                approved: false,
                needs_revision: true,
                rewrite_instruction: instruction.to_string(),
                review_summary:
                    "Reply action did not represent the Knowledge Agent's typed hand-off"
                        .to_string(),
                risks: vec![issue_code.to_string()],
                ..Default::default()
            };
            let repair_capacity_available =
                ensure_repair_and_reauthorization_capacity(&self.inputs.budget);
            if !self.typed_route_repair_issued {
                if repair_capacity_available {
                    self.typed_route_repair_issued = true;
                    return Ok(AuthorizationManifest::repairable(
                        "repair_required",
                        instruction,
                        review,
                    ));
                }
                self.inputs
                    .budget
                    .mark_degraded("typed_route_repair_skipped_required_authorization_tail");
            }

            // The route is already an independent AI semantic decision. Preserve that decision
            // as one minimal, customer-safe action instead of replacing it with a generic hold.
            if materialize_typed_route_fallback(
                &mut draft.decision,
                self.inputs.knowledge_route,
                principal_channel_available,
            ) {
                self.typed_route_fallback_applied = true;
                self.inputs
                    .budget
                    .mark_degraded("typed_route_fallback_applied");
            }
        }

        let decision = &draft.decision;
        let has_appointment_action = decision
            .appointment_request
            .as_ref()
            .is_some_and(|request| request.requested);
        let has_commitment_lifecycle_action = !decision.commitment_updates.is_empty();
        let requires_semantic_authorization =
            decision.should_reply || has_appointment_action || has_commitment_lifecycle_action;
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
            if (has_appointment_action || has_commitment_lifecycle_action) && !decision.should_reply
            {
                review.review_summary =
                    "Required structured action authorization was unavailable because the run budget was exhausted"
                        .to_string();
                review
                    .risks
                    .push("budget_exceeded_no_action_authorization".to_string());
            }
            (review, false)
        } else {
            let (mut review, claim_gate, embedded_light_gate) =
                if decision.should_reply || has_commitment_lifecycle_action {
                    let review_mode = effective_review_mode(
                        &self.last_planner,
                        decision,
                        self.inputs.runtime,
                        draft.repair_attempt > 0,
                    );
                    let can_try_embedded = embedded_light_claim_gate_eligible(
                        review_mode,
                        decision,
                        self.inputs.knowledge_route,
                        &self.opened_chunk_ids,
                        self.inputs.invocation_kind,
                        self.inputs.inbound.is_synthetic_relay,
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
                        draft.repair_attempt == 0,
                        self.inputs.invocation_kind,
                    );
                    if can_try_embedded {
                        let review = review_future.await?;
                        if let Some(claim_gate) = embedded_light_claim_gate_evaluation(
                            &review,
                            decision,
                            self.inputs.authority.evidence_catalog(),
                        ) {
                            (review, claim_gate, true)
                        } else {
                            let claim_gate = evaluate_independent_claim_gate_with_authority(
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
                            )
                            .await;
                            (review, claim_gate, false)
                        }
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
                        let (review, claim_gate) = tokio::join!(review_future, claim_gate_future);
                        (review?, claim_gate, false)
                    }
                } else {
                    let claim_gate = evaluate_independent_claim_gate_with_authority(
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
                    )
                    .await;
                    (
                        local_decision_review(decision, &self.inputs.budget, self.inputs.runtime),
                        claim_gate,
                        false,
                    )
                };
            let priced_from_catalog = apply_independent_claim_gate(
                claim_gate,
                decision,
                &mut review,
                self.inputs.active_products,
            );
            align_principal_claim_repair(
                self.inputs.knowledge_route,
                principal_channel_available,
                &mut review,
            );
            review.claim_analysis.insert(
                "claimGateMode",
                if embedded_light_gate {
                    "embedded_light"
                } else {
                    "independent"
                },
            );
            (review, priced_from_catalog)
        };

        if let Some(risk) = self.dropped_commitment_update_risk.as_ref() {
            if !review.risks.iter().any(|existing| existing == risk) {
                review.risks.push(risk.clone());
            }
        }

        if should_run_targeted_rewrite(decision, &review, self.inputs.runtime) {
            if !ensure_repair_and_reauthorization_capacity(&self.inputs.budget) {
                self.inputs
                    .budget
                    .mark_degraded("action_repair_skipped_required_authorization_tail");
                return Ok(Self::hold_for_exhausted_repair(
                    draft,
                    review,
                    "budget_insufficient_for_action_repair_and_reauthorization",
                    "The candidate requires another repair, but the bounded run budget does not retain enough capacity for repair plus independent reauthorization",
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
        if self.typed_route_fallback_applied
            && !review
                .risks
                .iter()
                .any(|risk| risk == "typed_route_fallback_applied")
        {
            review
                .risks
                .push("typed_route_fallback_applied".to_string());
        }
        let pending_events = finalized.pending_events;

        if matches!(finalized.status, GatewayStatusFinal::Approved)
            && review.needs_revision
            && !review.should_hold
        {
            if !ensure_repair_and_reauthorization_capacity(&self.inputs.budget) {
                self.inputs
                    .budget
                    .mark_degraded("quality_revision_skipped_required_authorization_tail");
                return Ok(Self::hold_for_exhausted_repair(
                    draft,
                    review,
                    "budget_insufficient_for_revision_and_reauthorization",
                    "The candidate requires a quality revision, but the bounded run budget does not retain enough capacity for revision plus independent reauthorization",
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

    fn embedded_candidate() -> AgentDecision {
        AgentDecision {
            decision_phase: "final".to_string(),
            next_step: "respond".to_string(),
            should_reply: true,
            reply_text: "在的，刚看到。".to_string(),
            risk_level: "low".to_string(),
            knowledge_need: "not_required".to_string(),
            autonomy_mode: "auto".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn embedded_light_gate_eligibility_is_structural_and_conservative() {
        let route = KnowledgeRouteResult::default();
        let opened = HashSet::new();
        let mut decision = embedded_candidate();
        assert!(embedded_light_claim_gate_eligible(
            "light",
            &decision,
            &route,
            &opened,
            ReviewInvocationKind::Conversation,
            false,
        ));

        decision.verification.needed = true;
        assert!(!embedded_light_claim_gate_eligible(
            "light",
            &decision,
            &route,
            &opened,
            ReviewInvocationKind::Conversation,
            false,
        ));
        decision.verification.needed = false;
        decision.escalation_request = Some(crate::models::EscalationRequest {
            needed: true,
            category: None,
            reason: None,
            question_for_principal: None,
            self_serviceable_part: None,
            is_generalizable: false,
        });
        assert!(!embedded_light_claim_gate_eligible(
            "light",
            &decision,
            &route,
            &opened,
            ReviewInvocationKind::Conversation,
            false,
        ));
    }

    #[test]
    fn empty_optional_action_shells_are_structural_noops() {
        let mut decision = embedded_candidate();
        decision.commitment = Some(Default::default());
        decision.assets_to_send = vec![Default::default()];
        decision.namecard_to_send = Some(Default::default());
        assert!(
            !has_conflicting_delivery_side_effects(&decision),
            "empty optional objects must not force an independent ClaimGate"
        );

        decision.namecard_to_send = Some(super::super::types::NamecardDirective {
            card_id: "card-1".to_string(),
            ..Default::default()
        });
        assert!(has_conflicting_delivery_side_effects(&decision));

        decision.namecard_to_send = None;
        decision.assets_to_send = vec![super::super::types::AssetSendDirective {
            asset_id: "asset-1".to_string(),
            ..Default::default()
        }];
        assert!(has_conflicting_delivery_side_effects(&decision));

        decision.assets_to_send.clear();
        decision.commitment = Some(super::super::types::CommitmentDecision {
            text: "follow up with the customer".to_string(),
            ..Default::default()
        });
        assert!(has_conflicting_delivery_side_effects(&decision));
    }

    fn principal_route() -> KnowledgeRouteResult {
        KnowledgeRouteResult {
            resolution: super::super::types::KnowledgeResolution {
                answerability: super::super::types::KnowledgeAnswerability::Unsupported,
                required_authority:
                    super::super::types::KnowledgeRequiredAuthority::AuthorizedOperator,
                recommended_next_step: super::super::types::KnowledgeNextStep::AskPrincipal,
                missing_information: vec!["current state".to_string()],
                authority_question: "What is the current state?".to_string(),
            },
            ..Default::default()
        }
    }

    fn clarify_route(missing_information: Vec<&str>) -> KnowledgeRouteResult {
        KnowledgeRouteResult {
            resolution: super::super::types::KnowledgeResolution {
                answerability: super::super::types::KnowledgeAnswerability::Unsupported,
                required_authority: super::super::types::KnowledgeRequiredAuthority::Customer,
                recommended_next_step: super::super::types::KnowledgeNextStep::ClarifyCustomer,
                missing_information: missing_information
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn defer_route(
        authority: super::super::types::KnowledgeRequiredAuthority,
    ) -> KnowledgeRouteResult {
        KnowledgeRouteResult {
            resolution: super::super::types::KnowledgeResolution {
                answerability: super::super::types::KnowledgeAnswerability::PartiallySupported,
                required_authority: authority,
                recommended_next_step: super::super::types::KnowledgeNextStep::Defer,
                missing_information: vec!["current verified record".to_string()],
                ..Default::default()
            },
            ..Default::default()
        }
    }

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

    #[test]
    fn typed_principal_route_requires_customer_reply_and_complete_action() {
        let route = principal_route();
        let mut decision = AgentDecision::default();
        assert_eq!(
            typed_route_consistency_issue(&route, &decision, true),
            Some(TypedRouteConsistencyIssue {
                handoff: KnowledgeHandoffKind::AskPrincipal,
                kind: TypedRouteConsistencyIssueKind::MissingCustomerReply,
            })
        );

        decision.should_reply = true;
        decision.reply_text = "I will verify this and follow up.".to_string();
        assert_eq!(
            typed_route_consistency_issue(&route, &decision, true),
            Some(TypedRouteConsistencyIssue {
                handoff: KnowledgeHandoffKind::AskPrincipal,
                kind: TypedRouteConsistencyIssueKind::MissingEscalationRequest,
            })
        );

        decision.next_step = "ask_principal".to_string();
        decision.escalation_request = Some(crate::models::EscalationRequest {
            needed: true,
            category: Some(crate::models::ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string()),
            reason: Some("Current authority is required".to_string()),
            question_for_principal: Some("What is the current state?".to_string()),
            self_serviceable_part: None,
            is_generalizable: false,
        });
        assert_eq!(typed_route_consistency_issue(&route, &decision, true), None);
    }

    #[test]
    fn typed_principal_route_is_inert_without_configured_capability() {
        assert_eq!(
            typed_route_consistency_issue(&principal_route(), &AgentDecision::default(), false),
            None
        );
    }

    #[test]
    fn principal_route_fallback_replaces_unsupported_reply_with_complete_action() {
        let route = principal_route();
        let mut decision = AgentDecision {
            should_reply: true,
            reply_text: "明天下午三点可以到店".to_string(),
            next_step: "respond".to_string(),
            appointment_request: Some(Default::default()),
            last_commitment: Some("明天下午三点到店".to_string()),
            operation_state: Some("appointment_pending".to_string()),
            follow_up: Some(Default::default()),
            ..Default::default()
        };

        assert!(materialize_typed_route_fallback(
            &mut decision,
            &route,
            true
        ));
        assert_eq!(
            decision.reply_text,
            super::super::escalation::fallback_holding_reply()
        );
        assert!(!decision.reply_text.contains("current state"));
        assert_eq!(decision.next_step, "ask_principal");
        assert_eq!(decision.autonomy_mode, "assisted");
        assert_eq!(decision.knowledge_need, "required");
        assert!(decision.appointment_request.is_none());
        assert!(decision.last_commitment.is_none());
        assert!(decision.operation_state.is_none());
        assert!(decision.follow_up.is_none());
        let request = decision
            .escalation_request
            .as_ref()
            .expect("fallback must preserve the principal action");
        assert!(request.needed);
        assert_eq!(
            request.category.as_deref(),
            Some(crate::models::ESCALATION_CATEGORY_OUT_OF_SCOPE)
        );
        assert!(request
            .reason
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()));
        assert!(request
            .question_for_principal
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()));
        assert_eq!(typed_route_consistency_issue(&route, &decision, true), None);
    }

    #[test]
    fn principal_route_fallback_does_not_activate_for_non_principal_route() {
        let route = KnowledgeRouteResult::default();
        let mut decision = AgentDecision {
            reply_text: "保持原样".to_string(),
            ..Default::default()
        };
        assert!(!materialize_typed_route_fallback(
            &mut decision,
            &route,
            true
        ));
        assert_eq!(decision.reply_text, "保持原样");
        assert!(decision.escalation_request.is_none());
    }

    #[test]
    fn typed_clarify_route_requires_a_pure_customer_question() {
        let route = clarify_route(vec!["preferred date", "location"]);
        let mut decision = AgentDecision {
            should_reply: true,
            reply_text: "Which date works for you?".to_string(),
            next_step: "respond".to_string(),
            ..Default::default()
        };
        assert_eq!(
            typed_route_consistency_issue(&route, &decision, true),
            Some(TypedRouteConsistencyIssue {
                handoff: KnowledgeHandoffKind::ClarifyCustomer,
                kind: TypedRouteConsistencyIssueKind::IncorrectNextStep,
            })
        );

        decision.next_step = "clarify".to_string();
        decision.commitment = Some(super::super::types::CommitmentDecision {
            text: "follow up after checking the schedule".to_string(),
            ..Default::default()
        });
        assert_eq!(
            typed_route_consistency_issue(&route, &decision, true),
            Some(TypedRouteConsistencyIssue {
                handoff: KnowledgeHandoffKind::ClarifyCustomer,
                kind: TypedRouteConsistencyIssueKind::ConflictingDeliverySideEffects,
            })
        );

        decision.commitment = None;
        assert_eq!(typed_route_consistency_issue(&route, &decision, true), None);
    }

    #[test]
    fn clarify_fallback_uses_structured_missing_information_and_clears_actions() {
        let route = clarify_route(vec!["你更方便的日期", "希望到访的地点"]);
        let mut decision = AgentDecision {
            should_reply: true,
            reply_text: "明天下午已经安排好了".to_string(),
            next_step: "respond".to_string(),
            appointment_request: Some(Default::default()),
            last_commitment: Some("明天下午确认".to_string()),
            assets_to_send: vec![Default::default()],
            ..Default::default()
        };

        assert!(materialize_typed_route_fallback(
            &mut decision,
            &route,
            true
        ));
        assert_eq!(decision.next_step, "clarify");
        assert!(!decision.reply_text.contains("你更方便的日期"));
        assert!(!decision.reply_text.contains("希望到访的地点"));
        assert_eq!(
            decision.clarification_intent,
            "你更方便的日期、希望到访的地点"
        );
        assert!(decision.appointment_request.is_none());
        assert!(decision.last_commitment.is_none());
        assert!(decision.assets_to_send.is_empty());
        assert!(decision.escalation_request.is_none());
        assert_eq!(typed_route_consistency_issue(&route, &decision, true), None);
    }

    #[test]
    fn clarify_fallback_handles_empty_missing_information_without_message_matching() {
        let route = clarify_route(Vec::new());
        let mut decision = AgentDecision::default();

        assert!(materialize_typed_route_fallback(
            &mut decision,
            &route,
            true
        ));
        assert_eq!(decision.next_step, "clarify");
        assert!(decision.reply_text.contains("最希望我先帮你确认"));
        assert_eq!(typed_route_consistency_issue(&route, &decision, true), None);
    }

    #[test]
    fn licensed_and_external_defer_routes_require_explicit_defer_action() {
        for (authority, expected_handoff) in [
            (
                super::super::types::KnowledgeRequiredAuthority::LicensedProfessional,
                KnowledgeHandoffKind::DeferLicensedProfessional,
            ),
            (
                super::super::types::KnowledgeRequiredAuthority::ExternalSystem,
                KnowledgeHandoffKind::DeferExternalSystem,
            ),
        ] {
            let route = defer_route(authority);
            let mut decision = AgentDecision {
                should_reply: true,
                reply_text: "I cannot confirm this yet.".to_string(),
                next_step: "respond".to_string(),
                ..Default::default()
            };
            assert_eq!(
                typed_route_consistency_issue(&route, &decision, true),
                Some(TypedRouteConsistencyIssue {
                    handoff: expected_handoff,
                    kind: TypedRouteConsistencyIssueKind::IncorrectNextStep,
                })
            );

            assert!(materialize_typed_route_fallback(
                &mut decision,
                &route,
                true
            ));
            assert_eq!(decision.next_step, "defer");
            assert!(!decision.reply_text.contains("current verified record"));
            assert!(decision.escalation_request.is_none());
            assert_eq!(typed_route_consistency_issue(&route, &decision, true), None);
        }
    }

    #[test]
    fn malformed_typed_route_is_not_materialized() {
        let route = KnowledgeRouteResult {
            resolution: super::super::types::KnowledgeResolution {
                answerability: super::super::types::KnowledgeAnswerability::Unsupported,
                required_authority: super::super::types::KnowledgeRequiredAuthority::ExternalSystem,
                recommended_next_step: super::super::types::KnowledgeNextStep::AskPrincipal,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut decision = AgentDecision::default();

        assert_eq!(typed_route_consistency_issue(&route, &decision, true), None);
        assert!(!materialize_typed_route_fallback(
            &mut decision,
            &route,
            true
        ));
    }

    #[test]
    fn harness_opened_verified_chunk_forces_full_review_without_a_preselected_route() {
        let route = KnowledgeRouteResult::default();
        let mut opened = HashSet::new();
        assert!(!requires_full_knowledge_review(&route, &opened));

        opened.insert("verified-chunk-id".to_string());
        assert!(
            requires_full_knowledge_review(&route, &opened),
            "a chunk opened by the Harness must not bypass the full Reviewer merely because the initial route was empty"
        );
    }

    #[test]
    fn repair_grants_only_the_missing_generation_and_required_authorization_capacity() {
        let budget = RunBudget::new("repair", 100_000, 5, 0);
        budget.try_reserve_llm_call().unwrap();
        budget.try_reserve_llm_call().unwrap();
        budget.try_reserve_llm_call().unwrap();

        assert!(ensure_repair_and_reauthorization_capacity(&budget));
        assert_eq!(budget.snapshot().llm_call_bonus, 1);

        budget.try_reserve_llm_call().unwrap(); // repaired generation
        budget.try_reserve_llm_call().unwrap(); // primary Reviewer
        budget.try_reserve_llm_call().unwrap(); // ClaimGate
        assert_eq!(budget.snapshot().llm_calls_used, 6);
        assert!(budget.try_reserve_llm_call().is_err());
    }

    #[test]
    fn repair_does_not_expand_an_exhausted_token_ceiling() {
        let budget = RunBudget::new("repair", 100, 5, 0);
        budget.record_call(100);

        assert!(
            !ensure_repair_and_reauthorization_capacity(&budget),
            "a bounded repair may add call slots, never token authority"
        );
        assert_eq!(budget.snapshot().llm_call_bonus, 0);
    }

    #[test]
    fn ask_principal_claim_repair_is_driven_by_the_structured_claim_gate_result() {
        let mut review = super::super::types::DecisionReviewResult {
            approved: false,
            rewrite_instruction: "remove unsupported schedule claim".to_string(),
            claim_analysis: doc! { "unsupportedBusinessClaimCount": 1_i64 },
            ..Default::default()
        };

        align_principal_claim_repair(&principal_route(), true, &mut review);

        assert!(review.rewrite_instruction.contains("ask_principal"));
        assert!(review
            .rewrite_instruction
            .contains("remove unsupported schedule claim"));
        assert!(!review.needs_revision);
    }
}

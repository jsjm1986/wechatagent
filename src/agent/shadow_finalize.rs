//! Read-only terminal aggregation for Shadow execution.
//!
//! This module reuses the live claim gate, final safety aggregator, and state
//! action policy. It intentionally does not persist finalize events, taxonomy
//! candidates, reviews, tasks, messages, or outbox entries.

use crate::error::AppResult;
use crate::models::{Contact, ConversationMessage, OperationKnowledgeChunk};
use crate::routes::AppState;

use super::decision::load_operation_state_policy_for_contact;
use super::guards::{classify_reviewed_decision_action, enforce_state_action_policy};
use super::review::{
    contact_has_principal_product_exemption, ensure_independent_claim_gate,
    finalize_review_for_send_at, GatewayStatusFinal,
};
use super::runtime::UserRuntimeParameters;
use super::types::{AgentDecision, DecisionReviewResult};

#[derive(Debug)]
pub(crate) struct ShadowFinalizeResult {
    pub decision: AgentDecision,
    pub review: DecisionReviewResult,
    pub final_status: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn finalize_shadow_decision(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    recent_messages: &[ConversationMessage],
    mut decision: AgentDecision,
    mut review: DecisionReviewResult,
    runtime: &UserRuntimeParameters,
    knowledge_chunks: &[OperationKnowledgeChunk],
    promote_risks: Vec<String>,
    run_id: &str,
) -> AppResult<ShadowFinalizeResult> {
    let shadow_snapshot = super::budget::current_shadow_evaluation_snapshot();
    let (active_profile, active_products, evaluated_at) = match shadow_snapshot {
        Some(snapshot) => (
            snapshot.active_profile.clone(),
            snapshot.active_products.clone(),
            snapshot.evaluated_at,
        ),
        None => {
            let profile =
                super::domain_profile::load_active_domain_profile(&state.db, &contact.workspace_id)
                    .await?;
            let products = if profile.transaction_facts_enabled {
                super::entitlements::load_active_products(&state.db, &contact.workspace_id).await
            } else {
                Vec::new()
            };
            (profile, products, mongodb::bson::DateTime::now())
        }
    };
    let priced_from_catalog = ensure_independent_claim_gate(
        state,
        contact,
        inbound,
        recent_messages,
        &decision,
        &mut review,
        knowledge_chunks,
        &active_products,
        &active_profile,
        evaluated_at,
        Some(run_id),
    )
    .await;
    let principal_product_exempted = contact_has_principal_product_exemption(contact);
    let outcome = finalize_review_for_send_at(
        review,
        &mut decision,
        runtime,
        contact,
        knowledge_chunks,
        promote_risks,
        inbound.content.as_str(),
        &active_profile.commitment_markers,
        priced_from_catalog,
        principal_product_exempted,
        evaluated_at,
    );

    // pending_events are diagnostic descriptions only. Persisting them would
    // make an evaluation mutate production business state.
    let mut review = outcome.review;
    let mut gateway_status = outcome.status;

    if matches!(gateway_status, GatewayStatusFinal::Approved) {
        let policy = load_operation_state_policy_for_contact(
            state,
            &contact.workspace_id,
            decision.operation_state.as_deref().unwrap_or(""),
            &contact.wxid,
        )
        .await?;
        let action = classify_reviewed_decision_action(&decision, &review);
        if enforce_state_action_policy(policy.as_ref(), action).is_err() {
            decision.should_reply = false;
            decision.autonomy_mode = "blocked".to_string();
            review.approved = false;
            review.final_review_status = "held_by_ai_policy".to_string();
            if !review
                .risks
                .iter()
                .any(|risk| risk == "state_action_policy_blocked")
            {
                review.risks.push("state_action_policy_blocked".to_string());
            }
            gateway_status = GatewayStatusFinal::Held("held_by_ai_policy".to_string());
        }
    }

    let final_status = shadow_terminal_status(&gateway_status, &decision, &review);
    review.final_review_status = final_status.clone();

    Ok(ShadowFinalizeResult {
        decision,
        review,
        final_status,
    })
}

fn shadow_terminal_status(
    gateway_status: &GatewayStatusFinal,
    decision: &AgentDecision,
    review: &DecisionReviewResult,
) -> String {
    if matches!(gateway_status, GatewayStatusFinal::Approved) {
        if !decision.should_reply {
            "no_reply".to_string()
        } else if review.needs_revision && !review.revision_direction.trim().is_empty() {
            // Live execution would now run its one-shot revision. Shadow does
            // not claim that the unrevised draft was sendable.
            "revision_required".to_string()
        } else {
            "approved".to_string()
        }
    } else {
        gateway_status.gateway_status_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrevised_draft_is_never_reported_as_approved() {
        let decision = AgentDecision {
            should_reply: true,
            ..Default::default()
        };
        let review = DecisionReviewResult {
            approved: true,
            needs_revision: true,
            revision_direction: "降低压迫感".to_string(),
            ..Default::default()
        };

        assert_eq!(
            shadow_terminal_status(&GatewayStatusFinal::Approved, &decision, &review),
            "revision_required"
        );
    }

    #[test]
    fn approved_silence_and_hard_hold_keep_distinct_terminals() {
        let silent = AgentDecision::default();
        let review = DecisionReviewResult {
            approved: true,
            ..Default::default()
        };
        assert_eq!(
            shadow_terminal_status(&GatewayStatusFinal::Approved, &silent, &review),
            "no_reply"
        );

        let reply = AgentDecision {
            should_reply: true,
            ..Default::default()
        };
        assert_eq!(
            shadow_terminal_status(
                &GatewayStatusFinal::Held("held_by_ai_policy".to_string()),
                &reply,
                &review,
            ),
            "held_by_ai_policy"
        );
    }
}

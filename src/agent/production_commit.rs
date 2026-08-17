//! Atomic production commit for one authorized Harness turn.
//!
//! Model-owned semantics end at `CommitPlan`. This module owns only deterministic authority:
//! task fencing, tenant scoping, lifecycle transitions, idempotency, appointment request shape,
//! commitment lifecycle, and the Durable Outbox transaction.

use std::sync::Arc;

use async_trait::async_trait;
use mongodb::{
    bson::{doc, oid::ObjectId, to_document, Bson, DateTime, Document},
    error::{ErrorKind, WriteFailure},
    ClientSession,
};

use crate::error::{AppError, AppResult};
use crate::models::{
    AgentDecisionReview, Appointment, CommitmentEntry, Contact, ContentAsset, ConversationMessage,
    DomainProfile, OperationDomainConfig, OperationPlaybook, ReferralCard,
};
use crate::prompts;
use crate::routes::AppState;
use crate::tasks::TaskRunContext;

use super::appointment_request::validate_appointment_request;
use super::authority::AuthoritySnapshot;
use super::budget::current_run_budget;
use super::commitment_lifecycle::build_commitment_transition_mutations;
use super::decision::{
    initial_operation_state_for_contact, load_operation_state_policy_for_contact,
};
use super::gateway::{
    apply_taxonomy_guard_outcome, compute_taxonomy_guard_outcome, pick_dimension_display_name,
    precheck_send_gateway, split_reply_into_segments,
};
use super::guards::{
    action_policy_state_key, decision_operation_state_candidate, enforce_reviewed_decision_actions,
    initial_operation_state_key, operation_states, planner_from_decision,
};
use super::model_turn::TurnCommitter;
use super::outbox::{prepare_outbox_entry, EnqueueRequest, PreparedOutboxEntry};
use super::review::review_passed;
use super::run_envelope::{
    assert_final_review_status_valid, assert_gateway_status_valid, assert_lifecycle_valid,
    derive_lifecycle_from_status, AgentRunLogTerminalFields, LIFECYCLE_RUNNING, LIFECYCLE_STARTED,
};
use super::runtime::UserRuntimeParameters;
use super::taxonomy::{global_taxonomy_cache, upsert_candidate as taxonomy_upsert_candidate};
use super::turn_loop::{
    authorization_projection_controls, AuthorizationManifest, CommitPlan, CommitReceipt,
    CommitResult,
};
use super::types::{AgentTrigger, KnowledgeRouteResult, SendGatewayResult};

const MAX_TRANSACTION_ATTEMPTS: usize = 4;

pub(crate) struct ProductionCommitInputs<'a> {
    pub state: &'a AppState,
    pub contact: &'a Contact,
    pub inbound: &'a ConversationMessage,
    pub trigger: AgentTrigger<'a>,
    pub task_context: Option<&'a TaskRunContext>,
    pub playbook: Option<&'a OperationPlaybook>,
    pub domain_config: Option<&'a OperationDomainConfig>,
    pub runtime: &'a UserRuntimeParameters,
    pub context_pack: &'a Document,
    pub knowledge_route: &'a KnowledgeRouteResult,
    pub active_profile: &'a DomainProfile,
    pub sendable_assets: &'a [ContentAsset],
    pub referral_cards: &'a [ReferralCard],
    pub source_event_id: &'a str,
    pub source_kind: &'a str,
    pub context_refreshed: bool,
    pub should_abort_send: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub authority: &'a AuthoritySnapshot,
}

pub(crate) struct ProductionCommitter<'a> {
    inputs: ProductionCommitInputs<'a>,
}

impl<'a> ProductionCommitter<'a> {
    pub(crate) fn new(inputs: ProductionCommitInputs<'a>) -> Self {
        Self { inputs }
    }

    async fn tighten_authorization(&self, plan: &mut CommitPlan) -> AppResult<PostCommitWork> {
        let mut post_commit = PostCommitWork::default();
        if plan.authorization.disposition != "authorized" {
            normalize_review_terminal(plan);
            return Ok(post_commit);
        }

        if self
            .inputs
            .should_abort_send
            .as_ref()
            .is_some_and(|guard| guard())
        {
            hold_plan(
                plan,
                "superseded_by_new_inbound",
                None,
                "a newer inbound superseded this turn before commit",
                "superseded_by_new_inbound",
            );
            return Ok(post_commit);
        }

        let cache = global_taxonomy_cache(&self.inputs.state.db);
        cache
            .find_or_load(&self.inputs.state.db, &self.inputs.contact.workspace_id)
            .await?;
        let dimension_kinds =
            super::domain_profile::decision_dimension_kinds(self.inputs.active_profile);
        let fsm_customer_stage_keys = operation_states(self.inputs.domain_config)
            .into_iter()
            .filter_map(|state| state.get_str("key").ok().map(ToString::to_string))
            .collect::<Vec<_>>();
        let taxonomy = compute_taxonomy_guard_outcome(
            &plan.draft.decision,
            &dimension_kinds,
            &fsm_customer_stage_keys,
            &self.inputs.contact.workspace_id,
            &self.inputs.contact.account_id,
            &cache,
        );
        post_commit.taxonomy_candidates = taxonomy
            .candidate_writes
            .iter()
            .map(|(kind, raw)| TaxonomyCandidateWork {
                kind: kind.clone(),
                raw: raw.clone(),
                display_name: pick_dimension_display_name(
                    &plan.draft.decision.dimension_display_names,
                    kind,
                )
                .map(ToString::to_string),
            })
            .collect();
        apply_taxonomy_guard_outcome(
            &mut plan.draft.decision,
            &mut plan.authorization.review,
            &taxonomy,
        );

        let source_operation_state = action_policy_state_key(
            self.inputs.domain_config,
            self.inputs.contact.operation_state.as_deref(),
            None,
        )
        .unwrap_or_else(|| initial_operation_state_key(self.inputs.domain_config));
        post_commit.source_operation_state = Some(source_operation_state);
        let operation_state = action_policy_state_key(
            self.inputs.domain_config,
            self.inputs.contact.operation_state.as_deref(),
            decision_operation_state_candidate(&plan.draft.decision),
        );
        let operation_state = match operation_state {
            Some(value) => value,
            None => {
                initial_operation_state_for_contact(self.inputs.state, self.inputs.contact).await?
            }
        };
        post_commit.domain_version = self.inputs.domain_config.map(|config| config.version);
        let policy = load_operation_state_policy_for_contact(
            self.inputs.state,
            &self.inputs.contact.workspace_id,
            &operation_state,
            &self.inputs.contact.wxid,
        )
        .await?;
        post_commit.operation_state = Some(operation_state.clone());
        // `operation_state` above is the state whose policy governs this turn.  It is not always
        // a requested transition: when the model omitted a proposal, it is simply the current
        // (or initial) policy state. Freeze a projection target only when the model's proposal
        // survived the deterministic state-machine resolution.
        post_commit.target_operation_state =
            decision_operation_state_candidate(&plan.draft.decision)
                .filter(|candidate| operation_state == *candidate)
                .map(ToString::to_string);
        post_commit.policy_version = policy.as_ref().map(|item| item.version);
        let actions = super::guards::reviewed_decision_actions(
            &plan.draft.decision,
            &plan.authorization.review,
        );
        if let Err((action, reason)) = enforce_reviewed_decision_actions(
            policy.as_ref(),
            &plan.draft.decision,
            &plan.authorization.review,
        ) {
            post_commit.state_action_hold = Some(doc! {
                "actions": actions.clone(),
                "action": action,
                "operation_state": operation_state,
                "reason": &reason,
            });
            hold_plan(
                plan,
                "held_by_ai_policy",
                Some("held_by_ai_policy"),
                &reason,
                "state_action_policy_blocked",
            );
            return Ok(post_commit);
        }

        let final_precheck = precheck_send_gateway(
            self.inputs.state,
            self.inputs.contact,
            &self.inputs.trigger,
            self.inputs.runtime,
        )
        .await?;
        if plan.draft.decision.should_reply && !final_precheck.allowed {
            let status = if final_precheck.status == "quiet_hours_deferred" {
                "quiet_hours_deferred"
            } else {
                "gateway_blocked"
            };
            post_commit.gateway_block = Some(to_document(&final_precheck).unwrap_or_default());
            hold_plan(
                plan,
                status,
                None,
                &final_precheck.reason,
                "final_send_precheck_blocked",
            );
            return Ok(post_commit);
        }

        if super::escalation::is_principal_relay_trigger(&self.inputs.trigger)
            && super::escalation::relay_output_leaks_internal_payload(
                &plan.draft.decision.reply_text,
            )
        {
            hold_plan(
                plan,
                "blocked_by_safety_guard",
                Some("blocked_by_safety_guard"),
                "principal relay output contained an internal payload marker",
                "principal_relay_internal_payload",
            );
        }
        normalize_review_terminal(plan);
        Ok(post_commit)
    }

    async fn prepare_commit(
        &self,
        plan: &CommitPlan,
        decision_id: ObjectId,
        now: DateTime,
        post_commit: PostCommitWork,
    ) -> AppResult<PreparedCommit> {
        let mut prompt_versions = prompts::prompt_versions(
            &self.inputs.state.db,
            &self.inputs.contact.workspace_id,
            &[
                "user.reply.system",
                "user.reply.policy",
                "user.reply.fast.task",
                "user.persona_world_state.system",
                "user.knowledge.router",
                "user.review.system",
                "user.review.light.system",
                "user.memory_consolidator.system",
                "user.memory_consolidator.task",
            ],
            Some("user"),
            self.inputs.playbook,
        )
        .await?;
        if let Some(budget) = current_run_budget() {
            for (key, value) in budget.prompt_versions() {
                prompt_versions.insert(key, value);
            }
        }

        let mut outbox = Vec::new();
        let mut skipped_media = Vec::new();
        let mut principal_media = Vec::new();
        let send_authorized = plan.authorization.disposition == "authorized"
            && plan.draft.decision.should_reply
            && !plan.draft.decision.reply_text.trim().is_empty();
        if send_authorized {
            let segments = split_reply_into_segments(
                &plan.draft.decision.reply_text,
                self.inputs.state.config.agent_reply_max_segment_chars,
                self.inputs.state.config.agent_reply_max_segments,
            );
            let segment_total = segments.len();
            for (index, content) in segments.into_iter().enumerate() {
                let source_event_id = if segment_total > 1 {
                    format!(
                        "{}#seg{index}",
                        segment_idempotency_base(self.inputs.source_event_id, &plan.run_id)
                    )
                } else {
                    self.inputs.source_event_id.to_string()
                };
                outbox.push(prepare_outbox_entry(
                    &EnqueueRequest {
                        workspace_id: self.inputs.contact.workspace_id.clone(),
                        account_id: self.inputs.contact.account_id.clone(),
                        contact_wxid: self.inputs.contact.wxid.clone(),
                        run_id: plan.run_id.clone(),
                        decision_id: Some(decision_id),
                        source_event_id,
                        source_kind: self.inputs.trigger.kind().to_string(),
                        content,
                        media_asset_id: None,
                        referral_card_id: None,
                        max_attempts: 3,
                    },
                    now,
                    ObjectId::new(),
                )?);
            }

            for (index, directive) in plan.draft.decision.assets_to_send.iter().enumerate() {
                let asset = ObjectId::parse_str(&directive.asset_id)
                    .ok()
                    .and_then(|id| {
                        self.inputs
                            .sendable_assets
                            .iter()
                            .find(|asset| asset.id == Some(id))
                    })
                    .filter(|asset| {
                        asset.workspace_id == self.inputs.contact.workspace_id
                            && super::media_send::validate_asset_sendable(asset)
                    });
                let Some(asset) = asset else {
                    skipped_media.push(directive.asset_id.clone());
                    continue;
                };
                if asset.requires_principal_approval == Some(true) {
                    principal_media.push(directive.asset_id.clone());
                    continue;
                }
                let mut prepared = prepare_outbox_entry(
                    &EnqueueRequest {
                        workspace_id: self.inputs.contact.workspace_id.clone(),
                        account_id: self.inputs.contact.account_id.clone(),
                        contact_wxid: self.inputs.contact.wxid.clone(),
                        run_id: plan.run_id.clone(),
                        decision_id: Some(decision_id),
                        source_event_id: self.inputs.source_event_id.to_string(),
                        source_kind: self.inputs.trigger.kind().to_string(),
                        content: String::new(),
                        media_asset_id: Some(directive.asset_id.clone()),
                        referral_card_id: None,
                        max_attempts: 3,
                    },
                    now,
                    ObjectId::new(),
                )?;
                prepared.entry.run_sequence = 10_000 + index as i32;
                outbox.push(prepared);
            }

            if let Some(directive) = plan.draft.decision.namecard_to_send.as_ref() {
                let card = ObjectId::parse_str(&directive.card_id)
                    .ok()
                    .and_then(|id| {
                        self.inputs
                            .referral_cards
                            .iter()
                            .find(|card| card.id == Some(id))
                    })
                    .filter(|card| {
                        card.workspace_id == self.inputs.contact.workspace_id
                            && super::referral::validate_card_sendable(
                                card,
                                &self.inputs.contact.account_id,
                            )
                    });
                if card.is_some() {
                    outbox.push(prepare_outbox_entry(
                        &EnqueueRequest {
                            workspace_id: self.inputs.contact.workspace_id.clone(),
                            account_id: self.inputs.contact.account_id.clone(),
                            contact_wxid: self.inputs.contact.wxid.clone(),
                            run_id: plan.run_id.clone(),
                            decision_id: Some(decision_id),
                            source_event_id: format!("{}#namecard", self.inputs.source_event_id),
                            source_kind: self.inputs.trigger.kind().to_string(),
                            content: String::new(),
                            media_asset_id: None,
                            referral_card_id: Some(directive.card_id.clone()),
                            max_attempts: 3,
                        },
                        now,
                        ObjectId::new(),
                    )?);
                } else {
                    skipped_media.push(directive.card_id.clone());
                }
            }
        }

        if let Some(claim) = self
            .inputs
            .task_context
            .and_then(|context| context.claim.as_ref())
        {
            for prepared in &mut outbox {
                prepared.entry.task_send_authorization_token = Some(claim.claim_token.clone());
            }
        }

        let appointment = if plan.authorization.disposition == "authorized" {
            build_appointment(plan, self.inputs.contact, now)?
        } else {
            None
        };
        // A commitment is tied to a customer-visible text promise. Media/namecard rows are
        // independent side effects and the delivery finalizer intentionally does not promote
        // them, so they must never create a pending commitment that can no longer converge.
        let commitment = if has_text_outbox(&outbox) {
            build_commitment(plan, now)
        } else {
            None
        };

        Ok(PreparedCommit {
            decision_id,
            prompt_versions,
            outbox,
            appointment,
            commitment,
            skipped_media,
            principal_media,
            post_commit,
        })
    }

    async fn commit_once(
        &self,
        plan: &mut CommitPlan,
        prepared: &PreparedCommit,
        session: &mut ClientSession,
    ) -> Result<CommitReceipt, AttemptError> {
        let now = DateTime::now();
        let mut transaction_state_action_hold: Option<Document> = None;
        let mut task_snapshot = None;
        let mut task_owned = true;
        if let Some(context) = self.inputs.task_context {
            let filter = context
                .claim
                .as_ref()
                .map(crate::tasks::TaskClaim::owned_running_filter)
                .unwrap_or_else(|| doc! { "_id": context.task_id });
            task_snapshot = self
                .inputs
                .state
                .db
                .tasks()
                .clone_with_type::<Document>()
                .find_one_with_session(filter, None, session)
                .await?;
            task_owned = task_snapshot.is_some();
            if !task_owned {
                hold_plan(
                    plan,
                    "stale_task_claim",
                    None,
                    "task ownership changed before atomic commit",
                    "stale_task_claim",
                );
            }
        }

        let mut selected_outbox = if plan.authorization.disposition == "authorized" && task_owned {
            prepared
                .outbox
                .iter()
                .map(|item| item.entry.clone())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut existing_outbox_ids = Vec::new();
        for entry in &selected_outbox {
            if let Some(existing) = self
                .inputs
                .state
                .db
                .collection_agent_send_outbox()
                .find_one_with_session(
                    doc! {
                        "workspace_id": &entry.workspace_id,
                        "account_id": &entry.account_id,
                        "idempotency_key": &entry.idempotency_key,
                    },
                    None,
                    session,
                )
                .await?
            {
                existing_outbox_ids.push(existing.id.ok_or_else(|| {
                    AppError::External("existing outbox row missing _id".to_string())
                })?);
            }
        }

        let duplicate_batch =
            !selected_outbox.is_empty() && existing_outbox_ids.len() == selected_outbox.len();
        if duplicate_batch {
            selected_outbox.clear();
        } else if !existing_outbox_ids.is_empty() {
            selected_outbox.clear();
            hold_plan(
                plan,
                "blocked_by_safety_guard",
                Some("blocked_by_safety_guard"),
                "outbox idempotency snapshot contained only part of the reply batch",
                "outbox_partial_batch_conflict",
            );
        }

        let mut duplicate_batch = duplicate_batch;
        let mut terminal_status =
            terminal_status(plan, !selected_outbox.is_empty(), duplicate_batch);
        let mut gateway = gateway_result(&terminal_status, plan);

        let mut appointment_will_commit = plan.authorization.disposition == "authorized"
            && !duplicate_batch
            && prepared.appointment.is_some();
        let mut lifecycle_will_commit = plan.authorization.disposition == "authorized"
            && !duplicate_batch
            && selected_outbox.is_empty()
            && !plan.draft.decision.commitment_updates.is_empty();
        if !selected_outbox.is_empty() || appointment_will_commit || lifecycle_will_commit {
            let mut contact_filter = doc! {
                "workspace_id": &self.inputs.contact.workspace_id,
                "account_id": &self.inputs.contact.account_id,
                "wxid": &self.inputs.contact.wxid,
            };
            if let Some(id) = self.inputs.contact.id {
                contact_filter.insert("_id", id);
            }
            let durable_contact = self
                .inputs
                .state
                .db
                .contacts()
                .find_one_with_session(contact_filter, None, session)
                .await?;
            let Some(durable_contact) = durable_contact else {
                return Err(AppError::Conflict(
                    "durable contact missing before atomic side-effect commit".to_string(),
                )
                .into());
            };
            // Revalidate both configuration versions used by the pre-transaction authorization.
            // A publish can change the state machine without touching this contact; allowing the
            // old decision to commit under the new machine would be the same TOCTOU as a contact
            // state race. The post-delivery worker performs the same check for deferred controls,
            // while this transaction protects the reply/appointment side effects themselves.
            let current_domain_config = self
                .inputs
                .state
                .db
                .operation_domain_configs()
                .find_one_with_session(
                    doc! {
                        "workspace_id": &self.inputs.contact.workspace_id,
                        "domain": "user_operations",
                        "current_version": true,
                    },
                    None,
                    session,
                )
                .await?;
            let domain_version_matches = versions_match(
                prepared.post_commit.domain_version,
                current_domain_config.as_ref().map(|config| config.version),
            );
            if !domain_version_matches {
                let expected_version = prepared.post_commit.domain_version;
                let current_version = current_domain_config.as_ref().map(|config| config.version);
                hold_plan(
                    plan,
                    "held_by_ai_policy",
                    Some("held_by_ai_policy"),
                    "operation domain changed before atomic commit; authorization must be recomputed",
                    "operation_domain_changed_before_commit",
                );
                selected_outbox.clear();
                duplicate_batch = false;
                appointment_will_commit = false;
                lifecycle_will_commit = false;
                terminal_status = self::terminal_status(plan, false, false);
                gateway = gateway_result(&terminal_status, plan);
                transaction_state_action_hold = Some(doc! {
                    "transaction_revalidated": true,
                    "reason": "operation_domain_changed_before_commit",
                    "expected_domain_version": expected_version.map(Bson::Int32).unwrap_or(Bson::Null),
                    "current_domain_version": current_version.map(Bson::Int32).unwrap_or(Bson::Null),
                });
                gateway
                    .policy_blocks
                    .push("operation_domain_changed_before_commit".to_string());
            // Revalidate the operation state used for the pre-transaction policy decision. The
            // contact write below repeats this predicate as a CAS, so a concurrent state change
            // cannot turn stale authorization into an outbox or appointment side effect.
            } else if durable_contact.operation_state != self.inputs.contact.operation_state {
                hold_plan(
                    plan,
                    "held_by_ai_policy",
                    Some("held_by_ai_policy"),
                    "operation state changed before atomic commit; authorization must be recomputed",
                    "operation_state_changed_before_commit",
                );
                selected_outbox.clear();
                duplicate_batch = false;
                appointment_will_commit = false;
                lifecycle_will_commit = false;
                terminal_status = self::terminal_status(plan, false, false);
                gateway = gateway_result(&terminal_status, plan);
            } else if plan.authorization.disposition == "authorized" {
                // Re-read the current policy inside the transaction. A policy publish can happen
                // without changing the contact state; version matching closes that second TOCTOU
                // window left by a pre-transaction policy lookup.
                if let Some(operation_state) = prepared.post_commit.operation_state.as_deref() {
                    let current_policy = self
                        .inputs
                        .state
                        .db
                        .operation_state_policies()
                        .find_one_with_session(
                            doc! {
                                "workspace_id": &self.inputs.contact.workspace_id,
                                "domain": "user_operations",
                                "state_key": operation_state,
                                "current_version": true,
                            },
                            None,
                            session,
                        )
                        .await?;
                    let policy_version_matches =
                        match (prepared.post_commit.policy_version, current_policy.as_ref()) {
                            (Some(expected), Some(current)) => {
                                current.status == "active" && current.version == expected
                            }
                            (Some(_), None) => false,
                            (None, Some(_)) => false,
                            (None, None) => true,
                        };
                    let policy_action_error = current_policy.as_ref().and_then(|policy| {
                        enforce_reviewed_decision_actions(
                            Some(policy),
                            &plan.draft.decision,
                            &plan.authorization.review,
                        )
                        .err()
                    });
                    if !policy_version_matches || policy_action_error.is_some() {
                        let (action, reason) = policy_action_error.unwrap_or_else(|| {
                            (
                                "reply",
                                "operation-state policy changed before atomic commit".to_string(),
                            )
                        });
                        plan.authorization
                            .review
                            .risks
                            .push("operation_state_policy_changed_before_commit".to_string());
                        hold_plan(
                            plan,
                            "held_by_ai_policy",
                            Some("held_by_ai_policy"),
                            &reason,
                            "state_action_policy_changed_before_commit",
                        );
                        selected_outbox.clear();
                        duplicate_batch = false;
                        appointment_will_commit = false;
                        lifecycle_will_commit = false;
                        terminal_status = self::terminal_status(plan, false, false);
                        gateway = gateway_result(&terminal_status, plan);
                        let actions = super::guards::reviewed_decision_actions(
                            &plan.draft.decision,
                            &plan.authorization.review,
                        );
                        // Keep the first-stage policy audit and append the transaction-time
                        // revalidation result for operators diagnosing a race.
                        let mut hold = prepared
                            .post_commit
                            .state_action_hold
                            .clone()
                            .unwrap_or_default();
                        hold.insert("transaction_revalidated", true);
                        hold.insert("actions", actions);
                        hold.insert("action", action);
                        hold.insert("operation_state", operation_state);
                        hold.insert("reason", reason);
                        transaction_state_action_hold = Some(hold);
                        // `PreparedCommit` is immutable by design; the final receipt carries the
                        // transaction-time fact through the gateway details below.
                        gateway
                            .policy_blocks
                            .push("operation_state_policy_changed_before_commit".to_string());
                    }
                }
            }
        }

        if task_owned {
            if let Some(context) = self.inputs.task_context {
                let filter = context
                    .claim
                    .as_ref()
                    .map(crate::tasks::TaskClaim::owned_running_filter)
                    .unwrap_or_else(|| doc! { "_id": context.task_id });
                let update =
                    task_terminal_update(&terminal_status, prepared.decision_id, &self.inputs, now);
                let result = self
                    .inputs
                    .state
                    .db
                    .tasks()
                    .update_one_with_session(filter, update, None, session)
                    .await?;
                if result.matched_count != 1 {
                    return Err(AttemptError::TaskBecameStale);
                }
            }
        }

        let review_doc = build_decision_review_document(
            &self.inputs,
            plan,
            prepared,
            task_snapshot.as_ref(),
            &gateway,
            &terminal_status,
            selected_outbox
                .iter()
                .filter(|entry| entry.media_asset_id.is_none() && entry.referral_card_id.is_none())
                .count() as i32,
            commitment_should_be_persisted(prepared.commitment.as_ref(), &selected_outbox),
            now,
        )?;
        self.inputs
            .state
            .db
            .decision_reviews()
            .clone_with_type::<Document>()
            .insert_one_with_session(review_doc, None, session)
            .await?;

        let mut mutation_count = 0usize;
        let mut appointment_id = None;
        let mut appointment_created = false;
        if plan.authorization.disposition == "authorized" && !duplicate_batch {
            if let Some(appointment) = prepared.appointment.as_ref() {
                let existing = self
                    .inputs
                    .state
                    .db
                    .appointments()
                    .find_one_with_session(
                        doc! {
                            "workspace_id": &appointment.workspace_id,
                            "account_id": &appointment.account_id,
                            "idempotency_key": &appointment.idempotency_key,
                        },
                        None,
                        session,
                    )
                    .await?;
                if let Some(existing) = existing {
                    appointment_id = existing.id.map(|id| id.to_hex());
                } else {
                    self.inputs
                        .state
                        .db
                        .appointments()
                        .insert_one_with_session(appointment, None, session)
                        .await?;
                    appointment_id = appointment.id.map(|id| id.to_hex());
                    appointment_created = true;
                    mutation_count += 1;
                }
            }
        }

        let mut commitment_ids = Vec::new();
        let mut lifecycle_transition_ids = Vec::new();
        let mut lifecycle_stale_ids = Vec::new();
        if !selected_outbox.is_empty() {
            let mut update = doc! {
                "$max": { "last_agent_run_at": now },
                "$set": { "updated_at": now },
            };
            if let Some(commitment) = prepared.commitment.as_ref() {
                let commitment_bson = mongodb::bson::to_bson(commitment)?;
                update.insert(
                    "$push",
                    doc! { "commitments": {
                        "$each": [commitment_bson],
                        "$slice": -8i32,
                    } },
                );
                commitment_ids.push(commitment.id.clone());
            }
            let mut contact_filter = doc! {
                "workspace_id": &self.inputs.contact.workspace_id,
                "account_id": &self.inputs.contact.account_id,
                "wxid": &self.inputs.contact.wxid,
            };
            if let Some(id) = self.inputs.contact.id {
                contact_filter.insert("_id", id);
            }
            if let Some(commitment) = prepared.commitment.as_ref() {
                contact_filter.insert("commitments.id", doc! { "$ne": &commitment.id });
            }
            match self.inputs.contact.operation_state.as_deref() {
                Some(state) => contact_filter.insert("operation_state", state),
                None => contact_filter.insert("operation_state", Bson::Null),
            };
            let result = self
                .inputs
                .state
                .db
                .contacts()
                .update_one_with_session(contact_filter, update, None, session)
                .await?;
            if result.matched_count != 1 {
                return Err(
                    AppError::Conflict("contact changed before reply commit".to_string()).into(),
                );
            }
            mutation_count += 1;
        } else if appointment_will_commit {
            // Appointment-only decisions still need a write-side CAS on the contact. A read
            // alone would not make MongoDB detect a concurrent operation-state transition.
            let mut contact_filter = doc! {
                "workspace_id": &self.inputs.contact.workspace_id,
                "account_id": &self.inputs.contact.account_id,
                "wxid": &self.inputs.contact.wxid,
            };
            if let Some(id) = self.inputs.contact.id {
                contact_filter.insert("_id", id);
            }
            match self.inputs.contact.operation_state.as_deref() {
                Some(state) => contact_filter.insert("operation_state", state),
                None => contact_filter.insert("operation_state", Bson::Null),
            };
            let result = self
                .inputs
                .state
                .db
                .contacts()
                .update_one_with_session(
                    contact_filter,
                    doc! { "$set": { "updated_at": now } },
                    None,
                    session,
                )
                .await?;
            if result.matched_count != 1 {
                return Err(AppError::Conflict(
                    "contact operation state changed before appointment commit".to_string(),
                )
                .into());
            }
            mutation_count += 1;
        }

        if lifecycle_will_commit {
            let mutations = build_commitment_transition_mutations(
                &self.inputs.contact.workspace_id,
                &self.inputs.contact.account_id,
                &self.inputs.contact.wxid,
                &plan.draft.decision.commitment_updates,
                None,
                &prepared.decision_id.to_hex(),
                now,
            )
            .map_err(AppError::External)?;
            for mut mutation in mutations {
                if let Some(id) = self.inputs.contact.id {
                    mutation.filter.insert("_id", id);
                }
                let result = self
                    .inputs
                    .state
                    .db
                    .contacts()
                    .update_one_with_session(mutation.filter, mutation.pipeline, None, session)
                    .await?;
                if result.matched_count == 1 {
                    lifecycle_transition_ids.push(mutation.commitment_id);
                    mutation_count += 1;
                } else {
                    // Another authorized turn may have terminalized the same active row first.
                    // The stale transition remains auditable but must not invalidate this run.
                    lifecycle_stale_ids.push(mutation.commitment_id);
                }
            }
            self.inputs
                .state
                .db
                .decision_reviews()
                .clone_with_type::<Document>()
                .update_one_with_session(
                    doc! { "_id": prepared.decision_id },
                    doc! { "$set": {
                        "commitment_lifecycle_applied_at": now,
                        "commitment_lifecycle_transition_ids": &lifecycle_transition_ids,
                        "commitment_lifecycle_stale_ids": &lifecycle_stale_ids,
                    } },
                    None,
                    session,
                )
                .await?;
        }

        let mut outbox_ids = Vec::new();
        for entry in &selected_outbox {
            self.inputs
                .state
                .db
                .collection_agent_send_outbox()
                .insert_one_with_session(entry, None, session)
                .await?;
            if let Some(id) = entry.id {
                outbox_ids.push(id.to_hex());
            }
        }

        let run_set = build_run_terminal_set(
            &self.inputs,
            plan,
            &gateway,
            &terminal_status,
            !selected_outbox.is_empty(),
            now,
        )?;
        let result = self
            .inputs
            .state
            .db
            .agent_run_logs()
            .update_one_with_session(
                doc! {
                    "run_id": &plan.run_id,
                    "lifecycle": { "$in": [LIFECYCLE_STARTED, LIFECYCLE_RUNNING] },
                },
                doc! { "$set": run_set },
                None,
                session,
            )
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::Conflict(
                "run envelope was already terminal before commit".to_string(),
            )
            .into());
        }

        let receipt_status = if !selected_outbox.is_empty()
            || appointment_created
            || !lifecycle_transition_ids.is_empty()
        {
            "committed"
        } else if terminal_status == "no_reply" || duplicate_batch {
            "no_op"
        } else {
            "held"
        };
        let receipt = CommitReceipt {
            status: receipt_status.to_string(),
            environment: "production".to_string(),
            committed_at: now,
            outbox_ids,
            appointment_id,
            commitment_ids,
            mutation_count,
            details: doc! {
                "gateway_status": &terminal_status,
                "decision_review_id": prepared.decision_id.to_hex(),
                "duplicate_outbox_ids": existing_outbox_ids,
                "taxonomy_candidates": prepared.post_commit.taxonomy_documents(),
                "skipped_media_ids": &prepared.skipped_media,
                "principal_media_ids": &prepared.principal_media,
                "state_action_hold": transaction_state_action_hold
                    .or_else(|| prepared.post_commit.state_action_hold.clone()),
                "gateway_block": prepared.post_commit.gateway_block.clone(),
                "appointment_created": appointment_created,
                "commitment_lifecycle_transition_ids": lifecycle_transition_ids,
                "commitment_lifecycle_stale_ids": lifecycle_stale_ids,
                "projection_eligible": terminal_status == "outbox_enqueued" || terminal_status == "no_reply",
            },
        };
        self.inputs
            .authority
            .persist_commit_state_with_session(
                &self.inputs.state.db,
                session,
                plan.authorization.to_document(),
                receipt.to_document(),
            )
            .await?;
        Ok(receipt)
    }

    pub(crate) async fn persist_post_commit_work(&self, receipt: &CommitReceipt) {
        let candidates = receipt
            .details
            .get_array("taxonomy_candidates")
            .ok()
            .cloned()
            .unwrap_or_default();
        for candidate in candidates {
            let Some(candidate) = candidate.as_document() else {
                continue;
            };
            let Ok(kind) = candidate.get_str("kind") else {
                continue;
            };
            let Ok(raw) = candidate.get_str("raw") else {
                continue;
            };
            let display_name = candidate.get_str("display_name").ok();
            if let Err(error) = taxonomy_upsert_candidate(
                &self.inputs.state.db,
                &self.inputs.contact.workspace_id,
                &self.inputs.contact.account_id,
                kind,
                raw,
                Some("atomic production Harness commit"),
                50,
                display_name,
            )
            .await
            {
                tracing::warn!(%error, kind, raw, "post-commit taxonomy candidate failed");
            }
        }
    }
}

#[async_trait]
impl TurnCommitter for ProductionCommitter<'_> {
    async fn commit(&mut self, mut plan: CommitPlan) -> AppResult<CommitResult> {
        let post_commit = self.tighten_authorization(&mut plan).await?;
        let decision_id = ObjectId::new();
        let now = DateTime::now();
        let prepared = self
            .prepare_commit(&plan, decision_id, now, post_commit)
            .await?;

        for attempt in 0..MAX_TRANSACTION_ATTEMPTS {
            let mut session = self.inputs.state.db.client().start_session(None).await?;
            session.start_transaction(None).await?;
            match self.commit_once(&mut plan, &prepared, &mut session).await {
                Ok(receipt) => match commit_transaction(&mut session).await {
                    Ok(()) => {
                        if !receipt.outbox_ids.is_empty() {
                            super::outbox_dispatcher::notify_outbox_work();
                        }
                        return Ok(CommitResult { plan, receipt });
                    }
                    Err(error)
                        if attempt + 1 < MAX_TRANSACTION_ATTEMPTS && retryable_db(&error) =>
                    {
                        tokio::time::sleep(retry_delay(attempt)).await;
                    }
                    Err(error) => return Err(error.into()),
                },
                Err(AttemptError::TaskBecameStale) => {
                    let _ = session.abort_transaction().await;
                    hold_plan(
                        &mut plan,
                        "stale_task_claim",
                        None,
                        "task ownership changed during atomic commit",
                        "stale_task_claim",
                    );
                    if attempt + 1 < MAX_TRANSACTION_ATTEMPTS {
                        continue;
                    }
                    return Err(AppError::Conflict(
                        "task ownership repeatedly changed during commit".to_string(),
                    ));
                }
                Err(AttemptError::App(error)) => {
                    let retryable = matches!(&error, AppError::Db(error) if retryable_db(error));
                    let _ = session.abort_transaction().await;
                    if retryable && attempt + 1 < MAX_TRANSACTION_ATTEMPTS {
                        tokio::time::sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        unreachable!("bounded production commit loop always returns")
    }
}

#[derive(Debug, Default)]
struct PostCommitWork {
    taxonomy_candidates: Vec<TaxonomyCandidateWork>,
    state_action_hold: Option<Document>,
    gateway_block: Option<Document>,
    operation_state: Option<String>,
    target_operation_state: Option<String>,
    policy_version: Option<i32>,
    domain_version: Option<i32>,
    source_operation_state: Option<String>,
}

impl PostCommitWork {
    fn taxonomy_documents(&self) -> Vec<Bson> {
        self.taxonomy_candidates
            .iter()
            .map(|candidate| {
                Bson::Document(doc! {
                    "kind": &candidate.kind,
                    "raw": &candidate.raw,
                    "display_name": candidate.display_name.clone(),
                })
            })
            .collect()
    }
}

#[derive(Debug)]
struct TaxonomyCandidateWork {
    kind: String,
    raw: String,
    display_name: Option<String>,
}

#[derive(Debug)]
struct PreparedCommit {
    decision_id: ObjectId,
    prompt_versions: Document,
    outbox: Vec<PreparedOutboxEntry>,
    appointment: Option<Appointment>,
    commitment: Option<CommitmentEntry>,
    skipped_media: Vec<String>,
    principal_media: Vec<String>,
    post_commit: PostCommitWork,
}

enum AttemptError {
    TaskBecameStale,
    App(AppError),
}

impl From<AppError> for AttemptError {
    fn from(value: AppError) -> Self {
        Self::App(value)
    }
}

impl From<mongodb::error::Error> for AttemptError {
    fn from(value: mongodb::error::Error) -> Self {
        Self::App(value.into())
    }
}

impl From<mongodb::bson::ser::Error> for AttemptError {
    fn from(value: mongodb::bson::ser::Error) -> Self {
        Self::App(value.into())
    }
}

fn hold_plan(
    plan: &mut CommitPlan,
    gateway_status: &str,
    review_status: Option<&str>,
    reason: &str,
    risk: &str,
) {
    let mut review = plan.authorization.review.clone();
    if let Some(review_status) = review_status {
        review.approved = false;
        review.should_hold = true;
        review.final_review_status = review_status.to_string();
    }
    if !risk.is_empty() && !review.risks.iter().any(|existing| existing == risk) {
        review.risks.push(risk.to_string());
    }
    if review.review_summary.trim().is_empty() {
        review.review_summary = reason.to_string();
    }
    plan.draft.decision.should_reply = false;
    plan.draft.decision.autonomy_mode = "blocked".to_string();
    plan.authorization = AuthorizationManifest::held(gateway_status, reason, review);
}

fn normalize_review_terminal(plan: &mut CommitPlan) {
    let review = &mut plan.authorization.review;
    if review.final_review_status.trim().is_empty() {
        review.final_review_status = if plan.authorization.disposition == "authorized" {
            "approved".to_string()
        } else {
            "held_by_ai_policy".to_string()
        };
    }
    if assert_final_review_status_valid(&review.final_review_status).is_err() {
        review.approved = false;
        review.should_hold = true;
        review.final_review_status = "held_by_ai_policy".to_string();
        if !review
            .risks
            .iter()
            .any(|risk| risk == "invalid_final_review_status_coerced")
        {
            review
                .risks
                .push("invalid_final_review_status_coerced".to_string());
        }
    }
    let (claims, evidence_status) = {
        let manifest = AuthorizationManifest::held(
            plan.authorization.final_status.clone(),
            plan.authorization.reason.clone(),
            review.clone(),
        );
        (manifest.claim_manifest, manifest.evidence_status)
    };
    plan.authorization.claim_manifest = claims;
    plan.authorization.evidence_status = evidence_status;
}

fn build_appointment(
    plan: &CommitPlan,
    contact: &Contact,
    now: DateTime,
) -> AppResult<Option<Appointment>> {
    let request = validate_appointment_request(plan.draft.decision.appointment_request.as_ref())
        .map_err(|issue| {
            AppError::External(format!(
                "appointment request reached commit with invalid structure: {}",
                issue.code()
            ))
        })?;
    let Some(request) = request else {
        return Ok(None);
    };
    Ok(Some(Appointment {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        idempotency_key: format!("appointment-request:v1:{}", plan.turn_id),
        status: "requested".to_string(),
        request_text: request.request_text,
        requested_start: request.preferred_start,
        requested_end: request.preferred_end,
        confirmed_start: None,
        confirmed_end: None,
        location: request.location_preference,
        confirmation_source_type: None,
        confirmation_source_id: None,
        source_turn_id: plan.turn_id.clone(),
        version: 1,
        created_at: now,
        updated_at: now,
    }))
}

fn build_commitment(plan: &CommitPlan, now: DateTime) -> Option<CommitmentEntry> {
    let text = plan
        .draft
        .decision
        .last_commitment
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let mut commitment = CommitmentEntry::from_plain_text(text.to_string());
    commitment.created_at = now;
    // A commitment becomes authoritative only after every text segment in this decision has been
    // confirmed delivered. The dispatcher promotes this marker atomically during finalization.
    commitment.status = "pending_delivery".to_string();
    commitment.source_id = Some(plan.turn_id.clone());
    if let Some(structured) = plan.draft.decision.commitment.as_ref() {
        if structured.text.trim() == text {
            commitment.due_at = parse_optional_rfc3339(&structured.due_at);
        }
    }
    Some(commitment)
}

fn has_text_outbox(outbox: &[PreparedOutboxEntry]) -> bool {
    outbox.iter().any(|prepared| {
        prepared.entry.media_asset_id.is_none() && prepared.entry.referral_card_id.is_none()
    })
}

/// A review may advertise a pending commitment only when the same transaction is going to write
/// the commitment to the contact. In particular, a duplicate or partial outbox batch has already
/// been removed from `selected_outbox`; retaining the prepared commitment in that review would
/// leave the delivery finalizer chasing a row that this transaction never created.
fn commitment_should_be_persisted(
    commitment: Option<&CommitmentEntry>,
    selected_outbox: &[crate::models::OutboxEntry],
) -> bool {
    commitment.is_some()
        && selected_outbox
            .iter()
            .any(|entry| entry.media_asset_id.is_none() && entry.referral_card_id.is_none())
}

fn parse_optional_rfc3339(value: &str) -> Option<DateTime> {
    let value = value.trim();
    (!value.is_empty())
        .then(|| DateTime::parse_rfc3339_str(value).ok())
        .flatten()
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn segment_idempotency_base<'a>(source_event_id: &'a str, run_id: &'a str) -> &'a str {
    if source_event_id.trim().is_empty() {
        run_id
    } else {
        source_event_id
    }
}

fn terminal_status(plan: &CommitPlan, has_outbox: bool, duplicate_batch: bool) -> String {
    if duplicate_batch {
        "skipped_duplicate".to_string()
    } else if has_outbox {
        "outbox_enqueued".to_string()
    } else if plan.authorization.disposition == "authorized" {
        "no_reply".to_string()
    } else {
        match plan.authorization.final_status.as_str() {
            "quiet_hours_deferred" => "quiet_hours_deferred".to_string(),
            "superseded_by_new_inbound" => "superseded_by_new_inbound".to_string(),
            "stale_task_claim" => "stale_task_claim".to_string(),
            "gateway_blocked" => "gateway_blocked".to_string(),
            "blocked_by_required_field" => "blocked_by_required_field".to_string(),
            "blocked_by_budget" => "blocked_by_budget".to_string(),
            "blocked_unverified_product_claim" => "blocked_unverified_product_claim".to_string(),
            "blocked_by_safety_guard" => "blocked_by_safety_guard".to_string(),
            "ai_waiting_for_more_context" => "ai_waiting_for_more_context".to_string(),
            _ => "held_by_ai_policy".to_string(),
        }
    }
}

fn gateway_result(status: &str, plan: &CommitPlan) -> SendGatewayResult {
    let allowed = matches!(status, "outbox_enqueued" | "no_reply" | "skipped_duplicate");
    SendGatewayResult {
        allowed,
        status: status.to_string(),
        reason: if allowed {
            plan.authorization.reason.clone()
        } else if plan.authorization.reason.trim().is_empty() {
            status.to_string()
        } else {
            plan.authorization.reason.clone()
        },
        policy_blocks: (!allowed)
            .then(|| vec![status.to_string()])
            .unwrap_or_default(),
        run_mode: "live".to_string(),
        message_id: None,
    }
}

fn task_terminal_update(
    status: &str,
    decision_id: ObjectId,
    inputs: &ProductionCommitInputs<'_>,
    now: DateTime,
) -> Document {
    match status {
        "outbox_enqueued" => doc! {
            "$set": {
                "status": "outbox_enqueued",
                "gateway_status": "outbox_enqueued",
                "outbox_decision_id": decision_id,
                "updated_at": now,
            },
            "$unset": { "claimed_at": "" },
        },
        "quiet_hours_deferred" => {
            let wake_at = super::quiet_hours::next_wake_at(
                inputs.runtime.quiet_hours_end,
                inputs.runtime.quiet_hours_tz_offset_hours,
                &inputs.contact.wxid,
                inputs.state.config.wake_jitter_max_seconds,
            );
            doc! {
                "$set": {
                    "status": "pending",
                    "run_at": wake_at,
                    "gateway_status": "quiet_hours_deferred",
                    "cancel_reason": "deferred by final quiet-hours precheck",
                    "updated_at": now,
                },
                "$inc": { "attempt_count": -1i32 },
                "$unset": {
                    "claimed_at": "",
                    "claim_token": "",
                    "outbox_decision_id": "",
                    "next_retry_at": "",
                },
            }
        }
        _ => doc! {
            "$set": {
                "status": "cancelled",
                "gateway_status": status,
                "cancel_reason": status,
                "updated_at": now,
            },
            "$unset": {
                "claimed_at": "",
                "claim_token": "",
                "outbox_decision_id": "",
            },
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn build_decision_review_document(
    inputs: &ProductionCommitInputs<'_>,
    plan: &CommitPlan,
    prepared: &PreparedCommit,
    task_snapshot: Option<&Document>,
    gateway: &SendGatewayResult,
    status: &str,
    expected_text_segments: i32,
    commitment_persisted: bool,
    now: DateTime,
) -> AppResult<Document> {
    let review = &plan.authorization.review;
    let planner = planner_from_decision(&plan.draft.decision, "atomic production commit");
    let mut context_pack_snapshot = inputs.context_pack.clone();
    context_pack_snapshot.insert(
        "knowledgeRoute",
        to_document(inputs.knowledge_route).unwrap_or_default(),
    );
    context_pack_snapshot.insert("runPlanner", to_document(&planner).unwrap_or_default());
    let source_claim = inputs
        .task_context
        .and_then(|context| context.claim.as_ref());
    let action_only_lifecycle_review =
        !plan.draft.decision.should_reply && !plan.draft.decision.commitment_updates.is_empty();
    let review_row = AgentDecisionReview {
        id: Some(prepared.decision_id),
        workspace_id: inputs.contact.workspace_id.clone(),
        account_id: inputs.contact.account_id.clone(),
        contact_wxid: Some(inputs.contact.wxid.clone()),
        run_id: Some(plan.run_id.clone()),
        inbound_message_id: inputs.inbound.message_id.clone(),
        reply_text: non_empty(&plan.draft.decision.reply_text),
        approved: if action_only_lifecycle_review {
            review.approved && !review.should_hold
        } else {
            review_passed(review, inputs.runtime)
        },
        scores: to_document(&review.scores).unwrap_or_default(),
        formula_breakdown: review.formula_breakdown.clone(),
        risks: review.risks.clone(),
        rewrite_instruction: non_empty(&review.rewrite_instruction),
        review_summary: non_empty(&review.review_summary),
        playbook_id: inputs.playbook.and_then(|item| item.id),
        playbook_version: inputs.playbook.map(|item| item.version),
        used_knowledge_ids: plan
            .draft
            .decision
            .used_knowledge_ids
            .iter()
            .filter_map(|id| ObjectId::parse_str(id).ok())
            .collect(),
        prompt_versions: prepared.prompt_versions.clone(),
        operation_state: plan.draft.decision.operation_state.clone(),
        next_best_action: plan.draft.decision.next_best_action.clone(),
        context_pack_snapshot,
        domain_config_snapshot: inputs
            .domain_config
            .and_then(|config| to_document(config).ok())
            .unwrap_or_default(),
        runtime_parameters_snapshot: inputs.runtime.as_document(),
        send_gateway_result: to_document(gateway).unwrap_or_default(),
        outcome_status: Some("pending".to_string()),
        reaction_analysis: Document::new(),
        reaction_claimed_at: None,
        reaction_claim_token: None,
        reaction_claim_generation: 0,
        source_task_id: inputs.task_context.map(|context| context.task_id),
        source_task_claim_token: source_claim.map(|claim| claim.claim_token.clone()),
        reviewer_misjudge_signal: None,
        expected_text_segments,
        status: status.to_string(),
        created_at: now,
    };
    let mut document = to_document(&review_row)?;
    if commitment_persisted {
        let Some(commitment) = prepared.commitment.as_ref() else {
            return Err(AppError::External(
                "review marked commitment persisted without a prepared commitment".to_string(),
            ));
        };
        document.insert(
            "pending_commitment_ids",
            vec![Bson::String(commitment.id.clone())],
        );
    }
    if !plan.draft.decision.commitment_updates.is_empty() {
        document.insert(
            "commitment_lifecycle_updates",
            mongodb::bson::to_bson(&plan.draft.decision.commitment_updates)?,
        );
        document.insert(
            "commitment_lifecycle_source_id",
            prepared.decision_id.to_hex(),
        );
        let timing = if status == "outbox_enqueued" {
            "after_delivery"
        } else if status == "no_reply" && plan.authorization.disposition == "authorized" {
            "in_transaction"
        } else {
            "not_applied"
        };
        document.insert("commitment_lifecycle_timing", timing);
        if commitment_persisted {
            if let Some(commitment) = prepared.commitment.as_ref() {
                document.insert("commitment_lifecycle_replacement_id", commitment.id.clone());
            }
        }
    }
    // Projection is asynchronous and intentionally receives an analytical-only contract.  Freeze
    // the already-authorized operational controls in the same review row so a later model pass
    // cannot invent, remove, or reinterpret a state/cooldown side effect.
    document.insert(
        "authorized_projection_controls",
        authorized_projection_controls(plan, &prepared.post_commit),
    );
    if task_snapshot.and_then(|task| task.get_str("kind").ok())
        == Some(crate::webhooks::DURABLE_INBOUND_REPLY_KIND)
    {
        document.insert("reply_coverage_kind", "passive_reply");
        if let Some(id) =
            task_snapshot.and_then(|task| task.get_object_id("latest_inbound_id").ok())
        {
            document.insert("covers_through_inbound_id", id);
        }
        if let Some(created_at) =
            task_snapshot.and_then(|task| task.get_datetime("latest_inbound_created_at").ok())
        {
            document.insert("covers_through_inbound_created_at", *created_at);
        }
    }
    let principal_media_titles = prepared
        .principal_media
        .iter()
        .map(|asset_id| {
            ObjectId::parse_str(asset_id)
                .ok()
                .and_then(|id| {
                    inputs
                        .sendable_assets
                        .iter()
                        .find(|asset| asset.id == Some(id))
                })
                .map(|asset| asset.title.clone())
                .unwrap_or_else(|| asset_id.clone())
        })
        .collect::<Vec<_>>();
    if let Some(intent) = super::escalation::build_principal_escalation_intent(
        &plan.draft.decision,
        review,
        inputs.domain_config,
        &plan.authorization.disposition,
        status,
        &principal_media_titles,
        now,
    ) {
        document.insert("principal_escalation_intent", intent);
    }
    Ok(document)
}

fn authorized_projection_controls(plan: &CommitPlan, post_commit: &PostCommitWork) -> Document {
    let authorized = plan.authorization.disposition == "authorized";
    authorization_projection_controls(
        authorized,
        &plan.draft.decision,
        &plan.authorization.review,
        post_commit.source_operation_state.as_deref(),
        post_commit.target_operation_state.as_deref(),
        post_commit.operation_state.as_deref(),
        post_commit.policy_version,
        post_commit.domain_version,
    )
}

fn versions_match(expected: Option<i32>, current: Option<i32>) -> bool {
    expected == current
}

fn build_run_terminal_set(
    inputs: &ProductionCommitInputs<'_>,
    plan: &CommitPlan,
    gateway: &SendGatewayResult,
    status: &str,
    has_outbox: bool,
    now: DateTime,
) -> AppResult<Document> {
    assert_gateway_status_valid(status)?;
    assert_final_review_status_valid(&plan.authorization.review.final_review_status)?;
    let lifecycle = derive_lifecycle_from_status(status, None).to_string();
    assert_lifecycle_valid(&lifecycle)?;
    let planner = planner_from_decision(&plan.draft.decision, "atomic production commit");
    let budget = current_run_budget().map(|budget| budget.snapshot());
    let fields = AgentRunLogTerminalFields {
        workspace_id: Some(inputs.contact.workspace_id.clone()),
        account_id: Some(inputs.contact.account_id.clone()),
        contact_wxid: Some(inputs.contact.wxid.clone()),
        trigger_kind: Some(inputs.trigger.kind().to_string()),
        source_event_id: Some(inputs.source_event_id.to_string()),
        source_kind: Some(inputs.source_kind.to_string()),
        lifecycle: Some(lifecycle.clone()),
        status: Some(status.to_string()),
        planner: Some(to_document(&planner).unwrap_or_default()),
        context: Some(doc! {
            "refreshed": inputs.context_refreshed,
            "version": inputs.context_pack.get_i32("version").unwrap_or_default(),
        }),
        knowledge_route: Some(to_document(inputs.knowledge_route).unwrap_or_default()),
        decision: Some(to_document(&plan.draft.decision).unwrap_or_default()),
        review: Some(to_document(&plan.authorization.review).unwrap_or_default()),
        gateway_result: Some(to_document(gateway).unwrap_or_default()),
        error: None,
        error_summary: None,
        abort_reason: (lifecycle == super::run_envelope::LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL)
            .then(|| status.to_string()),
        token_budget: budget.as_ref().map(|budget| budget.token_budget),
        tokens_used: budget.as_ref().map(|budget| budget.tokens_used),
        llm_calls_used: budget.as_ref().map(|budget| budget.llm_calls_used),
        unknown_usage_calls: budget.as_ref().map(|budget| budget.unknown_usage_calls),
        degraded_reasons: budget.map(|budget| budget.degraded_reasons),
        revision_applied: Some(plan.authorization.review.revision_applied),
        revision_reason: Some(
            plan.authorization
                .review
                .revision_applied
                .then(|| "bounded_harness_repair_applied".to_string())
                .unwrap_or_default(),
        ),
        pre_revision_summary: None,
        post_revision_summary: None,
        self_critique: non_empty(&plan.draft.decision.self_critique),
        autonomy_mode: Some(plan.draft.decision.autonomy_mode.clone()),
        conversation_mode: Some(plan.draft.decision.conversation_mode.clone()),
        conversation_mode_reason: plan.draft.decision.conversation_mode_reason.clone(),
        final_review_status: Some(plan.authorization.review.final_review_status.clone()),
        outbox_status: has_outbox.then(|| "pending".to_string()),
        memory_consolidator_warnings: None,
    };
    let mut set = fields.to_set_document();
    set.insert("updated_at", now);
    Ok(set)
}

async fn commit_transaction(session: &mut ClientSession) -> mongodb::error::Result<()> {
    loop {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => return Err(error),
        }
    }
}

fn retryable_db(error: &mongodb::error::Error) -> bool {
    error.contains_label("TransientTransactionError") || is_duplicate_key_error(error)
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    let duplicate = |code| matches!(code, 11000 | 11001);
    match &*error.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => duplicate(write_error.code),
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .as_ref()
            .is_some_and(|errors| errors.iter().any(|error| duplicate(error.code))),
        ErrorKind::Command(command) => duplicate(command.code),
        _ => false,
    }
}

fn retry_delay(attempt: usize) -> std::time::Duration {
    std::time::Duration::from_millis(5_u64 << attempt.min(6))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::AppointmentRequestDecision;

    fn prepared_outbox_for_test(
        source_event_id: &str,
        media_asset_id: Option<&str>,
        referral_card_id: Option<&str>,
    ) -> PreparedOutboxEntry {
        prepare_outbox_entry(
            &EnqueueRequest {
                workspace_id: "ws".to_string(),
                account_id: "account".to_string(),
                contact_wxid: "wx".to_string(),
                run_id: "run".to_string(),
                decision_id: Some(ObjectId::new()),
                source_event_id: source_event_id.to_string(),
                source_kind: "inbound".to_string(),
                content: if media_asset_id.is_some() || referral_card_id.is_some() {
                    String::new()
                } else {
                    "text".to_string()
                },
                media_asset_id: media_asset_id.map(ToString::to_string),
                referral_card_id: referral_card_id.map(ToString::to_string),
                max_attempts: 3,
            },
            DateTime::from_millis(1),
            ObjectId::new(),
        )
        .expect("valid prepared outbox entry")
    }

    #[test]
    fn commitments_require_a_text_outbox_entry() {
        let media = prepared_outbox_for_test("media", Some("asset-1"), None);
        let namecard = prepared_outbox_for_test("namecard", None, Some("card-1"));
        let text = prepared_outbox_for_test("text", None, None);

        assert!(!has_text_outbox(&[media, namecard]));
        assert!(has_text_outbox(&[text]));
    }

    #[test]
    fn review_only_references_a_commitment_written_by_this_batch() {
        let commitment = CommitmentEntry::from_plain_text("投递后回访".to_string());
        let media = prepared_outbox_for_test("media", Some("asset-1"), None).entry;
        let text = prepared_outbox_for_test("text", None, None).entry;

        assert!(!commitment_should_be_persisted(Some(&commitment), &[]));
        assert!(!commitment_should_be_persisted(Some(&commitment), &[media]));
        assert!(commitment_should_be_persisted(Some(&commitment), &[text]));
        assert!(!commitment_should_be_persisted(None, &[]));
    }

    #[test]
    fn authorized_projection_controls_freeze_structured_intent_not_projection_output() {
        let mut decision = super::super::types::AgentDecision {
            should_reply: false,
            operation_state: Some("need_discovery".to_string()),
            operation_state_reason: Some("客户明确提出下一步问题".to_string()),
            operation_state_confidence: Some(8),
            cooldown_until: Some("2026-08-20T10:00:00+08:00".to_string()),
            appointment_request: Some(AppointmentRequestDecision {
                requested: true,
                ..AppointmentRequestDecision::default()
            }),
            ..super::super::types::AgentDecision::default()
        };
        decision
            .domain_signals
            .insert("customer_stage", "relationship_building");
        let plan = CommitPlan {
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            authority_bundle_hash: "authority".to_string(),
            draft: super::super::turn_loop::DraftEnvelope {
                iteration: 0,
                repair_attempt: 0,
                draft_hash: "draft".to_string(),
                decision,
                promote_risks: Vec::new(),
            },
            authorization: AuthorizationManifest::authorized(
                "approved",
                super::super::types::DecisionReviewResult::default(),
            ),
        };
        let post_commit = PostCommitWork {
            operation_state: Some("need_discovery".to_string()),
            target_operation_state: Some("need_discovery".to_string()),
            policy_version: Some(3),
            domain_version: Some(7),
            source_operation_state: Some("new_contact".to_string()),
            ..PostCommitWork::default()
        };
        let controls = authorized_projection_controls(&plan, &post_commit);

        assert_eq!(controls.get_i32("authorization_fence_version"), Ok(1));
        assert_eq!(controls.get_bool("authorized"), Ok(true));
        assert_eq!(controls.get_str("operation_state"), Ok("need_discovery"));
        assert_eq!(
            controls.get_str("operation_state_source"),
            Ok("operation_state")
        );
        assert_eq!(controls.get_i32("policy_version"), Ok(3));
        assert_eq!(controls.get_i32("domain_version"), Ok(7));
        assert_eq!(
            controls.get_str("source_operation_state"),
            Ok("new_contact")
        );
        assert!(controls.get_datetime("cooldown_until").is_ok());
        assert!(controls
            .get_array("actions")
            .expect("frozen actions")
            .iter()
            .any(|value| value.as_str() == Some("appointment_request")));

        let mut held = plan;
        held.authorization = AuthorizationManifest::held(
            "held_by_ai_policy",
            "policy changed",
            super::super::types::DecisionReviewResult::default(),
        );
        let held_controls = authorized_projection_controls(&held, &post_commit);
        assert_eq!(held_controls.get_bool("authorized"), Ok(false));
        assert!(held_controls.get_datetime("cooldown_until").is_err());
        assert!(held_controls
            .get_array("actions")
            .expect("held actions")
            .is_empty());
    }

    #[test]
    fn invalid_operation_state_candidate_never_becomes_projection_target() {
        let plan = CommitPlan {
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            authority_bundle_hash: "authority".to_string(),
            draft: super::super::turn_loop::DraftEnvelope {
                iteration: 0,
                repair_attempt: 0,
                draft_hash: "draft".to_string(),
                decision: super::super::types::AgentDecision {
                    operation_state: Some("invented_state".to_string()),
                    ..super::super::types::AgentDecision::default()
                },
                promote_risks: Vec::new(),
            },
            authorization: AuthorizationManifest::authorized(
                "approved",
                super::super::types::DecisionReviewResult::default(),
            ),
        };
        let post_commit = PostCommitWork {
            operation_state: Some("new_contact".to_string()),
            source_operation_state: Some("new_contact".to_string()),
            policy_version: Some(1),
            domain_version: Some(1),
            ..PostCommitWork::default()
        };

        let controls = authorized_projection_controls(&plan, &post_commit);
        assert_eq!(controls.get("operation_state"), Some(&Bson::Null));
        assert_eq!(controls.get("operation_state_source"), Some(&Bson::Null));
        assert_eq!(controls.get_str("policy_state"), Ok("new_contact"));
    }

    #[test]
    fn authorized_no_reply_can_record_a_request_without_implying_confirmation() {
        let mut plan = CommitPlan {
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            authority_bundle_hash: "authority".to_string(),
            draft: super::super::turn_loop::DraftEnvelope {
                iteration: 0,
                repair_attempt: 0,
                draft_hash: "draft".to_string(),
                decision: super::super::types::AgentDecision {
                    appointment_request: Some(AppointmentRequestDecision {
                        requested: true,
                        request_text: "客户希望到院面诊".to_string(),
                        preferred_start: "2026-08-20T10:00:00+08:00".to_string(),
                        ..AppointmentRequestDecision::default()
                    }),
                    ..super::super::types::AgentDecision::default()
                },
                promote_risks: Vec::new(),
            },
            authorization: AuthorizationManifest::authorized(
                "approved",
                super::super::types::DecisionReviewResult::default(),
            ),
        };
        normalize_review_terminal(&mut plan);
        let contact = Contact {
            id: None,
            workspace_id: "ws".to_string(),
            account_id: "account".to_string(),
            wxid: "wx".to_string(),
            nickname: None,
            remark: None,
            alias: None,
            avatar_url: None,
            sex: None,
            agent_status: crate::models::AgentStatus::Managed,
            human_profile_note: None,
            custom_agent_instructions: None,
            operation_mode_override: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            manual_tags: Vec::new(),
            manual_tags_updated_at: None,
            manual_tags_by: None,
            confirmed_tags: Vec::new(),
            bayesian_signals: Vec::new(),
            personality_profile: None,
            tags_version: 0,
            domain_attributes: None,
            domain_attributes_updated_at: None,
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: Document::new(),
            profile_attributes: Document::new(),
            profile_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            last_outbound_style: None,
            intent_trajectory: Vec::new(),
            outcome_events: Vec::new(),
            locale: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        let appointment = build_appointment(&plan, &contact, DateTime::now())
            .unwrap()
            .expect("appointment request");

        assert!(!plan.draft.decision.should_reply);
        assert_eq!(appointment.status, "requested");
        assert!(appointment.confirmed_start.is_none());
        assert!(appointment.confirmed_end.is_none());
        assert!(appointment.confirmation_source_type.is_none());
        assert!(appointment.confirmation_source_id.is_none());
    }

    #[test]
    fn terminal_status_never_treats_a_held_plan_as_sendable() {
        let mut plan = CommitPlan {
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            authority_bundle_hash: "authority".to_string(),
            draft: super::super::turn_loop::DraftEnvelope {
                iteration: 0,
                repair_attempt: 0,
                draft_hash: "draft".to_string(),
                decision: super::super::types::AgentDecision::default(),
                promote_risks: Vec::new(),
            },
            authorization: AuthorizationManifest::authorized(
                "approved",
                super::super::types::DecisionReviewResult::default(),
            ),
        };
        hold_plan(
            &mut plan,
            "blocked_by_safety_guard",
            Some("blocked_by_safety_guard"),
            "unsupported",
            "unsupported_claim",
        );

        assert_eq!(
            terminal_status(&plan, false, false),
            "blocked_by_safety_guard"
        );
        assert_eq!(plan.authorization.disposition, "held");
        assert!(!plan.draft.decision.should_reply);
    }

    #[test]
    fn commitments_wait_for_delivery_before_becoming_authoritative() {
        let plan = CommitPlan {
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            authority_bundle_hash: "authority".to_string(),
            draft: super::super::turn_loop::DraftEnvelope {
                iteration: 0,
                repair_attempt: 0,
                draft_hash: "draft".to_string(),
                decision: super::super::types::AgentDecision {
                    last_commitment: Some("在投递后回访".to_string()),
                    ..Default::default()
                },
                promote_risks: Vec::new(),
            },
            authorization: AuthorizationManifest::authorized(
                "approved",
                super::super::types::DecisionReviewResult::default(),
            ),
        };
        let commitment = build_commitment(&plan, DateTime::from_millis(1)).expect("commitment");
        assert_eq!(commitment.status, "pending_delivery");
    }
}

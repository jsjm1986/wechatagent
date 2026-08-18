//! Shared Agent Harness turn kernel.
//!
//! Semantic choices stay model-owned. This module only sequences capabilities, bounds work,
//! detects lack of progress, requires per-candidate authorization, and presents one commit plan
//! to the selected environment.

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use mongodb::bson::{doc, to_document, Bson, DateTime, Document};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

use super::guards::reviewed_decision_actions;
use super::types::{
    is_reply_protocol_violation, AgentDecision, DecisionReviewResult, ToolCallRequest,
};

pub(crate) const TURN_LOOP_MAX_ITERATIONS: usize = 4;
pub(crate) const TURN_LOOP_MAX_REPAIRS: usize = 2;
pub(crate) const TURN_LOOP_PHASE_TIMEOUT: Duration = Duration::from_secs(75);
pub(crate) const TURN_LOOP_AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(150);
pub(crate) const TURN_LOOP_TOTAL_TIMEOUT: Duration = Duration::from_secs(240);
const TURN_LOOP_CONTEXT_MAX_CHARS: usize = 12_000;

/// Version for the structured authorization fence stored beside every current production
/// decision.  The fence is deliberately independent of the model/review schema: a dispatcher
/// can reject an unknown fence version without trying to interpret free-form output.
pub(crate) const AUTHORIZATION_FENCE_VERSION: i32 = 1;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnGenerateRequest {
    pub iteration: usize,
    pub repair_attempt: usize,
    pub tool_context: String,
    pub authorization_feedback: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DraftEnvelope {
    pub iteration: usize,
    pub repair_attempt: usize,
    pub draft_hash: String,
    pub decision: AgentDecision,
    #[serde(default)]
    pub promote_risks: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolDispatchBatch {
    #[serde(default)]
    pub context_fragment: String,
    #[serde(default)]
    pub trace: Vec<Document>,
    #[serde(default)]
    pub dispatched: usize,
    /// Stable identity of all independently verified evidence available after this batch.
    /// Environments should prefer the authority ledger hash over a model-produced value.
    #[serde(default)]
    pub evidence_fingerprint: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthorizationManifest {
    /// `authorized`, `repairable`, or `held`.
    pub disposition: String,
    pub final_status: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub repair_instruction: String,
    #[serde(default)]
    pub claim_manifest: Vec<Document>,
    #[serde(default)]
    pub evidence_status: String,
    #[serde(default)]
    pub review: DecisionReviewResult,
}

impl AuthorizationManifest {
    pub(crate) fn authorized(
        final_status: impl Into<String>,
        review: DecisionReviewResult,
    ) -> Self {
        let (claim_manifest, evidence_status) = claim_authorization_projection(&review);
        Self {
            disposition: "authorized".to_string(),
            final_status: final_status.into(),
            reason: review.review_summary.clone(),
            repair_instruction: String::new(),
            claim_manifest,
            evidence_status,
            review,
        }
    }

    pub(crate) fn repairable(
        final_status: impl Into<String>,
        instruction: impl Into<String>,
        review: DecisionReviewResult,
    ) -> Self {
        let (claim_manifest, evidence_status) = claim_authorization_projection(&review);
        Self {
            disposition: "repairable".to_string(),
            final_status: final_status.into(),
            reason: review.review_summary.clone(),
            repair_instruction: instruction.into(),
            claim_manifest,
            evidence_status,
            review,
        }
    }

    pub(crate) fn held(
        final_status: impl Into<String>,
        reason: impl Into<String>,
        review: DecisionReviewResult,
    ) -> Self {
        let (claim_manifest, evidence_status) = claim_authorization_projection(&review);
        Self {
            disposition: "held".to_string(),
            final_status: final_status.into(),
            reason: reason.into(),
            repair_instruction: String::new(),
            claim_manifest,
            evidence_status,
            review,
        }
    }

    pub(crate) fn to_document(&self) -> Document {
        to_document(self).unwrap_or_default()
    }
}

/// Build the deterministic operational controls frozen with an authorized decision.
///
/// This is shared by the transactional production committer and the smaller administrative /
/// deterministic reply paths.  It contains no reply-text inspection and no semantic classifier;
/// callers provide the already-resolved source state, policy state/version, and domain version.
/// The dispatcher and delivery finalizer consume this same shape, so a path cannot accidentally
/// fall back to an unversioned authorization contract merely because it did not use the LLM turn
/// kernel.
pub(crate) fn authorization_projection_controls(
    authorized: bool,
    decision: &AgentDecision,
    review: &DecisionReviewResult,
    source_operation_state: Option<&str>,
    target_operation_state: Option<&str>,
    policy_state: Option<&str>,
    policy_version: Option<i32>,
    domain_version: Option<i32>,
) -> Document {
    // The target is resolved by the deterministic state-machine/policy layer before this
    // function is called. Never re-read the model's raw proposal here: an unknown or illegal
    // proposal must not become a durable projection instruction merely because it was present in
    // the JSON candidate.
    let operation_state = authorized.then_some(target_operation_state).flatten();
    let actions = if authorized {
        reviewed_decision_actions(decision, review)
            .into_iter()
            .map(|action| Bson::String(action.to_string()))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut controls = doc! {
        "authorization_fence_version": AUTHORIZATION_FENCE_VERSION,
        "authorized": authorized,
        "actions": Bson::Array(actions),
        "operation_state": operation_state
            .map(|value| Bson::String(value.to_string()))
            .unwrap_or(Bson::Null),
        "operation_state_source": operation_state
            .map(|_| Bson::String("operation_state".to_string()))
            .unwrap_or(Bson::Null),
        "operation_state_reason": if authorized {
            decision
                .operation_state_reason
                .clone()
                .map(Bson::String)
                .unwrap_or(Bson::Null)
        } else {
            Bson::Null
        },
        "operation_state_confidence": if authorized {
            decision
                .operation_state_confidence
                .map(Bson::Int32)
                .unwrap_or(Bson::Null)
        } else {
            Bson::Null
        },
        "policy_state": if authorized {
            policy_state
                .map(|value| Bson::String(value.to_string()))
                .unwrap_or(Bson::Null)
        } else {
            Bson::Null
        },
        "source_operation_state": if authorized {
            source_operation_state
                .map(|value| Bson::String(value.to_string()))
                .unwrap_or(Bson::Null)
        } else {
            Bson::Null
        },
        "policy_version": if authorized {
            policy_version.map(Bson::Int32).unwrap_or(Bson::Null)
        } else {
            Bson::Null
        },
        "domain_version": if authorized {
            domain_version.map(Bson::Int32).unwrap_or(Bson::Null)
        } else {
            Bson::Null
        },
    };
    if authorized {
        if let Some(value) = decision
            .cooldown_until
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| DateTime::parse_rfc3339_str(value).ok())
        {
            controls.insert("cooldown_until", value);
        }
    }
    controls
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitPlan {
    pub run_id: String,
    pub turn_id: String,
    pub authority_bundle_hash: String,
    pub draft: DraftEnvelope,
    pub authorization: AuthorizationManifest,
}

/// A committer may tighten the final deterministic authorization boundary before persisting it.
/// Returning the committed plan keeps the in-memory outcome identical to the durable decision;
/// callers never continue with the pre-commit candidate after task fencing or lifecycle checks
/// have downgraded it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitResult {
    pub plan: CommitPlan,
    pub receipt: CommitReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitReceipt {
    /// `committed`, `simulated`, `held`, or `no_op`.
    pub status: String,
    pub environment: String,
    pub committed_at: DateTime,
    #[serde(default)]
    pub outbox_ids: Vec<String>,
    #[serde(default)]
    pub appointment_id: Option<String>,
    #[serde(default)]
    pub commitment_ids: Vec<String>,
    #[serde(default)]
    pub mutation_count: usize,
    #[serde(default)]
    pub details: Document,
}

impl Default for CommitReceipt {
    fn default() -> Self {
        Self {
            status: "no_op".to_string(),
            environment: String::new(),
            committed_at: DateTime::from_millis(0),
            outbox_ids: Vec::new(),
            appointment_id: None,
            commitment_ids: Vec::new(),
            mutation_count: 0,
            details: Document::new(),
        }
    }
}

impl CommitReceipt {
    pub(crate) fn to_document(&self) -> Document {
        to_document(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TurnOutcome {
    pub draft: DraftEnvelope,
    pub authorization: AuthorizationManifest,
    pub commit_receipt: CommitReceipt,
}

#[async_trait]
pub(crate) trait TurnEnvironment: Send {
    async fn generate(
        &mut self,
        request: TurnGenerateRequest,
    ) -> AppResult<(AgentDecision, Vec<String>)>;

    async fn dispatch_tools(
        &mut self,
        calls: &[ToolCallRequest],
        deadline: tokio::time::Instant,
    ) -> AppResult<ToolDispatchBatch>;

    async fn authorize(&mut self, draft: &mut DraftEnvelope) -> AppResult<AuthorizationManifest>;

    async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult>;

    /// Persist the bounded trace after commit. This is deliberately post-commit and fail-soft in
    /// the kernel: an observability failure must never make a durable send look uncommitted and
    /// trigger a duplicate retry.
    async fn persist_runtime_state(
        &mut self,
        _loop_trace: &[Document],
        _authorization: &AuthorizationManifest,
        _commit_receipt: &CommitReceipt,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) async fn run_turn<E: TurnEnvironment>(
    input: &TurnKernelInput<'_>,
    environment: &mut E,
) -> AppResult<TurnOutcome> {
    run_turn_with_config(input, environment, TurnLoopConfig::default()).await
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TurnLoopTimeouts {
    phase_timeout: Duration,
    repair_timeout: Duration,
    authorization_timeout: Duration,
    total_timeout: Duration,
}

impl TurnLoopTimeouts {
    pub(crate) fn from_seconds(
        phase_timeout_seconds: u64,
        repair_timeout_seconds: u64,
        authorization_timeout_seconds: u64,
        total_timeout_seconds: u64,
    ) -> Self {
        let phase_timeout_seconds = phase_timeout_seconds.max(1);
        let repair_timeout_seconds = repair_timeout_seconds.max(1);
        let authorization_timeout_seconds = authorization_timeout_seconds.max(1);
        Self {
            phase_timeout: Duration::from_secs(phase_timeout_seconds),
            repair_timeout: Duration::from_secs(repair_timeout_seconds),
            authorization_timeout: Duration::from_secs(authorization_timeout_seconds),
            total_timeout: Duration::from_secs(
                total_timeout_seconds.max(
                    phase_timeout_seconds
                        .max(repair_timeout_seconds)
                        .max(authorization_timeout_seconds),
                ),
            ),
        }
    }

    pub(crate) fn total_deadline_from_now(&self) -> tokio::time::Instant {
        tokio::time::Instant::now() + self.total_timeout
    }

    pub(crate) async fn run_initial_phase<T>(
        &self,
        phase: &'static str,
        total_deadline: tokio::time::Instant,
        future: impl std::future::Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        timeout_at(
            phase,
            phase_deadline(total_deadline, self.phase_timeout),
            future,
        )
        .await
    }
}

impl Default for TurnLoopTimeouts {
    fn default() -> Self {
        Self {
            phase_timeout: TURN_LOOP_PHASE_TIMEOUT,
            repair_timeout: TURN_LOOP_AUTHORIZATION_TIMEOUT,
            authorization_timeout: TURN_LOOP_AUTHORIZATION_TIMEOUT,
            total_timeout: TURN_LOOP_TOTAL_TIMEOUT,
        }
    }
}

pub(crate) async fn run_turn_with_timeouts<E: TurnEnvironment>(
    input: &TurnKernelInput<'_>,
    environment: &mut E,
    timeouts: TurnLoopTimeouts,
) -> AppResult<TurnOutcome> {
    let total_deadline = timeouts.total_deadline_from_now();
    run_turn_with_deadline(input, environment, timeouts, total_deadline).await
}

pub(crate) async fn run_turn_with_deadline<E: TurnEnvironment>(
    input: &TurnKernelInput<'_>,
    environment: &mut E,
    timeouts: TurnLoopTimeouts,
    total_deadline: tokio::time::Instant,
) -> AppResult<TurnOutcome> {
    run_turn_with_config_at(
        input,
        environment,
        TurnLoopConfig {
            max_iterations: TURN_LOOP_MAX_ITERATIONS,
            max_repairs: TURN_LOOP_MAX_REPAIRS,
            phase_timeout: timeouts.phase_timeout,
            repair_timeout: timeouts.repair_timeout,
            authorization_timeout: timeouts.authorization_timeout,
            total_timeout: timeouts.total_timeout,
        },
        Some(total_deadline),
    )
    .await
}

#[derive(Debug, Clone, Copy)]
struct TurnLoopConfig {
    max_iterations: usize,
    max_repairs: usize,
    phase_timeout: Duration,
    repair_timeout: Duration,
    authorization_timeout: Duration,
    total_timeout: Duration,
}

impl Default for TurnLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: TURN_LOOP_MAX_ITERATIONS,
            max_repairs: TURN_LOOP_MAX_REPAIRS,
            phase_timeout: TURN_LOOP_PHASE_TIMEOUT,
            repair_timeout: TURN_LOOP_AUTHORIZATION_TIMEOUT,
            authorization_timeout: TURN_LOOP_AUTHORIZATION_TIMEOUT,
            total_timeout: TURN_LOOP_TOTAL_TIMEOUT,
        }
    }
}

pub(crate) struct TurnKernelInput<'a> {
    pub run_id: &'a str,
    pub turn_id: &'a str,
    pub authority_bundle_hash: &'a str,
}

#[cfg(test)]
async fn run_turn_with_config<E: TurnEnvironment>(
    input: &TurnKernelInput<'_>,
    environment: &mut E,
    config: TurnLoopConfig,
) -> AppResult<TurnOutcome> {
    run_turn_with_config_at(input, environment, config, None).await
}

async fn run_turn_with_config_at<E: TurnEnvironment>(
    input: &TurnKernelInput<'_>,
    environment: &mut E,
    config: TurnLoopConfig,
    inherited_total_deadline: Option<tokio::time::Instant>,
) -> AppResult<TurnOutcome> {
    let configured_deadline = tokio::time::Instant::now() + config.total_timeout;
    let total_deadline = inherited_total_deadline
        .map(|deadline| deadline.min(configured_deadline))
        .unwrap_or(configured_deadline);
    let mut tool_context = String::new();
    let mut authorization_feedback = String::new();
    let mut repairs = 0usize;
    let mut trace = Vec::new();
    let mut seen_progress = HashSet::new();
    let mut seen_context_fragments = HashSet::new();
    let mut evidence_fingerprint = String::new();
    let mut last_draft: Option<DraftEnvelope> = None;

    for iteration in 0..config.max_iterations {
        let generation_timeout = if repairs > 0 {
            config.repair_timeout
        } else {
            config.phase_timeout
        };
        let generate_deadline = phase_deadline(total_deadline, generation_timeout);
        let generated = timeout_at(
            if repairs > 0 {
                "repair_generate"
            } else {
                "generate"
            },
            generate_deadline,
            environment.generate(TurnGenerateRequest {
                iteration,
                repair_attempt: repairs,
                tool_context: tool_context.clone(),
                authorization_feedback: authorization_feedback.clone(),
            }),
        )
        .await?;
        let (decision, promote_risks) = generated;
        let draft_hash = stable_draft_hash(&decision);
        let mut draft = DraftEnvelope {
            iteration,
            repair_attempt: repairs,
            draft_hash: draft_hash.clone(),
            decision,
            promote_risks,
        };
        trace.push(doc! {
            "iteration": iteration as i64,
            "phase": &draft.decision.decision_phase,
            "next_step": &draft.decision.next_step,
            "draft_hash": &draft_hash,
            "tool_call_count": draft.decision.tool_calls.len() as i64,
            "repair_attempt": repairs as i64,
        });

        if draft.decision.decision_phase == "tool_calling" {
            if draft.decision.tool_calls.is_empty() {
                let authorization = AuthorizationManifest::held(
                    "held_invalid_tool_plan",
                    "tool-calling phase contained no executable read-only tool request",
                    DecisionReviewResult::default(),
                );
                return commit_outcome(
                    input,
                    environment,
                    draft,
                    authorization,
                    trace,
                    iteration + 1,
                    repairs,
                    total_deadline,
                )
                .await;
            }
            let dispatch_deadline = phase_deadline(total_deadline, config.phase_timeout);
            let batch = timeout_at(
                "tool_dispatch",
                dispatch_deadline,
                environment.dispatch_tools(&draft.decision.tool_calls, dispatch_deadline.at),
            )
            .await?;
            append_bounded_context(
                &mut tool_context,
                &batch.context_fragment,
                &mut seen_context_fragments,
            );
            if !batch.evidence_fingerprint.trim().is_empty() {
                evidence_fingerprint = batch.evidence_fingerprint.clone();
            } else {
                evidence_fingerprint = stable_text_hash(&tool_context);
            }
            trace.extend(batch.trace);
            trace.push(doc! {
                "iteration": iteration as i64,
                "phase": "tool_dispatch",
                "dispatched": batch.dispatched as i64,
                "evidence_fingerprint": &evidence_fingerprint,
            });
            let progress_identity = stable_progress_hash(&draft_hash, &evidence_fingerprint);
            if !seen_progress.insert(progress_identity) {
                let authorization = AuthorizationManifest::held(
                    "held_no_progress",
                    "turn loop repeated the same structured tool plan without new verified evidence",
                    DecisionReviewResult::default(),
                );
                return commit_outcome(
                    input,
                    environment,
                    draft,
                    authorization,
                    trace,
                    iteration + 1,
                    repairs,
                    total_deadline,
                )
                .await;
            }
            last_draft = Some(draft);
            authorization_feedback.clear();
            continue;
        }

        let progress_identity = stable_progress_hash(&draft_hash, &evidence_fingerprint);
        let has_protocol_violation = draft
            .promote_risks
            .iter()
            .any(|risk| is_reply_protocol_violation(risk));
        if !has_protocol_violation && !seen_progress.insert(progress_identity) {
            let authorization = AuthorizationManifest::held(
                "held_no_progress",
                "turn loop produced the same structured draft without new verified evidence",
                DecisionReviewResult::default(),
            );
            return commit_outcome(
                input,
                environment,
                draft,
                authorization,
                trace,
                iteration + 1,
                repairs,
                total_deadline,
            )
            .await;
        }
        if has_protocol_violation {
            trace.push(doc! {
                "iteration": iteration as i64,
                "phase": "protocol_authorization",
                "violation_count": draft.promote_risks.iter()
                    .filter(|risk| is_reply_protocol_violation(risk))
                    .count() as i64,
            });
        }

        let authorization_deadline = phase_deadline(total_deadline, config.authorization_timeout);
        let authorization = timeout_at(
            "authorization",
            authorization_deadline,
            environment.authorize(&mut draft),
        )
        .await?;
        let finalized_draft_hash = stable_draft_hash(&draft.decision);
        if finalized_draft_hash != draft.draft_hash {
            trace.push(doc! {
                "iteration": iteration as i64,
                "phase": "authorization_finalization",
                "candidate_draft_hash": &draft.draft_hash,
                "finalized_draft_hash": &finalized_draft_hash,
            });
            draft.draft_hash = finalized_draft_hash;
        }
        trace.push(doc! {
            "iteration": iteration as i64,
            "phase": "authorization",
            "disposition": &authorization.disposition,
            "final_status": &authorization.final_status,
            "claim_count": authorization.claim_manifest.len() as i64,
            "evidence_status": &authorization.evidence_status,
        });
        match authorization.disposition.as_str() {
            "authorized" | "held" => {
                return commit_outcome(
                    input,
                    environment,
                    draft,
                    authorization,
                    trace,
                    iteration + 1,
                    repairs,
                    total_deadline,
                )
                .await;
            }
            "repairable" if repairs < config.max_repairs => {
                if authorization.repair_instruction.trim().is_empty() {
                    let held = AuthorizationManifest::held(
                        "held_invalid_repair",
                        "authorization requested repair without a structured direction",
                        authorization.review,
                    );
                    return commit_outcome(
                        input,
                        environment,
                        draft,
                        held,
                        trace,
                        iteration + 1,
                        repairs,
                        total_deadline,
                    )
                    .await;
                }
                repairs += 1;
                authorization_feedback = authorization.repair_instruction;
                last_draft = Some(draft);
            }
            "repairable" => {
                let held = AuthorizationManifest::held(
                    "held_repair_exhausted",
                    "bounded authorization repair attempts were exhausted",
                    authorization.review,
                );
                return commit_outcome(
                    input,
                    environment,
                    draft,
                    held,
                    trace,
                    iteration + 1,
                    repairs,
                    total_deadline,
                )
                .await;
            }
            _ => {
                let held = AuthorizationManifest::held(
                    "held_invalid_authorization",
                    "authorization environment returned an invalid disposition",
                    authorization.review,
                );
                return commit_outcome(
                    input,
                    environment,
                    draft,
                    held,
                    trace,
                    iteration + 1,
                    repairs,
                    total_deadline,
                )
                .await;
            }
        }
    }

    let draft = last_draft.ok_or_else(|| {
        AppError::External("turn loop completed without producing a draft".to_string())
    })?;
    let authorization = AuthorizationManifest::held(
        "held_iteration_exhausted",
        "bounded turn iterations were exhausted before authorization",
        DecisionReviewResult::default(),
    );
    commit_outcome(
        input,
        environment,
        draft,
        authorization,
        trace,
        config.max_iterations,
        repairs,
        total_deadline,
    )
    .await
}

async fn commit_outcome<E: TurnEnvironment>(
    input: &TurnKernelInput<'_>,
    environment: &mut E,
    draft: DraftEnvelope,
    mut authorization: AuthorizationManifest,
    loop_trace: Vec<Document>,
    _iterations: usize,
    _repairs: usize,
    _deadline: tokio::time::Instant,
) -> AppResult<TurnOutcome> {
    if authorization.disposition == "authorized" {
        let invalid_reason = if draft.decision.decision_phase == "tool_calling"
            || !draft.decision.tool_calls.is_empty()
        {
            Some("authorization cannot commit an unresolved tool-calling draft")
        } else if draft.decision.should_reply && draft.decision.reply_text.trim().is_empty() {
            Some("authorization cannot send an empty reply body")
        } else {
            None
        };
        if let Some(reason) = invalid_reason {
            authorization = AuthorizationManifest::held(
                "held_invalid_authorized_draft",
                reason,
                authorization.review,
            );
        }
    }
    let plan = CommitPlan {
        run_id: input.run_id.to_string(),
        turn_id: input.turn_id.to_string(),
        authority_bundle_hash: input.authority_bundle_hash.to_string(),
        draft: draft.clone(),
        authorization: authorization.clone(),
    };
    // Once durable commit starts it must run to a definite result. Cancelling here at the model
    // loop deadline can leave Mongo committed while the caller observes a timeout and retries the
    // turn, which is exactly the duplicate-side-effect ambiguity the commit protocol prevents.
    let committed = environment.commit(plan).await?;
    let committed_draft = committed.plan.draft;
    let committed_authorization = committed.plan.authorization;
    let commit_receipt = committed.receipt;
    if let Err(error) = environment
        .persist_runtime_state(&loop_trace, &committed_authorization, &commit_receipt)
        .await
    {
        tracing::warn!(%error, "turn runtime snapshot persistence failed after commit");
    }
    Ok(TurnOutcome {
        draft: committed_draft,
        authorization: committed_authorization,
        commit_receipt,
    })
}

async fn timeout_at<T>(
    phase: &'static str,
    deadline: TurnDeadline,
    future: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    tokio::time::timeout_at(deadline.at, future)
        .await
        .map_err(|_| {
            AppError::External(format!("turn_loop_timeout:{phase}:{}", deadline.limit_kind))
        })?
}

#[derive(Debug, Clone, Copy)]
struct TurnDeadline {
    at: tokio::time::Instant,
    limit_kind: &'static str,
}

fn phase_deadline(total_deadline: tokio::time::Instant, phase_timeout: Duration) -> TurnDeadline {
    let phase_deadline = tokio::time::Instant::now() + phase_timeout;
    if phase_deadline < total_deadline {
        TurnDeadline {
            at: phase_deadline,
            limit_kind: "phase_budget",
        }
    } else {
        TurnDeadline {
            at: total_deadline,
            limit_kind: "total_budget",
        }
    }
}

fn stable_draft_hash(decision: &AgentDecision) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(decision).unwrap_or_default(),
    ))
}

fn stable_progress_hash(draft_hash: &str, evidence_fingerprint: &str) -> String {
    stable_text_hash(&format!("{draft_hash}\n{evidence_fingerprint}"))
}

fn stable_text_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn append_bounded_context(
    context: &mut String,
    fragment: &str,
    seen_fragments: &mut HashSet<String>,
) {
    if fragment.trim().is_empty() {
        return;
    }
    let fragment_hash = stable_text_hash(fragment);
    if !seen_fragments.insert(fragment_hash) {
        return;
    }
    if !context.is_empty() {
        context.push('\n');
    }
    context.push_str(fragment);
    if context.chars().count() > TURN_LOOP_CONTEXT_MAX_CHARS {
        let chars = context.chars().collect::<Vec<_>>();
        *context = chars[chars.len() - TURN_LOOP_CONTEXT_MAX_CHARS..]
            .iter()
            .collect();
    }
}

fn claim_authorization_projection(review: &DecisionReviewResult) -> (Vec<Document>, String) {
    let claims = review
        .claim_analysis
        .get_array("claimManifest")
        .ok()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_document().cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let status = review
        .claim_analysis
        .get_str("evidenceStatus")
        .unwrap_or_default()
        .to_string();
    (claims, status)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct ScriptedEnvironment {
        drafts: VecDeque<AgentDecision>,
        authorizations: VecDeque<AuthorizationManifest>,
        generated: usize,
        dispatched: usize,
        committed: usize,
    }

    #[async_trait]
    impl TurnEnvironment for ScriptedEnvironment {
        async fn generate(
            &mut self,
            _request: TurnGenerateRequest,
        ) -> AppResult<(AgentDecision, Vec<String>)> {
            self.generated += 1;
            self.drafts
                .pop_front()
                .map(|decision| (decision, Vec::new()))
                .ok_or_else(|| AppError::External("draft script exhausted".to_string()))
        }

        async fn dispatch_tools(
            &mut self,
            calls: &[ToolCallRequest],
            _deadline: tokio::time::Instant,
        ) -> AppResult<ToolDispatchBatch> {
            self.dispatched += calls.len();
            Ok(ToolDispatchBatch {
                context_fragment: "verified tool result".to_string(),
                dispatched: calls.len(),
                ..ToolDispatchBatch::default()
            })
        }

        async fn authorize(
            &mut self,
            _draft: &mut DraftEnvelope,
        ) -> AppResult<AuthorizationManifest> {
            self.authorizations
                .pop_front()
                .ok_or_else(|| AppError::External("authorization script exhausted".to_string()))
        }

        async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult> {
            self.committed += 1;
            Ok(CommitResult {
                receipt: CommitReceipt {
                    status: if plan.authorization.disposition == "authorized" {
                        "simulated".to_string()
                    } else {
                        "held".to_string()
                    },
                    environment: "test".to_string(),
                    committed_at: DateTime::now(),
                    ..CommitReceipt::default()
                },
                plan,
            })
        }
    }

    fn final_decision(text: &str) -> AgentDecision {
        AgentDecision {
            decision_phase: "final".to_string(),
            next_step: "respond".to_string(),
            should_reply: true,
            reply_text: text.to_string(),
            ..AgentDecision::default()
        }
    }

    fn tool_decision() -> AgentDecision {
        AgentDecision {
            decision_phase: "tool_calling".to_string(),
            next_step: "retrieve".to_string(),
            tool_calls: vec![ToolCallRequest {
                tool: "knowledge.search".to_string(),
                arguments: doc! { "query": "specific customer question" },
            }],
            ..AgentDecision::default()
        }
    }

    fn input() -> TurnKernelInput<'static> {
        TurnKernelInput {
            run_id: "run",
            turn_id: "turn",
            authority_bundle_hash: "bundle",
        }
    }

    #[tokio::test]
    async fn tool_result_then_authorized_draft_commits_once() {
        let mut env = ScriptedEnvironment {
            drafts: VecDeque::from([tool_decision(), final_decision("answer")]),
            authorizations: VecDeque::from([AuthorizationManifest::authorized(
                "approved",
                DecisionReviewResult::default(),
            )]),
            generated: 0,
            dispatched: 0,
            committed: 0,
        };

        let outcome = run_turn(&input(), &mut env).await.unwrap();

        assert_eq!(outcome.authorization.disposition, "authorized");
        assert_eq!(env.generated, 2);
        assert_eq!(env.dispatched, 1);
        assert_eq!(env.committed, 1);
    }

    #[tokio::test]
    async fn identical_repair_draft_is_held_without_spending_all_iterations() {
        let first = final_decision("unchanged");
        let mut env = ScriptedEnvironment {
            drafts: VecDeque::from([first.clone(), first]),
            authorizations: VecDeque::from([AuthorizationManifest::repairable(
                "repair_required",
                "narrow unsupported assertion",
                DecisionReviewResult::default(),
            )]),
            generated: 0,
            dispatched: 0,
            committed: 0,
        };

        let outcome = run_turn(&input(), &mut env).await.unwrap();

        assert_eq!(outcome.authorization.final_status, "held_no_progress");
        assert!(env.generated < TURN_LOOP_MAX_ITERATIONS);
        assert_eq!(env.committed, 1);
    }

    #[tokio::test]
    async fn repeated_protocol_invalid_draft_reaches_authorizer_terminal_status() {
        struct ProtocolEnvironment {
            generated: usize,
            authorized: usize,
        }

        #[async_trait]
        impl TurnEnvironment for ProtocolEnvironment {
            async fn generate(
                &mut self,
                _request: TurnGenerateRequest,
            ) -> AppResult<(AgentDecision, Vec<String>)> {
                self.generated += 1;
                Ok((
                    AgentDecision {
                        decision_phase: "final".to_string(),
                        next_step: "repair".to_string(),
                        should_reply: false,
                        ..AgentDecision::default()
                    },
                    vec!["missing_required_field:should_reply".to_string()],
                ))
            }

            async fn dispatch_tools(
                &mut self,
                _calls: &[ToolCallRequest],
                _deadline: tokio::time::Instant,
            ) -> AppResult<ToolDispatchBatch> {
                unreachable!()
            }

            async fn authorize(
                &mut self,
                _draft: &mut DraftEnvelope,
            ) -> AppResult<AuthorizationManifest> {
                self.authorized += 1;
                if self.authorized == 1 {
                    Ok(AuthorizationManifest::repairable(
                        "repair_required",
                        "return the complete final contract",
                        DecisionReviewResult::default(),
                    ))
                } else {
                    Ok(AuthorizationManifest::held(
                        "blocked_by_required_field",
                        "protocol repair failed",
                        DecisionReviewResult::default(),
                    ))
                }
            }

            async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult> {
                Ok(CommitResult {
                    receipt: CommitReceipt {
                        status: "held".to_string(),
                        environment: "test".to_string(),
                        committed_at: DateTime::now(),
                        ..CommitReceipt::default()
                    },
                    plan,
                })
            }
        }

        let mut env = ProtocolEnvironment {
            generated: 0,
            authorized: 0,
        };
        let outcome = run_turn(&input(), &mut env).await.unwrap();

        assert_eq!(
            outcome.authorization.final_status,
            "blocked_by_required_field"
        );
        assert_eq!(env.generated, 2);
        assert_eq!(env.authorized, 2);
    }

    #[tokio::test]
    async fn draft_claim_self_report_cannot_override_authorizer() {
        let mut decision = final_decision("unsupported candidate");
        decision
            .claim_manifest
            .push(super::super::types::DraftClaim {
                claim_id: "c1".to_string(),
                text: "unsupported candidate".to_string(),
                requires_evidence: true,
                proposed_source_ids: vec!["self-reported-source".to_string()],
                ..super::super::types::DraftClaim::default()
            });
        let mut env = ScriptedEnvironment {
            drafts: VecDeque::from([decision]),
            authorizations: VecDeque::from([AuthorizationManifest::held(
                "held_by_ai_policy",
                "independent authorization found no support",
                DecisionReviewResult::default(),
            )]),
            generated: 0,
            dispatched: 0,
            committed: 0,
        };

        let outcome = run_turn(&input(), &mut env).await.unwrap();

        assert_eq!(outcome.authorization.disposition, "held");
        assert_eq!(outcome.commit_receipt.status, "held");
        assert_eq!(env.committed, 1);
    }

    #[tokio::test]
    async fn phase_timeout_covers_environment_calls() {
        struct SlowEnvironment;
        #[async_trait]
        impl TurnEnvironment for SlowEnvironment {
            async fn generate(
                &mut self,
                _request: TurnGenerateRequest,
            ) -> AppResult<(AgentDecision, Vec<String>)> {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok((final_decision("late"), Vec::new()))
            }
            async fn dispatch_tools(
                &mut self,
                _calls: &[ToolCallRequest],
                _deadline: tokio::time::Instant,
            ) -> AppResult<ToolDispatchBatch> {
                unreachable!()
            }
            async fn authorize(
                &mut self,
                _draft: &mut DraftEnvelope,
            ) -> AppResult<AuthorizationManifest> {
                unreachable!()
            }
            async fn commit(&mut self, _plan: CommitPlan) -> AppResult<CommitResult> {
                unreachable!()
            }
        }

        let error = run_turn_with_config(
            &input(),
            &mut SlowEnvironment,
            TurnLoopConfig {
                max_iterations: 4,
                max_repairs: 2,
                phase_timeout: Duration::from_millis(10),
                repair_timeout: Duration::from_millis(10),
                authorization_timeout: Duration::from_millis(10),
                total_timeout: Duration::from_millis(100),
            },
        )
        .await
        .expect_err("slow environment call must hit its phase timeout");
        assert!(error
            .to_string()
            .contains("turn_loop_timeout:generate:phase_budget"));
    }

    #[tokio::test]
    async fn initial_gateway_phases_share_phase_and_total_deadlines() {
        let phase_limited = TurnLoopTimeouts {
            phase_timeout: Duration::from_millis(10),
            repair_timeout: Duration::from_millis(100),
            authorization_timeout: Duration::from_millis(100),
            total_timeout: Duration::from_millis(100),
        };
        let phase_error = phase_limited
            .run_initial_phase(
                "knowledge_route",
                phase_limited.total_deadline_from_now(),
                async {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    Ok(())
                },
            )
            .await
            .expect_err("knowledge routing must retain its own phase cap");
        assert!(phase_error
            .to_string()
            .contains("turn_loop_timeout:knowledge_route:phase_budget"));

        let total_limited = TurnLoopTimeouts {
            phase_timeout: Duration::from_millis(100),
            ..phase_limited
        };
        let total_error = total_limited
            .run_initial_phase(
                "initial_reply",
                tokio::time::Instant::now() + Duration::from_millis(10),
                async {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    Ok(())
                },
            )
            .await
            .expect_err("the shared total deadline must cap initial reply generation");
        assert!(total_error
            .to_string()
            .contains("turn_loop_timeout:initial_reply:total_budget"));
    }

    #[tokio::test]
    async fn harness_entry_does_not_reset_an_inherited_total_deadline() {
        struct DelayedGenerateEnvironment {
            committed: usize,
        }

        #[async_trait]
        impl TurnEnvironment for DelayedGenerateEnvironment {
            async fn generate(
                &mut self,
                _request: TurnGenerateRequest,
            ) -> AppResult<(AgentDecision, Vec<String>)> {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok((final_decision("must not complete"), Vec::new()))
            }

            async fn dispatch_tools(
                &mut self,
                _calls: &[ToolCallRequest],
                _deadline: tokio::time::Instant,
            ) -> AppResult<ToolDispatchBatch> {
                unreachable!()
            }

            async fn authorize(
                &mut self,
                _draft: &mut DraftEnvelope,
            ) -> AppResult<AuthorizationManifest> {
                unreachable!()
            }

            async fn commit(&mut self, _plan: CommitPlan) -> AppResult<CommitResult> {
                self.committed += 1;
                unreachable!()
            }
        }

        let timeouts = TurnLoopTimeouts {
            phase_timeout: Duration::from_millis(100),
            repair_timeout: Duration::from_millis(100),
            authorization_timeout: Duration::from_millis(100),
            total_timeout: Duration::from_millis(100),
        };
        let total_deadline = tokio::time::Instant::now() + Duration::from_millis(35);
        timeouts
            .run_initial_phase("knowledge_route", total_deadline, async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(())
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let mut env = DelayedGenerateEnvironment { committed: 0 };
        let error = run_turn_with_deadline(&input(), &mut env, timeouts, total_deadline)
            .await
            .expect_err("elapsed pre-Harness work must consume the shared total budget");
        assert!(error
            .to_string()
            .contains("turn_loop_timeout:generate:total_budget"));
        assert_eq!(env.committed, 0);
    }

    #[tokio::test]
    async fn phase_budget_refreshes_while_total_deadline_remains_absolute() {
        struct TwoSlowPhases;

        #[async_trait]
        impl TurnEnvironment for TwoSlowPhases {
            async fn generate(
                &mut self,
                _request: TurnGenerateRequest,
            ) -> AppResult<(AgentDecision, Vec<String>)> {
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok((final_decision("ready"), Vec::new()))
            }

            async fn dispatch_tools(
                &mut self,
                _calls: &[ToolCallRequest],
                _deadline: tokio::time::Instant,
            ) -> AppResult<ToolDispatchBatch> {
                unreachable!()
            }

            async fn authorize(
                &mut self,
                _draft: &mut DraftEnvelope,
            ) -> AppResult<AuthorizationManifest> {
                tokio::time::sleep(Duration::from_millis(80)).await;
                Ok(AuthorizationManifest::authorized(
                    "approved",
                    DecisionReviewResult::default(),
                ))
            }

            async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult> {
                Ok(CommitResult {
                    plan,
                    receipt: CommitReceipt::default(),
                })
            }
        }

        let outcome = run_turn_with_config(
            &input(),
            &mut TwoSlowPhases,
            TurnLoopConfig {
                max_iterations: 2,
                max_repairs: 1,
                phase_timeout: Duration::from_millis(60),
                repair_timeout: Duration::from_millis(100),
                authorization_timeout: Duration::from_millis(100),
                total_timeout: Duration::from_millis(250),
            },
        )
        .await
        .expect("generation must not consume the authorization phase budget");
        assert_eq!(outcome.authorization.disposition, "authorized");

        let authorization_error = run_turn_with_config(
            &input(),
            &mut TwoSlowPhases,
            TurnLoopConfig {
                max_iterations: 2,
                max_repairs: 1,
                phase_timeout: Duration::from_millis(60),
                repair_timeout: Duration::from_millis(60),
                authorization_timeout: Duration::from_millis(20),
                total_timeout: Duration::from_millis(250),
            },
        )
        .await
        .expect_err("authorization must use its own phase budget");
        assert!(authorization_error
            .to_string()
            .contains("turn_loop_timeout:authorization:phase_budget"));

        let error = run_turn_with_config(
            &input(),
            &mut TwoSlowPhases,
            TurnLoopConfig {
                max_iterations: 2,
                max_repairs: 1,
                phase_timeout: Duration::from_millis(200),
                repair_timeout: Duration::from_millis(200),
                authorization_timeout: Duration::from_millis(200),
                total_timeout: Duration::from_millis(100),
            },
        )
        .await
        .expect_err("the absolute turn deadline must still cap multiple phases");
        assert!(error
            .to_string()
            .contains("turn_loop_timeout:authorization:total_budget"));
    }

    #[tokio::test]
    async fn repeated_tool_plan_may_continue_when_verified_evidence_changes() {
        struct EvidenceEnvironment {
            drafts: VecDeque<AgentDecision>,
            evidence: VecDeque<&'static str>,
            dispatched: usize,
        }
        #[async_trait]
        impl TurnEnvironment for EvidenceEnvironment {
            async fn generate(
                &mut self,
                _request: TurnGenerateRequest,
            ) -> AppResult<(AgentDecision, Vec<String>)> {
                Ok((
                    self.drafts
                        .pop_front()
                        .ok_or_else(|| AppError::External("draft script exhausted".to_string()))?,
                    Vec::new(),
                ))
            }

            async fn dispatch_tools(
                &mut self,
                calls: &[ToolCallRequest],
                _deadline: tokio::time::Instant,
            ) -> AppResult<ToolDispatchBatch> {
                self.dispatched += calls.len();
                let fingerprint = self.evidence.pop_front().unwrap_or_default().to_string();
                Ok(ToolDispatchBatch {
                    context_fragment: format!("verified evidence {fingerprint}"),
                    dispatched: calls.len(),
                    evidence_fingerprint: fingerprint,
                    ..ToolDispatchBatch::default()
                })
            }

            async fn authorize(
                &mut self,
                _draft: &mut DraftEnvelope,
            ) -> AppResult<AuthorizationManifest> {
                Ok(AuthorizationManifest::authorized(
                    "approved",
                    DecisionReviewResult::default(),
                ))
            }

            async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult> {
                Ok(CommitResult {
                    receipt: CommitReceipt {
                        status: if plan.authorization.disposition == "authorized" {
                            "simulated".to_string()
                        } else {
                            "held".to_string()
                        },
                        environment: "test".to_string(),
                        committed_at: DateTime::now(),
                        ..CommitReceipt::default()
                    },
                    plan,
                })
            }
        }

        let tool = tool_decision();
        let mut env = EvidenceEnvironment {
            drafts: VecDeque::from([tool.clone(), tool, final_decision("answer")]),
            evidence: VecDeque::from(["ledger-1", "ledger-2"]),
            dispatched: 0,
        };

        let outcome = run_turn(&input(), &mut env).await.unwrap();

        assert_eq!(outcome.authorization.disposition, "authorized");
        assert_eq!(env.dispatched, 2);
    }

    #[tokio::test]
    async fn repeated_tool_plan_is_held_when_verified_evidence_does_not_change() {
        struct NoProgressEnvironment {
            generated: usize,
            dispatched: usize,
        }
        #[async_trait]
        impl TurnEnvironment for NoProgressEnvironment {
            async fn generate(
                &mut self,
                _request: TurnGenerateRequest,
            ) -> AppResult<(AgentDecision, Vec<String>)> {
                self.generated += 1;
                Ok((tool_decision(), Vec::new()))
            }

            async fn dispatch_tools(
                &mut self,
                calls: &[ToolCallRequest],
                _deadline: tokio::time::Instant,
            ) -> AppResult<ToolDispatchBatch> {
                self.dispatched += calls.len();
                Ok(ToolDispatchBatch {
                    context_fragment: "same evidence".to_string(),
                    dispatched: calls.len(),
                    evidence_fingerprint: "same-ledger".to_string(),
                    ..ToolDispatchBatch::default()
                })
            }

            async fn authorize(
                &mut self,
                _draft: &mut DraftEnvelope,
            ) -> AppResult<AuthorizationManifest> {
                unreachable!()
            }

            async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult> {
                Ok(CommitResult {
                    receipt: CommitReceipt {
                        status: "held".to_string(),
                        environment: "test".to_string(),
                        committed_at: DateTime::now(),
                        ..CommitReceipt::default()
                    },
                    plan,
                })
            }
        }

        let mut env = NoProgressEnvironment {
            generated: 0,
            dispatched: 0,
        };
        let outcome = run_turn(&input(), &mut env).await.unwrap();

        assert_eq!(outcome.authorization.final_status, "held_no_progress");
        assert_eq!(env.generated, 2);
        assert_eq!(env.dispatched, 2);
    }

    #[tokio::test]
    async fn authorized_empty_send_is_downgraded_to_held_before_commit() {
        let empty = AgentDecision {
            decision_phase: "final".to_string(),
            next_step: "respond".to_string(),
            should_reply: true,
            reply_text: "   ".to_string(),
            ..AgentDecision::default()
        };
        let mut env = ScriptedEnvironment {
            drafts: VecDeque::from([empty]),
            authorizations: VecDeque::from([AuthorizationManifest::authorized(
                "approved",
                DecisionReviewResult::default(),
            )]),
            generated: 0,
            dispatched: 0,
            committed: 0,
        };

        let outcome = run_turn(&input(), &mut env).await.unwrap();

        assert_eq!(
            outcome.authorization.final_status,
            "held_invalid_authorized_draft"
        );
        assert_eq!(outcome.commit_receipt.status, "held");
    }

    #[tokio::test]
    async fn durable_commit_is_not_cancelled_by_the_model_loop_deadline() {
        struct SlowCommitEnvironment;
        #[async_trait]
        impl TurnEnvironment for SlowCommitEnvironment {
            async fn generate(
                &mut self,
                _request: TurnGenerateRequest,
            ) -> AppResult<(AgentDecision, Vec<String>)> {
                Ok((final_decision("ready"), Vec::new()))
            }

            async fn dispatch_tools(
                &mut self,
                _calls: &[ToolCallRequest],
                _deadline: tokio::time::Instant,
            ) -> AppResult<ToolDispatchBatch> {
                unreachable!()
            }

            async fn authorize(
                &mut self,
                _draft: &mut DraftEnvelope,
            ) -> AppResult<AuthorizationManifest> {
                Ok(AuthorizationManifest::authorized(
                    "approved",
                    DecisionReviewResult::default(),
                ))
            }

            async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult> {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(CommitResult {
                    plan,
                    receipt: CommitReceipt::default(),
                })
            }
        }

        let outcome = run_turn_with_config(
            &input(),
            &mut SlowCommitEnvironment,
            TurnLoopConfig {
                max_iterations: 2,
                max_repairs: 1,
                phase_timeout: Duration::from_millis(10),
                repair_timeout: Duration::from_millis(10),
                authorization_timeout: Duration::from_millis(10),
                total_timeout: Duration::from_millis(10),
            },
        )
        .await
        .expect("durable commit must finish once authorization has completed");

        assert_eq!(outcome.draft.decision.reply_text, "ready");
    }

    #[tokio::test]
    async fn authorization_finalization_mutates_the_committed_terminal_draft() {
        struct FinalizingHoldEnvironment;

        #[async_trait]
        impl TurnEnvironment for FinalizingHoldEnvironment {
            async fn generate(
                &mut self,
                _request: TurnGenerateRequest,
            ) -> AppResult<(AgentDecision, Vec<String>)> {
                Ok((final_decision("candidate"), Vec::new()))
            }

            async fn dispatch_tools(
                &mut self,
                _calls: &[ToolCallRequest],
                _deadline: tokio::time::Instant,
            ) -> AppResult<ToolDispatchBatch> {
                unreachable!()
            }

            async fn authorize(
                &mut self,
                draft: &mut DraftEnvelope,
            ) -> AppResult<AuthorizationManifest> {
                draft.decision.should_reply = false;
                Ok(AuthorizationManifest::held(
                    "held_by_authorizer",
                    "terminal authorization hold",
                    DecisionReviewResult::default(),
                ))
            }

            async fn commit(&mut self, plan: CommitPlan) -> AppResult<CommitResult> {
                assert!(!plan.draft.decision.should_reply);
                Ok(CommitResult {
                    receipt: CommitReceipt {
                        status: "held".to_string(),
                        environment: "test".to_string(),
                        committed_at: DateTime::now(),
                        ..CommitReceipt::default()
                    },
                    plan,
                })
            }
        }

        let outcome = run_turn(&input(), &mut FinalizingHoldEnvironment)
            .await
            .unwrap();

        assert_eq!(outcome.authorization.disposition, "held");
        assert!(!outcome.draft.decision.should_reply);
        assert_eq!(
            outcome.draft.draft_hash,
            stable_draft_hash(&outcome.draft.decision)
        );
    }
}

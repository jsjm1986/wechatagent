//! 决策请示通道——台账 CRUD 层（pending 台账增删查改 / 知识缺口提案 / relay task 入队）。
//! 全部 async + db 访问。

use super::logic::{is_duplicate_key_error, is_pending_dedupe_conflict, short_code_from_seed};
use crate::error::{AppError, AppResult};
use crate::models::{
    AgentPrincipalEscalation, AgentTask, AskHumanPolicy, AuthorityObservation,
    OperationDomainConfig, OperationKnowledgeChunk, PrincipalDecision, PrincipalEscalationProtocol,
    ALLOWED_ESCALATION_CATEGORY, PRINCIPAL_CARD_DELIVERY_FAILED_TERMINAL,
    PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE, PRINCIPAL_CARD_DELIVERY_QUEUED,
    PRINCIPAL_CARD_DELIVERY_SENT, PRINCIPAL_CARD_DELIVERY_UNKNOWN,
    PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED, PRINCIPAL_RELAY_STATE_ENQUEUED,
    PRINCIPAL_RELAY_STATE_PENDING, PRINCIPAL_RELAY_STATE_TERMINAL,
};
use crate::routes::AppState;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::{
    FindOneAndUpdateOptions, ReturnDocument, TransactionOptions, UpdateOptions,
};

#[allow(clippy::too_many_arguments)]
fn build_pending_escalation_entry(
    id: Option<ObjectId>,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    short_code: String,
    category: &str,
    reason: &str,
    question_for_principal: &str,
    principal_wxid: &str,
    is_generalizable: bool,
    domain_config: &OperationDomainConfig,
    policy: AskHumanPolicy,
    principal_account_id: &str,
    delivery_content: String,
    now: DateTime,
) -> AgentPrincipalEscalation {
    AgentPrincipalEscalation {
        id,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: contact_wxid.to_string(),
        short_code,
        status: PRINCIPAL_ESCALATION_STATUS_PENDING.to_string(),
        category: category.to_string(),
        reason: reason.to_string(),
        question_for_principal: question_for_principal.to_string(),
        principal_wxid: principal_wxid.to_string(),
        protocol: Some(PrincipalEscalationProtocol {
            domain: domain_config.domain.clone(),
            policy_version: domain_config.version,
            policy,
            principal_account_id: principal_account_id.to_string(),
            delivery_generation: 1,
            delivery_state: PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE.to_string(),
            delivery_content,
            delivery_outbox_id: None,
            failure_cleanup_completed_at: None,
        }),
        decision: None,
        authorization_expires_at: None,
        is_generalizable,
        knowledge_proposal_emitted: false,
        last_holding_reply_ms: None,
        // Only a confirmed Outbox delivery may set this timestamp.
        last_pushed_at_ms: None,
        created_at: now,
        updated_at: now,
        resolved_at: None,
        resolved_via: None,
        relay_state: None,
        relay_task_id: None,
        relay_enqueued_at: None,
        relay_terminal_at: None,
        relay_terminal_reason: None,
    }
}

const PRINCIPAL_INTENT_CLAIM_TIMEOUT_MS: i64 = 120_000;
const PRINCIPAL_INTENT_RECONCILE_LIMIT: usize = 100;

#[derive(Debug)]
enum PrincipalIntentOutcome {
    Materialized(ObjectId),
    Deduplicated(ObjectId),
    Deferred(String),
    Invalid(String),
}

#[derive(Debug)]
enum DeterministicEscalationInsert {
    Entry(AgentPrincipalEscalation),
    Deduplicated(ObjectId),
}

/// Materialize one review-scoped ask-human intent immediately after commit.
/// Periodic reconciliation uses the same claim/fencing path, so this is only a
/// latency optimization and never the sole execution opportunity.
pub(crate) async fn materialize_principal_escalation_intent(
    state: &AppState,
    review_id: ObjectId,
) -> AppResult<bool> {
    let Some(review) = claim_principal_escalation_intent(state, Some(review_id)).await? else {
        return Ok(false);
    };
    settle_claimed_principal_intent(state, review).await?;
    Ok(true)
}

/// Recover pending/retry ask-human intents written atomically with decision
/// reviews. Each claim is fenced; multiple workers may call this concurrently.
pub(crate) async fn reconcile_principal_escalation_intents_once(
    state: &AppState,
) -> AppResult<u64> {
    let mut settled = 0_u64;
    for _ in 0..PRINCIPAL_INTENT_RECONCILE_LIMIT {
        let Some(review) = claim_principal_escalation_intent(state, None).await? else {
            break;
        };
        match settle_claimed_principal_intent(state, review).await {
            Ok(()) => settled += 1,
            Err(error) => {
                tracing::warn!(%error, "principal escalation intent reconciliation failed");
            }
        }
    }
    Ok(settled)
}

async fn claim_principal_escalation_intent(
    state: &AppState,
    review_id: Option<ObjectId>,
) -> AppResult<Option<Document>> {
    let now = DateTime::now();
    let stale_before =
        DateTime::from_millis(now.timestamp_millis() - PRINCIPAL_INTENT_CLAIM_TIMEOUT_MS);
    let mut filter = doc! {
        "$or": [
            {
                "principal_escalation_intent.status": { "$in": [
                    super::PRINCIPAL_INTENT_STATUS_PENDING,
                    super::PRINCIPAL_INTENT_STATUS_RETRY,
                ] },
                "$or": [
                    { "principal_escalation_intent.next_retry_at": { "$lte": now } },
                    { "principal_escalation_intent.next_retry_at": { "$exists": false } },
                ],
            },
            {
                "principal_escalation_intent.status": super::PRINCIPAL_INTENT_STATUS_PROCESSING,
                "principal_escalation_intent.claimed_at": { "$lt": stale_before },
            },
        ],
    };
    if let Some(review_id) = review_id {
        filter.insert("_id", review_id);
    }
    let claim_token = uuid::Uuid::new_v4().to_string();
    Ok(state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .find_one_and_update(
            filter,
            doc! {
                "$set": {
                    "principal_escalation_intent.status": super::PRINCIPAL_INTENT_STATUS_PROCESSING,
                    "principal_escalation_intent.claim_token": &claim_token,
                    "principal_escalation_intent.claimed_at": now,
                    "principal_escalation_intent.updated_at": now,
                },
                "$inc": {
                    "principal_escalation_intent.attempts": 1i64,
                    "principal_escalation_intent.claim_generation": 1i64,
                },
            },
            FindOneAndUpdateOptions::builder()
                .sort(doc! { "principal_escalation_intent.next_retry_at": 1, "created_at": 1 })
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?)
}

async fn settle_claimed_principal_intent(state: &AppState, review: Document) -> AppResult<()> {
    let review_id = review.get_object_id("_id").map_err(|error| {
        AppError::External(format!("principal intent review id missing: {error}"))
    })?;
    let intent = review
        .get_document("principal_escalation_intent")
        .map_err(|error| {
            AppError::External(format!("principal intent payload missing: {error}"))
        })?;
    let claim_token = intent
        .get_str("claim_token")
        .map_err(|error| {
            AppError::External(format!("principal intent claim token missing: {error}"))
        })?
        .to_string();
    let attempts = intent
        .get_i64("attempts")
        .or_else(|_| intent.get_i32("attempts").map(i64::from))
        .unwrap_or(1);

    match process_claimed_principal_intent(state, &review).await {
        Ok(PrincipalIntentOutcome::Materialized(escalation_id)) => {
            finish_principal_intent(
                state,
                review_id,
                &claim_token,
                super::PRINCIPAL_INTENT_STATUS_MATERIALIZED,
                Some(escalation_id),
                None,
            )
            .await?;
            Ok(())
        }
        Ok(PrincipalIntentOutcome::Deduplicated(escalation_id)) => {
            finish_principal_intent(
                state,
                review_id,
                &claim_token,
                super::PRINCIPAL_INTENT_STATUS_DEDUPLICATED,
                Some(escalation_id),
                None,
            )
            .await?;
            Ok(())
        }
        Ok(PrincipalIntentOutcome::Deferred(reason)) => {
            retry_principal_intent(state, review_id, &claim_token, attempts, &reason).await?;
            Ok(())
        }
        Ok(PrincipalIntentOutcome::Invalid(reason)) => {
            finish_principal_intent(
                state,
                review_id,
                &claim_token,
                super::PRINCIPAL_INTENT_STATUS_INVALID,
                None,
                Some(&reason),
            )
            .await?;
            Ok(())
        }
        Err(error) => {
            let _ = retry_principal_intent(
                state,
                review_id,
                &claim_token,
                attempts,
                &error.to_string(),
            )
            .await;
            Err(error)
        }
    }
}

async fn process_claimed_principal_intent(
    state: &AppState,
    review: &Document,
) -> AppResult<PrincipalIntentOutcome> {
    let review_id = review.get_object_id("_id").map_err(|error| {
        AppError::External(format!("principal intent review id missing: {error}"))
    })?;
    let workspace_id = review.get_str("workspace_id").map_err(|error| {
        AppError::External(format!("principal intent workspace missing: {error}"))
    })?;
    let account_id = review.get_str("account_id").map_err(|error| {
        AppError::External(format!("principal intent account missing: {error}"))
    })?;
    let contact_wxid = review.get_str("contact_wxid").map_err(|error| {
        AppError::External(format!("principal intent contact missing: {error}"))
    })?;
    let intent = review
        .get_document("principal_escalation_intent")
        .map_err(|error| {
            AppError::External(format!("principal intent payload missing: {error}"))
        })?;
    let request_doc = intent
        .get_document("request")
        .map_err(|error| AppError::External(format!("principal intent request missing: {error}")))?
        .clone();
    let request: crate::models::EscalationRequest = mongodb::bson::from_document(request_doc)
        .map_err(|error| {
            AppError::External(format!("principal intent request invalid: {error}"))
        })?;
    let Some(category) = request.category.as_deref() else {
        return Ok(PrincipalIntentOutcome::Invalid(
            "principal intent category missing".to_string(),
        ));
    };
    let Some(reason) = request
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(PrincipalIntentOutcome::Invalid(
            "principal intent reason missing".to_string(),
        ));
    };
    let Some(question) = request
        .question_for_principal
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(PrincipalIntentOutcome::Invalid(
            "principal intent question missing".to_string(),
        ));
    };
    if !request.needed || !ALLOWED_ESCALATION_CATEGORY.contains(&category) {
        return Ok(PrincipalIntentOutcome::Invalid(
            "principal intent request is outside the transport schema".to_string(),
        ));
    }

    if let Some(existing) = state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "_id": review_id }, None)
        .await?
    {
        if existing.workspace_id != workspace_id
            || existing.account_id != account_id
            || existing.contact_wxid != contact_wxid
            || existing.category != category
        {
            return Ok(PrincipalIntentOutcome::Invalid(
                "deterministic escalation identity conflicts with another scope".to_string(),
            ));
        }
        if existing.status == PRINCIPAL_ESCALATION_STATUS_PENDING {
            materialize_principal_card_delivery(state, &existing).await?;
        }
        return Ok(PrincipalIntentOutcome::Materialized(review_id));
    }

    if let Some(existing) = state
        .db
        .agent_principal_escalations()
        .find_one(
            doc! {
                "_id": { "$ne": review_id },
                "workspace_id": workspace_id,
                "account_id": account_id,
                "contact_wxid": contact_wxid,
                "category": category,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            None,
        )
        .await?
    {
        if let Some(existing_id) = existing.id {
            return Ok(PrincipalIntentOutcome::Deduplicated(existing_id));
        }
    }

    let Some(contact) = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": contact_wxid,
            },
            None,
        )
        .await?
    else {
        return Ok(PrincipalIntentOutcome::Deferred(
            "customer contact is not available yet".to_string(),
        ));
    };
    let Some(domain_config) = crate::agent::load_user_operation_domain_config_for_contact(
        state,
        workspace_id,
        contact_wxid,
    )
    .await?
    else {
        return Ok(PrincipalIntentOutcome::Deferred(
            "ask-human configuration is not available yet".to_string(),
        ));
    };
    let policy = super::resolve_ask_human_policy(&domain_config);
    if super::stuck_suppressed(category, &policy) {
        return Ok(PrincipalIntentOutcome::Deferred(
            "this escalation category is disabled by current policy".to_string(),
        ));
    }
    let frozen_policy = super::freeze_ask_human_policy(&policy, account_id);
    let Some(decider) = frozen_policy.decider_chain.first() else {
        return Ok(PrincipalIntentOutcome::Deferred(
            "ask-human decider chain is not configured yet".to_string(),
        ));
    };
    let principal_wxid = decider.wxid.trim().to_string();
    let Some(principal_account_id) = decider
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return Ok(PrincipalIntentOutcome::Deferred(
            "ask-human decider account is not configured yet".to_string(),
        ));
    };
    if principal_wxid.is_empty() || principal_wxid == contact_wxid {
        return Ok(PrincipalIntentOutcome::Deferred(
            "ask-human decider identity is invalid".to_string(),
        ));
    }
    let now_ms = DateTime::now().timestamp_millis();
    let since_ms = now_ms - 24 * 3600 * 1000;
    let today = super::count_pushes_today(state, workspace_id, &principal_wxid, since_ms).await?;
    let last_push = super::latest_push_ms(state, workspace_id, &principal_wxid).await?;
    if !super::push_allowed(&policy, today, last_push, now_ms) {
        return Ok(PrincipalIntentOutcome::Deferred(
            "principal push policy is temporarily blocking delivery".to_string(),
        ));
    }
    let customer_label = contact
        .remark
        .clone()
        .or(contact.nickname.clone())
        .or(contact.alias.clone())
        .unwrap_or_else(|| contact.wxid.clone());
    match insert_pending_escalation_with_id(
        state,
        review_id,
        workspace_id,
        account_id,
        contact_wxid,
        category,
        reason,
        question,
        &principal_wxid,
        request.is_generalizable,
        &domain_config,
        frozen_policy,
        &principal_account_id,
        &customer_label,
    )
    .await?
    {
        DeterministicEscalationInsert::Entry(entry) => {
            if entry.status == PRINCIPAL_ESCALATION_STATUS_PENDING {
                materialize_principal_card_delivery(state, &entry).await?;
            }
            Ok(PrincipalIntentOutcome::Materialized(review_id))
        }
        DeterministicEscalationInsert::Deduplicated(existing_id) => {
            Ok(PrincipalIntentOutcome::Deduplicated(existing_id))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_pending_escalation_with_id(
    state: &AppState,
    escalation_id: ObjectId,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    category: &str,
    reason: &str,
    question_for_principal: &str,
    principal_wxid: &str,
    is_generalizable: bool,
    domain_config: &OperationDomainConfig,
    policy: AskHumanPolicy,
    principal_account_id: &str,
    customer_label: &str,
) -> AppResult<DeterministicEscalationInsert> {
    let now = DateTime::now();
    for attempt in 0..5u32 {
        let seed = (now.timestamp_millis() as u64)
            .wrapping_add(attempt as u64 * 2_654_435_761)
            .wrapping_add(u64::from_be_bytes([
                0,
                0,
                0,
                0,
                escalation_id.bytes()[0],
                escalation_id.bytes()[1],
                escalation_id.bytes()[2],
                escalation_id.bytes()[3],
            ])) as u32;
        let short_code = short_code_from_seed(seed);
        let delivery_content = super::logic::render_principal_card(
            &short_code,
            customer_label,
            reason,
            question_for_principal,
        );
        let entry = build_pending_escalation_entry(
            Some(escalation_id),
            workspace_id,
            account_id,
            contact_wxid,
            short_code,
            category,
            reason,
            question_for_principal,
            principal_wxid,
            is_generalizable,
            domain_config,
            policy.clone(),
            principal_account_id,
            delivery_content,
            now,
        );
        match state
            .db
            .agent_principal_escalations()
            .insert_one(&entry, None)
            .await
        {
            Ok(_) => return Ok(DeterministicEscalationInsert::Entry(entry)),
            Err(error) => {
                if let Some(existing) = state
                    .db
                    .agent_principal_escalations()
                    .find_one(doc! { "_id": escalation_id }, None)
                    .await?
                {
                    return Ok(DeterministicEscalationInsert::Entry(existing));
                }
                if is_pending_dedupe_conflict(&error) {
                    if let Some(existing) = state
                        .db
                        .agent_principal_escalations()
                        .find_one(
                            doc! {
                                "workspace_id": workspace_id,
                                "account_id": account_id,
                                "contact_wxid": contact_wxid,
                                "category": category,
                                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                            },
                            None,
                        )
                        .await?
                    {
                        if let Some(existing_id) = existing.id {
                            return Ok(DeterministicEscalationInsert::Deduplicated(existing_id));
                        }
                    }
                }
                if is_duplicate_key_error(&error) {
                    continue;
                }
                return Err(error.into());
            }
        }
    }
    Err(AppError::External(
        "deterministic escalation short-code allocation exhausted".to_string(),
    ))
}

async fn finish_principal_intent(
    state: &AppState,
    review_id: ObjectId,
    claim_token: &str,
    status: &str,
    escalation_id: Option<ObjectId>,
    error: Option<&str>,
) -> AppResult<()> {
    let now = DateTime::now();
    let mut set = doc! {
        "principal_escalation_intent.status": status,
        "principal_escalation_intent.updated_at": now,
        "principal_escalation_intent.completed_at": now,
    };
    if let Some(escalation_id) = escalation_id {
        set.insert("principal_escalation_intent.escalation_id", escalation_id);
    }
    if let Some(error) = error {
        set.insert(
            "principal_escalation_intent.last_error",
            error.chars().take(1024).collect::<String>(),
        );
    }
    let mut unset = doc! {
        "principal_escalation_intent.claim_token": "",
        "principal_escalation_intent.claimed_at": "",
        "principal_escalation_intent.next_retry_at": "",
    };
    if error.is_none() {
        unset.insert("principal_escalation_intent.last_error", "");
    }
    let result = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! {
                "_id": review_id,
                "principal_escalation_intent.status": super::PRINCIPAL_INTENT_STATUS_PROCESSING,
                "principal_escalation_intent.claim_token": claim_token,
            },
            doc! {
                "$set": set,
                "$unset": unset,
            },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        tracing::info!(
            %review_id,
            %claim_token,
            terminal_status = status,
            "discarded stale principal escalation intent result after claim ownership changed"
        );
    }
    Ok(())
}

async fn retry_principal_intent(
    state: &AppState,
    review_id: ObjectId,
    claim_token: &str,
    attempts: i64,
    reason: &str,
) -> AppResult<()> {
    let exponent = attempts.saturating_sub(1).clamp(0, 6) as u32;
    let delay_seconds = (60_i64.saturating_mul(1_i64 << exponent)).min(3600);
    let now = DateTime::now();
    let next_retry_at =
        DateTime::from_millis(now.timestamp_millis() + delay_seconds.saturating_mul(1000));
    let result = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! {
                "_id": review_id,
                "principal_escalation_intent.status": super::PRINCIPAL_INTENT_STATUS_PROCESSING,
                "principal_escalation_intent.claim_token": claim_token,
            },
            doc! {
                "$set": {
                    "principal_escalation_intent.status": super::PRINCIPAL_INTENT_STATUS_RETRY,
                    "principal_escalation_intent.next_retry_at": next_retry_at,
                    "principal_escalation_intent.last_error": reason.chars().take(1024).collect::<String>(),
                    "principal_escalation_intent.updated_at": now,
                },
                "$unset": {
                    "principal_escalation_intent.claim_token": "",
                    "principal_escalation_intent.claimed_at": "",
                },
            },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        tracing::info!(
            %review_id,
            %claim_token,
            "discarded stale principal escalation intent retry after claim ownership changed"
        );
    }
    Ok(())
}

/// Materialize one frozen principal-card intent into the existing durable Outbox.
/// The source event is deterministic per escalation generation, so a crash after
/// Outbox insert but before the acknowledgement update converges to the same row.
pub(crate) async fn materialize_principal_card_delivery(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<()> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let protocol = entry
        .protocol
        .as_ref()
        .ok_or_else(|| AppError::Conflict("principal escalation protocol missing".to_string()))?;
    if entry.status != PRINCIPAL_ESCALATION_STATUS_PENDING
        || protocol.delivery_state != PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE
    {
        return Ok(());
    }
    activate_awaiting_principal_owner(state, entry).await?;
    let generation = protocol.delivery_generation;
    let source_event_id = format!("principal-card:{}:{generation}", escalation_id.to_hex());
    let outcome = crate::agent::outbox::enqueue(
        state,
        crate::agent::outbox::EnqueueRequest {
            workspace_id: entry.workspace_id.clone(),
            account_id: protocol.principal_account_id.clone(),
            contact_wxid: entry.principal_wxid.clone(),
            run_id: source_event_id.clone(),
            decision_id: None,
            source_event_id,
            source_kind: crate::agent::run_envelope::SOURCE_KIND_PRINCIPAL_ESCALATION.to_string(),
            content: protocol.delivery_content.clone(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await?;
    let outbox_id = match outcome {
        crate::agent::outbox::EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        crate::agent::outbox::EnqueueOutcome::IdempotentSkip {
            existing_outbox_id, ..
        } => existing_outbox_id,
    };
    state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "_id": escalation_id,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.delivery_generation": generation,
                "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE,
            },
            doc! { "$set": {
                "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_QUEUED,
                "protocol.delivery_outbox_id": outbox_id,
            } },
            None,
        )
        .await?;
    Ok(())
}

/// Reconcile principal-card Outbox facts back into escalation state and recover
/// interrupted enqueue acknowledgements. Legacy rows without a protocol are ignored.
pub(crate) async fn reconcile_principal_card_deliveries_once(state: &AppState) -> AppResult<u64> {
    use futures::TryStreamExt;
    let mut cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! { "$or": [
                {
                    "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                    "protocol.delivery_state": { "$in": [
                        PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE,
                        PRINCIPAL_CARD_DELIVERY_QUEUED,
                    ] },
                },
                {
                    "status": PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED,
                    "protocol.failure_cleanup_completed_at": { "$exists": false },
                },
            ] },
            mongodb::options::FindOptions::builder().limit(100).build(),
        )
        .await?;
    let mut changed = 0_u64;
    while let Some(entry) = cursor.try_next().await? {
        let Some(protocol) = entry.protocol.as_ref() else {
            continue;
        };
        if entry.status == PRINCIPAL_ESCALATION_STATUS_PENDING {
            activate_awaiting_principal_owner(state, &entry).await?;
        }
        if entry.status == PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED {
            changed += u64::from(complete_failed_delivery_cleanup(state, &entry).await?);
            continue;
        }
        if protocol.delivery_state == PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE {
            materialize_principal_card_delivery(state, &entry).await?;
            changed += 1;
            continue;
        }
        let Some(outbox_id) = protocol.delivery_outbox_id else {
            continue;
        };
        let Some(outbox) = state
            .db
            .collection_agent_send_outbox()
            .find_one(doc! { "_id": outbox_id }, None)
            .await?
        else {
            continue;
        };
        let (delivery_state, delivered_at, escalation_status) = match outbox.status.as_str() {
            "sent" => (
                PRINCIPAL_CARD_DELIVERY_SENT,
                outbox.sent_at.unwrap_or(outbox.updated_at),
                PRINCIPAL_ESCALATION_STATUS_PENDING,
            ),
            "failed_terminal" | "canceled" => (
                PRINCIPAL_CARD_DELIVERY_FAILED_TERMINAL,
                outbox.updated_at,
                PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED,
            ),
            "delivery_unknown" => (
                PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                outbox.updated_at,
                PRINCIPAL_ESCALATION_STATUS_PENDING,
            ),
            _ => continue,
        };
        let generation = protocol.delivery_generation;
        let mut set = doc! {
            "protocol.delivery_state": delivery_state,
            "status": escalation_status,
        };
        if delivery_state == PRINCIPAL_CARD_DELIVERY_SENT {
            set.insert("last_pushed_at_ms", delivered_at.timestamp_millis());
            set.insert("updated_at", delivered_at);
        }
        let result = state
            .db
            .agent_principal_escalations()
            .update_one(
                doc! {
                    "_id": entry.id,
                    "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                    "protocol.delivery_generation": generation,
                    "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_QUEUED,
                    "protocol.delivery_outbox_id": outbox_id,
                },
                doc! { "$set": set },
                None,
            )
            .await?;
        changed += result.modified_count;
        if result.modified_count == 1
            && escalation_status == PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED
        {
            let _ = complete_failed_delivery_cleanup(state, &entry).await?;
        }
    }
    Ok(changed)
}

fn awaiting_owner_id(escalation_id: mongodb::bson::oid::ObjectId) -> String {
    escalation_id.to_hex()
}

fn awaiting_owner_patch(awaiting: mongodb::bson::Bson, owners: mongodb::bson::Bson) -> Document {
    let mut patch = Document::new();
    patch.insert(crate::models::AWAITING_PRINCIPAL_DECISION_ATTR, awaiting);
    patch.insert(crate::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR, owners);
    patch
}

fn activate_awaiting_owner_pipeline(owner: &str, now: DateTime) -> Vec<Document> {
    let owners_key = crate::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR;
    let owners_path = format!("$domain_attributes.{owners_key}");
    let patch = awaiting_owner_patch(
        true.into(),
        doc! { "$setUnion": ["$$owners", [owner]] }.into(),
    );
    vec![doc! { "$set": {
        "domain_attributes": {
            "$let": {
                "vars": {
                    "attrs": { "$cond": [
                        { "$eq": [{ "$type": "$domain_attributes" }, "object"] },
                        "$domain_attributes",
                        {},
                    ] },
                    "owners": { "$cond": [
                        { "$isArray": &owners_path },
                        &owners_path,
                        [],
                    ] },
                },
                "in": { "$mergeObjects": ["$$attrs", patch] },
            },
        },
        "domain_attributes_updated_at": now,
    } }]
}

fn remove_awaiting_owner_pipeline(owner: &str, now: DateTime) -> Vec<Document> {
    let owners_key = crate::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR;
    let owners_path = format!("$domain_attributes.{owners_key}");
    let patch = awaiting_owner_patch(
        doc! { "$gt": [{ "$size": "$$remaining" }, 0] }.into(),
        "$$remaining".into(),
    );
    vec![doc! { "$set": {
        "domain_attributes": {
            "$let": {
                "vars": {
                    "attrs": { "$cond": [
                        { "$eq": [{ "$type": "$domain_attributes" }, "object"] },
                        "$domain_attributes",
                        {},
                    ] },
                    "owners": { "$cond": [
                        { "$isArray": &owners_path },
                        &owners_path,
                        [],
                    ] },
                },
                "in": {
                    "$let": {
                        "vars": {
                            "remaining": { "$filter": {
                                "input": "$$owners",
                                "as": "candidate",
                                "cond": { "$ne": ["$$candidate", owner] },
                            } },
                        },
                        "in": { "$mergeObjects": ["$$attrs", patch] },
                    },
                },
            },
        },
        "domain_attributes_updated_at": now,
    } }]
}

async fn activate_awaiting_principal_owner(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<()> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let owner = awaiting_owner_id(escalation_id);
    let result = state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid,
            },
            activate_awaiting_owner_pipeline(&owner, DateTime::now()),
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "principal_escalation_contact_missing".to_string(),
        ));
    }
    Ok(())
}

async fn remove_awaiting_principal_owner(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<()> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let owner = awaiting_owner_id(escalation_id);
    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid,
            },
            remove_awaiting_owner_pipeline(&owner, DateTime::now()),
            None,
        )
        .await?;
    Ok(())
}

/// Mark a resolved principal relay as terminal, then release only this
/// escalation's awaiting ownership. Missing `relay_state` is accepted only
/// through this explicit task-bound path for rolling-upgrade compatibility;
/// background reconciliation still never guesses or replays legacy rows.
pub(crate) async fn terminalize_principal_relay(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
    reason: &str,
) -> AppResult<bool> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let now = DateTime::now();
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            doc! {
                "_id": escalation_id,
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "contact_wxid": &entry.contact_wxid,
                "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                "$or": [
                    { "relay_state": { "$in": [
                        PRINCIPAL_RELAY_STATE_PENDING,
                        PRINCIPAL_RELAY_STATE_ENQUEUED,
                    ] } },
                    { "relay_state": { "$exists": false } },
                ],
            },
            doc! { "$set": {
                "relay_state": PRINCIPAL_RELAY_STATE_TERMINAL,
                "relay_terminal_at": now,
                "relay_terminal_reason": reason,
                "updated_at": now,
            } },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
    let terminal = if let Some(updated) = updated {
        updated
    } else {
        state
            .db
            .agent_principal_escalations()
            .find_one(
                doc! {
                    "_id": escalation_id,
                    "workspace_id": &entry.workspace_id,
                    "account_id": &entry.account_id,
                    "contact_wxid": &entry.contact_wxid,
                    "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                    "relay_state": PRINCIPAL_RELAY_STATE_TERMINAL,
                },
                None,
            )
            .await?
            .ok_or_else(|| AppError::Conflict("principal_relay_terminal_state_changed".into()))?
    };
    remove_awaiting_principal_owner(state, &terminal).await?;
    Ok(terminal.relay_terminal_reason.as_deref() == Some(reason))
}

/// Resolve the escalation explicitly bound to a relay task and terminalize it.
/// The deterministic new-protocol task id equals the escalation id; the short
/// code fallback is retained for already-running legacy tasks only.
pub(crate) async fn terminalize_principal_relay_for_task(
    state: &AppState,
    task: &AgentTask,
    reason: &str,
) -> AppResult<bool> {
    let mut identity = doc! {
        "workspace_id": &task.workspace_id,
        "account_id": &task.account_id,
        "contact_wxid": &task.contact_wxid,
        "short_code": task.content.trim(),
        "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
    };
    if let Some(task_id) = task.id {
        identity.insert(
            "$or",
            vec![
                doc! { "_id": task_id },
                doc! { "relay_task_id": task_id },
                doc! {
                    "relay_task_id": { "$exists": false },
                    "relay_state": { "$exists": false },
                },
            ],
        );
    }
    let entry = state
        .db
        .agent_principal_escalations()
        .find_one(identity, None)
        .await?
        .ok_or_else(|| AppError::Conflict("principal_relay_escalation_not_found".into()))?;
    terminalize_principal_relay(state, &entry, reason).await
}

/// Reconcile the coarse contact awaiting marker after a terminal card failure.
/// Cleanup acknowledgement is written last, so an interrupted pass is retried.
async fn complete_failed_delivery_cleanup(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<bool> {
    remove_awaiting_principal_owner(state, entry).await?;
    let acknowledged = state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "_id": entry.id,
                "status": PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED,
                "protocol.failure_cleanup_completed_at": { "$exists": false },
            },
            doc! { "$set": {
                "protocol.failure_cleanup_completed_at": DateTime::now(),
            } },
            None,
        )
        .await?;
    Ok(acknowledged.modified_count == 1)
}

/// 查某 workspace 下某领导 wxid 当前所有 pending 台账（按创建时间升序）。
pub(crate) async fn list_pending_for_principal(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    principal_wxid: &str,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.principal_account_id": account_id,
                "protocol.delivery_state": { "$in": [
                    PRINCIPAL_CARD_DELIVERY_SENT,
                    PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                ] },
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// 把一条 pending 台账标 resolved，写入真人裁决 + 授权过期时间。
pub(crate) async fn resolve_escalation(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
    decision: &PrincipalDecision,
    authorization_expires_at: Option<DateTime>,
    resolved_via: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let now = DateTime::now();
    let decision_bson = mongodb::bson::to_bson(decision)?;
    let mut set = doc! {
        "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
        "decision": decision_bson,
        "updated_at": now,
        "resolved_at": now,
        "resolved_via": resolved_via,
        "relay_state": PRINCIPAL_RELAY_STATE_PENDING,
        "relay_task_id": escalation_id,
    };
    if let Some(exp) = authorization_expires_at {
        set.insert("authorization_expires_at", exp);
    }
    let mut filter = doc! {
        "_id": escalation_id,
        "workspace_id": &entry.workspace_id,
        "short_code": &entry.short_code,
        "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
        "principal_wxid": &entry.principal_wxid,
    };
    if let Some(protocol) = entry.protocol.as_ref() {
        filter.insert("protocol.delivery_generation", protocol.delivery_generation);
        if resolved_via == "wechat" {
            filter.insert(
                "protocol.delivery_state",
                doc! { "$in": [
                    PRINCIPAL_CARD_DELIVERY_SENT,
                    PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                ] },
            );
        }
    }
    // The resolved ledger row and the authority observation are one authority-bearing state
    // transition. If either write fails, neither is committed; the relay reconciler can safely
    // retry the still-pending escalation instead of exposing a decision without provenance.
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let transaction_result: AppResult<Option<AgentPrincipalEscalation>> = async {
        let updated = state
            .db
            .agent_principal_escalations()
            .find_one_and_update_with_session(
                filter,
                doc! { "$set": set },
                mongodb::options::FindOneAndUpdateOptions::builder()
                    .return_document(mongodb::options::ReturnDocument::After)
                    .build(),
                &mut session,
            )
            .await?;
        if let Some(resolved) = updated.as_ref() {
            crate::agent::authority::record_authority_observation_with_session(
                &state.db,
                &mut session,
                AuthorityObservation {
                    id: Some(ObjectId::new()),
                    workspace_id: resolved.workspace_id.clone(),
                    account_id: resolved.account_id.clone(),
                    contact_wxid: resolved.contact_wxid.clone(),
                    source_type: "principal_decision".to_string(),
                    source_id: escalation_id.to_hex(),
                    subject: "business".to_string(),
                    content: format!(
                        "verdict={}; substance={}; constraints={}",
                        decision.verdict,
                        decision.substance,
                        if decision.constraints.is_empty() {
                            "none".to_string()
                        } else {
                            decision.constraints.join("; ")
                        }
                    ),
                    authority_boundary: "Authorizes only the exact principal decision substance and listed constraints for this escalation; it does not become general knowledge or authorize unrelated prices, schedules, outcomes, or services.".to_string(),
                    valid_from: Some(now),
                    valid_until: authorization_expires_at,
                    status: "active".to_string(),
                    superseded_by: None,
                    source_run_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await?;
        }
        Ok(updated)
    }
    .await;
    let updated = match transaction_result {
        Ok(updated) => updated,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    };
    crate::knowledge_wiki::chunk_revisions::commit_chunk_transaction(&mut session).await?;
    if let Some(resolved) = updated.as_ref() {
        // Task materialization is deliberately after the authority transaction. A crash here
        // leaves relay_state=pending, which the existing reconciler can materialize idempotently.
        materialize_relay_task(state, resolved).await?;
    }
    Ok(updated)
}

/// 真人决策可泛化时，发一条知识缺口提案（draft + needs_review）。
/// 复用现有知识子系统的 draft 契约——绝不自动验证（AI 永不自动验证红线）。
/// 写 workspace 共享域（account_id=None），与既有 chat 补库共享域一致，
/// 保证提案对整个 workspace 召回可见，而非账号私有。
pub(crate) async fn emit_knowledge_gap_proposal(
    state: &AppState,
    escalation: &AgentPrincipalEscalation,
    decision: &PrincipalDecision,
) -> AppResult<()> {
    // title 从 substance 提炼（不再用 escalation.reason——同 sediment，reason 是卡点原因/
    // reviewer 质检点评，当知识标题会扭曲召回）；draft 提案加「待审核：」前缀以区分未复核。
    let raw_title = derive_sediment_title(
        state,
        &escalation.workspace_id,
        &escalation.account_id,
        &escalation.contact_wxid,
        &decision.substance,
    )
    .await;
    let title = format!("待审核：{raw_title}");
    let body = format!(
        "源自客户「{}」请示 #{}。\n领导裁决：{}\n约束：{}",
        escalation.contact_wxid,
        escalation.short_code,
        decision.substance,
        if decision.constraints.is_empty() {
            "无".to_string()
        } else {
            decision.constraints.join("；")
        }
    );
    let chunk_id = ObjectId::new();
    let chunk = OperationKnowledgeChunk {
        id: Some(chunk_id),
        workspace_id: escalation.workspace_id.clone(),
        account_id: None, // workspace 共享域（与既有 chat 补库共享域一致）
        domain: crate::routes::knowledge::default_user_operations_domain(),
        status: "draft".to_string(),
        integrity_status: Some("needs_review".to_string()),
        title,
        body: Some(body),
        ..OperationKnowledgeChunk::default()
    };

    // Chunk 与 create revision 必须原子落地。PrincipalAuthorized 只描述来源，
    // 生命周期仍由 revision funnel 保持 draft + needs_review，绝不自动 verify。
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let result: AppResult<()> = async {
        state
            .db
            .operation_knowledge_chunks()
            .insert_one_with_session(chunk, None, &mut session)
            .await?;
        crate::knowledge_wiki::chunk_revisions::apply_chunk_revision_with_session(
            &state.db,
            &escalation.workspace_id,
            chunk_id,
            crate::knowledge_wiki::chunk_revisions::RevisionRequest {
                op: crate::knowledge_wiki::chunk_revisions::RevisionOp::Create,
                source:
                    crate::knowledge_wiki::chunk_revisions::ProvenanceSource::PrincipalAuthorized,
                patch: Document::new(),
                reason: Some(format!(
                    "principal escalation {} authorized sediment",
                    escalation.short_code
                )),
                actor: Some(escalation.principal_wxid.clone()),
            },
            &mut session,
        )
        .await?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => {
            crate::knowledge_wiki::chunk_revisions::commit_chunk_transaction(&mut session).await?;
            Ok(())
        }
        Err(error) => {
            let _ = session.abort_transaction().await;
            Err(error)
        }
    }
}

/// 从领导裁决 substance 提炼一个确定性的知识标题兜底：
/// 取首句（截到第一个句末标点 `。！？!?` 或换行之前），再按 chars 限长 40。
/// 空 substance → 固定安全标题（配合 sediment 空 substance 已提前跳过，实际仅有 substance 时被用到）。
/// LLM 提炼失败时回退到本函数，保证 title 永远可读、沉淀永不失败。
// 目前仅被单测消费；Task 3（derive_sediment_title 的 LLM 兜底）/ Task 4
// （sediment 落 title）接线后即成为生产调用点，暂 allow(dead_code) 保持 build 无警告。
#[allow(dead_code)]
pub(crate) fn derive_sediment_title_fallback(substance: &str) -> String {
    let trimmed = substance.trim();
    if trimmed.is_empty() {
        return "领导授权沉淀".to_string();
    }
    // 首句：截到第一个句末标点 / 换行之前。
    let first = trimmed
        .split(|c| matches!(c, '。' | '！' | '？' | '!' | '?' | '\n'))
        .next()
        .unwrap_or(trimmed)
        .trim();
    let first = if first.is_empty() { trimmed } else { first };
    // 按 chars 限长 40（多字节安全），超长截断加省略号。
    let mut chars: Vec<char> = first.chars().collect();
    if chars.len() > 40 {
        chars.truncate(40);
        let mut out: String = chars.into_iter().collect();
        out.push('…');
        out
    } else {
        chars.into_iter().collect()
    }
}

/// 从领导裁决 substance 提炼知识标题：优先 LLM 提炼，任何失败/空结果回退确定性兜底。
/// 绝不因提炼失败让沉淀失败——title 永远可读、非空。
// 目前尚无生产调用点（Task 4 接线 sediment 落 title 时才启用），暂 allow(dead_code)
// 保持 build 无警告。
#[allow(dead_code)]
pub(crate) async fn derive_sediment_title(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    substance: &str,
) -> String {
    let trimmed = substance.trim();
    if trimmed.is_empty() {
        return derive_sediment_title_fallback(substance);
    }
    let system =
        match crate::prompts::load_prompt(&state.db, workspace_id, "escalation.sediment.title")
            .await
        {
            Ok(s) => s,
            Err(_) => return derive_sediment_title_fallback(substance),
        };
    let user = format!("决策实质：{}", trimmed);
    let value = match crate::agent::generate_agent_json(
        state,
        workspace_id,
        Some(account_id),
        Some(contact_wxid),
        None,
        "escalation.sediment.title",
        &system,
        &user,
    )
    .await
    {
        Ok(v) => v,
        Err(_) => return derive_sediment_title_fallback(substance),
    };
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if title.is_empty() {
        return derive_sediment_title_fallback(substance);
    }
    // LLM 也可能给超长 title——用兜底同款 chars 限长逻辑收口（40 chars）。
    let capped: String = title.chars().take(40).collect();
    if title.chars().count() > 40 {
        format!("{capped}…")
    } else {
        capped
    }
}

/// 反查：在**入站消息自身所属 workspace** 内，from_wxid 是否是某 domain 的决策人。
/// KD-04：判断 from_wxid 是否为本 workspace 任一 current_version 域配置的决策人
/// （解析后的 decider_chain 成员，含旧 principal_decider 回落）。返回 Some(domain) 表示
/// 是决策人（domain 供调用方观测，webhooks 仅用 is_some 分流）；None 表示非决策人。
/// 从只查旧标量 principal_decider 改为复用 resolve_ask_human_policy——修复推荐配置
/// （只配 decider_chain）下领导回复不被识别的缺陷。
/// 🔒 关键：必须用入站消息自己的 workspace_id 约束查询——否则 A workspace 的领导 wxid
/// 若恰好也是 B workspace 某业务号的好友，B 收到他消息时会被误路由进 A 的请示流（跨域串扰）。
pub(crate) async fn lookup_principal_config(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    from_wxid: &str,
) -> AppResult<Option<String>> {
    use futures::TryStreamExt;
    if let Some(entry) = state
        .db
        .agent_principal_escalations()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": from_wxid,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.principal_account_id": account_id,
                "protocol.delivery_state": { "$in": [
                    PRINCIPAL_CARD_DELIVERY_SENT,
                    PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                ] },
            },
            None,
        )
        .await?
    {
        return Ok(entry.protocol.map(|protocol| protocol.domain));
    }
    let mut cursor = state
        .db
        .operation_domain_configs()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "current_version": true,
            },
            None,
        )
        .await?;
    while let Some(cfg) = cursor.try_next().await? {
        if crate::agent::escalation::policy::resolve_ask_human_policy(&cfg)
            .decider_chain
            .iter()
            .any(|decider| {
                decider.wxid == from_wxid
                    && decider
                        .account_id
                        .as_deref()
                        .is_none_or(|configured| configured == account_id)
            })
        {
            return Ok(Some(cfg.domain));
        }
    }
    Ok(None)
}

/// Idempotently materialize a durable relay intent as an immediately runnable task.
/// The task `_id` equals the escalation `_id`, so a crash or concurrent reconciler
/// can retry the upsert without creating a second relay.
pub(crate) async fn materialize_relay_task(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<()> {
    if entry.status != PRINCIPAL_ESCALATION_STATUS_RESOLVED
        || entry.relay_state.as_deref() != Some(PRINCIPAL_RELAY_STATE_PENDING)
    {
        return Err(AppError::Conflict(
            "principal_relay_intent_not_pending".to_string(),
        ));
    }
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let task_id = entry
        .relay_task_id
        .ok_or_else(|| AppError::External("principal relay intent missing task id".to_string()))?;
    if task_id != escalation_id {
        return Err(AppError::Conflict(
            "principal_relay_task_identity_mismatch".to_string(),
        ));
    }
    let now = DateTime::now();
    let task = AgentTask {
        id: Some(task_id),
        workspace_id: entry.workspace_id.clone(),
        account_id: entry.account_id.clone(),
        contact_wxid: entry.contact_wxid.clone(),
        kind: "principal_decision_relay".to_string(),
        run_at: now,
        expires_at: None,
        content: entry.short_code.clone(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: false,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    };
    let mut task_doc = mongodb::bson::to_document(&task)?;
    task_doc.remove("_id");
    state
        .db
        .tasks()
        .update_one(
            doc! {
                "_id": task_id,
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "contact_wxid": &entry.contact_wxid,
                "kind": "principal_decision_relay",
                "content": &entry.short_code,
            },
            doc! { "$setOnInsert": task_doc },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;

    let marked = state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "_id": escalation_id,
                "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                "relay_state": PRINCIPAL_RELAY_STATE_PENDING,
                "relay_task_id": task_id,
            },
            doc! { "$set": {
                "relay_state": PRINCIPAL_RELAY_STATE_ENQUEUED,
                "relay_enqueued_at": now,
                "updated_at": now,
            } },
            None,
        )
        .await?;
    if marked.modified_count == 0 {
        let current = state
            .db
            .agent_principal_escalations()
            .find_one(
                doc! {
                    "_id": escalation_id,
                    "relay_state": PRINCIPAL_RELAY_STATE_ENQUEUED,
                    "relay_task_id": task_id,
                },
                None,
            )
            .await?;
        if current.is_none() {
            return Err(AppError::Conflict(
                "principal_relay_intent_changed".to_string(),
            ));
        }
    }
    Ok(())
}

/// Recover new-protocol resolutions whose relay intent was persisted but whose
/// task materialization or acknowledgement was interrupted. Legacy resolved rows
/// without `relay_state` are deliberately ignored.
pub(crate) async fn reconcile_pending_relay_intents_once(state: &AppState) -> AppResult<u64> {
    use futures::TryStreamExt;

    let mut cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! {
                "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                "relay_state": PRINCIPAL_RELAY_STATE_PENDING,
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "resolved_at": 1, "_id": 1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut reconciled = 0_u64;
    while let Some(entry) = cursor.try_next().await? {
        match materialize_relay_task(state, &entry).await {
            Ok(()) => reconciled += 1,
            Err(error) => {
                tracing::warn!(
                    short_code = %entry.short_code,
                    error = %error,
                    "principal relay intent reconciliation failed"
                );
            }
        }
    }
    Ok(reconciled)
}

/// 按 workspace + status 列请示台账（admin 收件箱/SLA 看板用），created_at 升序。
pub(crate) async fn list_escalations_by_workspace(
    state: &AppState,
    workspace_id: &str,
    status: &str,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! { "workspace_id": workspace_id, "status": status },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// Atomically open the next delivery generation for a frozen decider.
/// The previous generation must already be terminal, so no still-runnable card
/// can race the reassignment. Delivery time is written only after Outbox confirms sent.
pub(crate) async fn reassign_escalation(
    state: &AppState,
    workspace_id: &str,
    short_code: &str,
    expected_principal_wxid: &str,
    expected_generation: i64,
    to_wxid: &str,
    to_account_id: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let now = DateTime::now();
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            doc! {
                "workspace_id": workspace_id,
                "short_code": short_code,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "principal_wxid": expected_principal_wxid,
                "protocol.delivery_generation": expected_generation,
                "protocol.delivery_state": { "$in": [
                    PRINCIPAL_CARD_DELIVERY_SENT,
                    PRINCIPAL_CARD_DELIVERY_FAILED_TERMINAL,
                    PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                ] },
            },
            doc! {
                "$set": {
                    "principal_wxid": to_wxid,
                    "protocol.principal_account_id": to_account_id,
                    "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE,
                    "updated_at": now,
                },
                "$inc": { "protocol.delivery_generation": 1i64 },
                "$unset": {
                    "protocol.delivery_outbox_id": "",
                    "last_pushed_at_ms": "",
                },
            },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
    Ok(updated)
}

/// All new-protocol rows whose current card was confirmed delivered and whose
/// frozen policy has a timeout. Legacy rows are intentionally not guessed.
pub(crate) async fn list_timeout_eligible_escalations(
    state: &AppState,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! {
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_SENT,
                "protocol.policy.timeoutHours": { "$type": "number" },
                "last_pushed_at_ms": { "$type": "number" },
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "last_pushed_at_ms": 1, "_id": 1 })
                .limit(500)
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// Pending new-protocol rows whose current card can no longer make progress by
/// itself: the delivery is terminally failed or unverifiable
/// (`failed_terminal` / `delivery_unknown`), or the row claims `sent` but has
/// no numeric push time (anomalous shape). None of these ever gains a trusted
/// push timestamp, so [`list_timeout_eligible_escalations`] can never surface
/// them; the scan converges them on the `created_at` time base instead.
/// `pending_enqueue` / `queued` rows are excluded — the delivery reconciler
/// still owns their progress.
pub(crate) async fn list_stranded_delivery_escalations(
    state: &AppState,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! {
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.policy.timeoutHours": { "$type": "number" },
                "$or": [
                    { "protocol.delivery_state": { "$in": [
                        PRINCIPAL_CARD_DELIVERY_FAILED_TERMINAL,
                        PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                    ] } },
                    {
                        "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_SENT,
                        "last_pushed_at_ms": { "$not": { "$type": "number" } },
                    },
                ],
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": 1, "_id": 1 })
                .limit(500)
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// 更新链尾安抚话术发送时刻（去重用）。仅 pending 可更新。
pub(crate) async fn touch_last_holding_reply_ms(
    state: &AppState,
    workspace_id: &str,
    short_code: &str,
    now_ms: i64,
) -> AppResult<()> {
    state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "workspace_id": workspace_id,
                "short_code": short_code,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            doc! { "$set": { "last_holding_reply_ms": now_ms } },
            None,
        )
        .await?;
    Ok(())
}

/// 统计某决策人当日（since_ms 起）已被推送的请示卡数（骚扰门 daily_push_cap 用）。
/// 以 last_pushed_at_ms（首推+改派刷新）为推送时刻（每条 pending = 一次推卡）。
pub(crate) async fn count_pushes_today(
    state: &AppState,
    workspace_id: &str,
    principal_wxid: &str,
    since_ms: i64,
) -> AppResult<u32> {
    let count = state
        .db
        .agent_principal_escalations()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                // KD-05：用真实最近推送时刻，而非 created_at（改派后 created_at 不刷新会漏计）。
                "last_pushed_at_ms": { "$gte": since_ms },
            },
            None,
        )
        .await?;
    Ok(count as u32)
}

/// 查某决策人最近一次被推卡的时刻（毫秒）——骚扰门 dedupe_window_hours 用。
/// 以 last_pushed_at_ms（首推+改派刷新）作推送时刻（与 count_pushes_today 同口径）。
/// 无任何台账 → None（首次推卡，dedupe 不拦）。
pub(crate) async fn latest_push_ms(
    state: &AppState,
    workspace_id: &str,
    principal_wxid: &str,
) -> AppResult<Option<i64>> {
    let latest = state
        .db
        .agent_principal_escalations()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                "last_pushed_at_ms": { "$type": "number" },
            },
            mongodb::options::FindOneOptions::builder()
                // KD-05：按真实最近推送时刻排序取最近一次推卡时刻（改派刷新后才准）。
                .sort(doc! { "last_pushed_at_ms": -1 })
                .build(),
        )
        .await?;
    Ok(latest.and_then(|e| e.last_pushed_at_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awaiting_owner_pipelines_use_wire_field_names() {
        let activate = format!(
            "{:?}",
            activate_awaiting_owner_pipeline("owner-a", DateTime::from_millis(1))
        );
        let remove = format!(
            "{:?}",
            remove_awaiting_owner_pipeline("owner-a", DateTime::from_millis(1))
        );
        for rendered in [&activate, &remove] {
            assert!(rendered.contains(crate::models::AWAITING_PRINCIPAL_DECISION_ATTR));
            assert!(rendered.contains(crate::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR));
            assert!(!rendered.contains("awaiting_key"));
            assert!(!rendered.contains("owners_key"));
        }
        assert!(activate.contains("$setUnion"));
        assert!(remove.contains("$filter"));
        assert!(remove.contains("$ne"));
    }

    #[test]
    fn fallback_takes_first_sentence() {
        // 句号截断：只取首句
        let t = derive_sediment_title_fallback("同意给他八折。本周内付款有效。");
        assert_eq!(t, "同意给他八折");
    }

    #[test]
    fn fallback_no_terminator_takes_whole_when_short() {
        let t = derive_sediment_title_fallback("同意八折");
        assert_eq!(t, "同意八折");
    }

    #[test]
    fn fallback_truncates_long_by_chars_not_bytes() {
        // 41 个中文字符（多字节）应截到 40 + 省略号，且不 panic（按 chars 截断）
        let s = "一".repeat(41);
        let t = derive_sediment_title_fallback(&s);
        assert_eq!(t.chars().count(), 41); // 40 + '…'
        assert!(t.ends_with('…'));
    }

    #[test]
    fn fallback_empty_returns_safe_title() {
        assert_eq!(derive_sediment_title_fallback("   "), "领导授权沉淀");
    }

    #[test]
    fn fallback_newline_is_sentence_terminator() {
        let t = derive_sediment_title_fallback("同意八折\n补充说明若干");
        assert_eq!(t, "同意八折");
    }
}

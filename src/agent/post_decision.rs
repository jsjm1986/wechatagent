//! Durable post-decision projection worker.
//!
//! Customer delivery must not wait for profile, memory, or analytical projections. The gateway
//! freezes the projection input on the decision review before creating Outbox rows. This worker
//! claims that snapshot with a lease and replays the idempotent projections after the text
//! decision has reached a send-authorized (or no-reply) state.

use std::time::Duration;

use mongodb::bson::{doc, from_bson, oid::ObjectId, to_bson, Bson, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::error::{AppError, AppResult};
use crate::models::{ConversationMessage, OperatingMemory, OperationDomainConfig};
use crate::routes::AppState;

use super::runtime::UserRuntimeParameters;
use super::types::AgentDecision;

const CLAIM_LEASE_MS: i64 = 120_000;
const POLL_MS: u64 = 500;
const MAX_BACKOFF_MS: i64 = 5 * 60_000;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_projection_snapshot(
    state: &AppState,
    review_id: ObjectId,
    decision: &AgentDecision,
    memory: &OperatingMemory,
    context_pack: &Document,
    domain_config: Option<&OperationDomainConfig>,
    active_profile: &crate::models::DomainProfile,
    active_products: &[crate::models::Product],
    ascending_window: &[ConversationMessage],
    run_id: &str,
) -> AppResult<()> {
    let payload = doc! {
        "decision": to_bson(decision)?,
        "memory": to_bson(memory)?,
        "context_pack": context_pack.clone(),
        "domain_config": domain_config.map(to_bson).transpose()?.unwrap_or(Bson::Null),
        "active_profile": to_bson(active_profile)?,
        "active_products": to_bson(active_products)?,
        // Raw webhook payloads are not needed by evidence resolution and can be large.
        "ascending_window": to_bson(&ascending_window.iter().cloned().map(|mut message| {
            message.raw = None;
            message
        }).collect::<Vec<_>>())?,
        "run_id": run_id,
    };
    let result = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id },
            doc! { "$set": {
                "post_decision_status": "prepared",
                "post_decision_payload": payload,
                "post_decision_attempts": 0i32,
            } },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::External(
            "decision review disappeared before post-decision snapshot persistence".to_string(),
        ));
    }
    Ok(())
}

/// Activate only after no-reply settlement or durable text-send authorization.
pub(crate) async fn activate_projection(state: &AppState, review_id: ObjectId) -> AppResult<()> {
    state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id, "post_decision_status": "prepared" },
            doc! { "$set": {
                "post_decision_status": "pending",
                "post_decision_next_retry_at": DateTime::now(),
            } },
            None,
        )
        .await?;
    Ok(())
}

pub(crate) async fn discard_projection(state: &AppState, review_id: ObjectId, reason: &str) {
    let _ = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id, "post_decision_status": "prepared" },
            doc! {
                "$set": { "post_decision_status": "discarded", "post_decision_error": reason },
                "$unset": { "post_decision_payload": "" },
            },
            None,
        )
        .await;
}

// `prepared` is runnable only under an already-authorized review status. The gateway persists
// the snapshot after task binding, while send decisions remain `outbox_enqueuing` until the task
// CAS commits. Thus this also recovers a crash between review commit and explicit activation.
fn runnable_filter(now: DateTime) -> Document {
    doc! {
        "status": { "$in": ["outbox_enqueued", "sent", "no_reply"] },
        "$or": [
            {
                "post_decision_status": { "$in": ["prepared", "pending", "retry"] },
                "$or": [
                    { "post_decision_next_retry_at": { "$exists": false } },
                    { "post_decision_next_retry_at": null },
                    { "post_decision_next_retry_at": { "$lte": now } },
                ],
            },
            {
                "post_decision_status": "processing",
                "$or": [
                    { "post_decision_locked_until": { "$exists": false } },
                    { "post_decision_locked_until": null },
                    { "post_decision_locked_until": { "$lte": now } },
                ],
            },
        ],
    }
}

async fn claim_one(state: &AppState) -> AppResult<Option<Document>> {
    let now = DateTime::now();
    // Recover a crash after the durable Review/task commit but before gateway activation.
    state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_many(
            doc! {
                "post_decision_status": "prepared",
                "status": { "$in": ["outbox_enqueued", "sent", "no_reply"] },
            },
            doc! { "$set": {
                "post_decision_status": "pending",
                "post_decision_next_retry_at": now,
            } },
            None,
        )
        .await?;
    let token = uuid::Uuid::new_v4().to_string();
    let locked_until = DateTime::from_millis(now.timestamp_millis() + CLAIM_LEASE_MS);
    Ok(state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .find_one_and_update(
            runnable_filter(now),
            doc! {
                "$set": {
                    "post_decision_status": "processing",
                    "post_decision_claim_token": &token,
                    "post_decision_locked_until": locked_until,
                },
                "$inc": { "post_decision_attempts": 1i32 },
            },
            FindOneAndUpdateOptions::builder()
                .sort(doc! { "post_decision_next_retry_at": 1, "created_at": 1, "_id": 1 })
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?)
}

fn payload_error(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::External(format!("post-decision {context}: {error}"))
}

fn decode<T: serde::de::DeserializeOwned>(payload: &Document, key: &str) -> AppResult<T> {
    let value = payload
        .get(key)
        .cloned()
        .ok_or_else(|| AppError::External(format!("post-decision payload missing {key}")))?;
    from_bson(value).map_err(|error| payload_error(&format!("decode {key}"), error))
}

async fn process_claimed(state: &AppState, review: Document) -> AppResult<()> {
    let review_id = review
        .get_object_id("_id")
        .map_err(|error| payload_error("review _id", error))?;
    let token = review
        .get_str("post_decision_claim_token")
        .map_err(|error| payload_error("claim token", error))?
        .to_string();
    let payload = review
        .get_document("post_decision_payload")
        .map_err(|error| payload_error("payload", error))?
        .clone();
    let run_id = payload
        .get_str("run_id")
        .map_err(|error| payload_error("run_id", error))?
        .to_string();
    let contact_wxid = review
        .get_str("contact_wxid")
        .map_err(|error| payload_error("contact_wxid", error))?;
    let workspace_id = review
        .get_str("workspace_id")
        .map_err(|error| payload_error("workspace_id", error))?;
    let account_id = review
        .get_str("account_id")
        .map_err(|error| payload_error("account_id", error))?;
    let contact = state
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
        .await?;
    let Some(contact) = contact else {
        state.db.decision_reviews().clone_with_type::<Document>().update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": &token },
            doc! { "$set": { "post_decision_status": "discarded", "post_decision_error": "contact_not_found" }, "$unset": { "post_decision_payload": "", "post_decision_claim_token": "", "post_decision_locked_until": "" } },
            None,
        ).await?;
        return Ok(());
    };

    let decision: AgentDecision = decode(&payload, "decision")?;
    let memory: OperatingMemory = decode(&payload, "memory")?;
    let context_pack = payload
        .get_document("context_pack")
        .map_err(|error| payload_error("context_pack", error))?
        .clone();
    let domain_config: Option<OperationDomainConfig> = match payload.get("domain_config") {
        Some(Bson::Null) | None => None,
        Some(value) => Some(
            from_bson(value.clone())
                .map_err(|error| payload_error("decode domain_config", error))?,
        ),
    };
    let active_profile: crate::models::DomainProfile = decode(&payload, "active_profile")?;
    let active_products: Vec<crate::models::Product> = decode(&payload, "active_products")?;
    let window: Vec<ConversationMessage> = decode(&payload, "ascending_window")?;
    let mut runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    runtime.apply_active_profile(&active_profile);

    let profile_done = review
        .get_bool("post_decision_profile_done")
        .unwrap_or(false);
    if !profile_done {
        super::gateway::apply_agent_updates(
            state,
            &contact,
            &decision,
            &runtime,
            domain_config.as_ref(),
            &active_profile,
            &active_products,
            &window,
            &run_id,
        )
        .await?;
        state.db.decision_reviews().clone_with_type::<Document>().update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": &token },
            doc! { "$set": { "post_decision_profile_done": true } },
            None,
        ).await?;
    }

    let memory_done = review
        .get_bool("post_decision_memory_done")
        .unwrap_or(false);
    if !memory_done {
        super::gateway::apply_operating_memory_update(
            state,
            &contact,
            &memory,
            &decision,
            &context_pack,
            false,
            &window,
            &run_id,
        )
        .await?;
        state.db.decision_reviews().clone_with_type::<Document>().update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": &token },
            doc! { "$set": { "post_decision_memory_done": true } },
            None,
        ).await?;
    }

    state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": &token },
            doc! {
                "$set": { "post_decision_status": "completed", "post_decision_completed_at": DateTime::now() },
                "$unset": { "post_decision_payload": "", "post_decision_claim_token": "", "post_decision_locked_until": "", "post_decision_next_retry_at": "", "post_decision_error": "" },
            },
            None,
        )
        .await?;
    Ok(())
}

async fn settle_failure(state: &AppState, review: &Document, error: &AppError) {
    let (Ok(review_id), Ok(token)) = (
        review.get_object_id("_id"),
        review.get_str("post_decision_claim_token"),
    ) else {
        return;
    };
    let attempts = review.get_i32("post_decision_attempts").unwrap_or(1).max(1);
    let shift = (attempts - 1).min(8) as u32;
    let backoff = (1_000i64.saturating_mul(1i64 << shift)).min(MAX_BACKOFF_MS);
    let _ = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": token },
            doc! {
                "$set": {
                    "post_decision_status": "retry",
                    "post_decision_next_retry_at": DateTime::from_millis(DateTime::now().timestamp_millis() + backoff),
                    "post_decision_error": error.to_string(),
                },
                "$unset": { "post_decision_claim_token": "", "post_decision_locked_until": "" },
            },
            None,
        )
        .await;
}

pub async fn run_worker(state: AppState) {
    loop {
        match claim_one(&state).await {
            Ok(Some(review)) => {
                if let Err(error) = process_claimed(&state, review.clone()).await {
                    tracing::warn!(%error, review_id = ?review.get_object_id("_id").ok(), "post-decision projection failed; scheduled retry");
                    settle_failure(&state, &review, &error).await;
                }
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(POLL_MS)).await,
            Err(error) => {
                tracing::error!(%error, "post-decision projection claim failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

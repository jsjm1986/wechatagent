//! Durable post-decision projection worker.
//!
//! Customer delivery never waits for analytical projection. The Gateway stores a compact,
//! bounded snapshot and continues even when snapshot preparation fails. Worker lanes claim jobs
//! with both a review fencing token and a cross-process contact lease, so different contacts may
//! run concurrently while one contact remains strictly single-flight.

use std::time::Duration;

use futures::{future::join_all, TryStreamExt};
use mongodb::bson::{doc, from_bson, oid::ObjectId, to_bson, Bson, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::models::{ConversationMessage, MessageDirection, OperationDomainConfig};
use crate::routes::AppState;

use super::decision::{
    load_operation_state_policy_for_contact, load_user_operation_domain_config_for_contact,
};
use super::guards::{
    action_policy_state_key, check_state_transition, enforce_state_action_policy,
    initial_operation_state_key,
};
use super::runtime::UserRuntimeParameters;
use super::types::{AgentDecision, DecisionReviewResult, DeferredProjectionDecision};

const MIN_CLAIM_LEASE_MS: i64 = 120_000;
const POLL_MS: u64 = 500;
const MAX_BACKOFF_MS: i64 = 5 * 60_000;
const MAX_SNAPSHOT_MESSAGES: usize = 20;
const MAX_SNAPSHOT_PRODUCTS: usize = 100;
const MAX_MESSAGE_CHARS: usize = 4_000;
const CONTACT_LEASE_COLLECTION: &str = "post_decision_contact_leases";
const SCRUB_POLL_SECONDS: u64 = 60 * 60;

fn snapshot_hash(payload: &Document) -> AppResult<String> {
    let bytes = mongodb::bson::to_vec(payload)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn scrub_deadline(state: &AppState, now: DateTime) -> DateTime {
    let retention_ms = state
        .config
        .post_decision_failed_snapshot_retention_days
        .saturating_mul(24 * 60 * 60 * 1_000);
    DateTime::from_millis(now.timestamp_millis().saturating_add(retention_ms))
}

fn ensure_snapshot_size(state: &AppState, payload: &Document) -> AppResult<usize> {
    let size = mongodb::bson::to_vec(payload)?.len();
    if size > state.config.post_decision_snapshot_max_bytes {
        return Err(AppError::External(format!(
            "post-decision snapshot too large: {size} > {}",
            state.config.post_decision_snapshot_max_bytes
        )));
    }
    Ok(size)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn compact_authorized_decision(decision: &AgentDecision) -> Document {
    doc! {
        "shouldReply": decision.should_reply,
        "replyText": truncate_chars(&decision.reply_text, MAX_MESSAGE_CHARS),
        "operationState": decision.operation_state.clone().map(Bson::from).unwrap_or(Bson::Null),
        "conversationMode": &decision.conversation_mode,
        "riskLevel": &decision.risk_level,
        "knowledgeNeed": &decision.knowledge_need,
        "usedKnowledgeIds": &decision.used_knowledge_ids,
        "safeClaimsUsed": &decision.safe_claims_used,
    }
}

fn compact_contact_snapshot(contact: &crate::models::Contact) -> AppResult<Document> {
    Ok(doc! {
        "wxid": &contact.wxid,
        "nickname": contact.nickname.clone().map(Bson::from).unwrap_or(Bson::Null),
        "remark": contact.remark.clone().map(Bson::from).unwrap_or(Bson::Null),
        "humanProfileNote": contact.human_profile_note.as_deref().map(|v| Bson::from(truncate_chars(v, 2_000))).unwrap_or(Bson::Null),
        "agentProfile": to_bson(&contact.agent_profile)?,
        "memorySummary": contact.memory_summary.as_deref().map(|v| Bson::from(truncate_chars(v, 2_000))).unwrap_or(Bson::Null),
        "manualTags": to_bson(&contact.manual_tags)?,
        "confirmedTags": to_bson(&contact.confirmed_tags)?,
        "bayesianSignals": to_bson(&contact.bayesian_signals)?,
        "domainAttributes": to_bson(&contact.domain_attributes)?,
        "operationState": contact.operation_state.clone().map(Bson::from).unwrap_or(Bson::Null),
        "profileAttributes": contact.profile_attributes.clone(),
        "commitments": to_bson(&contact.commitments)?,
        "intentTrajectory": to_bson(&contact.intent_trajectory.iter().rev().take(10).cloned().collect::<Vec<_>>())?,
        "outcomeEvents": to_bson(&contact.outcome_events.iter().rev().take(20).cloned().collect::<Vec<_>>())?,
        "locale": contact.locale.clone().map(Bson::from).unwrap_or(Bson::Null),
    })
}

fn compact_memory_snapshot(memory: &crate::models::OperatingMemory) -> AppResult<Document> {
    Ok(doc! {
        "userUnderstanding": memory.user_understanding.clone(),
        "relationshipState": memory.relationship_state.clone(),
        "productFit": memory.product_fit.clone(),
        "nextAction": memory.next_action.clone(),
        "memoryCard": to_bson(&memory.memory_card)?,
        "memoryCardVersion": memory.memory_card_version,
    })
}

fn compact_messages(messages: &[ConversationMessage]) -> Vec<Document> {
    let start = messages.len().saturating_sub(MAX_SNAPSHOT_MESSAGES);
    messages[start..]
        .iter()
        .map(|message| {
            doc! {
                "id": message.id.map(Bson::ObjectId).unwrap_or(Bson::Null),
                "messageId": message.message_id.clone().map(Bson::from).unwrap_or(Bson::Null),
                "direction": match message.direction { MessageDirection::Inbound => "inbound", MessageDirection::Outbound => "outbound" },
                "content": truncate_chars(&message.content, MAX_MESSAGE_CHARS),
                "msgType": message.msg_type.clone().map(Bson::from).unwrap_or(Bson::Null),
                "createdAt": message.created_at,
            }
        })
        .collect()
}

fn compact_products(products: &[crate::models::Product]) -> AppResult<Vec<Document>> {
    products
        .iter()
        .take(MAX_SNAPSHOT_PRODUCTS)
        .map(|product| {
            Ok(doc! {
                "productId": &product.product_id,
                "name": &product.name,
                "price": product.price.map(Bson::Int64).unwrap_or(Bson::Null),
                "currency": product.currency.clone().map(Bson::from).unwrap_or(Bson::Null),
                "sku": product.sku.clone().map(Bson::from).unwrap_or(Bson::Null),
                "summary": product.summary.as_deref().map(|v| Bson::from(truncate_chars(v, 1_000))).unwrap_or(Bson::Null),
                "attributes": product.attributes.clone(),
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_projection_snapshot(
    state: &AppState,
    review_id: ObjectId,
    decision: &AgentDecision,
    memory: &crate::models::OperatingMemory,
    context_pack: &Document,
    domain_config: Option<&OperationDomainConfig>,
    active_profile: &crate::models::DomainProfile,
    active_products: &[crate::models::Product],
    ascending_window: &[ConversationMessage],
    contact: &crate::models::Contact,
    run_id: &str,
) -> AppResult<()> {
    let contact_id = contact
        .id
        .ok_or_else(|| AppError::External("post-decision contact missing _id".to_string()))?;
    let raw_contact = state
        .db
        .contacts()
        .clone_with_type::<Document>()
        .find_one(
            doc! { "_id": contact_id },
            mongodb::options::FindOneOptions::builder()
                .projection(doc! { "profile_revision": 1 })
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::External("post-decision contact disappeared".to_string()))?;
    let profile_revision = raw_contact
        .get_i64("profile_revision")
        .or_else(|_| raw_contact.get_i32("profile_revision").map(i64::from))
        .unwrap_or(0);
    let payload = doc! {
        "authorized_decision": compact_authorized_decision(decision),
        "memory_snapshot": compact_memory_snapshot(memory)?,
        "context_pack": context_pack.clone(),
        "domain_config": domain_config.map(to_bson).transpose()?.unwrap_or(Bson::Null),
        // Runtime validation and objective reconciliation need the exact profile/products used by
        // the send-authorized run. Contact and memory are deliberately compacted above.
        "active_profile": to_bson(active_profile)?,
        "active_products": to_bson(&active_products.iter().take(MAX_SNAPSHOT_PRODUCTS).cloned().collect::<Vec<_>>())?,
        "contact_snapshot": compact_contact_snapshot(contact)?,
        "product_snapshot": to_bson(&compact_products(active_products)?)?,
        "ascending_window": to_bson(&compact_messages(ascending_window))?,
        "locale": contact.locale.clone().map(Bson::from).unwrap_or(Bson::Null),
        "run_id": run_id,
        "baseline_profile_revision": profile_revision,
        "projection_review_id": review_id,
    };
    let payload_size = ensure_snapshot_size(state, &payload)?;
    let payload_hash = snapshot_hash(&payload)?;
    let result = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id },
            doc! { "$set": {
                "post_decision_status": "prepared",
                "post_decision_payload": payload,
                "post_decision_payload_bytes": payload_size.min(i32::MAX as usize) as i32,
                "post_decision_payload_sha256": payload_hash,
                "post_decision_attempts": 0i32,
                "post_decision_prepared_at": DateTime::now(),
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

pub(crate) async fn mark_preparation_failed(state: &AppState, review_id: ObjectId, reason: &str) {
    let now = DateTime::now();
    let scrub_at = scrub_deadline(state, now);
    let _ = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id },
            doc! { "$set": {
                "post_decision_status": "failed_terminal",
                "post_decision_error_kind": "snapshot_preparation",
                "post_decision_error": truncate_chars(reason, 2_000),
                "post_decision_terminal_at": now,
                "post_decision_scrub_at": scrub_at,
            } },
            None,
        )
        .await;
}

fn serialize_projection_input(input: &Document) -> AppResult<String> {
    serde_json::to_string(input).map_err(|error| payload_error("encode projection input", error))
}

/// Build a valid JSON input within the model-facing character budget. Low-priority sections are
/// removed first; recent conversation and the send-authorized decision are retained longest.
fn projection_user_payload(
    payload: &Document,
    max_chars: usize,
) -> AppResult<(String, Vec<String>)> {
    let mut input = doc! {
        "authorizedDecision": payload.get("authorized_decision").or_else(|| payload.get("decision")).cloned().unwrap_or(Bson::Null),
        "ascendingConversationWindow": payload.get("ascending_window").cloned().unwrap_or(Bson::Null),
        "contactSnapshot": payload.get("contact_snapshot").cloned().unwrap_or(Bson::Null),
        "operatingMemory": payload.get("memory_snapshot").or_else(|| payload.get("memory")).cloned().unwrap_or(Bson::Null),
        "activeProfile": payload.get("active_profile").cloned().unwrap_or(Bson::Null),
        "activeProducts": payload.get("product_snapshot").or_else(|| payload.get("active_products")).cloned().unwrap_or(Bson::Null),
        "domainConfig": payload.get("domain_config").cloned().unwrap_or(Bson::Null),
        "contextPack": payload.get("context_pack").cloned().unwrap_or(Bson::Null),
    };
    let mut truncated = Vec::new();
    let mut encoded = serialize_projection_input(&input)?;
    if encoded.chars().count() <= max_chars {
        return Ok((encoded, truncated));
    }

    // Additional context is lowest priority and can be reconstructed from later runs.
    for key in [
        "contextPack",
        "domainConfig",
        "activeProducts",
        "activeProfile",
        "operatingMemory",
    ] {
        input.insert(key, Bson::Null);
        truncated.push(key.to_string());
        encoded = serialize_projection_input(&input)?;
        if encoded.chars().count() <= max_chars {
            return Ok((encoded, truncated));
        }
    }

    // Preserve the most recent turns while reducing an unusually large conversation snapshot.
    loop {
        let removed = match input.get_mut("ascendingConversationWindow") {
            Some(Bson::Array(rows)) if rows.len() > 1 && encoded.chars().count() > max_chars => {
                let remove = (rows.len() / 2).max(1);
                rows.drain(0..remove);
                true
            }
            _ => false,
        };
        if !removed {
            break;
        }
        if !truncated
            .iter()
            .any(|value| value == "ascendingConversationWindow")
        {
            truncated.push("ascendingConversationWindow".to_string());
        }
        encoded = serialize_projection_input(&input)?;
    }
    if encoded.chars().count() <= max_chars {
        return Ok((encoded, truncated));
    }

    // Contact state is useful but never more authoritative than the frozen authorized decision.
    input.insert("contactSnapshot", Bson::Null);
    truncated.push("contactSnapshot".to_string());
    encoded = serialize_projection_input(&input)?;
    if encoded.chars().count() <= max_chars {
        return Ok((encoded, truncated));
    }

    Err(payload_error(
        "projection prompt too large",
        format!(
            "minimal input is {} chars, limit is {max_chars}",
            encoded.chars().count()
        ),
    ))
}

/// Generate the same deferred projection used by the durable worker without creating or
/// updating a review row. Simulation uses this entry point to carry analytical state between
/// turns while keeping every production collection untouched.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn generate_projection_read_only(
    state: &AppState,
    decision: &AgentDecision,
    memory: &crate::models::OperatingMemory,
    context_pack: &Document,
    domain_config: Option<&OperationDomainConfig>,
    active_profile: &crate::models::DomainProfile,
    active_products: &[crate::models::Product],
    ascending_window: &[ConversationMessage],
    contact: &crate::models::Contact,
    run_id: &str,
) -> AppResult<DeferredProjectionDecision> {
    let payload = doc! {
        "authorized_decision": compact_authorized_decision(decision),
        "memory_snapshot": compact_memory_snapshot(memory)?,
        "context_pack": context_pack.clone(),
        "domain_config": domain_config.map(to_bson).transpose()?.unwrap_or(Bson::Null),
        "active_profile": to_bson(active_profile)?,
        "active_products": to_bson(&active_products.iter().take(MAX_SNAPSHOT_PRODUCTS).cloned().collect::<Vec<_>>())?,
        "contact_snapshot": compact_contact_snapshot(contact)?,
        "product_snapshot": to_bson(&compact_products(active_products)?)?,
        "ascending_window": to_bson(&compact_messages(ascending_window))?,
        "locale": contact.locale.clone().map(Bson::from).unwrap_or(Bson::Null),
        "run_id": run_id,
    };
    ensure_snapshot_size(state, &payload)?;

    let locale = contact.locale.as_deref();
    let (system, task) = tokio::join!(
        crate::prompts::load_prompt_for_contact(
            &state.db,
            &contact.workspace_id,
            "user.projection.system",
            &contact.wxid,
            locale,
        ),
        crate::prompts::load_prompt_for_contact(
            &state.db,
            &contact.workspace_id,
            "user.projection.task",
            &contact.wxid,
            locale,
        ),
    );
    let (system, _) = system?;
    let (task, _) = task?;
    let guidance = super::entitlements::render_relationship_type_suggestion_guidance();
    let fixed_chars = system
        .chars()
        .count()
        .saturating_add(task.chars().count())
        .saturating_add(guidance.chars().count())
        .saturating_add(32);
    let input_budget = state
        .config
        .post_decision_prompt_max_chars
        .saturating_sub(fixed_chars);
    if input_budget < 512 {
        return Err(payload_error(
            "projection prompt too large",
            format!(
                "fixed prompts consume {fixed_chars} chars of {}",
                state.config.post_decision_prompt_max_chars
            ),
        ));
    }
    let (projection_input, _) = projection_user_payload(&payload, input_budget)?;
    let user = format!("{task}{guidance}\n\n冻结输入：\n{projection_input}");
    let child_run_id = format!("{run_id}:projection");
    let raw = super::generate_agent_json(
        state,
        &contact.workspace_id,
        Some(&contact.account_id),
        Some(&contact.wxid),
        Some(&child_run_id),
        "user.projection.task",
        &system,
        &user,
    )
    .await?;
    DeferredProjectionDecision::from_value(raw)
        .map_err(|error| payload_error("validate projection result", error))
}

async fn load_or_freeze_projection_prompts(
    state: &AppState,
    review_id: ObjectId,
    token: &str,
    payload: &Document,
    workspace_id: &str,
    contact_wxid: &str,
) -> AppResult<(String, Option<i32>, String, Option<i32>)> {
    if let (Ok(system), Ok(task)) = (
        payload.get_str("projection_system"),
        payload.get_str("projection_task"),
    ) {
        return Ok((
            system.to_string(),
            payload.get_i32("projection_system_version").ok(),
            task.to_string(),
            payload.get_i32("projection_task_version").ok(),
        ));
    }
    let locale = payload.get_str("locale").ok();
    let (system, task) = tokio::join!(
        crate::prompts::load_prompt_for_contact(
            &state.db,
            workspace_id,
            "user.projection.system",
            contact_wxid,
            locale,
        ),
        crate::prompts::load_prompt_for_contact(
            &state.db,
            workspace_id,
            "user.projection.task",
            contact_wxid,
            locale,
        ),
    );
    let (system, system_version) = system?;
    let (task, task_version) = task?;
    let mut bounded_payload = payload.clone();
    bounded_payload.insert("projection_system", &system);
    bounded_payload.insert(
        "projection_system_version",
        system_version.map(Bson::Int32).unwrap_or(Bson::Null),
    );
    bounded_payload.insert("projection_task", &task);
    bounded_payload.insert(
        "projection_task_version",
        task_version.map(Bson::Int32).unwrap_or(Bson::Null),
    );
    ensure_snapshot_size(state, &bounded_payload)?;
    let result = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! {
                "_id": review_id,
                "post_decision_status": "processing",
                "post_decision_claim_token": token,
            },
            doc! { "$set": {
                "post_decision_payload.projection_system": &system,
                "post_decision_payload.projection_system_version": system_version.map(Bson::Int32).unwrap_or(Bson::Null),
                "post_decision_payload.projection_task": &task,
                "post_decision_payload.projection_task_version": task_version.map(Bson::Int32).unwrap_or(Bson::Null),
                "post_decision_prompt_versions": {
                    "user.projection.system": system_version.map(Bson::Int32).unwrap_or(Bson::Null),
                    "user.projection.task": task_version.map(Bson::Int32).unwrap_or(Bson::Null),
                },
            } },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(claim_lost());
    }
    Ok((system, system_version, task, task_version))
}

async fn load_or_generate_projection(
    state: &AppState,
    review: &Document,
    review_id: ObjectId,
    token: &str,
    payload: &Document,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    run_id: &str,
) -> AppResult<DeferredProjectionDecision> {
    let raw = if let Some(value) = review.get("post_decision_projection_result") {
        from_bson::<serde_json::Value>(value.clone())
            .map_err(|error| payload_error("decode persisted projection result", error))?
    } else {
        let (system, _, task, _) = load_or_freeze_projection_prompts(
            state,
            review_id,
            token,
            payload,
            workspace_id,
            contact_wxid,
        )
        .await?;
        let guidance = super::entitlements::render_relationship_type_suggestion_guidance();
        let fixed_chars = system
            .chars()
            .count()
            .saturating_add(task.chars().count())
            .saturating_add(guidance.chars().count())
            .saturating_add(32);
        let input_budget = state
            .config
            .post_decision_prompt_max_chars
            .saturating_sub(fixed_chars);
        if input_budget < 512 {
            return Err(payload_error(
                "projection prompt too large",
                format!(
                    "fixed prompts consume {fixed_chars} chars of {}",
                    state.config.post_decision_prompt_max_chars
                ),
            ));
        }
        let (projection_input, truncated_sections) =
            projection_user_payload(payload, input_budget)?;
        let user = format!("{task}{guidance}\n\n冻结输入：\n{projection_input}");
        let child_run_id = format!("{run_id}:projection");
        let side_budget = std::sync::Arc::new(super::RunBudget::new(
            child_run_id.clone(),
            state.config.post_decision_token_budget,
            1,
            0,
        ));
        let generation = super::RUN_BUDGET.scope(
            side_budget,
            super::generate_agent_json(
                state,
                workspace_id,
                Some(account_id),
                Some(contact_wxid),
                Some(&child_run_id),
                "user.projection.task",
                &system,
                &user,
            ),
        );
        tokio::pin!(generation);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let value = loop {
            tokio::select! {
                result = &mut generation => break result?,
                _ = heartbeat.tick() => renew_claim_and_contact_lease(state, review_id, token).await?,
            }
        };
        let mut bounded_payload = payload.clone();
        bounded_payload.insert("projection_system", &system);
        bounded_payload.insert("projection_task", &task);
        bounded_payload.insert("projection_result", to_bson(&value)?);
        ensure_snapshot_size(state, &bounded_payload)?;
        let result = state
            .db
            .decision_reviews()
            .clone_with_type::<Document>()
            .update_one(
                doc! {
                    "_id": review_id,
                    "post_decision_status": "processing",
                    "post_decision_claim_token": token,
                    "post_decision_projection_result": { "$exists": false },
                },
                doc! { "$set": {
                    "post_decision_projection_result": to_bson(&value)?,
                    "post_decision_projection_generated_at": DateTime::now(),
                    "post_decision_prompt_chars": user.chars().count().min(i32::MAX as usize) as i32,
                    "post_decision_truncated_sections": truncated_sections,
                    "post_decision_safe_to_regenerate": true,
                } },
                None,
            )
            .await?;
        if result.matched_count != 1 {
            return Err(claim_lost());
        }
        value
    };
    let unknown = DeferredProjectionDecision::unknown_fields(&raw);
    if !unknown.is_empty() {
        let _ = state
            .db
            .decision_reviews()
            .clone_with_type::<Document>()
            .update_one(
                doc! { "_id": review_id, "post_decision_claim_token": token },
                doc! { "$set": { "post_decision_unknown_fields": unknown } },
                None,
            )
            .await;
    }
    DeferredProjectionDecision::from_value(raw)
        .map_err(|error| payload_error("validate projection result", error))
}

async fn guard_projection_taxonomy(
    state: &AppState,
    contact: &crate::models::Contact,
    domain_config: Option<&OperationDomainConfig>,
    active_profile: &crate::models::DomainProfile,
    decision: &mut AgentDecision,
    run_id: &str,
) -> AppResult<()> {
    guard_projection_taxonomy_inner(
        state,
        contact,
        domain_config,
        active_profile,
        decision,
        Some(run_id),
    )
    .await
}

/// Apply the projection taxonomy contract without emitting taxonomy candidates. Simulation uses
/// the normalized decision only as an in-memory preview, so candidate writes would violate its
/// no-side-effect boundary.
pub(crate) async fn normalize_projection_read_only(
    state: &AppState,
    contact: &crate::models::Contact,
    domain_config: Option<&OperationDomainConfig>,
    active_profile: &crate::models::DomainProfile,
    projection: DeferredProjectionDecision,
) -> AppResult<AgentDecision> {
    let mut decision = projection.into_agent_decision();
    guard_projection_taxonomy_inner(
        state,
        contact,
        domain_config,
        active_profile,
        &mut decision,
        None,
    )
    .await?;
    Ok(decision)
}

async fn guard_projection_taxonomy_inner(
    state: &AppState,
    contact: &crate::models::Contact,
    domain_config: Option<&OperationDomainConfig>,
    active_profile: &crate::models::DomainProfile,
    decision: &mut AgentDecision,
    persist_candidate_run_id: Option<&str>,
) -> AppResult<()> {
    let cache = super::taxonomy::global_taxonomy_cache(&state.db);
    cache.find_or_load(&state.db, &contact.workspace_id).await?;
    let dimension_kinds = super::domain_profile::decision_dimension_kinds(active_profile);
    let fsm_customer_stage_keys = super::guards::operation_states(domain_config)
        .into_iter()
        .filter_map(|state| state.get_str("key").ok().map(ToString::to_string))
        .collect::<Vec<_>>();
    let outcome = super::gateway::compute_taxonomy_guard_outcome(
        decision,
        &dimension_kinds,
        &fsm_customer_stage_keys,
        &contact.workspace_id,
        &contact.account_id,
        &cache,
    );
    let candidates = outcome
        .candidate_writes
        .iter()
        .map(|(kind, raw)| {
            (
                kind.clone(),
                raw.clone(),
                super::gateway::pick_dimension_display_name(
                    &decision.dimension_display_names,
                    kind,
                )
                .map(ToString::to_string),
            )
        })
        .collect::<Vec<_>>();
    let mut review = DecisionReviewResult::default();
    super::gateway::apply_taxonomy_guard_outcome(decision, &mut review, &outcome);
    if let Some(run_id) = persist_candidate_run_id {
        for (kind, raw, display_name) in candidates {
            if let Err(error) = super::taxonomy::upsert_candidate_once_per_run(
                &state.db,
                &contact.workspace_id,
                &contact.account_id,
                &kind,
                &raw,
                Some("post-decision projection"),
                50,
                display_name.as_deref(),
                run_id,
            )
            .await
            {
                tracing::warn!(%error, kind, raw, "projection taxonomy candidate upsert failed");
            }
        }
    }
    super::domain_signals::normalize_domain_signals(decision);
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

/// Apply the operational controls frozen by the atomic production commit.
///
/// The post-decision model is intentionally allowed to write analytical profile fields only.  A
/// state transition or cooldown is a customer-side side effect, so it is read from the immutable
/// review contract, revalidated against the current domain/policy versions, and then committed
/// with a contact-state CAS.  The review marker makes retries idempotent; the underlying writes
/// are also safe to repeat after a process crash between the write and marker update.
pub(crate) async fn apply_authorized_projection_controls(
    state: &AppState,
    review_id: ObjectId,
) -> AppResult<()> {
    let review = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": review_id }, None)
        .await?
        .ok_or_else(|| {
            AppError::Conflict(format!(
                "decision review {review_id} disappeared before control finalization"
            ))
        })?;
    let controls = match review.get_document("authorized_projection_controls") {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if !controls.get_bool("authorized").unwrap_or(false) {
        return Ok(());
    }
    if review
        .get_str("authorized_projection_controls_status")
        .ok()
        .is_some_and(|status| !status.trim().is_empty())
    {
        return Ok(());
    }

    let workspace_id = review
        .get_str("workspace_id")
        .map_err(|error| payload_error("authorized controls workspace_id", error))?;
    let account_id = review
        .get_str("account_id")
        .map_err(|error| payload_error("authorized controls account_id", error))?;
    let contact_wxid = review
        .get_str("contact_wxid")
        .map_err(|error| payload_error("authorized controls contact_wxid", error))?;
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
        .await?
        .ok_or_else(|| AppError::Conflict("contact_missing_before_authorized_controls".into()))?;

    let current_domain_config =
        load_user_operation_domain_config_for_contact(state, workspace_id, contact_wxid).await?;
    let expected_domain_version = controls.get_i32("domain_version").ok();
    let domain_version_matches = match (expected_domain_version, current_domain_config.as_ref()) {
        (Some(expected), Some(current)) => current.version == expected,
        (Some(_), None) | (None, Some(_)) => false,
        (None, None) => true,
    };
    if !domain_version_matches {
        return suppress_authorized_projection_controls(
            state,
            &review,
            "operation domain version changed before control finalization",
            doc! {
                "expected_domain_version": expected_domain_version.map(Bson::Int32).unwrap_or(Bson::Null),
                "current_domain_version": current_domain_config.as_ref().map(|config| Bson::Int32(config.version)).unwrap_or(Bson::Null),
            },
        )
        .await;
    }

    let expected_policy_state = control_string(controls, "policy_state");
    let expected_source_state = control_string(controls, "source_operation_state");
    let current_effective_state = action_policy_state_key(
        current_domain_config.as_ref(),
        contact.operation_state.as_deref(),
        None,
    )
    .unwrap_or_else(|| initial_operation_state_key(current_domain_config.as_ref()));
    if expected_source_state.as_deref() != Some(current_effective_state.as_str()) {
        return suppress_authorized_projection_controls(
            state,
            &review,
            "contact operation state changed before control finalization",
            doc! {
                "expected_source_state": expected_source_state.map(Bson::String).unwrap_or(Bson::Null),
                "current_source_state": current_effective_state,
                "target_policy_state": expected_policy_state.clone().map(Bson::String).unwrap_or(Bson::Null),
            },
        )
        .await;
    }

    let current_policy = if let Some(policy_state) = expected_policy_state.as_deref() {
        load_operation_state_policy_for_contact(state, workspace_id, policy_state, contact_wxid)
            .await?
    } else {
        None
    };
    let expected_policy_version = controls.get_i32("policy_version").ok();
    let policy_version_matches = match (expected_policy_version, current_policy.as_ref()) {
        (Some(expected), Some(current)) => {
            current.status == "active" && current.version == expected
        }
        (Some(_), None) | (None, Some(_)) => false,
        (None, None) => true,
    };
    if !policy_version_matches {
        return suppress_authorized_projection_controls(
            state,
            &review,
            "operation state policy changed before control finalization",
            doc! {
                "expected_policy_version": expected_policy_version.map(Bson::Int32).unwrap_or(Bson::Null),
                "current_policy_version": current_policy.as_ref().map(|policy| Bson::Int32(policy.version)).unwrap_or(Bson::Null),
                "policy_state": expected_policy_state.map(Bson::String).unwrap_or(Bson::Null),
            },
        )
        .await;
    }

    let actions = control_actions(controls);
    for action in &actions {
        if let Err(reason) = enforce_state_action_policy(current_policy.as_ref(), action) {
            return suppress_authorized_projection_controls(
                state,
                &review,
                &reason,
                doc! {
                    "action": action,
                    "actions": actions.clone(),
                    "policy_state": expected_policy_state.clone().map(Bson::String).unwrap_or(Bson::Null),
                },
            )
            .await;
        }
    }

    let operation_state = control_string(controls, "operation_state");
    let operation_state_source = control_string(controls, "operation_state_source");
    let mut result_status = "applied";
    let mut result_details = Document::new();
    // An invalid candidate is intentionally represented by a null executable target. Preserve
    // the rejected proposal as an audit event, but never feed it into the contact CAS below.
    if let Some((prior, attempted, reason)) = rejected_operation_state_candidate(
        &review,
        controls,
        current_domain_config.as_ref(),
        &contact,
    ) {
        result_status = "applied_with_state_rejection";
        result_details.insert("state_transition_reason", reason.clone());
        emit_authorized_controls_event(
            state,
            &review,
            "agent.operation_state_transition_rejected",
            "rejected",
            &format!("operation_state 拒绝迁移 {prior} → {attempted}：{reason}"),
            doc! {
                "prior_state": prior,
                "attempted_state": attempted,
                "reason": reason,
            },
        )
        .await;
    }
    // Keep the raw contact value (including an absent/null value) as the fencing token for every
    // side effect below. A state transition may advance this token; a lost CAS must fence the
    // dependent cooldown rather than letting it land on whatever state happens to be current.
    let mut expected_contact_operation_state = contact.operation_state.clone();
    let mut state_control_fenced = false;

    if let Some(next_state) = operation_state.as_deref() {
        if let Some(reason) = check_state_transition(
            current_domain_config.as_ref(),
            contact.operation_state.as_deref(),
            next_state,
        ) {
            result_status = "applied_with_state_rejection";
            result_details.insert("state_transition_reason", reason.clone());
            emit_authorized_controls_event(
                state,
                &review,
                "agent.operation_state_transition_rejected",
                "rejected",
                &format!(
                    "operation_state 拒绝迁移 {} → {}：{}",
                    contact.operation_state.as_deref().unwrap_or("<empty>"),
                    next_state,
                    reason
                ),
                doc! {
                    "prior_state": contact.operation_state.clone().unwrap_or_default(),
                    "attempted_state": next_state,
                    "reason": reason,
                },
            )
            .await;
        } else if contact.operation_state.as_deref() != Some(next_state) {
            let filter = scoped_control_contact_state_filter(
                &review,
                &contact,
                contact.operation_state.as_deref(),
            );
            let mut set_fields = doc! {
                "operation_state": next_state,
                "operation_state_updated_at": DateTime::now(),
                "updated_at": DateTime::now(),
            };
            if let Some(reason) = control_string(controls, "operation_state_reason") {
                set_fields.insert("operation_state_reason", reason);
            }
            if let Ok(confidence) = controls.get_i32("operation_state_confidence") {
                set_fields.insert("operation_state_confidence", confidence);
            }
            if operation_state_source.as_deref() == Some("customer_stage") {
                set_fields.insert("domain_attributes.customer_stage", next_state);
                set_fields.insert("domain_attributes_updated_at", DateTime::now());
            }
            let write = state
                .db
                .contacts()
                .update_one(filter, doc! { "$set": set_fields }, None)
                .await?;
            if write.matched_count != 1 {
                let current = state
                    .db
                    .contacts()
                    .find_one(scoped_control_contact_filter(&review, &contact), None)
                    .await?
                    .ok_or_else(|| {
                        AppError::Conflict("contact_missing_after_authorized_state_cas".into())
                    })?;
                if current.operation_state.as_deref() != Some(next_state) {
                    state_control_fenced = true;
                    result_status = "fenced_conflict";
                    result_details.insert("attempted_state", next_state);
                    emit_authorized_controls_event(
                        state,
                        &review,
                        "agent.authorized_projection_controls_fenced",
                        "fenced",
                        "authorized operation-state control lost its contact CAS",
                        doc! {
                            "attempted_state": next_state,
                            "current_state": current.operation_state.clone().map(Bson::String).unwrap_or(Bson::Null),
                        },
                    )
                    .await;
                } else {
                    // Another worker completed the same idempotent transition. Continue only
                    // with a new fencing token for the state that was actually observed.
                    expected_contact_operation_state = current.operation_state.clone();
                }
            } else {
                expected_contact_operation_state = Some(next_state.to_string());
                emit_authorized_controls_event(
                    state,
                    &review,
                    "agent.operation_state_transitioned",
                    "transitioned",
                    &format!("operation_state {} → {}", contact.operation_state.as_deref().unwrap_or("<empty>"), next_state),
                    doc! {
                        "prior_state": contact.operation_state.clone().unwrap_or_default(),
                        "next_state": next_state,
                        "reason": control_string(controls, "operation_state_reason").unwrap_or_default(),
                        "confidence": controls.get_i32("operation_state_confidence").unwrap_or(0),
                    },
                )
                .await;
            }
        }
    }

    let has_cooldown_action = actions.iter().any(|action| action == "cooldown");
    if has_cooldown_action && !state_control_fenced {
        if let Ok(cooldown_until) = controls.get_datetime("cooldown_until") {
            let result = state
                .db
                .contacts()
                .update_one(
                    scoped_control_contact_state_filter(
                        &review,
                        &contact,
                        expected_contact_operation_state.as_deref(),
                    ),
                    doc! {
                        "$max": { "cooldown_until": *cooldown_until },
                        "$set": { "updated_at": DateTime::now() },
                    },
                    None,
                )
                .await?;
            if result.matched_count != 1 {
                result_status = "fenced_conflict";
                result_details.insert("cooldown_update", "contact_cas_missed");
            }
        }
    } else if has_cooldown_action && state_control_fenced {
        result_details.insert("cooldown_update", "state_transition_fenced");
    }

    mark_authorized_projection_controls(state, review_id, result_status, result_details).await
}

fn control_string(document: &Document, key: &str) -> Option<String> {
    document
        .get_str(key)
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn control_actions(document: &Document) -> Vec<String> {
    document
        .get_array("actions")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Bson::as_str)
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty())
        .collect()
}

/// Recover an invalid model-proposed operation state for audit purposes without turning it into
/// an executable projection control.  The production commit deliberately stores a null target
/// when the proposal is unknown or not reachable from the contact's source state; the raw review
/// field remains useful as immutable evidence of what was rejected.  This helper is only called
/// after the domain and policy/version fences have passed, so the reason is evaluated against the
/// same state machine that authorized the rest of the decision.
fn rejected_operation_state_candidate_for_source(
    review: &Document,
    controls: &Document,
    domain_config: Option<&OperationDomainConfig>,
    stored_state: Option<&str>,
) -> Option<(String, String, String)> {
    if control_string(controls, "operation_state").is_some() {
        return None;
    }
    let attempted = control_string(review, "operation_state")?;
    let prior = action_policy_state_key(domain_config, stored_state, None)
        .unwrap_or_else(|| initial_operation_state_key(domain_config));
    if attempted == prior {
        return None;
    }
    let reason = check_state_transition(domain_config, stored_state, &attempted)?;
    Some((prior, attempted, reason))
}

fn rejected_operation_state_candidate(
    review: &Document,
    controls: &Document,
    domain_config: Option<&OperationDomainConfig>,
    contact: &crate::models::Contact,
) -> Option<(String, String, String)> {
    rejected_operation_state_candidate_for_source(
        review,
        controls,
        domain_config,
        contact.operation_state.as_deref(),
    )
}

fn scoped_control_contact_filter(review: &Document, contact: &crate::models::Contact) -> Document {
    let mut filter = doc! {
        "workspace_id": review.get_str("workspace_id").unwrap_or(&contact.workspace_id),
        "account_id": review.get_str("account_id").unwrap_or(&contact.account_id),
        "wxid": review.get_str("contact_wxid").unwrap_or(&contact.wxid),
    };
    if let Some(id) = contact.id {
        filter.insert("_id", id);
    }
    filter
}

/// Add the raw operation-state fencing predicate used by all customer-side control writes.
/// `None` deliberately matches both legacy documents with a missing field and documents that
/// explicitly store BSON null, preserving the same compatibility semantics as the old CAS.
fn scoped_control_contact_state_filter(
    review: &Document,
    contact: &crate::models::Contact,
    expected_state: Option<&str>,
) -> Document {
    let mut filter = scoped_control_contact_filter(review, contact);
    match expected_state {
        Some(state) => {
            filter.insert("operation_state", state);
        }
        None => {
            filter.insert(
                "$or",
                vec![
                    doc! { "operation_state": Bson::Null },
                    doc! { "operation_state": { "$exists": false } },
                ],
            );
        }
    }
    filter
}

async fn suppress_authorized_projection_controls(
    state: &AppState,
    review: &Document,
    reason: &str,
    details: Document,
) -> AppResult<()> {
    let review_id = review
        .get_object_id("_id")
        .map_err(|error| payload_error("authorized controls review_id", error))?;
    mark_authorized_projection_controls(
        state,
        review_id,
        "suppressed_policy_changed",
        details.clone(),
    )
    .await?;
    emit_authorized_controls_event(
        state,
        review,
        "agent.authorized_projection_controls_suppressed",
        "suppressed",
        reason,
        details,
    )
    .await;
    Ok(())
}

async fn mark_authorized_projection_controls(
    state: &AppState,
    review_id: ObjectId,
    status: &str,
    details: Document,
) -> AppResult<()> {
    let mut set_fields = doc! {
        "authorized_projection_controls_status": status,
        "authorized_projection_controls_applied_at": DateTime::now(),
    };
    if !details.is_empty() {
        set_fields.insert("authorized_projection_controls_details", details);
    }
    state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! {
                "_id": review_id,
                "$or": [
                    { "authorized_projection_controls_status": { "$exists": false } },
                    { "authorized_projection_controls_status": Bson::Null },
                ],
            },
            doc! { "$set": set_fields },
            None,
        )
        .await?;
    Ok(())
}

async fn emit_authorized_controls_event(
    state: &AppState,
    review: &Document,
    kind: &str,
    status: &str,
    summary: &str,
    details: Document,
) {
    let Ok(review_id) = review.get_object_id("_id") else {
        return;
    };
    let workspace_id = review.get_str("workspace_id").unwrap_or_default();
    let account_id = review.get_str("account_id").unwrap_or_default();
    let contact_wxid = review.get_str("contact_wxid").ok();
    let dedupe_key = format!(
        "authorized_projection_controls:{}:{kind}",
        review_id.to_hex()
    );
    if let Err(error) = super::gateway::write_event_for_account_with_dedupe(
        state,
        workspace_id,
        account_id,
        contact_wxid,
        kind,
        status,
        summary,
        Some(details),
        Some(dedupe_key),
    )
    .await
    {
        tracing::warn!(%error, %review_id, "authorized projection controls audit event failed");
    }
}

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

fn contact_lease_id(workspace_id: &str, account_id: &str, contact_wxid: &str) -> String {
    format!("{workspace_id}\u{1f}{account_id}\u{1f}{contact_wxid}")
}

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    let text = error.to_string();
    text.contains("E11000") || text.contains("duplicate key")
}

async fn acquire_contact_lease(
    state: &AppState,
    review: &Document,
    token: &str,
    locked_until: DateTime,
) -> AppResult<bool> {
    let workspace_id = review
        .get_str("workspace_id")
        .map_err(|e| payload_error("workspace_id", e))?;
    let account_id = review
        .get_str("account_id")
        .map_err(|e| payload_error("account_id", e))?;
    let contact_wxid = review
        .get_str("contact_wxid")
        .map_err(|e| payload_error("contact_wxid", e))?;
    let review_id = review
        .get_object_id("_id")
        .map_err(|e| payload_error("review _id", e))?;
    let now = DateTime::now();
    let lease_id = contact_lease_id(workspace_id, account_id, contact_wxid);
    let result = state
        .db
        .raw()
        .collection::<Document>(CONTACT_LEASE_COLLECTION)
        .find_one_and_update(
            doc! {
                "_id": &lease_id,
                "$or": [
                    { "claim_token": token },
                    { "locked_until": { "$exists": false } },
                    { "locked_until": null },
                    { "locked_until": { "$lte": now } },
                ],
            },
            doc! {
                "$set": {
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "contact_wxid": contact_wxid,
                    "review_id": review_id,
                    "claim_token": token,
                    "locked_until": locked_until,
                    "updated_at": now,
                },
                "$setOnInsert": { "created_at": now },
            },
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await;
    match result {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(error) if duplicate_key(&error) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn release_contact_lease(state: &AppState, review: &Document, token: &str) {
    let (Ok(workspace_id), Ok(account_id), Ok(contact_wxid)) = (
        review.get_str("workspace_id"),
        review.get_str("account_id"),
        review.get_str("contact_wxid"),
    ) else {
        return;
    };
    let lease_id = contact_lease_id(workspace_id, account_id, contact_wxid);
    let _ = state
        .db
        .raw()
        .collection::<Document>(CONTACT_LEASE_COLLECTION)
        .delete_one(doc! { "_id": lease_id, "claim_token": token }, None)
        .await;
}

async fn claim_one(state: &AppState) -> AppResult<Option<Document>> {
    const CANDIDATE_SCAN_LIMIT: i64 = 32;

    let now = DateTime::now();
    let mut cursor = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .find(
            runnable_filter(now),
            FindOptions::builder()
                .sort(doc! { "post_decision_next_retry_at": 1, "created_at": 1, "_id": 1 })
                .limit(CANDIDATE_SCAN_LIMIT)
                .build(),
        )
        .await?;
    let mut candidates = Vec::new();
    while let Some(candidate) = cursor.try_next().await? {
        candidates.push(candidate);
    }

    // Only the oldest candidate for one contact is considered in a scan. A hot contact with a
    // deep queue therefore cannot consume every lane or cause repeated review status rewrites.
    let mut seen_contacts = std::collections::HashSet::new();
    for candidate in candidates {
        let contact_key = match (
            candidate.get_str("workspace_id"),
            candidate.get_str("account_id"),
            candidate.get_str("contact_wxid"),
        ) {
            (Ok(workspace), Ok(account), Ok(contact)) => {
                contact_lease_id(workspace, account, contact)
            }
            _ => continue,
        };
        if !seen_contacts.insert(contact_key) {
            continue;
        }

        let review_id = candidate
            .get_object_id("_id")
            .map_err(|error| payload_error("review _id", error))?;
        let token = uuid::Uuid::new_v4().to_string();
        let locked_until = DateTime::from_millis(now.timestamp_millis() + claim_lease_ms(state));
        if !acquire_contact_lease(state, &candidate, &token, locked_until).await? {
            tracing::debug!(review_id = %review_id, "post-decision contact busy; candidate deferred without review mutation");
            continue;
        }

        let mut claim_filter = runnable_filter(now);
        claim_filter.insert("_id", review_id);
        let claimed = state
            .db
            .decision_reviews()
            .clone_with_type::<Document>()
            .find_one_and_update(
                claim_filter,
                doc! {
                    "$set": {
                        "post_decision_status": "processing",
                        "post_decision_claim_token": &token,
                        "post_decision_locked_until": locked_until,
                        "post_decision_last_claimed_at": now,
                    },
                    "$inc": { "post_decision_attempts": 1i32 },
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?;
        if let Some(review) = claimed {
            return Ok(Some(review));
        }
        // Another process won the review CAS after our candidate read. Release only our token;
        // the winner's contact lease (if any) is fenced by its distinct claim token.
        release_contact_lease(state, &candidate, &token).await;
    }
    Ok(None)
}

fn claim_lease_ms(state: &AppState) -> i64 {
    let attempts = state.config.llm_max_retries.max(1) as i64;
    let request_ms = (state.config.llm_timeout_seconds.max(1) as i64)
        .saturating_mul(attempts)
        .saturating_mul(1_000);
    let retry_sleep_ms = state
        .config
        .llm_retry_base_ms
        .saturating_mul(attempts.saturating_sub(1) as u64) as i64;
    request_ms
        .saturating_add(retry_sleep_ms)
        .saturating_add(60_000)
        .max(MIN_CLAIM_LEASE_MS)
}

async fn renew_claim_and_contact_lease(
    state: &AppState,
    review_id: ObjectId,
    token: &str,
) -> AppResult<()> {
    let locked_until =
        DateTime::from_millis(DateTime::now().timestamp_millis() + claim_lease_ms(state));
    let result = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": token },
            doc! { "$set": { "post_decision_locked_until": locked_until } },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(claim_lost());
    }
    let lease = state
        .db
        .raw()
        .collection::<Document>(CONTACT_LEASE_COLLECTION)
        .update_one(
            doc! { "review_id": review_id, "claim_token": token },
            doc! { "$set": { "locked_until": locked_until, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    if lease.matched_count != 1 {
        return Err(claim_lost());
    }
    Ok(())
}

fn payload_error(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::External(format!("post-decision {context}: {error}"))
}

fn claim_lost() -> AppError {
    AppError::Conflict("post_decision_projection_claim_lost".to_string())
}

fn decode<T: serde::de::DeserializeOwned>(payload: &Document, key: &str) -> AppResult<T> {
    let value = payload
        .get(key)
        .cloned()
        .ok_or_else(|| AppError::External(format!("post-decision payload missing {key}")))?;
    from_bson(value).map_err(|error| payload_error(&format!("decode {key}"), error))
}

async fn has_newer_applied_projection(state: &AppState, review: &Document) -> AppResult<bool> {
    let review_id = review
        .get_object_id("_id")
        .map_err(|e| payload_error("review _id", e))?;
    let created_at = review
        .get_datetime("created_at")
        .map_err(|e| payload_error("created_at", e))?;
    Ok(state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .find_one(
            doc! {
                "workspace_id": review.get_str("workspace_id").map_err(|e| payload_error("workspace_id", e))?,
                "account_id": review.get_str("account_id").map_err(|e| payload_error("account_id", e))?,
                "contact_wxid": review.get_str("contact_wxid").map_err(|e| payload_error("contact_wxid", e))?,
                "post_decision_profile_done": true,
                "$or": [
                    { "created_at": { "$gt": *created_at } },
                    { "created_at": *created_at, "_id": { "$gt": review_id } },
                ],
            },
            mongodb::options::FindOneOptions::builder().projection(doc! { "_id": 1 }).build(),
        )
        .await?
        .is_some())
}

async fn apply_append_only_projection(
    state: &AppState,
    contact: &crate::models::Contact,
    decision: &AgentDecision,
    active_profile: &crate::models::DomainProfile,
    active_products: &[crate::models::Product],
    window: &[ConversationMessage],
    run_id: &str,
) -> AppResult<()> {
    super::memory::write_memory_candidates(state, contact, decision, run_id).await?;
    super::memory::write_tag_observations(state, contact, decision, window, run_id).await?;
    let stage = decision
        .domain_signals
        .get_str("customer_stage")
        .ok()
        .or(decision.customer_stage.as_deref());
    if let Some(stage) = stage {
        let evidences =
            crate::agent::tag_evidence::resolve_evidence(window, &decision.stage_evidence_turns);
        if !evidences.is_empty() {
            super::memory::write_stage_observation(state, contact, stage, &evidences, run_id)
                .await?;
        }
    }
    // Preserve idempotent weak-signal ledgers while clearing every stateful profile field.
    let signal_only = AgentDecision {
        agent_generated_signals: decision.agent_generated_signals.clone(),
        ..AgentDecision::default()
    };
    let runtime = UserRuntimeParameters::from_config(None, state);
    super::gateway::apply_agent_updates(
        state,
        contact,
        &signal_only,
        &runtime,
        None,
        active_profile,
        active_products,
        window,
        run_id,
        None,
    )
    .await
    .map(|_| ())
}

async fn process_claimed(state: &AppState, review: Document) -> AppResult<()> {
    let review_id = review
        .get_object_id("_id")
        .map_err(|e| payload_error("review _id", e))?;
    let token = review
        .get_str("post_decision_claim_token")
        .map_err(|e| payload_error("claim token", e))?
        .to_string();
    let payload = review
        .get_document("post_decision_payload")
        .map_err(|e| payload_error("payload", e))?
        .clone();
    let run_id = payload
        .get_str("run_id")
        .map_err(|e| payload_error("run_id", e))?
        .to_string();
    let contact_wxid = review
        .get_str("contact_wxid")
        .map_err(|e| payload_error("contact_wxid", e))?;
    let workspace_id = review
        .get_str("workspace_id")
        .map_err(|e| payload_error("workspace_id", e))?;
    let account_id = review
        .get_str("account_id")
        .map_err(|e| payload_error("account_id", e))?;
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! { "workspace_id": workspace_id, "account_id": account_id, "wxid": contact_wxid },
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

    // A no-reply decision has no transport delivery callback.  The gateway makes a best-effort
    // attempt immediately after the atomic commit, and this durable worker is the retry path for
    // an outage or process crash at that boundary.  The control function is guarded by the review
    // marker and contact CAS, so replaying it cannot create an additional side effect.
    if review.get_str("status").ok() == Some("no_reply") {
        apply_authorized_projection_controls(state, review_id).await?;
    }

    let context_pack = payload
        .get_document("context_pack")
        .map_err(|e| payload_error("context_pack", e))?
        .clone();
    let domain_config: Option<OperationDomainConfig> = match payload.get("domain_config") {
        Some(Bson::Null) | None => None,
        Some(value) => {
            Some(from_bson(value.clone()).map_err(|e| payload_error("decode domain_config", e))?)
        }
    };
    let active_profile: crate::models::DomainProfile = decode(&payload, "active_profile")?;
    let active_products: Vec<crate::models::Product> = decode(&payload, "active_products")?;
    let window_docs: Vec<Document> = decode(&payload, "ascending_window")?;
    let window = window_docs
        .into_iter()
        .map(|row| ConversationMessage {
            id: row
                .get_object_id("id")
                .or_else(|_| row.get_object_id("_id"))
                .ok(),
            workspace_id: workspace_id.to_string(),
            account_id: account_id.to_string(),
            contact_wxid: contact_wxid.to_string(),
            message_id: row
                .get_str("messageId")
                .or_else(|_| row.get_str("message_id"))
                .ok()
                .map(ToString::to_string),
            dedupe_key: None,
            direction: if row.get_str("direction").ok() == Some("outbound") {
                MessageDirection::Outbound
            } else {
                MessageDirection::Inbound
            },
            content: row.get_str("content").unwrap_or_default().to_string(),
            msg_type: row
                .get_str("msgType")
                .or_else(|_| row.get_str("msg_type"))
                .ok()
                .map(ToString::to_string),
            media_ref: None,
            raw: None,
            is_synthetic_relay: false,
            created_at: row
                .get_datetime("createdAt")
                .or_else(|_| row.get_datetime("created_at"))
                .copied()
                .unwrap_or_else(|_| DateTime::now()),
        })
        .collect::<Vec<_>>();
    let mut runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    runtime.apply_active_profile(&active_profile);

    let projection = load_or_generate_projection(
        state,
        &review,
        review_id,
        &token,
        &payload,
        workspace_id,
        account_id,
        contact_wxid,
        &run_id,
    )
    .await?;
    renew_claim_and_contact_lease(state, review_id, &token).await?;
    // From this point onward taxonomy/profile/memory effects may become durable. Regeneration
    // with a different model result would mix generations, so close the recovery gate first.
    let regeneration_gate = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": &token },
            doc! { "$set": { "post_decision_safe_to_regenerate": false } },
            None,
        )
        .await?;
    if regeneration_gate.matched_count != 1 {
        return Err(claim_lost());
    }
    let mut decision = projection.into_agent_decision();
    guard_projection_taxonomy(
        state,
        &contact,
        domain_config.as_ref(),
        &active_profile,
        &mut decision,
        &run_id,
    )
    .await?;
    renew_claim_and_contact_lease(state, review_id, &token).await?;

    let mut stale = has_newer_applied_projection(state, &review).await?;
    let profile_done = review
        .get_bool("post_decision_profile_done")
        .unwrap_or(false);
    if !profile_done {
        let mut conflict_kind: Option<&str> = None;
        if stale {
            apply_append_only_projection(
                state,
                &contact,
                &decision,
                &active_profile,
                &active_products,
                &window,
                &run_id,
            )
            .await?;
            conflict_kind = Some("newer_projection");
        } else {
            let baseline_profile_revision = payload
                .get_i64("baseline_profile_revision")
                .or_else(|_| payload.get_i32("baseline_profile_revision").map(i64::from))
                .unwrap_or(0);
            let outcome = super::gateway::apply_agent_updates(
                state,
                &contact,
                &decision,
                &runtime,
                domain_config.as_ref(),
                &active_profile,
                &active_products,
                &window,
                &run_id,
                Some(super::gateway::ProjectionWriteGuard {
                    baseline_profile_revision,
                    review_id,
                }),
            )
            .await?;
            match outcome {
                super::gateway::AgentUpdateOutcome::Applied => {}
                super::gateway::AgentUpdateOutcome::AlreadyApplied => {
                    conflict_kind = Some("same_review_replay");
                }
                super::gateway::AgentUpdateOutcome::FencedConflict => {
                    stale = true;
                    conflict_kind = Some("contact_revision_or_sequence");
                    apply_append_only_projection(
                        state,
                        &contact,
                        &decision,
                        &active_profile,
                        &active_products,
                        &window,
                        &run_id,
                    )
                    .await?;
                }
            }
        }
        let marked = state.db.decision_reviews().clone_with_type::<Document>().update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": &token },
            doc! { "$set": {
                "post_decision_profile_done": true,
                "post_decision_profile_skipped_stale": stale,
                "post_decision_profile_conflict_kind": conflict_kind.map(Bson::from).unwrap_or(Bson::Null),
            } },
            None,
        ).await?;
        if marked.matched_count != 1 {
            return Err(claim_lost());
        }
    }

    let memory_done = review
        .get_bool("post_decision_memory_done")
        .unwrap_or(false);
    if !memory_done {
        renew_claim_and_contact_lease(state, review_id, &token).await?;
        if !stale {
            let memory = super::memory::load_or_create_operating_memory(state, &contact).await?;
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
        }
        let marked = state.db.decision_reviews().clone_with_type::<Document>().update_one(
            doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": &token },
            doc! { "$set": { "post_decision_memory_done": true, "post_decision_memory_skipped_stale": stale } },
            None,
        ).await?;
        if marked.matched_count != 1 {
            return Err(claim_lost());
        }
    }

    let completed = state.db.decision_reviews().clone_with_type::<Document>().update_one(
        doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": &token },
        doc! {
            "$set": {
                "post_decision_status": "completed",
                "post_decision_completed_at": DateTime::now(),
                "post_decision_error_kind": Bson::Null,
            },
            "$unset": { "post_decision_payload": "", "post_decision_projection_result": "", "post_decision_claim_token": "", "post_decision_locked_until": "", "post_decision_next_retry_at": "", "post_decision_error": "" },
        },
        None,
    ).await?;
    if completed.matched_count != 1 {
        return Err(claim_lost());
    }
    Ok(())
}

fn projection_error_kind(error: &AppError) -> &'static str {
    let text = error.to_string();
    if text.contains("claim_lost") {
        "claim_lost"
    } else if text.contains("validate projection result") {
        "invalid_projection"
    } else if text.contains("projection prompt too large") {
        "prompt_too_large"
    } else if text.contains("payload missing")
        || text.contains("decode ")
        || text.contains("snapshot too large")
    {
        "invalid_snapshot"
    } else if matches!(error, AppError::LlmUnavailable { .. }) {
        "llm_unavailable"
    } else if matches!(error, AppError::Db(_)) {
        "database"
    } else {
        "processing"
    }
}

fn permanent_projection_error(error: &AppError) -> bool {
    matches!(
        projection_error_kind(error),
        "invalid_projection" | "invalid_snapshot" | "prompt_too_large"
    )
}

async fn settle_failure(state: &AppState, review: &Document, error: &AppError) {
    let (Ok(review_id), Ok(token)) = (
        review.get_object_id("_id"),
        review.get_str("post_decision_claim_token"),
    ) else {
        return;
    };
    let attempts = review.get_i32("post_decision_attempts").unwrap_or(1).max(1);
    let terminal =
        permanent_projection_error(error) || attempts >= state.config.post_decision_max_attempts;
    let kind = projection_error_kind(error);
    let now = DateTime::now();
    let scrub_at = scrub_deadline(state, now);
    let update = if terminal {
        doc! {
            "$set": {
                "post_decision_status": "failed_terminal",
                "post_decision_error_kind": kind,
                "post_decision_error": truncate_chars(&error.to_string(), 2_000),
                "post_decision_terminal_at": now,
                "post_decision_scrub_at": scrub_at,
            },
            "$unset": { "post_decision_claim_token": "", "post_decision_locked_until": "" },
        }
    } else {
        let shift = (attempts - 1).min(8) as u32;
        let backoff = (1_000i64.saturating_mul(1i64 << shift)).min(MAX_BACKOFF_MS);
        doc! {
            "$set": {
                "post_decision_status": "retry",
                "post_decision_next_retry_at": DateTime::from_millis(now.timestamp_millis() + backoff),
                "post_decision_error_kind": kind,
                "post_decision_error": truncate_chars(&error.to_string(), 2_000),
            },
            "$unset": { "post_decision_claim_token": "", "post_decision_locked_until": "" },
        }
    };
    let _ = state.db.decision_reviews().clone_with_type::<Document>().update_one(
        doc! { "_id": review_id, "post_decision_status": "processing", "post_decision_claim_token": token },
        update,
        None,
    ).await;
}

async fn scrub_expired_terminal_snapshots(state: &AppState) -> AppResult<u64> {
    let now = DateTime::now();
    let result = state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_many(
            doc! {
                "post_decision_status": { "$in": ["failed_terminal", "discarded"] },
                "post_decision_scrub_at": { "$lte": now },
                "$or": [
                    { "post_decision_payload": { "$exists": true } },
                    { "post_decision_projection_result": { "$exists": true } },
                ],
            },
            doc! {
                "$set": { "post_decision_scrubbed_at": now },
                "$unset": {
                    "post_decision_payload": "",
                    "post_decision_projection_result": "",
                    "post_decision_safe_to_regenerate": "",
                },
            },
            None,
        )
        .await?;
    Ok(result.modified_count)
}

async fn run_snapshot_scrubber(state: AppState) {
    loop {
        match scrub_expired_terminal_snapshots(&state).await {
            Ok(count) if count > 0 => {
                tracing::info!(count, "scrubbed expired post-decision snapshots")
            }
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "post-decision snapshot scrub failed"),
        }
        tokio::time::sleep(Duration::from_secs(SCRUB_POLL_SECONDS)).await;
    }
}

async fn run_worker_lane(state: AppState, lane: usize) {
    loop {
        match claim_one(&state).await {
            Ok(Some(review)) => {
                let token = review
                    .get_str("post_decision_claim_token")
                    .unwrap_or_default()
                    .to_string();
                if let Err(error) = process_claimed(&state, review.clone()).await {
                    tracing::warn!(%error, lane, review_id = ?review.get_object_id("_id").ok(), "post-decision projection failed");
                    settle_failure(&state, &review, &error).await;
                }
                release_contact_lease(&state, &review, &token).await;
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(POLL_MS)).await,
            Err(error) => {
                tracing::error!(%error, lane, "post-decision projection claim failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

pub async fn run_worker(state: AppState) {
    let concurrency = state.config.post_decision_worker_concurrency.max(1);
    let lanes = join_all((0..concurrency).map(|lane| run_worker_lane(state.clone(), lane)));
    let scrubber = run_snapshot_scrubber(state);
    let _ = tokio::join!(lanes, scrubber);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_hash_is_stable_and_sensitive() {
        assert_eq!(
            snapshot_hash(&doc! { "a": 1 }).unwrap(),
            snapshot_hash(&doc! { "a": 1 }).unwrap()
        );
        assert_ne!(
            snapshot_hash(&doc! { "a": 1 }).unwrap(),
            snapshot_hash(&doc! { "a": 2 }).unwrap()
        );
    }

    #[test]
    fn projection_input_trims_low_priority_sections_first() {
        let payload = doc! {
            "authorized_decision": { "replyText": "ok" },
            "ascending_window": [{ "content": "recent" }],
            "contact_snapshot": { "nickname": "n" },
            "memory_snapshot": { "large": "x".repeat(1000) },
            "context_pack": { "large": "y".repeat(1000) },
        };
        let (encoded, truncated) = projection_user_payload(&payload, 500).unwrap();
        assert!(encoded.chars().count() <= 500);
        assert!(truncated.contains(&"contextPack".to_string()));
        assert!(encoded.contains("authorizedDecision"));
        assert!(encoded.contains("recent"));
    }

    #[test]
    fn compact_text_truncates_on_unicode_boundaries() {
        assert_eq!(truncate_chars("甲乙丙", 2), "甲乙");
    }

    #[test]
    fn compact_messages_preserves_evidence_object_id() {
        let id = ObjectId::new();
        let messages = vec![ConversationMessage {
            id: Some(id),
            workspace_id: "ws".to_string(),
            account_id: "account".to_string(),
            contact_wxid: "wxid".to_string(),
            message_id: Some("business-message-id".to_string()),
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: "明确推进".to_string(),
            msg_type: None,
            media_ref: None,
            raw: None,
            is_synthetic_relay: false,
            created_at: DateTime::now(),
        }];

        let compacted = compact_messages(&messages);
        assert_eq!(compacted[0].get_object_id("id").ok(), Some(id));
        assert_eq!(
            compacted[0].get_str("messageId").ok(),
            Some("business-message-id")
        );
    }

    #[test]
    fn contact_lease_identity_is_tenant_and_contact_scoped() {
        assert_ne!(
            contact_lease_id("w1", "a", "c"),
            contact_lease_id("w2", "a", "c")
        );
        assert_ne!(
            contact_lease_id("w", "a1", "c"),
            contact_lease_id("w", "a2", "c")
        );
    }

    #[test]
    fn runnable_filter_keeps_no_reply_on_the_durable_projection_lane() {
        let filter = runnable_filter(DateTime::now());
        let statuses = filter
            .get_document("status")
            .expect("status filter is present")
            .get_array("$in")
            .expect("status filter is an allow-list");
        assert!(statuses
            .iter()
            .any(|status| status.as_str() == Some("no_reply")));
    }

    #[test]
    fn permanent_error_classification_is_bounded() {
        let invalid = payload_error("validate projection result", "forbidden field");
        assert!(permanent_projection_error(&invalid));
        let transient = AppError::External("temporary processing failure".to_string());
        assert!(!permanent_projection_error(&transient));
    }

    #[test]
    fn rejected_state_evidence_is_audit_only_when_target_is_null() {
        let domain = crate::prompts::default_domain_configs("ws")
            .into_iter()
            .find(|config| config.domain == "user_operations")
            .expect("default user operation domain");
        let review = doc! { "operation_state": "customer_success" };
        let controls = doc! { "operation_state": Bson::Null };

        let rejected = rejected_operation_state_candidate_for_source(
            &review,
            &controls,
            Some(&domain),
            Some("new_contact"),
        )
        .expect("unreachable target should remain auditable");
        assert_eq!(rejected.0, "new_contact");
        assert_eq!(rejected.1, "customer_success");
        assert!(rejected.2.contains("state_transition_invalid"));

        let applied_controls = doc! { "operation_state": "need_discovery" };
        assert!(rejected_operation_state_candidate_for_source(
            &review,
            &applied_controls,
            Some(&domain),
            Some("new_contact"),
        )
        .is_none());
    }
}

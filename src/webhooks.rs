use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use dashmap::DashMap;

use axum::{body::Bytes, extract::State, http::HeaderMap, Json};
use hmac::{Hmac, Mac};
use mongodb::{
    bson::{doc, oid::ObjectId, to_document, DateTime, Document},
    error::{ErrorKind, WriteFailure},
    options::{FindOneAndUpdateOptions, FindOptions, UpdateOptions},
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    agent,
    error::{AppError, AppResult},
    models::{AgentStatus, AgentTask, Contact, ConversationMessage, MessageDirection},
    routes::AppState,
};

/// Durable, single-flight handoff from webhook ingestion to the Agent task
/// worker. The task keeps this stable key across terminal states so a later
/// inbound can revive the same row and fence any older owner.
pub const DURABLE_INBOUND_REPLY_KIND: &str = "inbound_reply";
const DURABLE_INBOUND_ACTIVE_KEY: &str = "inbound_reply";
const HANDOFF_PENDING: &str = "pending";
const HANDOFF_MATERIALIZED: &str = "materialized";
const HANDOFF_DEFERRED: &str = "deferred"; // legacy read compatibility only
const HANDOFF_IGNORED: &str = "ignored_not_managed";
const HANDOFF_QUARANTINED: &str = "quarantined";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableInboundTask {
    pub task_id: ObjectId,
    pub run_at_ms: i64,
}

pub(crate) fn durable_inbound_task_id(
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
) -> ObjectId {
    let mut hasher = Sha256::new();
    for value in [workspace_id, account_id, wxid, DURABLE_INBOUND_REPLY_KIND] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 12];
    bytes.copy_from_slice(&digest[..12]);
    ObjectId::from_bytes(bytes)
}

pub async fn mark_inbound_handoff(
    state: &AppState,
    message_id: ObjectId,
    status: &str,
) -> AppResult<()> {
    state
        .db
        .messages()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": message_id, "handoff_status": { "$in": [HANDOFF_PENDING, HANDOFF_DEFERRED] } },
            doc! { "$set": {
                "handoff_status": status,
                "handoff_updated_at": DateTime::now(),
            } },
            None,
        )
        .await?;
    Ok(())
}

/// Materialize or refresh the one durable inbound task for a tenant/contact.
/// Message order is `(created_at, _id)`: `created_at` is the primary arrival
/// fact, while `_id` is only a deterministic tie-breaker for equal timestamps.
/// ObjectId ordering alone is insufficient because different processes may
/// generate non-monotonic random tails within the same second.
pub async fn materialize_durable_inbound_task(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    debounce_window_ms: u64,
) -> AppResult<DurableInboundTask> {
    let run_at = DateTime::from_millis(
        inbound
            .created_at
            .timestamp_millis()
            .saturating_add(debounce_window_ms.min(i64::MAX as u64) as i64),
    );
    materialize_durable_inbound_task_at(state, contact, inbound, run_at, "debouncing").await
}

/// Materialize the contact's single passive-reply obligation at an explicit policy time.
/// Quiet hours and ordinary debounce both use this row; `kind` never changes, so a newer
/// inbound always fences an older generator/outbox regardless of the policy transition.
pub async fn materialize_durable_inbound_task_at(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    run_at: DateTime,
    schedule_reason: &str,
) -> AppResult<DurableInboundTask> {
    let message_id = inbound.id.ok_or_else(|| {
        AppError::External("durable inbound handoff requires persisted message _id".to_string())
    })?;
    let task_id =
        durable_inbound_task_id(&contact.workspace_id, &contact.account_id, &contact.wxid);
    let now = DateTime::now();
    let task = AgentTask {
        id: Some(task_id),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        kind: DURABLE_INBOUND_REPLY_KIND.to_string(),
        run_at,
        expires_at: None,
        content: message_id.to_hex(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: true,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: Some(schedule_reason.to_string()),
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    };
    let mut insert_doc = to_document(&task)?;
    insert_doc.insert("active_task_key", DURABLE_INBOUND_ACTIVE_KEY);
    insert_doc.insert("latest_inbound_id", message_id);
    insert_doc.insert("latest_inbound_created_at", inbound.created_at);
    // Stable lower bound for context expansion before the first successful delivery.
    insert_doc.insert("obligation_started_inbound_id", message_id);
    insert_doc.insert("obligation_started_inbound_created_at", inbound.created_at);

    let newer = doc! { "$or": [
        { "latest_inbound_created_at": { "$exists": false } },
        { "latest_inbound_created_at": null },
        { "latest_inbound_created_at": { "$lt": inbound.created_at } },
        { "latest_inbound_created_at": inbound.created_at, "$or": [
            { "latest_inbound_id": { "$exists": false } },
            { "latest_inbound_id": null },
            { "latest_inbound_id": { "$lt": message_id } },
        ]},
    ]};
    let tasks = state.db.tasks().clone_with_type::<Document>();
    match tasks.insert_one(insert_doc, None).await {
        Ok(_) => {}
        Err(error) if is_duplicate_key_error(&error) => {
            // A manual reply pauses the obligation. New inbound extends its upper watermark but
            // must not start an AI reply before that manual delivery settles.
            let mut manual_filter = doc! {
                "_id": task_id,
                "manual_reply_run_id": { "$exists": true },
            };
            manual_filter.extend(newer.clone());
            let manual = tasks
                .update_one(
                    manual_filter,
                    doc! { "$set": {
                        "latest_inbound_id": message_id,
                        "latest_inbound_created_at": inbound.created_at,
                        "run_at": run_at,
                        "content": message_id.to_hex(),
                        "updated_at": now,
                    }},
                    None,
                )
                .await?;
            if manual.matched_count == 0 {
                let mut refresh_filter = doc! {
                    "_id": task_id,
                    "manual_reply_run_id": { "$exists": false },
                };
                refresh_filter.extend(newer);
                tasks.update_one(
                    refresh_filter,
                    doc! {
                        "$set": {
                            "workspace_id": &contact.workspace_id, "account_id": &contact.account_id,
                            "contact_wxid": &contact.wxid, "kind": DURABLE_INBOUND_REPLY_KIND,
                            "active_task_key": DURABLE_INBOUND_ACTIVE_KEY,
                            "latest_inbound_id": message_id, "latest_inbound_created_at": inbound.created_at,
                            "run_at": run_at, "content": message_id.to_hex(), "status": "pending",
                            "gateway_status": schedule_reason, "review_required": true,
                            "attempt_count": 0, "max_attempts": 3, "claim_recovery_count": 0,
                            "updated_at": now,
                        },
                        "$unset": {
                            "expires_at": "", "source_decision_id": "", "next_retry_at": "",
                            "cancel_reason": "", "error": "", "claimed_at": "", "claim_token": "",
                            "outbox_decision_id": "", "prepared_commit_kind": "", "prepared_commit": "",
                            "manual_reply_run_id": "",
                        },
                    },
                    None,
                ).await?;
            }
        }
        Err(error) => return Err(error.into()),
    }

    mark_inbound_handoff(state, message_id, HANDOFF_MATERIALIZED).await?;
    let stored = tasks
        .find_one(doc! { "_id": task_id }, None)
        .await?
        .ok_or_else(|| AppError::External("durable inbound task disappeared".to_string()))?;
    Ok(DurableInboundTask {
        task_id,
        run_at_ms: stored
            .get_datetime("run_at")
            .copied()
            .unwrap_or(run_at)
            .timestamp_millis(),
    })
}

pub(crate) fn policy_run_at(
    runtime: &crate::agent::UserRuntimeParameters,
    contact: &Contact,
    state: &AppState,
) -> DateTime {
    if runtime.quiet_hours_enabled
        && agent::quiet_hours::is_quiet_now(
            runtime.quiet_hours_start,
            runtime.quiet_hours_end,
            runtime.quiet_hours_tz_offset_hours,
        )
    {
        agent::quiet_hours::next_wake_at(
            runtime.quiet_hours_end,
            runtime.quiet_hours_tz_offset_hours,
            &contact.wxid,
            state.config.wake_jitter_max_seconds,
        )
    } else {
        DateTime::now()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ManualReplyCoverage {
    pub task_id: ObjectId,
    pub inbound_id: ObjectId,
    pub inbound_created_at: DateTime,
}

/// Freeze the inbound watermark covered by a manual reply and pause the reusable passive-reply
/// obligation. The task transition fences any older AI owner before its Outbox is canceled.
pub(crate) async fn pause_reply_obligation_for_manual(
    state: &AppState,
    contact: &Contact,
    run_id: &str,
) -> AppResult<Option<ManualReplyCoverage>> {
    let latest = state
        .db
        .messages()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "direction": "inbound",
            },
            mongodb::options::FindOneOptions::builder()
                .sort(doc! { "created_at": -1, "_id": -1 })
                .build(),
        )
        .await?;
    let Some(latest) = latest else {
        return Ok(None);
    };
    let Some(inbound_id) = latest.id else {
        return Ok(None);
    };

    // Repair a historical/crash gap before pausing so every covered inbound has one stable row.
    let task_id =
        durable_inbound_task_id(&contact.workspace_id, &contact.account_id, &contact.wxid);
    if state
        .db
        .tasks()
        .count_documents(doc! { "_id": task_id }, None)
        .await?
        == 0
    {
        materialize_durable_inbound_task_at(
            state,
            contact,
            &latest,
            DateTime::now(),
            "manual_reply_preparing",
        )
        .await?;
    }

    let previous = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one_and_update(
            doc! {
                "_id": task_id,
                "kind": DURABLE_INBOUND_REPLY_KIND,
                "$or": [
                    { "manual_reply_run_id": { "$exists": false } },
                    { "manual_reply_run_id": run_id },
                ],
            },
            doc! {
                "$set": {
                    "status": "pending",
                    "gateway_status": "manual_reply_pending",
                    "manual_reply_run_id": run_id,
                    "manual_reply_started_at": DateTime::now(),
                    "manual_covers_through_inbound_id": inbound_id,
                    "manual_covers_through_inbound_created_at": latest.created_at,
                    "updated_at": DateTime::now(),
                },
                "$unset": {
                    "claim_token": "", "claimed_at": "", "outbox_decision_id": "",
                    "next_retry_at": "", "error": "",
                },
            },
            FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::Before)
                .build(),
        )
        .await?;
    let Some(previous) = previous else {
        return Err(AppError::Conflict(
            "another_manual_reply_is_pending".to_string(),
        ));
    };
    if let Ok(decision_id) = previous.get_object_id("outbox_decision_id") {
        if let Err(error) = agent::cancel_for_decision(
            state,
            &contact.workspace_id,
            decision_id,
            "superseded_by_manual_reply",
        )
        .await
        {
            if let Err(release_error) = settle_manual_reply_obligation(
                state,
                &contact.workspace_id,
                &contact.account_id,
                &contact.wxid,
                run_id,
                false,
            )
            .await
            {
                tracing::error!(%release_error, %run_id, "failed to release manual reply pause after old outbox cancellation failure");
            }
            return Err(error.into());
        }
    }
    Ok(Some(ManualReplyCoverage {
        task_id,
        inbound_id,
        inbound_created_at: latest.created_at,
    }))
}

/// Advance a passive-reply watermark without ever regressing a newer delivery.
async fn advance_covered_watermark(
    state: &AppState,
    task_id: ObjectId,
    inbound_id: ObjectId,
    inbound_created_at: DateTime,
) -> AppResult<()> {
    state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .update_one(
            doc! {
                "_id": task_id,
                "kind": DURABLE_INBOUND_REPLY_KIND,
                "$or": [
                    { "covered_through_inbound_created_at": { "$exists": false } },
                    { "covered_through_inbound_created_at": null },
                    { "covered_through_inbound_created_at": { "$lt": inbound_created_at } },
                    {
                        "covered_through_inbound_created_at": inbound_created_at,
                        "$or": [
                            { "covered_through_inbound_id": { "$exists": false } },
                            { "covered_through_inbound_id": null },
                            { "covered_through_inbound_id": { "$lt": inbound_id } },
                        ],
                    },
                ],
            },
            doc! { "$set": {
                "covered_through_inbound_id": inbound_id,
                "covered_through_inbound_created_at": inbound_created_at,
                "updated_at": DateTime::now(),
            } },
            None,
        )
        .await?;
    Ok(())
}

/// Settle an AI passive reply after all of its text segments are confirmed delivered. A policy
/// edit may already have invalidated the old claim, and a newer inbound may already have refreshed
/// the reusable task. The frozen watermark is still advanced, but the obligation is completed only
/// when that watermark remains the latest inbound and no manual reply owns the contact.
pub(crate) async fn settle_ai_reply_obligation(
    state: &AppState,
    task_id: ObjectId,
    inbound_id: ObjectId,
    inbound_created_at: DateTime,
) -> AppResult<bool> {
    advance_covered_watermark(state, task_id, inbound_id, inbound_created_at).await?;
    let result = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .update_one(
            doc! {
                "_id": task_id,
                "kind": DURABLE_INBOUND_REPLY_KIND,
                "latest_inbound_id": inbound_id,
                "latest_inbound_created_at": inbound_created_at,
                "manual_reply_run_id": { "$exists": false },
            },
            doc! {
                "$set": {
                    "status": "sent",
                    "gateway_status": "agent_reply_delivered",
                    "covered_through_inbound_id": inbound_id,
                    "covered_through_inbound_created_at": inbound_created_at,
                    "updated_at": DateTime::now(),
                },
                "$unset": {
                    "claim_token": "", "claimed_at": "", "outbox_decision_id": "",
                    "next_retry_at": "", "error": "",
                },
            },
            None,
        )
        .await?;
    Ok(result.matched_count == 1)
}

/// Resolve a manual reply pause. Delivery covers only the frozen watermark; inbound messages that
/// arrived while MCP was sending remain on the same obligation and are rescheduled by current
/// Workspace policy. A failed enqueue/terminal send never advances coverage.
pub(crate) async fn settle_manual_reply_obligation(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    run_id: &str,
    delivered: bool,
) -> AppResult<bool> {
    let task_id = durable_inbound_task_id(workspace_id, account_id, contact_wxid);
    let Some(task) = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": task_id, "manual_reply_run_id": run_id }, None)
        .await?
    else {
        return Ok(false);
    };
    let Some(contact) = state
        .db
        .contacts()
        .find_one(
            doc! { "workspace_id": workspace_id, "account_id": account_id, "wxid": contact_wxid },
            None,
        )
        .await?
    else {
        return Ok(false);
    };
    let config = agent::load_user_operation_domain_config(state, workspace_id).await?;
    let runtime = crate::agent::UserRuntimeParameters::from_config(config.as_ref(), state);
    let run_at = policy_run_at(&runtime, &contact, state);
    let covers_id = task.get_object_id("manual_covers_through_inbound_id").ok();
    let covers_at = task
        .get_datetime("manual_covers_through_inbound_created_at")
        .copied()
        .ok();

    if delivered {
        if let (Some(id), Some(at)) = (covers_id, covers_at) {
            advance_covered_watermark(state, task_id, id, at).await?;
            // Exact latest-watermark CAS: an inbound racing this update prevents false completion.
            let completed = state
                .db
                .tasks()
                .clone_with_type::<Document>()
                .update_one(
                    doc! {
                        "_id": task_id,
                        "manual_reply_run_id": run_id,
                        "latest_inbound_id": id,
                        "latest_inbound_created_at": at,
                    },
                    doc! {
                        "$set": {
                            "status": "sent",
                            "gateway_status": "manual_reply_delivered",
                            "covered_through_inbound_id": id,
                            "covered_through_inbound_created_at": at,
                            "updated_at": DateTime::now(),
                        },
                        "$unset": {
                            "manual_reply_run_id": "", "manual_reply_started_at": "",
                            "manual_covers_through_inbound_id": "",
                            "manual_covers_through_inbound_created_at": "", "claim_token": "",
                            "claimed_at": "", "outbox_decision_id": "", "next_retry_at": "",
                        },
                    },
                    None,
                )
                .await?;
            if completed.matched_count == 1 {
                return Ok(true);
            }
        }
    }

    // Failed delivery, or a successful delivery with a later inbound: release only this manual
    // owner's pause and keep the single obligation runnable under the current Workspace policy.
    let result = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": task_id, "manual_reply_run_id": run_id },
            doc! {
                "$set": {
                    "status": "pending",
                    "run_at": run_at,
                    "gateway_status": if delivered {
                        "manual_reply_delivered_newer_inbound_pending"
                    } else {
                        "manual_reply_failed_rescheduled"
                    },
                    "updated_at": DateTime::now(),
                },
                "$unset": {
                    "manual_reply_run_id": "", "manual_reply_started_at": "",
                    "manual_covers_through_inbound_id": "",
                    "manual_covers_through_inbound_created_at": "", "claim_token": "",
                    "claimed_at": "", "outbox_decision_id": "", "next_retry_at": "",
                },
            },
            None,
        )
        .await?;
    Ok(result.matched_count == 1)
}

fn manual_outbox_settlement(statuses: &[String]) -> Option<bool> {
    if statuses.is_empty() {
        return None;
    }
    if statuses.iter().all(|status| status == "sent") {
        return Some(true);
    }
    // Any sent segment makes delivery partial and irreversible. Keep the pause until an operator
    // resolves it; never let an AI reply duplicate or contradict the already delivered prefix.
    if statuses.iter().any(|status| {
        matches!(
            status.as_str(),
            "sent" | "pending" | "in_flight" | "delivery_unknown"
        )
    }) {
        return None;
    }
    Some(false)
}

/// Recover manual pauses after external cancellation, terminal failure, or a crash between pause
/// and Outbox creation. `delivery_unknown` deliberately remains paused to avoid a duplicate reply.
pub(crate) async fn reconcile_manual_reply_obligations(state: &AppState) -> AppResult<u64> {
    use futures::TryStreamExt;
    const ORPHAN_GRACE_MS: i64 = 5 * 60 * 1000;

    let mut cursor = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find(
            doc! { "kind": DURABLE_INBOUND_REPLY_KIND, "manual_reply_run_id": { "$type": "string" } },
            FindOptions::builder().limit(100).build(),
        )
        .await?;
    let mut settled = 0u64;
    while let Some(task) = cursor.try_next().await? {
        let Ok(run_id) = task.get_str("manual_reply_run_id") else {
            continue;
        };
        let Ok(workspace_id) = task.get_str("workspace_id") else {
            continue;
        };
        let Ok(account_id) = task.get_str("account_id") else {
            continue;
        };
        let Ok(wxid) = task.get_str("contact_wxid") else {
            continue;
        };
        let mut outboxes = state
            .db
            .collection_agent_send_outbox()
            .find(
                doc! { "run_id": run_id, "source_kind": "manual_send" },
                None,
            )
            .await?;
        let mut statuses = Vec::new();
        while let Some(entry) = outboxes.try_next().await? {
            statuses.push(entry.status);
        }
        let action = if statuses.is_empty() {
            let started = task
                .get_datetime("manual_reply_started_at")
                .or_else(|_| task.get_datetime("updated_at"))
                .map(|v| v.timestamp_millis())
                .unwrap_or_else(|_| DateTime::now().timestamp_millis());
            (DateTime::now().timestamp_millis() - started >= ORPHAN_GRACE_MS).then_some(false)
        } else {
            manual_outbox_settlement(&statuses)
        };
        if let Some(delivered) = action {
            if settle_manual_reply_obligation(
                state,
                workspace_id,
                account_id,
                wxid,
                run_id,
                delivered,
            )
            .await?
            {
                settled = settled.saturating_add(1);
            }
        }
    }
    Ok(settled)
}

/// Recompute every unfinished passive-reply obligation after a workspace policy edit.
///
/// The task transition is the authorization fence: it clears the old claim before asking the
/// Outbox to cancel that decision. A worker already beyond the remote boundary is allowed to
/// settle truthfully; every earlier stage is stopped and the same obligation is rescheduled.
pub async fn reconcile_workspace_reply_obligations(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<u64> {
    use futures::TryStreamExt;

    let config = agent::load_user_operation_domain_config(state, workspace_id).await?;
    let runtime = crate::agent::UserRuntimeParameters::from_config(config.as_ref(), state);
    let tasks = state.db.tasks().clone_with_type::<Document>();
    let mut cursor = tasks
        .find(
            doc! {
                "workspace_id": workspace_id,
                "kind": { "$in": [
                    DURABLE_INBOUND_REPLY_KIND,
                    agent::quiet_hours::DEFERRED_INBOUND_REPLY_KIND,
                ] },
                "status": { "$in": ["pending", "retry", "failed", "running", "outbox_enqueued"] },
            },
            None,
        )
        .await?;
    let mut snapshots = Vec::new();
    while let Some(task) = cursor.try_next().await? {
        snapshots.push(task);
    }

    let mut changed = 0u64;
    for task in snapshots {
        let Ok(task_id) = task.get_object_id("_id") else {
            continue;
        };
        let Ok(account_id) = task.get_str("account_id") else {
            continue;
        };
        let Ok(contact_wxid) = task.get_str("contact_wxid") else {
            continue;
        };
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
            continue;
        };
        let run_at = policy_run_at(&runtime, &contact, state);
        let old_decision = task.get_object_id("outbox_decision_id").ok();
        let is_legacy =
            task.get_str("kind").ok() == Some(agent::quiet_hours::DEFERRED_INBOUND_REPLY_KIND);

        // Invalidate ownership first. Dispatcher authorization always joins back to this task.
        let result = tasks
            .update_one(
                doc! {
                    "_id": task_id,
                    "status": { "$in": ["pending", "retry", "failed", "running", "outbox_enqueued"] },
                },
                if is_legacy {
                    doc! {
                        "$set": {
                            "status": "cancelled",
                            "gateway_status": "merged_into_reply_obligation",
                            "updated_at": DateTime::now(),
                        },
                        "$unset": {
                            "claim_token": "", "claimed_at": "", "active_task_key": "",
                            "outbox_decision_id": "", "next_retry_at": "",
                        },
                    }
                } else {
                    doc! {
                        "$set": {
                            "status": "pending",
                            "run_at": run_at,
                            "gateway_status": "policy_reconciled",
                            "attempt_count": 0,
                            "updated_at": DateTime::now(),
                        },
                        "$unset": {
                            "claim_token": "", "claimed_at": "", "next_retry_at": "",
                            "outbox_decision_id": "", "error": "",
                        },
                    }
                },
                None,
            )
            .await?;
        if result.matched_count == 0 {
            continue;
        }
        changed = changed.saturating_add(1);
        if let Some(decision_id) = old_decision {
            agent::cancel_for_decision(
                state,
                workspace_id,
                decision_id,
                "quiet_hours_policy_changed",
            )
            .await?;
        }

        if is_legacy {
            if let Some(inbound) = state
                .db
                .messages()
                .find_one(
                    doc! {
                        "workspace_id": workspace_id,
                        "account_id": account_id,
                        "contact_wxid": contact_wxid,
                        "direction": "inbound",
                    },
                    mongodb::options::FindOneOptions::builder()
                        .sort(doc! { "created_at": -1, "_id": -1 })
                        .build(),
                )
                .await?
            {
                materialize_durable_inbound_task_at(
                    state,
                    &contact,
                    &inbound,
                    run_at,
                    "policy_reconciled",
                )
                .await?;
            }
        }
    }
    Ok(changed)
}

/// Recover messages persisted before their task handoff completed. This runs at
/// the beginning of every task-worker tick and is safe to repeat.
pub async fn reconcile_pending_inbound_handoffs(state: &AppState) -> AppResult<u64> {
    use futures::TryStreamExt;

    let mut cursor = state
        .db
        .messages()
        .clone_with_type::<Document>()
        .find(
            doc! { "direction": "inbound", "handoff_status": { "$in": [HANDOFF_PENDING, HANDOFF_DEFERRED] } },
            FindOptions::builder()
                .sort(doc! { "created_at": 1, "_id": 1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut recovered = 0u64;
    while let Some(raw) = cursor.try_next().await? {
        // A single undecodable row must never abort this scan: both task-worker
        // ticks propagate an Err from here, and a broken row sorts first forever
        // (`created_at` ascending), so failing the whole function would stop
        // every later pending handoff from materializing. Isolate the row out of
        // the scan filter and keep going.
        let inbound: ConversationMessage = match mongodb::bson::from_document(raw.clone()) {
            Ok(inbound) => inbound,
            Err(error) => {
                quarantine_undecodable_inbound_handoff(state, &raw, &error.to_string()).await;
                continue;
            }
        };
        let Some(message_id) = inbound.id else {
            continue;
        };
        let contact = state
            .db
            .contacts()
            .find_one(
                doc! {
                    "workspace_id": &inbound.workspace_id,
                    "account_id": &inbound.account_id,
                    "wxid": &inbound.contact_wxid,
                },
                None,
            )
            .await?;
        // The process may have crashed after inserting the inbound fact but
        // before the ordinary webhook path upserted its contact. Re-run the
        // same idempotent contact materialization here. Explicitly
        // non-operatable identities are terminal; an operatable new contact is
        // recreated as `normal`, matching the uninterrupted webhook path.
        let contact = match contact {
            Some(contact) => Some(contact),
            None if is_operatable_person(&inbound.contact_wxid) => {
                upsert_webhook_contact(
                    state,
                    &inbound.workspace_id,
                    &inbound.account_id,
                    &inbound.contact_wxid,
                    &Value::Null,
                )
                .await?
            }
            None => None,
        };
        let Some(contact) = contact else {
            mark_inbound_handoff(state, message_id, HANDOFF_IGNORED).await?;
            continue;
        };
        if contact.agent_status != AgentStatus::Managed {
            mark_inbound_handoff(state, message_id, HANDOFF_IGNORED).await?;
            continue;
        }

        let domain_config = agent::load_user_operation_domain_config_for_contact(
            state,
            &contact.workspace_id,
            &contact.id.map(|id| id.to_hex()).unwrap_or_default(),
        )
        .await?;
        let runtime =
            crate::agent::UserRuntimeParameters::from_config(domain_config.as_ref(), state);
        let active_profile =
            agent::domain_profile::load_active_domain_profile(&state.db, &contact.workspace_id)
                .await?;
        let quiet = runtime.quiet_hours_enabled
            && agent::quiet_hours::is_quiet_now(
                runtime.quiet_hours_start,
                runtime.quiet_hours_end,
                runtime.quiet_hours_tz_offset_hours,
            );
        if quiet {
            let run_at = agent::quiet_hours::next_wake_at(
                runtime.quiet_hours_end,
                runtime.quiet_hours_tz_offset_hours,
                &contact.wxid,
                state.config.wake_jitter_max_seconds,
            );
            materialize_durable_inbound_task_at(
                state,
                &contact,
                &inbound,
                run_at,
                "quiet_hours_waiting",
            )
            .await?;
        } else {
            let window_ms = crate::agent::domain_profile::resolve_debounce_window_ms(
                &active_profile,
                state.config.message_debounce_window_ms,
            );
            materialize_durable_inbound_task(state, &contact, &inbound, window_ms).await?;
        }
        recovered = recovered.saturating_add(1);
    }
    Ok(recovered)
}

/// Isolate one undecodable pending-handoff row (defect #2, poison pill). The
/// row is stamped `handoff_status=quarantined` directly by `_id` on the raw
/// Document path (no typed deserialization), which removes it from the
/// `$in: [pending, deferred]` scan filter, and an admin-visible event records
/// the decode error. Everything here is best-effort: if the quarantine write
/// fails the row simply stays pending and is retried on the next tick.
async fn quarantine_undecodable_inbound_handoff(
    state: &AppState,
    raw: &Document,
    decode_error: &str,
) {
    let Some(row_id) = raw.get("_id").cloned() else {
        tracing::warn!(
            decode_error,
            "undecodable inbound handoff row lacks _id; cannot quarantine"
        );
        return;
    };
    let update = state
        .db
        .messages()
        .clone_with_type::<Document>()
        .update_one(
            doc! {
                "_id": &row_id,
                "handoff_status": { "$in": [HANDOFF_PENDING, HANDOFF_DEFERRED] },
            },
            doc! { "$set": {
                "handoff_status": HANDOFF_QUARANTINED,
                "handoff_updated_at": DateTime::now(),
            } },
            None,
        )
        .await;
    let quarantined_now = match update {
        Ok(result) => result.modified_count > 0,
        Err(error) => {
            tracing::warn!(
                %error,
                row_id = %row_id,
                "failed to quarantine undecodable inbound handoff row"
            );
            return;
        }
    };
    // A concurrent worker may have quarantined the same row first; only the
    // winner records the event so it is written exactly once per row.
    if !quarantined_now {
        return;
    }
    tracing::warn!(
        row_id = %row_id,
        decode_error,
        "quarantined undecodable inbound handoff row"
    );
    let workspace_id = raw
        .get_str("workspace_id")
        .map(str::to_owned)
        .unwrap_or_else(|_| state.config.default_workspace_id.clone());
    let account_id = raw
        .get_str("account_id")
        .map(str::to_owned)
        .unwrap_or_else(|_| state.config.default_account_id.clone());
    let contact_wxid = raw.get_str("contact_wxid").map(str::to_owned).ok();
    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id,
                account_id,
                contact_wxid,
                kind: "inbound_handoff_quarantined".to_string(),
                status: "warning".to_string(),
                summary: "入站消息行无法反序列化，已隔离出交接扫描，待运维排查".to_string(),
                details: Some(doc! {
                    "message_id": &row_id,
                    "decode_error": decode_error,
                }),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;
}

/// Stable quota identity for one tenant/account/window.
fn webhook_rate_limit_bucket_id(
    workspace_id: &str,
    account_id: &str,
    window_start_ms: i64,
) -> String {
    let mut hasher = Sha256::new();
    for value in [workspace_id, account_id] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    hasher.update(window_start_ms.to_be_bytes());
    format!("webhook:{}", hex::encode(hasher.finalize()))
}

/// Cross-replica fixed-window webhook quota. One deterministic Mongo document is the
/// linearization point for `(workspace, account, window)`. The update pipeline increments even
/// above capacity. Concurrent first upserts can race on `_id`; the loser retries the same atomic
/// increment without upsert, so every accepted request is counted exactly once.
async fn shared_webhook_rate_limit(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    capacity: u32,
    window_seconds: u32,
) -> AppResult<Option<u64>> {
    let window_ms = i64::from(window_seconds.max(1)).saturating_mul(1_000);
    let now_ms = DateTime::now().timestamp_millis();
    let window_start_ms = now_ms.div_euclid(window_ms).saturating_mul(window_ms);
    let window_end_ms = window_start_ms.saturating_add(window_ms);
    let bucket_id = webhook_rate_limit_bucket_id(workspace_id, account_id, window_start_ms);
    let expires_at = DateTime::from_millis(window_end_ms.saturating_add(window_ms));
    let collection = state
        .db
        .raw()
        .collection::<Document>("webhook_rate_limit_windows");
    let update = vec![doc! { "$set": {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "window_start_ms": window_start_ms,
        "window_end_ms": window_end_ms,
        "expires_at": expires_at,
        "count": { "$add": [{ "$ifNull": ["$count", 0_i64] }, 1_i64] },
        "updated_at": DateTime::now(),
    }}];
    let first = collection
        .find_one_and_update(
            doc! { "_id": &bucket_id },
            update.clone(),
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await;
    let bucket = match first {
        Ok(bucket) => bucket,
        Err(error) if is_duplicate_key_error(&error) => {
            collection
                .find_one_and_update(
                    doc! { "_id": &bucket_id },
                    update,
                    FindOneAndUpdateOptions::builder()
                        .return_document(mongodb::options::ReturnDocument::After)
                        .build(),
                )
                .await?
        }
        Err(error) => return Err(error.into()),
    }
    .ok_or_else(|| AppError::External("webhook rate-limit bucket disappeared".to_string()))?;
    let count = bucket.get_i64("count").unwrap_or(i64::MAX);
    if count > i64::from(capacity.max(1)) {
        let retry_ms = window_end_ms.saturating_sub(now_ms).max(1);
        return Ok(Some(((retry_ms + 999) / 1_000) as u64));
    }
    Ok(None)
}

// ───────── Legacy in-process debounce compatibility (tests/tools only) ─────────
//
// Production webhook ingestion no longer calls this scheduler: it materializes one durable
// `inbound_reply` task and relies on the task lease/CAS protocol for cross-replica single-flight.
// These public helpers remain temporarily for historical integration tests and external tools;
// they MUST NOT be reintroduced into the production webhook path.
//
// Historical problem: user bursts previously spawned one independent pipeline per webhook.
// decision→review→send 流水线（~10-15s），三条 → 三条并发流水线 → 发三条
// 回复，且 min_reply_interval 存在 TOCTOU、画像/记忆并发写竞态。
//
// 方案 = 去抖聚合 + 单联系人串行 + 新消息抢占重算：
// - 按联系人单 runner（PENDING 里 entry 存在即"runner 存活"），同一联系人两条
//   流水线不可能重叠 → 天然串行；
// - 每条入站刷新 deadline（去抖窗口重置），runner 等用户说完再只跑一次，
//   聚合由 gateway 的 load_recent_messages 天然完成；
// - 每条入站 generation +1；runner 跑完一轮发现 generation 变了就重算，并把
//   "运行期间到新消息"协作式传给网关（should_abort_send），让已过时的生成在
//   落盘/入队前主动放弃。
//
// `PENDING` is intentionally process-local and therefore unsuitable for production replicas.
// The durable task path above is the sole production authority.

pub fn contact_key(workspace_id: &str, account_id: &str, wxid: &str) -> String {
    format!("{workspace_id}:{account_id}:{wxid}")
}

/// 单联系人的去抖 / 抢占共享状态。`generation` 每入站 +1，既是去抖触发也是
/// 抢占信号；`deadline_ms` 每入站刷新即重置去抖窗口；`latest_inbound` 是最新
/// 入站快照（短锁，绝不跨 `.await` 持有）。
pub struct PendingState {
    pub generation: AtomicU64,
    deadline_ms: AtomicI64,
    pub latest_inbound: parking_lot::Mutex<ConversationMessage>,
}

static PENDING: LazyLock<DashMap<String, Arc<PendingState>>> = LazyLock::new(DashMap::new);

fn now_ms() -> i64 {
    DateTime::now().timestamp_millis()
}

/// 去抖截止时刻 = now + window，饱和加防溢出（纯函数，便于单测）。
fn next_deadline_ms(now: i64, window_ms: u64) -> i64 {
    now.saturating_add(window_ms as i64)
}

/// 抢占判定：当前 generation 与 runner 起跑时的快照不同 → 期间有新入站。
fn barge_in_triggered(gen_at_start: u64, current_generation: u64) -> bool {
    gen_at_start != current_generation
}

/// 注册一条入站到去抖调度器。在 DashMap `entry()` shard 锁内原子决策
/// spawn-vs-bump：已有 runner 只刷新 deadline / 替换最新入站 / bump generation
/// （不再 spawn）；没有则插入新状态并 spawn 一个 runner。返回 true 表示本次
/// 新起了 runner（调用方据此 spawn）。
pub fn register_inbound(
    key: String,
    inbound: ConversationMessage,
    window_ms: u64,
) -> (Arc<PendingState>, bool) {
    let deadline = next_deadline_ms(now_ms(), window_ms);
    let entry = PENDING.entry(key).or_insert_with(|| {
        Arc::new(PendingState {
            generation: AtomicU64::new(0),
            deadline_ms: AtomicI64::new(deadline),
            latest_inbound: parking_lot::Mutex::new(inbound.clone()),
        })
    });
    let st = entry.clone();
    // generation 起始 0，本次入站统一 +1 → 首条 runner 起跑快照见到 1。
    let prev_gen = st.generation.fetch_add(1, Ordering::AcqRel);
    st.deadline_ms.store(deadline, Ordering::Release);
    *st.latest_inbound.lock() = inbound;
    let spawned_now = prev_gen == 0;
    (st, spawned_now)
}

/// 去抖 runner 主体：等用户说完（deadline 到）→ 快照 generation + 最新入站 →
/// reload contact（非 managed 则退休）→ 一次反应分析 + 一次聚合网关（带抢占
/// guard）→ 若期间有新入站则重算，否则原子退休。
pub async fn run_debounce_pipeline(
    state: AppState,
    key: String,
    st: Arc<PendingState>,
    workspace_id: String,
    account_id: String,
    from_wxid: String,
    app_id: Option<String>,
) {
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let state_for_panic = state.clone();
    let workspace_for_panic = workspace_id.clone();
    let account_for_panic = account_id.clone();
    let wxid_for_panic = from_wxid.clone();
    let app_for_panic = app_id.clone();
    let key_for_panic = key.clone();

    let inner =
        async move {
            loop {
                // (a) 去抖睡眠——可被后到入站刷新 deadline 反复重置。
                loop {
                    let now = now_ms();
                    let dl = st.deadline_ms.load(Ordering::Acquire);
                    if now >= dl {
                        break;
                    }
                    let wait = (dl - now).max(0) as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                }

                // (b) 快照本轮 generation + 最新入站（锁立即释放，绝不跨 .await）。
                let gen_at_start = st.generation.load(Ordering::Acquire);
                let inbound = st.latest_inbound.lock().clone();

                // (c) reload contact——窗口期可能转 unmanaged / 被删，早退。
                let contact =
                    match reload_managed_contact(&state, &workspace_id, &account_id, &from_wxid)
                        .await
                    {
                        Ok(Some(c)) => c,
                        Ok(None) => {
                            PENDING.remove(&key);
                            return;
                        }
                        Err(error) => {
                            let _ = agent::write_event_for_account(
                                &state,
                                &workspace_id,
                                &account_id,
                                Some(&from_wxid),
                                "agent_error",
                                "failed",
                                &format!("debounce reload contact failed: {error}"),
                                app_id.clone().map(|v| doc! { "app_id": v }),
                            )
                            .await;
                            PENDING.remove(&key);
                            return;
                        }
                    };

                // (d) 一次反应分析（每串只在最新入站上跑一次 → 串行化，修反应写竞态）。
                // 旁路分析：失败只 warn，绝不阻断本轮回复。
                if let Err(error) = agent::record_user_reaction(&state, &contact, &inbound).await {
                    let _ = agent::write_event_for_account(
                        &state,
                        &workspace_id,
                        &account_id,
                        Some(&from_wxid),
                        "agent_error",
                        "failed",
                        &format!("record_user_reaction failed: {error}"),
                        app_id.clone().map(|v| doc! { "app_id": v }),
                    )
                    .await;
                }

                // (e) 一次聚合网关（无条件执行——与 (d) 解耦：reaction 是对上一轮结果的旁路分析，
                // 与生成本轮回复无因果依赖，其失败绝不该吞本轮应答）。带协作式抢占 guard：
                // 运行期间 generation 变了即放弃。
                let guard_state = st.clone();
                let guard: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
                    barge_in_triggered(gen_at_start, guard_state.generation.load(Ordering::Acquire))
                });
                if let Err(error) =
                    agent::handle_managed_message_aggregated(&state, contact, &inbound, Some(guard))
                        .await
                {
                    let _ = agent::write_event_for_account(
                        &state,
                        &workspace_id,
                        &account_id,
                        Some(&from_wxid),
                        "agent_error",
                        "failed",
                        &error.to_string(),
                        app_id.clone().map(|v| doc! { "app_id": v }),
                    )
                    .await;
                }

                // (f) 运行期间有新入站 → 重算（deadline 已被 register_inbound 刷新过）。
                if barge_in_triggered(gen_at_start, st.generation.load(Ordering::Acquire)) {
                    continue;
                }

                // (g) 原子退休：谓词在 shard 锁内复核 generation 未变才移除；若晚到
                // 入站刚 bump 过 generation，谓词失败 → 不移除 → 回 loop 重算。
                if PENDING
                    .remove_if(&key, |_, s| {
                        s.generation.load(Ordering::Acquire) == gen_at_start
                    })
                    .is_some()
                {
                    return;
                }
            }
        };

    if let Err(panic_payload) = AssertUnwindSafe(inner).catch_unwind().await {
        // runner panic：写事件 + 移除 state，下条入站会重 spawn。一次 panic 最多
        // 丢在途这一串（与旧 per-webhook spawn 同爆炸半径）。
        PENDING.remove(&key_for_panic);
        let panic_msg = panic_payload_message(&panic_payload);
        tracing::error!(
            workspace_id = %workspace_for_panic,
            account_id = %account_for_panic,
            wxid = %wxid_for_panic,
            panic = %panic_msg,
            "debounce pipeline panicked"
        );
        let _ = agent::write_event_for_account(
            &state_for_panic,
            &workspace_for_panic,
            &account_for_panic,
            Some(&wxid_for_panic),
            "webhook_handler_panic",
            "warning",
            &format!("debounce pipeline panicked: {panic_msg}"),
            app_for_panic.map(|v| doc! { "app_id": v }),
        )
        .await;
    }
}

/// reload contact 并判定是否仍 managed。返回 `Ok(None)` 表示不存在或已非 managed
/// （runner 应退休，只持久化不应答）。
async fn reload_managed_contact(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
) -> AppResult<Option<Contact>> {
    let contact = state
        .db
        .contacts()
        .find_one(
            managed_contact_reload_filter(workspace_id, account_id, wxid),
            None,
        )
        .await?;
    Ok(contact.filter(|c| c.agent_status == AgentStatus::Managed))
}

fn managed_contact_reload_filter(workspace_id: &str, account_id: &str, wxid: &str) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "wxid": wxid,
    }
}

pub async fn wechat_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Json<Value>> {
    // 方案 B 验签在下方「解析 appId、查到账号密钥之后」进行（fail-closed 全路径验签），
    // 见 resolve_account_context 之后的 webhook_verify_signature 块。此处仅解析 body。
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("invalid json body: {}", e)))?;

    // GeWe 控制事件不喂 Agent，立刻 200 返回，保证 MCP 那边 5s timeout 内收到 ack。
    // 方案 B（fail-closed）下按「是否产生副作用」分两处放置：
    // (a) `testMsg` 探活无副作用 → 留在验签门之前直接 ack（GeWe 控制台「测试回调」按钮用）。
    // (b) `TypeName=Offline/Online` 落库 `online`（供 outbox dispatcher 发送前 gate，掉线
    //     defer 不盲发）有副作用 → 下沉到验签门之后（见 resolve_account_context 之后）。
    if let Some(test_msg) = find_string(&payload, &["testMsg", "TestMsg"]) {
        return Ok(Json(serde_json::json!({
            "ok": true,
            "ignored": "callback_test",
            "echo": test_msg
        })));
    }

    // P2：MCP（GeWe-agent）转发的 payload 是 GeWe 原始 body 直接透传 + 顶层加
    // 一个 `_mcp` envelope（tenantId/accountId/sourceMsgId 等）。GeWe 字段一般是
    // 大写驼峰（`Appid` / `Wxid` / `FromUserName` / `Content` / `MsgId` / `NewMsgId`
    // / `TypeName` / `ToUserName`），少量小写驼峰（`appId` / `fromWxid`），所以
    // find_string 的 keys 必须同时覆盖两种风格。`_mcp.appId` 也算一份兜底。
    let app_id = find_string(
        &payload,
        &["appId", "app_id", "appid", "Appid", "AppId", "APPID"],
    );
    let (workspace_id, account_id, webhook_secret) =
        match resolve_account_context(&state, app_id.as_deref()).await {
            Ok(triple) => triple,
            Err(AppError::BadRequest(msg)) => {
                // P1：未知 appId 不再静默回退到 default account_id；写一条 admin-visible
                // 事件后明确 400，让运维侧能看到「webhook 入站但无对应 account」。
                let _ = emit_unknown_app_id_event(&state, app_id.as_deref()).await;
                return Err(AppError::BadRequest(msg));
            }
            Err(other) => return Err(other),
        };

    // 方案 B 验签门（fail-closed）：签名开关打开时，任何副作用之前必须验签通过。
    // 校验 gewe-agent 每账号 x-webhook-signature + x-webhook-timestamp 时效。
    if state.config.webhook_verify_signature {
        let now_ms = DateTime::now().timestamp_millis();
        if let Err(reason) = verify_webhook_signature(
            webhook_secret.as_deref(),
            headers
                .get("x-webhook-timestamp")
                .and_then(|v| v.to_str().ok()),
            headers
                .get("x-webhook-signature")
                .and_then(|v| v.to_str().ok()),
            &body,
            now_ms,
            state.config.webhook_timestamp_skew_seconds,
        ) {
            tracing::warn!(
                ?reason,
                account_id = %account_id,
                body_len = body.len(),
                "webhook rejected: signature verification failed"
            );
            return Err(AppError::BadRequest("invalid signature".into()));
        }
    }

    // (b) `TypeName=Offline/Online`：账号在线状态事件，落库 `online` 建状态源（供 outbox
    //     dispatcher 发送前 gate，掉线 defer 不盲发）。写 online 有副作用，必须在验签门之后。
    if let Some(type_name) = find_string(&payload, &["TypeName", "typeName"]) {
        let lower = type_name.to_ascii_lowercase();
        if lower == "offline" || lower == "online" {
            let online = lower == "online";
            if let Some(app_id) = app_id.as_deref() {
                // fail-soft：状态落库失败不应让 MCP 侧收不到 ack（会触发重推）。
                let res = state
                    .db
                    .accounts()
                    .update_one(
                        doc! { "app_id": app_id },
                        doc! { "$set": { "online": online, "last_sync_at": DateTime::now() } },
                        None,
                    )
                    .await;
                if let Err(err) = res {
                    tracing::warn!(?err, app_id, online, "persist account online state failed");
                }
            }
            return Ok(Json(serde_json::json!({
                "ok": true,
                "ignored": if online { "online_event" } else { "offline_event" },
                "type": type_name
            })));
        }
    }

    // LP-14 / Task 20: the quota is shared by every replica for this workspace/account.
    if let Some(retry_after) = shared_webhook_rate_limit(
        &state,
        &workspace_id,
        &account_id,
        state.config.webhook_rate_limit_capacity,
        state.config.webhook_rate_limit_window_seconds,
    )
    .await?
    {
        let _ = maybe_emit_rate_limit_event(&state, &workspace_id, &account_id).await;
        return Err(AppError::RateLimited {
            retry_after,
            account_id,
        });
    }

    let from_wxid = gewe_data_string(&payload, "FromUserName")
        .or_else(|| {
            find_string(
                &payload,
                &[
                    // 小写驼峰（手工 / 自测 / 部分推送）
                    "fromWxid",
                    "from_wxid",
                    "fromUserName",
                    "from_user_name",
                    "fromusername",
                    "from",
                    // GeWe 大写驼峰（MCP 透传的真实推送主字段）
                    "FromUserName",
                    "FromWxid",
                    "Wxid",
                ],
            )
        })
        .ok_or_else(|| AppError::BadRequest("webhook missing sender wxid".to_string()))?;
    let content = gewe_data_string(&payload, "Content")
        .or_else(|| {
            find_string(
                &payload,
                &[
                    // 小写驼峰
                    "content",
                    "text",
                    "msgContent",
                    "msg_content",
                    "message",
                    "messageContent",
                    // GeWe 大写驼峰
                    "Content",
                    "PushContent",
                ],
            )
        })
        .unwrap_or_default();
    // 领导回复分流：from_wxid 是本 workspace 的 principal_decider → 走请示通道，不进客户链路。
    // 必须在落库 / contact-managed 处理之前分流——领导可能同时也是某 contact，
    // consumed=true 时短路返回，避免领导自己的消息被当成客户入站处理。
    if (crate::agent::escalation::lookup_principal_config(
        &state,
        &workspace_id,
        &account_id,
        &from_wxid,
    )
    .await?)
        .is_some()
    {
        let consumed = crate::agent::escalation::handle_principal_reply(
            &state,
            &workspace_id,
            &account_id,
            &from_wxid,
            &content,
        )
        .await?;
        if consumed {
            return Ok(Json(
                serde_json::json!({ "ok": true, "routed": "principal" }),
            ));
        }
    }
    let message_id = find_string(
        &payload,
        &[
            // 小写驼峰
            "newMsgId",
            "new_msg_id",
            "msgId",
            "msg_id",
            "messageId",
            "id",
            // GeWe 大写驼峰
            "NewMsgId",
            "MsgId",
            "MessageId",
        ],
    );
    // P2：dedupe key 优先用 GeWe sourceMsgId（MCP 那边按
    // `${slot.id}:${appId}:${sourceMsgId}` 做转发去重，且 5s timeout 内不重试，
    // 单次推送绝不能丢）。也兼顾 _mcp envelope 里冗余的 sourceMsgId / msgId
    // 字段，万一 GeWe 顶层 MsgId 缺失仍能正确去重。
    let envelope_msg_id = payload
        .get("_mcp")
        .and_then(|env| env.get("sourceMsgId"))
        .and_then(value_to_string);
    let effective_message_id = message_id.clone().or(envelope_msg_id);
    // A-03（已知边界，不修）：无任何 msgId（顶层 MsgId/NewMsgId + _mcp.sourceMsgId 全缺）时
    // dedupe_key 回落 payload-hash，同内容连发的第二条 hash 相同 → 命中 unique 索引被当 duplicate
    // 丢弃。生产 GeWe AddMsg 恒带 NewMsgId → effective_message_id 必有值走 message:{id} 分支，
    // 此路径仅自测 / 无 ID payload 触发。掺接收时刻/nonce 会削弱重放去重，收益不抵，故不修。
    let dedupe_key = effective_message_id
        .as_ref()
        .map(|id| format!("message:{id}"))
        .unwrap_or_else(|| format!("payload:{}", stable_payload_hash(&payload)));

    // P0-19：dedupe 原子化。原 check-then-insert 存在 TOCTOU 竞态，两个并发
    // webhook 的 find_one 都可能返回 None，导致同一条入站消息被双写。改为
    // 直接 insert_one + 捕获 11000 duplicate key 错误（依赖
    // db/indexes.rs:55-63 的 partial unique index `workspace_id+account_id+dedupe_key`），
    // 让 MongoDB 在写入时原子去重。
    let raw = to_document(&payload).ok();
    // F1：解析入站消息类型 + 媒体引用，不再写死 None。
    let msg_type = parse_inbound_msg_type(&payload);
    let media_ref = extract_inbound_media_ref(&payload, msg_type);
    let mut inbound = ConversationMessage {
        id: None,
        workspace_id: workspace_id.clone(),
        account_id: account_id.clone(),
        contact_wxid: from_wxid.clone(),
        message_id: effective_message_id.clone(),
        dedupe_key: Some(dedupe_key.clone()),
        direction: MessageDirection::Inbound,
        content,
        msg_type: Some(msg_type.to_string()),
        media_ref,
        raw,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    };
    // SR-177: the durable handoff marker is part of the same Mongo insert as
    // the inbound fact. A crash after this write but before task materialization
    // is recovered by `reconcile_pending_inbound_handoffs` on the task worker.
    let mut inbound_doc = to_document(&inbound)?;
    inbound_doc.insert("handoff_status", HANDOFF_PENDING);
    match state
        .db
        .messages()
        .clone_with_type::<Document>()
        .insert_one(inbound_doc, None)
        .await
    {
        Ok(result) => {
            inbound.id = result.inserted_id.as_object_id();
        }
        Err(error) if is_duplicate_key_error(&error) => {
            return Ok(Json(serde_json::json!({ "ok": true, "duplicate": true })));
        }
        Err(error) => return Err(error.into()),
    }

    let mut contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": &account_id,
                "wxid": &from_wxid
            },
            None,
        )
        .await?;

    if contact.is_none() {
        contact = upsert_webhook_contact(&state, &workspace_id, &account_id, &from_wxid, &payload)
            .await?;
    }

    let Some(contact) = contact else {
        // 非私聊真人（gh_ 公众号 / @chatroom 群）：消息已落库（见上方 messages().insert_one），
        // 但不建运营池联系人、不触发 Agent 流水线（这类 wxid 本就不可能 managed）。
        if let Some(message_oid) = inbound.id {
            let _ = mark_inbound_handoff(&state, message_oid, HANDOFF_IGNORED).await;
        }
        return Ok(Json(
            serde_json::json!({ "ok": true, "skipped": "not_operatable_contact" }),
        ));
    };

    let now = DateTime::now();
    // S1（自学习采集管道）：在 contact 的 last_inbound_at / last_outbound_at 被本轮
    // 更新覆盖之前，先快照出"上一条入站 / 上一条出站"时间，用于构造 T1 行为信号
    // （reply_latency / reactivation）。采集是 best-effort 旁路，绝不阻断应答。
    let prev_last_inbound_ms = contact.last_inbound_at.map(|d| d.timestamp_millis());
    let prev_last_outbound_ms = contact.last_outbound_at.map(|d| d.timestamp_millis());
    // A-06：last_inbound_at/last_message_at/updated_at 是统计/信号旁路字段，落库失败不应连累
    // 本轮应答（inbound 已在上方 insert 成功、去重已保证）。降 best-effort：失败仅 warn，与紧邻的
    // collect_inbound_behavior_signals（下方）旁路纪律对齐。
    if let Err(e) = state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact.id },
            doc! {
                "$set": {
                    "last_inbound_at": now,
                    "last_message_at": now,
                    "updated_at": now
                }
            },
            None,
        )
        .await
    {
        tracing::warn!(contact_wxid = %from_wxid, error = ?e, "更新 last_inbound_at 失败（统计旁路，不影响应答）");
    }

    // S1：落 T1 行为信号（观察层，不解释、不评分）。每条带 dedupe_key，重复
    // webhook / 重放只落一次。任何一段失败仅 warn，不影响后续 Agent 应答。
    collect_inbound_behavior_signals(
        &state,
        &workspace_id,
        &account_id,
        &from_wxid,
        effective_message_id.as_deref(),
        &inbound.content,
        now,
        prev_last_inbound_ms,
        prev_last_outbound_ms,
    )
    .await;

    // P2：MCP（GeWe-agent）那一侧 fetch(messageWebhookUrl) 用了 5s AbortController
    // timeout 且失败不重试。Agent 决策 + Review 流水线一次约 10–15s，远超
    // 5s，必须把它挪到后台 spawn，主请求落库后立即 ack。
    //
    // 并发多消息去抖：不再每条 webhook 直接 spawn 一条流水线，而是注册到按联系人
    // 的去抖调度器。已有 runner 时只刷新 deadline + bump generation（不 spawn）；
    // 没有时插入状态并 spawn 一个 runner。runner 等去抖窗口到再跑一次聚合流水线，
    // 运行期间到的新消息会触发抢占重算（见 run_debounce_pipeline）。
    let managed = contact.agent_status == AgentStatus::Managed;
    let mut deferred = false;
    if managed {
        // #69 作息门控：静默时段（运营方进程本地时区）客户来消息时**不立即回**，
        // Schedule the contact's single inbound_reply obligation at the wake time. The inbound is persisted above,
        // 醒来时 gateway 的 load_recent_messages 会天然聚合这段时间的全部消息一次性回。
        // 开关/时段来自运营域配置（RuntimeParametersTyped，前端可改），默认启用。
        let domain_config = agent::load_user_operation_domain_config_for_contact(
            &state,
            &workspace_id,
            &contact.id.map(|id| id.to_hex()).unwrap_or_default(),
        )
        .await?;
        let runtime =
            crate::agent::UserRuntimeParameters::from_config(domain_config.as_ref(), &state);
        let active_profile =
            agent::domain_profile::load_active_domain_profile(&state.db, &workspace_id).await?;
        let quiet = runtime.quiet_hours_enabled
            && agent::quiet_hours::is_quiet_now(
                runtime.quiet_hours_start,
                runtime.quiet_hours_end,
                runtime.quiet_hours_tz_offset_hours,
            );
        if quiet {
            let run_at = agent::quiet_hours::next_wake_at(
                runtime.quiet_hours_end,
                runtime.quiet_hours_tz_offset_hours,
                &contact.wxid,
                state.config.wake_jitter_max_seconds,
            );
            materialize_durable_inbound_task_at(
                &state,
                &contact,
                &inbound,
                run_at,
                "quiet_hours_waiting",
            )
            .await?;
            deferred = true;
        } else {
            let window_ms = crate::agent::domain_profile::resolve_debounce_window_ms(
                &active_profile,
                state.config.message_debounce_window_ms,
            );
            let durable_task =
                materialize_durable_inbound_task(&state, &contact, &inbound, window_ms).await?;
            let task_id = durable_task.task_id;
            let run_at_ms = durable_task.run_at_ms;
            let bg_state = state.clone();
            tokio::spawn(async move {
                let now_ms = DateTime::now().timestamp_millis();
                let wait_ms = run_at_ms.saturating_sub(now_ms).max(0) as u64;
                tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                if let Err(error) = crate::tasks::run_due_task_by_id(&bg_state, task_id).await {
                    tracing::error!(%task_id, %error, "durable inbound immediate wake failed; periodic worker will retry");
                }
            });
        }
    } else if let Some(message_oid) = inbound.id {
        mark_inbound_handoff(&state, message_oid, HANDOFF_IGNORED).await?;
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "managed": managed,
        "queued": managed && !deferred,
        "deferred": deferred
    })))
}

/// #69 作息门控：静默时段入站时，确保存在一条"醒来回复"跟进任务。
///
/// kind = [`agent::quiet_hours::DEFERRED_INBOUND_REPLY_KIND`]，与 planner 主动催进的
/// `follow_up` 区分——precheck 据此豁免 `context_changed`（这条任务的存在意义恰恰
/// 就是回 task 创建后累积的客户消息）。`run_at` = 下一次醒来时刻；醒来后由 task
/// worker → handle_follow_up_task → gateway 走完整决策/审查/拆短/outbox 链路。
///
/// 去重：仿 planner `has_pending_follow_up` —— 同 contact 已有未终态的 wake 任务则
/// 不再插（静默时段连发多条 → 1 task → 醒来基于累积消息回 1 次）。先查后插存在
/// TOCTOU 窗口，但 precheck 的 rate_limited 闸在醒来时会兜住重复触达，可接受。
///
/// `pub`：暴露给 tests/quiet_hours_deferral.rs 集成测试直接驱动排程链路
/// （`Utc::now` 不可注入，集成测试只验 DB 写入 + 去重 + 埋点，时区由纯函数单测覆盖）。
pub async fn ensure_wake_followup_task(
    state: &AppState,
    contact: &Contact,
    wake_hour: u32,
    tz_offset_hours: i32,
) -> AppResult<()> {
    // Compatibility entry point for tests/tools. Production webhook ingestion calls
    // `materialize_durable_inbound_task_at` with the exact persisted inbound.
    let Some(inbound) = state
        .db
        .messages()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "direction": "inbound",
            },
            mongodb::options::FindOneOptions::builder()
                .sort(doc! { "created_at": -1, "_id": -1 })
                .build(),
        )
        .await?
    else {
        return Ok(());
    };
    let run_at = agent::quiet_hours::next_wake_at(
        wake_hour,
        tz_offset_hours,
        &contact.wxid,
        state.config.wake_jitter_max_seconds,
    );
    materialize_durable_inbound_task_at(state, contact, &inbound, run_at, "quiet_hours_waiting")
        .await?;
    Ok(())
}

fn stable_payload_hash(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// 判定 mongodb 错误是否为 DuplicateKey（code 11000 / 11001）。
/// 与 `agent::outbox::is_duplicate_key_error` 同语义；不跨 mod 复用以避免
/// webhook 反向依赖 agent 内部 helper。
/// S1（自学习采集管道）：落本条入站对应的 T1 行为信号（best-effort 旁路）。
///
/// 在 contact 的 last_* 时间戳被本轮覆盖之前由调用方快照 `prev_*_ms` 传入。
/// 缺 `message_id` 时退化用 `observed_at` 毫秒作为 dedupe 后缀——保证仍幂等
/// （同一时刻的同 contact 不会重复落），但跨重放去重精度略降。
///
/// 任何一段失败只 `warn`，绝不向上抛——采集出错不能拖累用户应答。
#[allow(clippy::too_many_arguments)]
async fn collect_inbound_behavior_signals(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
    message_id: Option<&str>,
    content: &str,
    inbound_at: DateTime,
    prev_last_inbound_ms: Option<i64>,
    prev_last_outbound_ms: Option<i64>,
) {
    use crate::behavior_signals as bs;
    let dedupe_suffix = message_id
        .map(ToString::to_string)
        .unwrap_or_else(|| inbound_at.timestamp_millis().to_string());

    let mut signals = vec![
        bs::build_reply_latency(
            workspace_id,
            account_id,
            wxid,
            &dedupe_suffix,
            inbound_at,
            prev_last_outbound_ms,
        ),
        bs::build_reply_length(
            workspace_id,
            account_id,
            wxid,
            &dedupe_suffix,
            inbound_at,
            content,
        ),
    ];
    if bs::is_reactivation(
        prev_last_inbound_ms,
        inbound_at,
        bs::REACTIVATION_THRESHOLD_MS,
    ) {
        signals.push(bs::build_reactivation(
            workspace_id,
            account_id,
            wxid,
            &dedupe_suffix,
            inbound_at,
        ));
    }

    for signal in signals {
        let signal_type = signal.signal_type.clone();
        let result = bs::persist_signal(state, signal).await;
        bs::record_signal_metric(state, workspace_id, &result).await;
        if let Err(error) = result {
            tracing::warn!(
                error = %error,
                wxid = %wxid,
                signal_type = %signal_type,
                "behavior_signal persist failed (best-effort, ignored)"
            );
        }
    }
}

fn is_duplicate_key_error(err: &mongodb::error::Error) -> bool {
    match &*err.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            write_error.code == 11000 || write_error.code == 11001
        }
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .as_ref()
            .map(|errs| errs.iter().any(|e| e.code == 11000 || e.code == 11001))
            .unwrap_or(false),
        _ => false,
    }
}

/// 把 panic payload 解析成可读字符串。与 supervisor::panic_payload_to_string
/// 同语义；不跨 mod 复用以保持 webhook 模块 self-contained。
fn panic_payload_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(value_to_string) {
                    return Some(found);
                }
            }
            for child in map.values() {
                if let Some(found) = find_string(child, keys) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_string(item, keys)),
        _ => None,
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// 从 GeWe AddMsg 的 `Data.<field>.string` 取字符串。真实推送里发件人/内容都是
/// `{string:...}` 包裹且嵌在 `Data` 下——通用 find_string 会被顶层同名/近义键
/// (`Wxid` / `PushContent`)遮蔽,故对 GeWe 形态显式走此路径,优先于 find_string。
/// 取不到返回 None(交调用方回落 find_string)。命中空串返回 Some("")——刻意直接
/// 用空内容,不回落到带发件人名前缀的 PushContent 通知串。
fn gewe_data_string(payload: &Value, field: &str) -> Option<String> {
    payload
        .get("Data")
        .and_then(|d| d.get(field))
        .and_then(|f| f.get("string"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

/// 从 GeWe AddMsg 的 `Data.MsgType.low` 取微信消息类型数字码(`{low:N}` 包裹)。
/// 返回数字的字符串形式(交 classify_inbound_msg_type 归一)。取不到返回 None。
fn gewe_data_msg_type_code(payload: &Value) -> Option<String> {
    payload
        .get("Data")
        .and_then(|d| d.get("MsgType"))
        .and_then(|m| m.get("low"))
        .and_then(|n| n.as_i64())
        .map(|n| n.to_string())
}

/// F1 评审 I1：从入站 payload 解析归一化的消息类型。候选键**仅限“消息类型”语义
/// 专用键**——GeWe 大写驼峰 `MsgType`（微信数字码真实字段）+ 手工/自测 payload 的
/// 小写别名 `msgType`/`msg_type`。**刻意不收泛化裸键 `type`/`Type`**：`find_string`
/// 深度递归整棵 JSON（含 `_mcp` envelope 及任意嵌套对象），而 webhook envelope 里
/// 与消息类型无关的 `{"type":"event",...}` 极常见，泛化键会被误命中、把本应默认
/// `text` 的纯文本消息误标为非文本，破坏 text 主链路。真实字段就是 `MsgType`，删掉
/// `type`/`Type` 既消除误伤面又不漏真实字段。
///
/// payload 无任何类型字段时默认 `"text"`（runbook W1-W3 等纯文本自测 payload 不带
/// 类型字段，行为不变）；有则交给 `classify_inbound_msg_type` 归一（未知码 → `"unknown"`）。
fn parse_inbound_msg_type(payload: &Value) -> &'static str {
    let raw_msg_type = gewe_data_msg_type_code(payload)
        .or_else(|| find_string(payload, &["msgType", "msg_type", "MsgType"]));
    match raw_msg_type.as_deref() {
        Some(raw_type) => classify_inbound_msg_type(raw_type),
        None => "text",
    }
}

/// 把 webhook payload 里的原始消息类型（GeWe 透传的微信 `MsgType` 数字码，或
/// 手工/自测 payload 的字符串别名）归一化为稳定的 `msg_type` 字符串。F1 地基：
/// 让非文本入站可被识别（图片/语音/视频/名片/链接卡片等），不再被当空文本硬答。
///
/// 未知类型一律归 `"unknown"`——**绝不崩、绝不当 text**，下游据此走非文本分支
/// （F2 才做媒体理解/过渡话术，本函数只负责识别归类）。
///
/// 微信协议私聊 MsgType 数字码：1=文本 3=图片 34=语音 43=视频 42=名片
/// 47=表情 48=位置 49=appmsg(链接/文件/小程序) 50=语音/视频通话 51=状态同步
/// 10000/10002=系统消息。GeWe 私聊真实非文本入站 payload 仓内暂无确认样例，
/// 数字码以微信协议为准；新码值落 `"unknown"` 而非误判，安全侧。
fn classify_inbound_msg_type(raw: &str) -> &'static str {
    match raw.trim() {
        "1" | "text" | "Text" => "text",
        "3" | "image" | "Image" | "img" => "image",
        "34" | "voice" | "Voice" => "voice",
        "43" | "video" | "Video" => "video",
        "42" | "namecard" | "card" => "namecard",
        "47" | "emoji" | "sticker" => "emoji",
        "48" | "location" => "location",
        "49" | "appmsg" | "link" | "file" | "miniprogram" => "appmsg",
        "50" | "voip" => "voip",
        "51" => "statussync",
        "10000" | "10002" | "sysmsg" | "system" => "system",
        _ => "unknown",
    }
}

/// 从入站 payload 提取媒体引用（图片 cdn url / 文件 id / 语音 path 等），供后续
/// 多模态理解链路（F2）定位媒体内容。`text` 消息恒返回 None。
///
/// GeWe 富媒体引用通常嵌在 `Content` 的 XML 里（`cdnurl`/`aeskey`）或独立 media
/// 字段；仓内暂无确认的非文本入站样例，故此处只从已知候选字段名尽力提取一个可
/// 定位引用，找不到返回 None（**不崩、不造假**）。F2 接通 MCP 媒体下载后再补全。
fn extract_inbound_media_ref(payload: &Value, msg_type: &str) -> Option<String> {
    if msg_type == "text" {
        return None;
    }
    find_string(
        payload,
        &[
            // 小写驼峰（自测/手工）
            "mediaUrl",
            "media_url",
            "fileUrl",
            "file_url",
            "cdnUrl",
            "cdn_url",
            "cdnurl",
            "mediaId",
            "media_id",
            "fileId",
            "file_id",
            // GeWe 大写驼峰
            "MediaUrl",
            "FileUrl",
            "CdnUrl",
            "MediaId",
            "FileId",
        ],
    )
}

async fn resolve_account_context(
    state: &AppState,
    app_id: Option<&str>,
) -> AppResult<(String, String, Option<String>)> {
    if let Some(app_id) = app_id {
        if let Some(account) = state
            .db
            .accounts()
            .find_one(doc! { "app_id": app_id }, None)
            .await?
        {
            // 第三元 = 该账号 webhook_secret，供方案 B 验签门使用。
            return Ok((
                account.workspace_id,
                account.account_id,
                account.webhook_secret,
            ));
        }
        // P1：appId 提供了但 wechat_accounts 没匹配 —— 之前会静默回退到
        // default_account_id，导致 inbound 落到错的 account 下，managed contact
        // 永远 lookup 不到，AI 不回复。改成显式 400，让 webhook 侧能看到。
        return Err(AppError::BadRequest(format!(
            "webhook appId {app_id} not registered in wechat_accounts"
        )));
    }
    // A-05：无 appId 时的账号归属防线。验签门（handler 的 webhook_verify_signature 块）在本函数
    // 之后执行——verify=true 时无 appId → 返回 secret=None → verify_webhook_signature 必
    // SecretNotConfigured → 400，default 回退到不了副作用点，无需在此付 count 代价。仅当未开验签
    // （default 回退是唯一防线）时才校验：多账号无 appId 无法判断消息归属 → 400（防落到 default
    // account 张冠李戴）；单账号（≤1）无歧义 → 回落 default，不打断上游确实不带 appId 的单账号部署。
    if !state.config.webhook_verify_signature {
        let account_count = state.db.accounts().count_documents(doc! {}, None).await?;
        if account_count > 1 {
            return Err(AppError::BadRequest(
                "webhook 缺 appId 且存在多个账号，无法判断消息归属".into(),
            ));
        }
    }
    Ok((
        state.config.default_workspace_id.clone(),
        state.config.default_account_id.clone(),
        None,
    ))
}

/// P1：webhook 收到未知 appId 时写一条 admin-visible 事件，便于运维诊断
/// 「inbound 200 但 contact 不存在 / managed 不工作」类问题。
async fn emit_unknown_app_id_event(state: &AppState, app_id: Option<&str>) -> AppResult<()> {
    let summary = match app_id {
        Some(id) => format!("webhook 入站 appId={id} 在 wechat_accounts 中未注册，已拒收"),
        // A-05：此事件仅在 resolve_account_context 返 BadRequest 时写。无 appId 走到 BadRequest
        // 只有一种情形——未开验签 + 多账号（单账号无 appId 会回落 default 返 Ok、不进本事件）。
        None => "webhook 入站缺失 appId 字段且存在多个账号，无法判断归属，已拒收".to_string(),
    };
    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: state.config.default_account_id.clone(),
                contact_wxid: None,
                kind: "webhook_unknown_app_id".to_string(),
                status: "rejected".to_string(),
                summary,
                details: app_id.map(|id| doc! { "app_id": id }),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;
    Ok(())
}

/// 判定 wxid 是否能进运营池的私聊真人：排除公众号（gh_ 前缀）、群（@chatroom）、
/// 微信官方系统保留号（weixin/fmessage/... 复用 mcp::is_system_account 同源判据）。
/// 建档 upsert（:1049）+ m029 存量清理共用此判据，杜绝两处漂移。
pub(crate) fn is_operatable_person(wxid: &str) -> bool {
    !(wxid.starts_with("gh_")
        || wxid.contains("@chatroom")
        || wxid.contains("@openim")
        || crate::mcp::is_system_account(wxid))
}

/// 账号不能运营自己：判断某 wxid 是否等于当前账号自身 wxid。
/// 与真人判据 `is_operatable_person` 解耦——这是「不能自己运营自己」的逻辑铁律，
/// 不是「是否真人」的判断。`account_self_wxid` 为 None（账号未同步 wxid）时返回 false（无从判定，不拦）。
pub(crate) fn is_self_account(wxid: &str, account_self_wxid: Option<&str>) -> bool {
    matches!(account_self_wxid, Some(self_wxid) if self_wxid == wxid)
}

async fn upsert_webhook_contact(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
    _payload: &Value,
) -> AppResult<Option<Contact>> {
    // 非私聊真人（公众号/群）不进运营池——消息仍在调用点落库，只是不建 contact。
    if !is_operatable_person(wxid) {
        return Ok(None);
    }
    // 昵称/头像不再从 payload 取：真实 GeWe payload 发件人只有 wxid，
    // find_string 会递归命中 _mcp.nickName（账号自己昵称 "Demi"）。改从 roster 富化。
    let (roster_nickname, roster_avatar) =
        roster_identity_for(state, workspace_id, account_id, wxid)
            .await
            .unwrap_or((None, None));
    // P1：兜底 —— 如果同 (workspace_id, wxid) 已有 managed 记录在另一个
    // account_id 下，本次 inbound 与 managed contact 出现 account_id 错配，
    // 写一条 admin-visible 事件提醒（不创建影子副本会更激进，留给后续 PR）。
    if let Some(existing) = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "wxid": wxid,
                "agent_status": "managed"
            },
            None,
        )
        .await?
    {
        if existing.account_id != account_id {
            let _ = state
                .db
                .events()
                .insert_one(
                    crate::models::AgentEvent {
                        id: None,
                        workspace_id: workspace_id.to_string(),
                        account_id: account_id.to_string(),
                        contact_wxid: Some(wxid.to_string()),
                        kind: "webhook_managed_contact_account_mismatch".to_string(),
                        status: "warning".to_string(),
                        summary: format!(
                            "同一 wxid 在 account={} 下被标记 managed，本次 inbound 落到 account={}，将创建 normal 影子记录，AI 不会自动回复",
                            existing.account_id, account_id
                        ),
                        details: Some(doc! {
                            "managed_account_id": existing.account_id.clone(),
                            "inbound_account_id": account_id,
                            "wxid": wxid,
                        }),
                        created_at: DateTime::now(),
                        dedupe_key: None,
                    },
                    None,
                )
                .await;
        }
    }
    // 只在 roster 命中时写 nickname/avatar_url——否则无条件 $set None 会覆盖已有值。
    let mut set_doc = doc! { "updated_at": DateTime::now() };
    if let Some(nick) = &roster_nickname {
        set_doc.insert("nickname", nick);
    }
    if let Some(av) = &roster_avatar {
        set_doc.insert("avatar_url", av);
    }
    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": wxid
            },
            doc! {
                "$set": set_doc,
                "$setOnInsert": {
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "wxid": wxid,
                    "agent_status": "normal",
                    "created_at": DateTime::now()
                }
            },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": wxid
            },
            None,
        )
        .await
        .map_err(AppError::from)
}

/// P1-2：rate_limit 事件 partial-unique 去重 key。
///
/// 形式 `rate_limit:{account}:{day_bucket}`，`day_bucket = epoch_ms / 86_400_000`。
/// 同一账号在同一 UTC 天最多一条 `webhook_rate_limited` 事件，由 partial unique
/// index `workspace_id + dedupe_key` 在并发下原子约束。
fn rate_limit_event_dedupe_key(account_id: &str, day_bucket: i64) -> String {
    format!("rate_limit:{}:{}", account_id, day_bucket)
}

fn build_rate_limit_event(
    workspace_id: &str,
    account_id: &str,
    day_bucket: i64,
) -> crate::models::AgentEvent {
    crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "webhook_rate_limited".to_string(),
        status: "blocked".to_string(),
        summary: "webhook 入口触发 per-account 限流".to_string(),
        details: None,
        created_at: DateTime::now(),
        dedupe_key: Some(rate_limit_event_dedupe_key(account_id, day_bucket)),
    }
}

/// LP-14 / Task 20：限流命中时按 account 当日去重写一条 agent_event，避免事件爆量。
///
/// P1-2：旧实现 `find_one + insert_one` 在并发限流命中时存在 TOCTOU——
/// 两条请求都查到 `None`，都写入，事件爆量。改为携带 `dedupe_key` 原子写：
/// `dedupe_key = "rate_limit:{account}:{day_bucket}"`，配合 partial unique
/// index（`workspace_id + dedupe_key`）让重复 insert 直接命中 dup-key error
/// 后被吞掉；首条写入获胜，后续都视为"今天已记录"。
async fn maybe_emit_rate_limit_event(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<()> {
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let now_ms = DateTime::now().timestamp_millis();
    let day_bucket = now_ms / day_ms;
    let event = build_rate_limit_event(workspace_id, account_id, day_bucket);
    match state.db.events().insert_one(&event, None).await {
        Ok(_) => Ok(()),
        Err(error) if is_duplicate_key_error(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// 从 roster friends 里按 wxid 找身份 `(nickname, avatar_url)`。找不到返 None。
/// 纯函数：不触网、不访库，便于单测。
pub(crate) fn pick_identity_from_friends(
    friends: &[crate::mcp::RosterFriend],
    wxid: &str,
) -> Option<(Option<String>, Option<String>)> {
    friends
        .iter()
        .find(|f| f.wxid == wxid)
        .map(|f| (f.nickname.clone(), f.avatar_url.clone()))
}

/// 查 roster 快照拿某 wxid 的 `(nickname, avatar_url)`。快照缺失/读失败/无该 wxid → None。
/// best-effort：读失败只返 None（吞错），绝不 panic、绝不阻断建档。
async fn roster_identity_for(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
) -> Option<(Option<String>, Option<String>)> {
    let snap = crate::mcp::read_roster_snapshot(state, workspace_id, account_id)
        .await
        .ok()
        .flatten()?;
    pick_identity_from_friends(&snap.friends, wxid)
}

#[cfg(test)]
mod roster_identity_tests {
    use super::*;
    use crate::mcp::RosterFriend;

    #[test]
    fn pick_identity_from_friends_finds_match() {
        let friends = vec![
            RosterFriend {
                wxid: "wxid_a".into(),
                nickname: Some("小明".into()),
                remark: None,
                avatar_url: Some("http://img/a".into()),
                sex: Some(0),
                is_non_human: false,
            },
            RosterFriend {
                wxid: "wxid_b".into(),
                nickname: None,
                remark: None,
                avatar_url: None,
                sex: Some(0),
                is_non_human: false,
            },
        ];
        assert_eq!(
            pick_identity_from_friends(&friends, "wxid_a"),
            Some((Some("小明".to_string()), Some("http://img/a".to_string())))
        );
        assert_eq!(
            pick_identity_from_friends(&friends, "wxid_b"),
            Some((None, None))
        );
        assert_eq!(pick_identity_from_friends(&friends, "wxid_missing"), None);
    }
}

#[cfg(test)]
mod inbound_msg_type_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_inbound_msg_type_maps_known_numeric_codes() {
        // GeWe 透传的微信 MsgType 数字码
        assert_eq!(classify_inbound_msg_type("1"), "text");
        assert_eq!(classify_inbound_msg_type("3"), "image");
        assert_eq!(classify_inbound_msg_type("34"), "voice");
        assert_eq!(classify_inbound_msg_type("43"), "video");
        assert_eq!(classify_inbound_msg_type("42"), "namecard");
        assert_eq!(classify_inbound_msg_type("49"), "appmsg");
    }

    #[test]
    fn classify_inbound_msg_type_maps_string_aliases() {
        // 手工/自测 payload 的字符串别名
        assert_eq!(classify_inbound_msg_type("text"), "text");
        assert_eq!(classify_inbound_msg_type("image"), "image");
        assert_eq!(classify_inbound_msg_type("voice"), "voice");
        assert_eq!(classify_inbound_msg_type("video"), "video");
        assert_eq!(classify_inbound_msg_type("link"), "appmsg");
    }

    #[test]
    fn classify_inbound_msg_type_trims_whitespace() {
        assert_eq!(classify_inbound_msg_type(" 3 "), "image");
        assert_eq!(classify_inbound_msg_type("\ttext\n"), "text");
    }

    #[test]
    fn classify_inbound_msg_type_unknown_never_falls_back_to_text() {
        // 未知类型归 unknown：不崩、不当 text（下游据此走非文本分支）
        assert_eq!(classify_inbound_msg_type("某新类型"), "unknown");
        assert_eq!(classify_inbound_msg_type("9999"), "unknown");
        assert_eq!(classify_inbound_msg_type(""), "unknown");
    }

    #[test]
    fn extract_media_ref_is_none_for_text() {
        let payload = json!({ "mediaUrl": "http://x/a.jpg", "content": "hi" });
        assert_eq!(extract_inbound_media_ref(&payload, "text"), None);
    }

    #[test]
    fn extract_media_ref_pulls_known_fields_for_media() {
        let payload = json!({ "fromWxid": "wx1", "cdnUrl": "http://cdn/x.jpg" });
        assert_eq!(
            extract_inbound_media_ref(&payload, "image"),
            Some("http://cdn/x.jpg".to_string())
        );
        let payload2 = json!({ "_mcp": { "MediaId": "media-123" } });
        assert_eq!(
            extract_inbound_media_ref(&payload2, "voice"),
            Some("media-123".to_string())
        );
    }

    #[test]
    fn extract_media_ref_none_when_no_reference_present() {
        // 非文本但 payload 无任何已知媒体引用字段 → None（不造假）
        let payload = json!({ "fromWxid": "wx1", "content": "" });
        assert_eq!(extract_inbound_media_ref(&payload, "image"), None);
    }

    #[test]
    fn parse_inbound_msg_type_uses_dedicated_keys() {
        // a. 专用键正常生效（回归不破）：顶层 MsgType 数字码 + 小写别名
        assert_eq!(parse_inbound_msg_type(&json!({ "MsgType": "3" })), "image");
        assert_eq!(
            parse_inbound_msg_type(&json!({ "msgType": "voice" })),
            "voice"
        );
        assert_eq!(
            parse_inbound_msg_type(&json!({ "msg_type": "43" })),
            "video"
        );
    }

    #[test]
    fn parse_inbound_msg_type_ignores_unrelated_nested_type_fields() {
        // b. 核心回归（I1）：payload 无任何类型字段，但嵌套对象带与消息类型无关的
        // type/Type（webhook envelope 极常见，如 {"type":"event"}）。泛化键已删，
        // find_string 深度递归不再误命中 → 仍按 text 处理，text 主链路一字不变。
        let payload = json!({
            "fromWxid": "wx1",
            "content": "你好，在吗",
            "_mcp": { "type": "event", "meta": { "Type": "callback" } },
        });
        assert_eq!(parse_inbound_msg_type(&payload), "text");

        // 顶层直接带无关 type 也不被误命中
        let payload2 = json!({ "type": "event", "content": "纯文本" });
        assert_eq!(parse_inbound_msg_type(&payload2), "text");
    }

    #[test]
    fn parse_inbound_msg_type_defaults_text_for_plain_payload() {
        // c. 仿 runbook 自测纯文本 payload（不带任何类型字段）→ text（主链路不变）
        let payload = json!({
            "appId": "wx_app_1",
            "fromWxid": "wxid_customer",
            "content": "我想了解一下你们的产品",
        });
        assert_eq!(parse_inbound_msg_type(&payload), "text");
    }

    #[test]
    fn gewe_addmsg_extracts_msg_type_from_data_low() {
        // MsgType.low=3 → image。
        let payload = json!({
            "TypeName": "AddMsg",
            "Data": {
                "FromUserName": { "string": "wxid_x" },
                "Content": { "string": "x" },
                "MsgType": { "low": 3 }
            }
        });
        assert_eq!(gewe_data_msg_type_code(&payload).as_deref(), Some("3"));
        assert_eq!(parse_inbound_msg_type(&payload), "image");
        // MsgType.low=1 → text(真实文本入站)。
        let text_payload = json!({ "Data": { "MsgType": { "low": 1 } } });
        assert_eq!(parse_inbound_msg_type(&text_payload), "text");
    }

    fn real_gewe_addmsg() -> serde_json::Value {
        // 2026-07-09 线上 117 亲验的真实 GeWe AddMsg 形态(经 gewe-agent 转发):
        // 顶层大写驼峰 + Data 嵌套 + {string}/{low} 包裹 + _mcp envelope。
        json!({
            "Wxid": "wxid_3yeirsb75afd22",
            "TypeName": "AddMsg",
            "Appid": "wx_WSHYpbq5Fdp_yGcOEl9Pn",
            "Data": {
                "FromUserName": { "string": "wxid_ydzaomn4scsb12" },
                "ToUserName": { "string": "wxid_3yeirsb75afd22" },
                "Content": { "string": "你好" },
                "MsgType": { "low": 1 },
                "PushContent": "吴界 : 你好",
                "NewMsgId": { "high": 1976706754, "low": 1032436816 }
            },
            "_mcp": { "event": "wechat.message.created", "sourceMsgId": "8489890863244754000" }
        })
    }

    #[test]
    fn gewe_addmsg_extracts_real_sender_not_account_self() {
        let payload = real_gewe_addmsg();
        // 修复:显式走 Data.FromUserName.string 拿真实发件人(吴界)。
        assert_eq!(
            gewe_data_string(&payload, "FromUserName").as_deref(),
            Some("wxid_ydzaomn4scsb12")
        );
        // 回归留证:通用 find_string 会被顶层 Wxid 遮蔽 → 归错成账号自己。
        // 这正是本次修复的 bug,保留断言防止有人把提取改回纯 find_string。
        assert_eq!(
            find_string(&payload, &["fromWxid", "FromUserName", "FromWxid", "Wxid"]).as_deref(),
            Some("wxid_3yeirsb75afd22")
        );
    }

    #[test]
    fn gewe_addmsg_extracts_clean_content_not_pushcontent() {
        let payload = real_gewe_addmsg();
        // 修复:Data.Content.string 拿干净正文。
        assert_eq!(
            gewe_data_string(&payload, "Content").as_deref(),
            Some("你好")
        );
        // 回归留证:find_string 会先命中 Data.PushContent 通知串(带发件人名前缀)。
        assert_eq!(
            find_string(&payload, &["content", "Content", "PushContent"]).as_deref(),
            Some("吴界 : 你好")
        );
    }

    #[test]
    fn flat_payload_still_parses_via_fallback() {
        // 扁平自测/biz-test payload 无 Data → helper 返 None → 走 find_string 回落,行为不变。
        let payload = json!({ "fromWxid": "wx_flat", "content": "hello flat" });
        assert_eq!(gewe_data_string(&payload, "FromUserName"), None);
        assert_eq!(gewe_data_string(&payload, "Content"), None);
        assert_eq!(
            find_string(&payload, &["fromWxid"]).as_deref(),
            Some("wx_flat")
        );
        assert_eq!(
            find_string(&payload, &["content"]).as_deref(),
            Some("hello flat")
        );
    }

    #[test]
    fn is_operatable_person_rejects_official_and_group() {
        assert!(!is_operatable_person("gh_416c280c4978"));
        assert!(!is_operatable_person("7842243308@chatroom"));
        assert!(!is_operatable_person("971559326@chatroom"));
        // 企业微信/开放 IM 号非私聊真人。
        assert!(!is_operatable_person("25984984932102183@openim"));
    }

    #[test]
    fn is_operatable_person_accepts_real_wxid() {
        assert!(is_operatable_person("wxid_ydzaomn4scsb12"));
        assert!(is_operatable_person("wxid_3yeirsb75afd22"));
        // 边界：gh 出现在中间不算公众号（只认前缀）。
        assert!(is_operatable_person("wxid_gh_not_prefix"));
    }

    #[test]
    fn is_operatable_person_rejects_system_accounts() {
        assert!(!is_operatable_person("weixin")); // 微信团队
        assert!(!is_operatable_person("fmessage")); // 朋友推荐消息
        assert!(!is_operatable_person("newsapp"));
        // 真人 + 媒体号 wxid_* 仍放行（媒体号靠手动移除，非此拦）
        assert!(is_operatable_person("wxid_8874178741811")); // 福州晚报
    }

    #[test]
    fn is_self_account_detects_account_own_wxid() {
        // 账号自身 wxid == 目标 wxid → 拦。
        assert!(is_self_account(
            "wxid_3yeirsb75afd22",
            Some("wxid_3yeirsb75afd22")
        ));
        // 不同 wxid → 不拦。
        assert!(!is_self_account(
            "wxid_ydzaomn4scsb12",
            Some("wxid_3yeirsb75afd22")
        ));
        // 账号未同步 wxid（None）→ 无从判定，不拦。
        assert!(!is_self_account("wxid_ydzaomn4scsb12", None));
    }
}

#[cfg(test)]
mod debounce_tests {
    use super::*;

    #[test]
    fn contact_key_is_workspace_account_wxid() {
        assert_eq!(contact_key("ws", "acct", "wx1"), "ws:acct:wx1");
    }

    #[test]
    fn managed_contact_reload_filter_is_fully_tenant_scoped() {
        let filter = managed_contact_reload_filter("ws-a", "acct-a", "wx-a");
        assert_eq!(filter.get_str("workspace_id").unwrap(), "ws-a");
        assert_eq!(filter.get_str("account_id").unwrap(), "acct-a");
        assert_eq!(filter.get_str("wxid").unwrap(), "wx-a");
    }

    #[test]
    fn next_deadline_adds_window() {
        assert_eq!(next_deadline_ms(1_000, 4_000), 5_000);
        assert_eq!(next_deadline_ms(0, 1_000), 1_000);
    }

    #[test]
    fn next_deadline_saturates_instead_of_overflow() {
        // 饱和加：i64::MAX + window 不应回绕成负数（否则 runner 立即认为已过期）。
        assert_eq!(next_deadline_ms(i64::MAX, 4_000), i64::MAX);
        assert_eq!(next_deadline_ms(i64::MAX - 1, 4_000), i64::MAX);
    }

    #[test]
    fn barge_in_triggers_only_on_generation_change() {
        // generation 未变 → 无新入站 → 不抢占。
        assert!(!barge_in_triggered(3, 3));
        // generation 变了 → 期间有新入站 → 抢占重算。
        assert!(barge_in_triggered(3, 4));
        assert!(barge_in_triggered(0, 1));
    }

    #[test]
    fn register_first_inbound_spawns_then_subsequent_only_bump() {
        // 用唯一 key 避免与其它测试共享全局 PENDING。
        let key = "ws-test:acct-test:wx-debounce-spawn".to_string();
        PENDING.remove(&key);
        let msg = ConversationMessage {
            id: None,
            workspace_id: "ws-test".to_string(),
            account_id: "acct-test".to_string(),
            contact_wxid: "wx-debounce-spawn".to_string(),
            message_id: None,
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: "hi".to_string(),
            msg_type: None,
            media_ref: None,
            raw: None,
            is_synthetic_relay: false,
            created_at: DateTime::now(),
        };

        let (st1, spawned1) = register_inbound(key.clone(), msg.clone(), 4_000);
        assert!(spawned1, "首条入站 SHALL 触发 spawn");
        assert_eq!(st1.generation.load(Ordering::Acquire), 1);

        // 第二、三条：runner 已活，只 bump generation，不再 spawn。
        let (st2, spawned2) = register_inbound(key.clone(), msg.clone(), 4_000);
        assert!(!spawned2, "后续入站 SHALL NOT 再 spawn");
        assert_eq!(st2.generation.load(Ordering::Acquire), 2);
        let (st3, spawned3) = register_inbound(key.clone(), msg.clone(), 4_000);
        assert!(!spawned3);
        assert_eq!(st3.generation.load(Ordering::Acquire), 3);

        PENDING.remove(&key);
    }

    /// 测试用最小入站消息构造器（内容/key 由调用方区分）。
    fn test_inbound(wxid: &str, content: &str) -> ConversationMessage {
        ConversationMessage {
            id: None,
            workspace_id: "ws-test".to_string(),
            account_id: "acct-test".to_string(),
            contact_wxid: wxid.to_string(),
            message_id: None,
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: content.to_string(),
            msg_type: None,
            media_ref: None,
            raw: None,
            is_synthetic_relay: false,
            created_at: DateTime::now(),
        }
    }

    /// 正常退休：runner 起跑快照 gen_at_start，期间无新入站 → remove_if 谓词
    /// （generation 未变）成立 → 原子移除，key 不再驻留 PENDING。
    #[test]
    fn retire_succeeds_when_generation_unchanged() {
        let key = "ws-test:acct-test:wx-retire-ok".to_string();
        PENDING.remove(&key);

        let (st, spawned) =
            register_inbound(key.clone(), test_inbound("wx-retire-ok", "hi"), 4_000);
        assert!(spawned);
        let gen_at_start = st.generation.load(Ordering::Acquire);
        assert_eq!(gen_at_start, 1);

        // runner (g) 步：谓词在 shard 锁内复核 generation 未变才移除。
        let removed = PENDING
            .remove_if(&key, |_, s| {
                s.generation.load(Ordering::Acquire) == gen_at_start
            })
            .is_some();
        assert!(removed, "generation 未变时 SHALL 成功退休");
        assert!(!PENDING.contains_key(&key), "退休后 key 不得驻留");
    }

    /// 退休竞态（核心不变量，对应 plan「清理竞态证明」）：runner 起跑快照
    /// gen_at_start=1，跑流水线期间晚到一条入站把 generation bump 到 2；runner
    /// 到达 (g) 步执行 remove_if(gen==1) → 谓词失败 → 不移除 → key 仍在 →
    /// runner 据此回 loop 重算。证明边界期到达的消息绝不被丢。
    #[test]
    fn retire_blocked_when_late_inbound_bumped_generation() {
        let key = "ws-test:acct-test:wx-retire-race".to_string();
        PENDING.remove(&key);

        let (st, _) = register_inbound(key.clone(), test_inbound("wx-retire-race", "first"), 4_000);
        let gen_at_start = st.generation.load(Ordering::Acquire);
        assert_eq!(gen_at_start, 1);

        // 晚到入站：runner 已过抢占检查、正走向退休的窗口里到达，bump generation。
        let (_, spawned2) =
            register_inbound(key.clone(), test_inbound("wx-retire-race", "late"), 4_000);
        assert!(!spawned2, "晚到入站不得再 spawn——runner 仍在");

        // runner (g) 步：谓词复核 gen==gen_at_start(=1)，实际已是 2 → 失败 → 不移除。
        let removed = PENDING
            .remove_if(&key, |_, s| {
                s.generation.load(Ordering::Acquire) == gen_at_start
            })
            .is_some();
        assert!(
            !removed,
            "晚到入站 bump 后 SHALL NOT 退休（否则丢这条消息）"
        );
        assert!(
            PENDING.contains_key(&key),
            "退休被阻时 runner 状态必须留存以供重算"
        );
        // runner 据 barge_in_triggered 判定需重算。
        assert!(barge_in_triggered(
            gen_at_start,
            PENDING
                .get(&key)
                .unwrap()
                .generation
                .load(Ordering::Acquire)
        ));

        PENDING.remove(&key);
    }

    /// 退休后重 spawn：runner 成功退休移除 key 后，新入站落 Vacant 分支 →
    /// 插入全新状态（generation 从 0 重新 +1 = 1）→ spawned_now 再次为 true。
    #[test]
    fn retire_then_new_inbound_respawns() {
        let key = "ws-test:acct-test:wx-respawn".to_string();
        PENDING.remove(&key);

        let (st1, spawned1) = register_inbound(key.clone(), test_inbound("wx-respawn", "a"), 4_000);
        assert!(spawned1);
        let gen0 = st1.generation.load(Ordering::Acquire);
        PENDING.remove_if(&key, |_, s| s.generation.load(Ordering::Acquire) == gen0);
        assert!(!PENDING.contains_key(&key));

        // 退休后的新入站：必须重新 spawn（runner 已退场）。
        let (st2, spawned2) = register_inbound(key.clone(), test_inbound("wx-respawn", "b"), 4_000);
        assert!(spawned2, "退休后新入站 SHALL 重新 spawn runner");
        assert_eq!(
            st2.generation.load(Ordering::Acquire),
            1,
            "重 spawn 后 generation 从全新状态的 1 起算"
        );

        PENDING.remove(&key);
    }

    /// 并发 spawn 原子性：N 个线程同时注册同一 key，DashMap entry 持 shard 写锁
    /// 串行化 → 恰好一个线程拿到 spawned_now=true（防 double-spawn），最终
    /// generation == N。断言的是计数不变量，不依赖线程调度顺序 → 不 flaky。
    #[test]
    fn concurrent_register_same_key_spawns_exactly_once() {
        use std::sync::atomic::AtomicU32;
        use std::sync::{Arc as StdArc, Barrier};
        use std::thread;

        let key = "ws-test:acct-test:wx-concurrent".to_string();
        PENDING.remove(&key);

        const N: usize = 16;
        let barrier = StdArc::new(Barrier::new(N));
        let spawn_count = StdArc::new(AtomicU32::new(0));

        let handles: Vec<_> = (0..N)
            .map(|i| {
                let key = key.clone();
                let barrier = barrier.clone();
                let spawn_count = spawn_count.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let (_, spawned) = register_inbound(
                        key.clone(),
                        test_inbound("wx-concurrent", &format!("m{i}")),
                        4_000,
                    );
                    if spawned {
                        spawn_count.fetch_add(1, Ordering::AcqRel);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread join");
        }

        assert_eq!(
            spawn_count.load(Ordering::Acquire),
            1,
            "N 线程并发注册同一 key 必须恰好 spawn 一次"
        );
        assert_eq!(
            PENDING
                .get(&key)
                .unwrap()
                .generation
                .load(Ordering::Acquire),
            N as u64,
            "每条入站各 bump 一次 generation，最终须等于线程数"
        );

        PENDING.remove(&key);
    }

    /// 抢占链端到端：runner 起跑快照 gen_at_start，期间多条入站把 generation
    /// 推高 → barge_in_triggered 成立 → 网关 guard 返回 true → 放弃在途生成重算。
    /// 无新入站时 guard 恒 false，正常走完。
    #[test]
    fn barge_in_chain_from_register_to_guard() {
        let key = "ws-test:acct-test:wx-barge-chain".to_string();
        PENDING.remove(&key);

        let (st, _) = register_inbound(key.clone(), test_inbound("wx-barge-chain", "1"), 4_000);
        let gen_at_start = st.generation.load(Ordering::Acquire);

        // 无新入站：guard 视角 generation 未变 → 不抢占。
        assert!(!barge_in_triggered(
            gen_at_start,
            st.generation.load(Ordering::Acquire)
        ));

        // 期间到 2 条新入站。
        register_inbound(key.clone(), test_inbound("wx-barge-chain", "2"), 4_000);
        register_inbound(key.clone(), test_inbound("wx-barge-chain", "3"), 4_000);

        assert!(
            barge_in_triggered(gen_at_start, st.generation.load(Ordering::Acquire)),
            "期间有新入站时 guard SHALL 触发抢占重算"
        );

        PENDING.remove(&key);
    }
}

#[cfg(test)]
mod rate_limit_dedupe_tests {
    use super::*;

    /// P1-2：同一账号 + 同一 day_bucket → 同一 dedupe_key，
    /// partial unique index 才能在并发下原子去重。
    #[test]
    fn dedupe_key_is_stable_per_account_and_day() {
        let a = rate_limit_event_dedupe_key("acct_42", 19_876);
        let b = rate_limit_event_dedupe_key("acct_42", 19_876);
        assert_eq!(a, b);
        assert_eq!(a, "rate_limit:acct_42:19876");
    }

    /// 跨天必须不同 key，否则次日的限流事件被错误压制。
    #[test]
    fn dedupe_key_segregates_by_day_bucket() {
        let day_a = rate_limit_event_dedupe_key("acct_42", 19_876);
        let day_b = rate_limit_event_dedupe_key("acct_42", 19_877);
        assert_ne!(day_a, day_b);
    }

    /// 不同账号不可共享 key（否则 A 触发限流，B 整天再触发都被压制）。
    #[test]
    fn dedupe_key_segregates_by_account() {
        let a = rate_limit_event_dedupe_key("acct_a", 19_876);
        let b = rate_limit_event_dedupe_key("acct_b", 19_876);
        assert_ne!(a, b);
    }

    #[test]
    fn limiter_segregates_same_account_id_by_workspace() {
        let a = webhook_rate_limit_bucket_id("ws_a", "shared_account", 60_000);
        let b = webhook_rate_limit_bucket_id("ws_b", "shared_account", 60_000);
        assert_ne!(a, b);
        assert_eq!(
            a,
            webhook_rate_limit_bucket_id("ws_a", "shared_account", 60_000)
        );
    }

    #[test]
    fn rate_limit_event_preserves_resolved_tenant_scope() {
        let event = build_rate_limit_event("ws_non_default", "shared_account", 19_876);
        assert_eq!(event.workspace_id, "ws_non_default");
        assert_eq!(event.account_id, "shared_account");
        assert_eq!(
            event.dedupe_key.as_deref(),
            Some("rate_limit:shared_account:19876")
        );
    }
}

/// 方案 B：校验 gewe-agent 每账号签名 + 时间戳时效（纯函数，便于单测）。
///
/// gewe-agent 侧签名内容 = `"<timestamp_header.trim()>." + raw_body`，
/// HMAC-SHA256(每 slot 明文 messageWebhookSecret)，hex 写到
/// `x-webhook-signature: sha256=<hex>`，配套 `x-webhook-timestamp`（毫秒）。
/// 全部通过返回 Ok；否则返回具体拒绝原因（handler 统一转 400 + 脱敏 warn 日志）。
/// `secret=None`/空 → SecretNotConfigured（验签开关打开时的 fail-closed 语义）。
#[derive(Debug, PartialEq, Eq)]
enum WebhookSigError {
    SecretNotConfigured,
    MissingSignature,
    MissingTimestamp,
    BadTimestamp,
    TimestampOutOfWindow,
    BadSignatureFormat,
    Mismatch,
}

/// A-04（已知边界，不修）：仅校验 secret 存在 + 时间戳 ±skew 窗口 + HMAC-SHA256，无 nonce /
/// 一次性签名记录。攻击者截获一条合法签名请求可在 skew（默认 300s）内原样重放。但重放无重复副作用：
/// AddMsg 重放命中 message-id dedupe 幂等短路、Offline/Online 重放幂等 $set、领导回复经
/// resolve_escalation 幂等 → 不产生重复发送。加 nonce 需状态存储，收益不抵成本，故不修。
fn verify_webhook_signature(
    secret: Option<&str>,
    timestamp_header: Option<&str>,
    signature_header: Option<&str>,
    body: &[u8],
    now_ms: i64,
    skew_seconds: i64,
) -> Result<(), WebhookSigError> {
    let secret = secret
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::SecretNotConfigured)?;
    let sig = signature_header
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::MissingSignature)?;
    let ts_str = timestamp_header
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::MissingTimestamp)?;
    let ts_ms: i64 = ts_str.parse().map_err(|_| WebhookSigError::BadTimestamp)?;
    if (now_ms - ts_ms).abs() > skew_seconds.saturating_mul(1000) {
        return Err(WebhookSigError::TimestampOutOfWindow);
    }
    let hex_part = sig.strip_prefix("sha256=").unwrap_or(sig);
    let expected = hex::decode(hex_part).map_err(|_| WebhookSigError::BadSignatureFormat)?;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| WebhookSigError::SecretNotConfigured)?;
    // 与 gewe-agent 一致：先喂 "<ts>." 再喂 raw body。
    mac.update(ts_str.as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| WebhookSigError::Mismatch)
}

#[cfg(test)]
mod webhook_sig_tests {
    use super::*;

    // 与 gewe-agent webhook-signing.ts 逐字节对齐的金标：
    // HMAC-SHA256(secret="test-secret", "<ts>." + body) hex。
    const SECRET: &str = "test-secret";
    const TS: &str = "1720500000000";
    const BODY: &[u8] = b"{\"foo\":\"bar\"}";
    // python: hmac.new(b"test-secret", b"1720500000000." + BODY, sha256).hexdigest()
    const GOLDEN_HEX: &str = "1936755de0397e2cc912ab1652aaeccb278cae4bb489f16f0dbe3173a8057cbe";
    const NOW_MS: i64 = 1_720_500_000_000; // 与 TS 相等 → 偏差 0
    const SKEW: i64 = 300;

    fn header() -> String {
        format!("sha256={GOLDEN_HEX}")
    }

    #[test]
    fn accepts_correct_signature_within_window() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn accepts_signature_without_sha256_prefix() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(GOLDEN_HEX), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn accepts_uppercase_hex() {
        let h = format!("sha256={}", GOLDEN_HEX.to_uppercase());
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&h), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn rejects_tampered_body() {
        assert_eq!(
            verify_webhook_signature(
                Some(SECRET),
                Some(TS),
                Some(&header()),
                b"{\"foo\":\"BAR\"}",
                NOW_MS,
                SKEW
            ),
            Err(WebhookSigError::Mismatch)
        );
    }

    #[test]
    fn rejects_wrong_secret() {
        assert_eq!(
            verify_webhook_signature(
                Some("other-secret"),
                Some(TS),
                Some(&header()),
                BODY,
                NOW_MS,
                SKEW
            ),
            Err(WebhookSigError::Mismatch)
        );
    }

    #[test]
    fn rejects_timestamp_out_of_window_future() {
        // now 比 ts 早 301s（ts 在未来 301s）→ 超窗
        let now = NOW_MS - 301_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Err(WebhookSigError::TimestampOutOfWindow)
        );
    }

    #[test]
    fn rejects_timestamp_out_of_window_past() {
        // now 比 ts 晚 301s → 超窗
        let now = NOW_MS + 301_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Err(WebhookSigError::TimestampOutOfWindow)
        );
    }

    #[test]
    fn accepts_timestamp_at_window_edge() {
        // 恰好 300s → 不超窗（用 <= 边界语义）
        let now = NOW_MS + 300_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Ok(())
        );
    }

    #[test]
    fn rejects_missing_signature() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), None, BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingSignature)
        );
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some("  "), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingSignature)
        );
    }

    #[test]
    fn rejects_missing_timestamp() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), None, Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingTimestamp)
        );
    }

    #[test]
    fn rejects_bad_timestamp() {
        assert_eq!(
            verify_webhook_signature(
                Some(SECRET),
                Some("not-a-number"),
                Some(&header()),
                BODY,
                NOW_MS,
                SKEW
            ),
            Err(WebhookSigError::BadTimestamp)
        );
    }

    #[test]
    fn rejects_bad_signature_format() {
        assert_eq!(
            verify_webhook_signature(
                Some(SECRET),
                Some(TS),
                Some("sha256=not-hex!!"),
                BODY,
                NOW_MS,
                SKEW
            ),
            Err(WebhookSigError::BadSignatureFormat)
        );
    }

    #[test]
    fn rejects_secret_not_configured() {
        assert_eq!(
            verify_webhook_signature(None, Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::SecretNotConfigured)
        );
        assert_eq!(
            verify_webhook_signature(Some("  "), Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::SecretNotConfigured)
        );
    }
}

#[cfg(test)]
mod reply_obligation_tests {
    use super::manual_outbox_settlement;

    fn statuses(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn manual_outbox_settlement_requires_every_segment_sent() {
        assert_eq!(manual_outbox_settlement(&statuses(&["sent"])), Some(true));
        assert_eq!(
            manual_outbox_settlement(&statuses(&["sent", "sent"])),
            Some(true)
        );
        assert_eq!(
            manual_outbox_settlement(&statuses(&["sent", "pending"])),
            None
        );
        assert_eq!(
            manual_outbox_settlement(&statuses(&["sent", "failed_terminal"])),
            None
        );
    }

    #[test]
    fn manual_outbox_settlement_only_releases_confirmed_non_delivery() {
        assert_eq!(manual_outbox_settlement(&[]), None);
        for active in ["pending", "in_flight", "delivery_unknown"] {
            assert_eq!(manual_outbox_settlement(&statuses(&[active])), None);
        }
        assert_eq!(
            manual_outbox_settlement(&statuses(&["canceled", "failed_terminal"])),
            Some(false)
        );
    }
}

//! Durable operational incidents and administrator notifications.
//!
//! These records are intentionally independent from Ask-Human business
//! escalations. The Ask-Human configuration supplies only the notification
//! audience; no customer decision, principal card, or approval record is made.

use std::collections::HashSet;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument},
};

use crate::{
    error::{AppError, AppResult},
    models::{DeciderRef, SystemIncident, SystemIncidentRecipient},
    routes::AppState,
};

use super::{
    escalation::resolve_ask_human_policy,
    outbox::{enqueue, EnqueueOutcome, EnqueueRequest},
    run_envelope::SOURCE_KIND_SYSTEM_INCIDENT,
};

pub(crate) const INCIDENT_KIND_LLM_ACCOUNT_UNAVAILABLE: &str = "llm_account_unavailable";
const INCIDENT_STATUS_ACTIVE: &str = "active";
const INCIDENT_STATUS_RECOVERED: &str = "recovered";
const PHASE_OUTAGE: &str = "outage";
const PHASE_RECOVERY: &str = "recovery";

pub(crate) const LLM_OUTAGE_NOTIFICATION: &str =
    "【系统告警】大模型账户当前不可用，客户自动回复已暂停，待处理任务已保留。请检查并恢复账户余额或凭据，或切换可用的大模型服务配置。";
pub(crate) const LLM_RECOVERY_NOTIFICATION: &str =
    "【系统恢复】大模型服务已恢复可用，系统将继续处理此前保留的客户回复任务。";

fn incident_key(provider_id: &str) -> String {
    format!("{INCIDENT_KIND_LLM_ACCOUNT_UNAVAILABLE}:{provider_id}")
}

fn notification_content(phase: &str) -> Option<&'static str> {
    match phase {
        PHASE_OUTAGE => Some(LLM_OUTAGE_NOTIFICATION),
        PHASE_RECOVERY => Some(LLM_RECOVERY_NOTIFICATION),
        _ => None,
    }
}

fn phase_marker_field(phase: &str) -> Option<&'static str> {
    match phase {
        PHASE_OUTAGE => Some("outage_enqueued_generation"),
        PHASE_RECOVERY => Some("recovery_enqueued_generation"),
        _ => None,
    }
}

fn source_event_id(
    incident_id: ObjectId,
    generation: i64,
    phase: &str,
    recipient_index: usize,
) -> String {
    format!(
        "system-incident:{}:{generation}:{phase}:{recipient_index}",
        incident_id.to_hex()
    )
}

fn parse_source_event_id(value: &str) -> Option<(ObjectId, i64, &str, usize)> {
    let mut parts = value.split(':');
    if parts.next()? != "system-incident" {
        return None;
    }
    let incident_id = ObjectId::parse_str(parts.next()?).ok()?;
    let generation = parts.next()?.parse::<i64>().ok()?;
    let phase = parts.next()?;
    let recipient_index = parts.next()?.parse::<usize>().ok()?;
    if parts.next().is_some() || generation < 1 {
        return None;
    }
    Some((incident_id, generation, phase, recipient_index))
}

fn freeze_recipients(
    deciders: impl IntoIterator<Item = DeciderRef>,
    fallback_account: &str,
) -> Vec<SystemIncidentRecipient> {
    let mut seen = HashSet::new();
    deciders
        .into_iter()
        .filter_map(|decider| {
            let wxid = decider.wxid.trim();
            let account_id = decider
                .account_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(fallback_account);
            if wxid.is_empty() || account_id.is_empty() {
                return None;
            }
            let identity = (account_id.to_string(), wxid.to_string());
            seen.insert(identity.clone())
                .then_some(SystemIncidentRecipient {
                    account_id: identity.0,
                    wxid: identity.1,
                })
        })
        .collect()
}

async fn resolve_recipients(
    state: &AppState,
    workspace_id: &str,
    triggering_account_id: Option<&str>,
) -> AppResult<Vec<SystemIncidentRecipient>> {
    let fallback_account = triggering_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(state.config.default_account_id.trim());
    let deciders = match super::load_user_operation_domain_config(state, workspace_id).await? {
        Some(config) => resolve_ask_human_policy(&config).decider_chain,
        None => Vec::new(),
    };
    Ok(freeze_recipients(deciders, fallback_account))
}

fn recovery_candidate_filter(workspace_id: &str, request_started_at: DateTime) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "status": INCIDENT_STATUS_ACTIVE,
        "last_failure_started_at": { "$lt": request_started_at },
    }
}

/// Record one account-unavailable observation. Concurrent observations converge
/// on one active generation and only increase its occurrence counter.
pub(crate) async fn observe_llm_account_unavailable(
    state: &AppState,
    workspace_id: &str,
    triggering_account_id: Option<&str>,
    provider_id: &str,
    model: &str,
    reason: &str,
    request_started_at: DateTime,
) -> AppResult<()> {
    let key = incident_key(provider_id);
    loop {
        let existing = state
            .db
            .system_incidents()
            .find_one(
                doc! { "workspace_id": workspace_id, "incident_key": &key },
                None,
            )
            .await?;
        if let Some(existing) = existing {
            let incident_id = existing
                .id
                .ok_or_else(|| AppError::External("system incident missing _id".to_string()))?;
            if existing.status == INCIDENT_STATUS_ACTIVE {
                if request_started_at < existing.first_failure_started_at {
                    return Ok(());
                }
                let now = DateTime::now();
                let updated = state
                    .db
                    .system_incidents()
                    .update_one(
                        doc! {
                            "_id": incident_id,
                            "status": INCIDENT_STATUS_ACTIVE,
                            "generation": existing.generation,
                        },
                        doc! {
                            "$set": {
                                "last_observed_at": now,
                                "updated_at": now,
                                "reason": reason,
                            },
                            "$max": { "last_failure_started_at": request_started_at },
                            "$inc": { "occurrence_count": 1i64 },
                        },
                        None,
                    )
                    .await?;
                if updated.matched_count == 1 {
                    materialize_incident_notifications(state, &existing).await?;
                    return Ok(());
                }
                continue;
            }

            // A request that started no later than the successful recovery probe
            // is a late result from the already recovered generation. It must not
            // reopen the outage merely because its response arrived later.
            if existing
                .recovery_probe_started_at
                .is_some_and(|probe| request_started_at <= probe)
            {
                return Ok(());
            }

            let recipients = resolve_recipients(state, workspace_id, triggering_account_id).await?;
            let now = DateTime::now();
            let next_generation = existing.generation.saturating_add(1).max(1);
            let reopened = state
                .db
                .system_incidents()
                .find_one_and_update(
                    doc! {
                        "_id": incident_id,
                        "status": INCIDENT_STATUS_RECOVERED,
                        "generation": existing.generation,
                    },
                    doc! {
                        "$set": {
                            "status": INCIDENT_STATUS_ACTIVE,
                            "generation": next_generation,
                            "provider_id": provider_id,
                            "model": model,
                            "reason": reason,
                            "recipients": mongodb::bson::to_bson(&recipients)?,
                            "occurrence_count": 1i64,
                            "first_failure_started_at": request_started_at,
                            "last_failure_started_at": request_started_at,
                            "first_observed_at": now,
                            "last_observed_at": now,
                            "updated_at": now,
                        },
                        "$unset": {
                            "recovered_at": "",
                            "outage_enqueued_generation": "",
                            "recovery_enqueued_generation": "",
                            "recovery_probe_started_at": "",
                        },
                    },
                    FindOneAndUpdateOptions::builder()
                        .return_document(ReturnDocument::After)
                        .build(),
                )
                .await?;
            if let Some(reopened) = reopened {
                materialize_incident_notifications(state, &reopened).await?;
                return Ok(());
            }
            continue;
        }

        let now = DateTime::now();
        let incident = SystemIncident {
            id: Some(ObjectId::new()),
            workspace_id: workspace_id.to_string(),
            incident_key: key.clone(),
            kind: INCIDENT_KIND_LLM_ACCOUNT_UNAVAILABLE.to_string(),
            status: INCIDENT_STATUS_ACTIVE.to_string(),
            generation: 1,
            provider_id: provider_id.to_string(),
            model: model.to_string(),
            reason: reason.to_string(),
            recipients: resolve_recipients(state, workspace_id, triggering_account_id).await?,
            occurrence_count: 1,
            first_failure_started_at: request_started_at,
            last_failure_started_at: request_started_at,
            outage_enqueued_generation: None,
            recovery_enqueued_generation: None,
            first_observed_at: now,
            last_observed_at: now,
            recovered_at: None,
            recovery_probe_started_at: None,
            created_at: now,
            updated_at: now,
        };
        match state
            .db
            .system_incidents()
            .insert_one(&incident, None)
            .await
        {
            Ok(_) => {
                materialize_incident_notifications(state, &incident).await?;
                return Ok(());
            }
            Err(error) if super::escalation::is_duplicate_key_error(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

/// A successful uncached upstream request is the recovery probe. Only an
/// active incident transitions; cache hits never call this function.
pub(crate) async fn observe_llm_recovery(
    state: &AppState,
    workspace_id: &str,
    request_started_at: DateTime,
) -> AppResult<()> {
    // A successful provider replacement also proves the workspace LLM path is
    // available, so recover every causally older active provider incident.
    let mut cursor = state
        .db
        .system_incidents()
        .find(
            recovery_candidate_filter(workspace_id, request_started_at),
            None,
        )
        .await?;
    let mut recovered_incidents = Vec::new();
    while let Some(candidate) = cursor.try_next().await? {
        let Some(incident_id) = candidate.id else {
            tracing::error!(workspace_id, "active system incident missing _id");
            continue;
        };
        let now = DateTime::now();
        let recovered = state
            .db
            .system_incidents()
            .find_one_and_update(
                doc! {
                    "_id": incident_id,
                    "status": INCIDENT_STATUS_ACTIVE,
                    "generation": candidate.generation,
                    "last_failure_started_at": { "$lt": request_started_at },
                },
                doc! { "$set": {
                    "status": INCIDENT_STATUS_RECOVERED,
                    "recovered_at": now,
                    "recovery_probe_started_at": request_started_at,
                    "updated_at": now,
                } },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?;
        if let Some(recovered) = recovered {
            recovered_incidents.push(recovered);
        }
    }

    // Wake blocked work before notification materialization. If this process
    // exits after the durable recovery CAS but before this update, the missing
    // recovery marker keeps the incident visible to reconcile_notifications,
    // which runs the same idempotent wake-up before retrying materialization.
    if !recovered_incidents.is_empty() {
        wake_provider_blocked_tasks_if_workspace_recovered(state, workspace_id).await?;
    }
    for recovered in recovered_incidents {
        materialize_incident_notifications(state, &recovered).await?;
    }
    Ok(())
}

async fn wake_provider_blocked_tasks_if_workspace_recovered(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<()> {
    let active = state
        .db
        .system_incidents()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "kind": INCIDENT_KIND_LLM_ACCOUNT_UNAVAILABLE,
                "status": INCIDENT_STATUS_ACTIVE,
            },
            None,
        )
        .await?;
    if active > 0 {
        return Ok(());
    }
    let now = DateTime::now();
    state
        .db
        .tasks()
        .update_many(
            doc! {
                "workspace_id": workspace_id,
                "status": "retry",
                "gateway_status": "blocked_provider_unavailable",
                "$or": [
                    { "next_retry_at": { "$gt": now } },
                    { "next_retry_at": { "$exists": false } },
                ],
            },
            doc! { "$set": {
                "next_retry_at": now,
                "updated_at": now,
            } },
            None,
        )
        .await?;
    Ok(())
}

async fn materialize_incident_notifications(
    state: &AppState,
    incident: &SystemIncident,
) -> AppResult<()> {
    materialize_phase(state, incident, PHASE_OUTAGE).await?;
    if incident.status == INCIDENT_STATUS_RECOVERED
        && phase_is_terminal(state, incident, PHASE_OUTAGE).await?
    {
        materialize_phase(state, incident, PHASE_RECOVERY).await?;
    }
    Ok(())
}

async fn materialize_phase(
    state: &AppState,
    incident: &SystemIncident,
    phase: &str,
) -> AppResult<()> {
    let incident_id = incident
        .id
        .ok_or_else(|| AppError::External("system incident missing _id".to_string()))?;
    let content = notification_content(phase).expect("known incident phase");
    for (index, recipient) in incident.recipients.iter().enumerate() {
        let source_event_id = source_event_id(incident_id, incident.generation, phase, index);
        let exists = state
            .db
            .collection_agent_send_outbox()
            .count_documents(
                doc! {
                    "workspace_id": &incident.workspace_id,
                    "account_id": &recipient.account_id,
                    "source_event_id": &source_event_id,
                    "contact_wxid": &recipient.wxid,
                },
                None,
            )
            .await?
            > 0;
        if exists {
            continue;
        }
        let outcome = enqueue(
            state,
            EnqueueRequest {
                workspace_id: incident.workspace_id.clone(),
                account_id: recipient.account_id.clone(),
                contact_wxid: recipient.wxid.clone(),
                run_id: source_event_id.clone(),
                decision_id: None,
                source_event_id,
                source_kind: SOURCE_KIND_SYSTEM_INCIDENT.to_string(),
                content: content.to_string(),
                media_asset_id: None,
                referral_card_id: None,
                max_attempts: 5,
            },
        )
        .await?;
        match outcome {
            EnqueueOutcome::Created { .. } | EnqueueOutcome::IdempotentSkip { .. } => {}
        }
    }
    let marker = phase_marker_field(phase).expect("known incident phase");
    state
        .db
        .system_incidents()
        .update_one(
            doc! { "_id": incident_id, "generation": incident.generation },
            doc! { "$set": { marker: incident.generation } },
            None,
        )
        .await?;
    Ok(())
}

async fn phase_is_terminal(
    state: &AppState,
    incident: &SystemIncident,
    phase: &str,
) -> AppResult<bool> {
    let incident_id = incident
        .id
        .ok_or_else(|| AppError::External("system incident missing _id".to_string()))?;
    for (index, recipient) in incident.recipients.iter().enumerate() {
        let terminal = state
            .db
            .collection_agent_send_outbox()
            .count_documents(
                doc! {
                    "workspace_id": &incident.workspace_id,
                    "account_id": &recipient.account_id,
                    "contact_wxid": &recipient.wxid,
                    "source_kind": SOURCE_KIND_SYSTEM_INCIDENT,
                    "source_event_id": source_event_id(
                        incident_id,
                        incident.generation,
                        phase,
                        index,
                    ),
                    "status": { "$in": vec![
                        "sent",
                        "failed_terminal",
                        "canceled",
                        "delivery_unknown",
                    ] },
                },
                None,
            )
            .await?
            == 1;
        if !terminal {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Recover a crash between incident persistence and Outbox materialization.
pub(crate) async fn reconcile_notifications(state: &AppState) -> AppResult<u64> {
    let mut cursor = state
        .db
        .system_incidents()
        .find(
            doc! { "$or": [
                {
                    "status": INCIDENT_STATUS_ACTIVE,
                    "$expr": { "$ne": ["$outage_enqueued_generation", "$generation"] },
                },
                {
                    "status": INCIDENT_STATUS_RECOVERED,
                    "$expr": { "$ne": ["$recovery_enqueued_generation", "$generation"] },
                },
            ] },
            FindOptions::builder()
                .sort(doc! { "updated_at": 1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut reconciled = 0_u64;
    while let Some(incident) = cursor.try_next().await? {
        let result = async {
            if incident.status == INCIDENT_STATUS_RECOVERED {
                wake_provider_blocked_tasks_if_workspace_recovered(state, &incident.workspace_id)
                    .await?;
            }
            materialize_incident_notifications(state, &incident).await
        }
        .await;

        // Rotate every scanned generation, including failures, so one poison or
        // transiently broken incident cannot permanently starve later rows from
        // the bounded oldest-first batch. The incomplete marker keeps it eligible
        // for a later retry.
        if let Some(incident_id) = incident.id {
            if let Err(error) = state
                .db
                .system_incidents()
                .update_one(
                    doc! { "_id": incident_id, "generation": incident.generation },
                    doc! { "$set": { "updated_at": DateTime::now() } },
                    None,
                )
                .await
            {
                tracing::error!(
                    ?error,
                    incident_id = %incident_id,
                    generation = incident.generation,
                    "rotating system incident reconciliation failed"
                );
            }
        }

        match result {
            Ok(()) => reconciled += 1,
            Err(error) => tracing::error!(
                ?error,
                workspace_id = %incident.workspace_id,
                generation = incident.generation,
                "system incident reconciliation failed"
            ),
        }
    }
    Ok(reconciled)
}

/// Authorize a claimed system notification against its immutable incident
/// generation and recipient snapshot. Called both after claim and immediately
/// before the irreversible remote send boundary.
pub(crate) async fn send_is_authorized(
    state: &AppState,
    entry: &crate::models::OutboxEntry,
) -> AppResult<bool> {
    if entry.source_kind != SOURCE_KIND_SYSTEM_INCIDENT {
        return Ok(true);
    }
    let Some((incident_id, generation, phase, recipient_index)) =
        parse_source_event_id(&entry.source_event_id)
    else {
        return Ok(false);
    };
    if !matches!(phase, PHASE_OUTAGE | PHASE_RECOVERY) {
        return Ok(false);
    }
    let Some(incident) = state
        .db
        .system_incidents()
        .find_one(
            doc! {
                "_id": incident_id,
                "workspace_id": &entry.workspace_id,
                "generation": generation,
            },
            None,
        )
        .await?
    else {
        return Ok(false);
    };
    Ok(notification_identity_is_authorized(
        &incident,
        generation,
        phase,
        recipient_index,
        &entry.account_id,
        &entry.contact_wxid,
        &entry.content,
    ))
}

fn notification_identity_is_authorized(
    incident: &SystemIncident,
    generation: i64,
    phase: &str,
    recipient_index: usize,
    account_id: &str,
    wxid: &str,
    content: &str,
) -> bool {
    if incident.generation != generation {
        return false;
    }
    let Some(recipient) = incident.recipients.get(recipient_index) else {
        return false;
    };
    let phase_authorized = match phase {
        // The outage was durably observed before recovery. Permit it to finish
        // in recovered state so a fast recovery cannot suppress the alert.
        PHASE_OUTAGE => matches!(
            incident.status.as_str(),
            INCIDENT_STATUS_ACTIVE | INCIDENT_STATUS_RECOVERED
        ),
        PHASE_RECOVERY => incident.status == INCIDENT_STATUS_RECOVERED,
        _ => false,
    };
    phase_authorized
        && recipient.account_id == account_id
        && recipient.wxid == wxid
        && notification_content(phase).is_some_and(|expected| expected == content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incident(status: &str) -> SystemIncident {
        let now = DateTime::from_millis(2_000);
        SystemIncident {
            id: Some(ObjectId::new()),
            workspace_id: "ws".into(),
            incident_key: "llm_account_unavailable:p1".into(),
            kind: INCIDENT_KIND_LLM_ACCOUNT_UNAVAILABLE.into(),
            status: status.into(),
            generation: 4,
            provider_id: "p1".into(),
            model: "model".into(),
            reason: "insufficient_balance".into(),
            recipients: vec![SystemIncidentRecipient {
                account_id: "account-a".into(),
                wxid: "leader-a".into(),
            }],
            occurrence_count: 2,
            first_failure_started_at: DateTime::from_millis(1_000),
            last_failure_started_at: DateTime::from_millis(1_500),
            outage_enqueued_generation: Some(4),
            recovery_enqueued_generation: None,
            first_observed_at: now,
            last_observed_at: now,
            recovered_at: None,
            recovery_probe_started_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn templates_are_fixed_and_do_not_echo_sensitive_context() {
        for text in [LLM_OUTAGE_NOTIFICATION, LLM_RECOVERY_NOTIFICATION] {
            assert!(!text.contains("apiKey"));
            assert!(!text.contains("baseUrl"));
            assert!(!text.contains("HTTP"));
            assert!(!text.contains("客户内容"));
        }
    }

    #[test]
    fn source_identity_round_trips_and_rejects_malformed_values() {
        let id = ObjectId::new();
        let source = source_event_id(id, 3, PHASE_OUTAGE, 2);
        assert_eq!(
            parse_source_event_id(&source),
            Some((id, 3, PHASE_OUTAGE, 2))
        );
        assert!(parse_source_event_id("system-incident:bad:1:outage:0").is_none());
        assert!(
            parse_source_event_id(&format!("system-incident:{}:0:outage:0", id.to_hex())).is_none()
        );
    }

    #[test]
    fn recipient_snapshot_deduplicates_and_binds_legacy_accounts() {
        let recipients = freeze_recipients(
            vec![
                DeciderRef {
                    wxid: " leader ".into(),
                    display_name: None,
                    account_id: None,
                },
                DeciderRef {
                    wxid: "leader".into(),
                    display_name: None,
                    account_id: Some("fallback".into()),
                },
                DeciderRef {
                    wxid: "leader".into(),
                    display_name: None,
                    account_id: Some("account-b".into()),
                },
                DeciderRef {
                    wxid: " ".into(),
                    display_name: None,
                    account_id: Some("account-c".into()),
                },
            ],
            "fallback",
        );
        assert_eq!(
            recipients,
            vec![
                SystemIncidentRecipient {
                    account_id: "fallback".into(),
                    wxid: "leader".into(),
                },
                SystemIncidentRecipient {
                    account_id: "account-b".into(),
                    wxid: "leader".into(),
                },
            ]
        );
    }

    #[test]
    fn notification_authorization_fences_generation_destination_and_content() {
        let active = incident(INCIDENT_STATUS_ACTIVE);
        assert!(notification_identity_is_authorized(
            &active,
            4,
            PHASE_OUTAGE,
            0,
            "account-a",
            "leader-a",
            LLM_OUTAGE_NOTIFICATION,
        ));
        assert!(!notification_identity_is_authorized(
            &active,
            3,
            PHASE_OUTAGE,
            0,
            "account-a",
            "leader-a",
            LLM_OUTAGE_NOTIFICATION,
        ));
        assert!(!notification_identity_is_authorized(
            &active,
            4,
            PHASE_OUTAGE,
            0,
            "wrong-account",
            "leader-a",
            LLM_OUTAGE_NOTIFICATION,
        ));
        assert!(!notification_identity_is_authorized(
            &active,
            4,
            PHASE_RECOVERY,
            0,
            "account-a",
            "leader-a",
            LLM_RECOVERY_NOTIFICATION,
        ));
        let recovered = incident(INCIDENT_STATUS_RECOVERED);
        assert!(notification_identity_is_authorized(
            &recovered,
            4,
            PHASE_RECOVERY,
            0,
            "account-a",
            "leader-a",
            LLM_RECOVERY_NOTIFICATION,
        ));
    }

    #[test]
    fn recovery_probe_filter_requires_observed_and_causally_older_failure() {
        let started = DateTime::from_millis(3_000);
        assert_eq!(
            recovery_candidate_filter("ws", started),
            doc! {
                "workspace_id": "ws",
                "status": INCIDENT_STATUS_ACTIVE,
                "last_failure_started_at": { "$lt": started },
            }
        );
    }

    struct ScopedEnv {
        previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl ScopedEnv {
        fn set(values: &[(&'static str, &str)]) -> Self {
            let previous = values
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var_os(key);
                    std::env::set_var(key, value);
                    (*key, previous)
                })
                .collect();
            Self { previous }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (key, value) in self.previous.drain(..).rev() {
                if let Some(value) = value {
                    std::env::set_var(key, value);
                } else {
                    std::env::remove_var(key);
                }
            }
        }
    }

    struct NeverLlm;

    #[async_trait::async_trait]
    impl crate::llm::LlmProvider for NeverLlm {
        async fn generate_json(&self, _system: &str, _user: &str) -> AppResult<serde_json::Value> {
            Err(AppError::External("unused test LLM".to_string()))
        }

        async fn generate_json_with_usage(
            &self,
            _system: &str,
            _user: &str,
        ) -> AppResult<crate::llm::LlmJsonResult> {
            Err(AppError::External("unused test LLM".to_string()))
        }
    }

    /// Real-Mongo regression for the cross-collection incident protocol. Kept
    /// ignored in the default suite because it starts a Docker container.
    #[tokio::test]
    #[serial_test::serial]
    #[ignore = "requires Docker"]
    async fn mongo_incident_concurrency_causality_and_notification_recovery() {
        use std::sync::{atomic::AtomicU64, Arc};

        use testcontainers::runners::AsyncRunner;
        use testcontainers_modules::mongo::Mongo;

        let external_uri = std::env::var("TEST_MONGODB_URI")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let (container, uri) = if let Some(uri) = external_uri {
            (None, uri)
        } else {
            let container = Mongo::default()
                .start()
                .await
                .expect("start Mongo test container");
            let host = container.get_host().await.expect("Mongo host");
            let port = container
                .get_host_port_ipv4(27017)
                .await
                .expect("Mongo port");
            (Some(container), format!("mongodb://{host}:{port}"))
        };
        let database_name = format!("system_incident_{}", uuid::Uuid::new_v4().simple());

        // AppConfig currently has no lightweight test constructor. Keep the
        // process-global mutation inside one serial, synchronous scope and
        // restore every previous value before any async database work starts.
        let env = ScopedEnv::set(&[
            ("MONGODB_URI", &uri),
            ("MONGODB_DATABASE", &database_name),
            ("MCP_API_KEY", "test-mcp-key"),
            ("OPENAI_API_KEY", "test-llm-key"),
            ("DEFAULT_WORKSPACE_ID", "ws-incident"),
            ("DEFAULT_ACCOUNT_ID", "account-a"),
        ]);
        let config = crate::config::AppConfig::from_env().expect("test config");
        drop(env);
        let db = crate::db::Database::connect(&uri, &database_name)
            .await
            .expect("connect Mongo");
        crate::db::migrations::run(&db)
            .await
            .expect("run migrations");
        db.ensure_indexes().await.expect("ensure indexes");

        let mcp = crate::mcp::McpClient::new(
            "http://test-mcp.invalid".to_string(),
            "test-mcp-key".to_string(),
        )
        .expect("test MCP client");
        let state = AppState {
            db,
            mcp,
            llm: Arc::new(NeverLlm),
            llm_registry: None,
            config,
            prompt_pack_version: Arc::new(AtomicU64::new(0)),
            chat_progress_bus: Arc::new(crate::knowledge_task::ChatProgressBus::new()),
            second_reviewer_llm: None,
            chunk_locks: Arc::new(dashmap::DashMap::new()),
            chunk_event_bus: tokio::sync::broadcast::channel(8).0,
            jwt_keys: None,
            auth_rate_limiter: Arc::new(crate::auth::rate_limit::AuthRateLimiter::new(
                60, 100, 100, 100,
            )),
            completeness_cache: Arc::new(dashmap::DashMap::new()),
        };

        let now = DateTime::now();
        state
            .db
            .operation_domain_configs()
            .insert_one(
                crate::models::OperationDomainConfig {
                    id: None,
                    workspace_id: "ws-incident".to_string(),
                    domain: "user_operations".to_string(),
                    name: "test".to_string(),
                    goal: String::new(),
                    methodology: String::new(),
                    workflow: String::new(),
                    tool_policy: String::new(),
                    automation_policy: String::new(),
                    review_policy: String::new(),
                    runtime_parameters: Document::new(),
                    state_machine: Document::new(),
                    status: "active".to_string(),
                    updated_at: now,
                    version: 1,
                    current_version: true,
                    previous_version: None,
                    seeded_by: Some("test".to_string()),
                    principal_decider: Some("leader-a".to_string()),
                    high_risk_escalation_mode: None,
                    ask_human_policy: None,
                    assist_mode_enabled: None,
                },
                None,
            )
            .await
            .expect("seed notification recipient");

        let failure_started_at = DateTime::from_millis(now.timestamp_millis() + 1_000);
        let observations = (0..8).map(|_| {
            observe_llm_account_unavailable(
                &state,
                "ws-incident",
                Some("account-a"),
                "provider-a",
                "model-a",
                "insufficient_balance",
                failure_started_at,
            )
        });
        for result in futures::future::join_all(observations).await {
            result.expect("concurrent outage observation");
        }

        let incident = state
            .db
            .system_incidents()
            .find_one(doc! { "workspace_id": "ws-incident" }, None)
            .await
            .expect("read incident")
            .expect("incident exists");
        assert_eq!(incident.status, INCIDENT_STATUS_ACTIVE);
        assert_eq!(incident.generation, 1);
        assert_eq!(incident.occurrence_count, 8);
        assert_eq!(incident.recipients.len(), 1);

        let outbox = state.db.collection_agent_send_outbox();
        assert_eq!(
            outbox
                .count_documents(
                    doc! {
                        "workspace_id": "ws-incident",
                        "source_kind": SOURCE_KIND_SYSTEM_INCIDENT,
                    },
                    None,
                )
                .await
                .expect("count outage notifications"),
            1,
            "concurrent observers must converge to one outage notification"
        );

        observe_llm_recovery(
            &state,
            "ws-incident",
            DateTime::from_millis(failure_started_at.timestamp_millis() - 1),
        )
        .await
        .expect("late old success observation");
        assert_eq!(
            state
                .db
                .system_incidents()
                .find_one(doc! { "workspace_id": "ws-incident" }, None)
                .await
                .unwrap()
                .unwrap()
                .status,
            INCIDENT_STATUS_ACTIVE,
            "a causally older success must not recover the outage"
        );

        let blocked_task_id = ObjectId::new();
        let blocked_until = DateTime::from_millis(now.timestamp_millis() + 300_000);
        state
            .db
            .tasks()
            .clone_with_type::<Document>()
            .insert_one(
                doc! {
                    "_id": blocked_task_id,
                    "workspace_id": "ws-incident",
                    "status": "retry",
                    "gateway_status": "blocked_provider_unavailable",
                    "next_retry_at": blocked_until,
                    "updated_at": now,
                },
                None,
            )
            .await
            .expect("seed blocked provider task");

        observe_llm_recovery(
            &state,
            "ws-incident",
            DateTime::from_millis(failure_started_at.timestamp_millis() + 1),
        )
        .await
        .expect("causally newer recovery");
        let recovered = state
            .db
            .system_incidents()
            .find_one(doc! { "workspace_id": "ws-incident" }, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.status, INCIDENT_STATUS_RECOVERED);
        let woken = state
            .db
            .tasks()
            .clone_with_type::<Document>()
            .find_one(doc! { "_id": blocked_task_id }, None)
            .await
            .unwrap()
            .unwrap();
        assert!(
            woken.get_datetime("next_retry_at").unwrap() < &blocked_until,
            "successful recovery must wake provider-blocked tasks immediately"
        );
        assert_eq!(
            outbox
                .count_documents(
                    doc! {
                        "workspace_id": "ws-incident",
                        "source_kind": SOURCE_KIND_SYSTEM_INCIDENT,
                    },
                    None,
                )
                .await
                .unwrap(),
            1,
            "recovery notification waits for outage notification terminal truth"
        );

        outbox
            .update_one(
                doc! {
                    "workspace_id": "ws-incident",
                    "content": LLM_OUTAGE_NOTIFICATION,
                },
                doc! { "$set": {
                    "status": "sent",
                    "sent_at": DateTime::now(),
                    "updated_at": DateTime::now(),
                } },
                None,
            )
            .await
            .expect("settle outage notification");
        reconcile_notifications(&state)
            .await
            .expect("reconcile recovery notification");
        assert_eq!(
            outbox
                .count_documents(
                    doc! {
                        "workspace_id": "ws-incident",
                        "source_kind": SOURCE_KIND_SYSTEM_INCIDENT,
                    },
                    None,
                )
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            outbox
                .count_documents(
                    doc! {
                        "workspace_id": "ws-incident",
                        "content": LLM_RECOVERY_NOTIFICATION,
                        "status": "pending",
                    },
                    None,
                )
                .await
                .unwrap(),
            1
        );

        // Simulate a crash immediately after the durable recovery CAS: the
        // recovery marker is missing and a provider-blocked task still points
        // into the future. Reconciliation must replay the wake-up idempotently.
        state
            .db
            .tasks()
            .clone_with_type::<Document>()
            .update_one(
                doc! { "_id": blocked_task_id },
                doc! { "$set": { "next_retry_at": blocked_until } },
                None,
            )
            .await
            .unwrap();
        state
            .db
            .system_incidents()
            .update_one(
                doc! { "_id": recovered.id.unwrap() },
                doc! { "$unset": { "recovery_enqueued_generation": "" } },
                None,
            )
            .await
            .unwrap();
        reconcile_notifications(&state)
            .await
            .expect("reconcile crash-window task wake-up");
        let rewoken = state
            .db
            .tasks()
            .clone_with_type::<Document>()
            .find_one(doc! { "_id": blocked_task_id }, None)
            .await
            .unwrap()
            .unwrap();
        assert!(
            rewoken.get_datetime("next_retry_at").unwrap() < &blocked_until,
            "reconciliation must close the recovery-CAS/task-wake crash window"
        );

        state.db.raw().drop(None).await.expect("drop test database");
        drop(container);
    }
}

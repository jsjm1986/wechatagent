//! Tenant-scoped appointment request and lifecycle endpoints.
//!
//! Conversation Agents may materialize only `requested` rows through the atomic turn committer.
//! Status transitions are a separate authority surface. The public HTTP handlers always derive
//! provenance from the authenticated admin session; principal and trusted-tool actors are reserved
//! for verified internal callers.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument, UpdateOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::{Appointment, AuthorityObservation, APPOINTMENT_STATUSES},
};

use super::AppState;

const MAX_LIST_LIMIT: i64 = 200;
const DEFAULT_LIST_LIMIT: i64 = 50;
const MAX_REQUEST_TEXT_CHARS: usize = 2_000;
const MAX_LOCATION_CHARS: usize = 500;
const MAX_IDEMPOTENCY_KEY_CHARS: usize = 300;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAppointmentRequest {
    pub account_id: String,
    pub contact_wxid: String,
    pub request_text: String,
    #[serde(default)]
    pub requested_start: Option<String>,
    #[serde(default)]
    pub requested_end: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAppointmentRequest {
    pub expected_version: i64,
    #[serde(default)]
    pub request_text: Option<String>,
    #[serde(default)]
    pub requested_start: Option<String>,
    #[serde(default)]
    pub requested_end: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransitionAppointmentRequest {
    pub status: String,
    pub expected_version: i64,
    #[serde(default)]
    pub confirmed_start: Option<String>,
    #[serde(default)]
    pub confirmed_end: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListAppointmentsQuery {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub contact_wxid: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppointmentMutationActor {
    Admin { source_id: String },
    Principal { source_id: String },
    TrustedTool { source_id: String },
    Agent { source_id: String },
}

impl AppointmentMutationActor {
    fn confirmation_provenance(&self) -> AppResult<(&'static str, &str)> {
        match self {
            Self::Admin { source_id } => Ok(("admin", source_id)),
            Self::Principal { source_id } => Ok(("principal", source_id)),
            Self::TrustedTool { source_id } => Ok(("trusted_tool", source_id)),
            Self::Agent { .. } => Err(AppError::Forbidden(
                "agent_may_only_create_appointment_requests".to_string(),
            )),
        }
    }

    fn assert_transition_authority(&self) -> AppResult<()> {
        if matches!(self, Self::Agent { .. }) {
            return Err(AppError::Forbidden(
                "agent_may_only_create_appointment_requests".to_string(),
            ));
        }
        Ok(())
    }
}

fn appointment_scope_filter(id: ObjectId, workspace_id: &str) -> Document {
    doc! { "_id": id, "workspace_id": workspace_id }
}

fn valid_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        (
            "requested",
            "pending_confirmation" | "confirmed" | "cancelled" | "expired"
        ) | (
            "pending_confirmation",
            "confirmed" | "reschedule_requested" | "cancelled" | "expired"
        ) | (
            "confirmed",
            "reschedule_requested" | "cancelled" | "completed" | "no_show"
        ) | (
            "reschedule_requested",
            "pending_confirmation" | "confirmed" | "cancelled" | "expired"
        )
    )
}

fn parse_optional_datetime(value: Option<&str>, field: &str) -> AppResult<Option<DateTime>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            DateTime::parse_rfc3339_str(value)
                .map_err(|_| AppError::BadRequest(format!("{field} must be RFC3339")))
        })
        .transpose()
}

fn validate_time_window(
    start: Option<DateTime>,
    end: Option<DateTime>,
    prefix: &str,
) -> AppResult<()> {
    if let (Some(start), Some(end)) = (start, end) {
        if end.timestamp_millis() <= start.timestamp_millis() {
            return Err(AppError::BadRequest(format!(
                "{prefix}End must be later than {prefix}Start"
            )));
        }
    }
    Ok(())
}

fn validate_requested_update_window(
    current_start: Option<DateTime>,
    current_end: Option<DateTime>,
    updated_start: Option<DateTime>,
    updated_end: Option<DateTime>,
) -> AppResult<()> {
    validate_time_window(
        updated_start.or(current_start),
        updated_end.or(current_end),
        "requested",
    )
}

fn required_trimmed(value: &str, field: &str, max_chars: usize) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.chars().count() > max_chars {
        return Err(AppError::BadRequest(format!(
            "{field} exceeds {max_chars} characters"
        )));
    }
    Ok(value.to_string())
}

fn optional_trimmed(
    value: Option<String>,
    field: &str,
    max_chars: usize,
) -> AppResult<Option<String>> {
    value
        .map(|value| required_trimmed(&value, field, max_chars))
        .transpose()
}

fn appointment_json(appointment: &Appointment) -> Value {
    json!({
        "id": appointment.id.map(|id| id.to_hex()),
        "workspaceId": appointment.workspace_id,
        "accountId": appointment.account_id,
        "contactWxid": appointment.contact_wxid,
        "idempotencyKey": appointment.idempotency_key,
        "status": appointment.status,
        "requestText": appointment.request_text,
        "requestedStart": appointment.requested_start.and_then(crate::models::dt_to_string),
        "requestedEnd": appointment.requested_end.and_then(crate::models::dt_to_string),
        "confirmedStart": appointment.confirmed_start.and_then(crate::models::dt_to_string),
        "confirmedEnd": appointment.confirmed_end.and_then(crate::models::dt_to_string),
        "location": appointment.location,
        "confirmationSourceType": appointment.confirmation_source_type,
        "confirmationSourceId": appointment.confirmation_source_id,
        "sourceTurnId": appointment.source_turn_id,
        "version": appointment.version,
        "createdAt": crate::models::dt_to_string(appointment.created_at),
        "updatedAt": crate::models::dt_to_string(appointment.updated_at),
    })
}

pub async fn create_appointment(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<CreateAppointmentRequest>,
) -> AppResult<Json<Value>> {
    let account_id = required_trimmed(&body.account_id, "accountId", 200)?;
    let contact_wxid = required_trimmed(&body.contact_wxid, "contactWxid", 300)?;
    let request_text = required_trimmed(&body.request_text, "requestText", MAX_REQUEST_TEXT_CHARS)?;
    let requested_start =
        parse_optional_datetime(body.requested_start.as_deref(), "requestedStart")?;
    let requested_end = parse_optional_datetime(body.requested_end.as_deref(), "requestedEnd")?;
    validate_time_window(requested_start, requested_end, "requested")?;
    let location = optional_trimmed(body.location, "location", MAX_LOCATION_CHARS)?;
    let id = ObjectId::new();
    let idempotency_key = match body.idempotency_key {
        Some(value) => required_trimmed(&value, "idempotencyKey", MAX_IDEMPOTENCY_KEY_CHARS)?,
        None => format!("admin-appointment-request:v1:{}", id.to_hex()),
    };

    let contact_exists = state
        .db
        .contacts()
        .count_documents(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "wxid": &contact_wxid,
            },
            None,
        )
        .await?
        > 0;
    if !contact_exists {
        return Err(AppError::NotFound("contact not found".to_string()));
    }

    let now = DateTime::now();
    let appointment = Appointment {
        id: Some(id),
        workspace_id: admin.current_workspace.clone(),
        account_id: account_id.clone(),
        contact_wxid: contact_wxid.clone(),
        idempotency_key: idempotency_key.clone(),
        status: "requested".to_string(),
        request_text,
        requested_start,
        requested_end,
        confirmed_start: None,
        confirmed_end: None,
        location,
        confirmation_source_type: None,
        confirmation_source_id: None,
        source_turn_id: format!("admin:{}:{}", admin.user_id, id.to_hex()),
        version: 1,
        created_at: now,
        updated_at: now,
    };
    let document = mongodb::bson::to_document(&appointment)?;
    let result = state
        .db
        .appointments()
        .clone_with_type::<Document>()
        .update_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "idempotency_key": &idempotency_key,
            },
            doc! { "$setOnInsert": document },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    let stored = state
        .db
        .appointments()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "idempotency_key": &idempotency_key,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::External("appointment upsert lost its row".to_string()))?;
    if stored.contact_wxid != contact_wxid {
        return Err(AppError::Conflict(
            "appointment_idempotency_identity_conflict".to_string(),
        ));
    }
    Ok(Json(json!({
        "created": result.upserted_id.is_some(),
        "appointment": appointment_json(&stored),
    })))
}

pub async fn list_appointments(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ListAppointmentsQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    if let Some(account_id) = query
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        filter.insert("account_id", account_id);
    }
    if let Some(contact_wxid) = query
        .contact_wxid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        filter.insert("contact_wxid", contact_wxid);
    }
    if let Some(status) = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !APPOINTMENT_STATUSES.contains(&status) {
            return Err(AppError::BadRequest(
                "invalid appointment status".to_string(),
            ));
        }
        filter.insert("status", status);
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let mut cursor = state
        .db
        .appointments()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1i32, "_id": -1i32 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(appointment) = cursor.try_next().await? {
        items.push(appointment_json(&appointment));
    }
    Ok(Json(json!({ "items": items })))
}

pub async fn get_appointment(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let id = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("invalid appointment id".to_string()))?;
    let appointment = state
        .db
        .appointments()
        .find_one(appointment_scope_filter(id, &admin.current_workspace), None)
        .await?
        .ok_or_else(|| AppError::NotFound("appointment not found".to_string()))?;
    Ok(Json(
        json!({ "appointment": appointment_json(&appointment) }),
    ))
}

pub async fn update_appointment(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(body): Json<UpdateAppointmentRequest>,
) -> AppResult<Json<Value>> {
    if body.expected_version < 1 {
        return Err(AppError::BadRequest(
            "positive expectedVersion is required".to_string(),
        ));
    }
    let id = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("invalid appointment id".to_string()))?;
    let current = state
        .db
        .appointments()
        .find_one(appointment_scope_filter(id, &admin.current_workspace), None)
        .await?
        .ok_or_else(|| AppError::NotFound("appointment not found".to_string()))?;
    if !matches!(
        current.status.as_str(),
        "requested" | "pending_confirmation" | "reschedule_requested"
    ) {
        return Err(AppError::Conflict(
            "appointment_request_fields_are_locked".to_string(),
        ));
    }

    let requested_start_update = body
        .requested_start
        .as_deref()
        .map(|value| {
            parse_optional_datetime(Some(value), "requestedStart")?
                .ok_or_else(|| AppError::BadRequest("requestedStart is required".to_string()))
        })
        .transpose()?;
    let requested_end_update = body
        .requested_end
        .as_deref()
        .map(|value| {
            parse_optional_datetime(Some(value), "requestedEnd")?
                .ok_or_else(|| AppError::BadRequest("requestedEnd is required".to_string()))
        })
        .transpose()?;
    if requested_start_update.is_some() || requested_end_update.is_some() {
        validate_requested_update_window(
            current.requested_start,
            current.requested_end,
            requested_start_update,
            requested_end_update,
        )?;
    }

    let mut set = doc! {
        "updated_at": DateTime::now(),
        "version": body.expected_version + 1,
    };
    if let Some(request_text) = body.request_text {
        set.insert(
            "request_text",
            required_trimmed(&request_text, "requestText", MAX_REQUEST_TEXT_CHARS)?,
        );
    }
    if let Some(requested_start) = requested_start_update {
        set.insert("requested_start", requested_start);
    }
    if let Some(requested_end) = requested_end_update {
        set.insert("requested_end", requested_end);
    }
    if let Some(location) = body.location {
        set.insert(
            "location",
            required_trimmed(&location, "location", MAX_LOCATION_CHARS)?,
        );
    }
    if set.len() == 2 {
        return Err(AppError::BadRequest(
            "at least one appointment field is required".to_string(),
        ));
    }
    let updated = state
        .db
        .appointments()
        .find_one_and_update(
            doc! {
                "_id": id,
                "workspace_id": &admin.current_workspace,
                "version": body.expected_version,
                "status": &current.status,
            },
            doc! { "$set": set },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::Conflict("appointment_version_conflict".to_string()))?;
    Ok(Json(json!({ "appointment": appointment_json(&updated) })))
}

pub async fn transition_appointment(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(body): Json<TransitionAppointmentRequest>,
) -> AppResult<Json<Value>> {
    let actor = AppointmentMutationActor::Admin {
        source_id: admin.user_id.clone(),
    };
    actor.assert_transition_authority()?;
    if body.expected_version < 1 {
        return Err(AppError::BadRequest(
            "positive expectedVersion is required".to_string(),
        ));
    }
    let target = body.status.trim();
    if !APPOINTMENT_STATUSES.contains(&target) || target == "requested" {
        return Err(AppError::BadRequest(
            "invalid appointment transition target".to_string(),
        ));
    }
    let id = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("invalid appointment id".to_string()))?;
    let current = state
        .db
        .appointments()
        .find_one(appointment_scope_filter(id, &admin.current_workspace), None)
        .await?
        .ok_or_else(|| AppError::NotFound("appointment not found".to_string()))?;
    if !valid_transition(&current.status, target) {
        return Err(AppError::Conflict(format!(
            "invalid_appointment_transition:{}->{}",
            current.status, target
        )));
    }

    let mut set = doc! {
        "status": target,
        "version": body.expected_version + 1,
        "updated_at": DateTime::now(),
    };
    let mut unset = Document::new();
    let mut confirmation_provenance = None;
    if target == "confirmed" {
        let confirmed_start =
            parse_optional_datetime(body.confirmed_start.as_deref(), "confirmedStart")?
                .ok_or_else(|| AppError::BadRequest("confirmedStart is required".to_string()))?;
        let confirmed_end = parse_optional_datetime(body.confirmed_end.as_deref(), "confirmedEnd")?
            .ok_or_else(|| AppError::BadRequest("confirmedEnd is required".to_string()))?;
        validate_time_window(Some(confirmed_start), Some(confirmed_end), "confirmed")?;
        let (source_type, source_id) = actor.confirmation_provenance()?;
        confirmation_provenance = Some((source_type.to_string(), source_id.to_string()));
        set.insert("confirmed_start", confirmed_start);
        set.insert("confirmed_end", confirmed_end);
        set.insert("confirmation_source_type", source_type);
        set.insert("confirmation_source_id", source_id);
    } else if matches!(target, "pending_confirmation" | "reschedule_requested") {
        unset.insert("confirmed_start", "");
        unset.insert("confirmed_end", "");
        unset.insert("confirmation_source_type", "");
        unset.insert("confirmation_source_id", "");
    }
    if let Some(location) = body.location {
        set.insert(
            "location",
            required_trimmed(&location, "location", MAX_LOCATION_CHARS)?,
        );
    }
    let mut update = doc! { "$set": set };
    if !unset.is_empty() {
        update.insert("$unset", unset);
    }
    let filter = doc! {
        "_id": id,
        "workspace_id": &admin.current_workspace,
        "version": body.expected_version,
        "status": &current.status,
    };
    let update_options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();
    let updated = if let Some((source_type, source_id)) = confirmation_provenance {
        // Confirmation is an authority-bearing transition. Keep the appointment CAS and its
        // provenance observation in one transaction so a crash cannot leave a confirmed row that
        // is absent from the authority bundle's source ledger.
        let mut session = state.db.client().start_session(None).await?;
        session.start_transaction(None).await?;
        let transaction_result: AppResult<Appointment> = async {
            let updated = state
                .db
                .appointments()
                .find_one_and_update_with_session(
                    filter,
                    update,
                    update_options,
                    &mut session,
                )
                .await?
                .ok_or_else(|| AppError::Conflict("appointment_version_conflict".to_string()))?;
            let appointment_id = updated.id.ok_or_else(|| {
                AppError::Conflict("confirmed appointment missing id".to_string())
            })?;
            crate::agent::authority::record_authority_observation_with_session(
                &state.db,
                &mut session,
                AuthorityObservation {
                    id: Some(ObjectId::new()),
                    workspace_id: updated.workspace_id.clone(),
                    account_id: updated.account_id.clone(),
                    contact_wxid: updated.contact_wxid.clone(),
                    source_type: source_type.clone(),
                    source_id: format!(
                        "appointment:{}:v{}",
                        appointment_id.to_hex(),
                        updated.version
                    ),
                    subject: "business".to_string(),
                    content: format!(
                        "appointment_id={}; status=confirmed; start={:?}; end={:?}; location={:?}; confirmation_source_type={}; confirmation_source_id={}",
                        appointment_id.to_hex(),
                        updated.confirmed_start,
                        updated.confirmed_end,
                        updated.location,
                        source_type,
                        source_id,
                    ),
                    authority_boundary: "Authorizes only this exact confirmed appointment status, time range, and recorded location for this contact; it does not authorize unrelated availability, prices, outcomes, or services.".to_string(),
                    valid_from: Some(updated.updated_at),
                    valid_until: None,
                    status: "active".to_string(),
                    superseded_by: None,
                    source_run_id: None,
                    created_at: updated.updated_at,
                    updated_at: updated.updated_at,
                },
            )
            .await?;
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
        updated
    } else {
        state
            .db
            .appointments()
            .find_one_and_update(filter, update, update_options)
            .await?
            .ok_or_else(|| AppError::Conflict("appointment_version_conflict".to_string()))?
    };
    Ok(Json(json!({ "appointment": appointment_json(&updated) })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_is_closed_and_terminal_states_cannot_reopen() {
        assert!(valid_transition("requested", "confirmed"));
        assert!(valid_transition("confirmed", "completed"));
        assert!(valid_transition("confirmed", "reschedule_requested"));
        for terminal in ["completed", "no_show", "cancelled", "expired"] {
            for target in APPOINTMENT_STATUSES {
                assert!(!valid_transition(terminal, target));
            }
        }
        assert!(!valid_transition("requested", "completed"));
        assert!(!valid_transition("pending_confirmation", "no_show"));
    }

    #[test]
    fn agent_cannot_transition_or_confirm_an_appointment() {
        let actor = AppointmentMutationActor::Agent {
            source_id: "turn-1".to_string(),
        };
        assert!(matches!(
            actor.assert_transition_authority(),
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            actor.confirmation_provenance(),
            Err(AppError::Forbidden(_))
        ));
    }

    #[test]
    fn confirmation_provenance_accepts_only_authoritative_actors() {
        let cases = [
            (
                AppointmentMutationActor::Admin {
                    source_id: "admin-1".to_string(),
                },
                "admin",
            ),
            (
                AppointmentMutationActor::Principal {
                    source_id: "principal-1".to_string(),
                },
                "principal",
            ),
            (
                AppointmentMutationActor::TrustedTool {
                    source_id: "receipt-1".to_string(),
                },
                "trusted_tool",
            ),
        ];
        for (actor, expected) in cases {
            let (source_type, source_id) = actor.confirmation_provenance().unwrap();
            assert_eq!(source_type, expected);
            assert!(!source_id.is_empty());
        }
    }

    #[test]
    fn item_lookup_filter_is_always_tenant_scoped() {
        let id = ObjectId::new();
        assert_eq!(
            appointment_scope_filter(id, "workspace-a"),
            doc! { "_id": id, "workspace_id": "workspace-a" }
        );
    }

    #[test]
    fn confirmation_requires_a_real_time_window() {
        let start = DateTime::parse_rfc3339_str("2026-08-20T10:00:00+08:00").unwrap();
        let end = DateTime::parse_rfc3339_str("2026-08-20T09:30:00+08:00").unwrap();
        assert!(validate_time_window(Some(start), Some(end), "confirmed").is_err());
    }

    #[test]
    fn one_sided_requested_time_update_validates_against_the_stored_bound() {
        let start = DateTime::parse_rfc3339_str("2026-09-01T10:00:00+08:00").unwrap();
        let end = DateTime::parse_rfc3339_str("2026-09-01T11:00:00+08:00").unwrap();
        let later = DateTime::parse_rfc3339_str("2026-09-01T12:00:00+08:00").unwrap();
        let earlier = DateTime::parse_rfc3339_str("2026-09-01T09:00:00+08:00").unwrap();

        assert!(
            validate_requested_update_window(Some(start), Some(end), Some(later), None).is_err()
        );
        assert!(
            validate_requested_update_window(Some(start), Some(end), None, Some(earlier)).is_err()
        );
        assert!(
            validate_requested_update_window(Some(start), Some(end), Some(earlier), None).is_ok()
        );
    }

    #[test]
    fn appointment_json_matches_contract_fixture() {
        let appointment = Appointment {
            id: Some(ObjectId::parse_str("64b64c76f1f0a3a72c2f9a10").unwrap()),
            workspace_id: "workspace-contract".to_string(),
            account_id: "account-contract".to_string(),
            contact_wxid: "wxid-contract".to_string(),
            idempotency_key: "appointment-request:v1:turn-contract".to_string(),
            status: "confirmed".to_string(),
            request_text: "客户希望周六上午到院面诊".to_string(),
            requested_start: Some(
                DateTime::parse_rfc3339_str("2026-08-22T10:00:00+08:00").unwrap(),
            ),
            requested_end: Some(DateTime::parse_rfc3339_str("2026-08-22T11:00:00+08:00").unwrap()),
            confirmed_start: Some(
                DateTime::parse_rfc3339_str("2026-08-22T10:30:00+08:00").unwrap(),
            ),
            confirmed_end: Some(DateTime::parse_rfc3339_str("2026-08-22T11:30:00+08:00").unwrap()),
            location: Some("院区一层咨询室".to_string()),
            confirmation_source_type: Some("admin".to_string()),
            confirmation_source_id: Some("admin-contract".to_string()),
            source_turn_id: "turn-contract".to_string(),
            version: 3,
            created_at: DateTime::parse_rfc3339_str("2026-08-15T09:00:00+08:00").unwrap(),
            updated_at: DateTime::parse_rfc3339_str("2026-08-15T09:30:00+08:00").unwrap(),
        };

        crate::routes::contract_snapshot::assert_contract_fixture(
            "appointment",
            appointment_json(&appointment),
        );
    }
}

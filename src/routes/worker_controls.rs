//! Authenticated control plane for supervised background-worker circuits.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde_json::{json, Value};

use super::AppState;
use crate::{auth::AuthenticatedAdmin, error::AppResult};

fn is_system_operator(username: &str, allowed_usernames: &[String]) -> bool {
    allowed_usernames.iter().any(|allowed| allowed == username)
}

fn require_system_operator(state: &AppState, admin: &AuthenticatedAdmin) -> AppResult<()> {
    if is_system_operator(&admin.username, &state.config.system_operator_usernames) {
        Ok(())
    } else {
        Err(crate::error::AppError::Forbidden(
            "system_operator_required".to_string(),
        ))
    }
}

fn optional_datetime_millis(row: &mongodb::bson::Document, key: &str) -> Option<i64> {
    row.get_datetime(key)
        .ok()
        .map(|value| value.timestamp_millis())
}

/// Explicit public projection. Probe tokens, panic payloads and raw Mongo fields
/// are process-internal and must never cross the control-plane boundary.
fn worker_control_json(row: &mongodb::bson::Document) -> Value {
    json!({
        "workerName": row.get_str("worker_name").unwrap_or_default(),
        "status": row.get_str("status").unwrap_or("closed"),
        "rapidPanicCount": row.get_i64("rapid_panic_count").unwrap_or_default(),
        "circuitGeneration": row.get_i64("circuit_generation").unwrap_or_default(),
        "hasPanicDiagnostic": row.get_str("last_panic").is_ok(),
        "openedAtMs": optional_datetime_millis(row, "opened_at"),
        "updatedAtMs": optional_datetime_millis(row, "updated_at"),
        "recoveredAtMs": optional_datetime_millis(row, "recovered_at"),
        "resumeRequestedAtMs": optional_datetime_millis(row, "resume_requested_at"),
        "resumeRequestedBy": row.get_str("resume_requested_by").ok(),
    })
}

pub(super) async fn list_worker_controls(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    require_system_operator(&state, &admin)?;
    let mut cursor = state
        .db
        .background_worker_controls()
        .find(
            doc! {},
            mongodb::options::FindOptions::builder()
                .sort(doc! { "worker_name": 1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        items.push(worker_control_json(&row));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn resume_worker_control(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(worker): Path<String>,
) -> AppResult<Json<Value>> {
    require_system_operator(&state, &admin)?;
    let resumed =
        crate::supervisor::resume_worker_circuit(&state, worker.trim(), &admin.username).await?;
    Ok(Json(json!({ "worker": worker, "resumed": resumed })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{doc, DateTime};

    #[test]
    fn operator_acl_is_exact_and_empty_is_fail_closed() {
        assert!(!is_system_operator("admin", &[]));
        assert!(is_system_operator("admin", &["admin".to_string()]));
        assert!(!is_system_operator("Admin", &["admin".to_string()]));
    }

    #[test]
    fn public_projection_excludes_tokens_and_panic_payload() {
        let value = worker_control_json(&doc! {
            "worker_name": "task_worker",
            "status": "probing",
            "rapid_panic_count": 5_i64,
            "circuit_generation": 2_i64,
            "last_panic": "secret panic detail",
            "probe_token": "secret-token",
            "probe_locked_until": DateTime::from_millis(1234),
            "updated_at": DateTime::from_millis(5678),
        });
        assert_eq!(value["workerName"], "task_worker");
        assert_eq!(value["hasPanicDiagnostic"], true);
        assert_eq!(value["updatedAtMs"], 5678);
        let encoded = serde_json::to_string(&value).unwrap();
        assert!(!encoded.contains("secret-token"));
        assert!(!encoded.contains("secret panic detail"));
        assert!(!encoded.contains("probeToken"));
    }

    #[test]
    fn worker_control_json_matches_contract_fixture() {
        let row = doc! {
            "worker_name": "task_worker",
            "status": "open",
            "rapid_panic_count": 5_i64,
            "circuit_generation": 2_i64,
            "last_panic": "redacted by projection",
            "opened_at": DateTime::from_millis(1_700_000_000_000),
            "updated_at": DateTime::from_millis(1_700_000_000_100),
            "recovered_at": DateTime::from_millis(1_700_000_000_200),
            "resume_requested_at": DateTime::from_millis(1_700_000_000_300),
            "resume_requested_by": "system-operator",
        };
        crate::routes::contract_snapshot::assert_contract_fixture(
            "worker_control",
            worker_control_json(&row),
        );
    }
}

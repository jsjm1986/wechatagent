//! Appointment lifecycle and authority boundary integration tests.
//!
//! These tests use the real Mongo handlers and are ignored by default because they require the
//! testcontainers environment. Run with:
//! `cargo test --test appointments_integration -- --ignored`.
#![cfg(test)]

mod common;

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use mongodb::bson::{doc, Document};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::routes::appointments::{
    create_appointment, get_appointment, transition_appointment, update_appointment,
    CreateAppointmentRequest, TransitionAppointmentRequest, UpdateAppointmentRequest,
};

use crate::common::TestApp;

fn test_admin(workspace_id: &str, user_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: user_id.to_string(),
        username: user_id.to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

async fn seed_contact(app: &TestApp, workspace_id: &str, account_id: &str, wxid: &str) {
    app.state
        .db
        .raw()
        .collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": wxid,
            },
            None,
        )
        .await
        .expect("seed contact");
}

async fn create_requested(
    app: &TestApp,
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
    idempotency_key: &str,
) -> String {
    let response = create_appointment(
        State(app.state.clone()),
        Extension(test_admin(workspace_id, "appointment_admin")),
        Json(CreateAppointmentRequest {
            account_id: account_id.to_string(),
            contact_wxid: wxid.to_string(),
            request_text: "客户希望安排面诊".to_string(),
            requested_start: Some("2026-09-01T10:00:00+08:00".to_string()),
            requested_end: Some("2026-09-01T11:00:00+08:00".to_string()),
            location: Some("院区 A".to_string()),
            idempotency_key: Some(idempotency_key.to_string()),
        }),
    )
    .await
    .expect("create requested appointment")
    .0;
    assert_eq!(response["appointment"]["status"], "requested");
    assert_eq!(response["appointment"]["version"], 1);
    response["appointment"]["id"]
        .as_str()
        .expect("appointment id")
        .to_string()
}

#[tokio::test]
#[ignore]
async fn appointment_lookup_is_workspace_scoped() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let wxid = "wx_appointment_scope";
    seed_contact(&app, &workspace_id, &account_id, wxid).await;
    let id = create_requested(
        &app,
        &workspace_id,
        &account_id,
        wxid,
        "appointment-scope-v1",
    )
    .await;

    let result = get_appointment(
        State(app.state.clone()),
        Extension(test_admin("other_workspace", "other_admin")),
        Path(id),
    )
    .await;
    assert!(matches!(result, Err(AppError::NotFound(_))));

    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn appointment_updates_enforce_window_cas_transition_and_admin_provenance() {
    // Confirmation records an authority observation in the same Mongo transaction as the
    // appointment CAS. Use the replica-set fixture so the test exercises the production
    // transaction contract instead of failing on standalone Mongo's retryable-write limit.
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let wxid = "wx_appointment_lifecycle";
    let admin = test_admin(&workspace_id, "confirming_admin");
    seed_contact(&app, &workspace_id, &account_id, wxid).await;
    let id = create_requested(
        &app,
        &workspace_id,
        &account_id,
        wxid,
        "appointment-lifecycle-v1",
    )
    .await;

    let reversed_window = update_appointment(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(id.clone()),
        Json(UpdateAppointmentRequest {
            expected_version: 1,
            request_text: None,
            requested_start: Some("2026-09-01T12:00:00+08:00".to_string()),
            requested_end: None,
            location: None,
        }),
    )
    .await;
    assert!(matches!(reversed_window, Err(AppError::BadRequest(_))));

    let updated = update_appointment(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(id.clone()),
        Json(UpdateAppointmentRequest {
            expected_version: 1,
            request_text: Some("客户希望先做面诊评估".to_string()),
            requested_start: None,
            requested_end: None,
            location: None,
        }),
    )
    .await
    .expect("valid CAS update")
    .0;
    assert_eq!(updated["appointment"]["version"], 2);

    let stale = update_appointment(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(id.clone()),
        Json(UpdateAppointmentRequest {
            expected_version: 1,
            request_text: Some("stale write".to_string()),
            requested_start: None,
            requested_end: None,
            location: None,
        }),
    )
    .await;
    assert!(matches!(stale, Err(AppError::Conflict(_))));

    let invalid_transition = transition_appointment(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(id.clone()),
        Json(TransitionAppointmentRequest {
            status: "completed".to_string(),
            expected_version: 2,
            confirmed_start: None,
            confirmed_end: None,
            location: None,
        }),
    )
    .await;
    assert!(matches!(invalid_transition, Err(AppError::Conflict(_))));

    let confirmed = transition_appointment(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(id),
        Json(TransitionAppointmentRequest {
            status: "confirmed".to_string(),
            expected_version: 2,
            confirmed_start: Some("2026-09-02T14:00:00+08:00".to_string()),
            confirmed_end: Some("2026-09-02T15:00:00+08:00".to_string()),
            location: Some("院区 B".to_string()),
        }),
    )
    .await
    .expect("admin confirmation")
    .0;
    assert_eq!(confirmed["appointment"]["status"], "confirmed");
    assert_eq!(confirmed["appointment"]["version"], 3);
    assert_eq!(confirmed["appointment"]["confirmationSourceType"], "admin");
    assert_eq!(
        confirmed["appointment"]["confirmationSourceId"],
        admin.user_id
    );

    app.cleanup().await;
}

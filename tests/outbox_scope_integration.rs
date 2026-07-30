#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use axum::Json;
use mongodb::bson::{doc, DateTime};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::OutboxEntry;
use wechatagent::routes::admin_outbox::{cancel_outbox, CancelOutboxRequest};

use crate::common::TestApp;

fn admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "outbox_scope_admin".into(),
        username: "outbox_scope_admin".into(),
        current_workspace: workspace_id.into(),
    }
}

fn entry(workspace_id: &str, account_id: &str, status: &str, run_id: &str) -> OutboxEntry {
    let now = DateTime::now();
    OutboxEntry {
        id: None,
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        contact_wxid: "wxid_outbox_scope".into(),
        run_id: run_id.into(),
        decision_id: None,
        source_event_id: format!("event-{run_id}"),
        source_kind: "test".into(),
        content: "do not cancel".into(),
        content_hash: format!("hash-{run_id}"),
        idempotency_key: format!("idem-{run_id}"),
        media_asset_id: None,
        referral_card_id: None,
        attempt: 0,
        max_attempts: 3,
        status: status.into(),
        cancel_reason: None,
        last_error: None,
        next_retry_at: None,
        worker_id: None,
        locked_until: None,
        claim_token: None,
        claim_generation: 0,
        cancel_requested: false,
        cancel_requested_at: None,
        send_started_at: None,
        task_send_authorization_token: None,
        reclaimed_in_flight: false,
        reclaim_count: 0,
        created_at: now,
        updated_at: now,
        sent_at: None,
    }
}

#[tokio::test]
#[ignore]
async fn wrong_account_cancel_is_conflict_with_zero_writes_for_pending_and_in_flight() {
    let app = TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let pending = app
        .state
        .db
        .collection_agent_send_outbox()
        .insert_one(entry(&workspace, "account-a", "pending", "pending"), None)
        .await
        .expect("seed pending")
        .inserted_id
        .as_object_id()
        .expect("pending id");
    let in_flight = app
        .state
        .db
        .collection_agent_send_outbox()
        .insert_one(
            entry(&workspace, "account-a", "in_flight", "in-flight"),
            None,
        )
        .await
        .expect("seed in-flight")
        .inserted_id
        .as_object_id()
        .expect("in-flight id");

    for id in [pending, in_flight] {
        let result = cancel_outbox(
            State(app.state.clone()),
            Extension(admin(&workspace)),
            Path(id.to_hex()),
            Json(
                serde_json::from_value::<CancelOutboxRequest>(serde_json::json!({
                    "expectedAccountId": "account-b",
                    "cancelReason": "wrong account must not write"
                }))
                .expect("request"),
            ),
        )
        .await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
    }

    let stored_pending = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": pending }, None)
        .await
        .expect("read pending")
        .expect("pending exists");
    let stored_in_flight = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": in_flight }, None)
        .await
        .expect("read in-flight")
        .expect("in-flight exists");
    let audit_count = app
        .state
        .db
        .events()
        .count_documents(
            doc! {
                "workspace_id": &workspace,
                "kind": { "$in": ["outbox_canceled", "outbox_cancel_requested"] },
                "contact_wxid": "wxid_outbox_scope",
            },
            None,
        )
        .await
        .expect("count audit events");
    app.cleanup().await;

    assert_eq!(stored_pending.status, "pending");
    assert!(stored_pending.cancel_reason.is_none());
    assert_eq!(stored_in_flight.status, "in_flight");
    assert!(!stored_in_flight.cancel_requested);
    assert!(stored_in_flight.cancel_reason.is_none());
    assert_eq!(audit_count, 0);
}

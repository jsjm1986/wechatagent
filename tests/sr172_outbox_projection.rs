//! SR-172 public-route projection regression.
#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{oid::ObjectId, DateTime};
use serde_json::Value;
use tokio::net::TcpListener;
use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::{ContentAsset, OutboxEntry, ReferralCard};
use wechatagent::routes::api_router;

use crate::common::TestApp;

async fn start_api(
    app: &TestApp,
    workspace_id: &str,
) -> (String, String, tokio::task::JoinHandle<()>) {
    let user = AdminUser {
        user_id: "sr172-admin".into(),
        username: "sr172-admin".into(),
        password_hash: "unused".into(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![workspace_id.to_string()],
        default_workspace: Some(workspace_id.to_string()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&user, None)
        .await
        .expect("insert SR-172 admin ACL");
    let session = create_session(&app.state.db, &user, 1, workspace_id)
        .await
        .expect("create SR-172 session");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind SR-172 API");
    let address = listener.local_addr().expect("SR-172 API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve SR-172 API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

fn media_asset(
    id: ObjectId,
    workspace_id: &str,
    account_id: &str,
    title: &str,
    file_name: &str,
) -> ContentAsset {
    let now = DateTime::now();
    ContentAsset {
        id: Some(id),
        workspace_id: workspace_id.into(),
        account_id: Some(account_id.into()),
        kind: "media".into(),
        title: title.into(),
        body: None,
        tags: vec![],
        url: None,
        media_id: None,
        usage_scene: None,
        media_type: Some("file".into()),
        file_path: Some(format!("{workspace_id}/{file_name}")),
        file_name: Some(file_name.into()),
        file_size: Some(1024),
        mime_type: Some("application/pdf".into()),
        file_sha256: Some(format!("sha-{id}")),
        sendable: Some(true),
        send_trigger_hint: Some("test".into()),
        target_stages: Some(vec![]),
        expression_pref: Some("file_primary".into()),
        requires_principal_approval: Some(false),
        review_status: Some("approved".into()),
        review_note: None,
        min_inject_tier: None,
        enabled: Some(true),
        allowed_insertion_levels: None,
        usage_guidance: None,
        created_at: now,
        updated_at: now,
    }
}

fn outbox_entry(
    workspace_id: &str,
    account_id: &str,
    run_id: &str,
    content: &str,
    media_asset_id: Option<ObjectId>,
    referral_card_id: Option<ObjectId>,
) -> OutboxEntry {
    let now = DateTime::now();
    OutboxEntry {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        contact_wxid: "wxid_shared_customer".into(),
        run_id: run_id.into(),
        decision_id: None,
        source_event_id: format!("event-{run_id}"),
        source_kind: "inbound_message".into(),
        content: content.into(),
        content_hash: format!("hash-{run_id}"),
        idempotency_key: format!("idem-{run_id}"),
        delivery_priority: 0,
        run_sequence: 0,
        media_asset_id: media_asset_id.map(|id| id.to_hex()),
        referral_card_id: referral_card_id.map(|id| id.to_hex()),
        attempt: 0,
        max_attempts: 3,
        status: "pending".into(),
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
        reclaimed_in_flight: run_id == "run-media-own",
        reclaim_count: if run_id == "run-media-own" { 2 } else { 0 },
        created_at: now,
        updated_at: now,
        sent_at: None,
    }
}

#[tokio::test]
#[ignore]
async fn sr172_public_route_preserves_payload_identity_and_account_scope() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_a = "sr172-account-a";
    let account_b = "sr172-account-b";

    let own_asset_id = ObjectId::new();
    let foreign_asset_id = ObjectId::new();
    app.state
        .db
        .content_assets()
        .insert_many(
            vec![
                media_asset(
                    own_asset_id,
                    &workspace_id,
                    account_a,
                    "Account A proposal",
                    "proposal-a.pdf",
                ),
                media_asset(
                    foreign_asset_id,
                    &workspace_id,
                    account_b,
                    "Account B private proposal",
                    "proposal-b.pdf",
                ),
            ],
            None,
        )
        .await
        .expect("insert SR-172 media assets");

    let card_id = ObjectId::new();
    let now = DateTime::now();
    app.state
        .db
        .referral_cards()
        .insert_one(
            ReferralCard {
                id: Some(card_id),
                workspace_id: workspace_id.clone(),
                account_id: Some(account_a.into()),
                target_wxid: "wxid_advisor_a".into(),
                display_name: "Advisor A".into(),
                send_trigger_hint: "test".into(),
                target_stages: vec![],
                tags: vec![],
                enabled: true,
                review_status: "approved".into(),
                review_note: None,
                created_at: now,
                updated_at: now,
            },
            None,
        )
        .await
        .expect("insert SR-172 referral card");

    app.state
        .db
        .collection_agent_send_outbox()
        .insert_many(
            vec![
                outbox_entry(&workspace_id, account_a, "run-text", "hello", None, None),
                outbox_entry(
                    &workspace_id,
                    account_a,
                    "run-media-own",
                    "",
                    Some(own_asset_id),
                    None,
                ),
                outbox_entry(
                    &workspace_id,
                    account_a,
                    "run-card",
                    "",
                    None,
                    Some(card_id),
                ),
                outbox_entry(
                    &workspace_id,
                    account_a,
                    "run-media-foreign-ref",
                    "",
                    Some(foreign_asset_id),
                    None,
                ),
                outbox_entry(
                    &workspace_id,
                    account_b,
                    "run-account-b-control",
                    "",
                    Some(foreign_asset_id),
                    None,
                ),
            ],
            None,
        )
        .await
        .expect("insert SR-172 outbox rows");

    let (base_url, cookie, server) = start_api(&app, &workspace_id).await;
    let response = reqwest::Client::new()
        .get(format!(
            "{base_url}/admin/outbox?accountId={account_a}&limit=20"
        ))
        .header(reqwest::header::COOKIE, cookie)
        .send()
        .await
        .expect("request SR-172 public route");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.expect("decode SR-172 response");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(body["total"], 4);
    assert_eq!(items.len(), 4);

    let by_run = |run_id: &str| {
        items
            .iter()
            .find(|item| item["runId"] == run_id)
            .unwrap_or_else(|| panic!("missing run {run_id}"))
    };
    assert_eq!(
        by_run("run-text")["payload"],
        serde_json::json!({ "kind": "text", "text": "hello" })
    );
    assert_eq!(by_run("run-media-own")["payload"]["kind"], "media");
    assert_eq!(
        by_run("run-media-own")["payload"]["assetId"],
        own_asset_id.to_hex()
    );
    assert_eq!(
        by_run("run-media-own")["payload"]["title"],
        "Account A proposal"
    );
    assert_eq!(by_run("run-media-own")["reclaimCount"], 2);
    assert_eq!(by_run("run-card")["payload"]["kind"], "referralCard");
    assert_eq!(by_run("run-card")["payload"]["cardId"], card_id.to_hex());
    assert_eq!(by_run("run-card")["payload"]["displayName"], "Advisor A");

    let foreign = by_run("run-media-foreign-ref");
    assert_eq!(foreign["payload"]["assetId"], foreign_asset_id.to_hex());
    assert!(foreign["payload"]["title"].is_null());
    assert!(foreign["payload"]["fileName"].is_null());
    assert!(items
        .iter()
        .all(|item| item["runId"] != "run-account-b-control"));

    server.abort();
    app.cleanup().await;
}

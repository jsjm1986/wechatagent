//! SR-138 redlines for prompt-pack bootstrap and explicit destructive reset.
//!
//! Rejected HTTP requests run on any isolated MongoDB and must leave all four
//! governed collections byte-for-byte unchanged. The accepted reset requires a
//! replica set because Soul/Prompt pointer publication uses Mongo transactions.

#![cfg(test)]

mod common;

use axum::Router;
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Document},
    options::FindOptions,
};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::session::{authenticate, bootstrap_admin_if_needed, create_session};
use wechatagent::auth::SESSION_COOKIE_NAME;
use wechatagent::routes::api_router;

use crate::common::TestApp;

const USERNAME: &str = "sr138_admin";
const PASSWORD: &str = "sr138-test-password";
const WORKSPACE: &str = "default";
const CONFIRMATION: &str = "RESET PROMPT PACK";
const GOVERNED_COLLECTIONS: [&str; 4] = [
    "agent_souls",
    "prompt_templates",
    "operation_playbooks",
    "operation_domain_configs",
];

async fn start_api(app: &TestApp) -> (String, String, tokio::task::JoinHandle<()>) {
    bootstrap_admin_if_needed(
        &app.state.db,
        Some(USERNAME),
        Some(PASSWORD),
        Some(WORKSPACE),
    )
    .await
    .expect("bootstrap SR-138 admin");
    let admin = authenticate(&app.state.db, USERNAME, PASSWORD)
        .await
        .expect("authenticate SR-138 admin");
    let session = create_session(&app.state.db, &admin, 1, WORKSPACE)
        .await
        .expect("create SR-138 session");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind SR-138 API");
    let address = listener.local_addr().expect("SR-138 API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve SR-138 API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

async fn collection_snapshot(app: &TestApp, name: &str) -> Vec<Document> {
    let mut cursor = app
        .state
        .db
        .raw()
        .collection::<Document>(name)
        .find(
            doc! { "workspace_id": WORKSPACE },
            FindOptions::builder().sort(doc! { "_id": 1 }).build(),
        )
        .await
        .expect("read governed collection");
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await.expect("read governed row") {
        rows.push(row);
    }
    rows
}

async fn governed_snapshot(app: &TestApp) -> Vec<(&'static str, Vec<Document>)> {
    let mut result = Vec::new();
    for name in GOVERNED_COLLECTIONS {
        result.push((name, collection_snapshot(app, name).await));
    }
    result
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn missing_wrong_or_unknown_confirmation_is_zero_write() {
    let app = TestApp::start().await;
    let (api, cookie, server) = start_api(&app).await;
    let endpoint = format!("{api}/prompt-templates/reset-system-pack");
    let client = reqwest::Client::new();
    let before = governed_snapshot(&app).await;
    let version_before = app
        .state
        .prompt_pack_version
        .load(std::sync::atomic::Ordering::SeqCst);

    let missing = client
        .post(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("send reset without body");
    assert!(missing.status().is_client_error());

    let wrong = client
        .post(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "confirmation": "reset prompt pack" }))
        .send()
        .await
        .expect("send reset with wrong confirmation");
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);

    let unknown = client
        .post(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "confirmation": CONFIRMATION,
            "force": true
        }))
        .send()
        .await
        .expect("send reset with unknown field");
    assert!(unknown.status().is_client_error());

    assert_eq!(governed_snapshot(&app).await, before);
    assert_eq!(
        app.state
            .prompt_pack_version
            .load(std::sync::atomic::Ordering::SeqCst),
        version_before,
        "rejected reset must not invalidate runtime prompt cache"
    );

    server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn exact_confirmation_allows_explicit_reset() {
    let app = TestApp::start_repl_set().await;
    let (api, cookie, server) = start_api(&app).await;
    let custom_id = mongodb::bson::oid::ObjectId::new();
    app.state
        .db
        .raw()
        .collection::<Document>("prompt_templates")
        .insert_one(
            doc! {
                "_id": custom_id,
                "workspace_id": WORKSPACE,
                "prompt_key": "sr138.custom.prompt",
                "agent_kind": "test",
                "layer": "test",
                "title": "SR-138 custom prompt",
                "content": "custom content removed only by explicit reset",
                "status": "draft",
                "version": 1_i32,
                "prompt_pack_version": "test",
                "created_by": "test",
                "created_at": mongodb::bson::DateTime::now(),
                "updated_at": mongodb::bson::DateTime::now(),
                "current_version": false,
                "seeded_by": "manual",
            },
            None,
        )
        .await
        .expect("seed custom prompt");

    let response = reqwest::Client::new()
        .post(format!("{api}/prompt-templates/reset-system-pack"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "confirmation": CONFIRMATION }))
        .send()
        .await
        .expect("send confirmed reset");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(app
        .state
        .db
        .raw()
        .collection::<Document>("prompt_templates")
        .find_one(doc! { "_id": custom_id }, None)
        .await
        .expect("read custom prompt after reset")
        .is_none());
    assert!(!collection_snapshot(&app, "prompt_templates")
        .await
        .is_empty());

    server.abort();
    app.cleanup().await;
}

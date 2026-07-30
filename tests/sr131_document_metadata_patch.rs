//! SR-131 redline: document metadata edits are narrow, versioned patches.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::session::{authenticate, bootstrap_admin_if_needed, create_session};
use wechatagent::auth::SESSION_COOKIE_NAME;
use wechatagent::models::OperationKnowledgeDocument;
use wechatagent::routes::api_router;

use crate::common::TestApp;

async fn start_api(
    app: &TestApp,
    workspace_id: &str,
) -> (String, String, tokio::task::JoinHandle<()>) {
    bootstrap_admin_if_needed(
        &app.state.db,
        Some("sr131_admin"),
        Some("sr131-test-password"),
        Some(workspace_id),
    )
    .await
    .expect("bootstrap admin");
    let admin = authenticate(&app.state.db, "sr131_admin", "sr131-test-password")
        .await
        .expect("authenticate admin");
    let session = create_session(&app.state.db, &admin, 1, workspace_id)
        .await
        .expect("create session");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test API");
    let address = listener.local_addr().expect("test API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve test API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

fn document(workspace_id: &str, document_id: ObjectId) -> OperationKnowledgeDocument {
    OperationKnowledgeDocument {
        id: Some(document_id),
        workspace_id: workspace_id.to_string(),
        account_id: Some("account-a".to_string()),
        domain: "user_operations".to_string(),
        source_type: "imported_markdown".to_string(),
        source_name: Some("source.md".to_string()),
        title: "source document".to_string(),
        summary: Some("original summary".to_string()),
        catalog_summary: Some("catalog summary".to_string()),
        routing_map: vec!["pricing".to_string()],
        risk_notes: vec!["risk".to_string()],
        product_tags: vec!["base".to_string()],
        business_topics: vec!["sales".to_string()],
        raw_content: Some("# immutable source\nbody".to_string()),
        content_hash: Some("hash-original".to_string()),
        line_index: vec![doc! { "line": 1_i32 }],
        section_index: vec![doc! { "section": "root" }],
        status: "active".to_string(),
        version: 7,
        created_at: DateTime::from_millis(1_700_000_000_000),
        updated_at: DateTime::from_millis(1_700_000_000_100),
        catalog_summary_persisted: Some("persisted catalog".to_string()),
        catalog_version: Some(11),
        catalog_desired_generation: 11,
        catalog_applied_generation: 11,
    }
}

async fn raw_document(app: &TestApp, id: ObjectId) -> Document {
    app.state
        .db
        .raw()
        .collection::<Document>("operation_knowledge_documents")
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("read document")
        .expect("document exists")
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn metadata_patch_is_dirty_strict_and_version_fenced() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr131-{}", ObjectId::new().to_hex());
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    let original = document(&workspace_id, document_id);
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(&original, None)
        .await
        .expect("insert document");

    let client = reqwest::Client::new();
    let endpoint = format!(
        "{api}/operation-knowledge/documents/{}",
        document_id.to_hex()
    );

    let changed = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "version": 7,
            "title": "edited title"
        }))
        .send()
        .await
        .expect("dirty patch request");
    assert_eq!(changed.status(), StatusCode::OK);
    let changed_body: serde_json::Value = changed.json().await.expect("dirty patch response");
    assert_eq!(changed_body["unchanged"], false);
    assert_eq!(changed_body["version"], 8);

    let stored = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .expect("read changed document")
        .expect("changed document exists");
    assert_eq!(stored.title, "edited title");
    assert_eq!(stored.version, 8);
    assert_eq!(stored.summary, original.summary);
    assert_eq!(stored.catalog_summary, original.catalog_summary);
    assert_eq!(stored.routing_map, original.routing_map);
    assert_eq!(stored.risk_notes, original.risk_notes);
    assert_eq!(stored.product_tags, original.product_tags);
    assert_eq!(stored.business_topics, original.business_topics);
    assert_eq!(stored.raw_content, original.raw_content);
    assert_eq!(stored.content_hash, original.content_hash);
    assert_eq!(stored.line_index, original.line_index);
    assert_eq!(stored.section_index, original.section_index);
    assert_eq!(stored.workspace_id, original.workspace_id);
    assert_eq!(stored.account_id, original.account_id);
    assert_eq!(stored.status, original.status);
    assert_eq!(stored.catalog_version, original.catalog_version);

    let before_noop = raw_document(&app, document_id).await;
    let noop = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "version": 8,
            "title": "edited title"
        }))
        .send()
        .await
        .expect("no-op patch request");
    assert_eq!(noop.status(), StatusCode::OK);
    let noop_body: serde_json::Value = noop.json().await.expect("no-op patch response");
    assert_eq!(noop_body["unchanged"], true);
    assert_eq!(noop_body["version"], 8);
    assert_eq!(raw_document(&app, document_id).await, before_noop);

    let forbidden = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "version": 8,
            "rawContent": "forged replacement"
        }))
        .send()
        .await
        .expect("forbidden field patch request");
    assert_eq!(forbidden.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(raw_document(&app, document_id).await, before_noop);

    let stale = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "version": 7,
            "summary": "stale summary"
        }))
        .send()
        .await
        .expect("stale patch request");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(raw_document(&app, document_id).await, before_noop);

    let cleared = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "version": 8,
            "summary": null
        }))
        .send()
        .await
        .expect("clear optional field request");
    assert_eq!(cleared.status(), StatusCode::OK);
    let cleared_body: serde_json::Value = cleared.json().await.expect("clear response");
    assert_eq!(cleared_body["version"], 9);
    let after_clear = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .expect("read cleared document")
        .expect("cleared document exists");
    assert!(after_clear.summary.is_none());
    assert_eq!(after_clear.raw_content, original.raw_content);

    server.abort();
    app.cleanup().await;
}

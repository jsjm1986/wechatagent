//! Atomicity and snapshot rollback regressions for knowledge chunk editing.
//! Requires a replica-set MongoDB because every exercised route uses a
//! multi-document transaction.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::session::{authenticate, bootstrap_admin_if_needed, create_session};
use wechatagent::auth::SESSION_COOKIE_NAME;
use wechatagent::models::{
    ChunkRevision, DomainField, DomainSchema, OperationKnowledgeChunk, OperationKnowledgeDocument,
};
use wechatagent::routes::api_router;

use crate::common::TestApp;

async fn start_api(
    app: &TestApp,
    workspace_id: &str,
) -> (String, String, tokio::task::JoinHandle<()>) {
    bootstrap_admin_if_needed(
        &app.state.db,
        Some("knowledge_transaction_admin"),
        Some("knowledge-transaction-password"),
        Some(workspace_id),
    )
    .await
    .expect("bootstrap admin");
    let admin = authenticate(
        &app.state.db,
        "knowledge_transaction_admin",
        "knowledge-transaction-password",
    )
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

fn workspace() -> String {
    format!("knowledge-txn-{}", ObjectId::new().to_hex())
}

fn chunk(
    workspace_id: &str,
    title: &str,
    body: &str,
    document_id: ObjectId,
) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: Some("account-a".to_string()),
        document_id: Some(document_id),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        body: Some(body.to_string()),
        product_tags: vec!["base".to_string()],
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        confidence_score: Some(100),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
        ..Default::default()
    }
}

fn document(workspace_id: &str, document_id: ObjectId) -> OperationKnowledgeDocument {
    let created_at = DateTime::from_millis(1_700_000_000_000);
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
        created_at,
        updated_at: DateTime::from_millis(1_700_000_000_100),
        catalog_summary_persisted: Some("persisted catalog".to_string()),
        catalog_version: Some(11),
        catalog_desired_generation: 11,
        catalog_applied_generation: 11,
    }
}

async fn seed_parent_document(app: &TestApp, workspace_id: &str, document_id: ObjectId) {
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(document(workspace_id, document_id), None)
        .await
        .expect("insert parent document");
}

fn required_stage_schema(workspace_id: &str) -> DomainSchema {
    DomainSchema {
        id: None,
        schema_id: "required-stage".to_string(),
        workspace_id: workspace_id.to_string(),
        name: "required stage".to_string(),
        version: 1,
        fields: vec![DomainField {
            name: "stage".to_string(),
            label: "Stage".to_string(),
            kind: "string".to_string(),
            required: true,
            allowed_values: None,
            alias_of: None,
        }],
        alias_dict: Default::default(),
        guard_dsl: None,
        is_active: true,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn split_commits_children_revisions_and_jobs_together() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    seed_parent_document(&app, &workspace_id, document_id).await;
    let source = chunk(&workspace_id, "source", "abcdefghij", document_id);
    let source_id = source.id.expect("source id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");

    let response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/split",
            source_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "offset": 5, "reason": "transaction split" }))
        .send()
        .await
        .expect("split request");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("split response");
    assert_eq!(body["newChunkIds"].as_array().map(Vec::len), Some(2));

    let stored_source = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": source_id }, None)
        .await
        .expect("load source")
        .expect("source exists");
    assert_eq!(stored_source.status, "archived");
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "previous_version_id": source_id.to_hex(),
                    "status": "draft",
                },
                None,
            )
            .await
            .unwrap(),
        2
    );
    let revisions = app
        .state
        .db
        .chunk_revisions()
        .count_documents(doc! { "workspace_id": &workspace_id }, None)
        .await
        .unwrap();
    assert_eq!(revisions, 3);
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        3
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn split_validation_failure_rolls_back_inserted_child() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    app.state
        .db
        .domain_schemas()
        .insert_one(required_stage_schema(&workspace_id), None)
        .await
        .expect("insert schema");
    let document_id = ObjectId::new();
    let mut source = chunk(&workspace_id, "invalid source", "abcdefghij", document_id);
    source.domain_attributes = Some(doc! { "other": "missing required stage" });
    let source_id = source.id.expect("source id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");

    let response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/split",
            source_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "offset": 5 }))
        .send()
        .await
        .expect("split request");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        1,
        "the child inserted before schema validation must be rolled back"
    );
    let stored_source = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": source_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_source.status, "active");
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn merge_commits_target_archive_revisions_and_jobs_together() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    seed_parent_document(&app, &workspace_id, document_id).await;
    let source = chunk(&workspace_id, "source", "source body", document_id);
    let target = chunk(&workspace_id, "target", "target body", document_id);
    let source_id = source.id.expect("source id");
    let target_id = target.id.expect("target id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many(vec![source, target], None)
        .await
        .expect("insert merge pair");

    let response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/merge",
            source_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "targetId": target_id.to_hex(),
            "reason": "transaction merge"
        }))
        .send()
        .await
        .expect("merge request");
    assert_eq!(response.status(), StatusCode::OK);

    let stored_source = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": source_id }, None)
        .await
        .unwrap()
        .unwrap();
    let stored_target = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": target_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_source.status, "archived");
    assert_eq!(
        stored_source.superseded_by.as_deref(),
        Some(target_id.to_hex().as_str())
    );
    assert_eq!(stored_target.status, "draft");
    assert_eq!(
        stored_target.integrity_status.as_deref(),
        Some("needs_review")
    );
    assert_eq!(
        stored_target.body.as_deref(),
        Some("target body\n\nsource body")
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "workspace_id": &workspace_id, "op": "merge" }, None)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        2
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn merge_source_validation_failure_rolls_back_target_update() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    app.state
        .db
        .domain_schemas()
        .insert_one(required_stage_schema(&workspace_id), None)
        .await
        .expect("insert schema");
    let document_id = ObjectId::new();
    seed_parent_document(&app, &workspace_id, document_id).await;
    let mut source = chunk(&workspace_id, "source", "source body", document_id);
    source.domain_attributes = Some(doc! { "other": "missing required stage" });
    let mut target = chunk(&workspace_id, "target", "target body", document_id);
    target.domain_attributes = Some(doc! { "stage": "valid" });
    let source_id = source.id.expect("source id");
    let target_id = target.id.expect("target id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many(vec![source, target], None)
        .await
        .expect("insert merge pair");

    let response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/merge",
            source_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "targetId": target_id.to_hex() }))
        .send()
        .await
        .expect("merge request");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let stored_source = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": source_id }, None)
        .await
        .unwrap()
        .unwrap();
    let stored_target = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": target_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_source.status, "active");
    assert_eq!(stored_source.superseded_by, None);
    assert_eq!(stored_target.status, "active");
    assert_eq!(stored_target.body.as_deref(), Some("target body"));
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0,
        "target revision written before source validation must be rolled back"
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn rollback_restores_snapshot_exactly_but_reenters_review() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    seed_parent_document(&app, &workspace_id, document_id).await;
    let original = chunk(&workspace_id, "original", "original body", document_id);
    let chunk_id = original.id.expect("chunk id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&original, None)
        .await
        .expect("insert chunk");

    let patch_response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/patch",
            chunk_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "patch": {
                "title": "changed",
                "body": "changed body with enough characters",
                "productTags": ["later"]
            },
            "reason": "create rollback target"
        }))
        .send()
        .await
        .expect("patch request");
    assert_eq!(patch_response.status(), StatusCode::OK);
    let patch_body: serde_json::Value = patch_response.json().await.unwrap();
    let target_revision_id = patch_body["revisionId"].as_str().unwrap().to_string();

    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": chunk_id },
            doc! {
                "$set": {
                    "usage_stats": {
                        "hit_count_30d": 9_i32,
                        "blocked_count_30d": 2_i32,
                    }
                }
            },
            None,
        )
        .await
        .expect("update runtime stats");

    let rollback_response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/rollback/{}",
            chunk_id.to_hex(),
            target_revision_id
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "reason": "exact restore" }))
        .send()
        .await
        .expect("rollback request");
    assert_eq!(rollback_response.status(), StatusCode::OK);

    let restored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.title, "original");
    assert_eq!(restored.body.as_deref(), Some("original body"));
    assert_eq!(restored.product_tags, vec!["base"]);
    assert_eq!(restored.status, "draft");
    assert_eq!(restored.integrity_status.as_deref(), Some("needs_review"));
    assert_eq!(restored.confidence_score, Some(0));
    assert_eq!(restored.workspace_id, workspace_id);
    assert_eq!(restored.account_id.as_deref(), Some("account-a"));
    assert_eq!(
        restored
            .usage_stats
            .as_ref()
            .map(|stats| stats.hit_count_30d),
        Some(9)
    );

    let rollback_revision = app
        .state
        .db
        .chunk_revisions()
        .find_one(
            doc! { "workspace_id": &workspace_id, "chunk_id": chunk_id.to_hex(), "op": "rollback" },
            None,
        )
        .await
        .unwrap()
        .expect("rollback revision");
    assert!(rollback_revision.before_snapshot.is_some());
    assert!(rollback_revision.after_snapshot.is_some());
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn rollback_legacy_revision_without_snapshot_fails_closed() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    let source = chunk(&workspace_id, "source", "source body", document_id);
    let chunk_id = source.id.expect("chunk id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");
    let legacy_revision_id = format!("legacy-{}", ObjectId::new().to_hex());
    app.state
        .db
        .chunk_revisions()
        .insert_one(
            ChunkRevision {
                id: None,
                workspace_id: workspace_id.clone(),
                chunk_id: chunk_id.to_hex(),
                revision_id: legacy_revision_id.clone(),
                op: "patch".to_string(),
                patch: doc! { "title": "legacy" },
                before_hash: "before".to_string(),
                after_hash: "after".to_string(),
                before_snapshot: None,
                after_snapshot: None,
                source: "human".to_string(),
                reason: None,
                created_at: DateTime::now(),
                created_by: Some("legacy-admin".to_string()),
            },
            None,
        )
        .await
        .expect("insert legacy revision");

    let response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/rollback/{}",
            chunk_id.to_hex(),
            legacy_revision_id
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("rollback request");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"], "chunk_revision_snapshot_unavailable");

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title, "source");
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "op": "rollback" },
                None
            )
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn patch_rejects_managed_fields_without_any_write() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    let source = chunk(&workspace_id, "source", "source body", document_id);
    let chunk_id = source.id.expect("chunk id");
    let before = mongodb::bson::to_document(&source).expect("serialize source");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");

    let response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/patch",
            chunk_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "patch": {
                "workspaceId": "foreign-workspace",
                "accountId": "foreign-account",
                "status": "active",
                "integrityStatus": "verified",
                "actor": "forged-admin"
            },
            "reason": "malicious managed-field patch"
        }))
        .send()
        .await
        .expect("patch request");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        mongodb::bson::to_document(&stored).expect("serialize stored"),
        before
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "chunk_id": chunk_id.to_hex() }, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "document_id": document_id }, None)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn legacy_put_cannot_move_scope_or_self_verify() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    seed_parent_document(&app, &workspace_id, document_id).await;
    let foreign_document_id = ObjectId::new();
    let source = chunk(&workspace_id, "source", "source body", document_id);
    let chunk_id = source.id.expect("chunk id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");

    let response = reqwest::Client::new()
        .put(format!(
            "{api}/operation-knowledge/chunks/{}",
            chunk_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "title": "edited title",
            "workspaceId": "foreign-workspace",
            "accountId": "foreign-account",
            "documentId": foreign_document_id.to_hex(),
            "domain": "foreign-domain",
            "status": "active",
            "integrityStatus": "verified",
            "confidenceScore": 100,
            "sourceAnchors": [{ "startOffset": 0, "endOffset": 4 }]
        }))
        .send()
        .await
        .expect("legacy PUT request");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["status"], "draft");
    assert_eq!(body["integrityStatus"], "needs_review");

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title, "edited title");
    assert_eq!(stored.workspace_id, workspace_id);
    assert_eq!(stored.account_id.as_deref(), Some("account-a"));
    assert_eq!(stored.document_id, Some(document_id));
    assert_eq!(stored.domain, "user_operations");
    assert_eq!(stored.status, "draft");
    assert_eq!(stored.integrity_status.as_deref(), Some("needs_review"));
    assert_eq!(stored.confidence_score, Some(0));
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(
                doc! { "_id": chunk_id, "workspace_id": "foreign-workspace" },
                None
            )
            .await
            .unwrap(),
        0
    );
    let revision = app
        .state
        .db
        .chunk_revisions()
        .find_one(doc! { "chunk_id": chunk_id.to_hex() }, None)
        .await
        .unwrap()
        .expect("committed revision");
    assert_eq!(
        revision.created_by.as_deref(),
        Some("knowledge_transaction_admin")
    );
    assert_eq!(revision.source, "human");
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn split_rejects_legacy_new_chunks_injection_without_any_write() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    let source = chunk(&workspace_id, "source", "abcdefghij", document_id);
    let source_id = source.id.expect("source id");
    let before = mongodb::bson::to_document(&source).expect("serialize source");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");

    let response = reqwest::Client::new()
        .post(format!(
            "{api}/operation-knowledge/chunks/{}/split",
            source_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "offset": 5,
            "newChunks": [{
                "workspace_id": "foreign-workspace",
                "status": "active",
                "integrity_status": "verified"
            }]
        }))
        .send()
        .await
        .expect("split request");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": source_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        mongodb::bson::to_document(&stored).expect("serialize stored"),
        before
    );
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(doc! { "workspace_id": "foreign-workspace" }, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn repair_apply_event_points_to_committed_revision() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    seed_parent_document(&app, &workspace_id, document_id).await;
    let source = chunk(&workspace_id, "source", "source body", document_id);
    let chunk_id = source.id.expect("chunk id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");

    let response = reqwest::Client::new()
        .post(format!("{api}/operation-knowledge/repair/applied"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "targetKind": "chunk",
            "targetId": chunk_id.to_hex(),
            "patch": { "summary": "AI proposed summary", "title": "ignored title" },
            "sessionId": "repair-session-success",
            "turn": 1,
            "acceptedFields": ["summary"],
            "skippedFields": [],
            "confidenceHint": 81,
            "extras": { "candidate": "kept only in audit" },
            "thenVerify": false
        }))
        .send()
        .await
        .expect("repair apply request");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("repair response");
    let revision_id = body["revisionId"]
        .as_str()
        .expect("committed revision id")
        .to_string();

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .unwrap()
        .expect("stored chunk");
    assert_eq!(stored.summary.as_deref(), Some("AI proposed summary"));
    assert_eq!(stored.status, "draft");
    assert_eq!(stored.integrity_status.as_deref(), Some("needs_review"));

    let revision = app
        .state
        .db
        .chunk_revisions()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "chunk_id": chunk_id.to_hex(),
                "revision_id": &revision_id,
            },
            None,
        )
        .await
        .unwrap()
        .expect("repair revision");
    assert_eq!(revision.source, "ai");
    assert_eq!(
        revision.created_by.as_deref(),
        Some("knowledge_transaction_admin")
    );

    let event = app
        .state
        .db
        .events()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "kind": "knowledge_repair_applied",
            },
            None,
        )
        .await
        .unwrap()
        .expect("repair applied event");
    let details = event.details.expect("event details");
    assert_eq!(details.get_str("revisionId").unwrap(), revision_id);
    assert_eq!(
        details.get_array("acceptedFields").unwrap(),
        &vec![mongodb::bson::Bson::String("summary".to_string())]
    );
    assert_eq!(
        details.get_array("skippedFields").unwrap(),
        &vec![mongodb::bson::Bson::String("title".to_string())],
        "skipped fields must be derived by the server, not trusted from the request"
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn repair_apply_event_failure_rolls_back_chunk_revision_and_catalog_job() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    seed_parent_document(&app, &workspace_id, document_id).await;
    let source = chunk(&workspace_id, "source", "source body", document_id);
    let chunk_id = source.id.expect("chunk id");
    let before = mongodb::bson::to_document(&source).expect("serialize source");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "agent_events",
                "validator": { "kind": { "$ne": "knowledge_repair_applied" } },
                "validationLevel": "strict",
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install repair event validator");

    let response = reqwest::Client::new()
        .post(format!("{api}/operation-knowledge/repair/applied"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "targetKind": "chunk",
            "targetId": chunk_id.to_hex(),
            "patch": { "summary": "must roll back" },
            "sessionId": "repair-session-failure",
            "turn": 1,
            "acceptedFields": ["summary"],
            "skippedFields": [],
            "confidenceHint": 75,
            "thenVerify": false
        }))
        .send()
        .await
        .expect("repair apply request");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .unwrap()
        .expect("stored chunk");
    assert_eq!(
        mongodb::bson::to_document(&stored).expect("serialize stored"),
        before,
        "event failure must roll back the main chunk replacement"
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "chunk_id": chunk_id.to_hex() }, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "document_id": document_id }, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(doc! { "kind": "knowledge_repair_applied" }, None)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn chunk_create_commits_row_revision_and_catalog_intent_together() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(document(&workspace_id, document_id), None)
        .await
        .expect("insert parent document");

    let response = reqwest::Client::new()
        .post(format!("{api}/operation-knowledge/chunks"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "documentId": document_id.to_hex(),
            "title": "new manual chunk",
            "body": "new chunk body",
            "status": "active",
            "integrityStatus": "verified",
            "confidenceScore": 100
        }))
        .send()
        .await
        .expect("create chunk request");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("create response");
    let chunk_id = ObjectId::parse_str(body["id"].as_str().expect("chunk id")).unwrap();
    let revision_id = body["revisionId"].as_str().expect("revision id");
    assert_eq!(body["status"], serde_json::json!("draft"));
    assert_eq!(body["integrityStatus"], serde_json::json!("needs_review"));

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .unwrap()
        .expect("created chunk");
    assert_eq!(stored.status, "draft");
    assert_eq!(stored.integrity_status.as_deref(), Some("needs_review"));
    assert_eq!(stored.confidence_score, Some(0));
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "chunk_id": chunk_id.to_hex(),
                    "revision_id": revision_id,
                    "op": "create",
                },
                None,
            )
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "document_id": document_id },
                None,
            )
            .await
            .unwrap(),
        1
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn chunk_create_catalog_failure_rolls_back_row_and_revision() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(document(&workspace_id, document_id), None)
        .await
        .expect("insert parent document");
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "catalog_rebuild_jobs",
                "validator": { "document_id": { "$ne": document_id } },
                "validationLevel": "strict",
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install catalog intent validator");

    let response = reqwest::Client::new()
        .post(format!("{api}/operation-knowledge/chunks"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "documentId": document_id.to_hex(),
            "title": "must roll back",
            "body": "must roll back"
        }))
        .send()
        .await
        .expect("create chunk request");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "document_id": document_id },
                None,
            )
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "document_id": document_id }, None)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn document_put_preserves_server_owned_fields_and_rejects_stale_version() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    let original = document(&workspace_id, document_id);
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(&original, None)
        .await
        .expect("insert document");

    let edit = serde_json::json!({
        "version": 7,
        "accountId": "forged-account",
        "domain": "forged-domain",
        "sourceType": "forged-source",
        "sourceName": "renamed.md",
        "title": "edited title",
        "summary": "edited summary",
        "catalogSummary": "edited catalog summary",
        "routingMap": ["delivery"],
        "riskNotes": ["new risk"],
        "productTags": ["new-product"],
        "businessTopics": ["delivery"],
        "rawContent": "forged replacement",
        "contentHash": "forged-hash",
        "lineIndex": [{ "line": 999 }],
        "sectionIndex": [{ "section": "forged" }],
        "status": "archived"
    });
    let response = reqwest::Client::new()
        .put(format!(
            "{api}/operation-knowledge/documents/{}",
            document_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&edit)
        .send()
        .await
        .expect("document edit request");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("edit response");
    assert_eq!(body["version"], serde_json::json!(8));

    let stored = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .expect("stored document");
    assert_eq!(stored.title, "edited title");
    assert_eq!(stored.summary.as_deref(), Some("edited summary"));
    assert_eq!(stored.source_name.as_deref(), Some("renamed.md"));
    assert_eq!(stored.version, 8);
    assert_eq!(stored.workspace_id, workspace_id);
    assert_eq!(stored.account_id.as_deref(), Some("account-a"));
    assert_eq!(stored.domain, "user_operations");
    assert_eq!(stored.source_type, "imported_markdown");
    assert_eq!(stored.raw_content, original.raw_content);
    assert_eq!(stored.content_hash, original.content_hash);
    assert_eq!(stored.line_index, original.line_index);
    assert_eq!(stored.section_index, original.section_index);
    assert_eq!(stored.status, "active");
    assert_eq!(stored.created_at, original.created_at);
    assert_eq!(
        stored.catalog_summary_persisted,
        original.catalog_summary_persisted
    );
    assert_eq!(stored.catalog_version, original.catalog_version);

    let stale = reqwest::Client::new()
        .put(format!(
            "{api}/operation-knowledge/documents/{}",
            document_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&edit)
        .send()
        .await
        .expect("stale document edit request");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let after_stale = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_stale.version, 8);
    assert_eq!(after_stale.title, "edited title");
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn document_patch_is_strict_dirty_and_version_fenced() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
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
    let response = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "version": 7, "title": "patched title" }))
        .send()
        .await
        .expect("document patch request");
    assert_eq!(response.status(), StatusCode::OK);
    let receipt: serde_json::Value = response.json().await.expect("patch receipt");
    assert_eq!(receipt["version"], serde_json::json!(8));
    assert_eq!(receipt["unchanged"], serde_json::json!(false));

    let raw = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("operation_knowledge_documents");
    let after_patch = raw
        .find_one(doc! { "_id": document_id }, None)
        .await
        .expect("read patched document")
        .expect("patched document exists");
    assert_eq!(after_patch.get_str("title").unwrap(), "patched title");
    assert_eq!(after_patch.get_i32("version").unwrap(), 8);
    assert_eq!(
        after_patch.get_str("raw_content").unwrap(),
        "# immutable source\nbody"
    );
    assert_eq!(
        after_patch.get_str("content_hash").unwrap(),
        "hash-original"
    );
    assert_eq!(after_patch.get_array("routing_map").unwrap().len(), 1);
    assert_eq!(after_patch.get_array("product_tags").unwrap().len(), 1);
    assert_eq!(after_patch.get_str("status").unwrap(), "active");

    let no_op = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "version": 8, "title": "patched title" }))
        .send()
        .await
        .expect("no-op patch request");
    assert_eq!(no_op.status(), StatusCode::OK);
    let no_op_receipt: serde_json::Value = no_op.json().await.expect("no-op receipt");
    assert_eq!(no_op_receipt["unchanged"], serde_json::json!(true));
    assert_eq!(no_op_receipt["version"], serde_json::json!(8));
    let after_no_op = raw
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_no_op, after_patch);

    let forbidden = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "version": 8,
            "rawContent": "forged replacement"
        }))
        .send()
        .await
        .expect("forbidden patch request");
    assert_eq!(forbidden.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let after_forbidden = raw
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_forbidden, after_patch);

    let stale = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "version": 7, "summary": "stale" }))
        .send()
        .await
        .expect("stale patch request");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let after_stale = raw
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_stale, after_patch);

    let clear = client
        .patch(&endpoint)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "version": 8, "summary": null }))
        .send()
        .await
        .expect("clear summary patch request");
    assert_eq!(clear.status(), StatusCode::OK);
    let after_clear = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_clear.version, 9);
    assert!(after_clear.summary.is_none());
    assert_eq!(after_clear.raw_content, original.raw_content);
    assert_eq!(after_clear.routing_map, original.routing_map);
    assert_eq!(after_clear.product_tags, original.product_tags);
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn legacy_chunk_delete_soft_archives_and_keeps_revision_history() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    seed_parent_document(&app, &workspace_id, document_id).await;
    let source = chunk(&workspace_id, "source", "source body", document_id);
    let chunk_id = source.id.expect("chunk id");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&source, None)
        .await
        .expect("insert source");

    let response = reqwest::Client::new()
        .delete(format!(
            "{api}/operation-knowledge/chunks/{}",
            chunk_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("legacy chunk delete request");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("archive response");
    assert_eq!(body["archived"], serde_json::json!(true));
    assert!(body["revisionId"].as_str().is_some());

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .unwrap()
        .expect("archived chunk remains readable");
    assert_eq!(stored.status, "archived");
    let revision = app
        .state
        .db
        .chunk_revisions()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "chunk_id": chunk_id.to_hex(),
                "op": "archive",
            },
            None,
        )
        .await
        .unwrap()
        .expect("archive revision");
    assert!(revision.before_snapshot.is_some());
    assert!(revision.after_snapshot.is_some());

    let repeated = reqwest::Client::new()
        .delete(format!(
            "{api}/operation-knowledge/chunks/{}",
            chunk_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("repeated legacy chunk delete request");
    assert_eq!(repeated.status(), StatusCode::OK);
    let repeated_body: serde_json::Value = repeated.json().await.unwrap();
    assert_eq!(repeated_body["unchanged"], serde_json::json!(true));
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "chunk_id": chunk_id.to_hex() },
                None,
            )
            .await
            .unwrap(),
        1,
        "idempotent re-archive must not create duplicate history"
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn document_delete_atomically_archives_parent_and_children() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    let parent = document(&workspace_id, document_id);
    let first = chunk(&workspace_id, "first", "first body", document_id);
    let second = chunk(&workspace_id, "second", "second body", document_id);
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(&parent, None)
        .await
        .expect("insert parent");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many([first.clone(), second.clone()], None)
        .await
        .expect("insert children");

    let response = reqwest::Client::new()
        .delete(format!(
            "{api}/operation-knowledge/documents/{}",
            document_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("document archive request");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["archived"], serde_json::json!(true));
    assert_eq!(body["archivedChunks"], serde_json::json!(2));
    assert_eq!(body["version"], serde_json::json!(8));

    let stored_parent = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .expect("parent remains");
    assert_eq!(stored_parent.status, "archived");
    assert_eq!(stored_parent.version, 8);
    assert_eq!(stored_parent.raw_content, parent.raw_content);
    assert_eq!(stored_parent.catalog_version, parent.catalog_version);
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(
                doc! {
                    "document_id": document_id,
                    "workspace_id": &workspace_id,
                    "status": "archived",
                },
                None,
            )
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "op": "archive" },
                None,
            )
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "document_id": document_id }, None)
            .await
            .unwrap(),
        2
    );

    let repeated = reqwest::Client::new()
        .delete(format!(
            "{api}/operation-knowledge/documents/{}",
            document_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("repeated document archive request");
    assert_eq!(repeated.status(), StatusCode::OK);
    let repeated_body: serde_json::Value = repeated.json().await.unwrap();
    assert_eq!(repeated_body["archivedChunks"], serde_json::json!(0));
    assert_eq!(repeated_body["version"], serde_json::json!(8));
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "op": "archive" },
                None,
            )
            .await
            .unwrap(),
        2
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn document_archive_child_revision_failure_rolls_back_everything() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = workspace();
    let (api, cookie, server) = start_api(&app, &workspace_id).await;
    let document_id = ObjectId::new();
    let parent = document(&workspace_id, document_id);
    let first_id = ObjectId::parse_str("000000000000000000000001").unwrap();
    let second_id = ObjectId::parse_str("000000000000000000000002").unwrap();
    let mut first = chunk(&workspace_id, "first", "first body", document_id);
    first.id = Some(first_id);
    let mut second = chunk(&workspace_id, "second", "second body", document_id);
    second.id = Some(second_id);
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(&parent, None)
        .await
        .expect("insert parent");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many([first.clone(), second.clone()], None)
        .await
        .expect("insert children");
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "chunk_revisions",
                "validator": { "chunk_id": { "$ne": second_id.to_hex() } },
                "validationLevel": "strict",
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install second revision validator");

    let response = reqwest::Client::new()
        .delete(format!(
            "{api}/operation-knowledge/documents/{}",
            document_id.to_hex()
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("document archive request");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let stored_parent = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .expect("parent remains active");
    assert_eq!(stored_parent.status, "active");
    assert_eq!(stored_parent.version, 7);
    for chunk_id in [first_id, second_id] {
        let stored = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(doc! { "_id": chunk_id }, None)
            .await
            .unwrap()
            .expect("child remains");
        assert_eq!(stored.status, "active");
    }
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0,
        "the first child revision must roll back when the second is rejected"
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "document_id": document_id }, None)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

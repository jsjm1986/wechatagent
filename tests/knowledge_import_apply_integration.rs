//! SR-112 import preview/apply transaction regressions through production HTTP.

#![cfg(test)]

mod common;

use axum::Router;
use chrono::Utc;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use reqwest::{Response, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use wechatagent::auth::{password::hash_password, session::create_session};
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::ImportJob;
use wechatagent::routes::api_router;
use wechatagent::routes::ext_knowledge::ingest_chunked_text;

use crate::common::TestApp;

struct TestApi {
    base_url: String,
    owner_cookie: String,
    other_cookie: String,
    owner_id: String,
    server: tokio::task::JoinHandle<()>,
}

async fn start_api(app: &TestApp, workspace_id: &str) -> TestApi {
    let owner = admin_user("import-owner", workspace_id);
    let other = admin_user("import-other", workspace_id);
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_many([owner.clone(), other.clone()], None)
        .await
        .expect("seed import admins");
    let owner_session = create_session(&app.state.db, &owner, 1, workspace_id)
        .await
        .expect("create owner session");
    let other_session = create_session(&app.state.db, &other, 1, workspace_id)
        .await
        .expect("create other session");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind import test API");
    let address = listener.local_addr().expect("import test API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve import test API");
    });
    TestApi {
        base_url: format!("http://{address}/api"),
        owner_cookie: format!("{SESSION_COOKIE_NAME}={}", owner_session.session_id),
        other_cookie: format!("{SESSION_COOKIE_NAME}={}", other_session.session_id),
        owner_id: owner.user_id,
        server,
    }
}

fn admin_user(username: &str, workspace_id: &str) -> AdminUser {
    AdminUser {
        user_id: format!("{username}-id"),
        username: username.to_string(),
        password_hash: hash_password("unused-in-integration-test").expect("hash password"),
        created_at: Utc::now(),
        last_login_at: None,
        workspaces: vec![workspace_id.to_string()],
        default_workspace: Some(workspace_id.to_string()),
    }
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical_json(&object[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

fn sha256_json(value: &Value) -> String {
    let bytes = serde_json::to_vec(&canonical_json(value)).expect("serialize preview");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn preview_chunk(candidate_id: &str, title: &str, body: &str) -> Value {
    json!({
        "candidateId": candidate_id,
        "domain": "user_operations",
        "knowledgeType": "fact",
        "businessContext": "integration test",
        "title": title,
        "summary": body,
        "body": body,
        "applicableScenes": [],
        "notApplicableScenes": [],
        "productTags": [],
        "businessTopics": [],
        "wikiType": "finding",
        "chunkType": "product_fact",
        "sourceQuote": body,
        "status": "draft",
        "priority": 0
    })
}

async fn seed_preview(
    app: &TestApp,
    workspace_id: &str,
    owner_id: &str,
    chunks: Vec<Value>,
) -> (ObjectId, String) {
    let preview_id = ObjectId::new();
    let content = "first body\nsecond body";
    let mut preview = json!({
        "previewId": preview_id.to_hex(),
        "document": {
            "domain": "user_operations",
            "sourceType": "imported_markdown",
            "sourceName": "sr112.md",
            "title": "SR-112 import",
            "rawContent": content,
            "status": "draft"
        },
        "items": [],
        "chunks": chunks,
        "integrityReport": {},
        "importReport": { "totalSegments": 1, "succeeded": 1, "failed": 0 }
    });
    let preview_hash = sha256_json(&preview);
    preview
        .as_object_mut()
        .expect("preview object")
        .insert("previewHash".to_string(), json!(&preview_hash));
    let now = DateTime::now();
    app.state
        .db
        .import_jobs()
        .insert_one(
            ImportJob {
                id: Some(preview_id),
                workspace_id: workspace_id.to_string(),
                account_id: None,
                source_name: "sr112.md".to_string(),
                content: content.to_string(),
                segments_total: 1,
                progress_done: 1,
                progress_succeeded: 1,
                progress_failed: 0,
                status: "completed".to_string(),
                owner_admin_id: Some(owner_id.to_string()),
                preview_hash: Some(preview_hash.clone()),
                apply_status: Some("ready".to_string()),
                apply_request_hash: None,
                apply_result: None,
                applied_at: None,
                result: Some(preview),
                error: None,
                claimed_at: None,
                claim_generation: 0,
                claim_token: None,
                claim_recovery_count: 0,
                expires_at: Some(DateTime::from_millis(
                    now.timestamp_millis() + 24 * 60 * 60 * 1000,
                )),
                created_at: now,
                updated_at: now,
            },
            None,
        )
        .await
        .expect("seed sealed import preview");
    (preview_id, preview_hash)
}

async fn apply_preview(
    api: &TestApi,
    cookie: &str,
    preview_id: ObjectId,
    preview_hash: &str,
    candidate_ids: &[&str],
) -> Response {
    reqwest::Client::new()
        .post(format!("{}/operation-knowledge/import-apply", api.base_url))
        .header(reqwest::header::COOKIE, cookie)
        .json(&json!({
            "previewId": preview_id.to_hex(),
            "previewHash": preview_hash,
            "chunks": candidate_ids
                .iter()
                .map(|candidate_id| json!({ "candidateId": candidate_id, "patch": {} }))
                .collect::<Vec<_>>()
        }))
        .send()
        .await
        .expect("send import apply request")
}

async fn artifact_counts(app: &TestApp, workspace_id: &str) -> (u64, u64, u64, u64) {
    let scope = doc! { "workspace_id": workspace_id };
    let documents = app
        .state
        .db
        .operation_knowledge_documents()
        .count_documents(scope.clone(), None)
        .await
        .expect("count documents");
    let chunks = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(scope.clone(), None)
        .await
        .expect("count chunks");
    let revisions = app
        .state
        .db
        .chunk_revisions()
        .count_documents(scope.clone(), None)
        .await
        .expect("count revisions");
    let catalog_jobs = app
        .state
        .db
        .catalog_rebuild_jobs()
        .count_documents(scope, None)
        .await
        .expect("count catalog jobs");
    (documents, chunks, revisions, catalog_jobs)
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn concurrent_apply_and_replay_commit_exactly_once() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let api = start_api(&app, &workspace_id).await;
    let (preview_id, preview_hash) = seed_preview(
        &app,
        &workspace_id,
        &api.owner_id,
        vec![
            preview_chunk("candidate-0001", "first", "first body"),
            preview_chunk("candidate-0002", "second", "second body"),
        ],
    )
    .await;

    let first = apply_preview(
        &api,
        &api.owner_cookie,
        preview_id,
        &preview_hash,
        &["candidate-0001", "candidate-0002"],
    );
    let second = apply_preview(
        &api,
        &api.owner_cookie,
        preview_id,
        &preview_hash,
        &["candidate-0001", "candidate-0002"],
    );
    let (first, second) = tokio::join!(first, second);
    let first_status = first.status();
    let second_status = second.status();
    assert_eq!(
        first_status,
        StatusCode::OK,
        "first concurrent apply failed"
    );
    assert_eq!(
        second_status,
        StatusCode::OK,
        "second concurrent apply must converge to the committed receipt"
    );
    let first_body: Value = first.json().await.expect("first apply receipt");
    let second_body: Value = second.json().await.expect("second apply receipt");
    assert_eq!(
        first_body, second_body,
        "concurrent calls must share a receipt"
    );

    let replay = apply_preview(
        &api,
        &api.owner_cookie,
        preview_id,
        &preview_hash,
        &["candidate-0001", "candidate-0002"],
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body: Value = replay.json().await.expect("replay receipt");
    assert_eq!(replay_body, first_body, "network replay must be stable");
    assert_eq!(artifact_counts(&app, &workspace_id).await, (1, 2, 2, 2));

    let job = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": preview_id }, None)
        .await
        .expect("load import intent")
        .expect("import intent exists");
    assert_eq!(job.apply_status.as_deref(), Some("applied"));
    assert!(job.apply_result.is_some());

    api.server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn second_candidate_failure_rolls_back_every_artifact_and_claim() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let api = start_api(&app, &workspace_id).await;
    let (preview_id, preview_hash) = seed_preview(
        &app,
        &workspace_id,
        &api.owner_id,
        vec![preview_chunk("candidate-0001", "first", "first body")],
    )
    .await;

    let response = apply_preview(
        &api,
        &api.owner_cookie,
        preview_id,
        &preview_hash,
        &["candidate-0001", "candidate-9999"],
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(artifact_counts(&app, &workspace_id).await, (0, 0, 0, 0));

    let job = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": preview_id }, None)
        .await
        .expect("load rolled-back import intent")
        .expect("import intent exists");
    assert_eq!(job.apply_status.as_deref(), Some("ready"));
    assert!(job.apply_request_hash.is_none());
    assert!(job.apply_result.is_none());

    api.server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn wrong_admin_and_wrong_hash_leave_zero_writes() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let api = start_api(&app, &workspace_id).await;
    let (preview_id, preview_hash) = seed_preview(
        &app,
        &workspace_id,
        &api.owner_id,
        vec![preview_chunk("candidate-0001", "first", "first body")],
    )
    .await;

    let wrong_admin = apply_preview(
        &api,
        &api.other_cookie,
        preview_id,
        &preview_hash,
        &["candidate-0001"],
    )
    .await;
    assert_eq!(wrong_admin.status(), StatusCode::NOT_FOUND);

    let wrong_hash = apply_preview(
        &api,
        &api.owner_cookie,
        preview_id,
        &"0".repeat(64),
        &["candidate-0001"],
    )
    .await;
    assert_eq!(wrong_hash.status(), StatusCode::CONFLICT);
    assert_eq!(artifact_counts(&app, &workspace_id).await, (0, 0, 0, 0));

    let job = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": preview_id }, None)
        .await
        .expect("load untouched import intent")
        .expect("import intent exists");
    assert_eq!(job.apply_status.as_deref(), Some("ready"));
    assert!(job.apply_request_hash.is_none());
    assert!(job.apply_result.is_none());

    api.server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn shared_ingest_concurrent_replay_is_exactly_once() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let text = concat!(
        "---CHUNK: first---\n",
        "{\"title\":\"first\",\"body\":\"first body\"}\n",
        "---END CHUNK---\n",
        "---CHUNK: second---\n",
        "{\"title\":\"second\",\"body\":\"second body\"}\n",
        "---END CHUNK---\n"
    );

    let first = ingest_chunked_text(
        &app.state,
        &workspace_id,
        Some("account-a"),
        "shared-ingest",
        text,
    );
    let second = ingest_chunked_text(
        &app.state,
        &workspace_id,
        Some("account-a"),
        "shared-ingest",
        text,
    );
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first concurrent ingest");
    let second = second.expect("second concurrent ingest");
    assert_eq!(first, second, "concurrent ingest must return one receipt");

    let replay = ingest_chunked_text(
        &app.state,
        &workspace_id,
        Some("account-a"),
        "shared-ingest",
        text,
    )
    .await
    .expect("replay committed ingest");
    assert_eq!(replay, first);
    assert_eq!(artifact_counts(&app, &workspace_id).await, (1, 2, 2, 2));
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn shared_ingest_catalog_failure_rolls_back_every_artifact() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "catalog_rebuild_jobs",
                "validator": { "workspace_id": { "$ne": &workspace_id } },
                "validationLevel": "strict",
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install catalog rejection validator");

    let result = ingest_chunked_text(
        &app.state,
        &workspace_id,
        Some("account-a"),
        "rollback-ingest",
        concat!(
            "---CHUNK: first---\n",
            "{\"title\":\"first\",\"body\":\"first body\"}\n",
            "---END CHUNK---\n"
        ),
    )
    .await;
    assert!(result.is_err(), "catalog rejection must fail the ingest");
    assert_eq!(artifact_counts(&app, &workspace_id).await, (0, 0, 0, 0));
    app.cleanup().await;
}

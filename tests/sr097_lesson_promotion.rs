//! SR-097: Lesson promotion is one idempotent transaction.
//! Requires a replica-set MongoDB and is ignored by default.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::session::{authenticate, bootstrap_admin_if_needed, create_session};
use wechatagent::auth::SESSION_COOKIE_NAME;
use wechatagent::routes::api_router;

use crate::common::TestApp;

async fn start_api(app: &TestApp) -> (String, String, tokio::task::JoinHandle<()>) {
    let workspace = app.state.config.default_workspace_id.clone();
    bootstrap_admin_if_needed(
        &app.state.db,
        Some("sr097_admin"),
        Some("sr097-test-password"),
        Some(&workspace),
    )
    .await
    .expect("bootstrap admin");
    let admin = authenticate(&app.state.db, "sr097_admin", "sr097-test-password")
        .await
        .expect("authenticate admin");
    let session = create_session(&app.state.db, &admin, 1, &workspace)
        .await
        .expect("create session");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind API");
    let address = listener.local_addr().expect("API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

async fn seed_lesson(app: &TestApp, suffix: &str) -> (ObjectId, String) {
    let id = ObjectId::new();
    let workspace = &app.state.config.default_workspace_id;
    let lesson_id = format!("{workspace}::success-{suffix}");
    let now = DateTime::now();
    app.state
        .db
        .raw()
        .collection::<Document>("lessons_learned")
        .insert_one(
            doc! {
                "_id": id,
                "workspace_id": workspace,
                "lesson_id": &lesson_id,
                "pattern_kind": "success",
                "count": 1_i64,
                "sample_run_ids": ["run-1"],
                "review_status": "pending_review",
                "promoted_chunk_id": null,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("seed lesson");
    (id, lesson_id)
}

fn promotion_body() -> serde_json::Value {
    serde_json::json!({
        "title": "A reviewed lesson",
        "body": "A durable peer case that still requires chunk review.",
        "summary": "reviewed lesson"
    })
}

#[tokio::test]
#[ignore]
async fn concurrent_and_repeated_promotions_converge_to_one_chunk() {
    let app = TestApp::start_repl_set().await;
    let (lesson_oid, lesson_id) = seed_lesson(&app, "concurrent").await;
    let (base_url, cookie, server) = start_api(&app).await;
    let url = format!("{base_url}/admin/lessons-learned/{lesson_id}/promote-to-peer-case");
    let client = reqwest::Client::new();
    let first = client
        .post(&url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&promotion_body())
        .send();
    let second = client
        .post(&url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&promotion_body())
        .send();
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first promotion response");
    let second = second.expect("second promotion response");
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let first: serde_json::Value = first.json().await.expect("first JSON");
    let second: serde_json::Value = second.json().await.expect("second JSON");
    assert_eq!(first["promotedChunkId"], lesson_oid.to_hex());
    assert_eq!(second["promotedChunkId"], lesson_oid.to_hex());
    assert_ne!(first["alreadyPromoted"], second["alreadyPromoted"]);

    let replay = client
        .post(&url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "title": "ignored replay", "body": "ignored replay" }))
        .send()
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay: serde_json::Value = replay.json().await.expect("replay JSON");
    assert_eq!(replay["promotedChunkId"], lesson_oid.to_hex());
    assert_eq!(replay["alreadyPromoted"], true);

    let workspace = &app.state.config.default_workspace_id;
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(
                doc! {
                    "workspace_id": workspace,
                    "provenance.source": "lesson_promotion",
                    "provenance.source_doc_id": &lesson_id,
                },
                None,
            )
            .await
            .expect("count chunks"),
        1
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": workspace,
                    "kind": "lesson_promoted_to_peer_case",
                    "details.lesson_id": &lesson_id,
                },
                None,
            )
            .await
            .expect("count events"),
        1
    );
    let lesson = app
        .state
        .db
        .raw()
        .collection::<Document>("lessons_learned")
        .find_one(doc! { "_id": lesson_oid }, None)
        .await
        .expect("load lesson")
        .expect("lesson exists");
    assert_eq!(lesson.get_str("review_status"), Ok("promoted"));
    let expected_chunk_id = lesson_oid.to_hex();
    assert_eq!(
        lesson.get_str("promoted_chunk_id").ok(),
        Some(expected_chunk_id.as_str())
    );

    server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn audit_failure_rolls_back_chunk_and_lesson_then_retry_succeeds() {
    let app = TestApp::start_repl_set().await;
    let (lesson_oid, lesson_id) = seed_lesson(&app, "rollback").await;
    let lessons = app.state.db.raw().collection::<Document>("lessons_learned");
    let before = lessons
        .find_one(doc! { "_id": lesson_oid }, None)
        .await
        .expect("load before")
        .expect("lesson before");
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "agent_events",
                "validator": { "kind": { "$ne": "lesson_promoted_to_peer_case" } },
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install event rejection validator");

    let (base_url, cookie, server) = start_api(&app).await;
    let url = format!("{base_url}/admin/lessons-learned/{lesson_id}/promote-to-peer-case");
    let failed = reqwest::Client::new()
        .post(&url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&promotion_body())
        .send()
        .await
        .expect("failed response");
    assert_eq!(failed.status(), StatusCode::BAD_GATEWAY);
    let after = lessons
        .find_one(doc! { "_id": lesson_oid }, None)
        .await
        .expect("load after")
        .expect("lesson after");
    assert_eq!(after, before, "failed audit must roll the Lesson CAS back");
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(doc! { "_id": lesson_oid }, None)
            .await
            .expect("count rolled-back chunk"),
        0
    );

    app.state
        .db
        .raw()
        .run_command(
            doc! { "collMod": "agent_events", "validator": {}, "validationAction": "error" },
            None,
        )
        .await
        .expect("remove event rejection validator");
    let retried = reqwest::Client::new()
        .post(&url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&promotion_body())
        .send()
        .await
        .expect("retry response");
    assert_eq!(retried.status(), StatusCode::OK);

    server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn migration_backfills_exact_pair_and_fails_before_ambiguous_write() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let lessons = app.state.db.raw().collection::<Document>("lessons_learned");
    let chunks = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_knowledge_chunks");
    let legacy_chunk = ObjectId::new();
    let legacy_lesson = format!("{workspace}::legacy");
    lessons
        .insert_one(
            doc! {
                "_id": ObjectId::new(), "workspace_id": &workspace,
                "lesson_id": &legacy_lesson, "pattern_kind": "success",
                "review_status": "promoted", "promoted_chunk_id": legacy_chunk.to_hex(),
                "created_at": DateTime::now(), "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert legacy lesson");
    chunks
        .insert_one(
            doc! {
                "_id": legacy_chunk, "workspace_id": &workspace,
                "chunk_type": "peer_case", "business_context": "lessons_learned::success",
            },
            None,
        )
        .await
        .expect("insert legacy chunk");
    wechatagent::db::migrations::m055_lesson_promotion_identity::run_step(&app.state.db)
        .await
        .expect("backfill exact pair");
    let backfilled = chunks
        .find_one(doc! { "_id": legacy_chunk }, None)
        .await
        .expect("load backfilled")
        .expect("backfilled chunk");
    assert_eq!(
        backfilled
            .get_document("provenance")
            .and_then(|p| p.get_str("source_doc_id")),
        Ok(legacy_lesson.as_str())
    );

    let pending_chunk = ObjectId::new();
    let pending_lesson = format!("{workspace}::pending-backfill");
    lessons
        .insert_one(
            doc! {
                "_id": ObjectId::new(), "workspace_id": &workspace,
                "lesson_id": &pending_lesson, "pattern_kind": "success",
                "review_status": "promoted", "promoted_chunk_id": pending_chunk.to_hex(),
                "created_at": DateTime::now(), "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert pending backfill lesson");
    chunks
        .insert_many(
            [
                doc! {
                    "_id": pending_chunk, "workspace_id": &workspace,
                    "chunk_type": "peer_case", "business_context": "lessons_learned::success",
                },
                doc! {
                    "_id": ObjectId::new(), "workspace_id": &workspace,
                    "chunk_type": "peer_case", "business_context": "lessons_learned::orphan",
                },
            ],
            None,
        )
        .await
        .expect("insert pending and orphan chunks");
    let error =
        wechatagent::db::migrations::m055_lesson_promotion_identity::run_step(&app.state.db)
            .await
            .expect_err("orphan must fail before any backfill");
    assert!(error.to_string().contains("orphan"));
    let untouched = chunks
        .find_one(doc! { "_id": pending_chunk }, None)
        .await
        .expect("load untouched")
        .expect("untouched chunk");
    assert!(matches!(
        untouched.get("provenance"),
        None | Some(Bson::Null)
    ));

    app.cleanup().await;
}

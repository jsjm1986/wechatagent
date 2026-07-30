//! SR-117 auto-ingest source claim/finalize recovery redlines.
//!
//! The tests use the production claim and transactional finalize primitives.
//! A replica-set MongoDB is required because a successful fetched-content
//! finalize commits the knowledge graph and source checkpoint atomically.

#![cfg(test)]

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::knowledge_wiki::ingest_worker::{
    claim_due_source_for_redline, finalize_claimed_content_for_redline,
};
use wechatagent::models::IngestSource;

use crate::common::TestApp;

const MARKDOWN: &str = concat!(
    "---CHUNK: current---\n",
    "{\"title\":\"current source\",\"body\":\"current source body\"}\n",
    "---END CHUNK---\n"
);

fn source(workspace_id: &str, source_id: &str, label: &str) -> IngestSource {
    let now = DateTime::now();
    IngestSource {
        id: None,
        source_id: source_id.to_string(),
        workspace_id: workspace_id.to_string(),
        source_generation: 1,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        kind: "rss".to_string(),
        url: "https://example.com/feed.xml".to_string(),
        schedule_minutes: 60,
        label: Some(label.to_string()),
        last_fetched_at: None,
        last_etag: None,
        last_content_hash: None,
        last_error: None,
        status: "active".to_string(),
        failure_streak: 0,
        ingest_count: 0,
        created_at: now,
        updated_at: now,
    }
}

async fn artifact_counts(app: &TestApp, workspace_id: &str) -> (u64, u64, u64, u64) {
    let documents = app
        .state
        .db
        .operation_knowledge_documents()
        .count_documents(doc! { "workspace_id": workspace_id }, None)
        .await
        .expect("count documents");
    let chunks = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": workspace_id }, None)
        .await
        .expect("count chunks");
    let revisions = app
        .state
        .db
        .chunk_revisions()
        .count_documents(doc! { "workspace_id": workspace_id }, None)
        .await
        .expect("count revisions");
    let catalog = app
        .state
        .db
        .catalog_rebuild_jobs()
        .count_documents(doc! { "workspace_id": workspace_id }, None)
        .await
        .expect("count catalog intents");
    (documents, chunks, revisions, catalog)
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn migration_backfills_legacy_generations_without_destroying_claim_data() {
    let app = TestApp::start_repl_set().await;
    let source_id = format!("sr117-{}", ObjectId::new().to_hex());
    let locked_until = DateTime::from_millis(DateTime::now().timestamp_millis() + 60_000);
    app.state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("ingest_sources")
        .insert_one(
            doc! {
                "source_id": &source_id,
                "workspace_id": "legacy-workspace",
                "kind": "rss",
                "url": "https://example.com/legacy.xml",
                "schedule_minutes": 60i64,
                "status": "active",
                "failure_streak": 0i32,
                "ingest_count": 0i64,
                "worker_id": "legacy-worker",
                "claim_token": "legacy-token",
                "locked_until": locked_until,
                "created_at": DateTime::now(),
                "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert legacy source");

    wechatagent::db::migrations::m053_ingest_source_claims::run_step(&app.state.db)
        .await
        .expect("first migration run");
    wechatagent::db::migrations::m053_ingest_source_claims::run_step(&app.state.db)
        .await
        .expect("idempotent migration rerun");

    let row = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("ingest_sources")
        .find_one(doc! { "source_id": &source_id }, None)
        .await
        .expect("load migrated source")
        .expect("migrated source exists");
    assert_eq!(row.get_i64("source_generation").unwrap(), 1);
    assert_eq!(row.get_i64("claim_generation").unwrap(), 0);
    assert_eq!(row.get_str("worker_id").unwrap(), "legacy-worker");
    assert_eq!(row.get_str("claim_token").unwrap(), "legacy-token");
    assert_eq!(row.get_datetime("locked_until").unwrap(), &locked_until);
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn configuration_change_fences_late_result_and_rolls_back_all_artifacts() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr117-{}", ObjectId::new().to_hex());
    let source_id = format!("source-{}", ObjectId::new().to_hex());
    app.state
        .db
        .ingest_sources()
        .insert_one(source(&workspace_id, &source_id, "stale-source"), None)
        .await
        .expect("insert source");
    let stale_claim = claim_due_source_for_redline(&app.state, &source_id, "old-worker")
        .await
        .expect("claim source")
        .expect("old worker owns claim");

    let changed = app
        .state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": &source_id, "source_generation": 1i64 },
            doc! {
                "$set": {
                    "url": "https://example.com/new-feed.xml",
                    "updated_at": DateTime::now(),
                },
                "$inc": { "source_generation": 1i64 },
                "$unset": {
                    "worker_id": "",
                    "claim_token": "",
                    "locked_until": "",
                },
            },
            None,
        )
        .await
        .expect("change source configuration");
    assert_eq!(changed.matched_count, 1);

    assert!(
        finalize_claimed_content_for_redline(&app.state, &stale_claim, MARKDOWN)
            .await
            .is_err(),
        "a claim from the old URL generation must lose ownership"
    );
    assert_eq!(artifact_counts(&app, &workspace_id).await, (0, 0, 0, 0));
    let current = app
        .state
        .db
        .ingest_sources()
        .find_one(doc! { "source_id": &source_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.source_generation, 2);
    assert_eq!(current.url, "https://example.com/new-feed.xml");
    assert!(current.last_content_hash.is_none());
    assert_eq!(current.ingest_count, 0);
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn expired_lease_is_reclaimed_and_only_new_owner_can_commit() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr117-{}", ObjectId::new().to_hex());
    let source_id = format!("source-{}", ObjectId::new().to_hex());
    app.state
        .db
        .ingest_sources()
        .insert_one(source(&workspace_id, &source_id, "reclaimed-source"), None)
        .await
        .expect("insert source");
    let stale = claim_due_source_for_redline(&app.state, &source_id, "crashed-worker")
        .await
        .expect("first claim")
        .expect("first owner");
    app.state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": &source_id },
            doc! { "$set": { "locked_until": DateTime::from_millis(0) } },
            None,
        )
        .await
        .expect("expire lease");
    let current = claim_due_source_for_redline(&app.state, &source_id, "recovery-worker")
        .await
        .expect("reclaim")
        .expect("new owner");
    assert_eq!(current.claim_generation, stale.claim_generation + 1);
    assert_ne!(current.claim_token, stale.claim_token);

    assert!(
        finalize_claimed_content_for_redline(&app.state, &stale, MARKDOWN)
            .await
            .is_err(),
        "expired owner must not commit"
    );
    finalize_claimed_content_for_redline(&app.state, &current, MARKDOWN)
        .await
        .expect("new owner commits");
    assert_eq!(artifact_counts(&app, &workspace_id).await, (1, 1, 1, 1));
    let settled = app
        .state
        .db
        .ingest_sources()
        .find_one(doc! { "source_id": &source_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert!(settled.claim_token.is_none());
    assert!(settled.worker_id.is_none());
    assert!(settled.locked_until.is_none());
    assert_eq!(settled.ingest_count, 1);
    assert!(settled.last_content_hash.is_some());
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn concurrent_workers_produce_one_claim_and_one_committed_graph() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr117-{}", ObjectId::new().to_hex());
    let source_id = format!("source-{}", ObjectId::new().to_hex());
    app.state
        .db
        .ingest_sources()
        .insert_one(source(&workspace_id, &source_id, "concurrent-source"), None)
        .await
        .expect("insert source");

    let (left, right) = tokio::join!(
        claim_due_source_for_redline(&app.state, &source_id, "worker-left"),
        claim_due_source_for_redline(&app.state, &source_id, "worker-right")
    );
    let claims = [left.unwrap(), right.unwrap()];
    assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
    let winner = claims.into_iter().flatten().next().unwrap();
    finalize_claimed_content_for_redline(&app.state, &winner, MARKDOWN)
        .await
        .expect("winning worker commits");

    assert_eq!(artifact_counts(&app, &workspace_id).await, (1, 1, 1, 1));
    let settled = app
        .state
        .db
        .ingest_sources()
        .find_one(doc! { "source_id": &source_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.claim_generation, 1);
    assert_eq!(settled.ingest_count, 1);
    app.cleanup().await;
}

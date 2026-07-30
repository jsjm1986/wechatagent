//! SR-115 catalog projection recovery redlines.
//!
//! These tests call the production worker batch entrypoint. They require a
//! replica-set MongoDB because catalog projection and job finalization commit
//! in one transaction.

#![cfg(test)]

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::knowledge_wiki::catalog_rebuild::run_catalog_rebuild_batch;
use wechatagent::models::{CatalogRebuildJob, OperationKnowledgeChunk, OperationKnowledgeDocument};

use crate::common::TestApp;

fn document(
    workspace_id: &str,
    document_id: ObjectId,
    desired: i64,
    applied: i64,
) -> OperationKnowledgeDocument {
    let now = DateTime::now();
    OperationKnowledgeDocument {
        id: Some(document_id),
        workspace_id: workspace_id.to_string(),
        account_id: Some("account-a".to_string()),
        domain: "user_operations".to_string(),
        source_type: "manual".to_string(),
        source_name: None,
        title: "catalog recovery document".to_string(),
        summary: None,
        catalog_summary: None,
        routing_map: vec![],
        risk_notes: vec![],
        product_tags: vec![],
        business_topics: vec![],
        raw_content: None,
        content_hash: None,
        line_index: vec![],
        section_index: vec![],
        status: "active".to_string(),
        version: 1,
        created_at: now,
        updated_at: now,
        catalog_summary_persisted: Some("stale snapshot".to_string()),
        catalog_version: Some(7),
        catalog_desired_generation: desired,
        catalog_applied_generation: applied,
    }
}

fn chunk(workspace_id: &str, document_id: ObjectId, title: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: Some("account-a".to_string()),
        document_id: Some(document_id),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("summary for {title}")),
        status: "active".to_string(),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
        ..Default::default()
    }
}

fn job(
    workspace_id: &str,
    document_id: ObjectId,
    job_id: &str,
    generation: i64,
    status: &str,
) -> CatalogRebuildJob {
    CatalogRebuildJob {
        id: None,
        job_id: job_id.to_string(),
        workspace_id: workspace_id.to_string(),
        document_id,
        queued_at: DateTime::now(),
        target_generation: generation,
        status: status.to_string(),
        attempts: 0,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        next_retry_at: None,
        last_error: None,
        started_at: None,
        finished_at: None,
    }
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn expired_claim_and_out_of_order_generation_converge_without_stale_overwrite() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr115-{}", ObjectId::new().to_hex());
    let document_id = ObjectId::new();
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(document(&workspace_id, document_id, 2, 0), None)
        .await
        .expect("seed parent");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(
            chunk(&workspace_id, document_id, "latest catalog content"),
            None,
        )
        .await
        .expect("seed chunk");

    let mut expired = job(
        &workspace_id,
        document_id,
        "expired-generation-1",
        1,
        "processing",
    );
    expired.attempts = 1;
    expired.claim_generation = 1;
    expired.worker_id = Some("crashed-worker".to_string());
    expired.claim_token = Some("expired-token".to_string());
    expired.locked_until = Some(DateTime::from_millis(
        DateTime::now().timestamp_millis() - 60_000,
    ));
    app.state
        .db
        .catalog_rebuild_jobs()
        .insert_many(
            [
                expired,
                job(
                    &workspace_id,
                    document_id,
                    "latest-generation-2",
                    2,
                    "queued",
                ),
            ],
            None,
        )
        .await
        .expect("seed jobs");

    let processed = run_catalog_rebuild_batch(&app.state.db)
        .await
        .expect("run recovery batch");
    assert_eq!(processed, 2);

    let stale = app
        .state
        .db
        .catalog_rebuild_jobs()
        .find_one(doc! { "job_id": "expired-generation-1" }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stale.status, "superseded");
    assert_eq!(stale.attempts, 2, "expired lease must have been reclaimed");
    assert_eq!(stale.claim_generation, 2);
    assert!(stale.claim_token.is_none());

    let latest = app
        .state
        .db
        .catalog_rebuild_jobs()
        .find_one(doc! { "job_id": "latest-generation-2" }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.status, "done");

    let parent = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent.catalog_desired_generation, 2);
    assert_eq!(parent.catalog_applied_generation, 2);
    assert_eq!(parent.catalog_version, Some(8));
    assert!(parent
        .catalog_summary_persisted
        .as_deref()
        .is_some_and(|value| value.contains("latest catalog content")));
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn concurrent_workers_commit_current_generation_once() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr115-{}", ObjectId::new().to_hex());
    let document_id = ObjectId::new();
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(document(&workspace_id, document_id, 1, 0), None)
        .await
        .unwrap();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(
            chunk(&workspace_id, document_id, "one committed projection"),
            None,
        )
        .await
        .unwrap();
    app.state
        .db
        .catalog_rebuild_jobs()
        .insert_many(
            [
                job(&workspace_id, document_id, "duplicate-a", 1, "queued"),
                job(&workspace_id, document_id, "duplicate-b", 1, "queued"),
            ],
            None,
        )
        .await
        .unwrap();

    let (left, right) = tokio::join!(
        run_catalog_rebuild_batch(&app.state.db),
        run_catalog_rebuild_batch(&app.state.db)
    );
    left.expect("left worker");
    right.expect("right worker");

    let parent = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent.catalog_applied_generation, 1);
    assert_eq!(
        parent.catalog_version,
        Some(8),
        "projection may commit only once"
    );
    let done = app
        .state
        .db
        .catalog_rebuild_jobs()
        .count_documents(doc! { "document_id": document_id, "status": "done" }, None)
        .await
        .unwrap();
    let superseded = app
        .state
        .db
        .catalog_rebuild_jobs()
        .count_documents(
            doc! { "document_id": document_id, "status": "superseded" },
            None,
        )
        .await
        .unwrap();
    assert_eq!((done, superseded), (1, 1));
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn rolling_deploy_legacy_intent_is_upgraded_to_a_durable_generation() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr115-{}", ObjectId::new().to_hex());
    let document_id = ObjectId::new();
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(document(&workspace_id, document_id, 0, 0), None)
        .await
        .unwrap();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(
            chunk(&workspace_id, document_id, "legacy intent projection"),
            None,
        )
        .await
        .unwrap();
    app.state
        .db
        .catalog_rebuild_jobs()
        .insert_one(
            job(
                &workspace_id,
                document_id,
                "legacy-target-zero",
                0,
                "queued",
            ),
            None,
        )
        .await
        .unwrap();

    assert_eq!(run_catalog_rebuild_batch(&app.state.db).await.unwrap(), 1);

    let upgraded = app
        .state
        .db
        .catalog_rebuild_jobs()
        .find_one(doc! { "job_id": "legacy-target-zero" }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(upgraded.status, "done");
    assert_eq!(upgraded.target_generation, 1);
    let parent = app
        .state
        .db
        .operation_knowledge_documents()
        .find_one(doc! { "_id": document_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent.catalog_desired_generation, 1);
    assert_eq!(parent.catalog_applied_generation, 1);
    assert!(parent
        .catalog_summary_persisted
        .as_deref()
        .is_some_and(|value| value.contains("legacy intent projection")));
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn missing_parent_job_reaches_discarded_terminal_state() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr115-{}", ObjectId::new().to_hex());
    let document_id = ObjectId::new();
    app.state
        .db
        .catalog_rebuild_jobs()
        .insert_one(job(&workspace_id, document_id, "orphan", 1, "queued"), None)
        .await
        .unwrap();

    assert_eq!(run_catalog_rebuild_batch(&app.state.db).await.unwrap(), 1);
    let orphan = app
        .state
        .db
        .catalog_rebuild_jobs()
        .find_one(doc! { "job_id": "orphan" }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(orphan.status, "discarded");
    assert!(orphan.finished_at.is_some());
}

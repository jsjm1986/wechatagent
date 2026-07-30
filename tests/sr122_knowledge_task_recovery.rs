//! SR-122/SR-123 durable knowledge-task recovery redlines.
//!
//! These tests drive the production claim and transactional step helpers. A
//! replica-set MongoDB is required because a knowledge mutation and its task
//! outcome commit in one transaction.

#![cfg(test)]

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::knowledge_task::{
    claim_task_for_redline, commit_add_chunk_step_for_redline, StepVerdict,
};
use wechatagent::models::KnowledgeChatTask;

use crate::common::TestApp;

fn pending_task(task_id: ObjectId, workspace_id: &str) -> KnowledgeChatTask {
    KnowledgeChatTask {
        id: Some(task_id),
        workspace_id: workspace_id.to_string(),
        account_id: "account-a".to_string(),
        session_id: format!("sr122-{task_id}"),
        owner_admin_id: Some("admin-a".to_string()),
        operator_id: Some("operator-a".to_string()),
        cards: vec![],
        dispatch_binding: None,
        planned_steps: vec![doc! {
            "stepId": "add-1",
            "action": "add_chunk",
            "summary": "durable task draft",
        }],
        completed_steps: vec![],
        step_intents: vec![],
        status: "pending".to_string(),
        error_kind: None,
        attempts: 0,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        heartbeat_at: None,
        created_at: DateTime::now(),
        started_at: None,
        finished_at: None,
    }
}

fn add_payload() -> mongodb::bson::Document {
    doc! {
        "patch": {
            "title": "Recovered worker draft",
            "summary": "One durable draft after replay",
            "body": "A task mutation and its outcome must commit atomically.",
            "knowledgeType": "methodology",
        },
        "summary": "A task mutation and its outcome must commit atomically.",
    }
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn expired_reclaim_fences_old_owner_and_replay_has_one_side_effect() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = format!("sr122-{}", ObjectId::new().to_hex());
    let task_id = ObjectId::new();
    app.state
        .db
        .knowledge_chat_tasks()
        .insert_one(pending_task(task_id, &workspace_id), None)
        .await
        .expect("seed pending task");

    let stale = claim_task_for_redline(&app.state, task_id)
        .await
        .expect("claim stale owner")
        .expect("pending task claimed");
    assert_eq!(stale.claim_generation, 1);
    app.state
        .db
        .knowledge_chat_tasks()
        .update_one(
            doc! {
                "_id": task_id,
                "claim_token": stale.claim_token.as_deref().unwrap(),
            },
            doc! { "$set": {
                "locked_until": DateTime::from_millis(DateTime::now().timestamp_millis() - 1),
            } },
            None,
        )
        .await
        .expect("expire stale lease");

    let current = claim_task_for_redline(&app.state, task_id)
        .await
        .expect("reclaim expired task")
        .expect("expired task reclaimed");
    assert_eq!(current.claim_generation, 2);
    assert_ne!(current.claim_token, stale.claim_token);

    let stale_error =
        commit_add_chunk_step_for_redline(&app.state, &stale, "add-1", &add_payload())
            .await
            .expect_err("stale owner must be fenced");
    assert!(stale_error
        .to_string()
        .contains("knowledge_task_claim_lost"));
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        0
    );

    let committed =
        commit_add_chunk_step_for_redline(&app.state, &current, "add-1", &add_payload())
            .await
            .expect("current owner commits");
    assert_eq!(committed.verdict, StepVerdict::Committed);
    let chunk_id = committed.chunk_id.expect("committed chunk id");

    let replay_error =
        commit_add_chunk_step_for_redline(&app.state, &current, "add-1", &add_payload())
            .await
            .expect_err("stable stepId must fence replay");
    assert!(replay_error
        .to_string()
        .contains("knowledge_task_claim_lost"));

    let saved = app
        .state
        .db
        .knowledge_chat_tasks()
        .find_one(doc! { "_id": task_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.completed_steps.len(), 1);
    assert_eq!(
        saved.completed_steps[0].get_str("status").unwrap(),
        "committed"
    );
    assert_eq!(
        saved.completed_steps[0].get_str("chunkId").unwrap(),
        chunk_id
    );
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .unwrap(),
        1
    );
    let persisted_chunk = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": ObjectId::parse_str(&chunk_id).unwrap(),
                "workspace_id": &workspace_id,
            },
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        persisted_chunk.account_id.as_deref(),
        Some("account-a"),
        "account-scoped task output must not widen into workspace-shared knowledge"
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "chunk_id": &chunk_id },
                None,
            )
            .await
            .unwrap(),
        1
    );
    app.cleanup().await;
}

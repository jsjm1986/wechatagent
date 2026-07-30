//! SR-036 real-Mongo regression: outcome task dedupe is workspace scoped.

mod common;

use mongodb::{
    bson::{doc, DateTime},
    error::{ErrorKind, WriteFailure},
};
use wechatagent::models::AgentTask;

fn outcome_task(workspace_id: &str, content: &str) -> AgentTask {
    let now = DateTime::now();
    AgentTask {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: "shared-account".to_string(),
        contact_wxid: "_outcome_aggregation".to_string(),
        kind: "outcome_aggregation".to_string(),
        run_at: now,
        expires_at: None,
        content: content.to_string(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: false,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    }
}

fn is_duplicate_key(error: &mongodb::error::Error) -> bool {
    match error.kind.as_ref() {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            matches!(write_error.code, 11000 | 11001)
        }
        ErrorKind::BulkWrite(failure) => failure
            .write_errors
            .as_ref()
            .is_some_and(|errors| errors.iter().any(|item| matches!(item.code, 11000 | 11001))),
        _ => false,
    }
}

#[tokio::test]
#[ignore]
async fn outcome_task_unique_key_allows_same_account_content_in_distinct_workspaces() {
    let app = common::TestApp::start().await;
    let content = r#"{"horizon":"7d","date":"2026-07-19"}"#;

    app.state
        .db
        .tasks()
        .insert_one(outcome_task("workspace-a", content), None)
        .await
        .expect("insert workspace-a outcome task");
    app.state
        .db
        .tasks()
        .insert_one(outcome_task("workspace-b", content), None)
        .await
        .expect("same account/content must be legal in another workspace");

    let duplicate = app
        .state
        .db
        .tasks()
        .insert_one(outcome_task("workspace-a", content), None)
        .await
        .expect_err("same workspace/account/content must be rejected");
    assert!(
        is_duplicate_key(&duplicate),
        "expected Mongo duplicate-key, got {duplicate:?}"
    );

    let stored = app
        .state
        .db
        .tasks()
        .count_documents(
            doc! {
                "kind": "outcome_aggregation",
                "account_id": "shared-account",
                "content": content,
            },
            None,
        )
        .await
        .expect("count outcome tasks");
    assert_eq!(stored, 2, "one legal task must remain per workspace");

    app.cleanup().await;
}

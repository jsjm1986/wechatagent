//! SR-126: knowledge worker evidence must assert business verdicts and side effects.

#![cfg(test)]

mod common;

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;
use wechatagent::knowledge_task::{execute_step, run_task, ChatProgressBus, StepVerdict};
use wechatagent::models::KnowledgeChatTask;

use crate::common::TestApp;

async fn exercise_step_verdicts(app: &TestApp) -> anyhow::Result<()> {
    let out = execute_step(&app.state, "ws_worker", "acc", "fix_chunk", &doc! {}).await?;
    anyhow::ensure!(
        out.verdict == StepVerdict::Failed,
        "fix_chunk={:?}",
        out.verdict
    );
    let manual = execute_step(&app.state, "ws_worker", "acc", "review_evolution", &doc! {}).await?;
    anyhow::ensure!(manual.verdict == StepVerdict::NeedsManual);
    let noop = execute_step(&app.state, "ws_worker", "acc", "analyze_logs", &doc! {}).await?;
    anyhow::ensure!(noop.verdict == StepVerdict::Noop);
    // mutating action（add_chunk / retag / dismiss）只走两阶段提交路径
    // （prepare → persist_step_intent → commit），execute_step 的重复直接实现已
    // 退役删除——直调必须报 unsupported，防止漂移实现复活。
    for action in ["add_chunk", "retag", "dismiss"] {
        anyhow::ensure!(
            execute_step(&app.state, "ws_worker", "acc", action, &doc! {})
                .await
                .is_err(),
            "{action} must be rejected by execute_step (two-phase only)"
        );
    }
    anyhow::ensure!(
        execute_step(&app.state, "ws_worker", "acc", "drop_table", &doc! {})
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
#[ignore]
async fn execute_step_reports_typed_business_verdicts() {
    let app = TestApp::start().await;
    let result = exercise_step_verdicts(&app).await;
    app.cleanup().await;
    result.expect("exercise typed worker verdicts");
}

async fn exercise_committed_add(app: &TestApp) -> anyhow::Result<()> {
    app.llm.push_response(json!({
        "patch": {
            "title": "Worker draft",
            "summary": "Worker-created auditable draft",
            "body": "Worker-created auditable draft body",
            "knowledgeType": "methodology"
        },
        "missingFields": [],
        "followupQuestions": [],
        "naturalReply": "Drafted"
    }));
    // add_chunk 是 mutating action：生产唯一路径是 run_task 内的两阶段提交
    // （prepare 产 intent → persist_step_intent → commit_mutating_step_once 事务落库）。
    let id = ObjectId::new();
    let now = DateTime::now();
    let task = KnowledgeChatTask {
        id: Some(id),
        workspace_id: "ws_worker".into(),
        account_id: "acc".into(),
        session_id: "sr126-worker-add".into(),
        owner_admin_id: Some("admin".into()),
        operator_id: Some("operator".into()),
        cards: vec![],
        dispatch_binding: None,
        planned_steps: vec![doc! {
            "stepId": "add-1",
            "cardId": "card-add",
            "action": "add_chunk",
            "summary": "Worker-created auditable draft body",
        }],
        completed_steps: vec![],
        step_intents: vec![],
        status: "pending".into(),
        error_kind: None,
        attempts: 0,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        heartbeat_at: None,
        created_at: now,
        started_at: None,
        finished_at: None,
    };
    app.state
        .db
        .knowledge_chat_session_seqs()
        .insert_one(
            doc! {
                "_id": "ws_worker|sr126-worker-add",
                "workspace_id": "ws_worker",
                "account_id": "acc",
                "session_id": "sr126-worker-add",
                "owner_admin_id": "admin",
                "seq": 0_i64,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await?;
    app.state
        .db
        .knowledge_chat_tasks()
        .insert_one(&task, None)
        .await?;
    run_task(&app.state, &Arc::new(ChatProgressBus::new()), task).await?;

    let saved = app
        .state
        .db
        .knowledge_chat_tasks()
        .find_one(doc! { "_id": id }, None)
        .await?
        .ok_or_else(|| anyhow::anyhow!("task disappeared"))?;
    anyhow::ensure!(
        saved.status == "completed",
        "task should complete: status={} error_kind={:?}",
        saved.status,
        saved.error_kind
    );
    // 两阶段证据：intent 先持久化，再 commit。
    anyhow::ensure!(
        saved
            .step_intents
            .iter()
            .any(|intent| intent.get_str("stepId").ok() == Some("add-1")),
        "step intent must be persisted before commit"
    );
    let entry = saved
        .completed_steps
        .iter()
        .find(|step| step.get_str("stepId").ok() == Some("add-1"))
        .ok_or_else(|| anyhow::anyhow!("committed step entry missing"))?;
    anyhow::ensure!(entry.get_str("status")? == "committed");
    let chunk_id = ObjectId::parse_str(entry.get_str("chunkId")?)?;
    let row = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id, "workspace_id": "ws_worker" }, None)
        .await?
        .ok_or_else(|| anyhow::anyhow!("committed verdict without persisted chunk"))?;
    anyhow::ensure!(row.status == "draft");
    anyhow::ensure!(row.integrity_status.as_deref() == Some("needs_review"));
    Ok(())
}

#[tokio::test]
#[ignore]
async fn committed_add_has_real_draft_side_effect() {
    // add_chunk commits a chunk + immutable revision in one transaction (two-phase path).
    let app = TestApp::start_repl_set().await;
    let result = exercise_committed_add(&app).await;
    app.cleanup().await;
    result.expect("committed add must persist a draft");
}

async fn exercise_task_summary(app: &TestApp) -> anyhow::Result<()> {
    let id = ObjectId::new();
    let now = DateTime::now();
    let task = KnowledgeChatTask {
        id: Some(id),
        workspace_id: "ws_worker".into(),
        account_id: "acc".into(),
        session_id: "sr126-worker-summary".into(),
        owner_admin_id: Some("admin".into()),
        operator_id: Some("operator".into()),
        cards: vec![],
        dispatch_binding: None,
        planned_steps: vec![
            doc! { "stepId": "noop", "cardId": "c1", "action": "analyze_logs" },
            doc! { "stepId": "manual", "cardId": "c2", "action": "review_evolution" },
            doc! { "stepId": "failed", "cardId": "c3", "action": "dismiss" },
        ],
        completed_steps: vec![],
        step_intents: vec![],
        status: "pending".into(),
        error_kind: None,
        attempts: 0,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        heartbeat_at: None,
        created_at: now,
        started_at: None,
        finished_at: None,
    };
    app.state
        .db
        .knowledge_chat_session_seqs()
        .insert_one(
            doc! {
                "_id": "ws_worker|sr126-worker-summary",
                "workspace_id": "ws_worker",
                "account_id": "acc",
                "session_id": "sr126-worker-summary",
                "owner_admin_id": "admin",
                "seq": 0_i64,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await?;
    app.state
        .db
        .knowledge_chat_tasks()
        .insert_one(&task, None)
        .await?;
    run_task(&app.state, &Arc::new(ChatProgressBus::new()), task).await?;
    let saved = app
        .state
        .db
        .knowledge_chat_tasks()
        .find_one(doc! { "_id": id }, None)
        .await?
        .ok_or_else(|| anyhow::anyhow!("task disappeared"))?;
    anyhow::ensure!(saved.status == "failed");
    anyhow::ensure!(saved.error_kind.as_deref() == Some("knowledge_task_step_failed"));
    let statuses: Vec<&str> = saved
        .completed_steps
        .iter()
        .filter_map(|step| step.get_str("status").ok())
        .collect();
    anyhow::ensure!(statuses == ["noop", "needs_manual", "failed"]);
    let failed = saved
        .completed_steps
        .iter()
        .find(|step| step.get_str("status").ok() == Some("failed"))
        .ok_or_else(|| anyhow::anyhow!("failed step missing"))?;
    anyhow::ensure!(!failed.get_str("error")?.trim().is_empty());
    let summary = app
        .state
        .db
        .knowledge_chat_turns()
        .find_one(
            doc! { "session_id": "sr126-worker-summary", "kind": "task_summary" },
            None,
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("summary turn missing"))?;
    let detail = summary
        .attachments
        .first()
        .ok_or_else(|| anyhow::anyhow!("summary detail missing"))?;
    anyhow::ensure!(detail.get_array("noopStepIds")?.len() == 1);
    anyhow::ensure!(detail.get_array("needsManualStepIds")?.len() == 1);
    anyhow::ensure!(detail.get_array("failedStepIds")?.len() == 1);
    anyhow::ensure!(detail.get_i32("committedCount")? == 0);
    Ok(())
}

#[tokio::test]
#[ignore]
async fn run_task_persists_mixed_verdict_buckets() {
    let app = TestApp::start().await;
    let result = exercise_task_summary(&app).await;
    app.cleanup().await;
    result.expect("run_task must persist business verdict buckets");
}

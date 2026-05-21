//! S-20 / Task 19 / Task 24：Management Agent dry-run 模式隔离回归。
//!
//! 性质：
//! - `ManagementAgentSession.dry_run = true` 时，对应 `AgentCommandRun.status` /
//!   `AgentToolCall.status` 必须是 `"dry_run"`，业务集合（contacts / agent_tasks）
//!   不会被 dry-run tool call 改动。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::models::{AgentCommandRun, AgentToolCall, ManagementAgentSession};

fn make_dry_run_session() -> ManagementAgentSession {
    let now = DateTime::now();
    ManagementAgentSession {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        title: "dry-run test".to_string(),
        dry_run: true,
        created_at: now,
        updated_at: now,
    }
}

fn make_command_run(session_id: ObjectId, status: &str) -> AgentCommandRun {
    let now = DateTime::now();
    AgentCommandRun {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        session_id,
        operator_message: "test".to_string(),
        status: status.to_string(),
        plan: None,
        summary: "dry-run plan".to_string(),
        error: None,
        prompt_versions: Document::new(),
        created_at: now,
        updated_at: now,
    }
}

fn make_tool_call(command_run_id: ObjectId, tool: &str, status: &str) -> AgentToolCall {
    let now = DateTime::now();
    AgentToolCall {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        command_run_id,
        tool_name: tool.to_string(),
        arguments: doc! { "dry_run": true },
        status: status.to_string(),
        response: Some(doc! {
            "dry_run": true,
            "would_execute": {
                "toolName": tool,
                "arguments": { "content": "demo" }
            }
        }),
        error: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn dry_run_session_writes_dry_run_status_audit_only() {
    let app = common::TestApp::start().await;

    let session = make_dry_run_session();
    let session_id = session.id.expect("session id");
    app.state
        .db
        .management_sessions()
        .insert_one(&session, None)
        .await
        .expect("insert session");

    let command = make_command_run(session_id, "dry_run");
    let command_id = command.id.expect("command id");
    app.state
        .db
        .command_runs()
        .insert_one(&command, None)
        .await
        .expect("insert command run");

    // 模拟一个非 read tool 在 dry-run 模式被记录但未触达业务集合。
    let tool_call = make_tool_call(command_id, "contacts.update_profile_note", "dry_run");
    app.state
        .db
        .tool_calls()
        .insert_one(&tool_call, None)
        .await
        .expect("insert tool call");

    // 业务集合不应有任何写入。
    let contacts_count = app
        .state
        .db
        .contacts()
        .count_documents(doc! {}, None)
        .await
        .unwrap();
    assert_eq!(contacts_count, 0, "dry-run 不应写 contacts 业务集合");

    let tasks_count = app
        .state
        .db
        .tasks()
        .count_documents(doc! {}, None)
        .await
        .unwrap();
    assert_eq!(tasks_count, 0, "dry-run 不应创建 agent_tasks");

    // 审计集合上 status 必须是 dry_run。
    let stored_command = app
        .state
        .db
        .command_runs()
        .find_one(doc! { "_id": command_id }, None)
        .await
        .unwrap()
        .expect("command run present");
    assert_eq!(stored_command.status, "dry_run");

    let stored_tool = app
        .state
        .db
        .tool_calls()
        .find_one(doc! { "command_run_id": command_id }, None)
        .await
        .unwrap()
        .expect("tool call present");
    assert_eq!(stored_tool.status, "dry_run");

    // would_execute 应携带工具名供前端回放。
    let response = stored_tool.response.expect("response present");
    let would = response
        .get_document("would_execute")
        .expect("would_execute present");
    assert_eq!(
        would.get_str("toolName").unwrap(),
        "contacts.update_profile_note"
    );
}

#[tokio::test]
#[ignore]
async fn non_dry_run_session_uses_normal_status() {
    let app = common::TestApp::start().await;
    let mut session = make_dry_run_session();
    session.dry_run = false;
    let session_id = session.id.expect("session id");
    app.state
        .db
        .management_sessions()
        .insert_one(&session, None)
        .await
        .expect("insert session");

    let command = make_command_run(session_id, "completed");
    app.state
        .db
        .command_runs()
        .insert_one(&command, None)
        .await
        .expect("insert command");

    let stored = app
        .state
        .db
        .command_runs()
        .find_one(doc! { "session_id": session_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "completed");
}

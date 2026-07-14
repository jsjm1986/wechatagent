//! W-Batch3 [S-01]/[S-02] 回归：admin `review_task_now` 的原子 CAS claim。
//!
//! 修复前 `review_task_now` 置 running 的 update filter 只有 `{_id, workspace_id}`
//! 无 status 前置、也不写 `claimed_at`：
//!   - S-01：与串行 worker 已 claim 的同一任务并发跑第二份 handler（双跑/双发）；
//!   - S-02：handler 失败停 running 后落入 reclaim 双分支盲区，本进程永不回收。
//! 修复后 filter 加 `status ∈ {pending,retry,failed}` CAS + 写 `claimed_at`，
//! 已 running / 终态的任务被拒（Conflict），可复核态被原子认领。
//!
//! `#[ignore]` 需 Docker（testcontainers MongoDB）；CI:
//! `cargo test --test review_task_now_claim -- --ignored`。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::AgentTask;
use wechatagent::routes::tasks::review_task_now;

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "op_admin".to_string(),
        username: "op_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一条指定状态的 memory_consolidation 任务（该 kind 无候选时走 sent 早退，
/// 不触达 LLM，适合无 MCP/LLM 的 claim 语义回归）。
fn task_with_status(ws: &str, acc: &str, wxid: &str, status: &str) -> AgentTask {
    let now = DateTime::now();
    AgentTask {
        id: Some(ObjectId::new()),
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        contact_wxid: wxid.to_string(),
        kind: "memory_consolidation".to_string(),
        run_at: now,
        expires_at: None,
        content: String::new(),
        status: status.to_string(),
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

/// S-01：任务已 running（worker 正跑）时，admin review_task_now 必须被 CAS 拒绝
/// （Conflict），绝不起第二份 handler。
#[tokio::test]
#[ignore]
async fn review_task_now_rejects_running_task() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();

    let task = task_with_status(&ws, &acc, "wx_running", "running");
    let task_id = task.id.unwrap();
    app.state
        .db
        .tasks()
        .insert_one(&task, None)
        .await
        .expect("insert running task");

    let err = review_task_now(
        State(app.state.clone()),
        Path(task_id.to_hex()),
        Extension(test_admin(&ws)),
    )
    .await
    .expect_err("running 任务应被 CAS 拒绝");
    assert!(
        matches!(err, AppError::Conflict(_)),
        "running 任务立即复核应返 Conflict，实际: {err:?}"
    );

    // 任务状态不被改动（仍 running，claimed_at 仍缺失）。
    let stored = app
        .state
        .db
        .tasks()
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("query")
        .expect("task exists");
    assert_eq!(stored.status, "running", "被拒的任务状态不应被改写");
    assert!(stored.claimed_at.is_none(), "被拒的任务不应写 claimed_at");
}

/// 终态任务（sent）不可复核——避免重跑已发送/已了结的任务。
#[tokio::test]
#[ignore]
async fn review_task_now_rejects_terminal_task() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();

    let task = task_with_status(&ws, &acc, "wx_sent", "sent");
    let task_id = task.id.unwrap();
    app.state
        .db
        .tasks()
        .insert_one(&task, None)
        .await
        .expect("insert sent task");

    let err = review_task_now(
        State(app.state.clone()),
        Path(task_id.to_hex()),
        Extension(test_admin(&ws)),
    )
    .await
    .expect_err("终态任务应被拒");
    assert!(
        matches!(err, AppError::Conflict(_)),
        "终态任务立即复核应返 Conflict，实际: {err:?}"
    );
}

/// S-02：可复核态（pending）被成功原子 claim，且写入 claimed_at——让 handler 失败
/// 停 running 后能被 reclaim_stale_running_tasks 分支A（claimed_at < stale）兜回。
/// memory_consolidation 无候选时走 sent 早退，handler 成功、任务落终态 sent。
#[tokio::test]
#[ignore]
async fn review_task_now_claims_pending_and_writes_claimed_at() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();

    let task = task_with_status(&ws, &acc, "wx_pending", "pending");
    let task_id = task.id.unwrap();
    app.state
        .db
        .tasks()
        .insert_one(&task, None)
        .await
        .expect("insert pending task");

    review_task_now(
        State(app.state.clone()),
        Path(task_id.to_hex()),
        Extension(test_admin(&ws)),
    )
    .await
    .expect("pending 任务应被成功 claim 并跑完 handler");

    let stored = app
        .state
        .db
        .tasks()
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("query")
        .expect("task exists");
    // handler（memory_consolidation 无候选）自写终态 sent；claimed_at 已在 claim 时写入。
    assert!(
        stored.claimed_at.is_some(),
        "成功 claim 必须写 claimed_at（否则 handler 失败停 running 落入 reclaim 盲区）"
    );
    assert_eq!(
        stored.status, "sent",
        "memory_consolidation 无候选应走 sent 早退终态"
    );
}

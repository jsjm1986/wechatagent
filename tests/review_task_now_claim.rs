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

use axum::extract::{Extension, Json, Path, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::{AgentStatus, AgentTask, Contact};
use wechatagent::routes::tasks::{cancel_agent_task, review_task_now, TaskActionRequest};

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "op_admin".to_string(),
        username: "op_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

fn task_action(account_id: &str) -> Json<TaskActionRequest> {
    Json(
        serde_json::from_value(serde_json::json!({ "expectedAccountId": account_id }))
            .expect("task action request"),
    )
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

/// 构造一条最小 Contact（memory_consolidation 无候选早退只需 contact 存在，
/// 不读 agent_status）。
fn make_contact(ws: &str, acc: &str, wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
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
        task_action(&acc),
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
        task_action(&acc),
    )
    .await
    .expect_err("终态任务应被拒");
    assert!(
        matches!(err, AppError::Conflict(_)),
        "终态任务立即复核应返 Conflict，实际: {err:?}"
    );
}

/// S-02：可复核态（pending）被成功原子 claim。处理中写入 claimed_at，使 handler
/// 失败停 running 后可被 reclaim；memory_consolidation 无候选成功落 sent 后必须清 lease。
#[tokio::test]
#[ignore]
async fn review_task_now_claims_pending_and_clears_lease_on_success() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();

    // handler（memory_consolidation）先按 wxid 查 contact，查不到直接返 NotFound；
    // 故须插一条真实 contact，且**不**插 memory_candidate → 走 consolidate_contact_memory
    // 无候选早退（memory.rs:1231 写 status=sent/gateway_status=no_candidates），不触达 LLM。
    app.state
        .db
        .contacts()
        .insert_one(&make_contact(&ws, &acc, "wx_pending"), None)
        .await
        .expect("insert contact");

    let task = task_with_status(&ws, &acc, "wx_pending", "pending");
    let task_id = task.id.unwrap();
    app.state
        .db
        .tasks()
        .insert_one(&task, None)
        .await
        .expect("insert pending task");

    let _resp = review_task_now(
        State(app.state.clone()),
        Path(task_id.to_hex()),
        Extension(test_admin(&ws)),
        task_action(&acc),
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
    // attempt_count 原子递增证明 claim 已发生；成功终态必须清理运行中 lease。
    assert_eq!(
        stored.attempt_count, 1,
        "成功 claim 必须原子递增 attempt_count"
    );
    assert!(
        stored.claimed_at.is_none(),
        "成功终态必须清 claimed_at，不能遗留看似仍被 worker 持有的 lease"
    );
    assert_eq!(
        stored.status, "sent",
        "memory_consolidation 无候选应走 sent 早退终态"
    );
}

/// SR-155：同 workspace 的错误账号不得复核或取消另一账号任务；任务和已绑定
/// Outbox 都必须保持逐字不变，证明拒绝发生在 claim/cancel 的原子账号 CAS 上。
#[tokio::test]
#[ignore]
async fn wrong_account_task_actions_are_conflict_with_zero_task_and_outbox_writes() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let task = task_with_status(&ws, "account-a", "wx_scope_guard", "pending");
    let task_id = task.id.expect("task id");
    app.state
        .db
        .tasks()
        .insert_one(&task, None)
        .await
        .expect("insert account-a task");

    let decision_id = ObjectId::new();
    app.state
        .db
        .raw()
        .collection::<Document>("agent_tasks")
        .update_one(
            doc! { "_id": task_id },
            doc! { "$set": { "outbox_decision_id": decision_id } },
            None,
        )
        .await
        .expect("bind task decision");

    let outbox_id = ObjectId::new();
    let now = DateTime::now();
    let outbox_collection = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox");
    outbox_collection
        .insert_one(
            doc! {
                "_id": outbox_id,
                "workspace_id": &ws,
                "account_id": "account-a",
                "contact_wxid": "wx_scope_guard",
                "run_id": "sr155-run",
                "decision_id": decision_id,
                "source_event_id": "sr155-event",
                "source_kind": "follow_up",
                "content": "must remain pending",
                "content_hash": "sr155-hash",
                "idempotency_key": format!("sr155-{}", outbox_id.to_hex()),
                "attempt": 0,
                "max_attempts": 3,
                "status": "pending",
                "cancel_requested": false,
                "claim_generation": 0i64,
                "reclaimed_in_flight": false,
                "reclaim_count": 0,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("insert linked outbox");

    let task_collection = app.state.db.raw().collection::<Document>("agent_tasks");
    let task_before = task_collection
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("read task before")
        .expect("task exists");
    let outbox_before = outbox_collection
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("read outbox before")
        .expect("outbox exists");

    let review = review_task_now(
        State(app.state.clone()),
        Path(task_id.to_hex()),
        Extension(test_admin(&ws)),
        task_action("account-b"),
    )
    .await;
    assert!(matches!(review, Err(AppError::Conflict(_))));

    let cancel = cancel_agent_task(
        State(app.state.clone()),
        Path(task_id.to_hex()),
        Extension(test_admin(&ws)),
        task_action("account-b"),
    )
    .await;
    assert!(matches!(cancel, Err(AppError::Conflict(_))));

    let task_after = task_collection
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("read task after")
        .expect("task remains");
    let outbox_after = outbox_collection
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("read outbox after")
        .expect("outbox remains");
    assert_eq!(
        task_after, task_before,
        "rejected actions must not mutate task"
    );
    assert_eq!(
        outbox_after, outbox_before,
        "rejected cancel must not mutate linked outbox"
    );
    app.cleanup().await;
}

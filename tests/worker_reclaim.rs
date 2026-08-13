//! HP-1 / Task 9 回归：Worker stale running 自动回收——**端到端驱动真实 reclaim**。
//!
//! 通过 `tokio::spawn(run_task_worker(...))` 让首次 tick 立即执行
//! `reclaim_stale_running_tasks`（tick 内部私有，不绕协议直调），断言：
//! 1. stale running（claimed_at 超过 task_claim_timeout_seconds）被回收为
//!    `retry` + `claim_recovery_count` 递增 + lease 字段（claimed_at/claim_token）
//!    被清除 + `gateway_status="claim_timeout_recovered"`；
//! 2. fresh running（claimed_at 在阈值内）不被误回收；
//! 3. 累计 `claim_recovery_count` 第 3 次回收直接落 `failed` +
//!    `gateway_status="claim_recovery_exhausted"`。
//!
//! 任务 kind 用 `DURABLE_INBOUND_REPLY_KIND`：task worker 的 due 扫描显式排除
//! 该 kind（由专用 inbound reply worker 处理，本测试不启动它），因此回收出的
//! `retry` 行不会在同一 tick 被 claim 执行，终态断言稳定。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）；CI 用
//! `cargo test -- --ignored` 触发。

mod common;

use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::models::AgentTask;
use wechatagent::webhooks::DURABLE_INBOUND_REPLY_KIND;

/// 构造一个 running 任务文档（含 claim_token / claim_generation lease 字段——
/// 与真实 claim 后的行同形；这两个字段不在 AgentTask 业务 DTO 上，需 raw 补写）。
fn running_task_doc(claimed_at_ms: i64, claim_recovery_count: i32) -> (ObjectId, Document) {
    let now = DateTime::now();
    let task_id = ObjectId::new();
    let task = AgentTask {
        id: Some(task_id),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: "user_stale".to_string(),
        kind: DURABLE_INBOUND_REPLY_KIND.to_string(),
        run_at: now,
        expires_at: None,
        content: "stale task".to_string(),
        status: "running".to_string(),
        source_decision_id: None,
        review_required: true,
        attempt_count: 1,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: Some(DateTime::from_millis(claimed_at_ms)),
        claim_recovery_count,
        created_at: now,
        updated_at: now,
    };
    let mut raw = mongodb::bson::to_document(&task).expect("serialize task");
    raw.insert("claim_token", ObjectId::new().to_hex());
    raw.insert("claim_generation", 1_i64);
    (task_id, raw)
}

async fn insert_raw_task(app: &common::TestApp, raw: Document) {
    app.state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .insert_one(raw, None)
        .await
        .expect("insert task");
}

/// 轮询直到指定任务进入期望 status（worker tick 异步推进），超时 panic。
async fn wait_for_task_status(
    app: &common::TestApp,
    task_id: ObjectId,
    expected: &str,
    timeout: Duration,
) -> Document {
    let coll = app.state.db.tasks().clone_with_type::<Document>();
    let start = std::time::Instant::now();
    let mut last = String::new();
    while start.elapsed() < timeout {
        let task = coll
            .find_one(doc! { "_id": task_id }, None)
            .await
            .expect("query task")
            .expect("task present");
        last = task.get_str("status").unwrap_or_default().to_string();
        if last == expected {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("task {task_id} did not reach status {expected:?} in {timeout:?}, last = {last:?}");
}

#[tokio::test]
#[ignore]
async fn stale_running_task_is_recovered_to_retry() {
    let app = common::TestApp::start().await;
    // task_claim_timeout_seconds = 5（来自 TestApp 配置）；claimed_at 设为 1 小时前必定超时。
    let one_hour_ago_ms = DateTime::now().timestamp_millis() - 60 * 60 * 1000;
    let (task_id, raw) = running_task_doc(one_hour_ago_ms, 0);
    insert_raw_task(&app, raw).await;

    // 驱动真实 worker：首次 tick 立即执行（reclaim 在 due 扫描之前）。
    let worker = tokio::spawn(wechatagent::tasks::run_task_worker(app.state.clone()));
    let recovered = wait_for_task_status(&app, task_id, "retry", Duration::from_secs(15)).await;
    worker.abort();
    let _ = worker.await;

    // 回收语义：recovery 计数递增、lease 字段清除、gateway_status 记录回收原因、
    // next_retry_at 已排上（立即可被下一轮 claim）。
    assert_eq!(recovered.get_i32("claim_recovery_count").unwrap_or(-1), 1);
    assert_eq!(
        recovered.get_str("gateway_status").unwrap_or_default(),
        "claim_timeout_recovered"
    );
    assert!(recovered.get("claimed_at").is_none(), "claimed_at unset");
    assert!(recovered.get("claim_token").is_none(), "claim_token unset");
    assert!(recovered.get_datetime("next_retry_at").is_ok());

    // 回收事件已落审计流。
    let event = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_events")
        .find_one(doc! { "kind": "task_claim_recovered" }, None)
        .await
        .expect("query events");
    assert!(event.is_some(), "task_claim_recovered event written");
}

#[tokio::test]
#[ignore]
async fn fresh_running_task_with_recent_claim_is_skipped() {
    let app = common::TestApp::start().await;
    // fresh：claimed_at=now，在 5s 阈值内。
    let (fresh_id, fresh_raw) = running_task_doc(DateTime::now().timestamp_millis(), 0);
    insert_raw_task(&app, fresh_raw).await;
    // sentinel stale 行：它变 retry 即证明 reclaim 扫描已完整跑过一轮，
    // 把"fresh 未被回收"的否命题锚定在肯定信号之后，避免时序 flake。
    let one_hour_ago_ms = DateTime::now().timestamp_millis() - 60 * 60 * 1000;
    let (sentinel_id, sentinel_raw) = running_task_doc(one_hour_ago_ms, 0);
    insert_raw_task(&app, sentinel_raw).await;

    let worker = tokio::spawn(wechatagent::tasks::run_task_worker(app.state.clone()));
    let _ = wait_for_task_status(&app, sentinel_id, "retry", Duration::from_secs(15)).await;
    worker.abort();
    let _ = worker.await;

    let fresh = app
        .state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": fresh_id }, None)
        .await
        .expect("query fresh task")
        .expect("fresh task present");
    assert_eq!(fresh.get_str("status").unwrap_or_default(), "running");
    assert_eq!(fresh.get_i32("claim_recovery_count").unwrap_or(-1), 0);
    assert!(fresh.get("claimed_at").is_some(), "lease intact");
}

#[tokio::test]
#[ignore]
async fn third_recovery_marks_task_failed() {
    let app = common::TestApp::start().await;
    // 已累计回收 2 次的 stale 行：本次回收 recovery_count=3 ≥ 3 → 直接 failed。
    let one_hour_ago_ms = DateTime::now().timestamp_millis() - 60 * 60 * 1000;
    let (task_id, raw) = running_task_doc(one_hour_ago_ms, 2);
    insert_raw_task(&app, raw).await;

    let worker = tokio::spawn(wechatagent::tasks::run_task_worker(app.state.clone()));
    let failed = wait_for_task_status(&app, task_id, "failed", Duration::from_secs(15)).await;
    worker.abort();
    let _ = worker.await;

    assert_eq!(failed.get_i32("claim_recovery_count").unwrap_or(-1), 3);
    assert_eq!(
        failed.get_str("gateway_status").unwrap_or_default(),
        "claim_recovery_exhausted"
    );
    assert!(failed
        .get_str("error")
        .unwrap_or_default()
        .contains("exceeded recovery attempts"));
    assert!(failed.get("claimed_at").is_none(), "claimed_at unset");
    assert!(failed.get("claim_token").is_none(), "claim_token unset");
}

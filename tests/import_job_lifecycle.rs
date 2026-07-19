//! 异步导入 job（`import_jobs` 集合）集成测试。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）；CI 用
//! `cargo test -- --ignored` 触发。验证四点，全部走公开 `state.db.import_jobs()`：
//! 1. `ImportJob` BSON 往返——字段是 snake_case（无 rename_all），与
//!    `db/indexes.rs` 的 `{workspace_id,status}` / `{status,claimed_at}` 索引键对齐。
//! 2. reclaim filter `{status:"running", claimed_at:{$lt: stale}}` 命中孤儿 job。
//! 3. IDOR——`get_import_preview_job` 端点用的
//!    `{_id, workspace_id}` filter 跨 workspace 查返 None（workspace 隔离）。
//! 4. 终态写 CAS——job 不在 running 时 `{_id, status:"running"}` 终态写 no-op，
//!    不覆盖被 reclaim 抢回的 pending 态（run_job 收尾竞态守卫回归）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::models::ImportJob;

fn pending_job(workspace_id: &str) -> ImportJob {
    let now = DateTime::now();
    ImportJob {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: Some("default".to_string()),
        source_name: "导入文本".to_string(),
        content: "# 标题\n正文若干".to_string(),
        segments_total: 3,
        progress_done: 0,
        progress_succeeded: 0,
        progress_failed: 0,
        status: "pending".to_string(),
        result: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        expires_at: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn import_job_bson_round_trips_with_snake_case_fields() {
    let app = common::TestApp::start().await;
    let job = pending_job("default");
    let job_id = job.id.expect("job id present");
    app.state
        .db
        .import_jobs()
        .insert_one(&job, None)
        .await
        .expect("insert pending job");

    // 反序列化回 struct（camelCase rename_all 会在此处炸——字段名不匹配）。
    let loaded = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": job_id }, None)
        .await
        .expect("query ok")
        .expect("job present");
    assert_eq!(loaded.status, "pending");
    assert_eq!(loaded.segments_total, 3);
    assert_eq!(loaded.workspace_id, "default");

    // 用 snake_case 字段名直接命中（证明索引键与 BSON 字段名一致）。
    let by_snake = app
        .state
        .db
        .import_jobs()
        .find_one(
            doc! { "workspace_id": "default", "status": "pending" },
            None,
        )
        .await
        .expect("query ok");
    assert!(by_snake.is_some(), "snake_case 字段名应命中");
}

#[tokio::test]
#[ignore]
async fn stale_running_import_job_matches_reclaim_filter() {
    let app = common::TestApp::start().await;
    let mut job = pending_job("default");
    let job_id = job.id.expect("job id present");
    // 构造孤儿：running + claimed_at 远早于 timeout（TestApp 配 5s）。
    job.status = "running".to_string();
    job.claimed_at = Some(DateTime::from_millis(
        DateTime::now().timestamp_millis() - 60 * 60 * 1000,
    ));
    app.state
        .db
        .import_jobs()
        .insert_one(&job, None)
        .await
        .expect("insert stale running job");

    // 复刻 reclaim_stale_running_jobs 的 filter，验证孤儿被命中。
    let timeout_secs = app.state.config.import_job_claim_timeout_seconds.max(1) as i64;
    let stale_before =
        DateTime::from_millis(DateTime::now().timestamp_millis() - timeout_secs * 1000);
    let matched = app
        .state
        .db
        .import_jobs()
        .find_one(
            doc! { "_id": job_id, "status": "running", "claimed_at": { "$lt": stale_before } },
            None,
        )
        .await
        .expect("query ok");
    assert!(
        matched.is_some(),
        "stale running job 应被 reclaim filter 命中"
    );
}

#[tokio::test]
#[ignore]
async fn import_job_query_is_workspace_scoped_idor() {
    let app = common::TestApp::start().await;
    let job = pending_job("ws_a");
    let job_id = job.id.expect("job id present");
    app.state
        .db
        .import_jobs()
        .insert_one(&job, None)
        .await
        .expect("insert job in ws_a");

    // get_import_preview_job 的 filter：本 workspace 命中。
    let same_ws = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": job_id, "workspace_id": "ws_a" }, None)
        .await
        .expect("query ok");
    assert!(same_ws.is_some(), "同 workspace 应能取到 job");

    // 跨 workspace（IDOR 尝试）→ None，不泄漏他人 job。
    let cross_ws = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": job_id, "workspace_id": "ws_b" }, None)
        .await
        .expect("query ok");
    assert!(
        cross_ws.is_none(),
        "跨 workspace 取 job 必须被拒（返 None）"
    );
}

/// 竞态回归：worker 收尾写终态用 `{_id, status:"running"}` CAS。若 job 已被
/// reclaim 抢回 pending（或被另一 worker 认领跑第二遍），原孤儿 worker 的终态写
/// 必须 no-op（modified=0），不覆盖新态。固化 `run_job` 收尾 CAS 守卫不变量。
#[tokio::test]
#[ignore]
async fn terminal_write_is_no_op_when_job_no_longer_running() {
    let app = common::TestApp::start().await;
    // job 已被 reclaim 回 pending（模拟孤儿被抢走后的状态）。
    let mut job = pending_job("default");
    let job_id = job.id.expect("job id present");
    job.status = "pending".to_string();
    app.state
        .db
        .import_jobs()
        .insert_one(&job, None)
        .await
        .expect("insert pending job");

    // 复刻 run_job 收尾的 CAS 终态写：filter 带 status:"running"。
    let res = app
        .state
        .db
        .import_jobs()
        .update_one(
            doc! { "_id": job_id, "status": "running" },
            doc! { "$set": { "status": "completed", "updated_at": DateTime::now() } },
            None,
        )
        .await
        .expect("update ok");
    assert_eq!(res.modified_count, 0, "job 不在 running 时终态写必须 no-op");

    // 验证 job 仍是 pending，未被孤儿 worker 覆盖成 completed。
    let after = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": job_id }, None)
        .await
        .expect("query ok")
        .expect("job present");
    assert_eq!(after.status, "pending", "pending 态不应被孤儿终态写覆盖");
}

/// TTL 清扫回归：pending job 不设 expires_at（TTL 忽略缺失字段，进行中 job 绝不
/// 被删）；worker 落终态时置 expires_at（24h 后被 TTL 删）。固化清扫不变量。
#[tokio::test]
#[ignore]
async fn terminal_job_sets_expires_at_pending_does_not() {
    let app = common::TestApp::start().await;
    let job = pending_job("default");
    let job_id = job.id.expect("job id present");
    app.state
        .db
        .import_jobs()
        .insert_one(&job, None)
        .await
        .expect("insert pending job");

    // pending：expires_at 缺失 → TTL 不命中，进行中 job 不被清扫。
    let pending = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": job_id }, None)
        .await
        .expect("query ok")
        .expect("job present");
    assert!(
        pending.expires_at.is_none(),
        "pending job 不应有 expires_at"
    );

    // 复刻 run_job 收尾终态写：置 status=completed + expires_at = now + 24h。
    let expires_at =
        DateTime::from_millis(DateTime::now().timestamp_millis() + 24 * 60 * 60 * 1000);
    app.state
        .db
        .import_jobs()
        .update_one(
            doc! { "_id": job_id },
            doc! { "$set": { "status": "completed", "expires_at": expires_at } },
            None,
        )
        .await
        .expect("terminal write ok");

    let done = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": job_id }, None)
        .await
        .expect("query ok")
        .expect("job present");
    assert_eq!(done.status, "completed");
    assert!(
        done.expires_at.is_some(),
        "终态 job 必须置 expires_at 供 TTL 清扫"
    );
}

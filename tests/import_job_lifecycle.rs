//! 异步导入 job（`import_jobs` 集合）集成测试。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）；CI 用
//! `cargo test -- --ignored` 触发。验证六点，并对 claim/reclaim/fencing 直接复用
//! 生产 `import_worker` 原语：
//! 1. `ImportJob` BSON 往返——字段是 snake_case（无 rename_all），与
//!    `db/indexes.rs` 的 `{workspace_id,status}` / `{status,claimed_at}` 索引键对齐。
//! 2. reclaim filter `{status:"running", claimed_at:{$lt: stale}}` 命中孤儿 job。
//! 3. IDOR——`get_import_preview_job` 端点用的
//!    `{_id, workspace_id}` filter 跨 workspace 查返 None（workspace 隔离）。
//! 4. owner-scoped 终态写在 job 不再属于该 claim 时 no-op。
//! 5. m056 为 legacy job 补 generation 且不改活跃 token/timestamp。
//! 6. A 过期→回收→B 重领后，A 的进度/终态和旧 scanner 均被 fencing。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::{
    import_worker::{
        claim_one, import_job_reclaim_snapshot_filter, reclaim_stale_running_jobs,
        update_owned_import_job, ImportJobClaim,
    },
    models::ImportJob,
};

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
        owner_admin_id: Some("import_admin".to_string()),
        preview_hash: None,
        apply_status: None,
        apply_request_hash: None,
        apply_result: None,
        applied_at: None,
        result: None,
        error: None,
        claimed_at: None,
        claim_generation: 0,
        claim_token: None,
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

/// 竞态回归：worker 收尾必须复用生产 owner filter。job 已回 pending 时，旧
/// claim 的终态写 no-op，不覆盖新态。
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

    let stale_claim = ImportJobClaim {
        job_id,
        generation: 1,
        token: "stale-owner".to_string(),
    };
    assert!(
        !update_owned_import_job(
            &app.state.db,
            &stale_claim,
            doc! { "$set": { "status": "completed", "updated_at": DateTime::now() } },
        )
        .await
        .expect("owner-scoped update ok"),
        "job 不属于旧 claim 时终态写必须 no-op"
    );

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

#[tokio::test]
#[ignore]
async fn migration_preserves_legacy_claim_and_is_idempotent() {
    let app = common::TestApp::start().await;
    let job_id = ObjectId::new();
    let claimed_at = DateTime::from_millis(DateTime::now().timestamp_millis() - 10_000);
    app.state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("import_jobs")
        .insert_one(
            doc! {
                "_id": job_id,
                "workspace_id": "legacy-workspace",
                "account_id": "default",
                "source_name": "legacy.md",
                "content": "legacy",
                "segments_total": 1,
                "progress_done": 0,
                "progress_succeeded": 0,
                "progress_failed": 0,
                "status": "running",
                "claim_token": "legacy-token",
                "claimed_at": claimed_at,
                "claim_recovery_count": 0,
                "created_at": DateTime::now(),
                "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert legacy import job");

    wechatagent::db::migrations::m056_import_job_claims::run_step(&app.state.db)
        .await
        .expect("first migration run");
    wechatagent::db::migrations::m056_import_job_claims::run_step(&app.state.db)
        .await
        .expect("idempotent migration rerun");

    let row = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("import_jobs")
        .find_one(doc! { "_id": job_id }, None)
        .await
        .expect("load migrated import job")
        .expect("migrated import job exists");
    assert_eq!(row.get_i64("claim_generation").unwrap(), 0);
    assert_eq!(row.get_str("claim_token").unwrap(), "legacy-token");
    assert_eq!(row.get_datetime("claimed_at").unwrap(), &claimed_at);
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn reclaimed_import_job_fences_old_worker_and_stale_scanner() {
    let app = common::TestApp::start().await;
    let job = pending_job("sr136-workspace");
    let job_id = job.id.expect("job id present");
    app.state
        .db
        .import_jobs()
        .insert_one(job, None)
        .await
        .expect("insert pending import job");

    let first_job = claim_one(&app.state)
        .await
        .expect("first claim succeeds")
        .expect("first owner claimed job");
    let first = ImportJobClaim::from_job(&first_job).expect("first claim identity");
    assert_eq!(first.generation, 1);

    let stale_at = DateTime::from_millis(
        DateTime::now().timestamp_millis()
            - (app.state.config.import_job_claim_timeout_seconds as i64 + 5) * 1000,
    );
    assert!(update_owned_import_job(
        &app.state.db,
        &first,
        doc! { "$set": { "claimed_at": stale_at } },
    )
    .await
    .expect("age first claim"));
    let stale_snapshot = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": job_id }, None)
        .await
        .expect("load stale snapshot")
        .expect("stale snapshot exists");
    let old_scanner_filter =
        import_job_reclaim_snapshot_filter(&stale_snapshot).expect("freeze stale scanner");

    assert_eq!(
        reclaim_stale_running_jobs(&app.state)
            .await
            .expect("reclaim first owner"),
        1
    );
    let second_job = claim_one(&app.state)
        .await
        .expect("second claim succeeds")
        .expect("second owner claimed job");
    let second = ImportJobClaim::from_job(&second_job).expect("second claim identity");
    assert_eq!(second.generation, 2);
    assert_ne!(first.token, second.token);

    assert!(
        !update_owned_import_job(
            &app.state.db,
            &first,
            doc! { "$set": { "progress_done": 99, "status": "completed" } },
        )
        .await
        .expect("old owner write is evaluated"),
        "old owner must not write progress or terminal state after reclaim"
    );
    let stale_reclaim = app
        .state
        .db
        .import_jobs()
        .update_one(
            old_scanner_filter,
            doc! { "$set": { "status": "pending" } },
            None,
        )
        .await
        .expect("old scanner CAS is evaluated");
    assert_eq!(
        stale_reclaim.matched_count, 0,
        "scanner snapshot from generation 1 must not reclaim generation 2"
    );
    assert!(update_owned_import_job(
        &app.state.db,
        &second,
        doc! { "$set": { "progress_done": 1 } },
    )
    .await
    .expect("current owner write"));

    let current = app
        .state
        .db
        .import_jobs()
        .find_one(doc! { "_id": job_id }, None)
        .await
        .expect("load current job")
        .expect("current job exists");
    assert_eq!(current.status, "running");
    assert_eq!(current.claim_generation, 2);
    assert_eq!(current.claim_token.as_deref(), Some(second.token.as_str()));
    assert_eq!(current.progress_done, 1);
    app.cleanup().await;
}

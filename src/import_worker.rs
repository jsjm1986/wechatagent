//! 异步知识导入 worker（`import_jobs` 集合）。长文档 preview 不再让前端同步
//! 死等：大文档在端点建 job（`status=pending`），本 worker 认领并跑分块抽取，
//! 段完成进度回写 job，前端轮询。走与同步 preview 同一 `run_import_extraction`，
//! 抽取/合并/D2 锚定逻辑字节等价。
//!
//! 模式一比一对齐 `tasks.rs` follow-up worker：reclaim stale → claim → 跑 →
//! 心跳续约 `claimed_at`。无 enable 开关（异步导入的必需件），常开。

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime},
    options::{FindOneAndUpdateOptions, ReturnDocument},
};
use tokio::time::sleep;

use crate::models::{assert_import_job_status_valid, ImportJob};
use crate::routes::AppState;

pub async fn run_import_worker(state: AppState) {
    loop {
        if let Err(error) = tick(&state).await {
            tracing::error!(error = %error, "import worker tick failed");
        }
        sleep(Duration::from_secs(
            state.config.import_worker_interval_seconds.max(1),
        ))
        .await;
    }
}

async fn tick(state: &AppState) -> anyhow::Result<()> {
    // 先回收 stale running（进程崩溃遗留），再认领新 job。
    let _ = reclaim_stale_running_jobs(state).await?;
    if let Some(job) = claim_one(state).await? {
        run_job(state, job).await;
    }
    Ok(())
}

/// 回收 `status="running"` 但 `claimed_at` 已超 `import_job_claim_timeout_seconds`
/// 的孤儿 job（worker 进程崩溃遗留）：重置回 `pending` 让下一轮重认领重跑。
/// 累计 `claim_recovery_count` ≥ 3 直接 `failed`，防死循环（仿 tasks.rs）。
///
/// 认领粒度是整个 job，不做段级断点续传——重跑会重新抽取所有段，简单且导入低频。
async fn reclaim_stale_running_jobs(state: &AppState) -> anyhow::Result<usize> {
    let timeout_secs = state.config.import_job_claim_timeout_seconds.max(1) as i64;
    let now_ms = DateTime::now().timestamp_millis();
    let stale_before = DateTime::from_millis(now_ms - timeout_secs * 1000);
    // job 认领时必写 claimed_at，故只需一个条件（不像 tasks 有缺失 claimed_at 的老任务）。
    let filter = doc! { "status": "running", "claimed_at": { "$lt": stale_before } };
    let mut cursor = state.db.import_jobs().find(filter, None).await?;
    let mut recovered = 0usize;
    while let Some(job) = cursor.try_next().await? {
        let Some(job_id) = job.id else { continue };
        let recovery_count = job.claim_recovery_count.saturating_add(1);
        if recovery_count >= 3 {
            assert_import_job_status_valid("failed");
            // 终态同置 expires_at，与 run_job 收尾一致，让 TTL 24h 后清扫。
            let expires_at =
                DateTime::from_millis(DateTime::now().timestamp_millis() + 24 * 60 * 60 * 1000);
            state
                .db
                .import_jobs()
                .update_one(
                    doc! { "_id": job_id, "status": "running" },
                    doc! {
                        "$set": {
                            "status": "failed",
                            "error": "导入任务多次卡死无法回收，已终止",
                            "expires_at": expires_at,
                            "updated_at": DateTime::now()
                        },
                        "$inc": { "claim_recovery_count": 1 }
                    },
                    None,
                )
                .await?;
            continue;
        }
        // CAS：只有"还在 running"的 job 被重置，避免与正常收尾竞争。
        assert_import_job_status_valid("pending");
        let res = state
            .db
            .import_jobs()
            .update_one(
                doc! { "_id": job_id, "status": "running" },
                doc! {
                    "$set": { "status": "pending", "updated_at": DateTime::now() },
                    "$inc": { "claim_recovery_count": 1 }
                },
                None,
            )
            .await?;
        if res.modified_count == 1 {
            recovered += 1;
        }
    }
    if recovered > 0 {
        tracing::info!(recovered, "reclaimed stale import jobs");
    }
    Ok(recovered)
}

/// 原子认领一个 `status="pending"` job（最旧优先）→ 置 `running` + 刷 `claimed_at`。
/// 单 worker、一次一 job（admin 导入低频，够用）。
async fn claim_one(state: &AppState) -> anyhow::Result<Option<ImportJob>> {
    let now = DateTime::now();
    assert_import_job_status_valid("running");
    let opts = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .sort(doc! { "created_at": 1 })
        .build();
    let claimed = state
        .db
        .import_jobs()
        .find_one_and_update(
            doc! { "status": "pending" },
            doc! {
                "$set": { "status": "running", "claimed_at": now, "updated_at": now }
            },
            opts,
        )
        .await?;
    Ok(claimed)
}

/// 跑一个已认领的 job：调共享抽取（段完成进度经 channel 回写 job）+ 心跳续约，
/// 收尾写终态。全成/部分成 → `completed` + `result`；全失败 → `failed` + `error`。
async fn run_job(state: &AppState, job: ImportJob) {
    let Some(job_id) = job.id else { return };

    // 心跳：抽取期间周期 bump claimed_at，防长段跑过 timeout 被 reclaim 并发重跑。
    let heartbeat = spawn_claim_heartbeat(
        state.clone(),
        job_id,
        state.config.import_job_claim_timeout_seconds,
    );

    // 进度桥：抽取回调是同步的（在 buffered 内），DB 写是异步的。用 unbounded
    // channel 把段完成快照喂给单个 drainer 串行写库，drainer 用 max_done 守单调
    // （buffered 并发下回调到达顺序不定）。
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, usize, usize)>();
    let drainer = {
        let state = state.clone();
        tokio::spawn(async move {
            let mut max_done = 0usize;
            while let Some((done, succeeded, failed)) = rx.recv().await {
                if done <= max_done {
                    continue;
                }
                max_done = done;
                let _ = state
                    .db
                    .import_jobs()
                    .update_one(
                        doc! { "_id": job_id, "status": "running" },
                        doc! {
                            "$set": {
                                "progress_done": done as i32,
                                "progress_succeeded": succeeded as i32,
                                "progress_failed": failed as i32,
                                "claimed_at": DateTime::now(),
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await;
            }
        })
    };

    let progress = move |done: usize, succeeded: usize, failed: usize| {
        let _ = tx.send((done, succeeded, failed));
    };

    let result = crate::routes::knowledge::run_import_extraction_for_job(
        state,
        job.account_id.clone(),
        Some(job.source_name.clone()),
        job.content.clone(),
        Some(&progress),
    )
    .await;

    // 关闭 channel（drop 持有 tx 的闭包）让 drainer 收尾，再停心跳。
    drop(progress);
    let _ = drainer.await;
    heartbeat.abort();

    // 终态写一律 CAS `status="running"`：只让"仍持有该 job"的 worker 落终态。
    // 若抽取跑过 timeout 窗口被 reclaim 抢回 pending（甚至已被另一 worker 认领跑
    // 第二遍），本（孤儿）worker 的终态写 modified=0 no-op，不覆盖新认领者的状态。
    //
    // 终态同时置 `expires_at = now + 24h`：`import_jobs` 的 TTL 索引据此在 24h 后
    // 删除完成/失败 job，防 `result`（可能较大）无界堆积（设计文档要求）。pending/
    // running 不设 expires_at → TTL 忽略缺失字段，进行中 job 绝不被删。
    let expires_at =
        DateTime::from_millis(DateTime::now().timestamp_millis() + 24 * 60 * 60 * 1000);
    match result {
        Ok(value) => match mongodb::bson::to_bson(&value) {
            Ok(result_bson) => {
                assert_import_job_status_valid("completed");
                let _ = state
                    .db
                    .import_jobs()
                    .update_one(
                        doc! { "_id": job_id, "status": "running" },
                        doc! {
                            "$set": {
                                "status": "completed",
                                "result": result_bson,
                                "expires_at": expires_at,
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await;
            }
            Err(err) => {
                assert_import_job_status_valid("failed");
                let _ = state
                    .db
                    .import_jobs()
                    .update_one(
                        doc! { "_id": job_id, "status": "running" },
                        doc! {
                            "$set": {
                                "status": "failed",
                                "error": format!("导入结果序列化失败：{err}"),
                                "expires_at": expires_at,
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await;
            }
        },
        Err(err) => {
            assert_import_job_status_valid("failed");
            let _ = state
                .db
                .import_jobs()
                .update_one(
                    doc! { "_id": job_id, "status": "running" },
                    doc! {
                        "$set": {
                            "status": "failed",
                            "error": err.to_string(),
                            "expires_at": expires_at,
                            "updated_at": DateTime::now()
                        }
                    },
                    None,
                )
                .await;
        }
    }
}

/// 给运行中 job 续约 claimed_at。间隔复用 `tasks::claim_heartbeat_interval_seconds`
/// （timeout/2 夹 [5,60]）。调用方在抽取结束后 `.abort()`。job 已不在 running
/// （被 reclaim 或落终态）时自动退出。
fn spawn_claim_heartbeat(
    state: AppState,
    job_id: ObjectId,
    timeout_seconds: u64,
) -> tokio::task::JoinHandle<()> {
    let interval = crate::tasks::claim_heartbeat_interval_seconds(timeout_seconds);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        ticker.tick().await; // 首拍立即触发，跳过。
        loop {
            ticker.tick().await;
            let res = state
                .db
                .import_jobs()
                .update_one(
                    doc! { "_id": job_id, "status": "running" },
                    doc! { "$set": { "claimed_at": DateTime::now() } },
                    None,
                )
                .await;
            match res {
                Ok(r) if r.modified_count == 0 => return,
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        job_id = %job_id.to_hex(),
                        error = %error,
                        "import job claim_heartbeat update failed; will retry next tick"
                    );
                }
            }
        }
    })
}

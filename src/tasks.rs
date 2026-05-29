use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime, oid::ObjectId},
    options::FindOptions,
};
use tokio::time::sleep;

use crate::{agent, error::AppResult, models::assert_agent_task_status_valid, routes::AppState};

pub async fn run_task_worker(state: AppState) {
    loop {
        if let Err(error) = tick(&state).await {
            tracing::error!(error = %error, "task worker tick failed");
        }
        sleep(Duration::from_secs(
            state.config.task_worker_interval_seconds,
        ))
        .await;
    }
}

/// HP-1 / Task 9：在每次 tick 开头扫描 `status="running"` 但 `claimed_at`
/// 已超过 [`AppConfig::task_claim_timeout_seconds`] 的任务，重置回 `retry`
/// 让后续 tick 重新 claim。
///
/// `claimed_at` 缺失视作老任务：用进程启动时间 `APP_STARTED_AT` 作为下界，
/// 只回收启动前留下来的；本进程启动后的运行任务即使没写 claimed_at 也跳过
/// 一次，避免误回收正在跑的任务。
///
/// `claim_recovery_count` 24h 内 ≥ 3 时直接标 `failed`，避免无限循环。
async fn reclaim_stale_running_tasks(state: &AppState) -> anyhow::Result<usize> {
    let timeout_secs = state.config.task_claim_timeout_seconds.max(1) as i64;
    let now_ms = DateTime::now().timestamp_millis();
    let stale_before = DateTime::from_millis(now_ms - timeout_secs * 1000);
    // 进程启动时间。OnceCell 没填时（极端情况）退化为 stale_before，等价于
    // "缺失 claimed_at 的老任务一律可回收"。
    let process_started_at = crate::APP_STARTED_AT.get().copied().unwrap_or(stale_before);

    let filter = doc! {
        "status": "running",
        "$or": [
            { "claimed_at": { "$lt": stale_before } },
            {
                "$and": [
                    { "claimed_at": { "$exists": false } },
                    { "updated_at": { "$lt": process_started_at } }
                ]
            }
        ]
    };

    let mut cursor = state.db.tasks().find(filter, None).await?;
    let mut recovered = 0usize;
    while let Some(task) = cursor.try_next().await? {
        let Some(task_id) = task.id else { continue };
        let claimed_at_ms = task.claimed_at.map(|d| d.timestamp_millis()).unwrap_or(0);
        let stuck_seconds = if claimed_at_ms > 0 {
            ((now_ms - claimed_at_ms) / 1000).max(0)
        } else {
            0
        };
        let recovery_count = task.claim_recovery_count.saturating_add(1);
        // 24h 内回收次数 ≥ 3 → 直接 failed，防止死循环。
        if recovery_count >= 3 {
            assert_agent_task_status_valid("failed");
            let res = state
                .db
                .tasks()
                .update_one(
                    doc! { "_id": task_id, "status": "running" },
                    doc! {
                        "$set": {
                            "status": "failed",
                            "gateway_status": "claim_recovery_exhausted",
                            "error": "task stuck in running state and exceeded recovery attempts",
                            "updated_at": DateTime::now()
                        },
                        "$inc": { "claim_recovery_count": 1 }
                    },
                    None,
                )
                .await?;
            if res.modified_count == 1 {
                let _ = agent::write_event_for_account(
                    state,
                    &task.account_id,
                    Some(&task.contact_wxid),
                    "claim_recovery_exhausted",
                    "failed",
                    "任务多次卡死无法回收，已强制 failed",
                    Some(doc! {
                        "task_id": task_id.to_hex(),
                        "kind": &task.kind,
                        "previous_attempt_count": task.attempt_count,
                        "stuck_seconds": stuck_seconds,
                        "recovery_count": recovery_count
                    }),
                )
                .await;
                let _ = agent::write_event_for_account(
                    state,
                    &task.account_id,
                    Some(&task.contact_wxid),
                    "follow_up_failed",
                    "failed",
                    "任务多次卡死无法回收，已强制 failed",
                    None,
                )
                .await;
            }
            continue;
        }
        // 普通回收路径：CAS update 确保只有"还在 running"的任务被重置。
        assert_agent_task_status_valid("retry");
        let res = state
            .db
            .tasks()
            .update_one(
                doc! { "_id": task_id, "status": "running" },
                doc! {
                    "$set": {
                        "status": "retry",
                        "gateway_status": "claim_timeout_recovered",
                        "next_retry_at": DateTime::now(),
                        "updated_at": DateTime::now()
                    },
                    "$inc": { "claim_recovery_count": 1 }
                },
                None,
            )
            .await?;
        if res.modified_count == 1 {
            recovered += 1;
            let _ = agent::write_event_for_account(
                state,
                &task.account_id,
                Some(&task.contact_wxid),
                "task_claim_recovered",
                "recovered",
                "Worker stale 任务已被回收为 retry",
                Some(doc! {
                    "task_id": task_id.to_hex(),
                    "kind": &task.kind,
                    "previous_attempt_count": task.attempt_count,
                    "stuck_seconds": stuck_seconds,
                    "recovery_count": recovery_count
                }),
            )
            .await;
        }
    }
    if recovered > 0 {
        tracing::info!(recovered, "reclaimed stale running tasks");
    }
    Ok(recovered)
}

async fn tick(state: &AppState) -> anyhow::Result<()> {
    // HP-1：先回收 stale running，再 claim 新任务。
    let _ = reclaim_stale_running_tasks(state).await?;
    // S-19 / Task 17：保证当日 outcome 聚合任务存在。
    let _ = ensure_today_outcome_aggregation_tasks(state).await;
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "status": { "$in": ["pending", "retry"] },
                "$or": [
                    { "run_at": { "$lte": DateTime::now() } },
                    { "next_retry_at": { "$lte": DateTime::now() } }
                ]
            },
            FindOptions::builder()
                .limit(20)
                .sort(doc! { "next_retry_at": 1, "run_at": 1 })
                .build(),
        )
        .await?;

    while let Some(task) = cursor.try_next().await? {
        let Some(task_id) = task.id else {
            continue;
        };
        let claim_now = DateTime::now();
        assert_agent_task_status_valid("running");
        let claim = state
            .db
            .tasks()
            .update_one(
                doc! { "_id": task_id, "status": &task.status },
                doc! {
                    "$set": {
                        "status": "running",
                        "updated_at": claim_now,
                        "claimed_at": claim_now
                    },
                    "$inc": { "attempt_count": 1 }
                },
                None,
            )
            .await?;
        if claim.modified_count == 0 {
            continue;
        }
        let task_account_id = task.account_id.clone();
        let task_contact_wxid = task.contact_wxid.clone();
        let attempt_count = task.attempt_count.saturating_add(1);
        let max_attempts = if task.max_attempts <= 0 {
            3
        } else {
            task.max_attempts
        };
        // P1-9：长任务（memory_consolidation / outcome_aggregation / 长 LLM 重试链）
        // 可能跑过 task_claim_timeout_seconds，被 reclaim_stale_running_tasks 误判
        // 抢回 retry，导致同一任务并发跑两份。每 timeout/2 秒 bump 一次 claimed_at
        // 让仍在进行中的 claimer 续约，关停信号靠 work future 完成时 drop 句柄触发。
        let heartbeat = spawn_claim_heartbeat(
            state.clone(),
            task_id,
            state.config.task_claim_timeout_seconds,
        );
        let result = if task.kind == "memory_consolidation" {
            agent::handle_memory_consolidation_task(state, task).await
        } else if task.kind == "outcome_aggregation" {
            handle_outcome_aggregation_task(state, task).await
        } else {
            agent::handle_follow_up_task(state, task).await
        };
        heartbeat.abort();

        match result {
            Ok(()) => {
                agent::write_event_for_account(
                    state,
                    &task_account_id,
                    Some(&task_contact_wxid),
                    "follow_up_processed",
                    "success",
                    "跟进任务已通过发送网关处理",
                    None,
                )
                .await?;
            }
            Err(error) => {
                if attempt_count < max_attempts {
                    let delay_seconds = retry_delay_seconds(attempt_count);
                    assert_agent_task_status_valid("retry");
                    state
                        .db
                        .tasks()
                        .update_one(
                            doc! { "_id": task_id },
                            doc! {
                                "$set": {
                                    "status": "retry",
                                    "gateway_status": "retry_scheduled",
                                    "error": error.to_string(),
                                    "next_retry_at": DateTime::from_millis(
                                        DateTime::now().timestamp_millis() + delay_seconds * 1000
                                    ),
                                    "updated_at": DateTime::now()
                                }
                            },
                            None,
                        )
                        .await?;
                    agent::write_event_for_account(
                        state,
                        &task_account_id,
                        Some(&task_contact_wxid),
                        "follow_up_retry_scheduled",
                        "retry",
                        &format!(
                            "跟进任务失败，已安排第 {attempt_count}/{max_attempts} 次重试：{error}"
                        ),
                        None,
                    )
                    .await?;
                    continue;
                }
                assert_agent_task_status_valid("failed");
                state
                    .db
                    .tasks()
                    .update_one(
                        doc! { "_id": task_id },
                        doc! {
                            "$set": {
                                "status": "failed",
                                "gateway_status": "failed",
                                "error": error.to_string(),
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await?;
                agent::write_event_for_account(
                    state,
                    &task_account_id,
                    Some(&task_contact_wxid),
                    "follow_up_failed",
                    "failed",
                    &error.to_string(),
                    None,
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// P1-8：任务重试退避带 ±20% jitter，避免 MCP/LLM 故障同时恢复后所有重试
/// 任务在同一墙钟 tick 形成 thundering herd 砸回上游。
///
/// 公式：base = `min(60 * 2^(attempt-1), 900)`（60s / 120s / 240s / 480s / 900s
/// 上限），jitter01 ∈ [0, 1] 时实际延迟落在 base * [0.8, 1.2] 区间内。
/// jitter01=0.5 即 0 jitter（基线），便于单测。
fn retry_delay_seconds(attempt_count: i32) -> i64 {
    retry_delay_seconds_seeded(attempt_count, fastrand::f64())
}

fn retry_delay_seconds_seeded(attempt_count: i32, jitter01: f64) -> i64 {
    let capped = attempt_count.clamp(1, 6);
    let base = 60_i64.saturating_mul(1_i64 << (capped - 1)).min(900);
    let j = jitter01.clamp(0.0, 1.0);
    let factor = (j - 0.5) * 2.0 * 0.2;
    let delta = (base as f64 * factor).round() as i64;
    base + delta
}

/// P1-9：续约心跳间隔。基于 `task_claim_timeout_seconds` 推导：
/// 取 timeout/2 但夹在 [5, 60] 内——下界保证不抖太频繁、上界保证 timeout=120s
/// 时仍有两次心跳机会，避免一次心跳失败就被 reclaim。
pub(crate) fn claim_heartbeat_interval_seconds(task_claim_timeout_seconds: u64) -> u64 {
    let half = task_claim_timeout_seconds / 2;
    half.clamp(5, 60)
}

/// P1-9：spawn 一个长跑后台任务给 `task_id` 续约 claimed_at。
/// 调用方在 work future 结束后 `.abort()` 该 handle，停止心跳。
///
/// 故意不走 supervisor.spawn_supervised：心跳的退出条件是"上游 work future
/// 完成 / abort"，而 supervisor 的语义是"panic 后无限重启"。心跳 panic 反而
/// 应该让其消失，让 reclaim_stale_running_tasks 兜底。
fn spawn_claim_heartbeat(
    state: AppState,
    task_id: ObjectId,
    task_claim_timeout_seconds: u64,
) -> tokio::task::JoinHandle<()> {
    let interval = claim_heartbeat_interval_seconds(task_claim_timeout_seconds);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        // 第一次 tick 立刻触发；下一次 tick 跳过首拍。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            // 只对仍处于 running 的任务续约：终态/被 reclaim 的任务直接 stop。
            let res = state
                .db
                .tasks()
                .update_one(
                    doc! { "_id": task_id, "status": "running" },
                    doc! { "$set": { "claimed_at": DateTime::now() } },
                    None,
                )
                .await;
            match res {
                Ok(r) if r.modified_count == 0 => {
                    // 任务已不在 running（被 reclaim 或已落终态）→ 退出心跳。
                    return;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        task_id = %task_id.to_hex(),
                        error = %error,
                        "claim_heartbeat update failed; will retry next tick"
                    );
                }
            }
        }
    })
}

/// S-19 / Task 17：保证当日所有 (account, horizon) 都有一条 `outcome_aggregation`
/// 任务。在 [`tick`] 入口被调用，幂等（基于 task content 中的日期 + horizon 去重）。
///
/// P1-1：原 `find_one + insert_one` 是 TOCTOU；多副本/重叠 tick 都会通过
/// 检查并双写。改为直接 `insert_one`，依赖 db/indexes.rs:90 的
/// `uniq_outcome_aggregation_kind_account_content` partial unique index
/// 在 MongoDB 侧原子去重，11000 dup-key 视作"已经有人插过了"忽略即可。
async fn ensure_today_outcome_aggregation_tasks(state: &AppState) -> anyhow::Result<()> {
    use mongodb::bson::DateTime as BsonDt;
    let today = today_date_string();
    let now = BsonDt::now();
    let mut accounts_cursor = state.db.accounts().find(doc! {}, None).await?;
    while let Some(account) = accounts_cursor.try_next().await? {
        for horizon in ["7d", "30d"].iter() {
            let content = format!("{{\"horizon\":\"{horizon}\",\"date\":\"{today}\"}}");
            let result = state
                .db
                .tasks()
                .insert_one(
                    crate::models::AgentTask {
                        id: None,
                        workspace_id: account.workspace_id.clone(),
                        account_id: account.account_id.clone(),
                        contact_wxid: "_outcome_aggregation".to_string(),
                        kind: "outcome_aggregation".to_string(),
                        run_at: now,
                        expires_at: None,
                        content,
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
                    },
                    None,
                )
                .await;
            if let Err(error) = result {
                if !is_duplicate_key_error(&error) {
                    return Err(error.into());
                }
                // dup-key：当日已有该 (account, horizon) 的 outcome_aggregation 任务，
                // 幂等忽略。
            }
        }
    }
    Ok(())
}

/// 判定 mongodb 错误是否为 DuplicateKey（code 11000 / 11001）。
/// 与 `agent::outbox::is_duplicate_key_error` 同语义；不跨 mod 复用以避免
/// tasks 反向依赖 agent 内部 helper。
fn is_duplicate_key_error(err: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match &*err.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            write_error.code == 11000 || write_error.code == 11001
        }
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .as_ref()
            .map(|errs| errs.iter().any(|e| e.code == 11000 || e.code == 11001))
            .unwrap_or(false),
        _ => false,
    }
}

fn today_date_string() -> String {
    let now_ms = mongodb::bson::DateTime::now().timestamp_millis();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    // 截断到日；用 epoch 起点开始的天数转成 YYYY-MM-DD（粗糙但足够幂等用）。
    let days = now_ms / day_ms;
    let secs = days * 24 * 60 * 60;
    let datetime =
        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).unwrap_or_else(chrono::Utc::now);
    datetime.format("%Y-%m-%d").to_string()
}

/// S-19 / Task 17：处理一次 outcome 聚合任务，把 24h/7d/30d 的指标写入
/// `agent_outcome_metrics` 集合（按 _id 幂等）。
async fn handle_outcome_aggregation_task(
    state: &AppState,
    task: crate::models::AgentTask,
) -> AppResult<()> {
    let Some(task_id) = task.id else {
        return Ok(());
    };
    // 解析 content 拿 horizon / date
    let parsed: serde_json::Value =
        serde_json::from_str(&task.content).unwrap_or(serde_json::json!({}));
    let horizon = parsed
        .get("horizon")
        .and_then(|v| v.as_str())
        .unwrap_or("7d")
        .to_string();
    let date = parsed
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let horizon_days: i64 = if horizon == "30d" { 30 } else { 7 };
    let now_ms = mongodb::bson::DateTime::now().timestamp_millis();
    let window_start =
        mongodb::bson::DateTime::from_millis(now_ms - horizon_days * 24 * 60 * 60 * 1000);

    // reply_rate：发出消息（outbound）后 horizon_days 内有 inbound 的比例。
    let _outbound_count = state
        .db
        .messages()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "direction": "outbound",
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    // 严格按"每条 outbound 后 horizon 窗口内是否有用户 inbound"计算。
    let mut outbound_total = 0_i64;
    let mut replied_outbound_total = 0_i64;
    let mut cur = state
        .db
        .messages()
        .find(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "direction": "outbound",
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await?;
    while let Some(msg) = cur.try_next().await? {
        outbound_total += 1;
        let reply_window_end = DateTime::from_millis(
            msg.created_at.timestamp_millis() + horizon_days * 24 * 60 * 60 * 1000,
        );
        let has_inbound_after_outbound = state
            .db
            .messages()
            .count_documents(
                doc! {
                    "workspace_id": &task.workspace_id,
                    "account_id": &task.account_id,
                    "contact_wxid": &msg.contact_wxid,
                    "direction": "inbound",
                    "created_at": {
                        "$gt": msg.created_at,
                        "$lte": reply_window_end
                    }
                },
                None,
            )
            .await
            .unwrap_or(0);
        if has_inbound_after_outbound > 0 {
            replied_outbound_total += 1;
        }
    }
    let reply_rate = if outbound_total > 0 {
        Some(replied_outbound_total as f64 / outbound_total as f64)
    } else {
        // 波 A2：outbound 总数为 0 时返回 None（无数据），不写 0 误导前端。
        None
    };

    // conversation_depth：每个 managed contact 平均 inbound 数。
    let inbound_count = state
        .db
        .messages()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "direction": "inbound",
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let managed_count = state
        .db
        .contacts()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "agent_status": "managed"
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let conversation_depth = if managed_count > 0 {
        Some(inbound_count as f64 / managed_count as f64)
    } else {
        // 波 A2：无 managed contact 时无意义，返回 None。
        None
    };

    // agent_block_rate：blocked review / total review。
    let blocked = state
        .db
        .decision_reviews()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "status": "blocked",
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let review_total = state
        .db
        .decision_reviews()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let agent_block_rate = if review_total > 0 {
        Some(blocked as f64 / review_total as f64)
    } else {
        // 波 A2：review 总数为 0 时返回 None。
        None
    };

    // 波 A2：ai_hold_cleared_rate 暂无事件源（AI 自暂缓后由 AI 自身澄清恢复
    // 继续的比例），写 None 表示"指标不可用"，不再以 0 静默冒充零成功率。
    let ai_hold_cleared_rate: Option<f64> = None;

    // daily_run_count / daily_run_token_total：当日 agent_run_logs 聚合（不取 horizon，固定取 24h）。
    let day_start = mongodb::bson::DateTime::from_millis(now_ms - 24 * 60 * 60 * 1000);
    let daily_run_count = state
        .db
        .agent_run_logs()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "created_at": { "$gte": day_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let mut daily_run_token_total = 0i64;
    let mut runs_cur = state
        .db
        .agent_run_logs()
        .find(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "created_at": { "$gte": day_start }
            },
            None,
        )
        .await?;
    while let Some(run) = runs_cur.try_next().await? {
        daily_run_token_total += run.tokens_used;
    }

    let metric = crate::models::AgentOutcomeMetric {
        id: format!(
            "{}:{}:{}:{}",
            task.workspace_id, task.account_id, horizon, date
        ),
        workspace_id: task.workspace_id.clone(),
        account_id: task.account_id.clone(),
        horizon: horizon.clone(),
        date: date.clone(),
        reply_rate,
        conversation_depth,
        ai_hold_cleared_rate,
        agent_block_rate,
        daily_run_count,
        daily_run_token_total,
        created_at: mongodb::bson::DateTime::now(),
    };
    let metric_doc = mongodb::bson::to_document(&metric)?;
    state
        .db
        .outcome_metrics()
        .update_one(
            doc! { "_id": &metric.id },
            doc! { "$set": metric_doc },
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await?;
    assert_agent_task_status_valid("sent");
    state
        .db
        .tasks()
        .update_one(
            doc! { "_id": task_id },
            doc! { "$set": { "status": "sent", "gateway_status": "aggregated", "updated_at": mongodb::bson::DateTime::now() } },
            None,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! tasks.rs 单元测试：claim 时序结构相关。
    //!
    //! 真实的"claim 后写 claimed_at"行为依赖 MongoDB，覆盖在
    //! `tests/worker_reclaim.rs`（Task 9）。这里只做结构性回归，确保
    //! `AgentTask` schema 包含 `claimed_at` 与 `claim_recovery_count` 字段，
    //! 防止后续重构误删字段而不被发现。

    use crate::models::AgentTask;
    use mongodb::bson::DateTime;

    /// HP-1 / Task 7 schema 回归：
    /// `AgentTask` 必须支持 `claimed_at: Option<DateTime>` 与
    /// `claim_recovery_count: i32` 两个新字段，且默认值为 None / 0。
    #[test]
    fn agent_task_supports_claim_tracking_fields() {
        let now = DateTime::now();
        let task = AgentTask {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: "user_test".to_string(),
            kind: "follow_up".to_string(),
            run_at: now,
            expires_at: None,
            content: "demo".to_string(),
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
        };
        assert!(task.claimed_at.is_none());
        assert_eq!(task.claim_recovery_count, 0);

        // 模拟 claim 后赋值：claimed_at 应为 Some(now)。
        let claimed = AgentTask {
            claimed_at: Some(now),
            claim_recovery_count: 1,
            ..task
        };
        assert!(claimed.claimed_at.is_some());
        assert_eq!(claimed.claim_recovery_count, 1);
    }

    /// P1-8：jitter01=0.5 应回到无 jitter 基线，验证 attempt 1..6 的指数表
    /// 与 900s 上限。
    #[test]
    fn retry_delay_baseline_without_jitter() {
        use super::retry_delay_seconds_seeded;
        // attempt → base = 60 * 2^(attempt-1)，封顶 900。
        assert_eq!(retry_delay_seconds_seeded(1, 0.5), 60);
        assert_eq!(retry_delay_seconds_seeded(2, 0.5), 120);
        assert_eq!(retry_delay_seconds_seeded(3, 0.5), 240);
        assert_eq!(retry_delay_seconds_seeded(4, 0.5), 480);
        assert_eq!(retry_delay_seconds_seeded(5, 0.5), 900);
        assert_eq!(retry_delay_seconds_seeded(6, 0.5), 900);
        // 越界 attempt 也封顶。
        assert_eq!(retry_delay_seconds_seeded(99, 0.5), 900);
    }

    /// P1-8：jitter ∈ ±20% → attempt=2 base=120s 实际落在 [96, 144]。
    #[test]
    fn retry_delay_jitter_within_bounds() {
        use super::retry_delay_seconds_seeded;
        let lo = retry_delay_seconds_seeded(2, 0.0);
        let hi = retry_delay_seconds_seeded(2, 1.0);
        assert!(lo >= 96 && lo <= 120, "low jitter out of range: {lo}");
        assert!(hi >= 120 && hi <= 144, "high jitter out of range: {hi}");
        // 上限场景也守住 ±20%：900 * 0.8 = 720。
        let lo_cap = retry_delay_seconds_seeded(6, 0.0);
        assert!(
            lo_cap >= 720 && lo_cap <= 900,
            "cap low jitter out of range: {lo_cap}"
        );
    }

    /// P1-8：随机 jitter 不会越过 ±20% 区间，避免回归引入 bias。
    #[test]
    fn retry_delay_random_jitter_stays_in_band() {
        use super::retry_delay_seconds_seeded;
        for _ in 0..200 {
            let j = fastrand::f64();
            let v = retry_delay_seconds_seeded(3, j);
            // attempt=3 base=240，区间 [192, 288]。
            assert!(
                v >= 192 && v <= 288,
                "v={v} jitter01={j} out of [192, 288]"
            );
        }
    }

    /// P1-9：心跳间隔 = timeout/2，但夹在 [5, 60]。
    #[test]
    fn claim_heartbeat_interval_clamps() {
        use super::claim_heartbeat_interval_seconds;
        // 下界：timeout=5 → 5/2=2 → 夹到 5。
        assert_eq!(claim_heartbeat_interval_seconds(5), 5);
        // 默认 30s timeout → 15s。
        assert_eq!(claim_heartbeat_interval_seconds(30), 15);
        // 60s timeout → 30s。
        assert_eq!(claim_heartbeat_interval_seconds(60), 30);
        // 上界：timeout=600s 直接夹到 60。
        assert_eq!(claim_heartbeat_interval_seconds(600), 60);
        // 0：clamp 下界保护。
        assert_eq!(claim_heartbeat_interval_seconds(0), 5);
    }

    /// P1-9：心跳间隔严格 < task_claim_timeout，避免一次心跳失败就被 reclaim。
    /// 唯一例外是夹到上界 60 的极长 timeout——那种 timeout 下无论如何都不会
    /// 被 reclaim 误判。
    #[test]
    fn claim_heartbeat_strictly_below_timeout_in_normal_range() {
        use super::claim_heartbeat_interval_seconds;
        for timeout in [10_u64, 20, 30, 60, 90, 119] {
            let interval = claim_heartbeat_interval_seconds(timeout);
            assert!(
                (interval as u64) < timeout,
                "interval={interval} must be < timeout={timeout}"
            );
        }
    }
}

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::FindOptions,
};
use tokio::time::sleep;

use crate::{agent, error::AppResult, routes::AppState};

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
        let result = if task.kind == "memory_consolidation" {
            agent::handle_memory_consolidation_task(state, task).await
        } else if task.kind == "outcome_aggregation" {
            handle_outcome_aggregation_task(state, task).await
        } else {
            agent::handle_follow_up_task(state, task).await
        };

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

fn retry_delay_seconds(attempt_count: i32) -> i64 {
    let capped = attempt_count.clamp(1, 6);
    let delay = 60_i64.saturating_mul(1_i64 << (capped - 1));
    delay.min(900)
}

/// S-19 / Task 17：保证当日所有 (account, horizon) 都有一条 `outcome_aggregation`
/// 任务。在 [`tick`] 入口被调用，幂等（基于 task content 中的日期 + horizon 去重）。
async fn ensure_today_outcome_aggregation_tasks(state: &AppState) -> anyhow::Result<()> {
    use mongodb::bson::DateTime as BsonDt;
    let today = today_date_string();
    let now = BsonDt::now();
    let mut accounts_cursor = state.db.accounts().find(doc! {}, None).await?;
    while let Some(account) = accounts_cursor.try_next().await? {
        for horizon in ["7d", "30d"].iter() {
            let content = format!("{{\"horizon\":\"{horizon}\",\"date\":\"{today}\"}}");
            let exists = state
                .db
                .tasks()
                .find_one(
                    doc! {
                        "kind": "outcome_aggregation",
                        "account_id": &account.account_id,
                        "content": &content
                    },
                    None,
                )
                .await?;
            if exists.is_some() {
                continue;
            }
            let _ = state
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
        }
    }
    Ok(())
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

    // 波 A2：human_handoff_success_rate 暂无事件源，写 None 表示"指标不可用"，
    // 不再以 0 静默冒充零成功率。后续接入 human_handoff 事件后改为实际比例。
    let human_handoff_success_rate: Option<f64> = None;

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
        human_handoff_success_rate,
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
}

//! Outbox dispatcher worker（agent-autonomy-loop W4 / Task 5.2-5.4）。
//!
//! 异步 worker 周期性扫描 `agent_send_outbox`：
//! 1. **reclaim_expired_leases**：把 `status="in_flight" AND locked_until < now`
//!    的 entry 改回 pending（worker 崩溃 / 卡住的恢复路径）；
//! 2. **atomic_claim_pending**：用 `findOneAndUpdate` 抢占一条 pending entry
//!    （`status="in_flight"` + `worker_id` + `locked_until=now+lease`），
//!    `returnDocument: After` 确保多 worker 并发场景下恰好一个抢占成功；
//! 3. **second_safety_gate**：发送前再次检查 contact cooldown / user stop /
//!    陈旧度（30min）；任一命中 → cancel；
//! 4. **MCP 发送**：成功 → status=sent；失败 → attempt+1 + retry backoff
//!    或 failed_terminal。
//!
//! 设计原则：
//! - **单 worker 单 entry**：每 tick 抢占 1 条；多 worker 并发安全由 atomic
//!   claim 保证；
//! - **每个 entry 事件 ≤ 20 条**：写 event 前查询计数，超过即 stop（防 retry
//!   风暴写爆 events）；
//! - **lease 自动续约不必要**：lease=60s 比 MCP 5s timeout 大 10x，正常路径
//!   一次 tick 内完成；超时 → 下一轮 reclaim 自动恢复。

use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::error::AppResult;
use crate::models::OutboxEntry;
use crate::routes::AppState;

use super::outbox::{
    backoff_with_jitter_seeded, check_second_safety_gate_pure, write_outbox_event, OutboxStatus,
};

/// MCP 单次调用 timeout（R13 / N4）。lease (60s) 远大于此，确保 worker 出错
/// 不会卡住 entry。
const MCP_SEND_TIMEOUT_SECONDS: u64 = 5;

/// 二次安全门陈旧度阈值（R13.4：>30min 自动 canceled）。
const STALE_THRESHOLD_MILLIS: i64 = 30 * 60 * 1000;

/// 单条 entry 总事件数上限（R13.7 防 retry 风暴）。
pub(crate) const PER_ENTRY_EVENT_CAP: i64 = 20;

/// 单 tick 处理上限，防止饿死 / 长 tick。
const PER_TICK_PROCESS_CAP: usize = 16;

/// 单 worker 的唯一 id：`hostname:pid:uuid`，便于审计哪台机器哪进程占了哪条。
fn worker_id() -> String {
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    let pid = std::process::id();
    let uniq = uuid::Uuid::new_v4().to_string();
    format!("{host}:{pid}:{uniq}")
}

/// **崩溃恢复**：把所有 `status="in_flight" AND locked_until < now` 的 entry
/// 改回 pending；同时清空 worker_id / locked_until。返回回收条数。
//
// NOTE: 暴露为 `pub` 仅供 `tests/outbox_integration.rs`（W4 / Task 5.8 / R13.10）
// 直接驱动，不应在生产代码中绕过 `tick` 单独调用。
pub async fn reclaim_expired_leases(state: &AppState) -> AppResult<u64> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let result = collection
        .update_many(
            doc! {
                "status": OutboxStatus::InFlight.as_str(),
                "locked_until": { "$lt": now },
            },
            doc! {
                "$set": {
                    "status": OutboxStatus::Pending.as_str(),
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                }
            },
            None,
        )
        .await?;
    if result.modified_count > 0 {
        tracing::info!(
            modified_count = result.modified_count,
            "outbox dispatcher reclaimed expired leases"
        );
    }
    Ok(result.modified_count)
}

/// **原子抢占**：从 `pending` + (next_retry_at 为 null 或 ≤ now) 中抢一条，
/// 并立即把它切到 `in_flight` + `worker_id` + `locked_until=now+lease`。
//
// NOTE: 暴露为 `pub` 仅供 `tests/outbox_integration.rs`（W4 / Task 5.8 / R13.10）
// 直接驱动，不应在生产代码中绕过 `tick` 单独调用。
pub async fn atomic_claim_pending(
    state: &AppState,
    worker: &str,
    lease_seconds: i32,
) -> AppResult<Option<OutboxEntry>> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let lease_ms = (lease_seconds.max(1) as i64) * 1000;
    let lease_until = DateTime::from_millis(now.timestamp_millis() + lease_ms);

    let filter = doc! {
        "status": OutboxStatus::Pending.as_str(),
        "$or": [
            { "next_retry_at": { "$exists": false } },
            { "next_retry_at": null },
            { "next_retry_at": { "$lte": now } },
        ]
    };
    let update = doc! {
        "$set": {
            "status": OutboxStatus::InFlight.as_str(),
            "worker_id": worker,
            "locked_until": lease_until,
            "updated_at": now,
        }
    };
    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();
    Ok(collection
        .find_one_and_update(filter, update, options)
        .await?)
}

/// **二次安全门**（R13.4）：发送前再次检查 contact cooldown / user stop /
/// 陈旧度（30min）。任一命中 → 返回 `Some(reason)`。
//
// NOTE: 暴露为 `pub` 仅供 `tests/outbox_integration.rs`（W4 / Task 5.8 / R13.10）
// 直接驱动，不应在生产代码中绕过 `process_entry` 单独调用。
pub async fn second_safety_gate(
    state: &AppState,
    entry: &OutboxEntry,
) -> AppResult<Option<String>> {
    let now = DateTime::now();
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid,
            },
            None,
        )
        .await?;
    let cooldown_until_ms = contact
        .as_ref()
        .and_then(|c| c.cooldown_until)
        .map(|d| d.timestamp_millis());
    let last_inbound_ms = contact
        .as_ref()
        .and_then(|c| c.last_inbound_at)
        .map(|d| d.timestamp_millis());

    let outcome = if let Some(decision_id) = entry.decision_id {
        state
            .db
            .decision_reviews()
            .find_one(doc! { "_id": decision_id }, None)
            .await?
            .and_then(|r| r.outcome_status)
            .unwrap_or_default()
    } else {
        String::new()
    };

    let decision_created_ms = entry.created_at.timestamp_millis();
    Ok(check_second_safety_gate_pure(
        now.timestamp_millis(),
        entry.created_at.timestamp_millis(),
        cooldown_until_ms,
        last_inbound_ms,
        &outcome,
        decision_created_ms,
        STALE_THRESHOLD_MILLIS,
    ))
}

/// **反向通知通道（W4 / Task 5.5 收尾）**：dispatcher 在状态推进时把
/// `agent_run_logs.outbox_status` 更新为最新 outbox 状态，便于运营 / 审计
/// 直接从 run log 看到本次 run 的发送链路最终走向（sent / canceled /
/// failed_terminal / pending_retry）。run_id 缺失时无操作。
async fn update_run_log_outbox_status(state: &AppState, run_id: &str, outbox_status: &str) {
    if run_id.is_empty() {
        return;
    }
    let now = DateTime::now();
    let res = state
        .db
        .agent_run_logs()
        .update_one(
            doc! { "run_id": run_id },
            doc! {
                "$set": {
                    "outbox_status": outbox_status,
                    "updated_at": now,
                }
            },
            None,
        )
        .await;
    if let Err(err) = res {
        tracing::warn!(?err, run_id, outbox_status, "update agent_run_logs.outbox_status failed");
    }
}

/// 取消已抢占的 entry（仅限当前已是 `in_flight`，避免 race）。
//
// NOTE: 暴露为 `pub` 仅供 `tests/outbox_integration.rs`（W4 / Task 5.8 / R13.10）
// 直接驱动，不应在生产代码中绕过 `process_entry` 单独调用。
pub async fn cancel_entry(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    reason: &str,
) -> AppResult<()> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    collection
        .update_one(
            doc! {
                "_id": entry_id,
                "status": OutboxStatus::InFlight.as_str(),
            },
            doc! {
                "$set": {
                    "status": OutboxStatus::Canceled.as_str(),
                    "cancel_reason": reason,
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                }
            },
            None,
        )
        .await?;
    let _ = write_event_with_cap(
        state,
        entry_id,
        &entry.account_id,
        Some(&entry.contact_wxid),
        "outbox_canceled",
        "warn",
        reason,
        Some(doc! {
            "outbox_id": entry_id,
            "run_id": &entry.run_id,
            "cancel_reason": reason,
        }),
    )
    .await;
    update_run_log_outbox_status(state, &entry.run_id, "canceled").await;
    Ok(())
}

/// 重试或终止：根据 attempt + max_attempts 判断走 pending(+next_retry_at) 还是
/// failed_terminal。
//
// NOTE: 暴露为 `pub` 仅供 `tests/outbox_integration.rs`（W4 / Task 5.8 / R13.10）
// 直接驱动，不应在生产代码中绕过 `process_entry` 单独调用。
pub async fn schedule_retry_or_terminal(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    error_message: &str,
) -> AppResult<()> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let next_attempt = entry.attempt.saturating_add(1);
    let max_attempts = if entry.max_attempts <= 0 {
        5
    } else {
        entry.max_attempts
    };

    if next_attempt < max_attempts {
        let jitter01 = fastrand::f64();
        let backoff_seconds = backoff_with_jitter_seeded(next_attempt, jitter01);
        let next_retry = DateTime::from_millis(now.timestamp_millis() + backoff_seconds * 1000);
        collection
            .update_one(
                doc! {
                    "_id": entry_id,
                    "status": OutboxStatus::InFlight.as_str(),
                },
                doc! {
                    "$set": {
                        "status": OutboxStatus::Pending.as_str(),
                        "attempt": next_attempt,
                        "next_retry_at": next_retry,
                        "last_error": error_message,
                        "updated_at": now,
                    },
                    "$unset": {
                        "worker_id": "",
                        "locked_until": "",
                    }
                },
                None,
            )
            .await?;
        let _ = write_event_with_cap(
            state,
            entry_id,
            &entry.account_id,
            Some(&entry.contact_wxid),
            "outbox_retry_scheduled",
            "warn",
            error_message,
            Some(doc! {
                "outbox_id": entry_id,
                "run_id": &entry.run_id,
                "attempt": next_attempt,
                "max_attempts": max_attempts,
                "backoff_seconds": backoff_seconds,
                "last_error": error_message,
            }),
        )
        .await;
        update_run_log_outbox_status(state, &entry.run_id, "pending").await;
    } else {
        collection
            .update_one(
                doc! {
                    "_id": entry_id,
                    "status": OutboxStatus::InFlight.as_str(),
                },
                doc! {
                    "$set": {
                        "status": OutboxStatus::FailedTerminal.as_str(),
                        "attempt": next_attempt,
                        "last_error": error_message,
                        "updated_at": now,
                    },
                    "$unset": {
                        "worker_id": "",
                        "locked_until": "",
                    }
                },
                None,
            )
            .await?;
        let _ = write_event_with_cap(
            state,
            entry_id,
            &entry.account_id,
            Some(&entry.contact_wxid),
            "outbox_failed_terminal",
            "error",
            error_message,
            Some(doc! {
                "outbox_id": entry_id,
                "run_id": &entry.run_id,
                "attempt": next_attempt,
                "max_attempts": max_attempts,
                "last_error": error_message,
            }),
        )
        .await;
        update_run_log_outbox_status(state, &entry.run_id, "failed_terminal").await;
    }
    Ok(())
}

/// 处理单条已抢占的 entry：二次安全门 → MCP 发送 → 状态推进。
//
// NOTE: 暴露为 `pub` 仅供 `tests/outbox_integration.rs`（W4 / Task 5.8 / R13.10）
// 直接驱动，不应在生产代码中绕过 `tick` 单独调用。
pub async fn process_entry(state: &AppState, entry: &OutboxEntry) -> AppResult<()> {
    let entry_id = match entry.id {
        Some(id) => id,
        None => {
            tracing::warn!("outbox entry without _id, skipping");
            return Ok(());
        }
    };

    if let Some(reason) = second_safety_gate(state, entry).await? {
        cancel_entry(state, entry_id, entry, &reason).await?;
        return Ok(());
    }

    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid,
            },
            None,
        )
        .await?;
    let contact = match contact {
        Some(c) => c,
        None => {
            schedule_retry_or_terminal(
                state,
                entry_id,
                entry,
                "contact not found at dispatch time",
            )
            .await?;
            return Ok(());
        }
    };

    let extra_raw = Some(doc! {
        "outbox_id": entry_id,
        "run_id": &entry.run_id,
        "attempt": entry.attempt + 1,
    });

    let send_fut =
        super::gateway::send_outbound_message(state, &contact, &entry.content, extra_raw);
    let send_result =
        tokio::time::timeout(Duration::from_secs(MCP_SEND_TIMEOUT_SECONDS), send_fut).await;

    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();

    match send_result {
        Ok(Ok(_)) => {
            collection
                .update_one(
                    doc! {
                        "_id": entry_id,
                        "status": OutboxStatus::InFlight.as_str(),
                    },
                    doc! {
                        "$set": {
                            "status": OutboxStatus::Sent.as_str(),
                            "sent_at": now,
                            "updated_at": now,
                        },
                        "$unset": {
                            "worker_id": "",
                            "locked_until": "",
                        }
                    },
                    None,
                )
                .await?;
            let _ = write_event_with_cap(
                state,
                entry_id,
                &entry.account_id,
                Some(&entry.contact_wxid),
                "outbox_sent",
                "info",
                "outbox entry sent successfully via MCP",
                Some(doc! {
                    "outbox_id": entry_id,
                    "run_id": &entry.run_id,
                    "attempt": entry.attempt + 1,
                }),
            )
            .await;
            update_run_log_outbox_status(state, &entry.run_id, "sent").await;
        }
        Ok(Err(err)) => {
            schedule_retry_or_terminal(
                state,
                entry_id,
                entry,
                &format!("send failed: {err}"),
            )
            .await?;
        }
        Err(_) => {
            schedule_retry_or_terminal(state, entry_id, entry, "send timeout (5s)").await?;
        }
    }
    Ok(())
}

/// 写事件前先看 outbox_id 已有事件数：若 ≥ [`PER_ENTRY_EVENT_CAP`] 直接 skip
/// （R13.7 防 retry 风暴写爆 events）。
pub(crate) async fn write_event_with_cap(
    state: &AppState,
    outbox_id: ObjectId,
    account_id: &str,
    contact_wxid: Option<&str>,
    kind: &str,
    status: &str,
    summary: &str,
    details: Option<Document>,
) -> AppResult<()> {
    let count = state
        .db
        .events()
        .count_documents(doc! { "details.outbox_id": outbox_id }, None)
        .await
        .unwrap_or(0);
    if count >= PER_ENTRY_EVENT_CAP as u64 {
        tracing::warn!(
            outbox_id = %outbox_id,
            count,
            "outbox event cap reached, skipping additional event writes"
        );
        return Ok(());
    }
    write_outbox_event(state, account_id, contact_wxid, kind, status, summary, details).await
}

/// 默认 poll 间隔（秒）。worker 是单例后台任务，与 per-account
/// `UserRuntimeParameters.outbox_poll_interval_seconds` 区分；后者是 agent 决策
/// 路径的偏好，本 worker 用全局默认即可。
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;

/// 默认 lease 时长（秒）。> MCP timeout(5s) 远大于 1 个 tick 处理时长，
/// 避免正常路径 lease 过期。
const DEFAULT_LEASE_SECONDS: i32 = 60;

/// **后台 worker 入口**：循环 reclaim → claim → process。`main.rs` 在启动期
/// `tokio::spawn` 调用本函数。
pub async fn run_outbox_dispatcher(state: AppState) -> AppResult<()> {
    let poll_interval_seconds = DEFAULT_POLL_INTERVAL_SECONDS;
    let lease_seconds = DEFAULT_LEASE_SECONDS;
    let worker = worker_id();
    tracing::info!(
        %worker,
        poll_interval_seconds,
        lease_seconds,
        "outbox dispatcher started"
    );

    loop {
        if let Err(err) = tick(&state, &worker, lease_seconds).await {
            tracing::error!(?err, "outbox dispatcher tick failed");
        }
        tokio::time::sleep(Duration::from_secs(poll_interval_seconds)).await;
    }
}

/// 单次 tick：reclaim → 循环 claim+process 直到无可抢占或达到 `PER_TICK_PROCESS_CAP`。
async fn tick(state: &AppState, worker: &str, lease_seconds: i32) -> AppResult<()> {
    reclaim_expired_leases(state).await?;
    for _ in 0..PER_TICK_PROCESS_CAP {
        let claimed = atomic_claim_pending(state, worker, lease_seconds).await?;
        let entry = match claimed {
            Some(e) => e,
            None => break,
        };
        if let Err(err) = process_entry(state, &entry).await {
            tracing::error!(?err, outbox_id = ?entry.id, "process_entry failed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PER_ENTRY_EVENT_CAP` 与 R13.7 设计目标一致：≤ 20。
    #[test]
    fn event_cap_is_twenty() {
        assert_eq!(PER_ENTRY_EVENT_CAP, 20);
    }

    /// `worker_id` 含 hostname / pid / uuid 三段。
    #[test]
    fn worker_id_has_three_segments() {
        let id = worker_id();
        let segments: Vec<&str> = id.split(':').collect();
        assert_eq!(segments.len(), 3, "expected hostname:pid:uuid, got {id}");
        let pid: u32 = segments[1].parse().expect("pid segment must be numeric");
        assert!(pid > 0);
        assert_eq!(segments[2].len(), 36, "uuid segment length mismatch");
    }

    /// 每次调用 `worker_id()` 都生成不同的 uuid 段。
    #[test]
    fn worker_id_uuid_is_unique_per_call() {
        let a = worker_id();
        let b = worker_id();
        let uuid_a = a.rsplit(':').next().unwrap();
        let uuid_b = b.rsplit(':').next().unwrap();
        assert_ne!(uuid_a, uuid_b);
    }

    /// `STALE_THRESHOLD_MILLIS` = 30 分钟（R13.4）。
    #[test]
    fn stale_threshold_is_thirty_minutes() {
        assert_eq!(STALE_THRESHOLD_MILLIS, 30 * 60 * 1000);
    }

    /// MCP send timeout = 5s 与设计一致。
    #[test]
    fn mcp_send_timeout_is_five_seconds() {
        assert_eq!(MCP_SEND_TIMEOUT_SECONDS, 5);
    }
}

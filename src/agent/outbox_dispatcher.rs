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
//! - **lease 自动续约不必要**：lease 严格大于单条 send 的外层 timeout（见
//!   `DEFAULT_LEASE_SECONDS` / `MCP_SEND_TIMEOUT_SECONDS`），正常路径一次 tick
//!   内完成；worker 崩溃 → 下一轮 reclaim 自动恢复。

use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::error::AppResult;
use crate::models::{AgentStatus, OutboxEntry};
use crate::routes::AppState;

use super::outbox::{
    backoff_with_jitter_seeded, check_second_safety_gate_pure, write_outbox_event, OutboxStatus,
};
use super::run_envelope::SOURCE_KIND_MANUAL_SEND;

/// dispatcher 对**整条 send**（可含多次顺序 MCP 调用）的外层 timeout。
///
/// 历史值 5s / 30s 在远程 MCP（如 47.108.57.147 + 微信协议出栈）上都太短：
/// MCP 已经把消息发出去 → wechat 协议层回包慢 → 外层 timeout 触发 → **取消正在
/// await 的 send future** → `logged_call_for_account` 尾部的 mcp_logs 写入随之被
/// 丢弃 → timeout 分支的 `mcp_already_succeeded` post-hoc 守卫查不到成功记录 →
/// 误重试 → 同一句话 / 同一文件被发第二次到客户（finding ①）。
///
/// 取值约束（不变量，由 `send_timeout_covers_worst_case_mcp_calls_and_stays_below_lease`
/// 守护）：
///   `mcp::MCP_CLIENT_TIMEOUT_SECONDS × MAX_SEQUENTIAL_MCP_CALLS_PER_SEND`
///     ≤ 本值 < `DEFAULT_LEASE_SECONDS`
/// 左界保证「已送达但回包慢」时是 reqwest 自己超时返回 `Err`（future 跑到尾部照常
/// 写 mcp_logs），而非被外层取消丢日志；右界保证 worker 还在发就不会被 reclaim 成
/// 另一次并发发送。60s × 2 次（媒体上传 + 发送）= 120s 下界，取 150s 留裕度。
const MCP_SEND_TIMEOUT_SECONDS: u64 = 150;

/// timeout 兜底里调 chat_search 核对的独立短超时——核对本身绝不能卡死 dispatcher。
/// 超时/出错即回落本地 mcp_call_logs 核对。
const CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS: u64 = 15;

/// 单条 send 内最坏情况下顺序发起的 MCP HTTP 调用次数。媒体发送 =
/// `media_upload_base64`（media_id 缓存未命中时）+ `message_send_*`，共 2 次；
/// 文本 / 名片各 1 次。dispatcher 外层 timeout 必须 > 本值 × 每次 reqwest 上界
/// （`crate::mcp::MCP_CLIENT_TIMEOUT_SECONDS`），见 `MCP_SEND_TIMEOUT_SECONDS`
/// 的取值约束（finding ①）。仅供下方不变量测试断言时序关系使用。
#[cfg(test)]
const MAX_SEQUENTIAL_MCP_CALLS_PER_SEND: u64 = 2;

/// 二次安全门陈旧度阈值（R13.4：>30min 自动 canceled）。
const STALE_THRESHOLD_MILLIS: i64 = 30 * 60 * 1000;

/// 单条 entry 总事件数上限（R13.7 防 retry 风暴）。
pub(crate) const PER_ENTRY_EVENT_CAP: i64 = 20;

/// 单 tick 处理上限，防止饿死 / 长 tick。
const PER_TICK_PROCESS_CAP: usize = 16;

/// ⑪：账号掉线时 defer 的推迟间隔（秒）。区别于发送失败重试——掉线不是发送失败，
/// 不消耗 max_attempts，只把 next_retry_at 推后一个固定间隔，account.online 恢复后
/// 由 atomic_claim_pending 照常抢占发送。取 60s 与 lease 同档：足够长避免掉线期间
/// 空转 reclaim，又足够短让恢复后及时补发。
const ACCOUNT_OFFLINE_DEFER_SECONDS: i64 = 60;

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
/// 改回 pending；同时清空 worker_id / locked_until，并置 `reclaimed_in_flight=true`。
/// 该标记告诉 `process_entry`：上一个 worker 在写 `sent` 前消失，**可能已把消息
/// 送达 MCP/微信**，重发前须先跑 `mcp_already_succeeded` post-hoc 核对。返回回收条数。
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
                    "reclaimed_in_flight": true,
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
    // finding ②：按 created_at 升序领取（FIFO），保证同一 run 的多段回复（文本
    // seg0/seg1…后接媒体，各段独立 enqueue、created_at 单调递增）按入队顺序发出，
    // 不被 MongoDB 自然序打乱成「媒体先于后续文本段」。_id 作同毫秒并列的稳定兜底
    // （ObjectId 内含进程内递增计数器，等价入队顺序）。
    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .sort(doc! { "created_at": 1, "_id": 1 })
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

/// F-02：max_attempts 兜底口径——与 enqueue 侧（`outbox.rs:244` `<=0→3`）对齐。
/// enqueue 恒产出 ≥1，故 `<=0` 分支对正常入队 entry 是死代码；仅历史脏文档 /
/// 手工写入的 `<=0` 走到，两处同口径才有确定一致行为。
fn effective_max_attempts(raw: i32) -> i32 {
    if raw <= 0 {
        3
    } else {
        raw
    }
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
    let max_attempts = effective_max_attempts(entry.max_attempts);

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

/// ⑪：账号掉线 defer——区别于发送失败重试。掉线不是发送失败，**不消耗
/// max_attempts**：只把 `next_retry_at` 推后 [`ACCOUNT_OFFLINE_DEFER_SECONDS`]、
/// 把 entry 从 `in_flight` 放回 `pending`、attempt 保持不变。account.online
/// 恢复后由 `atomic_claim_pending` 在 next_retry_at 到点后照常抢占发送。
/// 这是 AI 自治判断（账号离线则暂不外发、恢复后自行续发），不走 terminal。
pub async fn defer_account_offline(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
) -> AppResult<()> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let next_retry =
        DateTime::from_millis(now.timestamp_millis() + ACCOUNT_OFFLINE_DEFER_SECONDS * 1000);
    collection
        .update_one(
            doc! {
                "_id": entry_id,
                "status": OutboxStatus::InFlight.as_str(),
            },
            doc! {
                "$set": {
                    // attempt 刻意不变——掉线非发送失败，不耗重试额度、不走 terminal。
                    "status": OutboxStatus::Pending.as_str(),
                    "next_retry_at": next_retry,
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
        "agent.send_deferred_account_offline",
        "deferred",
        "账号离线，本条发送已推迟（AI 自治暂缓外发，恢复后自动续发），不消耗重试额度",
        Some(doc! {
            "outbox_id": entry_id,
            "run_id": &entry.run_id,
            "attempt": entry.attempt,
            "defer_seconds": ACCOUNT_OFFLINE_DEFER_SECONDS,
        }),
    )
    .await;
    update_run_log_outbox_status(state, &entry.run_id, "pending").await;
    Ok(())
}

/// 查某账号 `agent_send_outbox` 中 `status=sent` 的最大 `sent_at`（毫秒）。
/// 无 sent 历史返回 None。靠 (account_id,status,sent_at:-1) 索引取 limit(1)。
async fn account_last_sent_at_ms(state: &AppState, account_id: &str) -> AppResult<Option<i64>> {
    use mongodb::options::FindOneOptions;
    let collection = state.db.collection_agent_send_outbox();
    let opts = FindOneOptions::builder()
        .sort(doc! { "sent_at": -1 })
        .build();
    let doc = collection
        .find_one(
            doc! { "account_id": account_id, "status": OutboxStatus::Sent.as_str() },
            opts,
        )
        .await?;
    Ok(doc.and_then(|e| e.sent_at).map(|d| d.timestamp_millis()))
}

/// 账号级发送间隔闸命中：把本条 reschedule 到 `last_sent_at + interval`。
/// 仿 [`defer_account_offline`]——attempt 不变、不走 terminal、$unset 锁、写事件。
async fn defer_account_pacing(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    next_send_at_ms: i64,
) -> AppResult<()> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let next_retry = DateTime::from_millis(next_send_at_ms);
    collection
        .update_one(
            doc! {
                "_id": entry_id,
                "status": OutboxStatus::InFlight.as_str(),
            },
            doc! {
                "$set": {
                    // attempt 刻意不变——间隔闸非发送失败，不耗重试额度、不走 terminal。
                    "status": OutboxStatus::Pending.as_str(),
                    "next_retry_at": next_retry,
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
        "agent.send_deferred_account_pacing",
        "deferred",
        "账号发送过于密集，本条已按拟人节奏推迟（AI 自治控制外发频率，稍后自动续发），不消耗重试额度",
        Some(doc! {
            "outbox_id": entry_id,
            "run_id": &entry.run_id,
            "attempt": entry.attempt,
        }),
    )
    .await;
    update_run_log_outbox_status(state, &entry.run_id, "pending").await;
    Ok(())
}

/// post-hoc 核对：在 dispatcher timeout 之后，去 `mcp_call_logs` 查 5min 内
/// 是否已经存在 `tool_name=message_send_text` + 同 recipient + 同 content
/// 且 `error=null` 的成功记录。命中说明 MCP 实际上已经把消息送出，只是回包
/// 慢于 dispatcher 的 timeout，再发一次会让客户收到重复消息。
///
/// 时间下界用 `entry.created_at`（再向前回看 5 分钟做容差），避免历史相同
/// 内容的消息误判命中。
async fn mcp_already_succeeded(
    state: &AppState,
    account_id: &str,
    contact_wxid: &str,
    content: &str,
    entry_created_at: DateTime,
) -> AppResult<bool> {
    let lower_bound_millis = entry_created_at
        .timestamp_millis()
        .saturating_sub(5 * 60 * 1000);
    let lower_bound = DateTime::from_millis(lower_bound_millis);
    let count = state
        .db
        .mcp_logs()
        .count_documents(
            doc! {
                "account_id": account_id,
                "tool_name": "message_send_text",
                "request.recipient": contact_wxid,
                "request.content": content,
                "error": null,
                "created_at": { "$gte": lower_bound },
            },
            None,
        )
        .await?;
    Ok(count > 0)
}

/// P1-6：发送前 contact 状态门——纯函数，便于单测。
///
/// 入队后到 dispatcher 抢占之间 contact 可能被运营改成 `normal`（撤管）或
/// `paused`，此时 in-flight MCP 不应继续把消息送出去（违反"撤管即停"语义）。
/// `manual_send`（admin UI 主动发）不受此门约束——admin 已显式确认发送意图。
///
/// 返回 `Some(reason)` 表示应当 cancel；`None` 表示放行。
pub(crate) fn check_contact_status_pure(
    source_kind: &str,
    agent_status: &AgentStatus,
) -> Option<&'static str> {
    if source_kind == SOURCE_KIND_MANUAL_SEND {
        return None;
    }
    match agent_status {
        AgentStatus::Managed => None,
        _ => Some("contact_status_changed_unmanaged"),
    }
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

    if let Some(reason) = check_contact_status_pure(&entry.source_kind, &contact.agent_status) {
        cancel_entry(state, entry_id, entry, reason).await?;
        return Ok(());
    }

    // ⑪：账号掉线时不盲发。webhook 收到 Offline 事件落库 online=false（见 webhooks.rs），
    // 这里发送前查 account.online——掉线则 defer（推后 next_retry_at、不增 attempt、
    // 不走 terminal），account.online 恢复后照常抢占发送。account 查不到时保守放行
    // （默认/历史账号可能未建行），不阻断发送。
    let account = state
        .db
        .accounts()
        .find_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
            },
            None,
        )
        .await?;
    if matches!(&account, Some(acc) if !acc.online) {
        defer_account_offline(state, entry_id, entry).await?;
        return Ok(());
    }

    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();

    // 崩溃恢复幂等门：本条曾被 reclaim（上一个 worker 抢占后在写 sent 前消失），
    // 它可能已把消息送达 MCP/微信。重发前先 post-hoc 核对 mcp_call_logs；命中即
    // 标 sent 不重发，避免客户收到重复消息（与 timeout 分支同一核对函数）。
    if entry.reclaimed_in_flight {
        let already = if entry.referral_card_id.is_some() {
            // 名片无 media_id、tool 不同，text/media 版 post-hoc 核对都不适用。
            // reclaimed 是边缘场景且重复推名片危害小（客户最多多收一张名片），
            // 故跳过核对、放行重发（保守取 false = 视为未发过）。
            Ok(false)
        } else if let Some(asset_id) = entry.media_asset_id.as_deref() {
            // 硬伤④：媒体条目 content 为空、tool 为 message_send_*，text 版核对查不到
            // → 误判没发过 → 重发文件。改用 media_id 定位该素材的成功发送记录。
            super::media_send::media_already_succeeded(
                state,
                &entry.account_id,
                &entry.contact_wxid,
                asset_id,
                entry.created_at,
            )
            .await
        } else {
            mcp_already_succeeded(
                state,
                &entry.account_id,
                &entry.contact_wxid,
                &entry.content,
                entry.created_at,
            )
            .await
        };
        if let Ok(true) = already
        {
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
                            "last_error": "reclaimed after crash but MCP already succeeded — confirmed via mcp_call_logs",
                        },
                        "$unset": {
                            "worker_id": "",
                            "locked_until": "",
                            "reclaimed_in_flight": "",
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
                "outbox_sent_post_hoc",
                "warn",
                "outbox entry confirmed sent post-hoc via mcp_call_logs after crash reclaim",
                Some(doc! {
                    "outbox_id": entry_id,
                    "run_id": &entry.run_id,
                    "attempt": entry.attempt + 1,
                    "reason": "crash_reclaim",
                }),
            )
            .await;
            update_run_log_outbox_status(state, &entry.run_id, "sent").await;
            // 主动发送台账：post-hoc 确认送达同样记一条（素材/名片才记）。
            super::send_ledger::record_send_for_entry(state, entry, &contact, now).await;
            return Ok(());
        }
    }

    // 账号级最小发送间隔闸：查该账号上次实发时刻，距今 < 随机间隔则 reschedule。
    // 防"连珠炮"——单 worker 串行 for 循环里跨客户/多段消息背靠背零间隔发出 = 机器特征。
    // 位置在 reclaim 幂等门之后（不误拦本该 post-hoc 标 sent 的条目）、发送之前。
    // 查询失败 fail-soft 放行（宁可漏限一次也不丢消息）。
    if let Ok(Some(last_sent_ms)) = account_last_sent_at_ms(state, &entry.account_id).await {
        let interval_ms = super::pacing::account_send_interval_ms(
            fastrand::f64(),
            state.config.account_send_min_interval_ms,
            state.config.account_send_max_interval_ms,
        );
        let now_ms = DateTime::now().timestamp_millis();
        if now_ms - last_sent_ms < interval_ms {
            defer_account_pacing(state, entry_id, entry, last_sent_ms + interval_ms).await?;
            return Ok(());
        }
    }

    let extra_raw = Some(doc! {
        "outbox_id": entry_id,
        "run_id": &entry.run_id,
        "attempt": entry.attempt + 1,
    });

    let send_fut = async {
        if let Some(card_id) = entry.referral_card_id.as_deref() {
            super::referral::send_outbound_namecard(state, &contact, card_id).await
        } else if let Some(asset_id) = entry.media_asset_id.as_deref() {
            super::media_send::send_outbound_media(state, &contact, asset_id).await
        } else {
            super::gateway::send_outbound_message(state, &contact, &entry.content, extra_raw).await
        }
    };
    let send_result =
        tokio::time::timeout(Duration::from_secs(MCP_SEND_TIMEOUT_SECONDS), send_fut).await;

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
                            "reclaimed_in_flight": "",
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

            // 主动发送台账：素材/名片条目记一条（纯文本不记）。fail-soft，不影响已成发送。
            super::send_ledger::record_send_for_entry(state, entry, &contact, now).await;
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
            // post-hoc 核对：MCP 调用本身在 timeout 之前可能已经成功把消息送达
            // 微信协议（response 慢于 30s 的极端情况），此时 mcp_call_logs 已写入
            // tool_name + recipient + 定位字段（text=content / media=mediaId）且
            // error=null。命中即视为已送达，不再重发，避免给客户重复消息/重复文件。
            let already = if entry.referral_card_id.is_some() {
                // 名片无 media_id、tool 不同，text/media 版 post-hoc 核对都不适用。
                // timeout 是边缘场景且重复推名片危害小（客户最多多收一张名片），
                // 故跳过核对、放行重发（保守取 false = 视为未发过）。
                Ok(false)
            } else if let Some(asset_id) = entry.media_asset_id.as_deref() {
                super::media_send::media_already_succeeded(
                    state,
                    &entry.account_id,
                    &entry.contact_wxid,
                    asset_id,
                    entry.created_at,
                )
                .await
            } else {
                // 先查 MCP chat_search(server 真实已发记录，同步落库、失败不写)——不受本地
                // timeout 取消 mcp_call_logs 写入的影响。带独立短超时；超时/出错回落本地日志核对。
                match tokio::time::timeout(
                    Duration::from_secs(CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS),
                    crate::mcp::chat_search_outbound(
                        state,
                        &entry.account_id,
                        &entry.contact_wxid,
                        &entry.content,
                        entry.created_at,
                    ),
                )
                .await
                {
                    Ok(Ok(hit)) => Ok(hit),
                    // chat_search 出错 / 超时 → 回落本地 mcp_call_logs 核对(不倒退)。
                    Ok(Err(_)) | Err(_) => {
                        mcp_already_succeeded(
                            state,
                            &entry.account_id,
                            &entry.contact_wxid,
                            &entry.content,
                            entry.created_at,
                        )
                        .await
                    }
                }
            };
            if let Ok(true) = already
            {
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
                                "last_error": "send timeout (150s) but MCP already succeeded — confirmed via chat_search/mcp_call_logs",
                            },
                            "$unset": {
                                "worker_id": "",
                                "locked_until": "",
                                "reclaimed_in_flight": "",
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
                    "outbox_sent_post_hoc",
                    "warn",
                    "outbox entry confirmed sent post-hoc via chat_search/mcp_call_logs after timeout",
                    Some(doc! {
                        "outbox_id": entry_id,
                        "run_id": &entry.run_id,
                        "attempt": entry.attempt + 1,
                    }),
                )
                .await;
                update_run_log_outbox_status(state, &entry.run_id, "sent").await;
                // 主动发送台账：超时 post-hoc 确认送达同样记一条（素材/名片才记）。
                super::send_ledger::record_send_for_entry(state, entry, &contact, now).await;
            } else {
                schedule_retry_or_terminal(state, entry_id, entry, "send timeout (150s)").await?;
            }
        }
    }
    Ok(())
}

/// P2-3：闭集化 cap 后行为，方便单测。
///
/// * `WriteNormal` —— 未达 cap，正常写事件。
/// * `WriteSentinel` —— 首次达到 cap，写一条 `outbox.event_cap_reached`
///   sentinel 事件后转 silent。
/// * `Silent` —— 已经写过 sentinel，直接静音。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CapDecision {
    WriteNormal,
    WriteSentinel,
    Silent,
}

/// 纯函数：根据 outbox_id 已有事件计数 + 是否已写过 sentinel 决定行为。
pub(crate) fn decide_cap_action(count: u64, sentinel_already: bool) -> CapDecision {
    if count < PER_ENTRY_EVENT_CAP as u64 {
        CapDecision::WriteNormal
    } else if sentinel_already {
        CapDecision::Silent
    } else {
        CapDecision::WriteSentinel
    }
}

/// 写事件前先看 outbox_id 已有事件数：若 ≥ [`PER_ENTRY_EVENT_CAP`]
/// 则**仅写一条** `outbox.event_cap_reached` sentinel 后转 silent，否则
/// 正常写事件。Sentinel 通过 `details.kind == "event_cap_reached"` 去重，
/// 防止 retry 风暴写爆 events，又保证仪表盘能感知"该 entry 被截断"。
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
    if count < PER_ENTRY_EVENT_CAP as u64 {
        return write_outbox_event(
            state,
            account_id,
            contact_wxid,
            kind,
            status,
            summary,
            details,
        )
        .await;
    }
    let sentinel_already = state
        .db
        .events()
        .count_documents(
            doc! {
                "details.outbox_id": outbox_id,
                "details.kind": "event_cap_reached",
            },
            None,
        )
        .await
        .unwrap_or(0)
        > 0;
    match decide_cap_action(count, sentinel_already) {
        CapDecision::WriteNormal => {
            // 不会进入此分支：count >= cap 已在上方早返。
            unreachable!("decide_cap_action returned WriteNormal at or above cap")
        }
        CapDecision::Silent => {
            tracing::warn!(
                outbox_id = %outbox_id,
                count,
                "outbox event cap reached, skipping additional event writes"
            );
            Ok(())
        }
        CapDecision::WriteSentinel => {
            let sentinel = doc! {
                "outbox_id": outbox_id,
                "kind": "event_cap_reached",
                "cap": PER_ENTRY_EVENT_CAP,
                "observed_count": count as i64,
                "suppressed_kind": kind,
                "suppressed_status": status,
            };
            write_outbox_event(
                state,
                account_id,
                contact_wxid,
                "outbox.event_cap_reached",
                "capped",
                "outbox event cap reached, further events suppressed",
                Some(sentinel),
            )
            .await
        }
    }
}

/// 默认 poll 间隔（秒）。worker 是单例后台任务，与 per-account
/// `UserRuntimeParameters.outbox_poll_interval_seconds` 区分；后者是 agent 决策
/// 路径的偏好，本 worker 用全局默认即可。
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;

/// 默认 lease 时长（秒）。必须 **严格大于** `MCP_SEND_TIMEOUT_SECONDS`（见其
/// 取值约束不变量）：否则一条正在发送（最坏可达 150s）的 entry 会在 lease 到期
/// 时被 `reclaim_expired_leases` 回收成 pending，另一次 tick 抢占后并发再发一遍 →
/// 客户收重复消息。取 180s（> 150s send 上界）：worker 崩溃后 entry 至多滞留
/// 180s 才被 reclaim（单 worker 场景下这是可接受的恢复延迟）。
const DEFAULT_LEASE_SECONDS: i32 = 180;

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

    /// P1-6：managed contact 走 inbound_message 路径——放行。
    #[test]
    fn contact_status_gate_allows_managed_inbound() {
        assert!(
            check_contact_status_pure("inbound_message", &AgentStatus::Managed).is_none(),
            "managed contact 应当放行"
        );
    }

    /// P1-6：撤管后非 manual_send 路径必须 cancel——不能让 in-flight MCP
    /// 把消息发给已经退出托管的 contact。
    #[test]
    fn contact_status_gate_cancels_normal_when_not_manual() {
        let reason = check_contact_status_pure("inbound_message", &AgentStatus::Normal);
        assert_eq!(reason, Some("contact_status_changed_unmanaged"));
        let reason = check_contact_status_pure("follow_up_task", &AgentStatus::Normal);
        assert_eq!(reason, Some("contact_status_changed_unmanaged"));
    }

    /// P1-6：admin 主动 manual_send 不受 agent_status 门约束——admin
    /// 已显式确认发送意图（同 admin 直接联系语义）。
    #[test]
    fn contact_status_gate_passthrough_for_manual_send() {
        assert!(
            check_contact_status_pure(SOURCE_KIND_MANUAL_SEND, &AgentStatus::Normal).is_none(),
            "manual_send 不受 agent_status 门约束"
        );
        assert!(
            check_contact_status_pure(SOURCE_KIND_MANUAL_SEND, &AgentStatus::Managed).is_none()
        );
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

    /// finding ①：dispatcher 对整条 send 的外层 timeout 必须覆盖「单条 send 内
    /// 最坏顺序 MCP 调用次数 × 每次 reqwest 上界」，否则慢响应下外层 timeout 会
    /// 取消正在 await 的 send future → mcp_logs 写入被丢 → post-hoc 守卫查不到
    /// 成功记录 → 误重试 → 客户收重复消息。且必须 < lease，避免 worker 还在发
    /// 就被 reclaim 成另一次并发发送。
    #[test]
    fn send_timeout_covers_worst_case_mcp_calls_and_stays_below_lease() {
        let worst_case_send_seconds =
            crate::mcp::MCP_CLIENT_TIMEOUT_SECONDS * MAX_SEQUENTIAL_MCP_CALLS_PER_SEND;
        assert!(
            MCP_SEND_TIMEOUT_SECONDS >= worst_case_send_seconds,
            "dispatcher timeout {}s 必须 ≥ 最坏 send 耗时 {}s（reqwest {}s × {} 次），\
             否则外层 timeout 会取消已送达但回包慢的 send，丢掉 mcp_logs 写入 → 重复发送",
            MCP_SEND_TIMEOUT_SECONDS,
            worst_case_send_seconds,
            crate::mcp::MCP_CLIENT_TIMEOUT_SECONDS,
            MAX_SEQUENTIAL_MCP_CALLS_PER_SEND,
        );
        assert!(
            (MCP_SEND_TIMEOUT_SECONDS as i32) < DEFAULT_LEASE_SECONDS,
            "dispatcher timeout {}s 必须 < lease {}s，否则 send 未完就被 reclaim 重发",
            MCP_SEND_TIMEOUT_SECONDS,
            DEFAULT_LEASE_SECONDS,
        );
    }

    /// P2-3：cap 未触发 → 正常写事件。
    #[test]
    fn decide_cap_action_below_cap_writes_normal() {
        for c in [0u64, 1, 5, (PER_ENTRY_EVENT_CAP - 1) as u64] {
            assert_eq!(
                decide_cap_action(c, false),
                CapDecision::WriteNormal,
                "count={c} 应正常写"
            );
            assert_eq!(
                decide_cap_action(c, true),
                CapDecision::WriteNormal,
                "count={c} 即使已写 sentinel 也按正常写（应不可能但要稳）"
            );
        }
    }

    /// P2-3：首次到达 cap → 写一次 sentinel。
    #[test]
    fn decide_cap_action_first_hit_writes_sentinel() {
        assert_eq!(
            decide_cap_action(PER_ENTRY_EVENT_CAP as u64, false),
            CapDecision::WriteSentinel
        );
        assert_eq!(
            decide_cap_action((PER_ENTRY_EVENT_CAP + 5) as u64, false),
            CapDecision::WriteSentinel,
            "已经超出 cap 但 sentinel 还没写时，仍要补写一次"
        );
    }

    /// P2-3：sentinel 已写 → 静音；防止 retry 风暴写爆 events。
    #[test]
    fn decide_cap_action_silent_after_sentinel() {
        assert_eq!(
            decide_cap_action(PER_ENTRY_EVENT_CAP as u64, true),
            CapDecision::Silent
        );
        assert_eq!(
            decide_cap_action((PER_ENTRY_EVENT_CAP + 100) as u64, true),
            CapDecision::Silent
        );
    }

    /// 崩溃恢复幂等门：旧 outbox 文档（落库于 `reclaimed_in_flight` 字段引入
    /// 之前）缺该字段，必须按 `#[serde(default)]` 反序列化为 `false`（R11
    /// 向后兼容）。否则 process_entry 会对每条历史 entry 误跑 post-hoc 核对。
    #[test]
    fn legacy_outbox_doc_defaults_reclaimed_in_flight_false() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let legacy = doc! {
            "_id": ObjectId::new(),
            "workspace_id": "default",
            "account_id": "wx_acc_1",
            "contact_wxid": "wxid_alice",
            "run_id": "run-001",
            "source_event_id": "evt-001",
            "source_kind": "inbound_message",
            "content": "你好",
            "content_hash": "abc",
            "idempotency_key": "key-1",
            "attempt": 0_i32,
            "max_attempts": 3_i32,
            "status": "pending",
            "created_at": DateTime::now(),
            "updated_at": DateTime::now(),
        };
        let entry: crate::models::OutboxEntry =
            mongodb::bson::from_document(legacy).expect("legacy doc should deserialize");
        assert!(
            !entry.reclaimed_in_flight,
            "缺字段的旧文档必须默认 reclaimed_in_flight=false"
        );
    }

    /// F-02：dispatcher 侧 max_attempts 兜底须与 enqueue 侧（outbox.rs:244 `<=0→3`）
    /// 同口径。历史脏文档 / 手工写入的 max_attempts<=0 时，两处兜底一致才有确定行为。
    /// 该分支对 enqueue 正常产出的 entry 是死代码（enqueue 恒产出 ≥1），此测锁定口径对齐。
    /// 驱动生产纯函数 effective_max_attempts——改回 `<=0→5` 即变红（真回归哨兵，非 tautology）。
    #[test]
    fn effective_max_attempts_fallback_aligns_with_enqueue() {
        assert_eq!(effective_max_attempts(0), 3, "max_attempts=0 兜底须为 3(对齐 enqueue outbox.rs:244)");
        assert_eq!(effective_max_attempts(-1), 3, "max_attempts<0 兜底须为 3");
        assert_eq!(effective_max_attempts(1), 1, "max_attempts>0 原样透传");
        assert_eq!(effective_max_attempts(5), 5, "max_attempts>0 原样透传");
    }
}

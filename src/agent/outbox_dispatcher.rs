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

use std::{sync::LazyLock, time::Duration};

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument, UpdateOptions};

use crate::error::AppResult;
use crate::models::{AgentStatus, AgentTask, OutboxEntry};
use crate::routes::AppState;

use super::outbox::{
    backoff_with_jitter_seeded, check_second_safety_gate_pure, write_outbox_event, OutboxStatus,
};
use super::run_envelope::{
    SOURCE_KIND_PRINCIPAL_CLARIFICATION, SOURCE_KIND_PRINCIPAL_ESCALATION,
    SOURCE_KIND_SYSTEM_INCIDENT,
};

fn principal_card_source_identity(source_event_id: &str) -> Option<(ObjectId, i64)> {
    let mut parts = source_event_id.split(':');
    if parts.next()? != "principal-card" {
        return None;
    }
    let escalation_id = ObjectId::parse_str(parts.next()?).ok()?;
    let generation = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || generation < 1 {
        return None;
    }
    Some((escalation_id, generation))
}

/// Confirm that an internal principal card still belongs to the current frozen
/// escalation generation. This is checked after claim and again at the last
/// cancellable point so an admin resolution/reassignment cannot leave an old
/// queued card authorized for delivery.
///
/// The enqueue operation necessarily precedes the escalation acknowledgement.
/// If the dispatcher wins that narrow race, it completes the same generation
/// CAS itself instead of canceling a valid card.
async fn principal_card_send_is_authorized(
    state: &AppState,
    outbox_id: ObjectId,
    entry: &OutboxEntry,
) -> AppResult<bool> {
    if entry.source_kind != SOURCE_KIND_PRINCIPAL_ESCALATION {
        return Ok(true);
    }
    let Some((escalation_id, generation)) = principal_card_source_identity(&entry.source_event_id)
    else {
        return Ok(false);
    };

    let base_filter = doc! {
        "_id": escalation_id,
        "workspace_id": &entry.workspace_id,
        "status": crate::models::PRINCIPAL_ESCALATION_STATUS_PENDING,
        "principal_wxid": &entry.contact_wxid,
        "protocol.principal_account_id": &entry.account_id,
        "protocol.delivery_generation": generation,
        "protocol.delivery_content": &entry.content,
    };
    let mut acknowledged_filter = base_filter.clone();
    acknowledged_filter.insert(
        "protocol.delivery_state",
        crate::models::PRINCIPAL_CARD_DELIVERY_QUEUED,
    );
    acknowledged_filter.insert("protocol.delivery_outbox_id", outbox_id);
    if state
        .db
        .agent_principal_escalations()
        .find_one(acknowledged_filter, None)
        .await?
        .is_some()
    {
        return Ok(true);
    }

    let mut pending_ack_filter = base_filter.clone();
    pending_ack_filter.insert(
        "protocol.delivery_state",
        crate::models::PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE,
    );
    pending_ack_filter.insert("protocol.delivery_outbox_id", doc! { "$exists": false });
    let acknowledged = state
        .db
        .agent_principal_escalations()
        .update_one(
            pending_ack_filter,
            doc! { "$set": {
                "protocol.delivery_state": crate::models::PRINCIPAL_CARD_DELIVERY_QUEUED,
                "protocol.delivery_outbox_id": outbox_id,
            } },
            None,
        )
        .await?;
    if acknowledged.modified_count == 1 {
        return Ok(true);
    }

    // The escalation reconciler may have acknowledged the same outbox between
    // our initial read and CAS. Re-read the exact current-generation identity
    // before treating a zero-modification CAS as stale.
    let mut concurrently_acknowledged_filter = base_filter;
    concurrently_acknowledged_filter.insert(
        "protocol.delivery_state",
        crate::models::PRINCIPAL_CARD_DELIVERY_QUEUED,
    );
    concurrently_acknowledged_filter.insert("protocol.delivery_outbox_id", outbox_id);
    Ok(state
        .db
        .agent_principal_escalations()
        .find_one(concurrently_acknowledged_filter, None)
        .await?
        .is_some())
}

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

/// F-04：单条 entry 允许被 reclaim 的上限。超过则转 failed_terminal——worker 反复在
/// 同位置崩溃（无限 reclaim 永不进终态）时止损交 admin。reclaim ≠ 发送 attempt。
const OUTBOX_MAX_RECLAIMS: i32 = 5;

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
/// 送达 MCP/微信**，重发前须先跑 `verify_already_sent` post-hoc 核对（文本先查权威
/// chat_search、失败回落本地 mcp_call_logs）。返回回收条数。
//
// NOTE: 暴露为 `pub` 仅供 `tests/outbox_integration.rs`（W4 / Task 5.8 / R13.10）
// 直接驱动，不应在生产代码中绕过 `tick` 单独调用。
pub async fn reclaim_expired_leases(state: &AppState) -> AppResult<u64> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    // A cancellation that won before the irreversible send boundary can be completed safely.
    let canceled = collection
        .update_many(
            doc! {
                "status": OutboxStatus::InFlight.as_str(),
                "locked_until": { "$lt": now },
                "cancel_requested": true,
                "$or": [
                    { "send_started_at": { "$exists": false } },
                    { "send_started_at": null },
                ],
            },
            doc! {
                "$set": {
                    "status": OutboxStatus::Canceled.as_str(),
                    "updated_at": now,
                    "cancel_reason": "cancel request recovered before remote send",
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                    "claim_token": "",
                },
            },
            None,
        )
        .await?;
    // A namecard has no authoritative post-hoc query. A cancellation that arrived after any
    // payload crossed the remote boundary is also no longer safe to report as canceled or replay.
    // Stop both cases in an explicit terminal instead of risking a duplicate delivery.
    let unknown = collection
        .update_many(
            doc! {
                "status": OutboxStatus::InFlight.as_str(),
                "locked_until": { "$lt": now },
                "send_started_at": { "$ne": null },
                "$or": [
                    { "referral_card_id": { "$ne": null } },
                    { "cancel_requested": true },
                ],
            },
            doc! {
                "$set": {
                    "status": OutboxStatus::DeliveryUnknown.as_str(),
                    "updated_at": now,
                    "last_error": "namecard worker lease expired after remote send boundary; delivery requires manual verification",
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                    "claim_token": "",
                },
            },
            None,
        )
        .await?;
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
                    "claim_token": "",
                    "send_started_at": "",
                },
                "$inc": { "reclaim_count": 1 },
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
    // F-04：reclaim_count 超上限的 entry 转 failed_terminal。单 update 无法按 $inc 后的
    // 新值分流，故单独一遍 update_many（幂等：已 failed_terminal 不再匹配 status:Pending）。
    let terminated = collection
        .update_many(
            doc! {
                "status": OutboxStatus::Pending.as_str(),
                "reclaim_count": { "$gt": OUTBOX_MAX_RECLAIMS },
            },
            doc! {
                "$set": {
                    "status": OutboxStatus::FailedTerminal.as_str(),
                    "updated_at": now,
                    "last_error": "reclaim 超限（worker 反复崩溃，止损转终态）",
                }
            },
            None,
        )
        .await?;
    if terminated.modified_count > 0 {
        tracing::warn!(
            terminated = terminated.modified_count,
            "outbox reclaim 超限转 failed_terminal"
        );
    }
    Ok(result.modified_count + canceled.modified_count + unknown.modified_count)
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
    atomic_claim_pending_with_policy(state, worker, lease_seconds, false).await
}

/// Priority is the normal policy. Every fixed number of dispatcher claims, the caller requests
/// oldest-first ordering so low-priority operational/media rows cannot starve indefinitely.
async fn atomic_claim_pending_with_policy(
    state: &AppState,
    worker: &str,
    lease_seconds: i32,
    prefer_oldest: bool,
) -> AppResult<Option<OutboxEntry>> {
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let lease_ms = (lease_seconds.max(1) as i64) * 1000;
    let lease_until = DateTime::from_millis(now.timestamp_millis() + lease_ms);
    let claim_token = uuid::Uuid::new_v4().to_string();

    let filter = doc! {
        "status": OutboxStatus::Pending.as_str(),
        // Defensive invariant: a persisted cancellation intent must never be erased by a new
        // claim. Normal pending rows have this field false/missing; a true value means an older
        // transition lost a race and must be reconciled instead of sent.
        "cancel_requested": { "$ne": true },
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
            "claim_token": &claim_token,
            "locked_until": lease_until,
            "updated_at": now,
            "cancel_requested": false,
        },
        "$inc": { "claim_generation": 1i64 },
        "$unset": {
            "cancel_requested_at": "",
            "send_started_at": "",
        },
    };
    let sort = if prefer_oldest {
        // Aging pass: bounded FIFO service prevents starvation under a sustained inbound load.
        doc! { "created_at": 1, "run_sequence": 1, "_id": 1 }
    } else {
        // Customer/manual text wins globally; sequence remains stable inside equal-priority runs.
        doc! { "delivery_priority": -1, "created_at": 1, "run_sequence": 1, "_id": 1 }
    };
    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .sort(sort)
        .build();
    Ok(collection
        .find_one_and_update(filter, update, options)
        .await?)
}

fn active_claim_filter(entry_id: ObjectId, entry: &OutboxEntry) -> Option<Document> {
    let worker_id = entry.worker_id.as_deref()?;
    let claim_token = entry.claim_token.as_deref()?;
    Some(doc! {
        "_id": entry_id,
        "status": OutboxStatus::InFlight.as_str(),
        "worker_id": worker_id,
        "claim_token": claim_token,
    })
}

fn send_not_started_filter() -> Document {
    doc! {
        "$or": [
            { "send_started_at": { "$exists": false } },
            { "send_started_at": null },
        ]
    }
}

/// SR-034：Task 产生的 Outbox 在触达 MCP 前必须证明发送意图已由同一 task claim
/// 提交。非 Task / 历史 Outbox 没有 source_task_* 元数据，保持原有放行语义。
#[derive(Debug, Clone, PartialEq, Eq)]
enum TaskSendAuthorization {
    /// `None` 表示非 Task / 历史 Outbox；`Some(token)` 表示新协议 marker 已提交。
    Authorized(Option<String>),
    /// Gateway 已绑定 decision，但尚未完成全部分段/素材入队并提交 task 终态。
    Building,
    /// task 已被 reclaim、取消、换 token，或 decision 绑定已被新 owner 替换。
    Stale(String),
}

fn classify_task_send_authorization(
    status: &str,
    binding_token: &str,
    task_claim_token: Option<&str>,
    decision_matches: bool,
    marker: Option<&str>,
) -> TaskSendAuthorization {
    if task_claim_token != Some(binding_token) || !decision_matches {
        return TaskSendAuthorization::Stale(
            "source task claim or decision binding was replaced".to_string(),
        );
    }
    if marker.is_some_and(|marker| marker != binding_token) {
        return TaskSendAuthorization::Stale(
            "outbox authorization marker does not match task claim".to_string(),
        );
    }
    match status {
        "running" => TaskSendAuthorization::Building,
        // `sent` is reached only after all text segments are delivered. Media/namecard entries
        // belonging to the same decision may still be pending and remain authorized by the same
        // immutable token + outbox_decision_id pair.
        "outbox_enqueued" | "sent" => match marker {
            Some(marker) => TaskSendAuthorization::Authorized(Some(marker.to_string())),
            None => TaskSendAuthorization::Building,
        },
        other => TaskSendAuthorization::Stale(format!(
            "source task is no longer send-authorized (status={other})"
        )),
    }
}

async fn task_send_authorization(
    state: &AppState,
    entry: &OutboxEntry,
) -> AppResult<TaskSendAuthorization> {
    let Some(decision_id) = entry.decision_id else {
        return Ok(TaskSendAuthorization::Authorized(None));
    };
    let Some(review) = state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": decision_id }, None)
        .await?
    else {
        // 历史/手工 Outbox 允许没有 review；Task Outbox 的 review 在 enqueue 前已落库。
        return Ok(TaskSendAuthorization::Authorized(None));
    };
    // The decision review is the durable batch seal for every decision-backed Outbox. Rows may
    // be claimed while the Gateway is still creating later text segments; until the review moves
    // from `outbox_enqueuing` to `outbox_enqueued`, no segment may cross the remote boundary.
    //
    // For task-backed decisions, inspect the immutable task claim *before* returning Building.
    // A newer inbound can revoke that claim while the old review is still outbox_enqueuing; such
    // rows are stale and must converge to canceled rather than being deferred forever.
    let review_building = match review.status.as_str() {
        "outbox_enqueuing" => true,
        "outbox_enqueued" | "sent" => false,
        other => {
            return Ok(TaskSendAuthorization::Stale(format!(
                "decision batch is no longer send-authorized (status={other})"
            )))
        }
    };
    let binding = match (
        review.source_task_id,
        review.source_task_claim_token.as_deref(),
    ) {
        (None, None) => {
            return Ok(if review_building {
                TaskSendAuthorization::Building
            } else {
                TaskSendAuthorization::Authorized(None)
            })
        }
        (Some(task_id), Some(token)) if !token.trim().is_empty() => (task_id, token.to_string()),
        _ => {
            return Ok(TaskSendAuthorization::Stale(
                "task authorization metadata incomplete".to_string(),
            ));
        }
    };
    let task = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": binding.0 }, None)
        .await?;
    let Some(task) = task else {
        return Ok(TaskSendAuthorization::Stale(
            "source task no longer exists".to_string(),
        ));
    };
    let status = task.get_str("status").unwrap_or_default();
    let task_token = task.get_str("claim_token").ok();
    let decision_matches = task.get_object_id("outbox_decision_id").ok() == Some(decision_id);
    let mut marker = entry.task_send_authorization_token.as_deref();
    // Text authorization may commit before optional media/namecard rows are materialized. A row
    // missing its marker may inherit it only from the same already-committed task/token/decision.
    if marker.is_none()
        && matches!(status, "outbox_enqueued" | "sent")
        && task_token == Some(binding.1.as_str())
        && decision_matches
    {
        if let Some(entry_id) = entry.id {
            let repaired = state
                .db
                .collection_agent_send_outbox()
                .update_one(
                    doc! {
                        "_id": entry_id,
                        "decision_id": decision_id,
                        "status": OutboxStatus::InFlight.as_str(),
                        "$or": [
                            { "task_send_authorization_token": { "$exists": false } },
                            { "task_send_authorization_token": null },
                        ],
                    },
                    doc! { "$set": {
                        "task_send_authorization_token": &binding.1,
                        "updated_at": DateTime::now(),
                    } },
                    None,
                )
                .await?;
            if repaired.matched_count == 1 {
                marker = Some(binding.1.as_str());
            }
        }
    }
    let task_authorization =
        classify_task_send_authorization(status, &binding.1, task_token, decision_matches, marker);
    Ok(match task_authorization {
        TaskSendAuthorization::Stale(reason) => TaskSendAuthorization::Stale(reason),
        _ if review_building => TaskSendAuthorization::Building,
        other => other,
    })
}

/// Gateway 仍在构建同一 decision 时无损退回 pending。此路径不是发送失败，也不是
/// worker 崩溃：不增加 attempt/reclaim_count，不置 reclaimed_in_flight。
async fn defer_until_task_authorized(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
) -> AppResult<()> {
    let Some(mut filter) = active_claim_filter(entry_id, entry) else {
        return Ok(());
    };
    filter.insert("cancel_requested", doc! { "$ne": true });
    filter.extend(send_not_started_filter());
    let now = DateTime::now();
    let next_retry_at = DateTime::from_millis(now.timestamp_millis() + 1_000);
    let result = state
        .db
        .collection_agent_send_outbox()
        .update_one(
            filter,
            doc! {
                "$set": {
                    "status": OutboxStatus::Pending.as_str(),
                    "next_retry_at": next_retry_at,
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                    "claim_token": "",
                    "send_started_at": "",
                },
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        let _ = complete_requested_cancel_before_send(state, entry_id, entry).await?;
    } else {
        refresh_run_log_outbox_status(state, &entry.run_id).await;
    }
    Ok(())
}

async fn enforce_task_send_authorization(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
) -> AppResult<Option<Option<String>>> {
    match task_send_authorization(state, entry).await? {
        TaskSendAuthorization::Authorized(token) => Ok(Some(token)),
        TaskSendAuthorization::Building => {
            defer_until_task_authorized(state, entry_id, entry).await?;
            Ok(None)
        }
        TaskSendAuthorization::Stale(reason) => {
            cancel_entry(
                state,
                entry_id,
                entry,
                &format!("stale_task_claim: {reason}"),
            )
            .await?;
            Ok(None)
        }
    }
}

/// Finish an in-flight cancellation only while the remote send boundary has not been crossed.
/// Returns true only when this worker still owned the claim and committed the cancellation.
async fn complete_requested_cancel_before_send(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
) -> AppResult<bool> {
    let Some(mut filter) = active_claim_filter(entry_id, entry) else {
        return Ok(false);
    };
    filter.insert("cancel_requested", true);
    filter.extend(send_not_started_filter());
    let now = DateTime::now();
    let result = state
        .db
        .collection_agent_send_outbox()
        .update_one(
            filter,
            doc! {
                "$set": {
                    "status": OutboxStatus::Canceled.as_str(),
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                    "claim_token": "",
                },
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Ok(false);
    }
    let reason = entry
        .cancel_reason
        .as_deref()
        .unwrap_or("cancel requested before remote send");
    let _ = write_event_with_cap(
        state,
        &entry.workspace_id,
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
            "claim_generation": entry.claim_generation,
        }),
    )
    .await;
    refresh_run_log_outbox_status(state, &entry.run_id).await;
    Ok(true)
}

/// Last cancellable CAS. A successful update proves this claim is still current and no cancel
/// request won before the irreversible MCP call boundary.
fn remote_send_start_filter(
    entry_id: ObjectId,
    worker_id: &str,
    outbox_claim_token: &str,
    task_authorization_token: Option<&str>,
) -> Document {
    let mut filter = doc! {
        "_id": entry_id,
        "status": OutboxStatus::InFlight.as_str(),
        "worker_id": worker_id,
        "claim_token": outbox_claim_token,
        "cancel_requested": { "$ne": true },
    };
    if let Some(token) = task_authorization_token {
        filter.insert("task_send_authorization_token", token);
    }
    filter.extend(send_not_started_filter());
    filter
}

async fn begin_remote_send(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    task_authorization_token: Option<&str>,
) -> AppResult<bool> {
    let (Some(worker_id), Some(outbox_claim_token)) =
        (entry.worker_id.as_deref(), entry.claim_token.as_deref())
    else {
        return Ok(false);
    };
    // SR-034 最后不可逆边界只依赖当前 Outbox 单文档：该 marker 只能在持有
    // 同一 Task claim token 的 owner 成功提交 task 授权后写入。
    let filter = remote_send_start_filter(
        entry_id,
        worker_id,
        outbox_claim_token,
        task_authorization_token,
    );
    let now = DateTime::now();
    let result = state
        .db
        .collection_agent_send_outbox()
        .update_one(
            filter,
            doc! { "$set": {
                "send_started_at": now,
                "updated_at": now,
            } },
            None,
        )
        .await?;
    if result.matched_count == 1 {
        return Ok(true);
    }
    let _ = complete_requested_cancel_before_send(state, entry_id, entry).await?;
    Ok(false)
}

async fn commit_sent_if_owned(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    note: Option<&str>,
) -> AppResult<bool> {
    let Some(filter) = active_claim_filter(entry_id, entry) else {
        return Ok(false);
    };
    let now = DateTime::now();
    let mut set = doc! {
        "status": OutboxStatus::Sent.as_str(),
        "sent_at": now,
        "updated_at": now,
        "delivery_finalize_pending": entry.decision_id.is_some()
            && entry.media_asset_id.is_none()
            && entry.referral_card_id.is_none(),
    };
    if let Some(note) = note {
        set.insert("last_error", note);
    }
    let unset = sent_unset_fields(note.is_some());
    let result = state
        .db
        .collection_agent_send_outbox()
        .update_one(
            filter,
            doc! {
                "$set": set,
                "$unset": unset,
            },
            None,
        )
        .await?;
    Ok(result.matched_count == 1)
}

fn sent_unset_fields(preserve_diagnostic_note: bool) -> Document {
    let mut unset = doc! {
        "worker_id": "",
        "locked_until": "",
        "claim_token": "",
        "reclaimed_in_flight": "",
    };
    if !preserve_diagnostic_note {
        unset.insert("last_error", "");
    }
    unset
}

async fn mark_delivery_unknown_if_owned(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    reason: &str,
) -> AppResult<bool> {
    let Some(filter) = active_claim_filter(entry_id, entry) else {
        return Ok(false);
    };
    let now = DateTime::now();
    let result = state
        .db
        .collection_agent_send_outbox()
        .update_one(
            filter,
            doc! {
                "$set": {
                    "status": OutboxStatus::DeliveryUnknown.as_str(),
                    "last_error": reason,
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                    "claim_token": "",
                    "reclaimed_in_flight": "",
                },
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Ok(false);
    }
    let _ = write_event_with_cap(
        state,
        &entry.workspace_id,
        entry_id,
        &entry.account_id,
        Some(&entry.contact_wxid),
        "outbox_delivery_unknown",
        "warning",
        reason,
        Some(doc! {
            "outbox_id": entry_id,
            "run_id": &entry.run_id,
            "claim_generation": entry.claim_generation,
        }),
    )
    .await;
    refresh_run_log_outbox_status(state, &entry.run_id).await;
    Ok(true)
}

/// A cancellation may arrive after the worker crossed the remote-call boundary but before an
/// error/timeout is handled. Such an entry must never be returned to `pending`: the next claim
/// would clear `cancel_requested` and could replay a delivery that actually succeeded remotely.
async fn settle_late_cancel_as_delivery_unknown(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
) -> AppResult<bool> {
    let Some(mut filter) = active_claim_filter(entry_id, entry) else {
        return Ok(false);
    };
    filter.insert("cancel_requested", true);
    filter.insert("send_started_at", doc! { "$ne": null });
    let now = DateTime::now();
    let reason = "cancel requested after remote send boundary; delivery requires verification";
    let result = state
        .db
        .collection_agent_send_outbox()
        .update_one(
            filter,
            doc! {
                "$set": {
                    "status": OutboxStatus::DeliveryUnknown.as_str(),
                    "last_error": reason,
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                    "claim_token": "",
                    "reclaimed_in_flight": "",
                },
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Ok(false);
    }
    let _ = write_event_with_cap(
        state,
        &entry.workspace_id,
        entry_id,
        &entry.account_id,
        Some(&entry.contact_wxid),
        "outbox_delivery_unknown",
        "warning",
        reason,
        Some(doc! {
            "outbox_id": entry_id,
            "run_id": &entry.run_id,
            "claim_generation": entry.claim_generation,
            "cancel_requested": true,
        }),
    )
    .await;
    refresh_run_log_outbox_status(state, &entry.run_id).await;
    Ok(true)
}

/// **二次安全门**（R13.4）：发送前再次检查 managed / contact cooldown /
/// user stop / 陈旧度（30min）。任一命中 → 返回 `Some(reason)`。
///
/// 豁免仅限收件人不是客户的三类（领导请示/澄清卡、系统事件通知）；
/// `manual_send` 与普通托管发送同受本门约束（撤管即停，admin 确认不豁免
/// 撤管竞态——与 [`check_contact_status_pure`] 的语义定案一致）。
//
// NOTE: 暴露为 `pub` 仅供 `tests/outbox_integration.rs`（W4 / Task 5.8 / R13.10）
// 直接驱动，不应在生产代码中绕过 `process_entry` 单独调用。
pub async fn second_safety_gate(
    state: &AppState,
    entry: &OutboxEntry,
) -> AppResult<Option<String>> {
    if matches!(
        entry.source_kind.as_str(),
        SOURCE_KIND_PRINCIPAL_ESCALATION
            | SOURCE_KIND_PRINCIPAL_CLARIFICATION
            | SOURCE_KIND_SYSTEM_INCIDENT
    ) {
        return Ok(None);
    }
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

    let is_managed = contact
        .as_ref()
        .map_or(false, |c| c.agent_status == AgentStatus::Managed);
    // 名实注意：变量名沿用纯函数形参 `decision_created_ms`，但实际取的是
    // **outbox entry** 的 created_at（decision 产出 → enqueue 通常毫秒级，作
    // decision 创建时刻的近似）；user-stop 判定在极窄窗口下以 entry 时刻为准。
    let decision_created_ms = entry.created_at.timestamp_millis();
    Ok(check_second_safety_gate_pure(
        now.timestamp_millis(),
        entry.created_at.timestamp_millis(),
        cooldown_until_ms,
        last_inbound_ms,
        &outcome,
        decision_created_ms,
        STALE_THRESHOLD_MILLIS,
        is_managed,
    ))
}

fn aggregate_run_outbox_status<'a>(
    statuses: impl IntoIterator<Item = &'a str>,
) -> Option<&'static str> {
    let statuses: Vec<&str> = statuses.into_iter().collect();
    if statuses.is_empty() {
        return None;
    }
    if statuses
        .iter()
        .any(|status| *status == OutboxStatus::InFlight.as_str())
    {
        return Some(OutboxStatus::InFlight.as_str());
    }
    if statuses
        .iter()
        .any(|status| *status == OutboxStatus::Pending.as_str())
    {
        return Some(OutboxStatus::Pending.as_str());
    }

    let sent = statuses
        .iter()
        .filter(|status| **status == OutboxStatus::Sent.as_str())
        .count();
    if sent == statuses.len() {
        return Some(OutboxStatus::Sent.as_str());
    }
    if sent > 0 {
        return Some("partially_sent");
    }
    if statuses
        .iter()
        .any(|status| *status == OutboxStatus::FailedTerminal.as_str())
    {
        return Some(OutboxStatus::FailedTerminal.as_str());
    }
    if statuses
        .iter()
        .any(|status| *status == OutboxStatus::DeliveryUnknown.as_str())
    {
        return Some(OutboxStatus::DeliveryUnknown.as_str());
    }
    Some(OutboxStatus::Canceled.as_str())
}

fn run_outbox_refresh_write_filter(
    run_id: &str,
    run_status: Option<&str>,
    generation: i64,
) -> Document {
    let mut filter = doc! {
        "run_id": run_id,
        "outbox_refresh_generation": generation,
    };
    if let Some(status) = run_status {
        filter.insert("status", status);
    }
    filter
}

/// 按同一 run 的完整 outbox 集合重算聚合状态。不能让单条 entry 的最后写入者
/// 覆盖 run 级事实：`sent + canceled/failed_terminal` 稳定记为 `partially_sent`。
pub(crate) async fn refresh_run_log_outbox_status(state: &AppState, run_id: &str) {
    if run_id.is_empty() {
        return;
    }
    // 每次刷新先原子领取递增 generation。较早刷新即使在较晚刷新之后才完成查询，
    // 最终 CAS 也会因 generation 不匹配而放弃，避免旧快照把 sent 倒退成 pending。
    let snapshot = match state
        .db
        .agent_run_logs()
        .clone_with_type::<Document>()
        .find_one_and_update(
            doc! { "run_id": run_id },
            doc! { "$inc": { "outbox_refresh_generation": 1i64 } },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await
    {
        Ok(Some(log)) => log,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(
                ?err,
                run_id,
                "reserve run outbox aggregation generation failed"
            );
            return;
        }
    };
    let run_status = snapshot.get_str("status").ok().map(str::to_string);
    let generation = snapshot
        .get_i64("outbox_refresh_generation")
        .or_else(|_| snapshot.get_i32("outbox_refresh_generation").map(i64::from))
        .unwrap_or_default();
    let mut cursor = match state
        .db
        .collection_agent_send_outbox()
        .find(doc! { "run_id": run_id }, None)
        .await
    {
        Ok(cursor) => cursor,
        Err(err) => {
            tracing::warn!(?err, run_id, "query outbox statuses for run failed");
            return;
        }
    };
    let mut statuses = Vec::new();
    loop {
        match cursor.try_next().await {
            Ok(Some(entry)) => statuses.push(entry.status),
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(?err, run_id, "read outbox statuses for run failed");
                return;
            }
        }
    }
    let outbox_status = match run_status.as_deref() {
        Some("outbox_enqueuing") => OutboxStatus::Pending.as_str(),
        Some("outbox_enqueue_partial_failure") => {
            let Some(status) = aggregate_run_outbox_status(statuses.iter().map(String::as_str))
            else {
                return;
            };
            if status == OutboxStatus::Sent.as_str() {
                "partially_sent"
            } else {
                status
            }
        }
        _ => {
            let Some(status) = aggregate_run_outbox_status(statuses.iter().map(String::as_str))
            else {
                return;
            };
            status
        }
    };
    let now = DateTime::now();
    let write_filter = run_outbox_refresh_write_filter(run_id, run_status.as_deref(), generation);
    let res = state
        .db
        .agent_run_logs()
        .update_one(
            write_filter,
            doc! {
                "$set": {
                    "outbox_status": outbox_status,
                    "updated_at": now,
                }
            },
            None,
        )
        .await;
    match res {
        Ok(result) if result.matched_count == 0 => {
            tracing::debug!(
                run_id,
                generation,
                "stale outbox aggregation snapshot skipped"
            );
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(
                ?err,
                run_id,
                outbox_status,
                "update agent_run_logs.outbox_status failed"
            );
        }
    }
}

const DELIVERY_FINALIZE_LEASE_SECONDS: i64 = 60;
const DELIVERY_FINALIZE_RECONCILE_BATCH: i64 = 20;
const OUTBOX_ENQUEUE_RECONCILE_GRACE_SECONDS: i64 = 60;
const OUTBOX_ENQUEUE_RECONCILE_BATCH: i64 = 20;

#[derive(Debug, PartialEq, Eq)]
enum StaleEnqueueReconcileAction {
    Enqueued,
    PartialFailure,
    Failed,
}

fn stale_enqueue_reconcile_action(
    expected_text_segments: i32,
    actual_text_segments: u64,
) -> StaleEnqueueReconcileAction {
    if actual_text_segments == 0 {
        return StaleEnqueueReconcileAction::Failed;
    }
    // 升级前的 review 没有固化段数字段；已有文本条目是唯一可恢复事实。
    if expected_text_segments <= 0 {
        return StaleEnqueueReconcileAction::Enqueued;
    }
    if actual_text_segments >= expected_text_segments as u64 {
        StaleEnqueueReconcileAction::Enqueued
    } else {
        StaleEnqueueReconcileAction::PartialFailure
    }
}

fn stale_enqueue_effective_action(
    review_status: &str,
    expected_text_segments: i32,
    actual_text_segments: u64,
) -> Option<StaleEnqueueReconcileAction> {
    match review_status {
        "outbox_enqueuing" => Some(stale_enqueue_reconcile_action(
            expected_text_segments,
            actual_text_segments,
        )),
        "outbox_enqueued" | "delivery_finalizing" | "sent" => {
            Some(StaleEnqueueReconcileAction::Enqueued)
        }
        "outbox_enqueue_partial_failure" => Some(StaleEnqueueReconcileAction::PartialFailure),
        "outbox_enqueue_failed" => Some(StaleEnqueueReconcileAction::Failed),
        _ => None,
    }
}

fn stale_enqueue_run_update(action: &StaleEnqueueReconcileAction, now: DateTime) -> Document {
    match action {
        StaleEnqueueReconcileAction::Enqueued => doc! { "$set": {
            "status": "outbox_enqueued",
            "lifecycle": super::run_envelope::LIFECYCLE_COMPLETED,
            "updated_at": now,
        } },
        StaleEnqueueReconcileAction::PartialFailure => doc! { "$set": {
            "status": "outbox_enqueue_partial_failure",
            "lifecycle": super::run_envelope::LIFECYCLE_FAILED_AFTER_DECISION,
            "updated_at": now,
        } },
        StaleEnqueueReconcileAction::Failed => doc! { "$set": {
            "status": "outbox_enqueue_failed",
            "lifecycle": super::run_envelope::LIFECYCLE_FAILED_AFTER_DECISION,
            "outbox_status": OutboxStatus::Canceled.as_str(),
            "updated_at": now,
        } },
    }
}

fn stale_enqueue_review_status_compatible(
    action: &StaleEnqueueReconcileAction,
    status: &str,
) -> bool {
    match action {
        StaleEnqueueReconcileAction::Enqueued => matches!(
            status,
            "outbox_enqueuing" | "outbox_enqueued" | "delivery_finalizing" | "sent"
        ),
        StaleEnqueueReconcileAction::PartialFailure => {
            matches!(
                status,
                "outbox_enqueuing" | "outbox_enqueue_partial_failure"
            )
        }
        StaleEnqueueReconcileAction::Failed => {
            matches!(status, "outbox_enqueuing" | "outbox_enqueue_failed")
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DeliveryFinalizeReconcileAction {
    Finalize,
    Wait,
    Clear,
}

fn delivery_finalize_reconcile_action(
    review_status: Option<&str>,
) -> DeliveryFinalizeReconcileAction {
    match review_status {
        Some("outbox_enqueued" | "delivery_finalizing") => {
            DeliveryFinalizeReconcileAction::Finalize
        }
        Some("outbox_enqueuing") => DeliveryFinalizeReconcileAction::Wait,
        Some("sent") | Some(_) | None => DeliveryFinalizeReconcileAction::Clear,
    }
}

async fn clear_delivery_finalize_markers(state: &AppState, decision_id: ObjectId) {
    if let Err(err) = state
        .db
        .collection_agent_send_outbox()
        .update_many(
            doc! { "decision_id": decision_id, "delivery_finalize_pending": true },
            doc! {
                "$set": { "delivery_finalized_at": DateTime::now() },
                "$unset": { "delivery_finalize_pending": "" },
            },
            None,
        )
        .await
    {
        tracing::warn!(?err, %decision_id, "clear delivery finalization markers failed");
    }
}

/// 当一条纯文本 entry 已确认送达后，检查同一 decision 的全部文本分段是否均已送达。
/// review 上的短 lease 保证同一时刻只有一个提交者；outbox 上的 pending marker 让
/// dispatcher 能在进程崩溃后重跑。所有附属写入均幂等，review 仅在它们完成后置 sent。
async fn finalize_delivered_text_decision(
    state: &AppState,
    entry: &OutboxEntry,
    contact: &crate::models::Contact,
) {
    if entry.media_asset_id.is_some() || entry.referral_card_id.is_some() {
        return;
    }
    let Some(decision_id) = entry.decision_id else {
        return;
    };

    let review_snapshot = match state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": decision_id }, None)
        .await
    {
        Ok(Some(review)) => review,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(?err, %decision_id, "load decision review for delivery finalize failed");
            return;
        }
    };
    let is_expired_relay_holding = review_snapshot
        .send_gateway_result
        .get_str("deliveryKind")
        .ok()
        == Some("expired_principal_authorization_holding");
    let decision: Option<super::types::AgentDecision> = if is_expired_relay_holding {
        None
    } else {
        let run_log = match state
            .db
            .agent_run_logs()
            .find_one(doc! { "run_id": &entry.run_id }, None)
            .await
        {
            Ok(Some(log)) => log,
            Ok(None) => return,
            Err(err) => {
                tracing::warn!(?err, run_id = %entry.run_id, "load run log for delivery finalize failed");
                return;
            }
        };
        match mongodb::bson::from_document(run_log.decision.clone()) {
            Ok(decision) => Some(decision),
            Err(err) => {
                tracing::warn!(?err, run_id = %entry.run_id, "decode delivered decision failed");
                return;
            }
        }
    };
    let text_filter = doc! {
        "decision_id": decision_id,
        "media_asset_id": null,
        "referral_card_id": null,
    };
    let text_total = match state
        .db
        .collection_agent_send_outbox()
        .count_documents(text_filter.clone(), None)
        .await
    {
        Ok(count) => count,
        Err(err) => {
            tracing::warn!(?err, %decision_id, "count decision text outbox failed");
            return;
        }
    };
    // 新记录在入队前固化期望段数，配置热更新不会改变本 decision 的完整性口径。
    // 历史记录字段为 0；它们只有在 status=outbox_enqueued 后才会进入 finalizer，
    // 此时已存在的文本条目数就是旧链路可恢复的最佳事实。
    let expected_segments = if review_snapshot.expected_text_segments > 0 {
        review_snapshot.expected_text_segments as u64
    } else {
        text_total
    };
    if expected_segments == 0 {
        return;
    }
    if text_total < expected_segments {
        return;
    }
    let mut unsent_filter = text_filter;
    unsent_filter.insert("status", doc! { "$ne": OutboxStatus::Sent.as_str() });
    match state
        .db
        .collection_agent_send_outbox()
        .count_documents(unsent_filter, None)
        .await
    {
        Ok(0) => {}
        Ok(_) => return,
        Err(err) => {
            tracing::warn!(?err, %decision_id, "count unsent decision text outbox failed");
            return;
        }
    }

    let now = DateTime::now();
    let lock_until =
        DateTime::from_millis(now.timestamp_millis() + DELIVERY_FINALIZE_LEASE_SECONDS * 1000);
    let worker = uuid::Uuid::new_v4().to_string();
    let review = match state
        .db
        .decision_reviews()
        .find_one_and_update(
            doc! {
                "_id": decision_id,
                "$or": [
                    { "status": "outbox_enqueued" },
                    {
                        "status": "delivery_finalizing",
                        "$or": [
                            { "delivery_finalize_locked_until": { "$lt": now } },
                            { "delivery_finalize_locked_until": null },
                            { "delivery_finalize_locked_until": { "$exists": false } },
                        ],
                    },
                ],
            },
            doc! { "$set": {
                "status": "delivery_finalizing",
                "delivery_finalize_worker": &worker,
                "delivery_finalize_locked_until": lock_until,
            } },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await
    {
        Ok(Some(review)) => review,
        Ok(None) => {
            // 上一次 finalizer 可能已完成 review 写回、但在清 outbox marker 前崩溃。
            // 此时副作用已完成，只需清 marker，不能把 sent review 回退到 finalizing。
            if matches!(
                state
                    .db
                    .decision_reviews()
                    .find_one(doc! { "_id": decision_id, "status": "sent" }, None)
                    .await,
                Ok(Some(_))
            ) {
                clear_delivery_finalize_markers(state, decision_id).await;
            }
            return;
        }
        Err(err) => {
            tracing::warn!(?err, %decision_id, "claim delivered decision finalize failed");
            return;
        }
    };

    // Dynamic coverage fields intentionally stay outside AgentDecisionReview so historical
    // constructors remain source-compatible. Read them under the same finalizer lease.
    let coverage_snapshot = match state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": decision_id }, None)
        .await
    {
        Ok(Some(document)) => document,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(?err, %decision_id, "load reply coverage snapshot failed");
            return;
        }
    };

    // 发送已成事实。附属写入失败时保留 finalizing + outbox marker，由后续 tick 重试；
    // 绝不把 outbox 改回 pending，因而不会触发重复发送。
    let side_effect_result: AppResult<()> = async {
        if let Some(decision) = decision.as_ref() {
            if let Some(value) = decision
                .last_commitment
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let mut commitment =
                    crate::models::CommitmentEntry::from_plain_text(value.to_string());
                if let Some(structured) = &decision.commitment {
                    if structured.text.trim() == value {
                        commitment.due_at = super::types::parse_rfc3339_to_bson(&structured.due_at);
                    }
                }
                state
                    .db
                    .contacts()
                    .update_one(
                        doc! { "_id": contact.id, "commitments.text": { "$ne": value } },
                        super::gateway::build_commitment_push_update(&commitment),
                        None,
                    )
                    .await?;
            }

            if let Some(follow_up) = decision
                .follow_up
                .as_ref()
                .filter(|follow_up| follow_up.needed && !follow_up.content.trim().is_empty())
            {
                if let Some(run_at) = super::types::parse_follow_up_run_at(&follow_up.run_at) {
                    let defaults = crate::models::RuntimeParametersTyped::default();
                    let max_pending = review
                        .runtime_parameters_snapshot
                        .get_i64("maxPendingFollowUps")
                        .unwrap_or(defaults.max_pending_follow_ups);
                    let expires_hours = review
                        .runtime_parameters_snapshot
                        .get_i64("followUpExpiresHours")
                        .unwrap_or(defaults.follow_up_expires_hours);
                    let pending_count = state
                        .db
                        .tasks()
                        .count_documents(
                            doc! {
                                "workspace_id": &entry.workspace_id,
                                "account_id": &entry.account_id,
                                "contact_wxid": &entry.contact_wxid,
                                "kind": "follow_up",
                                "status": "pending",
                            },
                            None,
                        )
                        .await?;
                    if pending_count < max_pending.max(0) as u64 {
                        let expires_at = DateTime::from_millis(
                            run_at.timestamp_millis() + expires_hours.max(0) * 60 * 60 * 1000,
                        );
                        let now = DateTime::now();
                        let task = AgentTask {
                            id: None,
                            workspace_id: entry.workspace_id.clone(),
                            account_id: entry.account_id.clone(),
                            contact_wxid: entry.contact_wxid.clone(),
                            kind: "follow_up".to_string(),
                            run_at,
                            expires_at: Some(expires_at),
                            content: follow_up.content.clone(),
                            status: "pending".to_string(),
                            source_decision_id: Some(decision_id),
                            review_required: true,
                            attempt_count: 0,
                            max_attempts: 3,
                            next_retry_at: None,
                            gateway_status: Some("scheduled_after_delivery".to_string()),
                            cancel_reason: None,
                            error: None,
                            claimed_at: None,
                            claim_recovery_count: 0,
                            created_at: now,
                            updated_at: now,
                        };
                        let task_doc = mongodb::bson::to_document(&task)?;
                        state
                            .db
                            .tasks()
                            .update_one(
                                doc! { "_id": decision_id },
                                doc! { "$setOnInsert": task_doc },
                                UpdateOptions::builder().upsert(true).build(),
                            )
                            .await?;
                    }
                } else {
                    tracing::warn!(
                        decision_id = %decision_id,
                        run_id = %entry.run_id,
                        raw_run_at = %follow_up.run_at,
                        "skipping follow-up with missing or invalid run_at"
                    );
                }
            }
        }

        // A passive reply is fulfilled only here, after every text segment is confirmed sent.
        // The frozen decision watermark prevents a later inbound from being accidentally covered.
        match coverage_snapshot.get_str("reply_coverage_kind").ok() {
            Some("manual_reply") => {
                crate::webhooks::settle_manual_reply_obligation(
                    state,
                    &entry.workspace_id,
                    &entry.account_id,
                    &entry.contact_wxid,
                    &entry.run_id,
                    true,
                )
                .await?;
            }
            Some("passive_reply") => {
                if let (Some(task_id), Ok(inbound_id), Ok(inbound_created_at)) = (
                    review.source_task_id,
                    coverage_snapshot.get_object_id("covers_through_inbound_id"),
                    coverage_snapshot.get_datetime("covers_through_inbound_created_at"),
                ) {
                    crate::webhooks::settle_ai_reply_obligation(
                        state,
                        task_id,
                        inbound_id,
                        *inbound_created_at,
                    )
                    .await?;
                }
            }
            _ => {}
        }

        let relay_task = if let Some(source_task_id) = review.source_task_id {
            state
                .db
                .tasks()
                .find_one(
                    doc! {
                        "_id": source_task_id,
                        "kind": "principal_decision_relay",
                    },
                    None,
                )
                .await?
        } else {
            // Compatibility for reviews created before source_task_id was persisted.
            state
                .db
                .tasks()
                .find_one(
                    doc! {
                        "source_decision_id": decision_id,
                        "kind": "principal_decision_relay",
                    },
                    None,
                )
                .await?
        };
        if let Some(relay_task) = relay_task.as_ref() {
            super::escalation::terminalize_principal_relay_for_task(state, relay_task, "delivered")
                .await?;
        }

        crate::models::assert_agent_task_status_valid("sent");
        if let (Some(task_id), Some(task_token)) = (
            review.source_task_id,
            review.source_task_claim_token.as_deref(),
        ) {
            // New protocol: finalize only the task claim that produced this decision.
            state
                .db
                .tasks()
                .update_one(
                    doc! {
                        "_id": task_id,
                        "status": "outbox_enqueued",
                        "claim_token": task_token,
                        "outbox_decision_id": decision_id,
                    },
                    doc! {
                        "$set": {
                            "status": "sent",
                            "gateway_status": "sent",
                            "updated_at": DateTime::now(),
                        },
                        "$unset": { "claimed_at": "" },
                    },
                    None,
                )
                .await?;
        } else {
            // Compatibility for decisions created before source_task_* existed.
            state
                .db
                .tasks()
                .update_many(
                    doc! {
                        "source_decision_id": decision_id,
                        "status": { "$in": ["running", "outbox_enqueued"] },
                    },
                    doc! {
                        "$set": {
                            "status": "sent",
                            "gateway_status": "sent",
                            "updated_at": DateTime::now(),
                        },
                        "$unset": { "claimed_at": "" },
                    },
                    None,
                )
                .await?;
        }
        Ok(())
    }
    .await;

    if let Err(err) = side_effect_result {
        tracing::warn!(?err, %decision_id, "delivered decision side effects failed; will reconcile");
        let _ = state
            .db
            .decision_reviews()
            .update_one(
                doc! {
                    "_id": decision_id,
                    "status": "delivery_finalizing",
                    "delivery_finalize_worker": &worker,
                },
                doc! { "$unset": {
                    "delivery_finalize_worker": "",
                    "delivery_finalize_locked_until": "",
                } },
                None,
            )
            .await;
        return;
    }

    match state
        .db
        .decision_reviews()
        .update_one(
            doc! {
                "_id": decision_id,
                "status": "delivery_finalizing",
                "delivery_finalize_worker": &worker,
            },
            doc! {
                "$set": { "status": "sent" },
                "$unset": {
                    "delivery_finalize_worker": "",
                    "delivery_finalize_locked_until": "",
                },
            },
            None,
        )
        .await
    {
        Ok(result) if result.matched_count == 1 => {
            clear_delivery_finalize_markers(state, decision_id).await;
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(?err, %decision_id, "mark delivered decision sent failed; will reconcile");
        }
    }
}

/// 恢复“outbox 已 sent，但 review/承诺/follow-up 尚未完成”的窗口。
async fn reconcile_delivered_decision_finalizations(state: &AppState) -> AppResult<()> {
    let mut cursor = state
        .db
        .collection_agent_send_outbox()
        .find(
            doc! {
                "status": OutboxStatus::Sent.as_str(),
                "delivery_finalize_pending": true,
                "decision_id": { "$ne": null },
                "media_asset_id": null,
                "referral_card_id": null,
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": 1, "_id": 1 })
                .limit(DELIVERY_FINALIZE_RECONCILE_BATCH)
                .build(),
        )
        .await?;
    while let Some(entry) = cursor.try_next().await? {
        let Some(decision_id) = entry.decision_id else {
            continue;
        };
        let review = state
            .db
            .decision_reviews()
            .find_one(doc! { "_id": decision_id }, None)
            .await?;
        match delivery_finalize_reconcile_action(review.as_ref().map(|item| item.status.as_str())) {
            DeliveryFinalizeReconcileAction::Wait => continue,
            DeliveryFinalizeReconcileAction::Clear => {
                clear_delivery_finalize_markers(state, decision_id).await;
                continue;
            }
            DeliveryFinalizeReconcileAction::Finalize => {}
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
        if let Some(contact) = contact {
            finalize_delivered_text_decision(state, &entry, &contact).await;
        } else {
            clear_delivery_finalize_markers(state, decision_id).await;
        }
    }
    Ok(())
}

async fn reconcile_stale_task_claim(
    state: &AppState,
    review_id: ObjectId,
    workspace_id: &str,
    run_id: &str,
    decision_id: ObjectId,
    reason: &str,
    now: DateTime,
) -> AppResult<()> {
    super::outbox::cancel_for_decision(state, workspace_id, decision_id, "stale_task_claim")
        .await?;
    state
        .db
        .decision_reviews()
        .update_one(
            doc! { "_id": review_id },
            doc! { "$set": {
                "status": "stale_task_claim",
                "enqueue_reconciled_at": now,
                "task_claim_fence_reason": reason,
            } },
            None,
        )
        .await?;
    let committed = state
        .db
        .agent_run_logs()
        .update_one(
            doc! { "run_id": run_id, "status": "outbox_enqueuing" },
            doc! { "$set": {
                "status": "stale_task_claim",
                "lifecycle": super::run_envelope::LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL,
                "abort_reason": "stale_task_claim",
                "outbox_status": OutboxStatus::Canceled.as_str(),
                "updated_at": now,
            } },
            None,
        )
        .await?;
    if committed.matched_count == 1 {
        refresh_run_log_outbox_status(state, run_id).await;
    }
    tracing::warn!(%decision_id, run_id, reason, "stale task claim enqueue canceled");
    Ok(())
}

/// 恢复 gateway 在写入 `outbox_enqueuing` run log 后崩溃的窗口。
///
/// run log 是恢复标记并最后提交：review/task 先做幂等补偿，最后才以
/// `status=outbox_enqueuing` CAS 推进 run。恢复过程任一步再次崩溃时，下一轮仍会
/// 扫到该 run 并继续。若最后一条文本 outbox 仍在宽限期内则继续等待。
async fn reconcile_stale_outbox_enqueues(state: &AppState) -> AppResult<()> {
    let now = DateTime::now();
    let cutoff = DateTime::from_millis(
        now.timestamp_millis() - OUTBOX_ENQUEUE_RECONCILE_GRACE_SECONDS * 1000,
    );
    let mut runs = state
        .db
        .agent_run_logs()
        .find(
            doc! {
                "status": "outbox_enqueuing",
                "created_at": { "$lte": cutoff },
            },
            FindOptions::builder()
                .sort(doc! { "created_at": 1, "_id": 1 })
                .limit(OUTBOX_ENQUEUE_RECONCILE_BATCH)
                .build(),
        )
        .await?;

    while let Some(run_log) = runs.try_next().await? {
        let run_id = run_log.run_id.as_str();
        let Some(review) = state
            .db
            .decision_reviews()
            .find_one(doc! { "run_id": run_id }, None)
            .await?
        else {
            tracing::warn!(run_id, "stale outbox run lacks decision review");
            state
                .db
                .agent_run_logs()
                .update_one(
                    doc! { "run_id": run_id, "status": "outbox_enqueuing" },
                    stale_enqueue_run_update(&StaleEnqueueReconcileAction::Failed, now),
                    None,
                )
                .await?;
            continue;
        };
        let Some(decision_id) = review.id else {
            tracing::warn!(run_id, "stale outbox review lacks _id");
            continue;
        };
        let text_filter = doc! {
            "decision_id": decision_id,
            "media_asset_id": null,
            "referral_card_id": null,
        };
        let actual_text_segments = state
            .db
            .collection_agent_send_outbox()
            .count_documents(text_filter.clone(), None)
            .await?;

        if actual_text_segments > 0 {
            let mut latest = state
                .db
                .collection_agent_send_outbox()
                .find(
                    text_filter,
                    FindOptions::builder()
                        .sort(doc! { "created_at": -1, "_id": -1 })
                        .limit(1)
                        .build(),
                )
                .await?;
            if latest
                .try_next()
                .await?
                .map(|entry| entry.created_at.timestamp_millis() > cutoff.timestamp_millis())
                .unwrap_or(false)
            {
                continue;
            }
        }

        let Some(action) = stale_enqueue_effective_action(
            &review.status,
            review.expected_text_segments,
            actual_text_segments,
        ) else {
            tracing::warn!(
                %decision_id,
                run_id,
                review_status = %review.status,
                "stale outbox run has incompatible review status"
            );
            continue;
        };

        // SR-034：新协议的恢复只能投影已经由 Task CAS 提交的授权，不能从“Outbox
        // 数量看起来完整”反推出授权。running 表示原 owner 尚未提交（或已崩溃、等待
        // lease reclaim），此时保持 run/review 原状；token/decision 失配则撤销旧意图。
        let task_binding = match (
            review.source_task_id,
            review.source_task_claim_token.as_deref(),
        ) {
            (None, None) => None,
            (Some(task_id), Some(token)) if !token.trim().is_empty() => {
                Some((task_id, token.to_string()))
            }
            _ => {
                reconcile_stale_task_claim(
                    state,
                    decision_id,
                    &review.workspace_id,
                    run_id,
                    decision_id,
                    "incomplete source_task binding",
                    now,
                )
                .await?;
                continue;
            }
        };
        if let Some((task_id, task_token)) = task_binding.as_ref() {
            let task = state
                .db
                .tasks()
                .clone_with_type::<Document>()
                .find_one(doc! { "_id": task_id }, None)
                .await?;
            let Some(task) = task else {
                reconcile_stale_task_claim(
                    state,
                    decision_id,
                    &review.workspace_id,
                    run_id,
                    decision_id,
                    "source task missing",
                    now,
                )
                .await?;
                continue;
            };
            let token_matches = task.get_str("claim_token").ok() == Some(task_token.as_str());
            let decision_matches =
                task.get_object_id("outbox_decision_id").ok() == Some(decision_id);
            if !token_matches || !decision_matches {
                reconcile_stale_task_claim(
                    state,
                    decision_id,
                    &review.workspace_id,
                    run_id,
                    decision_id,
                    "source task token or decision binding was replaced",
                    now,
                )
                .await?;
                continue;
            }
            let task_status = task.get_str("status").unwrap_or_default();
            match (&action, task_status) {
                (StaleEnqueueReconcileAction::Enqueued, "running") => {
                    // Never authorize on behalf of a worker. Reclaim/new owner will eventually
                    // replace this token, after which the stale path above cancels these rows.
                    continue;
                }
                (StaleEnqueueReconcileAction::Enqueued, "outbox_enqueued" | "sent") => {
                    state
                        .db
                        .collection_agent_send_outbox()
                        .update_many(
                            doc! { "decision_id": decision_id },
                            doc! { "$set": {
                                "task_send_authorization_token": task_token,
                                "updated_at": now,
                            } },
                            None,
                        )
                        .await?;
                }
                (
                    StaleEnqueueReconcileAction::PartialFailure
                    | StaleEnqueueReconcileAction::Failed,
                    "running" | "outbox_enqueued" | "sent",
                ) => {
                    crate::models::assert_agent_task_status_valid("cancelled");
                    state
                        .db
                        .tasks()
                        .update_one(
                            doc! {
                                "_id": task_id,
                                "status": { "$in": ["running", "outbox_enqueued"] },
                                "claim_token": task_token,
                                "outbox_decision_id": decision_id,
                            },
                            doc! {
                                "$set": {
                                    "status": "cancelled",
                                    "gateway_status": match action {
                                        StaleEnqueueReconcileAction::PartialFailure => "outbox_enqueue_partial_failure",
                                        _ => "outbox_enqueue_failed",
                                    },
                                    "cancel_reason": "outbox enqueue interrupted and reconciled",
                                    "updated_at": now,
                                },
                                "$unset": { "claimed_at": "", "claim_token": "" },
                            },
                            None,
                        )
                        .await?;
                    super::outbox::cancel_for_decision(
                        state,
                        &review.workspace_id,
                        decision_id,
                        "outbox_enqueue_interrupted",
                    )
                    .await?;
                }
                _ => {
                    reconcile_stale_task_claim(
                        state,
                        decision_id,
                        &review.workspace_id,
                        run_id,
                        decision_id,
                        &format!("source task status is not recoverable: {task_status}"),
                        now,
                    )
                    .await?;
                    continue;
                }
            }
        }
        let review_status = match action {
            StaleEnqueueReconcileAction::Enqueued => "outbox_enqueued",
            StaleEnqueueReconcileAction::PartialFailure => "outbox_enqueue_partial_failure",
            StaleEnqueueReconcileAction::Failed => "outbox_enqueue_failed",
        };
        let review_ready = if review.status == "outbox_enqueuing" {
            let result = state
                .db
                .decision_reviews()
                .update_one(
                    doc! { "_id": decision_id, "status": "outbox_enqueuing" },
                    doc! { "$set": {
                        "status": review_status,
                        "enqueue_reconciled_at": now,
                        "actual_text_segments": actual_text_segments as i64,
                    } },
                    None,
                )
                .await?;
            if result.matched_count == 1 {
                true
            } else {
                state
                    .db
                    .decision_reviews()
                    .find_one(doc! { "_id": decision_id }, None)
                    .await?
                    .map(|current| stale_enqueue_review_status_compatible(&action, &current.status))
                    .unwrap_or(false)
            }
        } else {
            stale_enqueue_review_status_compatible(&action, &review.status)
        };
        if !review_ready {
            tracing::warn!(
                %decision_id,
                run_id,
                review_status = %review.status,
                "stale outbox run has incompatible review status"
            );
            continue;
        }

        // Legacy records did not persist source_task_*; retain their old best-effort repair.
        // New-protocol tasks were already handled with exact id/token/decision filters above.
        if task_binding.is_none() {
            match action {
                StaleEnqueueReconcileAction::Enqueued => {
                    crate::models::assert_agent_task_status_valid("outbox_enqueued");
                    state
                        .db
                        .tasks()
                        .update_many(
                            doc! {
                                "source_decision_id": decision_id,
                                "status": { "$in": ["pending", "retry", "running"] },
                            },
                            doc! { "$set": {
                                "status": "outbox_enqueued",
                                "gateway_status": "outbox_enqueued",
                                "updated_at": now,
                            } },
                            None,
                        )
                        .await?;
                }
                StaleEnqueueReconcileAction::PartialFailure
                | StaleEnqueueReconcileAction::Failed => {
                    crate::models::assert_agent_task_status_valid("cancelled");
                    state
                    .db
                    .tasks()
                    .update_many(
                        doc! {
                            "source_decision_id": decision_id,
                            "status": { "$in": ["pending", "retry", "running", "outbox_enqueued"] },
                        },
                        doc! { "$set": {
                            "status": "cancelled",
                            "gateway_status": review_status,
                            "cancel_reason": "outbox enqueue interrupted and reconciled",
                            "updated_at": now,
                        } },
                        None,
                    )
                    .await?;
                }
            }
        }
        // run 是恢复事务的提交标记，必须最后 CAS。此前任一步失败/崩溃都会保留
        // status=outbox_enqueuing，下一轮可继续幂等补偿 review/task。
        let committed = state
            .db
            .agent_run_logs()
            .update_one(
                doc! { "run_id": run_id, "status": "outbox_enqueuing" },
                stale_enqueue_run_update(&action, now),
                None,
            )
            .await?;
        if committed.matched_count == 1 {
            refresh_run_log_outbox_status(state, run_id).await;
        }
        tracing::warn!(
            %decision_id,
            run_id,
            review_status,
            expected_text_segments = review.expected_text_segments,
            actual_text_segments,
            "reconciled stale outbox enqueue"
        );
    }
    Ok(())
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
    let Some(mut filter) = active_claim_filter(entry_id, entry) else {
        return Ok(());
    };
    // Safety-gate cancellation is only truthful before the irreversible remote-call boundary.
    // A stale caller must not overwrite an already-started delivery with `canceled`.
    filter.extend(send_not_started_filter());
    let result = collection
        .update_one(
            filter,
            doc! {
                "$set": {
                    "status": OutboxStatus::Canceled.as_str(),
                    "cancel_reason": reason,
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                    "claim_token": "",
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Ok(());
    }
    let _ = write_event_with_cap(
        state,
        &entry.workspace_id,
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
    refresh_run_log_outbox_status(state, &entry.run_id).await;
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
    if settle_late_cancel_as_delivery_unknown(state, entry_id, entry).await? {
        return Ok(());
    }
    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let next_attempt = entry.attempt.saturating_add(1);
    let max_attempts = effective_max_attempts(entry.max_attempts);
    let Some(mut filter) = active_claim_filter(entry_id, entry) else {
        return Ok(());
    };
    // Cancellation wins over retry/terminal settlement. Without this predicate a cancellation
    // can land after the pre-check above and immediately be converted back to pending.
    filter.insert("cancel_requested", doc! { "$ne": true });

    if next_attempt < max_attempts {
        let jitter01 = fastrand::f64();
        let backoff_seconds = backoff_with_jitter_seeded(next_attempt, jitter01);
        let next_retry = DateTime::from_millis(now.timestamp_millis() + backoff_seconds * 1000);
        let result = collection
            .update_one(
                filter,
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
                        "claim_token": "",
                        "send_started_at": "",
                    }
                },
                None,
            )
            .await?;
        if result.matched_count == 0 {
            let _ = complete_requested_cancel_before_send(state, entry_id, entry).await?;
            let _ = settle_late_cancel_as_delivery_unknown(state, entry_id, entry).await?;
            return Ok(());
        }
        let _ = write_event_with_cap(
            state,
            &entry.workspace_id,
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
        refresh_run_log_outbox_status(state, &entry.run_id).await;
    } else {
        let result = collection
            .update_one(
                filter,
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
                        "claim_token": "",
                        "send_started_at": "",
                    }
                },
                None,
            )
            .await?;
        if result.matched_count == 0 {
            let _ = complete_requested_cancel_before_send(state, entry_id, entry).await?;
            let _ = settle_late_cancel_as_delivery_unknown(state, entry_id, entry).await?;
            return Ok(());
        }
        let _ = write_event_with_cap(
            state,
            &entry.workspace_id,
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
        refresh_run_log_outbox_status(state, &entry.run_id).await;
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
    let Some(mut filter) = active_claim_filter(entry_id, entry) else {
        return Ok(());
    };
    filter.insert("cancel_requested", doc! { "$ne": true });
    let result = collection
        .update_one(
            filter,
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
                    "claim_token": "",
                    "send_started_at": "",
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        let _ = complete_requested_cancel_before_send(state, entry_id, entry).await?;
        return Ok(());
    }
    let _ = write_event_with_cap(
        state,
        &entry.workspace_id,
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
    refresh_run_log_outbox_status(state, &entry.run_id).await;
    Ok(())
}

/// 查某 workspace 内某账号 `agent_send_outbox` 中 `status=sent` 的最大 `sent_at`。
/// 无 sent 历史返回 None。靠 (workspace_id,account_id,status,sent_at:-1) 索引取 limit(1)。
async fn account_last_sent_at_ms(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Option<i64>> {
    use mongodb::options::FindOneOptions;
    let collection = state.db.collection_agent_send_outbox();
    let opts = FindOneOptions::builder()
        .sort(doc! { "sent_at": -1 })
        .build();
    let doc = collection
        .find_one(account_last_sent_filter(workspace_id, account_id), opts)
        .await?;
    Ok(doc.and_then(|e| e.sent_at).map(|d| d.timestamp_millis()))
}

fn account_last_sent_filter(workspace_id: &str, account_id: &str) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "status": OutboxStatus::Sent.as_str(),
    }
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
    let Some(mut filter) = active_claim_filter(entry_id, entry) else {
        return Ok(());
    };
    filter.insert("cancel_requested", doc! { "$ne": true });
    let result = collection
        .update_one(
            filter,
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
                    "claim_token": "",
                    "send_started_at": "",
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        let _ = complete_requested_cancel_before_send(state, entry_id, entry).await?;
        return Ok(());
    }
    let _ = write_event_with_cap(
        state,
        &entry.workspace_id,
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
    refresh_run_log_outbox_status(state, &entry.run_id).await;
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
    workspace_id: &str,
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
            mcp_success_filter(workspace_id, account_id, contact_wxid, content, lower_bound),
            None,
        )
        .await?;
    Ok(count > 0)
}

fn mcp_success_filter(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    content: &str,
    lower_bound: DateTime,
) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "tool_name": "message_send_text",
        "request.recipient": contact_wxid,
        "request.content": content,
        "error": null,
        "$or": [
            { "response.ok": true },
            {
                "response.ok": { "$exists": false },
                "response.newMsgId": { "$type": "string", "$ne": "" },
            },
        ],
        "created_at": { "$gte": lower_bound },
    }
}

/// post-hoc 防重发核对。缺少成功证据不等于确认未送达：只有权威查询明确未命中，
/// 或发送前置条件能证明客户投递尚未发生，才返回 `NotDelivered`。
///
/// 供 `process_entry` 的两个窗口复用——崩溃恢复（reclaim）与发送 timeout——
/// 消除历史上两分支 text 路的不对称（F-01）：
/// - `referral_card` 条目：没有权威查询 API，恒为 `Inconclusive`；
/// - `media_asset` 条目：content 为空、tool 为 message_send_*，text 版核对查不到
///   → 改用 media_id 定位该素材的成功发送记录。
/// - 纯文本条目：**先查权威 `chat_search_outbound`**（MCP server 真实已发记录，
///   同步落库、不受本地 timeout 取消 mcp_call_logs 写入的影响），带
///   `CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS` 独立短超时；chat_search 出错 / 超时才
///   回落本地 `mcp_already_succeeded`（不因权威通道抖动而倒退成"必重发"）。
async fn verify_delivery(
    state: &AppState,
    entry: &OutboxEntry,
) -> AppResult<super::types::DeliveryVerification> {
    use super::types::DeliveryVerification;
    if entry.referral_card_id.is_some() {
        Ok(DeliveryVerification::Inconclusive)
    } else if let Some(asset_id) = entry.media_asset_id.as_deref() {
        super::media_send::media_delivery_verification(
            state,
            &entry.workspace_id,
            &entry.account_id,
            &entry.contact_wxid,
            asset_id,
            entry.created_at,
        )
        .await
    } else {
        match tokio::time::timeout(
            Duration::from_secs(CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS),
            crate::mcp::chat_search_outbound(
                state,
                &entry.workspace_id,
                &entry.account_id,
                &entry.contact_wxid,
                &entry.content,
                entry.created_at,
            ),
        )
        .await
        {
            Ok(Ok(true)) => Ok(DeliveryVerification::Delivered),
            // chat_search 是权威远端记录；明确空结果才可证明未送达。
            Ok(Ok(false)) => Ok(DeliveryVerification::NotDelivered),
            // 权威通道出错 / 超时 → 本地日志只能证明成功，不能以“无日志”证明失败。
            Ok(Err(_)) | Err(_) => {
                let hit = mcp_already_succeeded(
                    state,
                    &entry.workspace_id,
                    &entry.account_id,
                    &entry.contact_wxid,
                    &entry.content,
                    entry.created_at,
                )
                .await?;
                Ok(if hit {
                    DeliveryVerification::Delivered
                } else {
                    DeliveryVerification::Inconclusive
                })
            }
        }
    }
}

/// 将事后核验命中的既成送达事实收敛为 `sent`，并执行与正常成功相同的审计、
/// decision finalize 和素材/名片台账副作用。返回 false 表示 claim 已不属于本 worker。
async fn commit_verified_delivery(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    contact: Option<&crate::models::Contact>,
    note: &str,
    reason: &str,
) -> AppResult<bool> {
    if !commit_sent_if_owned(state, entry_id, entry, Some(note)).await? {
        return Ok(false);
    }
    let delivered_at = DateTime::now();
    let _ = write_event_with_cap(
        state,
        &entry.workspace_id,
        entry_id,
        &entry.account_id,
        Some(&entry.contact_wxid),
        "outbox_sent_post_hoc",
        "warn",
        note,
        Some(doc! {
            "outbox_id": entry_id,
            "run_id": &entry.run_id,
            "attempt": entry.attempt + 1,
            "reason": reason,
        }),
    )
    .await;
    refresh_run_log_outbox_status(state, &entry.run_id).await;
    if let Some(contact) = contact {
        finalize_delivered_text_decision(state, entry, contact).await;
        super::send_ledger::record_send_for_entry(state, entry, contact, delivered_at).await;
    }
    Ok(true)
}

async fn settle_ambiguous_send(
    state: &AppState,
    entry_id: ObjectId,
    entry: &OutboxEntry,
    contact: Option<&crate::models::Contact>,
    failure_reason: &str,
) -> AppResult<()> {
    use super::types::DeliveryVerification;
    match verify_delivery(state, entry).await {
        Ok(DeliveryVerification::Delivered) => {
            let note = format!(
                "{failure_reason}; delivery confirmed post-hoc via authoritative/local evidence"
            );
            if !commit_verified_delivery(state, entry_id, entry, contact, &note, "ambiguous_send")
                .await?
            {
                tracing::warn!(
                    outbox_id = %entry_id,
                    claim_generation = entry.claim_generation,
                    "post-hoc delivery confirmed but worker no longer owns claim"
                );
            }
        }
        Ok(DeliveryVerification::NotDelivered) => {
            schedule_retry_or_terminal(state, entry_id, entry, failure_reason).await?;
        }
        Ok(DeliveryVerification::Inconclusive) => {
            let reason = format!(
                "{failure_reason}; delivery verification inconclusive; automatic replay disabled"
            );
            let _ = mark_delivery_unknown_if_owned(state, entry_id, entry, &reason).await?;
        }
        Err(err) => {
            let reason = format!(
                "{failure_reason}; delivery verification failed: {err}; automatic replay disabled"
            );
            let _ = mark_delivery_unknown_if_owned(state, entry_id, entry, &reason).await?;
        }
    }
    Ok(())
}

/// P1-6：发送前 contact 状态门——纯函数，便于单测。
///
/// 入队后到 dispatcher 抢占之间 contact 可能被运营改成 `normal`（撤管）或
/// `paused`，此时 in-flight MCP 不应继续把消息送出去（违反"撤管即停"语义）。
///
/// 语义定案（保守方向）：`manual_send`（admin UI 主动发）与普通托管发送**同受**
/// 撤管即停约束——admin 确认的是"发这条消息"，不豁免其后发生的撤管竞态；撤管时
/// 宁可取消已确认的发送。上游 [`second_safety_gate`] 本就对 manual_send 判
/// `not_managed_at_send`，本门作为其后第二次 fresh 读的复核点保持同一判定，
/// 两读之间的状态翻转也收敛到 cancel。
/// 仅豁免收件人不是客户的三类：领导请示/澄清卡与系统事件通知。
///
/// 返回 `Some(reason)` 表示应当 cancel；`None` 表示放行。
pub(crate) fn check_contact_status_pure(
    source_kind: &str,
    agent_status: &AgentStatus,
) -> Option<&'static str> {
    if matches!(
        source_kind,
        SOURCE_KIND_PRINCIPAL_ESCALATION
            | SOURCE_KIND_PRINCIPAL_CLARIFICATION
            | SOURCE_KIND_SYSTEM_INCIDENT
    ) {
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

    if !principal_card_send_is_authorized(state, entry_id, entry).await? {
        cancel_entry(
            state,
            entry_id,
            entry,
            "principal_escalation_generation_no_longer_authorized",
        )
        .await?;
        return Ok(());
    }
    if !super::system_incident::send_is_authorized(state, entry).await? {
        cancel_entry(
            state,
            entry_id,
            entry,
            "system_incident_generation_no_longer_authorized",
        )
        .await?;
        return Ok(());
    }

    // SR-034 第一检查点：正常 Gateway 可能刚写入首段、尚未提交 task 授权；此时无损
    // defer。若 token/decision 已被新 owner 替换，则在任何业务查询和 MCP 前取消。
    if enforce_task_send_authorization(state, entry_id, entry)
        .await?
        .is_none()
    {
        return Ok(());
    }

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
    if contact.is_none()
        && !matches!(
            entry.source_kind.as_str(),
            SOURCE_KIND_PRINCIPAL_ESCALATION
                | SOURCE_KIND_PRINCIPAL_CLARIFICATION
                | SOURCE_KIND_SYSTEM_INCIDENT
        )
    {
        schedule_retry_or_terminal(state, entry_id, entry, "contact not found at dispatch time")
            .await?;
        return Ok(());
    }

    if let Some(contact) = contact.as_ref() {
        if let Some(reason) = check_contact_status_pure(&entry.source_kind, &contact.agent_status) {
            cancel_entry(state, entry_id, entry, reason).await?;
            return Ok(());
        }
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

    // 崩溃恢复幂等门：本条曾被 reclaim（上一个 worker 抢占后在写 sent 前消失）。
    // 只有权威核验明确未送达才继续本次发送；已送达则收敛为 sent，无法确认则进入
    // delivery_unknown，绝不把“没有本地成功日志”当作“确认未送达”。
    if entry.reclaimed_in_flight {
        use super::types::DeliveryVerification;
        match verify_delivery(state, entry).await {
            Ok(DeliveryVerification::Delivered) => {
                let note = "reclaimed after crash but delivery was confirmed post-hoc";
                if !commit_verified_delivery(
                    state,
                    entry_id,
                    entry,
                    contact.as_ref(),
                    note,
                    "crash_reclaim",
                )
                .await?
                {
                    tracing::warn!(
                        outbox_id = %entry_id,
                        claim_generation = entry.claim_generation,
                        "reclaim verification confirmed delivery but worker no longer owns claim"
                    );
                }
                return Ok(());
            }
            Ok(DeliveryVerification::NotDelivered) => {}
            Ok(DeliveryVerification::Inconclusive) => {
                let _ = mark_delivery_unknown_if_owned(
                    state,
                    entry_id,
                    entry,
                    "worker was reclaimed after the remote boundary and delivery cannot be verified; automatic replay disabled",
                )
                .await?;
                return Ok(());
            }
            Err(err) => {
                let reason = format!(
                    "worker was reclaimed and delivery verification failed: {err}; automatic replay disabled"
                );
                let _ = mark_delivery_unknown_if_owned(state, entry_id, entry, &reason).await?;
                return Ok(());
            }
        }
    }

    // 账号级最小发送间隔闸：查该账号上次实发时刻，距今 < 随机间隔则 reschedule。
    // 防"连珠炮"——单 worker 串行 for 循环里跨客户/多段消息背靠背零间隔发出 = 机器特征。
    // 位置在 reclaim 幂等门之后（不误拦本该 post-hoc 标 sent 的条目）、发送之前。
    // 查询失败 fail-soft 放行（宁可漏限一次也不丢消息）。
    // S5-4：间隔按本段字符数加权打字时间（长段比短句慢几拍，见 pacing.rs 常量）。
    if let Ok(Some(last_sent_ms)) =
        account_last_sent_at_ms(state, &entry.workspace_id, &entry.account_id).await
    {
        let interval_ms = super::pacing::account_send_interval_ms(
            fastrand::f64(),
            state.config.account_send_min_interval_ms,
            state.config.account_send_max_interval_ms,
            entry.content.chars().count(),
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

    // SR-034 第二检查点：覆盖第一检查后到远端边界前发生的 task 状态变化。已提交的
    // outbox_enqueued 授权对 task worker/reclaimer 是稳定终态；失权/取消则在这里停下。
    let Some(task_authorization_token) =
        enforce_task_send_authorization(state, entry_id, entry).await?
    else {
        return Ok(());
    };

    if !principal_card_send_is_authorized(state, entry_id, entry).await? {
        cancel_entry(
            state,
            entry_id,
            entry,
            "principal_escalation_generation_no_longer_authorized",
        )
        .await?;
        return Ok(());
    }
    if !super::system_incident::send_is_authorized(state, entry).await? {
        cancel_entry(
            state,
            entry_id,
            entry,
            "system_incident_generation_no_longer_authorized",
        )
        .await?;
        return Ok(());
    }

    // Last cancellable point. The CAS fails when an admin/user cancellation request won, the
    // lease was reclaimed, or another owner replaced this claim. In every case this worker must
    // stop before invoking MCP.
    if !begin_remote_send(state, entry_id, entry, task_authorization_token.as_deref()).await? {
        return Ok(());
    }

    let send_fut = async {
        if matches!(
            entry.source_kind.as_str(),
            SOURCE_KIND_PRINCIPAL_ESCALATION
                | SOURCE_KIND_PRINCIPAL_CLARIFICATION
                | SOURCE_KIND_SYSTEM_INCIDENT
        ) {
            let response = crate::mcp::logged_send_call_for_account(
                state,
                &entry.workspace_id,
                &entry.account_id,
                "message_send_text",
                serde_json::json!({
                    "recipient": &entry.contact_wxid,
                    "content": &entry.content,
                }),
            )
            .await
            .map_err(super::types::OutboundSendError::from)?;
            match super::gateway::classify_send_receipt(&response) {
                super::gateway::SendReceiptStatus::Succeeded => Ok(response),
                super::gateway::SendReceiptStatus::ExplicitlyFailed => {
                    Err(super::types::OutboundSendError::SafeToRetry(
                        "internal notification returned an explicit negative delivery receipt"
                            .to_string(),
                    ))
                }
                super::gateway::SendReceiptStatus::Inconclusive => {
                    Err(super::types::OutboundSendError::DeliveryUncertain(
                        "internal notification returned an unverifiable delivery receipt"
                            .to_string(),
                    ))
                }
            }
        } else if let Some(card_id) = entry.referral_card_id.as_deref() {
            super::referral::send_outbound_namecard(
                state,
                contact.as_ref().expect("non-system send requires contact"),
                card_id,
            )
            .await
        } else if let Some(asset_id) = entry.media_asset_id.as_deref() {
            super::media_send::send_outbound_media(
                state,
                contact.as_ref().expect("non-system send requires contact"),
                asset_id,
            )
            .await
        } else {
            super::gateway::send_outbound_message(
                state,
                contact.as_ref().expect("non-system send requires contact"),
                &entry.content,
                extra_raw,
            )
            .await
        }
    };
    let send_result =
        tokio::time::timeout(Duration::from_secs(MCP_SEND_TIMEOUT_SECONDS), send_fut).await;

    match send_result {
        Ok(Ok(_)) => {
            if !commit_sent_if_owned(state, entry_id, entry, None).await? {
                tracing::warn!(
                    outbox_id = %entry_id,
                    claim_generation = entry.claim_generation,
                    "MCP reported delivery but this worker no longer owns the claim; suppressing duplicate side effects"
                );
                return Ok(());
            }
            let delivered_at = DateTime::now();
            let _ = write_event_with_cap(
                state,
                &entry.workspace_id,
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
            refresh_run_log_outbox_status(state, &entry.run_id).await;
            if let Some(contact) = contact.as_ref() {
                finalize_delivered_text_decision(state, entry, contact).await;

                // 主动发送台账：素材/名片条目记一条（纯文本不记）。fail-soft，不影响已成发送。
                super::send_ledger::record_send_for_entry(state, entry, contact, delivered_at)
                    .await;
            }
        }
        Ok(Err(super::types::OutboundSendError::SafeToRetry(reason))) => {
            schedule_retry_or_terminal(state, entry_id, entry, &reason).await?;
        }
        Ok(Err(super::types::OutboundSendError::DeliveryUncertain(reason))) => {
            settle_ambiguous_send(state, entry_id, entry, contact.as_ref(), &reason).await?;
        }
        Err(_) => {
            settle_ambiguous_send(
                state,
                entry_id,
                entry,
                contact.as_ref(),
                "send timed out after the remote boundary",
            )
            .await?;
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
    workspace_id: &str,
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
        .count_documents(
            doc! { "workspace_id": workspace_id, "details.outbox_id": outbox_id },
            None,
        )
        .await
        .unwrap_or(0);
    if count < PER_ENTRY_EVENT_CAP as u64 {
        return write_outbox_event(
            state,
            workspace_id,
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
                "workspace_id": workspace_id,
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
                workspace_id,
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
static OUTBOX_CLAIM_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Process-local fast-path wakeup. MongoDB remains the durable source of truth and the periodic
/// poll below remains the cross-process/crash fallback; a notification only removes avoidable
/// sleep after a row has already been durably queued or task-authorized.
static OUTBOX_WORK_NOTIFY: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

pub(crate) fn notify_outbox_work() {
    OUTBOX_WORK_NOTIFY.notify_one();
}

/// Schedule one additional wake without changing durable retry/pacing timestamps. Task-bound
/// rows may have been harmlessly deferred while their authorization CAS was still Building; this
/// second signal picks them up when that one-second guard expires. The periodic poll remains the
/// fallback if the process exits before the timer fires.
pub(crate) fn notify_outbox_work_after(delay: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        OUTBOX_WORK_NOTIFY.notify_one();
    });
}

async fn wait_for_outbox_work(notify: &tokio::sync::Notify, fallback: Duration) {
    tokio::select! {
        _ = notify.notified() => {}
        _ = tokio::time::sleep(fallback) => {}
    }
}

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
        // Notify retains one permit when no waiter is registered, so an enqueue racing between
        // the empty scan and this wait is still observed. The timeout is the durable fallback.
        wait_for_outbox_work(
            &OUTBOX_WORK_NOTIFY,
            Duration::from_secs(poll_interval_seconds),
        )
        .await;
    }
}

/// 单次 tick：reclaim → 循环 claim+process 直到无可抢占或达到 `PER_TICK_PROCESS_CAP`。
async fn tick(state: &AppState, worker: &str, lease_seconds: i32) -> AppResult<()> {
    reclaim_expired_leases(state).await?;
    reconcile_stale_outbox_enqueues(state).await?;
    reconcile_delivered_decision_finalizations(state).await?;
    // Manual sends can be canceled/terminalized outside this worker. Release their pause once
    // durable Outbox truth is known; delivery_unknown deliberately remains paused.
    let _ = crate::webhooks::reconcile_manual_reply_obligations(state).await?;
    const AGING_CLAIM_EVERY: u64 = 10;
    for _ in 0..PER_TICK_PROCESS_CAP {
        let sequence = OUTBOX_CLAIM_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let prefer_oldest = sequence % AGING_CLAIM_EVERY == AGING_CLAIM_EVERY - 1;
        let claimed =
            atomic_claim_pending_with_policy(state, worker, lease_seconds, prefer_oldest).await?;
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
    use crate::agent::run_envelope::SOURCE_KIND_MANUAL_SEND;

    #[tokio::test]
    async fn outbox_notify_wakes_production_wait_without_poll_delay() {
        let notify = tokio::sync::Notify::new();
        // Exercise the race where enqueue happens after a scan but before the worker registers
        // its waiter. Notify must retain the permit.
        notify.notify_one();
        tokio::time::timeout(
            Duration::from_millis(50),
            wait_for_outbox_work(&notify, Duration::from_secs(5)),
        )
        .await
        .expect("durable enqueue notification should wake dispatcher immediately");
    }

    #[tokio::test]
    async fn outbox_production_wait_retains_periodic_fallback() {
        let notify = tokio::sync::Notify::new();
        tokio::time::timeout(
            Duration::from_millis(50),
            wait_for_outbox_work(&notify, Duration::from_millis(5)),
        )
        .await
        .expect("periodic fallback should wake dispatcher without a notification");
    }

    #[test]
    fn ordinary_success_clears_stale_error_but_post_hoc_note_is_preserved() {
        let ordinary = sent_unset_fields(false);
        assert!(ordinary.contains_key("last_error"));

        let post_hoc = sent_unset_fields(true);
        assert!(!post_hoc.contains_key("last_error"));
    }

    #[test]
    fn task_send_authorization_requires_same_claim_decision_and_marker() {
        assert_eq!(
            classify_task_send_authorization(
                "running",
                "task-token",
                Some("task-token"),
                true,
                Some("task-token")
            ),
            TaskSendAuthorization::Building,
            "prepared marker alone must not authorize a still-running task"
        );
        assert_eq!(
            classify_task_send_authorization(
                "outbox_enqueued",
                "task-token",
                Some("task-token"),
                true,
                None
            ),
            TaskSendAuthorization::Building,
            "committed task without its single-document marker must defer"
        );
        assert_eq!(
            classify_task_send_authorization(
                "outbox_enqueued",
                "task-token",
                Some("task-token"),
                true,
                Some("task-token")
            ),
            TaskSendAuthorization::Authorized(Some("task-token".to_string()))
        );
        for stale in [
            classify_task_send_authorization(
                "outbox_enqueued",
                "task-token",
                Some("new-owner-token"),
                true,
                Some("task-token"),
            ),
            classify_task_send_authorization(
                "outbox_enqueued",
                "task-token",
                Some("task-token"),
                false,
                Some("task-token"),
            ),
            classify_task_send_authorization(
                "outbox_enqueued",
                "task-token",
                Some("task-token"),
                true,
                Some("other-marker"),
            ),
            classify_task_send_authorization(
                "cancelled",
                "task-token",
                Some("task-token"),
                true,
                Some("task-token"),
            ),
        ] {
            assert!(matches!(stale, TaskSendAuthorization::Stale(_)));
        }
    }

    #[test]
    fn remote_send_boundary_filter_contains_every_fence() {
        let entry_id = ObjectId::parse_str("64b64c000000000000000034").unwrap();
        assert_eq!(
            remote_send_start_filter(
                entry_id,
                "outbox-worker",
                "outbox-claim",
                Some("task-token")
            ),
            doc! {
                "_id": entry_id,
                "status": "in_flight",
                "worker_id": "outbox-worker",
                "claim_token": "outbox-claim",
                "cancel_requested": { "$ne": true },
                "task_send_authorization_token": "task-token",
                "$or": [
                    { "send_started_at": { "$exists": false } },
                    { "send_started_at": null },
                ],
            }
        );
    }

    #[test]
    fn run_status_aggregation_is_order_independent() {
        let forward = aggregate_run_outbox_status(["sent", "canceled"]);
        let reverse = aggregate_run_outbox_status(["canceled", "sent"]);
        assert_eq!(forward, Some("partially_sent"));
        assert_eq!(reverse, forward);
        assert_eq!(
            aggregate_run_outbox_status(["sent", "failed_terminal"]),
            Some("partially_sent")
        );
        assert_eq!(
            aggregate_run_outbox_status(["sent", "pending"]),
            Some("pending")
        );
        assert_eq!(
            aggregate_run_outbox_status(["canceled", "failed_terminal"]),
            Some("failed_terminal")
        );
    }

    #[test]
    fn run_outbox_refresh_filter_rejects_old_generation_and_status() {
        assert_eq!(
            run_outbox_refresh_write_filter("run-1", Some("outbox_enqueued"), 7),
            doc! {
                "run_id": "run-1",
                "outbox_refresh_generation": 7i64,
                "status": "outbox_enqueued",
            }
        );
    }

    #[test]
    fn stale_enqueue_action_distinguishes_complete_partial_and_empty() {
        assert_eq!(
            stale_enqueue_reconcile_action(2, 2),
            StaleEnqueueReconcileAction::Enqueued
        );
        assert_eq!(
            stale_enqueue_reconcile_action(2, 1),
            StaleEnqueueReconcileAction::PartialFailure
        );
        assert_eq!(
            stale_enqueue_reconcile_action(2, 0),
            StaleEnqueueReconcileAction::Failed
        );
        assert_eq!(
            stale_enqueue_reconcile_action(0, 1),
            StaleEnqueueReconcileAction::Enqueued
        );
    }

    #[test]
    fn stale_enqueue_reentry_preserves_committed_review_outcome() {
        assert_eq!(
            stale_enqueue_effective_action("outbox_enqueued", 2, 0),
            Some(StaleEnqueueReconcileAction::Enqueued)
        );
        assert_eq!(
            stale_enqueue_effective_action("delivery_finalizing", 2, 1),
            Some(StaleEnqueueReconcileAction::Enqueued)
        );
        assert_eq!(
            stale_enqueue_effective_action("sent", 2, 1),
            Some(StaleEnqueueReconcileAction::Enqueued)
        );
        assert_eq!(
            stale_enqueue_effective_action("outbox_enqueue_partial_failure", 2, 2),
            Some(StaleEnqueueReconcileAction::PartialFailure)
        );
        assert_eq!(
            stale_enqueue_effective_action("outbox_enqueue_failed", 2, 2),
            Some(StaleEnqueueReconcileAction::Failed)
        );
        assert_eq!(
            stale_enqueue_effective_action("skipped_duplicate", 2, 2),
            None
        );
    }

    #[test]
    fn stale_enqueue_review_compatibility_rejects_cross_outcome_reentry() {
        assert!(stale_enqueue_review_status_compatible(
            &StaleEnqueueReconcileAction::Enqueued,
            "sent"
        ));
        assert!(stale_enqueue_review_status_compatible(
            &StaleEnqueueReconcileAction::PartialFailure,
            "outbox_enqueue_partial_failure"
        ));
        assert!(stale_enqueue_review_status_compatible(
            &StaleEnqueueReconcileAction::Failed,
            "outbox_enqueue_failed"
        ));
        assert!(!stale_enqueue_review_status_compatible(
            &StaleEnqueueReconcileAction::Enqueued,
            "outbox_enqueue_failed"
        ));
        assert!(!stale_enqueue_review_status_compatible(
            &StaleEnqueueReconcileAction::Failed,
            "sent"
        ));
    }

    #[test]
    fn delivery_finalize_marker_action_handles_process_and_terminal_states() {
        assert_eq!(
            delivery_finalize_reconcile_action(Some("outbox_enqueued")),
            DeliveryFinalizeReconcileAction::Finalize
        );
        assert_eq!(
            delivery_finalize_reconcile_action(Some("delivery_finalizing")),
            DeliveryFinalizeReconcileAction::Finalize
        );
        assert_eq!(
            delivery_finalize_reconcile_action(Some("outbox_enqueuing")),
            DeliveryFinalizeReconcileAction::Wait
        );
        assert_eq!(
            delivery_finalize_reconcile_action(Some("sent")),
            DeliveryFinalizeReconcileAction::Clear
        );
        assert_eq!(
            delivery_finalize_reconcile_action(Some("outbox_enqueue_partial_failure")),
            DeliveryFinalizeReconcileAction::Clear
        );
        assert_eq!(
            delivery_finalize_reconcile_action(None),
            DeliveryFinalizeReconcileAction::Clear
        );
    }

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

    /// 保守语义定案：manual_send 与普通托管发送同受"撤管即停"约束。撤管竞态下
    /// 宁可取消 admin 已确认的发送，也不把消息发给已退出托管的 contact（二次安全门
    /// 同判 not_managed_at_send，本门是其后的第二读复核）。
    #[test]
    fn contact_status_gate_cancels_manual_send_when_unmanaged() {
        assert_eq!(
            check_contact_status_pure(SOURCE_KIND_MANUAL_SEND, &AgentStatus::Normal),
            Some("contact_status_changed_unmanaged"),
            "manual_send 不豁免撤管竞态"
        );
    }

    /// managed contact 的 manual_send 照常放行（约束只在撤管后生效）。
    #[test]
    fn contact_status_gate_allows_manual_send_when_managed() {
        assert!(
            check_contact_status_pure(SOURCE_KIND_MANUAL_SEND, &AgentStatus::Managed).is_none()
        );
    }

    /// 领导请示/澄清卡与系统事件通知的收件人是幕后决策人而非客户，
    /// 不适用客户 agent_status 门。
    #[test]
    fn contact_status_gate_passthrough_for_principal_kinds() {
        assert!(check_contact_status_pure(
            SOURCE_KIND_PRINCIPAL_ESCALATION,
            &AgentStatus::Normal
        )
        .is_none());
        assert!(check_contact_status_pure(
            SOURCE_KIND_PRINCIPAL_CLARIFICATION,
            &AgentStatus::Normal
        )
        .is_none());
    }

    #[test]
    fn contact_status_gate_passthrough_for_system_incident() {
        assert!(
            check_contact_status_pure(SOURCE_KIND_SYSTEM_INCIDENT, &AgentStatus::Normal).is_none()
        );
    }

    #[test]
    fn pacing_and_delivery_filters_are_workspace_account_scoped() {
        let pacing = account_last_sent_filter("ws-a", "acct-a");
        assert_eq!(pacing.get_str("workspace_id").unwrap(), "ws-a");
        assert_eq!(pacing.get_str("account_id").unwrap(), "acct-a");
        assert_eq!(pacing.get_str("status").unwrap(), "sent");

        let delivery = mcp_success_filter(
            "ws-a",
            "acct-a",
            "wx-a",
            "hello",
            DateTime::from_millis(123),
        );
        assert_eq!(delivery.get_str("workspace_id").unwrap(), "ws-a");
        assert_eq!(delivery.get_str("account_id").unwrap(), "acct-a");
        assert_eq!(delivery.get_str("request.recipient").unwrap(), "wx-a");
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
        assert_eq!(
            effective_max_attempts(0),
            3,
            "max_attempts=0 兜底须为 3(对齐 enqueue outbox.rs:244)"
        );
        assert_eq!(effective_max_attempts(-1), 3, "max_attempts<0 兜底须为 3");
        assert_eq!(effective_max_attempts(1), 1, "max_attempts>0 原样透传");
        assert_eq!(effective_max_attempts(5), 5, "max_attempts>0 原样透传");
    }
}
#[test]
fn principal_card_source_identity_is_strict() {
    let id = ObjectId::new();
    assert_eq!(
        principal_card_source_identity(&format!("principal-card:{}:2", id.to_hex())),
        Some((id, 2))
    );
    assert!(principal_card_source_identity("principal-card:not-an-id:2").is_none());
    assert!(principal_card_source_identity(&format!("principal-card:{}:0", id.to_hex())).is_none());
    assert!(
        principal_card_source_identity(&format!("principal-card:{}:2:extra", id.to_hex()))
            .is_none()
    );
    assert!(principal_card_source_identity("evt-ordinary").is_none());
}

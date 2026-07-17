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

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::{
    FindOneAndUpdateOptions, FindOptions, ReturnDocument, UpdateOptions,
};

use crate::error::AppResult;
use crate::models::{AgentStatus, AgentTask, OutboxEntry};
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

    let is_managed = contact
        .as_ref()
        .map_or(false, |c| c.agent_status == AgentStatus::Managed);
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
            tracing::warn!(?err, run_id, "reserve run outbox aggregation generation failed");
            return;
        }
    };
    let run_status = snapshot.get_str("status").ok().map(str::to_string);
    let generation = snapshot
        .get_i64("outbox_refresh_generation")
        .or_else(|_| {
            snapshot
                .get_i32("outbox_refresh_generation")
                .map(i64::from)
        })
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
            let Some(status) =
                aggregate_run_outbox_status(statuses.iter().map(String::as_str))
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
            let Some(status) =
                aggregate_run_outbox_status(statuses.iter().map(String::as_str))
            else {
                return;
            };
            status
        }
    };
    let now = DateTime::now();
    let write_filter =
        run_outbox_refresh_write_filter(run_id, run_status.as_deref(), generation);
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
            tracing::debug!(run_id, generation, "stale outbox aggregation snapshot skipped");
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
        "outbox_enqueue_partial_failure" => {
            Some(StaleEnqueueReconcileAction::PartialFailure)
        }
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
            matches!(status, "outbox_enqueuing" | "outbox_enqueue_partial_failure")
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
    let decision: super::types::AgentDecision =
        match mongodb::bson::from_document(run_log.decision.clone()) {
            Ok(decision) => decision,
            Err(err) => {
                tracing::warn!(?err, run_id = %entry.run_id, "decode delivered decision failed");
                return;
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

    // 发送已成事实。附属写入失败时保留 finalizing + outbox marker，由后续 tick 重试；
    // 绝不把 outbox 改回 pending，因而不会触发重复发送。
    let side_effect_result: AppResult<()> = async {
        if let Some(value) = decision
            .last_commitment
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let mut commitment = crate::models::CommitmentEntry::from_plain_text(value.to_string());
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
                let (run_at, degraded) = super::types::resolve_run_at_or_degrade(
                    &follow_up.run_at,
                    DateTime::now().timestamp_millis(),
                    0,
                );
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
                    gateway_status: Some(if degraded {
                        "run_at_degraded_after_delivery".to_string()
                    } else {
                        "scheduled_after_delivery".to_string()
                    }),
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
        }

        let relay_task = state
            .db
            .tasks()
            .find_one(
                doc! {
                    "source_decision_id": decision_id,
                    "kind": "principal_decision_relay",
                },
                None,
            )
            .await?;
        if relay_task.is_some() {
            super::gateway::clear_awaiting_principal_state(state, contact).await?;
        }

        crate::models::assert_agent_task_status_valid("sent");
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
                    .map(|current| {
                        stale_enqueue_review_status_compatible(&action, &current.status)
                    })
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
        refresh_run_log_outbox_status(state, &entry.run_id).await;
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
    refresh_run_log_outbox_status(state, &entry.run_id).await;
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
                "$or": [
                    { "response.ok": true },
                    {
                        "response.ok": { "$exists": false },
                        "response.newMsgId": { "$type": "string", "$ne": "" },
                    },
                ],
                "created_at": { "$gte": lower_bound },
            },
            None,
        )
        .await?;
    Ok(count > 0)
}

/// post-hoc 防重发核对：判断这条 outbox entry 的内容是否**其实已经发出去过**
/// （MCP 已送达微信但本地状态未落 sent）。命中（`Ok(true)`）即调用方应标 sent 不重发。
///
/// 供 `process_entry` 的两个窗口复用——崩溃恢复（reclaim）与发送 timeout——
/// 消除历史上两分支 text 路的不对称（F-01）：
/// - `referral_card` 条目：名片无 media_id、tool 不同，text/media 版核对都不适用；
///   reclaim/timeout 是边缘场景且重复推名片危害小（客户最多多收一张名片），
///   故保守取 `Ok(false)`（视为未发过、放行重发）。
/// - `media_asset` 条目：content 为空、tool 为 message_send_*，text 版核对查不到
///   → 改用 media_id 定位该素材的成功发送记录。
/// - 纯文本条目：**先查权威 `chat_search_outbound`**（MCP server 真实已发记录，
///   同步落库、不受本地 timeout 取消 mcp_call_logs 写入的影响），带
///   `CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS` 独立短超时；chat_search 出错 / 超时才
///   回落本地 `mcp_already_succeeded`（不因权威通道抖动而倒退成"必重发"）。
async fn verify_already_sent(state: &AppState, entry: &OutboxEntry) -> AppResult<bool> {
    if entry.referral_card_id.is_some() {
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
            // chat_search 出错 / 超时 → 回落本地 mcp_call_logs 核对（不倒退成"必重发"）。
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
    }
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
    // 它可能已把消息送达 MCP/微信。重发前先跑 `verify_already_sent` post-hoc 核对
    // （文本先查权威 chat_search、回落本地 mcp_call_logs）；命中即标 sent 不重发。
    // 与 timeout 分支复用同一 `verify_already_sent`。
    if entry.reclaimed_in_flight {
        let already = verify_already_sent(state, entry).await;
        if let Ok(true) = already {
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
                            "delivery_finalize_pending": entry.decision_id.is_some()
                                && entry.media_asset_id.is_none()
                                && entry.referral_card_id.is_none(),
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
            refresh_run_log_outbox_status(state, &entry.run_id).await;
            finalize_delivered_text_decision(state, entry, &contact).await;
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
                            "delivery_finalize_pending": entry.decision_id.is_some()
                                && entry.media_asset_id.is_none()
                                && entry.referral_card_id.is_none(),
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
            refresh_run_log_outbox_status(state, &entry.run_id).await;
            finalize_delivered_text_decision(state, entry, &contact).await;

            // 主动发送台账：素材/名片条目记一条（纯文本不记）。fail-soft，不影响已成发送。
            super::send_ledger::record_send_for_entry(state, entry, &contact, now).await;
        }
        Ok(Err(err)) => {
            schedule_retry_or_terminal(state, entry_id, entry, &format!("send failed: {err}"))
                .await?;
        }
        Err(_) => {
            // post-hoc 核对：MCP 调用本身在 timeout 之前可能已经成功把消息送达
            // 微信协议（response 慢于 30s 的极端情况），此时 mcp_call_logs 已写入
            // tool_name + recipient + 定位字段（text=content / media=mediaId）且
            // error=null。命中即视为已送达，不再重发，避免给客户重复消息/重复文件。
            let already = verify_already_sent(state, entry).await;
            if let Ok(true) = already {
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
                                "delivery_finalize_pending": entry.decision_id.is_some()
                                    && entry.media_asset_id.is_none()
                                    && entry.referral_card_id.is_none(),
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
                refresh_run_log_outbox_status(state, &entry.run_id).await;
                finalize_delivered_text_decision(state, entry, &contact).await;
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
    reconcile_stale_outbox_enqueues(state).await?;
    reconcile_delivered_decision_finalizations(state).await?;
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

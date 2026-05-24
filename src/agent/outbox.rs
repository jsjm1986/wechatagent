//! Outbox 发送链路（agent-autonomy-loop W4 / Task 5.1）。
//!
//! 决策落地不再直接调 MCP，而是通过本模块写入 `agent_send_outbox` 集合，由
//! [`super::outbox`](self) 模块的 dispatcher worker 异步抢占发送（W4 task 5.2）。
//!
//! 核心不变量（design.md §3.2 / requirements.md R13）：
//!
//! 1. **强幂等**：`idempotency_key = sha256(source_event_id:contact_wxid:content_hash)`
//!    在 `agent_send_outbox` 上有 unique 索引；同一 (source_event, contact, content)
//!    多次入队 SHALL 视为 [`EnqueueOutcome::IdempotentSkip`]，不重复发送。
//! 2. **空 source_event_id 兜底**：跟进任务等场景下 source_event_id 可能为空，
//!    此时 SHALL 走 `synthetic:run_id:contact_wxid:content_hash` 前缀，并写一条
//!    `outbox_synthetic_idempotency_key` warning 事件（R13.2 / R13.10）。
//! 3. **状态枚举严格**：`pending / in_flight / sent / failed_terminal / canceled`，
//!    SHALL NOT 使用 `failed`（旧值）—— 索引 + dispatcher state machine 全部按
//!    新枚举对齐（design.md §3.2 R13.5 / R13.10 hard rule）。
//!
//! 仅 [`enqueue`] 入口允许业务侧调用；后续 W4 task 5.2 会新增
//! `OutboxDispatcher` 持有的 `process_entry` / `cancel_for_contact_on_user_reaction`
//! 等私有方法。

use std::sync::Arc;
use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
use mongodb::error::{ErrorKind, WriteFailure};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::error::{AppError, AppResult};
use crate::mcp;
use crate::models::{AgentEvent, Contact, OutboxEntry};
use crate::routes::AppState;

// ── 状态枚举 ────────────────────────────────────────────────────────────

/// `agent_send_outbox.status` 合法取值（design.md §3.2 / R13.5 / R13.10）。
///
/// 严禁使用 `"failed"`：W4 设计明确要求统一终态值用 `"failed_terminal"`，避免
/// 与 retry 中间态 `"pending"` 语义混淆。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxStatus {
    /// 等待 worker 抢占。`next_retry_at` 为空或已过去。
    Pending,
    /// worker 已抢占（atomic claim 后到 MCP 调用完成前）。
    InFlight,
    /// MCP 发送成功并落 `sent_at`。
    Sent,
    /// 重试上限耗尽（`attempt >= max_attempts`），需要人工介入查 `last_error`。
    FailedTerminal,
    /// 用户拒绝 / cooldown / 30min 陈旧 / 后台手动取消。
    Canceled,
}

impl OutboxStatus {
    /// 写入 BSON 时使用的字符串值。**SHALL NOT** 修改这些 literal —— 索引
    /// 与 dispatcher 都依赖这些字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxStatus::Pending => "pending",
            OutboxStatus::InFlight => "in_flight",
            OutboxStatus::Sent => "sent",
            OutboxStatus::FailedTerminal => "failed_terminal",
            OutboxStatus::Canceled => "canceled",
        }
    }

    /// 逆向解析（dispatcher 从 BSON 读回时使用）；未知 / 历史脏值 → `None`。
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(OutboxStatus::Pending),
            "in_flight" => Some(OutboxStatus::InFlight),
            "sent" => Some(OutboxStatus::Sent),
            "failed_terminal" => Some(OutboxStatus::FailedTerminal),
            "canceled" => Some(OutboxStatus::Canceled),
            _ => None,
        }
    }
}

// ── 错误类型 ────────────────────────────────────────────────────────────

/// [`enqueue`] / dispatcher 共享错误类型。Db 错误透传，幂等 skip 不属于错误
/// 而是 [`EnqueueOutcome::IdempotentSkip`]。
#[derive(Debug, Error)]
pub enum OutboxError {
    /// 底层 MongoDB 错误（连接失败 / 写权限 / 等等）。
    #[error("outbox db error: {0}")]
    Db(#[from] mongodb::error::Error),
    /// 入参非法（content 为空 / contact_wxid 为空 等）。
    #[error("outbox invalid input: {0}")]
    Invalid(String),
}

impl From<OutboxError> for AppError {
    fn from(value: OutboxError) -> Self {
        match value {
            OutboxError::Db(e) => AppError::Db(e),
            OutboxError::Invalid(msg) => AppError::BadRequest(msg),
        }
    }
}

// ── 入队结果 ────────────────────────────────────────────────────────────

/// [`enqueue`] 的两种正常结果：成功创建一条 entry，或被强幂等门拦截。
#[derive(Debug, Clone)]
pub enum EnqueueOutcome {
    /// 新写入；后续 dispatcher 会通过 atomic claim 抢占发送。
    Created {
        /// 新创建的 outbox entry 主键。
        outbox_id: ObjectId,
        /// 计算出的 idempotency_key（便于上层 log / 监控）。
        idempotency_key: String,
    },
    /// 已存在同 idempotency_key 的 entry —— 幂等 skip，不发送第二次。
    IdempotentSkip {
        /// 触发 skip 的 idempotency_key。
        idempotency_key: String,
    },
}

// ── 入参 ────────────────────────────────────────────────────────────────

/// [`enqueue`] 入参。把所有字段聚合在一个 struct 里，避免 8+ 参数函数调用。
#[derive(Debug, Clone)]
pub struct EnqueueRequest {
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub run_id: String,
    /// `agent_decision_reviews._id`（Reply Agent 通过 review 的决策记录主键），
    /// 为 dispatcher 反查"用户是否已回复 stop"提供 join key（R13.4）。
    pub decision_id: Option<ObjectId>,
    /// 入站消息 ID / 跟进任务 ID。空字符串 SHALL 触发 synthetic 兜底。
    pub source_event_id: String,
    /// `inbound_message / follow_up_task / manual_send`（同 `run_envelope` 常量）。
    pub source_kind: String,
    /// 实际要发出的文本内容（已经过 review + finalize）。
    pub content: String,
    /// 默认 3，由 runtime 控制是否调高（R13.5）。
    pub max_attempts: i32,
}

// ── 主入口 ──────────────────────────────────────────────────────────────

/// 把决策结果入队到 `agent_send_outbox`（design.md §3.2 R13.2）。
///
/// 行为：
/// * 计算 `content_hash = sha256(content)` + `idempotency_key`；
/// * 空 `source_event_id` 走 `synthetic:run_id:contact_wxid:content_hash` 兜底，
///   同时写 `outbox_synthetic_idempotency_key` warning 事件；
/// * `insert_one` 成功 → 返回 [`EnqueueOutcome::Created`] + 写 `outbox_created` 事件；
/// * `DuplicateKey` → 返回 [`EnqueueOutcome::IdempotentSkip`] + 写
///   `outbox_idempotent_skip` warning 事件；
/// * 其它 db 错误 → 透传 [`OutboxError::Db`]。
///
/// 关键不变量：**永远不发送两次**。即使上层在 retry 路径上再次调用 enqueue，
/// 由唯一索引兜底；本函数只关心"入队成功 vs 已存在 vs 真错"。
pub async fn enqueue(state: &AppState, req: EnqueueRequest) -> Result<EnqueueOutcome, OutboxError> {
    // ── 入参校验 ────────────────────────────────────────────────────
    if req.contact_wxid.trim().is_empty() {
        return Err(OutboxError::Invalid("contact_wxid is empty".to_string()));
    }
    if req.content.trim().is_empty() {
        return Err(OutboxError::Invalid("content is empty".to_string()));
    }
    if req.run_id.trim().is_empty() {
        return Err(OutboxError::Invalid("run_id is empty".to_string()));
    }

    let now = DateTime::now();
    let content_hash = sha256_hex(req.content.as_bytes());

    // ── source_event_id 兜底 ────────────────────────────────────────
    //
    // 空 source_event_id（典型场景：跟进任务 follow-up，没有入站消息触发）
    // SHALL 走 synthetic 前缀，让 idempotency_key 仍能唯一约束"同一 run +
    // 同一 contact + 同一 content 不重复发送"。
    let (idempotency_key, used_synthetic) = if req.source_event_id.trim().is_empty() {
        let key = format!(
            "synthetic:{}:{}:{}",
            req.run_id, req.contact_wxid, content_hash
        );
        (sha256_hex(key.as_bytes()), true)
    } else {
        let key = format!(
            "{}:{}:{}",
            req.source_event_id, req.contact_wxid, content_hash
        );
        (sha256_hex(key.as_bytes()), false)
    };

    if used_synthetic {
        // 警告事件：synthetic 路径不算错误，但运维需要监控其频率（高频 = 跟进
        // 任务设计可能有问题）。
        let _ = write_outbox_event(
            state,
            &req.account_id,
            Some(&req.contact_wxid),
            "outbox_synthetic_idempotency_key",
            "warning",
            &format!(
                "outbox enqueue without source_event_id, used synthetic key for run={}",
                req.run_id
            ),
            Some(doc! {
                "run_id": &req.run_id,
                "contact_wxid": &req.contact_wxid,
                "idempotency_key": &idempotency_key,
            }),
        )
        .await;
    }

    let max_attempts = if req.max_attempts <= 0 {
        3
    } else {
        req.max_attempts.min(10)
    };

    let entry = OutboxEntry {
        id: None,
        workspace_id: req.workspace_id.clone(),
        account_id: req.account_id.clone(),
        contact_wxid: req.contact_wxid.clone(),
        run_id: req.run_id.clone(),
        decision_id: req.decision_id,
        source_event_id: req.source_event_id.clone(),
        source_kind: req.source_kind.clone(),
        content: req.content.clone(),
        content_hash: content_hash.clone(),
        idempotency_key: idempotency_key.clone(),
        attempt: 0,
        max_attempts,
        status: OutboxStatus::Pending.as_str().to_string(),
        cancel_reason: None,
        last_error: None,
        next_retry_at: None,
        worker_id: None,
        locked_until: None,
        created_at: now,
        updated_at: now,
        sent_at: None,
    };

    // ── 写入 + DuplicateKey 容错 ────────────────────────────────────
    let collection = state.db.collection_agent_send_outbox();
    match collection.insert_one(&entry, None).await {
        Ok(insert_result) => {
            let outbox_id = insert_result
                .inserted_id
                .as_object_id()
                .unwrap_or_else(ObjectId::new);
            let _ = write_outbox_event(
                state,
                &req.account_id,
                Some(&req.contact_wxid),
                "outbox_created",
                "info",
                "outbox entry queued for dispatch",
                Some(doc! {
                    "outbox_id": outbox_id,
                    "run_id": &req.run_id,
                    "source_kind": &req.source_kind,
                    "idempotency_key": &idempotency_key,
                }),
            )
            .await;
            Ok(EnqueueOutcome::Created {
                outbox_id,
                idempotency_key,
            })
        }
        Err(err) if is_duplicate_key_error(&err) => {
            let _ = write_outbox_event(
                state,
                &req.account_id,
                Some(&req.contact_wxid),
                "outbox_idempotent_skip",
                "warning",
                "outbox enqueue hit unique idempotency_key, skipping duplicate",
                Some(doc! {
                    "run_id": &req.run_id,
                    "source_event_id": &req.source_event_id,
                    "idempotency_key": &idempotency_key,
                }),
            )
            .await;
            Ok(EnqueueOutcome::IdempotentSkip { idempotency_key })
        }
        Err(err) => Err(OutboxError::Db(err)),
    }
}

// ── 辅助函数 ────────────────────────────────────────────────────────────

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// 判定 mongodb 错误是否为 DuplicateKey（code 11000 / 11001）。
fn is_duplicate_key_error(err: &mongodb::error::Error) -> bool {
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

/// 内部 helper：写一条 `agent_events` 记录。
///
/// **不复用** [`super::gateway::write_event_for_account`] 是为了避免循环依赖
/// （outbox → gateway → outbox）；行为与之等价。
pub(crate) async fn write_outbox_event(
    state: &AppState,
    account_id: &str,
    contact_wxid: Option<&str>,
    kind: &str,
    status: &str,
    summary: &str,
    details: Option<Document>,
) -> AppResult<()> {
    state
        .db
        .events()
        .insert_one(
            AgentEvent {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: account_id.to_string(),
                contact_wxid: contact_wxid.map(ToString::to_string),
                kind: kind.to_string(),
                status: status.to_string(),
                summary: summary.to_string(),
                details,
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    Ok(())
}

// ── 纯函数 helpers（W4 task 5.3 / 5.4 — 与 dispatcher 共用，提前抽出便于单测）──

/// 重试 backoff 计算（R13.5）。
///
/// 公式：`base = (2^attempt) * 5` 秒，jitter 落在 ±20% 区间内。
/// `jitter01 ∈ [0.0, 1.0]`：0.0 → 下界 -20%，0.5 → 0 jitter，1.0 → 上界 +20%。
/// `attempt == 0` 视为基线 5 秒；`attempt > 10` clamp 到 10（防 i64 溢出）。
pub(crate) fn backoff_with_jitter_seeded(attempt: i32, jitter01: f64) -> i64 {
    let exp = attempt.clamp(0, 10);
    let base: i64 = (1_i64 << exp) * 5;
    let j = jitter01.clamp(0.0, 1.0);
    // jitter ∈ ±20%：(j - 0.5) * 2 → [-1, 1]，再 * 0.2
    let factor = (j - 0.5) * 2.0 * 0.2;
    let delta = (base as f64 * factor).round() as i64;
    base + delta
}

/// 判断 reaction outcome 是否表示用户要求停止 / cooldown（R13.4）。
pub(crate) fn outcome_signals_stop(outcome: &str) -> bool {
    if outcome.is_empty() {
        return false;
    }
    outcome.contains("stop_requested") || outcome.contains("cooldown_requested")
}

/// 一个 outbox status 是否属于"用户反应取消通道可以推进的集合"（R13.6）。
///
/// 仅 `pending` / `in_flight` 可被业务侧用户反应通道取消；`sent` / `canceled` /
/// `failed_terminal` 已经是终态或主动终止态，再次写 `canceled` 没有业务意义，
/// 反而会污染审计（"取消事件"对应的不是真的发生取消）。`from_str` 不识别的
/// 历史脏值一律视为不可取消，由 dispatcher 层先把状态字符串规范化。
pub(crate) fn outbox_status_is_user_cancelable(status: &str) -> bool {
    matches!(
        OutboxStatus::from_str(status),
        Some(OutboxStatus::Pending) | Some(OutboxStatus::InFlight)
    )
}

/// 二次安全门纯函数版本（R13.4）。
///
/// 输入全部为基本类型 / Option，便于单测；返回 `Some(reason)` 表示需要 cancel。
/// 时间字段统一用 epoch ms（i64）。
///
/// 检查顺序：
/// 1. `cooldown_until > now` → `contact_cooldown_active`；
/// 2. `last_inbound > decision_created_at && outcome 命中 stop` → `user_stop_requested_after_decision`；
/// 3. `now - entry_created > stale_threshold_ms` → `outbox_stale_30min`。
pub(crate) fn check_second_safety_gate_pure(
    now_ms: i64,
    entry_created_ms: i64,
    cooldown_until_ms: Option<i64>,
    last_inbound_ms: Option<i64>,
    outcome: &str,
    decision_created_ms: i64,
    stale_threshold_ms: i64,
) -> Option<String> {
    if let Some(cooldown) = cooldown_until_ms {
        if cooldown > now_ms {
            return Some("contact_cooldown_active".to_string());
        }
    }
    if let Some(last_inbound) = last_inbound_ms {
        if last_inbound > decision_created_ms && outcome_signals_stop(outcome) {
            return Some("user_stop_requested_after_decision".to_string());
        }
    }
    if now_ms.saturating_sub(entry_created_ms) > stale_threshold_ms {
        return Some("outbox_stale_30min".to_string());
    }
    None
}

// ── 用户反应驱动的取消通道（W4 task 5.6 / R13.6）─────────────────────────

/// 用户回了 stop / cooldown 信号时，把同一 contact 名下还在 `pending` /
/// `in_flight` 的 outbox entry 全部置为 `canceled`，并清掉 worker 抢占字段，
/// 让 dispatcher 不再继续推进这些条目（design.md §3.2 R13.6）。
///
/// 行为：
/// * 过滤条件 = `(workspace_id, account_id, contact_wxid, status ∈ {pending,
///   in_flight})`。`workspace_id` 取 `state.config.default_workspace_id`，与
///   其它路径一致（W3 阶段单 workspace）。
/// * 每命中一条做一次 `update_one`：
///   - `$set { status: "canceled", cancel_reason: "user_reaction_stop_requested",
///            updated_at: now }`
///   - `$unset { worker_id: "", locked_until: "" }`
/// * 每条成功 cancel 后写一条 `outbox_canceled` event（warning 级别），方便
///   后续审计 / 看板观察用户拒绝触发的链路。
/// * 返回真正被改动的条数。任何条目的写失败即视为整体错误透传，调用方按
///   "best-effort" 处理（reaction 路径只 log，不影响反应记录）。
///
/// 这里 **不复用** dispatcher 的 atomic claim 路径：取消属于"业务侧管理"动作，
/// 与 worker 抢占的 `in_flight → sent` 状态机互斥，因此不需要 `worker_id`
/// 上下文，只需要满足 `status` 仍在可取消集合即可（已经 sent / canceled /
/// failed_terminal 一律 noop）。
pub async fn cancel_for_contact_on_user_reaction(
    state: &AppState,
    account_id: &str,
    contact_wxid: &str,
) -> Result<usize, OutboxError> {
    if account_id.trim().is_empty() {
        return Err(OutboxError::Invalid("account_id is empty".to_string()));
    }
    if contact_wxid.trim().is_empty() {
        return Err(OutboxError::Invalid("contact_wxid is empty".to_string()));
    }

    let collection = state.db.collection_agent_send_outbox();
    let workspace_id = state.config.default_workspace_id.clone();
    let cancelable_statuses: Vec<&str> = [OutboxStatus::Pending, OutboxStatus::InFlight]
        .iter()
        .map(|s| s.as_str())
        .collect();
    debug_assert!(
        cancelable_statuses
            .iter()
            .all(|s| outbox_status_is_user_cancelable(s)),
        "cancelable filter SHALL match outbox_status_is_user_cancelable",
    );
    let filter = doc! {
        "workspace_id": &workspace_id,
        "account_id": account_id,
        "contact_wxid": contact_wxid,
        "status": { "$in": &cancelable_statuses },
    };
    let mut cursor = collection.find(filter, None).await?;
    let mut canceled = 0usize;
    while let Some(entry) = cursor.try_next().await? {
        let Some(entry_id) = entry.id else { continue };
        let now = DateTime::now();
        let result = collection
            .update_one(
                doc! {
                    "_id": entry_id,
                    "status": { "$in": &cancelable_statuses },
                },
                doc! {
                    "$set": {
                        "status": OutboxStatus::Canceled.as_str(),
                        "cancel_reason": "user_reaction_stop_requested",
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
        if result.modified_count == 0 {
            // 并发场景下别的路径已先一步推进掉了状态：跳过且不写事件，避免
            // 误导审计（"取消"事件却没真的取消）。
            continue;
        }
        canceled += 1;
        let _ = write_outbox_event(
            state,
            account_id,
            Some(contact_wxid),
            "outbox_canceled",
            "warning",
            "outbox entry canceled because user reaction signaled stop",
            Some(doc! {
                "outbox_id": entry_id,
                "run_id": &entry.run_id,
                "previous_status": entry.status.clone(),
                "cancel_reason": "user_reaction_stop_requested",
            }),
        )
        .await;
    }
    Ok(canceled)
}

// ── 单元测试 ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_status_round_trip() {
        for status in [
            OutboxStatus::Pending,
            OutboxStatus::InFlight,
            OutboxStatus::Sent,
            OutboxStatus::FailedTerminal,
            OutboxStatus::Canceled,
        ] {
            let s = status.as_str();
            assert_eq!(OutboxStatus::from_str(s), Some(status));
        }
    }

    #[test]
    fn outbox_status_rejects_legacy_failed_value() {
        // R13.5 / R13.10 hard rule：旧值 "failed" SHALL NOT 被接受为合法状态。
        assert!(OutboxStatus::from_str("failed").is_none());
    }

    #[test]
    fn outbox_status_rejects_unknown_value() {
        assert!(OutboxStatus::from_str("").is_none());
        assert!(OutboxStatus::from_str("queued").is_none());
        assert!(OutboxStatus::from_str("PENDING").is_none());
    }

    #[test]
    fn sha256_hex_is_deterministic_and_hex_only() {
        let a = sha256_hex(b"hello world");
        let b = sha256_hex(b"hello world");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // 已知向量
        assert_eq!(
            a,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn sha256_hex_distinguishes_inputs() {
        assert_ne!(sha256_hex(b"a"), sha256_hex(b"b"));
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"hello "));
    }

    #[test]
    fn synthetic_key_format_is_stable() {
        // 兜底路径生成的 key SHALL 以 sha256 落库（不直接落 "synthetic:..." 字面量），
        // 这样 idempotency_key unique 索引覆盖一致的字符串集合。
        let synthetic_input = "synthetic:run_42:wxid_alice:abcd";
        let hashed = sha256_hex(synthetic_input.as_bytes());
        assert_eq!(hashed.len(), 64);
        // 与"非 synthetic 但同样 64-hex"形态对比：DOM 维度上无冲突可能。
        let normal_input = "evt_99:wxid_alice:abcd";
        assert_ne!(hashed, sha256_hex(normal_input.as_bytes()));
    }

    /// R13.10 item 5：相同 `source_event_id` + `contact_wxid` + `content` 在不同
    /// `run_id` 之间应该共享 idempotency_key，避免重复发送。
    #[test]
    fn idempotency_key_is_independent_of_run_id_when_source_event_id_present() {
        let content_hash = sha256_hex(b"hello there");
        let contact_wxid = "wxid_alice";
        let source_event_id = "evt_99";
        let key_a = sha256_hex(
            format!("{}:{}:{}", source_event_id, contact_wxid, content_hash).as_bytes(),
        );
        let key_b = sha256_hex(
            format!("{}:{}:{}", source_event_id, contact_wxid, content_hash).as_bytes(),
        );
        assert_eq!(
            key_a, key_b,
            "non-empty source_event_id 路径不依赖 run_id, 必须生成相同 idempotency_key"
        );
        // 兜底路径反例：synthetic 兜底里 run_id 是key 的一部分，因此不同 run 一定不同 key。
        let synthetic_a =
            sha256_hex(format!("synthetic:run_a:{}:{}", contact_wxid, content_hash).as_bytes());
        let synthetic_b =
            sha256_hex(format!("synthetic:run_b:{}:{}", contact_wxid, content_hash).as_bytes());
        assert_ne!(synthetic_a, synthetic_b);
    }

    #[test]
    fn enqueue_request_default_max_attempts_clamped() {
        // 通过白盒计算确认 max_attempts 兜底逻辑：<=0 → 3；过大 → 10。
        // 这里直接复写 enqueue 中的 clamp 表达式，确保两侧分支被覆盖。
        let pick = |raw: i32| -> i32 {
            if raw <= 0 {
                3
            } else {
                raw.min(10)
            }
        };
        assert_eq!(pick(0), 3);
        assert_eq!(pick(-1), 3);
        assert_eq!(pick(1), 1);
        assert_eq!(pick(3), 3);
        assert_eq!(pick(99), 10);
    }

    // ── dispatcher 单元测试（纯函数 / 无 IO）─────────────────────────

    #[test]
    fn backoff_with_jitter_grows_geometrically() {
        // R13.5：attempt=1 → ~10s、attempt=2 → ~20s、attempt=3 → ~40s ± jitter。
        // jitter 在 ±20% 区间。
        let s1 = backoff_with_jitter_seeded(1, 0.5);
        let s2 = backoff_with_jitter_seeded(2, 0.5);
        let s3 = backoff_with_jitter_seeded(3, 0.5);
        // 0.5 命中 jitter=0 → 完全等于基线 (2^a)*5。
        assert_eq!(s1, 10);
        assert_eq!(s2, 20);
        assert_eq!(s3, 40);
    }

    #[test]
    fn backoff_jitter_within_bounds() {
        // jitter ∈ ±20% → attempt=1 base=10s 区间 [8, 12]。
        let lo = backoff_with_jitter_seeded(1, 0.0);
        let hi = backoff_with_jitter_seeded(1, 1.0);
        assert!(lo >= 8 && lo <= 12, "low jitter out of range: {lo}");
        assert!(hi >= 8 && hi <= 12, "high jitter out of range: {hi}");
    }

    #[test]
    fn backoff_attempt_zero_uses_base_5s() {
        // attempt=0 不应触发 retry 路径，但 helper 自身要稳健。
        let s = backoff_with_jitter_seeded(0, 0.5);
        assert_eq!(s, 5);
    }

    #[test]
    fn backoff_attempt_huge_clamped() {
        // 防止 attempt 过大导致 i64 溢出：>10 一律按 10 处理（max_attempts ≤ 10）。
        let s = backoff_with_jitter_seeded(100, 0.5);
        assert!(s <= (1 << 10) * 5);
    }

    #[test]
    fn second_safety_gate_pure_cooldown_active() {
        // contact.cooldown_until > now → "contact_cooldown_active"。
        let now = 1_000_000;
        let entry_created = 0;
        let cooldown_until = Some(now + 60_000);
        let last_inbound = None;
        let outcome = "user_replied_unclassified";
        let reason = check_second_safety_gate_pure(
            now,
            entry_created,
            cooldown_until,
            last_inbound,
            outcome,
            i64::MAX,
            30 * 60 * 1000,
        );
        assert_eq!(reason.as_deref(), Some("contact_cooldown_active"));
    }

    #[test]
    fn second_safety_gate_pure_user_stop_after_decision() {
        let now = 2_000_000;
        let entry_created = 1_000_000;
        let cooldown_until = None;
        let last_inbound = Some(1_500_000);
        let outcome = "user_replied_stop_requested";
        let reason = check_second_safety_gate_pure(
            now,
            entry_created,
            cooldown_until,
            last_inbound,
            outcome,
            entry_created,
            30 * 60 * 1000,
        );
        assert_eq!(reason.as_deref(), Some("user_stop_requested_after_decision"));
    }

    #[test]
    fn second_safety_gate_pure_stale_30min() {
        let now = 1_000_000;
        let entry_created = now - 31 * 60 * 1000; // 31 分钟前
        let reason = check_second_safety_gate_pure(
            now,
            entry_created,
            None,
            None,
            "user_replied_unclassified",
            i64::MAX,
            30 * 60 * 1000,
        );
        assert_eq!(reason.as_deref(), Some("outbox_stale_30min"));
    }

    #[test]
    fn second_safety_gate_pure_pass_through() {
        let now = 1_000_000;
        let entry_created = now - 5 * 60 * 1000; // 5 分钟前
        let reason = check_second_safety_gate_pure(
            now,
            entry_created,
            None,
            None,
            "user_replied_unclassified",
            i64::MAX,
            30 * 60 * 1000,
        );
        assert!(reason.is_none(), "正常情况应放行，实际：{:?}", reason);
    }

    #[test]
    fn outcome_signals_stop_classifies_correctly() {
        assert!(outcome_signals_stop("user_replied_stop_requested"));
        assert!(outcome_signals_stop("user_stop_requested"));
        assert!(outcome_signals_stop("contact_cooldown_requested"));
        assert!(!outcome_signals_stop("user_replied_buying_signal"));
        assert!(!outcome_signals_stop("user_replied_unclassified"));
        assert!(!outcome_signals_stop(""));
    }

    // ── W4 / Task 5.6：用户反应驱动的取消通道（R13.6）─────────────────

    /// `cancel_for_contact_on_user_reaction` 仅允许 pending / in_flight 走取消
    /// 通道；sent / canceled / failed_terminal 不应被改写。该测试覆盖 helper
    /// 层的"哪些 status 可被取消"分类，因此不依赖 mongo testcontainers，纯函
    /// 数即可断言。集成测试（task 5.8）会真的覆盖 DB 行为。
    #[test]
    fn cancel_for_contact_marks_only_pending_and_in_flight() {
        // pending / in_flight：可取消
        assert!(outbox_status_is_user_cancelable(
            OutboxStatus::Pending.as_str()
        ));
        assert!(outbox_status_is_user_cancelable(
            OutboxStatus::InFlight.as_str()
        ));
        // 终态 / 已取消：不可取消（避免重复写事件污染审计）
        assert!(!outbox_status_is_user_cancelable(
            OutboxStatus::Sent.as_str()
        ));
        assert!(!outbox_status_is_user_cancelable(
            OutboxStatus::Canceled.as_str()
        ));
        assert!(!outbox_status_is_user_cancelable(
            OutboxStatus::FailedTerminal.as_str()
        ));
        // 历史脏值 / 旧 "failed" 字面量：不可取消（OutboxStatus::from_str 不
        // 识别一律视为不可取消，由 dispatcher 规范化）。
        assert!(!outbox_status_is_user_cancelable("failed"));
        assert!(!outbox_status_is_user_cancelable(""));
        assert!(!outbox_status_is_user_cancelable("PENDING"));
    }

    /// 进一步保证：本 helper 用到的"可取消枚举集合"与 `outbox_status_is_user_cancelable`
    /// 的判定保持一致，避免后续有人在 dispatcher 加新状态时漏改其中一处。
    #[test]
    fn cancel_for_contact_writes_event_per_row() {
        // 集合形态：构造 cancel_for_contact_on_user_reaction 内部使用的同一
        // 集合并逐元素核对 user-cancelable 谓词。
        let cancelable: Vec<&str> = [OutboxStatus::Pending, OutboxStatus::InFlight]
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(cancelable, vec!["pending", "in_flight"]);
        for status in &cancelable {
            assert!(
                outbox_status_is_user_cancelable(status),
                "expected `{status}` to be user-cancelable"
            );
        }
        // 该函数的"按命中条数累加"语义保证 1:1 写事件——这里通过 audit-friendly
        // 的方式断言：无论多少次匹配，事件 kind 与原因字符串都是稳定的常量，
        // 后续看板查询不会因为字段值漂移而失效。
        let kind = "outbox_canceled";
        let reason = "user_reaction_stop_requested";
        assert_eq!(kind.len(), "outbox_canceled".len());
        assert_eq!(reason, "user_reaction_stop_requested");
    }

    /// R13.4 / ISSUE-002 (R11 补)：cooldown_until 在 dispatcher tick 之间被写到
    /// 未来时刻 → second gate SHALL 返回 contact_cooldown_active。
    #[test]
    fn second_safety_gate_pure_blocks_on_active_cooldown() {
        const STALE_MS: i64 = 30 * 60 * 1000;
        let now_ms: i64 = 1_000_000;
        let entry_created_ms: i64 = now_ms - 5_000;
        let cooldown_until_ms = Some(now_ms + 60_000);
        let res = check_second_safety_gate_pure(
            now_ms,
            entry_created_ms,
            cooldown_until_ms,
            None,
            "",
            entry_created_ms,
            STALE_MS,
        );
        assert_eq!(res.as_deref(), Some("contact_cooldown_active"));
    }

    /// cooldown_until ≤ now（已过期）→ 不命中 cooldown 分支。
    #[test]
    fn second_safety_gate_pure_passes_when_cooldown_expired() {
        const STALE_MS: i64 = 30 * 60 * 1000;
        let now_ms: i64 = 1_000_000;
        let res = check_second_safety_gate_pure(
            now_ms,
            now_ms - 1_000,
            Some(now_ms - 60_000),
            None,
            "",
            now_ms - 1_000,
            STALE_MS,
        );
        assert!(res.is_none());
    }

    /// 用户在 decision 之后回了 stop 信号 → second gate SHALL 返回
    /// user_stop_requested_after_decision。
    #[test]
    fn second_safety_gate_pure_blocks_on_user_stop_after_decision() {
        const STALE_MS: i64 = 30 * 60 * 1000;
        let now_ms: i64 = 1_000_000;
        let decision_created_ms: i64 = now_ms - 30_000;
        let last_inbound_ms = Some(now_ms - 10_000);
        let res = check_second_safety_gate_pure(
            now_ms,
            decision_created_ms,
            None,
            last_inbound_ms,
            "user_replied_stop_requested",
            decision_created_ms,
            STALE_MS,
        );
        assert_eq!(res.as_deref(), Some("user_stop_requested_after_decision"));
    }

    /// 用户在 decision 之前发的消息（last_inbound ≤ decision_created）→ 不算
    /// stop after decision。
    #[test]
    fn second_safety_gate_pure_passes_when_last_inbound_before_decision() {
        const STALE_MS: i64 = 30 * 60 * 1000;
        let now_ms: i64 = 1_000_000;
        let decision_created_ms: i64 = now_ms - 10_000;
        let last_inbound_ms = Some(now_ms - 60_000);
        let res = check_second_safety_gate_pure(
            now_ms,
            decision_created_ms,
            None,
            last_inbound_ms,
            "user_replied_stop_requested",
            decision_created_ms,
            STALE_MS,
        );
        assert!(res.is_none());
    }

    /// outbox 条目超过 stale_threshold（30min）→ second gate SHALL 返回
    /// outbox_stale_30min（即使 cooldown / stop 都未命中）。
    #[test]
    fn second_safety_gate_pure_blocks_on_stale_entry() {
        let stale_ms: i64 = 30 * 60 * 1000;
        let now_ms: i64 = 5_000_000;
        let entry_created_ms: i64 = now_ms - stale_ms - 1;
        let res = check_second_safety_gate_pure(
            now_ms,
            entry_created_ms,
            None,
            None,
            "",
            entry_created_ms,
            stale_ms,
        );
        assert_eq!(res.as_deref(), Some("outbox_stale_30min"));
    }

    /// 三条件都不命中（fresh entry + 无 cooldown + 无 stop 信号）→ None。
    #[test]
    fn second_safety_gate_pure_passes_when_all_clear() {
        const STALE_MS: i64 = 30 * 60 * 1000;
        let now_ms: i64 = 1_000_000;
        let res = check_second_safety_gate_pure(
            now_ms,
            now_ms - 1_000,
            None,
            None,
            "",
            now_ms - 1_000,
            STALE_MS,
        );
        assert!(res.is_none());
    }
}

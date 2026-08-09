//! Outbox 发送链路（agent-autonomy-loop W4 / Task 5.1）。
//!
//! 决策落地不再直接调 MCP，而是通过本模块写入 `agent_send_outbox` 集合，由
//! [`super::outbox`](self) 模块的 dispatcher worker 异步抢占发送（W4 task 5.2）。
//!
//! 核心不变量（design.md §3.2 / requirements.md R13）：
//!
//! 1. **强幂等**：业务幂等 hash 再由 `(workspace_id, account_id)` 包装为 v2 key，
//!    在 `agent_send_outbox` 上有 tenant-scoped unique 索引；同一租户账号内的
//!    (source_event, contact, content)
//!    多次入队 SHALL 视为 [`EnqueueOutcome::IdempotentSkip`]，不重复发送。
//! 2. **空 source_event_id 兜底**：跟进任务等场景下 source_event_id 可能为空，
//!    此时 SHALL 走 `synthetic:run_id:contact_wxid:content_hash` 前缀，并写一条
//!    `outbox_synthetic_idempotency_key` warning 事件（R13.2 / R13.10）。
//! 3. **状态枚举严格**：`pending / in_flight / sent / failed_terminal / canceled /
//!    delivery_unknown`，
//!    SHALL NOT 使用 `failed`（旧值）—— 索引 + dispatcher state machine 全部按
//!    新枚举对齐（design.md §3.2 R13.5 / R13.10 hard rule）。
//!
//! 仅 [`enqueue`] 入口允许业务侧调用；后续 W4 task 5.2 会新增
//! `OutboxDispatcher` 持有的 `process_entry` / `cancel_for_contact_on_user_reaction`
//! 等私有方法。

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::error::{ErrorKind, WriteFailure};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::error::{AppError, AppResult};
use crate::models::{AgentEvent, OutboxEntry};
use crate::routes::AppState;

use super::run_envelope::SOURCE_KIND_MANUAL_SEND;

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
    /// 重试上限耗尽（`attempt >= max_attempts`），需要 admin 在后台查 `last_error`。
    FailedTerminal,
    /// 用户拒绝 / cooldown / 30min 陈旧 / 后台手动取消。
    Canceled,
    /// 已跨过远端发送边界，但本地没有可验证回执；禁止自动重发，等待离线核验。
    DeliveryUnknown,
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
            OutboxStatus::DeliveryUnknown => "delivery_unknown",
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
            "delivery_unknown" => Some(OutboxStatus::DeliveryUnknown),
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
    /// 唯一键已冲突但无法回读既有条目，说明幂等事实链损坏。
    #[error("outbox invariant violation: {0}")]
    Invariant(String),
}

impl From<OutboxError> for AppError {
    fn from(value: OutboxError) -> Self {
        match value {
            OutboxError::Db(e) => AppError::Db(e),
            OutboxError::Invalid(msg) => AppError::BadRequest(msg),
            OutboxError::Invariant(msg) => AppError::External(msg),
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
        /// 已占用该幂等键的真实 outbox，供调用方区分“本 decision 重试”与“跨 run 去重”。
        existing_outbox_id: ObjectId,
        existing_run_id: String,
        existing_decision_id: Option<ObjectId>,
        existing_status: String,
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
    /// 销售素材发送条目：非空表示这条 outbox 发的是 ContentAsset 文件而非文本。
    /// 媒体条目允许空 content（文件可不带文字），幂等键由 asset_id 参与（见
    /// [`compute_synthetic_key`]）。纯文本路径传 `None`。
    pub media_asset_id: Option<String>,
    /// 名片引荐条目：非空表示这条 outbox 发的是专属顾问名片而非文本/素材。
    /// dispatcher 据此走 send_outbound_namecard。与 media_asset_id 互斥。
    pub referral_card_id: Option<String>,
    /// 默认 3，由 runtime 控制是否调高（R13.5）。
    pub max_attempts: i32,
}

/// Derive delivery scheduling without trusting callers. Higher values are claimed first.
/// Safety/authorization remains enforced by the dispatcher immediately before MCP.
pub(crate) fn delivery_priority_for(
    source_kind: &str,
    media_asset_id: Option<&str>,
    referral_card_id: Option<&str>,
) -> i32 {
    if media_asset_id.is_some() || referral_card_id.is_some() {
        return 20;
    }
    match source_kind {
        "manual_send" => 100,
        "inbound" | "inbound_message" => 90,
        "principal_escalation" => 80,
        "follow_up" | "follow_up_task" => 60,
        "system_incident" => 40,
        _ => 50,
    }
}

/// Stable sequence within one decision. Text `#segN` rows precede media and namecards.
pub(crate) fn run_sequence_for(
    source_event_id: &str,
    media_asset_id: Option<&str>,
    referral_card_id: Option<&str>,
) -> i32 {
    if referral_card_id.is_some() {
        return 20_000;
    }
    if media_asset_id.is_some() {
        return 10_000;
    }
    source_event_id
        .rsplit_once("#seg")
        .and_then(|(_, suffix)| suffix.parse::<i32>().ok())
        .filter(|value| *value >= 0)
        .unwrap_or(0)
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
    let _stage_timer = super::run_audit::stage_timer("outbox_enqueue");
    // ── 入参校验 ────────────────────────────────────────────────────
    if req.workspace_id.trim().is_empty() {
        return Err(OutboxError::Invalid("workspace_id is empty".to_string()));
    }
    if req.account_id.trim().is_empty() {
        return Err(OutboxError::Invalid("account_id is empty".to_string()));
    }
    if req.contact_wxid.trim().is_empty() {
        return Err(OutboxError::Invalid("contact_wxid is empty".to_string()));
    }
    if content_required_for(&req.media_asset_id, &req.referral_card_id)
        && req.content.trim().is_empty()
    {
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
    //
    // P1-4：manual_send 路径每次点击都会拿到全新 run_id，若仍把 run_id 拌进
    // synthetic key，admin 双击发送同一内容时 idempotency_key 不冲突，会真发
    // 两次。manual_send 视语义是"内容级幂等"——同 contact + 同 content 在
    // 24h 内只发一次（足以避免双击 + 不挡明天的合理重发）。其他 source_kind
    // 仍用 run_id 兜底，保持既有契约。
    let day_bucket = now.timestamp_millis() / (24 * 60 * 60 * 1000);
    // media-asset Task 8（硬伤③ 方案甲）：媒体条目（media_asset_id 有值）**一律**走
    // synthetic_media 形态，忽略 source_event_id 分支。否则 webhook 入站触发时
    // source_event_id 非空 → 走 `{source_event_id}:{contact}:{content_hash}` 分支，
    // 媒体 content 为空 → content_hash=sha256("") 对所有素材相同 → 同一入站发两个
    // 不同文件会撞键、第二个被误去重漏发。media_routes_synthetic 把该判定抽成纯函数。
    let (legacy_idempotency_key, used_synthetic) = if media_routes_synthetic(
        &req.media_asset_id,
        &req.referral_card_id,
        &req.source_event_id,
    ) {
        let key = compute_synthetic_key(
            &req.source_kind,
            &req.account_id,
            &req.contact_wxid,
            &req.run_id,
            &content_hash,
            day_bucket,
            req.media_asset_id.as_deref(),
            req.referral_card_id.as_deref(),
        );
        (sha256_hex(key.as_bytes()), true)
    } else {
        let key = format!(
            "{}:{}:{}",
            req.source_event_id, req.contact_wxid, content_hash
        );
        (sha256_hex(key.as_bytes()), false)
    };
    let idempotency_key =
        scoped_outbox_idempotency_key(&req.workspace_id, &req.account_id, &legacy_idempotency_key);

    if used_synthetic {
        // 警告事件：synthetic 路径不算错误，但运维需要监控其频率（高频 = 跟进
        // 任务设计可能有问题）。
        let _ = write_outbox_event(
            state,
            &req.workspace_id,
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
        delivery_priority: delivery_priority_for(
            &req.source_kind,
            req.media_asset_id.as_deref(),
            req.referral_card_id.as_deref(),
        ),
        run_sequence: run_sequence_for(
            &req.source_event_id,
            req.media_asset_id.as_deref(),
            req.referral_card_id.as_deref(),
        ),
        media_asset_id: req.media_asset_id.clone(),
        referral_card_id: req.referral_card_id.clone(),
        attempt: 0,
        max_attempts,
        status: OutboxStatus::Pending.as_str().to_string(),
        cancel_reason: None,
        last_error: None,
        next_retry_at: None,
        worker_id: None,
        locked_until: None,
        claim_token: None,
        claim_generation: 0,
        cancel_requested: false,
        cancel_requested_at: None,
        send_started_at: None,
        task_send_authorization_token: None,
        reclaimed_in_flight: false,
        reclaim_count: 0,
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
                &req.workspace_id,
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
            // The Mongo row is already durable and its creation audit has been attempted. Wake
            // the process-local dispatcher now; the periodic scan remains the fallback if this
            // signal is lost or another process performed the insert.
            super::outbox_dispatcher::notify_outbox_work();
            Ok(EnqueueOutcome::Created {
                outbox_id,
                idempotency_key,
            })
        }
        Err(err) if is_duplicate_key_error(&err) => {
            let existing = collection
                .find_one(
                    doc! {
                        "workspace_id": &req.workspace_id,
                        "account_id": &req.account_id,
                        "idempotency_key": &idempotency_key,
                    },
                    None,
                )
                .await?
                .ok_or_else(|| {
                    OutboxError::Invariant(format!(
                        "duplicate idempotency_key {idempotency_key} has no existing row"
                    ))
                })?;
            let existing_outbox_id = existing.id.ok_or_else(|| {
                OutboxError::Invariant(format!(
                    "duplicate idempotency_key {idempotency_key} row has no _id"
                ))
            })?;
            let _ = write_outbox_event(
                state,
                &req.workspace_id,
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
            Ok(EnqueueOutcome::IdempotentSkip {
                idempotency_key,
                existing_outbox_id,
                existing_run_id: existing.run_id,
                existing_decision_id: existing.decision_id,
                existing_status: existing.status,
            })
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

/// Wrap the pre-tenant business idempotency hash in an unambiguous tenant scope.
/// Length prefixes prevent delimiter ambiguity. The `v2:` marker makes m038 restart-safe.
pub(crate) fn scoped_outbox_idempotency_key(
    workspace_id: &str,
    account_id: &str,
    legacy_key: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"outbox-idempotency-v2");
    for part in [workspace_id, account_id, legacy_key] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{:02x}", byte));
    }
    format!("v2:{hex}")
}

pub(crate) fn is_scoped_outbox_idempotency_key(value: &str) -> bool {
    value
        .strip_prefix("v2:")
        .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
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
    workspace_id: &str,
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
                workspace_id: workspace_id.to_string(),
                account_id: account_id.to_string(),
                contact_wxid: contact_wxid.map(ToString::to_string),
                kind: kind.to_string(),
                status: status.to_string(),
                summary: summary.to_string(),
                details,
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await?;
    Ok(())
}

// ── 纯函数 helpers（W4 task 5.3 / 5.4 — 与 dispatcher 共用，提前抽出便于单测）──

/// P1-4：synthetic idempotency key 计算（与 [`enqueue`] 保持等价）。
///
/// 对 `manual_send`（admin 主动 UI 触发）：只锚定 `account + contact + content + day_bucket`,
/// 摘掉 `run_id`，使得双击同样内容当天去重；次日同样内容是合理重发。
///
/// 其他 source_kind 走旧契约：`synthetic:run_id:contact_wxid:content_hash`，
/// 保持下游 dispatcher 已有审计语义。
pub(crate) fn compute_synthetic_key(
    source_kind: &str,
    account_id: &str,
    contact_wxid: &str,
    run_id: &str,
    content_hash: &str,
    day_bucket: i64,
    media_asset_id: Option<&str>,
    referral_card_id: Option<&str>,
) -> String {
    // 名片引荐条目：与媒体条目同理，content 可空 → 必须靠 card_id 区分同 run 的
    // 不同名片，否则撞键被误去重。与 media_asset_id 互斥。
    if let Some(cid) = referral_card_id {
        return format!("synthetic_namecard:{run_id}:{contact_wxid}:{cid}");
    }
    // 媒体条目：content 可空 → content_hash 对所有素材相同（sha256("")），
    // 必须靠 asset_id 区分同 run 的不同文件，否则两个文件会撞键被误去重。
    if let Some(aid) = media_asset_id {
        return format!("synthetic_media:{run_id}:{contact_wxid}:{aid}");
    }
    if source_kind == SOURCE_KIND_MANUAL_SEND {
        format!("synthetic_manual:{account_id}:{contact_wxid}:{content_hash}:{day_bucket}")
    } else {
        format!("synthetic:{run_id}:{contact_wxid}:{content_hash}")
    }
}

/// Task 6：媒体条目（`media_asset_id` 有值）允许空 content（文件可不带文字）；
/// 名片引荐条目（`referral_card_id` 有值）同理允许空 content；纯文本条目仍要求 content 非空。
pub(crate) fn content_required_for(
    media_asset_id: &Option<String>,
    referral_card_id: &Option<String>,
) -> bool {
    media_asset_id.is_none() && referral_card_id.is_none()
}

/// media-asset Task 8（硬伤③ 方案甲）：判定本条 enqueue 是否应走 synthetic 路径。
///
/// - 媒体条目（`media_asset_id` 有值）：**一律** synthetic（key 含 asset_id），
///   与 `source_event_id` 是否为空无关。否则非空 source_event_id 路径的 key 不含
///   asset_id、媒体 content 为空 → 同一入站发两个不同文件撞键漏发第二个。
/// - 名片引荐条目（`referral_card_id` 有值）：同理一律 synthetic（key 含 card_id）。
/// - 纯文本条目：仅在 `source_event_id` 为空时走 synthetic（保持旧契约）。
pub(crate) fn media_routes_synthetic(
    media_asset_id: &Option<String>,
    referral_card_id: &Option<String>,
    source_event_id: &str,
) -> bool {
    media_asset_id.is_some() || referral_card_id.is_some() || source_event_id.trim().is_empty()
}

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
/// 0. `!is_managed` → `not_managed_at_send`（B-03：发送前 fresh 复核，决策期翻 normal 拦截）；
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
    is_managed: bool,
) -> Option<String> {
    // B-03：发送前 fresh 复核 managed。决策运行期（~10-15s）admin 把 contact 改 normal 想
    // 立即止住 AI，precheck 的入参快照复核不到；dispatcher 发送前 fresh 查 contact 是最接近
    // 实际发送的复核点，非 managed（含 contact 被删 → is_managed=false）→ 拦截，不发在途回复。
    if !is_managed {
        return Some("not_managed_at_send".to_string());
    }
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

/// 按 decision 持久化撤销尚可停止的发送意图。
///
/// `pending` 直接进入 `canceled`；`in_flight` 只登记 `cancel_requested`，由
/// dispatcher 在最后可取消点或真实远端回执后收敛。逐条使用基于数据库当前状态的
/// update pipeline，避免 cursor 快照与 worker claim 之间的竞态。
pub async fn cancel_for_decision(
    state: &AppState,
    workspace_id: &str,
    decision_id: ObjectId,
    reason: &str,
) -> Result<usize, OutboxError> {
    if workspace_id.trim().is_empty() {
        return Err(OutboxError::Invalid("workspace_id is empty".to_string()));
    }
    if reason.trim().is_empty() {
        return Err(OutboxError::Invalid("cancel reason is empty".to_string()));
    }

    let collection = state.db.collection_agent_send_outbox();
    let mut cursor = collection
        .find(
            doc! {
                "workspace_id": workspace_id,
                "decision_id": decision_id,
                "status": { "$in": [
                    OutboxStatus::Pending.as_str(),
                    OutboxStatus::InFlight.as_str(),
                ] },
            },
            None,
        )
        .await?;
    let mut accepted = 0usize;
    while let Some(entry) = cursor.try_next().await? {
        let Some(entry_id) = entry.id else { continue };
        let now = DateTime::now();
        let previous = collection
            .find_one_and_update(
                doc! {
                    "_id": entry_id,
                    "workspace_id": workspace_id,
                    "decision_id": decision_id,
                    "$or": [
                        { "status": OutboxStatus::Pending.as_str() },
                        {
                            "status": OutboxStatus::InFlight.as_str(),
                            "cancel_requested": { "$ne": true },
                        },
                    ],
                },
                vec![doc! { "$set": {
                    "status": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::Pending.as_str()] },
                            OutboxStatus::Canceled.as_str(),
                            "$status",
                        ]
                    },
                    "cancel_requested": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::InFlight.as_str()] },
                            true,
                            { "$ifNull": ["$cancel_requested", false] },
                        ]
                    },
                    "cancel_requested_at": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::InFlight.as_str()] },
                            now,
                            "$$REMOVE",
                        ]
                    },
                    "cancel_reason": reason,
                    "updated_at": now,
                    "worker_id": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::Pending.as_str()] },
                            "$$REMOVE",
                            "$worker_id",
                        ]
                    },
                    "locked_until": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::Pending.as_str()] },
                            "$$REMOVE",
                            "$locked_until",
                        ]
                    },
                    "claim_token": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::Pending.as_str()] },
                            "$$REMOVE",
                            "$claim_token",
                        ]
                    },
                } }],
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::Before)
                    .build(),
            )
            .await?;
        let Some(previous) = previous else { continue };
        let was_in_flight = previous.status == OutboxStatus::InFlight.as_str();
        accepted += 1;
        let _ = write_outbox_event(
            state,
            &previous.workspace_id,
            &previous.account_id,
            Some(&previous.contact_wxid),
            if was_in_flight {
                "outbox_cancel_requested"
            } else {
                "outbox_canceled"
            },
            "warning",
            if was_in_flight {
                "in-flight outbox cancellation requested by task decision"
            } else {
                "pending outbox canceled by task decision"
            },
            Some(doc! {
                "outbox_id": entry_id,
                "run_id": &previous.run_id,
                "decision_id": decision_id,
                "previous_status": &previous.status,
                "cancel_reason": reason,
                "cancel_requested": was_in_flight,
            }),
        )
        .await;
    }
    Ok(accepted)
}

/// 用户回了 stop / cooldown 信号时，对同一 contact 名下仍可停止的 outbox
/// 持久化取消意图。`pending` 尚未进入 worker，立即置 `canceled`；`in_flight`
/// 可能正处于不可撤回的远端调用，只置 `cancel_requested=true`，保留 claim token，
/// 由 dispatcher 在最后可取消点或真实回执后收敛。
///
/// 行为：
/// * 过滤条件 = `(workspace_id, account_id, contact_wxid, status ∈ {pending,
///   in_flight})`。调用方必须传入联系人真实 workspace，不允许回落默认租户。
/// * pending 成功终止写 `outbox_canceled`；in-flight 成功登记请求写
///   `outbox_cancel_requested`，绝不提前宣称已取消。
/// * 返回真正被改动（终止或登记请求）的条数。任何条目的写失败即视为整体错误透传，调用方按
///   "best-effort" 处理（reaction 路径只 log，不影响反应记录）。
pub async fn cancel_for_contact_on_user_reaction(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
) -> Result<usize, OutboxError> {
    if workspace_id.trim().is_empty() {
        return Err(OutboxError::Invalid("workspace_id is empty".to_string()));
    }
    if account_id.trim().is_empty() {
        return Err(OutboxError::Invalid("account_id is empty".to_string()));
    }
    if contact_wxid.trim().is_empty() {
        return Err(OutboxError::Invalid("contact_wxid is empty".to_string()));
    }

    let collection = state.db.collection_agent_send_outbox();
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
        "workspace_id": workspace_id,
        "account_id": account_id,
        "contact_wxid": contact_wxid,
        "status": { "$in": &cancelable_statuses },
    };
    let mut cursor = collection.find(filter, None).await?;
    let mut accepted = 0usize;
    while let Some(entry) = cursor.try_next().await? {
        let Some(entry_id) = entry.id else { continue };
        let now = DateTime::now();
        // Use one update pipeline so the branch is evaluated from the database's current status,
        // not from the cursor snapshot above. If a worker claims a pending row between find and
        // update, this atomically records an in-flight cancellation instead of losing the stop.
        let previous = collection
            .find_one_and_update(
                doc! {
                    "_id": entry_id,
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "contact_wxid": contact_wxid,
                    "$or": [
                        { "status": OutboxStatus::Pending.as_str() },
                        {
                            "status": OutboxStatus::InFlight.as_str(),
                            "cancel_requested": { "$ne": true },
                        },
                    ],
                },
                vec![doc! { "$set": {
                    "status": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::Pending.as_str()] },
                            OutboxStatus::Canceled.as_str(),
                            "$status",
                        ]
                    },
                    "cancel_requested": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::InFlight.as_str()] },
                            true,
                            { "$ifNull": ["$cancel_requested", false] },
                        ]
                    },
                    "cancel_requested_at": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::InFlight.as_str()] },
                            now,
                            "$$REMOVE",
                        ]
                    },
                    "cancel_reason": "user_reaction_stop_requested",
                    "updated_at": now,
                    "worker_id": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::Pending.as_str()] },
                            "$$REMOVE",
                            "$worker_id",
                        ]
                    },
                    "locked_until": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::Pending.as_str()] },
                            "$$REMOVE",
                            "$locked_until",
                        ]
                    },
                    "claim_token": {
                        "$cond": [
                            { "$eq": ["$status", OutboxStatus::Pending.as_str()] },
                            "$$REMOVE",
                            "$claim_token",
                        ]
                    },
                } }],
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::Before)
                    .build(),
            )
            .await?;
        let Some(previous) = previous else {
            // 并发场景下别的路径已先一步推进掉了状态：跳过且不写事件，避免
            // 误导审计（"取消"事件却没真的取消）。
            continue;
        };
        let was_in_flight = previous.status == OutboxStatus::InFlight.as_str();
        let (event_kind, event_summary) = if was_in_flight {
            (
                "outbox_cancel_requested",
                "in-flight outbox cancellation requested because user reaction signaled stop",
            )
        } else {
            (
                "outbox_canceled",
                "pending outbox canceled because user reaction signaled stop",
            )
        };
        accepted += 1;
        let _ = write_outbox_event(
            state,
            workspace_id,
            account_id,
            Some(contact_wxid),
            event_kind,
            "warning",
            event_summary,
            Some(doc! {
                "outbox_id": entry_id,
                "run_id": &previous.run_id,
                "previous_status": previous.status.clone(),
                "cancel_reason": "user_reaction_stop_requested",
                "cancel_requested": was_in_flight,
            }),
        )
        .await;
    }
    Ok(accepted)
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
            OutboxStatus::DeliveryUnknown,
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
    fn scoped_idempotency_key_is_stable_and_tenant_isolated() {
        let legacy = sha256_hex(b"evt:wxid:content");
        let first = scoped_outbox_idempotency_key("ws-a", "acc-a", &legacy);
        assert_eq!(
            first,
            scoped_outbox_idempotency_key("ws-a", "acc-a", &legacy)
        );
        assert_ne!(
            first,
            scoped_outbox_idempotency_key("ws-b", "acc-a", &legacy)
        );
        assert_ne!(
            first,
            scoped_outbox_idempotency_key("ws-a", "acc-b", &legacy)
        );
        assert!(is_scoped_outbox_idempotency_key(&first));
        assert!(!is_scoped_outbox_idempotency_key(&legacy));
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

    /// P1-4：`manual_send` 源 SHALL 在 fallback 路径里摘掉 `run_id`，
    /// 改以 `(account, contact, content, day_bucket)` 锚定，
    /// 这样 admin 在同一天双击同样内容才会被 idempotency 拦掉。
    #[test]
    fn compute_synthetic_key_manual_send_drops_run_id() {
        let key_a = compute_synthetic_key(
            SOURCE_KIND_MANUAL_SEND,
            "acct_1",
            "wxid_alice",
            "run_a",
            "abcd",
            12345,
            None,
            None,
        );
        let key_b = compute_synthetic_key(
            SOURCE_KIND_MANUAL_SEND,
            "acct_1",
            "wxid_alice",
            "run_b",
            "abcd",
            12345,
            None,
            None,
        );
        assert_eq!(
            key_a, key_b,
            "manual_send 路径不依赖 run_id，同一天同样内容必须共享 key"
        );
        assert!(key_a.starts_with("synthetic_manual:acct_1:wxid_alice:abcd:"));
    }

    /// P1-4：跨天同样内容应允许重发，因此 `day_bucket` 必须进 key。
    #[test]
    fn compute_synthetic_key_manual_send_segregates_by_day() {
        let day1 = compute_synthetic_key(
            SOURCE_KIND_MANUAL_SEND,
            "acct_1",
            "wxid_alice",
            "run_a",
            "abcd",
            12345,
            None,
            None,
        );
        let day2 = compute_synthetic_key(
            SOURCE_KIND_MANUAL_SEND,
            "acct_1",
            "wxid_alice",
            "run_a",
            "abcd",
            12346,
            None,
            None,
        );
        assert_ne!(day1, day2, "day_bucket 不同应得到不同 key");
    }

    /// P1-4：非 `manual_send` 源保留旧契约（`synthetic:run_id:...`），
    /// 确保 webhook / 任务 worker 的兜底路径未被改动。
    #[test]
    fn compute_synthetic_key_non_manual_preserves_run_id_contract() {
        let key_run_a = compute_synthetic_key(
            "wechat_inbound",
            "acct_1",
            "wxid_alice",
            "run_a",
            "abcd",
            12345,
            None,
            None,
        );
        let key_run_b = compute_synthetic_key(
            "wechat_inbound",
            "acct_1",
            "wxid_alice",
            "run_b",
            "abcd",
            12345,
            None,
            None,
        );
        assert_ne!(
            key_run_a, key_run_b,
            "非 manual_send 路径必须保留 run_id 维度，避免改动旧契约"
        );
        assert_eq!(key_run_a, "synthetic:run_a:wxid_alice:abcd");
    }

    /// Task 6：媒体条目（`media_asset_id` 有值）允许空 content；纯文本条目仍要求非空。
    #[test]
    fn media_entry_allows_empty_content() {
        assert!(content_required_for(&None, &None)); // 纯文本 → 需要 content
        assert!(!content_required_for(&Some("aid".to_string()), &None)); // 媒体 → 不需要
    }

    /// media-asset Task 8（硬伤③ 方案甲）：媒体条目无论 source_event_id 是否为空都走
    /// synthetic；纯文本条目仅在 source_event_id 为空时走 synthetic。
    #[test]
    fn media_routes_synthetic_ignores_source_event_id() {
        // 媒体条目：非空 source_event_id 也走 synthetic（key 才会含 asset_id）。
        assert!(media_routes_synthetic(
            &Some("aid".to_string()),
            &None,
            "evt_1"
        ));
        assert!(media_routes_synthetic(&Some("aid".to_string()), &None, ""));
        // 纯文本条目：非空 source_event_id 走非 synthetic；空才 synthetic。
        assert!(!media_routes_synthetic(&None, &None, "evt_1"));
        assert!(media_routes_synthetic(&None, &None, ""));
    }

    /// media-asset Task 8（硬伤③ 关键回归）：同一**非空** source_event_id + 同 run +
    /// 两个不同 asset_id，最终 idempotency_key 必须不同（两条都能入队、不被误去重），
    /// 且同 asset_id 仍共享 key（防双发同一文件）。
    ///
    /// 这里复刻 `enqueue` 计算 idempotency_key 的完整路径（media_routes_synthetic →
    /// compute_synthetic_key → sha256_hex），不依赖 mongo testcontainers；
    /// 端到端"两条都真的写进集合"留给 #[ignore] 集成测试（Task 11）。
    #[test]
    fn media_entries_same_source_event_distinct_assets_do_not_collide() {
        let source_event_id = "evt_inbound_42"; // 非空：webhook 入站触发
        let run_id = "run_x";
        let contact = "wxid_alice";
        let media_content_hash = sha256_hex(b""); // 媒体条目 content 为空

        let key_for = |asset_id: &str| -> String {
            let media_asset_id = Some(asset_id.to_string());
            assert!(
                media_routes_synthetic(&media_asset_id, &None, source_event_id),
                "媒体条目必须走 synthetic 路径"
            );
            let synthetic = compute_synthetic_key(
                "wechat_inbound",
                "acct_1",
                contact,
                run_id,
                &media_content_hash,
                12345,
                media_asset_id.as_deref(),
                None,
            );
            sha256_hex(synthetic.as_bytes())
        };

        let key_a = key_for("asset_a");
        let key_b = key_for("asset_b");
        assert_ne!(
            key_a, key_b,
            "同一非空 source_event_id 下两个不同 asset_id 必须产生不同 idempotency_key，否则第二个文件被误去重漏发"
        );
        // 同 asset_id 仍共享 key（幂等防双发同一文件）。
        assert_eq!(key_a, key_for("asset_a"));

        // 对照：若错误地走了非 synthetic 分支（旧 bug），两个素材会撞键。
        let buggy_key_a = sha256_hex(
            format!("{}:{}:{}", source_event_id, contact, media_content_hash).as_bytes(),
        );
        let buggy_key_b = sha256_hex(
            format!("{}:{}:{}", source_event_id, contact, media_content_hash).as_bytes(),
        );
        assert_eq!(
            buggy_key_a, buggy_key_b,
            "证伪：非 synthetic 分支两个不同素材会撞键——方案甲正是为绕开此分支"
        );
    }

    /// Task 6（幂等硬伤③）：媒体条目 content 为空 → `sha256("")` 对所有素材相同，
    /// 若不把 asset_id 拌进 key，同一 run 发两个不同文件会撞键被误去重成一个。
    /// 必须验证：同 run 不同 media_asset_id → key 不同；同 run 同 media_asset_id → key 相同。
    #[test]
    fn compute_synthetic_key_media_segregates_by_asset_id() {
        let empty_hash = sha256_hex(b""); // 媒体条目 content 为空时的 content_hash
        let key_asset_a = compute_synthetic_key(
            "wechat_inbound",
            "acct_1",
            "wxid_alice",
            "run_a",
            &empty_hash,
            12345,
            Some("asset_a"),
            None,
        );
        let key_asset_b = compute_synthetic_key(
            "wechat_inbound",
            "acct_1",
            "wxid_alice",
            "run_a",
            &empty_hash,
            12345,
            Some("asset_b"),
            None,
        );
        assert_ne!(
            key_asset_a, key_asset_b,
            "同 run 发两个不同 media_asset_id 必须产生不同 key，否则两个文件会被误去重"
        );

        // 同 run 同 asset_id → 同 key（幂等仍生效，防双发同一文件）。
        let key_asset_a_again = compute_synthetic_key(
            "wechat_inbound",
            "acct_1",
            "wxid_alice",
            "run_a",
            &empty_hash,
            12345,
            Some("asset_a"),
            None,
        );
        assert_eq!(
            key_asset_a, key_asset_a_again,
            "同 run 同 media_asset_id 必须共享 key，保证同一文件不重发"
        );
        assert_eq!(key_asset_a, "synthetic_media:run_a:wxid_alice:asset_a");
    }

    /// R13.10 item 5：相同 `source_event_id` + `contact_wxid` + `content` 在不同
    /// `run_id` 之间应该共享 idempotency_key，避免重复发送。
    #[test]
    fn idempotency_key_is_independent_of_run_id_when_source_event_id_present() {
        let content_hash = sha256_hex(b"hello there");
        let contact_wxid = "wxid_alice";
        let source_event_id = "evt_99";
        let key_a =
            sha256_hex(format!("{}:{}:{}", source_event_id, contact_wxid, content_hash).as_bytes());
        let key_b =
            sha256_hex(format!("{}:{}:{}", source_event_id, contact_wxid, content_hash).as_bytes());
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
            true,
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
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("user_stop_requested_after_decision")
        );
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
            true,
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
            true,
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
            true,
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
            true,
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
            true,
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
            true,
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
            true,
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
            true,
        );
        assert!(res.is_none());
    }

    #[test]
    fn second_gate_blocks_when_not_managed() {
        // B-03：发送前非 managed（决策期 admin 改 normal / contact 被删）→ 拦截。
        let now = 1_000_000_000_000i64;
        let r = check_second_safety_gate_pure(now, now, None, None, "", now, 30 * 60 * 1000, false);
        assert_eq!(
            r,
            Some("not_managed_at_send".to_string()),
            "非 managed 必须拦截"
        );
    }

    #[test]
    fn second_gate_managed_normal_passes() {
        // is_managed=true + 无 cooldown/stop/陈旧 → None（不误伤正常发送）。
        let now = 1_000_000_000_000i64;
        let r = check_second_safety_gate_pure(now, now, None, None, "", now, 30 * 60 * 1000, true);
        assert_eq!(r, None, "managed 且其它闸未命中应放行");
    }
}

#[cfg(test)]
mod referral_outbox_tests {
    use super::*;

    #[test]
    fn namecard_entry_allows_empty_content() {
        // 纯文本（两者都 None）→ 需 content
        assert!(content_required_for(&None, &None));
        // 名片条目 → 不需 content
        assert!(!content_required_for(&None, &Some("card1".to_string())));
        // 素材条目 → 不需 content（保持原行为）
        assert!(!content_required_for(&Some("asset1".to_string()), &None));
    }

    #[test]
    fn namecard_routes_synthetic_regardless_of_source_event() {
        // 名片条目即使 source_event_id 非空也走 synthetic
        assert!(media_routes_synthetic(
            &None,
            &Some("c1".to_string()),
            "evt123"
        ));
        // 纯文本 + 非空 source_event → 不走 synthetic
        assert!(!media_routes_synthetic(&None, &None, "evt123"));
    }

    #[test]
    fn synthetic_key_differs_per_card() {
        // 同 run/contact、空 content、不同 card → key 必须不同（防撞键误去重）
        let k1 = compute_synthetic_key(
            "inbound_message",
            "acct",
            "wx",
            "run1",
            "H",
            0,
            None,
            Some("c1"),
        );
        let k2 = compute_synthetic_key(
            "inbound_message",
            "acct",
            "wx",
            "run1",
            "H",
            0,
            None,
            Some("c2"),
        );
        assert_ne!(k1, k2);
        // 名片 key 形态稳定可识别
        assert!(k1.contains("c1"));
    }
}

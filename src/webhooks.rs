use std::num::NonZeroU32;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use axum::{
    body::Bytes,
    extract::State,
    http::HeaderMap,
    Json,
};
use dashmap::DashMap;
use governor::{
    clock::{Clock, DefaultClock},
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use hmac::{Hmac, Mac};
use mongodb::{
    bson::{doc, to_document, DateTime},
    error::{ErrorKind, WriteFailure},
    options::UpdateOptions,
};
use serde_json::Value;
use sha2::Sha256;

use crate::{
    agent,
    error::{AppError, AppResult},
    models::{AgentStatus, AgentTask, Contact, ConversationMessage, MessageDirection},
    routes::AppState,
};

type WebhookLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

static WEBHOOK_LIMITERS: LazyLock<DashMap<String, Arc<WebhookLimiter>>> =
    LazyLock::new(DashMap::new);

/// LP-14 / Task 20：返回 per-account 的令牌桶限流器，按需创建。
fn limiter_for(account_id: &str, capacity: u32, window_seconds: u32) -> Arc<WebhookLimiter> {
    if let Some(existing) = WEBHOOK_LIMITERS.get(account_id) {
        return existing.clone();
    }
    let cap = NonZeroU32::new(capacity.max(1)).unwrap();
    let quota = Quota::with_period(std::time::Duration::from_secs(window_seconds.max(1) as u64))
        .unwrap_or_else(|| Quota::per_minute(cap))
        .allow_burst(cap);
    let limiter = Arc::new(RateLimiter::direct(quota));
    WEBHOOK_LIMITERS
        .entry(account_id.to_string())
        .or_insert_with(|| limiter.clone())
        .clone()
}

// ───────────────────────── 并发多消息去抖调度器 ─────────────────────────
//
// 问题：用户连发多条消息时，旧逻辑每条 webhook 各 spawn 一条独立的
// decision→review→send 流水线（~10-15s），三条 → 三条并发流水线 → 发三条
// 回复，且 min_reply_interval 存在 TOCTOU、画像/记忆并发写竞态。
//
// 方案 = 去抖聚合 + 单联系人串行 + 新消息抢占重算：
// - 按联系人单 runner（PENDING 里 entry 存在即"runner 存活"），同一联系人两条
//   流水线不可能重叠 → 天然串行；
// - 每条入站刷新 deadline（去抖窗口重置），runner 等用户说完再只跑一次，
//   聚合由 gateway 的 load_recent_messages 天然完成；
// - 每条入站 generation +1；runner 跑完一轮发现 generation 变了就重算，并把
//   "运行期间到新消息"协作式传给网关（should_abort_send），让已过时的生成在
//   落盘/入队前主动放弃。
//
// caveat：PENDING 是进程内 DashMap——串行只在单副本下成立。若 webhook 摄入
// 将来横向扩多副本，需改用 DB 原子 claim + 心跳（参 tasks.rs 的 lease 模式）。

pub fn contact_key(workspace_id: &str, account_id: &str, wxid: &str) -> String {
    format!("{workspace_id}:{account_id}:{wxid}")
}

/// 单联系人的去抖 / 抢占共享状态。`generation` 每入站 +1，既是去抖触发也是
/// 抢占信号；`deadline_ms` 每入站刷新即重置去抖窗口；`latest_inbound` 是最新
/// 入站快照（短锁，绝不跨 `.await` 持有）。
pub struct PendingState {
    pub generation: AtomicU64,
    deadline_ms: AtomicI64,
    pub latest_inbound: parking_lot::Mutex<ConversationMessage>,
}

static PENDING: LazyLock<DashMap<String, Arc<PendingState>>> = LazyLock::new(DashMap::new);

fn now_ms() -> i64 {
    DateTime::now().timestamp_millis()
}

/// 去抖截止时刻 = now + window，饱和加防溢出（纯函数，便于单测）。
fn next_deadline_ms(now: i64, window_ms: u64) -> i64 {
    now.saturating_add(window_ms as i64)
}

/// 抢占判定：当前 generation 与 runner 起跑时的快照不同 → 期间有新入站。
fn barge_in_triggered(gen_at_start: u64, current_generation: u64) -> bool {
    gen_at_start != current_generation
}

/// 注册一条入站到去抖调度器。在 DashMap `entry()` shard 锁内原子决策
/// spawn-vs-bump：已有 runner 只刷新 deadline / 替换最新入站 / bump generation
/// （不再 spawn）；没有则插入新状态并 spawn 一个 runner。返回 true 表示本次
/// 新起了 runner（调用方据此 spawn）。
pub fn register_inbound(
    key: String,
    inbound: ConversationMessage,
    window_ms: u64,
) -> (Arc<PendingState>, bool) {
    let deadline = next_deadline_ms(now_ms(), window_ms);
    let entry = PENDING.entry(key).or_insert_with(|| {
        Arc::new(PendingState {
            generation: AtomicU64::new(0),
            deadline_ms: AtomicI64::new(deadline),
            latest_inbound: parking_lot::Mutex::new(inbound.clone()),
        })
    });
    let st = entry.clone();
    // generation 起始 0，本次入站统一 +1 → 首条 runner 起跑快照见到 1。
    let prev_gen = st.generation.fetch_add(1, Ordering::AcqRel);
    st.deadline_ms.store(deadline, Ordering::Release);
    *st.latest_inbound.lock() = inbound;
    let spawned_now = prev_gen == 0;
    (st, spawned_now)
}

/// 去抖 runner 主体：等用户说完（deadline 到）→ 快照 generation + 最新入站 →
/// reload contact（非 managed 则退休）→ 一次反应分析 + 一次聚合网关（带抢占
/// guard）→ 若期间有新入站则重算，否则原子退休。
pub async fn run_debounce_pipeline(
    state: AppState,
    key: String,
    st: Arc<PendingState>,
    account_id: String,
    from_wxid: String,
    app_id: Option<String>,
) {
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let state_for_panic = state.clone();
    let account_for_panic = account_id.clone();
    let wxid_for_panic = from_wxid.clone();
    let app_for_panic = app_id.clone();
    let key_for_panic = key.clone();

    let inner = async move {
        loop {
            // (a) 去抖睡眠——可被后到入站刷新 deadline 反复重置。
            loop {
                let now = now_ms();
                let dl = st.deadline_ms.load(Ordering::Acquire);
                if now >= dl {
                    break;
                }
                let wait = (dl - now).max(0) as u64;
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            }

            // (b) 快照本轮 generation + 最新入站（锁立即释放，绝不跨 .await）。
            let gen_at_start = st.generation.load(Ordering::Acquire);
            let inbound = st.latest_inbound.lock().clone();

            // (c) reload contact——窗口期可能转 unmanaged / 被删，早退。
            let contact = match reload_managed_contact(&state, &from_wxid, &account_id).await {
                Ok(Some(c)) => c,
                Ok(None) => {
                    PENDING.remove(&key);
                    return;
                }
                Err(error) => {
                    let _ = agent::write_event_for_account(
                        &state,
                        &account_id,
                        Some(&from_wxid),
                        "agent_error",
                        "failed",
                        &format!("debounce reload contact failed: {error}"),
                        app_id.clone().map(|v| doc! { "app_id": v }),
                    )
                    .await;
                    PENDING.remove(&key);
                    return;
                }
            };

            // (d) 一次反应分析（每串只在最新入站上跑一次 → 串行化，修反应写竞态）。
            if let Err(error) = agent::record_user_reaction(&state, &contact, &inbound).await {
                let _ = agent::write_event_for_account(
                    &state,
                    &account_id,
                    Some(&from_wxid),
                    "agent_error",
                    "failed",
                    &format!("record_user_reaction failed: {error}"),
                    app_id.clone().map(|v| doc! { "app_id": v }),
                )
                .await;
            } else {
                // (e) 一次聚合网关，带协作式抢占 guard：运行期间 generation 变了即放弃。
                let guard_state = st.clone();
                let guard: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
                    barge_in_triggered(gen_at_start, guard_state.generation.load(Ordering::Acquire))
                });
                if let Err(error) = agent::handle_managed_message_aggregated(
                    &state,
                    contact,
                    &inbound,
                    Some(guard),
                )
                .await
                {
                    let _ = agent::write_event_for_account(
                        &state,
                        &account_id,
                        Some(&from_wxid),
                        "agent_error",
                        "failed",
                        &error.to_string(),
                        app_id.clone().map(|v| doc! { "app_id": v }),
                    )
                    .await;
                }
            }

            // (f) 运行期间有新入站 → 重算（deadline 已被 register_inbound 刷新过）。
            if barge_in_triggered(gen_at_start, st.generation.load(Ordering::Acquire)) {
                continue;
            }

            // (g) 原子退休：谓词在 shard 锁内复核 generation 未变才移除；若晚到
            // 入站刚 bump 过 generation，谓词失败 → 不移除 → 回 loop 重算。
            if PENDING
                .remove_if(&key, |_, s| {
                    s.generation.load(Ordering::Acquire) == gen_at_start
                })
                .is_some()
            {
                return;
            }
        }
    };

    if let Err(panic_payload) = AssertUnwindSafe(inner).catch_unwind().await {
        // runner panic：写事件 + 移除 state，下条入站会重 spawn。一次 panic 最多
        // 丢在途这一串（与旧 per-webhook spawn 同爆炸半径）。
        PENDING.remove(&key_for_panic);
        let panic_msg = panic_payload_message(&panic_payload);
        tracing::error!(
            account_id = %account_for_panic,
            wxid = %wxid_for_panic,
            panic = %panic_msg,
            "debounce pipeline panicked"
        );
        let _ = agent::write_event_for_account(
            &state_for_panic,
            &account_for_panic,
            Some(&wxid_for_panic),
            "webhook_handler_panic",
            "warning",
            &format!("debounce pipeline panicked: {panic_msg}"),
            app_for_panic.map(|v| doc! { "app_id": v }),
        )
        .await;
    }
}

/// reload contact 并判定是否仍 managed。返回 `Ok(None)` 表示不存在或已非 managed
/// （runner 应退休，只持久化不应答）。
async fn reload_managed_contact(
    state: &AppState,
    wxid: &str,
    account_id: &str,
) -> AppResult<Option<Contact>> {
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! { "account_id": account_id, "wxid": wxid },
            None,
        )
        .await?;
    Ok(contact.filter(|c| c.agent_status == AgentStatus::Managed))
}


pub async fn wechat_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Json<Value>> {
    // 方案 B 验签在下方「解析 appId、查到账号密钥之后」进行（fail-closed 全路径验签），
    // 见 resolve_account_context 之后的 webhook_verify_signature 块。此处仅解析 body。
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("invalid json body: {}", e)))?;

    // GeWe 控制事件不喂 Agent，立刻 200 返回，保证 MCP 那边 5s timeout 内收到 ack。
    // 方案 B（fail-closed）下按「是否产生副作用」分两处放置：
    // (a) `testMsg` 探活无副作用 → 留在验签门之前直接 ack（GeWe 控制台「测试回调」按钮用）。
    // (b) `TypeName=Offline/Online` 落库 `online`（供 outbox dispatcher 发送前 gate，掉线
    //     defer 不盲发）有副作用 → 下沉到验签门之后（见 resolve_account_context 之后）。
    if let Some(test_msg) = find_string(&payload, &["testMsg", "TestMsg"]) {
        return Ok(Json(serde_json::json!({
            "ok": true,
            "ignored": "callback_test",
            "echo": test_msg
        })));
    }

    // P2：MCP（GeWe-agent）转发的 payload 是 GeWe 原始 body 直接透传 + 顶层加
     // 一个 `_mcp` envelope（tenantId/accountId/sourceMsgId 等）。GeWe 字段一般是
     // 大写驼峰（`Appid` / `Wxid` / `FromUserName` / `Content` / `MsgId` / `NewMsgId`
     // / `TypeName` / `ToUserName`），少量小写驼峰（`appId` / `fromWxid`），所以
     // find_string 的 keys 必须同时覆盖两种风格。`_mcp.appId` 也算一份兜底。
    let app_id = find_string(
        &payload,
        &["appId", "app_id", "appid", "Appid", "AppId", "APPID"],
    );
    let (workspace_id, account_id, webhook_secret) =
        match resolve_account_context(&state, app_id.as_deref()).await {
            Ok(triple) => triple,
            Err(AppError::BadRequest(msg)) => {
                // P1：未知 appId 不再静默回退到 default account_id；写一条 admin-visible
                // 事件后明确 400，让运维侧能看到「webhook 入站但无对应 account」。
                let _ = emit_unknown_app_id_event(&state, app_id.as_deref()).await;
                return Err(AppError::BadRequest(msg));
            }
            Err(other) => return Err(other),
        };

    // 方案 B 验签门（fail-closed）：签名开关打开时，任何副作用之前必须验签通过。
    // 校验 gewe-agent 每账号 x-webhook-signature + x-webhook-timestamp 时效。
    if state.config.webhook_verify_signature {
        let now_ms = DateTime::now().timestamp_millis();
        if let Err(reason) = verify_webhook_signature(
            webhook_secret.as_deref(),
            headers
                .get("x-webhook-timestamp")
                .and_then(|v| v.to_str().ok()),
            headers
                .get("x-webhook-signature")
                .and_then(|v| v.to_str().ok()),
            &body,
            now_ms,
            state.config.webhook_timestamp_skew_seconds,
        ) {
            tracing::warn!(
                ?reason,
                account_id = %account_id,
                body_len = body.len(),
                "webhook rejected: signature verification failed"
            );
            return Err(AppError::BadRequest("invalid signature".into()));
        }
    }

    // (b) `TypeName=Offline/Online`：账号在线状态事件，落库 `online` 建状态源（供 outbox
    //     dispatcher 发送前 gate，掉线 defer 不盲发）。写 online 有副作用，必须在验签门之后。
    if let Some(type_name) = find_string(&payload, &["TypeName", "typeName"]) {
        let lower = type_name.to_ascii_lowercase();
        if lower == "offline" || lower == "online" {
            let online = lower == "online";
            if let Some(app_id) = app_id.as_deref() {
                // fail-soft：状态落库失败不应让 MCP 侧收不到 ack（会触发重推）。
                let res = state
                    .db
                    .accounts()
                    .update_one(
                        doc! { "app_id": app_id },
                        doc! { "$set": { "online": online, "last_sync_at": DateTime::now() } },
                        None,
                    )
                    .await;
                if let Err(err) = res {
                    tracing::warn!(?err, app_id, online, "persist account online state failed");
                }
            }
            return Ok(Json(serde_json::json!({
                "ok": true,
                "ignored": if online { "online_event" } else { "offline_event" },
                "type": type_name
            })));
        }
    }

    // LP-14 / Task 20：per-account_id 限流；超额返回 429。
    let limiter = limiter_for(
        &account_id,
        state.config.webhook_rate_limit_capacity,
        state.config.webhook_rate_limit_window_seconds,
    );
    if let Err(neg) = limiter.check() {
        let retry_after = neg.wait_time_from(DefaultClock::default().now()).as_secs() + 1;
        let _ = maybe_emit_rate_limit_event(&state, &account_id).await;
        return Err(AppError::RateLimited {
            retry_after,
            account_id,
        });
    }

    let from_wxid = gewe_data_string(&payload, "FromUserName")
        .or_else(|| {
            find_string(
                &payload,
                &[
                    // 小写驼峰（手工 / 自测 / 部分推送）
                    "fromWxid",
                    "from_wxid",
                    "fromUserName",
                    "from_user_name",
                    "fromusername",
                    "from",
                    // GeWe 大写驼峰（MCP 透传的真实推送主字段）
                    "FromUserName",
                    "FromWxid",
                    "Wxid",
                ],
            )
        })
        .ok_or_else(|| AppError::BadRequest("webhook missing sender wxid".to_string()))?;
    let content = gewe_data_string(&payload, "Content")
        .or_else(|| {
            find_string(
                &payload,
                &[
                    // 小写驼峰
                    "content",
                    "text",
                    "msgContent",
                    "msg_content",
                    "message",
                    "messageContent",
                    // GeWe 大写驼峰
                    "Content",
                    "PushContent",
                ],
            )
        })
        .unwrap_or_default();
    // 领导回复分流：from_wxid 是本 workspace 的 principal_decider → 走请示通道，不进客户链路。
    // 必须在落库 / contact-managed 处理之前分流——领导可能同时也是某 contact，
    // consumed=true 时短路返回，避免领导自己的消息被当成客户入站处理。
    if (crate::agent::escalation::lookup_principal_config(&state, &workspace_id, &from_wxid).await?)
        .is_some()
    {
        let consumed = crate::agent::escalation::handle_principal_reply(
            &state,
            &workspace_id,
            &account_id,
            &from_wxid,
            &content,
        )
        .await?;
        if consumed {
            return Ok(Json(serde_json::json!({ "ok": true, "routed": "principal" })));
        }
    }
    let message_id = find_string(
        &payload,
        &[
            // 小写驼峰
            "newMsgId",
            "new_msg_id",
            "msgId",
            "msg_id",
            "messageId",
            "id",
            // GeWe 大写驼峰
            "NewMsgId",
            "MsgId",
            "MessageId",
        ],
    );
    // P2：dedupe key 优先用 GeWe sourceMsgId（MCP 那边按
     // `${slot.id}:${appId}:${sourceMsgId}` 做转发去重，且 5s timeout 内不重试，
     // 单次推送绝不能丢）。也兼顾 _mcp envelope 里冗余的 sourceMsgId / msgId
     // 字段，万一 GeWe 顶层 MsgId 缺失仍能正确去重。
    let envelope_msg_id = payload
        .get("_mcp")
        .and_then(|env| env.get("sourceMsgId"))
        .and_then(value_to_string);
    let effective_message_id = message_id.clone().or(envelope_msg_id);
    let dedupe_key = effective_message_id
        .as_ref()
        .map(|id| format!("message:{id}"))
        .unwrap_or_else(|| format!("payload:{}", stable_payload_hash(&payload)));

    // P0-19：dedupe 原子化。原 check-then-insert 存在 TOCTOU 竞态，两个并发
    // webhook 的 find_one 都可能返回 None，导致同一条入站消息被双写。改为
    // 直接 insert_one + 捕获 11000 duplicate key 错误（依赖
    // db/indexes.rs:55-63 的 partial unique index `workspace_id+account_id+dedupe_key`），
    // 让 MongoDB 在写入时原子去重。
    let raw = to_document(&payload).ok();
    // F1：解析入站消息类型 + 媒体引用，不再写死 None。
    let msg_type = parse_inbound_msg_type(&payload);
    let media_ref = extract_inbound_media_ref(&payload, msg_type);
    let inbound = ConversationMessage {
        id: None,
        workspace_id: workspace_id.clone(),
        account_id: account_id.clone(),
        contact_wxid: from_wxid.clone(),
        message_id: effective_message_id.clone(),
        dedupe_key: Some(dedupe_key.clone()),
        direction: MessageDirection::Inbound,
        content,
        msg_type: Some(msg_type.to_string()),
        media_ref,
        raw,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    };
    match state.db.messages().insert_one(&inbound, None).await {
        Ok(_) => {}
        Err(error) if is_duplicate_key_error(&error) => {
            return Ok(Json(serde_json::json!({ "ok": true, "duplicate": true })));
        }
        Err(error) => return Err(error.into()),
    }

    let mut contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": &account_id,
                "wxid": &from_wxid
            },
            None,
        )
        .await?;

    if contact.is_none() {
        contact = upsert_webhook_contact(&state, &workspace_id, &account_id, &from_wxid, &payload)
            .await?;
    }

    let Some(contact) = contact else {
        return Err(AppError::External("failed to create contact".to_string()));
    };

    let now = DateTime::now();
    // S1（自学习采集管道）：在 contact 的 last_inbound_at / last_outbound_at 被本轮
    // 更新覆盖之前，先快照出"上一条入站 / 上一条出站"时间，用于构造 T1 行为信号
    // （reply_latency / reactivation）。采集是 best-effort 旁路，绝不阻断应答。
    let prev_last_inbound_ms = contact.last_inbound_at.map(|d| d.timestamp_millis());
    let prev_last_outbound_ms = contact.last_outbound_at.map(|d| d.timestamp_millis());
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact.id },
            doc! {
                "$set": {
                    "last_inbound_at": now,
                    "last_message_at": now,
                    "updated_at": now
                }
            },
            None,
        )
        .await?;

    // S1：落 T1 行为信号（观察层，不解释、不评分）。每条带 dedupe_key，重复
    // webhook / 重放只落一次。任何一段失败仅 warn，不影响后续 Agent 应答。
    collect_inbound_behavior_signals(
        &state,
        &workspace_id,
        &from_wxid,
        effective_message_id.as_deref(),
        &inbound.content,
        now,
        prev_last_inbound_ms,
        prev_last_outbound_ms,
    )
    .await;

    // P2：MCP（GeWe-agent）那一侧 fetch(messageWebhookUrl) 用了 5s AbortController
    // timeout 且失败不重试。Agent 决策 + Review 流水线一次约 10–15s，远超
    // 5s，必须把它挪到后台 spawn，主请求落库后立即 ack。
    //
    // 并发多消息去抖：不再每条 webhook 直接 spawn 一条流水线，而是注册到按联系人
    // 的去抖调度器。已有 runner 时只刷新 deadline + bump generation（不 spawn）；
    // 没有时插入状态并 spawn 一个 runner。runner 等去抖窗口到再跑一次聚合流水线，
    // 运行期间到的新消息会触发抢占重算（见 run_debounce_pipeline）。
    let managed = contact.agent_status == AgentStatus::Managed;
    let mut deferred = false;
    if managed {
        // #69 作息门控：静默时段（运营方进程本地时区）客户来消息时**不立即回**，
        // 排一条 deferred_inbound_reply 跟进任务到醒来时刻。inbound 已在上面落库，
        // 醒来时 gateway 的 load_recent_messages 会天然聚合这段时间的全部消息一次性回。
        // 开关/时段来自运营域配置（RuntimeParametersTyped，前端可改），默认启用。
        let domain_config = agent::load_user_operation_domain_config_for_contact(
            &state,
            &workspace_id,
            &contact.id.map(|id| id.to_hex()).unwrap_or_default(),
        )
        .await?;
        let runtime =
            crate::agent::UserRuntimeParameters::from_config(domain_config.as_ref(), &state);
        let active_profile =
            agent::domain_profile::load_active_domain_profile(&state.db, &workspace_id).await;
        let quiet = agent::quiet_hours::effective_quiet_hours_enabled(
            &contact,
            &active_profile,
            runtime.quiet_hours_enabled,
        ) && agent::quiet_hours::is_quiet_now(
            runtime.quiet_hours_start,
            runtime.quiet_hours_end,
            runtime.quiet_hours_tz_offset_hours,
        );
        if quiet {
            ensure_wake_followup_task(
                &state,
                &contact,
                runtime.quiet_hours_end,
                runtime.quiet_hours_tz_offset_hours,
            )
            .await?;
            deferred = true;
        } else {
            let key = contact_key(&workspace_id, &account_id, &from_wxid);
            let active_profile = crate::agent::domain_profile::load_active_domain_profile(
                &state.db, &workspace_id,
            ).await;
            let window_ms = crate::agent::domain_profile::resolve_debounce_window_ms(
                &active_profile, state.config.message_debounce_window_ms,
            );
            let (st, spawned_now) = register_inbound(key.clone(), inbound.clone(), window_ms);
            if spawned_now {
                let bg_state = state.clone();
                let bg_account_id = account_id.clone();
                let bg_from_wxid = from_wxid.clone();
                let bg_app_id = app_id.clone();
                tokio::spawn(async move {
                    run_debounce_pipeline(
                        bg_state,
                        key,
                        st,
                        bg_account_id,
                        bg_from_wxid,
                        bg_app_id,
                    )
                    .await;
                });
            }
        }
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "managed": managed,
        "queued": managed && !deferred,
        "deferred": deferred
    })))
}

/// #69 作息门控：静默时段入站时，确保存在一条"醒来回复"跟进任务。
///
/// kind = [`agent::quiet_hours::DEFERRED_INBOUND_REPLY_KIND`]，与 planner 主动催进的
/// `follow_up` 区分——precheck 据此豁免 `context_changed`（这条任务的存在意义恰恰
/// 就是回 task 创建后累积的客户消息）。`run_at` = 下一次醒来时刻；醒来后由 task
/// worker → handle_follow_up_task → gateway 走完整决策/审查/拆短/outbox 链路。
///
/// 去重：仿 planner `has_pending_follow_up` —— 同 contact 已有未终态的 wake 任务则
/// 不再插（静默时段连发多条 → 1 task → 醒来基于累积消息回 1 次）。先查后插存在
/// TOCTOU 窗口，但 precheck 的 rate_limited 闸在醒来时会兜住重复触达，可接受。
///
/// `pub`：暴露给 tests/quiet_hours_deferral.rs 集成测试直接驱动排程链路
/// （`Utc::now` 不可注入，集成测试只验 DB 写入 + 去重 + 埋点，时区由纯函数单测覆盖）。
pub async fn ensure_wake_followup_task(
    state: &AppState,
    contact: &Contact,
    wake_hour: u32,
    tz_offset_hours: i32,
) -> AppResult<()> {
    let existing = state
        .db
        .tasks()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "kind": agent::quiet_hours::DEFERRED_INBOUND_REPLY_KIND,
                "status": { "$in": ["pending", "retry", "running"] },
            },
            None,
        )
        .await?;
    if existing > 0 {
        return Ok(());
    }
    let now = DateTime::now();
    let run_at = agent::quiet_hours::next_wake_at(wake_hour, tz_offset_hours, &contact.wxid, state.config.wake_jitter_max_seconds);
    // expiry 给 24h 余量（覆盖最长跨午夜窗口 + 醒来后 worker tick 间隔），过期未跑则作废。
    let expires_at = DateTime::from_millis(run_at.timestamp_millis() + 24 * 60 * 60 * 1000);
    let task = AgentTask {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        kind: agent::quiet_hours::DEFERRED_INBOUND_REPLY_KIND.to_string(),
        run_at,
        expires_at: Some(expires_at),
        content: "作息时段累积消息，醒来后基于完整上下文回复".to_string(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: true,
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
    state.db.tasks().insert_one(task, None).await?;
    // 观测埋点：仅真正新建 wake task 时写一条 deferred 事件，运营后台据此看到
    // "这条为何没秒回"。dedup 命中（上面 early-return）不写，避免静默连发刷屏。
    // best-effort：失败只吞掉，绝不阻断 webhook ack。
    let _ = agent::write_event_for_account(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        "quiet_hours_deferred_inbound",
        "deferred",
        "作息时段，客户消息延迟到醒来时刻回复",
        Some(doc! {
            "wakeAt": run_at,
            "kind": agent::quiet_hours::DEFERRED_INBOUND_REPLY_KIND,
            "tzOffsetHours": tz_offset_hours,
        }),
    )
    .await;
    Ok(())
}

fn stable_payload_hash(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// 判定 mongodb 错误是否为 DuplicateKey（code 11000 / 11001）。
/// 与 `agent::outbox::is_duplicate_key_error` 同语义；不跨 mod 复用以避免
/// webhook 反向依赖 agent 内部 helper。
/// S1（自学习采集管道）：落本条入站对应的 T1 行为信号（best-effort 旁路）。
///
/// 在 contact 的 last_* 时间戳被本轮覆盖之前由调用方快照 `prev_*_ms` 传入。
/// 缺 `message_id` 时退化用 `observed_at` 毫秒作为 dedupe 后缀——保证仍幂等
/// （同一时刻的同 contact 不会重复落），但跨重放去重精度略降。
///
/// 任何一段失败只 `warn`，绝不向上抛——采集出错不能拖累用户应答。
#[allow(clippy::too_many_arguments)]
async fn collect_inbound_behavior_signals(
    state: &AppState,
    workspace_id: &str,
    wxid: &str,
    message_id: Option<&str>,
    content: &str,
    inbound_at: DateTime,
    prev_last_inbound_ms: Option<i64>,
    prev_last_outbound_ms: Option<i64>,
) {
    use crate::behavior_signals as bs;
    let dedupe_suffix = message_id
        .map(ToString::to_string)
        .unwrap_or_else(|| inbound_at.timestamp_millis().to_string());

    let mut signals = vec![
        bs::build_reply_latency(
            workspace_id,
            wxid,
            &dedupe_suffix,
            inbound_at,
            prev_last_outbound_ms,
        ),
        bs::build_reply_length(workspace_id, wxid, &dedupe_suffix, inbound_at, content),
    ];
    if bs::is_reactivation(prev_last_inbound_ms, inbound_at, bs::REACTIVATION_THRESHOLD_MS) {
        signals.push(bs::build_reactivation(
            workspace_id,
            wxid,
            &dedupe_suffix,
            inbound_at,
        ));
    }

    for signal in signals {
        let signal_type = signal.signal_type.clone();
        let result = bs::persist_signal(state, signal).await;
        bs::record_signal_metric(state, workspace_id, &result).await;
        if let Err(error) = result {
            tracing::warn!(
                error = %error,
                wxid = %wxid,
                signal_type = %signal_type,
                "behavior_signal persist failed (best-effort, ignored)"
            );
        }
    }
}

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

/// 把 panic payload 解析成可读字符串。与 supervisor::panic_payload_to_string
/// 同语义；不跨 mod 复用以保持 webhook 模块 self-contained。
fn panic_payload_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(value_to_string) {
                    return Some(found);
                }
            }
            for child in map.values() {
                if let Some(found) = find_string(child, keys) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_string(item, keys)),
        _ => None,
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// 从 GeWe AddMsg 的 `Data.<field>.string` 取字符串。真实推送里发件人/内容都是
/// `{string:...}` 包裹且嵌在 `Data` 下——通用 find_string 会被顶层同名/近义键
/// (`Wxid` / `PushContent`)遮蔽,故对 GeWe 形态显式走此路径,优先于 find_string。
/// 取不到返回 None(交调用方回落 find_string)。命中空串返回 Some("")——刻意直接
/// 用空内容,不回落到带发件人名前缀的 PushContent 通知串。
fn gewe_data_string(payload: &Value, field: &str) -> Option<String> {
    payload
        .get("Data")
        .and_then(|d| d.get(field))
        .and_then(|f| f.get("string"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

/// 从 GeWe AddMsg 的 `Data.MsgType.low` 取微信消息类型数字码(`{low:N}` 包裹)。
/// 返回数字的字符串形式(交 classify_inbound_msg_type 归一)。取不到返回 None。
fn gewe_data_msg_type_code(payload: &Value) -> Option<String> {
    payload
        .get("Data")
        .and_then(|d| d.get("MsgType"))
        .and_then(|m| m.get("low"))
        .and_then(|n| n.as_i64())
        .map(|n| n.to_string())
}

/// F1 评审 I1：从入站 payload 解析归一化的消息类型。候选键**仅限“消息类型”语义
/// 专用键**——GeWe 大写驼峰 `MsgType`（微信数字码真实字段）+ 手工/自测 payload 的
/// 小写别名 `msgType`/`msg_type`。**刻意不收泛化裸键 `type`/`Type`**：`find_string`
/// 深度递归整棵 JSON（含 `_mcp` envelope 及任意嵌套对象），而 webhook envelope 里
/// 与消息类型无关的 `{"type":"event",...}` 极常见，泛化键会被误命中、把本应默认
/// `text` 的纯文本消息误标为非文本，破坏 text 主链路。真实字段就是 `MsgType`，删掉
/// `type`/`Type` 既消除误伤面又不漏真实字段。
///
/// payload 无任何类型字段时默认 `"text"`（runbook W1-W3 等纯文本自测 payload 不带
/// 类型字段，行为不变）；有则交给 `classify_inbound_msg_type` 归一（未知码 → `"unknown"`）。
fn parse_inbound_msg_type(payload: &Value) -> &'static str {
    let raw_msg_type = gewe_data_msg_type_code(payload)
        .or_else(|| find_string(payload, &["msgType", "msg_type", "MsgType"]));
    match raw_msg_type.as_deref() {
        Some(raw_type) => classify_inbound_msg_type(raw_type),
        None => "text",
    }
}

/// 把 webhook payload 里的原始消息类型（GeWe 透传的微信 `MsgType` 数字码，或
/// 手工/自测 payload 的字符串别名）归一化为稳定的 `msg_type` 字符串。F1 地基：
/// 让非文本入站可被识别（图片/语音/视频/名片/链接卡片等），不再被当空文本硬答。
///
/// 未知类型一律归 `"unknown"`——**绝不崩、绝不当 text**，下游据此走非文本分支
/// （F2 才做媒体理解/过渡话术，本函数只负责识别归类）。
///
/// 微信协议私聊 MsgType 数字码：1=文本 3=图片 34=语音 43=视频 42=名片
/// 47=表情 48=位置 49=appmsg(链接/文件/小程序) 50=语音/视频通话 51=状态同步
/// 10000/10002=系统消息。GeWe 私聊真实非文本入站 payload 仓内暂无确认样例，
/// 数字码以微信协议为准；新码值落 `"unknown"` 而非误判，安全侧。
fn classify_inbound_msg_type(raw: &str) -> &'static str {
    match raw.trim() {
        "1" | "text" | "Text" => "text",
        "3" | "image" | "Image" | "img" => "image",
        "34" | "voice" | "Voice" => "voice",
        "43" | "video" | "Video" => "video",
        "42" | "namecard" | "card" => "namecard",
        "47" | "emoji" | "sticker" => "emoji",
        "48" | "location" => "location",
        "49" | "appmsg" | "link" | "file" | "miniprogram" => "appmsg",
        "50" | "voip" => "voip",
        "51" => "statussync",
        "10000" | "10002" | "sysmsg" | "system" => "system",
        _ => "unknown",
    }
}

/// 从入站 payload 提取媒体引用（图片 cdn url / 文件 id / 语音 path 等），供后续
/// 多模态理解链路（F2）定位媒体内容。`text` 消息恒返回 None。
///
/// GeWe 富媒体引用通常嵌在 `Content` 的 XML 里（`cdnurl`/`aeskey`）或独立 media
/// 字段；仓内暂无确认的非文本入站样例，故此处只从已知候选字段名尽力提取一个可
/// 定位引用，找不到返回 None（**不崩、不造假**）。F2 接通 MCP 媒体下载后再补全。
fn extract_inbound_media_ref(payload: &Value, msg_type: &str) -> Option<String> {
    if msg_type == "text" {
        return None;
    }
    find_string(
        payload,
        &[
            // 小写驼峰（自测/手工）
            "mediaUrl",
            "media_url",
            "fileUrl",
            "file_url",
            "cdnUrl",
            "cdn_url",
            "cdnurl",
            "mediaId",
            "media_id",
            "fileId",
            "file_id",
            // GeWe 大写驼峰
            "MediaUrl",
            "FileUrl",
            "CdnUrl",
            "MediaId",
            "FileId",
        ],
    )
}

async fn resolve_account_context(
    state: &AppState,
    app_id: Option<&str>,
) -> AppResult<(String, String, Option<String>)> {
    if let Some(app_id) = app_id {
        if let Some(account) = state
            .db
            .accounts()
            .find_one(doc! { "app_id": app_id }, None)
            .await?
        {
            // 第三元 = 该账号 webhook_secret，供方案 B 验签门使用。
            return Ok((account.workspace_id, account.account_id, account.webhook_secret));
        }
        // P1：appId 提供了但 wechat_accounts 没匹配 —— 之前会静默回退到
        // default_account_id，导致 inbound 落到错的 account 下，managed contact
        // 永远 lookup 不到，AI 不回复。改成显式 400，让 webhook 侧能看到。
        return Err(AppError::BadRequest(format!(
            "webhook appId {app_id} not registered in wechat_accounts"
        )));
    }
    Ok((
        state.config.default_workspace_id.clone(),
        state.config.default_account_id.clone(),
        None,
    ))
}

/// P1：webhook 收到未知 appId 时写一条 admin-visible 事件，便于运维诊断
/// 「inbound 200 但 contact 不存在 / managed 不工作」类问题。
async fn emit_unknown_app_id_event(state: &AppState, app_id: Option<&str>) -> AppResult<()> {
    let summary = match app_id {
        Some(id) => format!("webhook 入站 appId={id} 在 wechat_accounts 中未注册，已拒收"),
        None => "webhook 入站缺失 appId 字段，已按 default account 处理".to_string(),
    };
    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: state.config.default_account_id.clone(),
                contact_wxid: None,
                kind: "webhook_unknown_app_id".to_string(),
                status: "rejected".to_string(),
                summary,
                details: app_id.map(|id| doc! { "app_id": id }),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;
    Ok(())
}

async fn upsert_webhook_contact(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
    payload: &Value,
) -> AppResult<Option<Contact>> {
    let nickname = find_string(payload, &["nickName", "nickname", "fromNickName"]);
    // P1：兜底 —— 如果同 (workspace_id, wxid) 已有 managed 记录在另一个
    // account_id 下，本次 inbound 与 managed contact 出现 account_id 错配，
    // 写一条 admin-visible 事件提醒（不创建影子副本会更激进，留给后续 PR）。
    if let Some(existing) = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "wxid": wxid,
                "agent_status": "managed"
            },
            None,
        )
        .await?
    {
        if existing.account_id != account_id {
            let _ = state
                .db
                .events()
                .insert_one(
                    crate::models::AgentEvent {
                        id: None,
                        workspace_id: workspace_id.to_string(),
                        account_id: account_id.to_string(),
                        contact_wxid: Some(wxid.to_string()),
                        kind: "webhook_managed_contact_account_mismatch".to_string(),
                        status: "warning".to_string(),
                        summary: format!(
                            "同一 wxid 在 account={} 下被标记 managed，本次 inbound 落到 account={}，将创建 normal 影子记录，AI 不会自动回复",
                            existing.account_id, account_id
                        ),
                        details: Some(doc! {
                            "managed_account_id": existing.account_id.clone(),
                            "inbound_account_id": account_id,
                            "wxid": wxid,
                        }),
                        created_at: DateTime::now(),
                        dedupe_key: None,
                    },
                    None,
                )
                .await;
        }
    }
    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": wxid
            },
            doc! {
                "$set": {
                    "nickname": &nickname,
                    "updated_at": DateTime::now()
                },
                "$setOnInsert": {
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "wxid": wxid,
                    "agent_status": "normal",
                    "created_at": DateTime::now()
                }
            },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": wxid
            },
            None,
        )
        .await
        .map_err(AppError::from)
}

/// P1-2：rate_limit 事件 partial-unique 去重 key。
///
/// 形式 `rate_limit:{account}:{day_bucket}`，`day_bucket = epoch_ms / 86_400_000`。
/// 同一账号在同一 UTC 天最多一条 `webhook_rate_limited` 事件，由 partial unique
/// index `workspace_id + dedupe_key` 在并发下原子约束。
fn rate_limit_event_dedupe_key(account_id: &str, day_bucket: i64) -> String {
    format!("rate_limit:{}:{}", account_id, day_bucket)
}

/// LP-14 / Task 20：限流命中时按 account 当日去重写一条 agent_event，避免事件爆量。
///
/// P1-2：旧实现 `find_one + insert_one` 在并发限流命中时存在 TOCTOU——
/// 两条请求都查到 `None`，都写入，事件爆量。改为携带 `dedupe_key` 原子写：
/// `dedupe_key = "rate_limit:{account}:{day_bucket}"`，配合 partial unique
/// index（`workspace_id + dedupe_key`）让重复 insert 直接命中 dup-key error
/// 后被吞掉；首条写入获胜，后续都视为"今天已记录"。
async fn maybe_emit_rate_limit_event(state: &AppState, account_id: &str) -> AppResult<()> {
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let now_ms = DateTime::now().timestamp_millis();
    let day_bucket = now_ms / day_ms;
    let dedupe_key = rate_limit_event_dedupe_key(account_id, day_bucket);
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "webhook_rate_limited".to_string(),
        status: "blocked".to_string(),
        summary: "webhook 入口触发 per-account 限流".to_string(),
        details: None,
        created_at: DateTime::now(),
        dedupe_key: Some(dedupe_key),
    };
    match state.db.events().insert_one(&event, None).await {
        Ok(_) => Ok(()),
        Err(error) if is_duplicate_key_error(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod inbound_msg_type_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_inbound_msg_type_maps_known_numeric_codes() {
        // GeWe 透传的微信 MsgType 数字码
        assert_eq!(classify_inbound_msg_type("1"), "text");
        assert_eq!(classify_inbound_msg_type("3"), "image");
        assert_eq!(classify_inbound_msg_type("34"), "voice");
        assert_eq!(classify_inbound_msg_type("43"), "video");
        assert_eq!(classify_inbound_msg_type("42"), "namecard");
        assert_eq!(classify_inbound_msg_type("49"), "appmsg");
    }

    #[test]
    fn classify_inbound_msg_type_maps_string_aliases() {
        // 手工/自测 payload 的字符串别名
        assert_eq!(classify_inbound_msg_type("text"), "text");
        assert_eq!(classify_inbound_msg_type("image"), "image");
        assert_eq!(classify_inbound_msg_type("voice"), "voice");
        assert_eq!(classify_inbound_msg_type("video"), "video");
        assert_eq!(classify_inbound_msg_type("link"), "appmsg");
    }

    #[test]
    fn classify_inbound_msg_type_trims_whitespace() {
        assert_eq!(classify_inbound_msg_type(" 3 "), "image");
        assert_eq!(classify_inbound_msg_type("\ttext\n"), "text");
    }

    #[test]
    fn classify_inbound_msg_type_unknown_never_falls_back_to_text() {
        // 未知类型归 unknown：不崩、不当 text（下游据此走非文本分支）
        assert_eq!(classify_inbound_msg_type("某新类型"), "unknown");
        assert_eq!(classify_inbound_msg_type("9999"), "unknown");
        assert_eq!(classify_inbound_msg_type(""), "unknown");
    }

    #[test]
    fn extract_media_ref_is_none_for_text() {
        let payload = json!({ "mediaUrl": "http://x/a.jpg", "content": "hi" });
        assert_eq!(extract_inbound_media_ref(&payload, "text"), None);
    }

    #[test]
    fn extract_media_ref_pulls_known_fields_for_media() {
        let payload = json!({ "fromWxid": "wx1", "cdnUrl": "http://cdn/x.jpg" });
        assert_eq!(
            extract_inbound_media_ref(&payload, "image"),
            Some("http://cdn/x.jpg".to_string())
        );
        let payload2 = json!({ "_mcp": { "MediaId": "media-123" } });
        assert_eq!(
            extract_inbound_media_ref(&payload2, "voice"),
            Some("media-123".to_string())
        );
    }

    #[test]
    fn extract_media_ref_none_when_no_reference_present() {
        // 非文本但 payload 无任何已知媒体引用字段 → None（不造假）
        let payload = json!({ "fromWxid": "wx1", "content": "" });
        assert_eq!(extract_inbound_media_ref(&payload, "image"), None);
    }

    #[test]
    fn parse_inbound_msg_type_uses_dedicated_keys() {
        // a. 专用键正常生效（回归不破）：顶层 MsgType 数字码 + 小写别名
        assert_eq!(parse_inbound_msg_type(&json!({ "MsgType": "3" })), "image");
        assert_eq!(parse_inbound_msg_type(&json!({ "msgType": "voice" })), "voice");
        assert_eq!(parse_inbound_msg_type(&json!({ "msg_type": "43" })), "video");
    }

    #[test]
    fn parse_inbound_msg_type_ignores_unrelated_nested_type_fields() {
        // b. 核心回归（I1）：payload 无任何类型字段，但嵌套对象带与消息类型无关的
        // type/Type（webhook envelope 极常见，如 {"type":"event"}）。泛化键已删，
        // find_string 深度递归不再误命中 → 仍按 text 处理，text 主链路一字不变。
        let payload = json!({
            "fromWxid": "wx1",
            "content": "你好，在吗",
            "_mcp": { "type": "event", "meta": { "Type": "callback" } },
        });
        assert_eq!(parse_inbound_msg_type(&payload), "text");

        // 顶层直接带无关 type 也不被误命中
        let payload2 = json!({ "type": "event", "content": "纯文本" });
        assert_eq!(parse_inbound_msg_type(&payload2), "text");
    }

    #[test]
    fn parse_inbound_msg_type_defaults_text_for_plain_payload() {
        // c. 仿 runbook 自测纯文本 payload（不带任何类型字段）→ text（主链路不变）
        let payload = json!({
            "appId": "wx_app_1",
            "fromWxid": "wxid_customer",
            "content": "我想了解一下你们的产品",
        });
        assert_eq!(parse_inbound_msg_type(&payload), "text");
    }

    #[test]
    fn gewe_addmsg_extracts_msg_type_from_data_low() {
        // MsgType.low=3 → image。
        let payload = json!({
            "TypeName": "AddMsg",
            "Data": {
                "FromUserName": { "string": "wxid_x" },
                "Content": { "string": "x" },
                "MsgType": { "low": 3 }
            }
        });
        assert_eq!(gewe_data_msg_type_code(&payload).as_deref(), Some("3"));
        assert_eq!(parse_inbound_msg_type(&payload), "image");
        // MsgType.low=1 → text(真实文本入站)。
        let text_payload = json!({ "Data": { "MsgType": { "low": 1 } } });
        assert_eq!(parse_inbound_msg_type(&text_payload), "text");
    }

    fn real_gewe_addmsg() -> serde_json::Value {
        // 2026-07-09 线上 117 亲验的真实 GeWe AddMsg 形态(经 gewe-agent 转发):
        // 顶层大写驼峰 + Data 嵌套 + {string}/{low} 包裹 + _mcp envelope。
        json!({
            "Wxid": "wxid_3yeirsb75afd22",
            "TypeName": "AddMsg",
            "Appid": "wx_WSHYpbq5Fdp_yGcOEl9Pn",
            "Data": {
                "FromUserName": { "string": "wxid_ydzaomn4scsb12" },
                "ToUserName": { "string": "wxid_3yeirsb75afd22" },
                "Content": { "string": "你好" },
                "MsgType": { "low": 1 },
                "PushContent": "吴界 : 你好",
                "NewMsgId": { "high": 1976706754, "low": 1032436816 }
            },
            "_mcp": { "event": "wechat.message.created", "sourceMsgId": "8489890863244754000" }
        })
    }

    #[test]
    fn gewe_addmsg_extracts_real_sender_not_account_self() {
        let payload = real_gewe_addmsg();
        // 修复:显式走 Data.FromUserName.string 拿真实发件人(吴界)。
        assert_eq!(
            gewe_data_string(&payload, "FromUserName").as_deref(),
            Some("wxid_ydzaomn4scsb12")
        );
        // 回归留证:通用 find_string 会被顶层 Wxid 遮蔽 → 归错成账号自己。
        // 这正是本次修复的 bug,保留断言防止有人把提取改回纯 find_string。
        assert_eq!(
            find_string(&payload, &["fromWxid", "FromUserName", "FromWxid", "Wxid"]).as_deref(),
            Some("wxid_3yeirsb75afd22")
        );
    }

    #[test]
    fn gewe_addmsg_extracts_clean_content_not_pushcontent() {
        let payload = real_gewe_addmsg();
        // 修复:Data.Content.string 拿干净正文。
        assert_eq!(gewe_data_string(&payload, "Content").as_deref(), Some("你好"));
        // 回归留证:find_string 会先命中 Data.PushContent 通知串(带发件人名前缀)。
        assert_eq!(
            find_string(&payload, &["content", "Content", "PushContent"]).as_deref(),
            Some("吴界 : 你好")
        );
    }

    #[test]
    fn flat_payload_still_parses_via_fallback() {
        // 扁平自测/biz-test payload 无 Data → helper 返 None → 走 find_string 回落,行为不变。
        let payload = json!({ "fromWxid": "wx_flat", "content": "hello flat" });
        assert_eq!(gewe_data_string(&payload, "FromUserName"), None);
        assert_eq!(gewe_data_string(&payload, "Content"), None);
        assert_eq!(find_string(&payload, &["fromWxid"]).as_deref(), Some("wx_flat"));
        assert_eq!(find_string(&payload, &["content"]).as_deref(), Some("hello flat"));
    }
}

#[cfg(test)]
mod debounce_tests {
    use super::*;

    #[test]
    fn contact_key_is_workspace_account_wxid() {
        assert_eq!(contact_key("ws", "acct", "wx1"), "ws:acct:wx1");
    }

    #[test]
    fn next_deadline_adds_window() {
        assert_eq!(next_deadline_ms(1_000, 4_000), 5_000);
        assert_eq!(next_deadline_ms(0, 1_000), 1_000);
    }

    #[test]
    fn next_deadline_saturates_instead_of_overflow() {
        // 饱和加：i64::MAX + window 不应回绕成负数（否则 runner 立即认为已过期）。
        assert_eq!(next_deadline_ms(i64::MAX, 4_000), i64::MAX);
        assert_eq!(next_deadline_ms(i64::MAX - 1, 4_000), i64::MAX);
    }

    #[test]
    fn barge_in_triggers_only_on_generation_change() {
        // generation 未变 → 无新入站 → 不抢占。
        assert!(!barge_in_triggered(3, 3));
        // generation 变了 → 期间有新入站 → 抢占重算。
        assert!(barge_in_triggered(3, 4));
        assert!(barge_in_triggered(0, 1));
    }

    #[test]
    fn register_first_inbound_spawns_then_subsequent_only_bump() {
        // 用唯一 key 避免与其它测试共享全局 PENDING。
        let key = "ws-test:acct-test:wx-debounce-spawn".to_string();
        PENDING.remove(&key);
        let msg = ConversationMessage {
            id: None,
            workspace_id: "ws-test".to_string(),
            account_id: "acct-test".to_string(),
            contact_wxid: "wx-debounce-spawn".to_string(),
            message_id: None,
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: "hi".to_string(),
            msg_type: None,
            media_ref: None,
            raw: None,
            is_synthetic_relay: false,
            created_at: DateTime::now(),
        };

        let (st1, spawned1) = register_inbound(key.clone(), msg.clone(), 4_000);
        assert!(spawned1, "首条入站 SHALL 触发 spawn");
        assert_eq!(st1.generation.load(Ordering::Acquire), 1);

        // 第二、三条：runner 已活，只 bump generation，不再 spawn。
        let (st2, spawned2) = register_inbound(key.clone(), msg.clone(), 4_000);
        assert!(!spawned2, "后续入站 SHALL NOT 再 spawn");
        assert_eq!(st2.generation.load(Ordering::Acquire), 2);
        let (st3, spawned3) = register_inbound(key.clone(), msg.clone(), 4_000);
        assert!(!spawned3);
        assert_eq!(st3.generation.load(Ordering::Acquire), 3);

        PENDING.remove(&key);
    }

    /// 测试用最小入站消息构造器（内容/key 由调用方区分）。
    fn test_inbound(wxid: &str, content: &str) -> ConversationMessage {
        ConversationMessage {
            id: None,
            workspace_id: "ws-test".to_string(),
            account_id: "acct-test".to_string(),
            contact_wxid: wxid.to_string(),
            message_id: None,
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: content.to_string(),
            msg_type: None,
            media_ref: None,
            raw: None,
            is_synthetic_relay: false,
            created_at: DateTime::now(),
        }
    }

    /// 正常退休：runner 起跑快照 gen_at_start，期间无新入站 → remove_if 谓词
    /// （generation 未变）成立 → 原子移除，key 不再驻留 PENDING。
    #[test]
    fn retire_succeeds_when_generation_unchanged() {
        let key = "ws-test:acct-test:wx-retire-ok".to_string();
        PENDING.remove(&key);

        let (st, spawned) = register_inbound(key.clone(), test_inbound("wx-retire-ok", "hi"), 4_000);
        assert!(spawned);
        let gen_at_start = st.generation.load(Ordering::Acquire);
        assert_eq!(gen_at_start, 1);

        // runner (g) 步：谓词在 shard 锁内复核 generation 未变才移除。
        let removed = PENDING
            .remove_if(&key, |_, s| {
                s.generation.load(Ordering::Acquire) == gen_at_start
            })
            .is_some();
        assert!(removed, "generation 未变时 SHALL 成功退休");
        assert!(!PENDING.contains_key(&key), "退休后 key 不得驻留");
    }

    /// 退休竞态（核心不变量，对应 plan「清理竞态证明」）：runner 起跑快照
    /// gen_at_start=1，跑流水线期间晚到一条入站把 generation bump 到 2；runner
    /// 到达 (g) 步执行 remove_if(gen==1) → 谓词失败 → 不移除 → key 仍在 →
    /// runner 据此回 loop 重算。证明边界期到达的消息绝不被丢。
    #[test]
    fn retire_blocked_when_late_inbound_bumped_generation() {
        let key = "ws-test:acct-test:wx-retire-race".to_string();
        PENDING.remove(&key);

        let (st, _) = register_inbound(key.clone(), test_inbound("wx-retire-race", "first"), 4_000);
        let gen_at_start = st.generation.load(Ordering::Acquire);
        assert_eq!(gen_at_start, 1);

        // 晚到入站：runner 已过抢占检查、正走向退休的窗口里到达，bump generation。
        let (_, spawned2) =
            register_inbound(key.clone(), test_inbound("wx-retire-race", "late"), 4_000);
        assert!(!spawned2, "晚到入站不得再 spawn——runner 仍在");

        // runner (g) 步：谓词复核 gen==gen_at_start(=1)，实际已是 2 → 失败 → 不移除。
        let removed = PENDING
            .remove_if(&key, |_, s| {
                s.generation.load(Ordering::Acquire) == gen_at_start
            })
            .is_some();
        assert!(!removed, "晚到入站 bump 后 SHALL NOT 退休（否则丢这条消息）");
        assert!(
            PENDING.contains_key(&key),
            "退休被阻时 runner 状态必须留存以供重算"
        );
        // runner 据 barge_in_triggered 判定需重算。
        assert!(barge_in_triggered(
            gen_at_start,
            PENDING.get(&key).unwrap().generation.load(Ordering::Acquire)
        ));

        PENDING.remove(&key);
    }

    /// 退休后重 spawn：runner 成功退休移除 key 后，新入站落 Vacant 分支 →
    /// 插入全新状态（generation 从 0 重新 +1 = 1）→ spawned_now 再次为 true。
    #[test]
    fn retire_then_new_inbound_respawns() {
        let key = "ws-test:acct-test:wx-respawn".to_string();
        PENDING.remove(&key);

        let (st1, spawned1) =
            register_inbound(key.clone(), test_inbound("wx-respawn", "a"), 4_000);
        assert!(spawned1);
        let gen0 = st1.generation.load(Ordering::Acquire);
        PENDING.remove_if(&key, |_, s| {
            s.generation.load(Ordering::Acquire) == gen0
        });
        assert!(!PENDING.contains_key(&key));

        // 退休后的新入站：必须重新 spawn（runner 已退场）。
        let (st2, spawned2) =
            register_inbound(key.clone(), test_inbound("wx-respawn", "b"), 4_000);
        assert!(spawned2, "退休后新入站 SHALL 重新 spawn runner");
        assert_eq!(
            st2.generation.load(Ordering::Acquire),
            1,
            "重 spawn 后 generation 从全新状态的 1 起算"
        );

        PENDING.remove(&key);
    }

    /// 并发 spawn 原子性：N 个线程同时注册同一 key，DashMap entry 持 shard 写锁
    /// 串行化 → 恰好一个线程拿到 spawned_now=true（防 double-spawn），最终
    /// generation == N。断言的是计数不变量，不依赖线程调度顺序 → 不 flaky。
    #[test]
    fn concurrent_register_same_key_spawns_exactly_once() {
        use std::sync::atomic::AtomicU32;
        use std::sync::{Arc as StdArc, Barrier};
        use std::thread;

        let key = "ws-test:acct-test:wx-concurrent".to_string();
        PENDING.remove(&key);

        const N: usize = 16;
        let barrier = StdArc::new(Barrier::new(N));
        let spawn_count = StdArc::new(AtomicU32::new(0));

        let handles: Vec<_> = (0..N)
            .map(|i| {
                let key = key.clone();
                let barrier = barrier.clone();
                let spawn_count = spawn_count.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let (_, spawned) = register_inbound(
                        key.clone(),
                        test_inbound("wx-concurrent", &format!("m{i}")),
                        4_000,
                    );
                    if spawned {
                        spawn_count.fetch_add(1, Ordering::AcqRel);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread join");
        }

        assert_eq!(
            spawn_count.load(Ordering::Acquire),
            1,
            "N 线程并发注册同一 key 必须恰好 spawn 一次"
        );
        assert_eq!(
            PENDING.get(&key).unwrap().generation.load(Ordering::Acquire),
            N as u64,
            "每条入站各 bump 一次 generation，最终须等于线程数"
        );

        PENDING.remove(&key);
    }

    /// 抢占链端到端：runner 起跑快照 gen_at_start，期间多条入站把 generation
    /// 推高 → barge_in_triggered 成立 → 网关 guard 返回 true → 放弃在途生成重算。
    /// 无新入站时 guard 恒 false，正常走完。
    #[test]
    fn barge_in_chain_from_register_to_guard() {
        let key = "ws-test:acct-test:wx-barge-chain".to_string();
        PENDING.remove(&key);

        let (st, _) = register_inbound(key.clone(), test_inbound("wx-barge-chain", "1"), 4_000);
        let gen_at_start = st.generation.load(Ordering::Acquire);

        // 无新入站：guard 视角 generation 未变 → 不抢占。
        assert!(!barge_in_triggered(
            gen_at_start,
            st.generation.load(Ordering::Acquire)
        ));

        // 期间到 2 条新入站。
        register_inbound(key.clone(), test_inbound("wx-barge-chain", "2"), 4_000);
        register_inbound(key.clone(), test_inbound("wx-barge-chain", "3"), 4_000);

        assert!(
            barge_in_triggered(gen_at_start, st.generation.load(Ordering::Acquire)),
            "期间有新入站时 guard SHALL 触发抢占重算"
        );

        PENDING.remove(&key);
    }
}

#[cfg(test)]
mod rate_limit_dedupe_tests {
    use super::*;

    /// P1-2：同一账号 + 同一 day_bucket → 同一 dedupe_key，
    /// partial unique index 才能在并发下原子去重。
    #[test]
    fn dedupe_key_is_stable_per_account_and_day() {
        let a = rate_limit_event_dedupe_key("acct_42", 19_876);
        let b = rate_limit_event_dedupe_key("acct_42", 19_876);
        assert_eq!(a, b);
        assert_eq!(a, "rate_limit:acct_42:19876");
    }

    /// 跨天必须不同 key，否则次日的限流事件被错误压制。
    #[test]
    fn dedupe_key_segregates_by_day_bucket() {
        let day_a = rate_limit_event_dedupe_key("acct_42", 19_876);
        let day_b = rate_limit_event_dedupe_key("acct_42", 19_877);
        assert_ne!(day_a, day_b);
    }

    /// 不同账号不可共享 key（否则 A 触发限流，B 整天再触发都被压制）。
    #[test]
    fn dedupe_key_segregates_by_account() {
        let a = rate_limit_event_dedupe_key("acct_a", 19_876);
        let b = rate_limit_event_dedupe_key("acct_b", 19_876);
        assert_ne!(a, b);
    }
}

/// 方案 B：校验 gewe-agent 每账号签名 + 时间戳时效（纯函数，便于单测）。
///
/// gewe-agent 侧签名内容 = `"<timestamp_header.trim()>." + raw_body`，
/// HMAC-SHA256(每 slot 明文 messageWebhookSecret)，hex 写到
/// `x-webhook-signature: sha256=<hex>`，配套 `x-webhook-timestamp`（毫秒）。
/// 全部通过返回 Ok；否则返回具体拒绝原因（handler 统一转 400 + 脱敏 warn 日志）。
/// `secret=None`/空 → SecretNotConfigured（验签开关打开时的 fail-closed 语义）。
#[derive(Debug, PartialEq, Eq)]
enum WebhookSigError {
    SecretNotConfigured,
    MissingSignature,
    MissingTimestamp,
    BadTimestamp,
    TimestampOutOfWindow,
    BadSignatureFormat,
    Mismatch,
}

fn verify_webhook_signature(
    secret: Option<&str>,
    timestamp_header: Option<&str>,
    signature_header: Option<&str>,
    body: &[u8],
    now_ms: i64,
    skew_seconds: i64,
) -> Result<(), WebhookSigError> {
    let secret = secret
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::SecretNotConfigured)?;
    let sig = signature_header
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::MissingSignature)?;
    let ts_str = timestamp_header
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::MissingTimestamp)?;
    let ts_ms: i64 = ts_str.parse().map_err(|_| WebhookSigError::BadTimestamp)?;
    if (now_ms - ts_ms).abs() > skew_seconds.saturating_mul(1000) {
        return Err(WebhookSigError::TimestampOutOfWindow);
    }
    let hex_part = sig.strip_prefix("sha256=").unwrap_or(sig);
    let expected = hex::decode(hex_part).map_err(|_| WebhookSigError::BadSignatureFormat)?;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| WebhookSigError::SecretNotConfigured)?;
    // 与 gewe-agent 一致：先喂 "<ts>." 再喂 raw body。
    mac.update(ts_str.as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&expected).map_err(|_| WebhookSigError::Mismatch)
}

#[cfg(test)]
mod webhook_sig_tests {
    use super::*;

    // 与 gewe-agent webhook-signing.ts 逐字节对齐的金标：
    // HMAC-SHA256(secret="test-secret", "<ts>." + body) hex。
    const SECRET: &str = "test-secret";
    const TS: &str = "1720500000000";
    const BODY: &[u8] = b"{\"foo\":\"bar\"}";
    // python: hmac.new(b"test-secret", b"1720500000000." + BODY, sha256).hexdigest()
    const GOLDEN_HEX: &str = "1936755de0397e2cc912ab1652aaeccb278cae4bb489f16f0dbe3173a8057cbe";
    const NOW_MS: i64 = 1_720_500_000_000; // 与 TS 相等 → 偏差 0
    const SKEW: i64 = 300;

    fn header() -> String {
        format!("sha256={GOLDEN_HEX}")
    }

    #[test]
    fn accepts_correct_signature_within_window() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn accepts_signature_without_sha256_prefix() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(GOLDEN_HEX), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn accepts_uppercase_hex() {
        let h = format!("sha256={}", GOLDEN_HEX.to_uppercase());
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&h), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn rejects_tampered_body() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), b"{\"foo\":\"BAR\"}", NOW_MS, SKEW),
            Err(WebhookSigError::Mismatch)
        );
    }

    #[test]
    fn rejects_wrong_secret() {
        assert_eq!(
            verify_webhook_signature(Some("other-secret"), Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::Mismatch)
        );
    }

    #[test]
    fn rejects_timestamp_out_of_window_future() {
        // now 比 ts 早 301s（ts 在未来 301s）→ 超窗
        let now = NOW_MS - 301_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Err(WebhookSigError::TimestampOutOfWindow)
        );
    }

    #[test]
    fn rejects_timestamp_out_of_window_past() {
        // now 比 ts 晚 301s → 超窗
        let now = NOW_MS + 301_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Err(WebhookSigError::TimestampOutOfWindow)
        );
    }

    #[test]
    fn accepts_timestamp_at_window_edge() {
        // 恰好 300s → 不超窗（用 <= 边界语义）
        let now = NOW_MS + 300_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Ok(())
        );
    }

    #[test]
    fn rejects_missing_signature() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), None, BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingSignature)
        );
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some("  "), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingSignature)
        );
    }

    #[test]
    fn rejects_missing_timestamp() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), None, Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingTimestamp)
        );
    }

    #[test]
    fn rejects_bad_timestamp() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some("not-a-number"), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::BadTimestamp)
        );
    }

    #[test]
    fn rejects_bad_signature_format() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some("sha256=not-hex!!"), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::BadSignatureFormat)
        );
    }

    #[test]
    fn rejects_secret_not_configured() {
        assert_eq!(
            verify_webhook_signature(None, Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::SecretNotConfigured)
        );
        assert_eq!(
            verify_webhook_signature(Some("  "), Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::SecretNotConfigured)
        );
    }
}

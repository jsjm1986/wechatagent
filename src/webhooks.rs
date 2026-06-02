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
    models::{AgentStatus, Contact, ConversationMessage, MessageDirection},
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

fn contact_key(workspace_id: &str, account_id: &str, wxid: &str) -> String {
    format!("{workspace_id}:{account_id}:{wxid}")
}

/// 单联系人的去抖 / 抢占共享状态。`generation` 每入站 +1，既是去抖触发也是
/// 抢占信号；`deadline_ms` 每入站刷新即重置去抖窗口；`latest_inbound` 是最新
/// 入站快照（短锁，绝不跨 `.await` 持有）。
struct PendingState {
    generation: AtomicU64,
    deadline_ms: AtomicI64,
    latest_inbound: parking_lot::Mutex<ConversationMessage>,
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
fn register_inbound(
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
async fn run_debounce_pipeline(
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
    // P0-E：HMAC 签名校验。MCP（GeWe）侧约定按 `MCP_API_KEY` 作 HMAC-SHA256
    // 签 raw body，hex 写到 header `X-MCP-Signature`。env `WEBHOOK_VERIFY_SIGNATURE`
    // 可临时关停（默认开），仅用于灰度切换 + 联调，不应该在生产长期 false。
    if state.config.webhook_verify_signature {
        let provided = headers
            .get("x-mcp-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !verify_hmac_sha256(state.config.mcp_api_key.as_bytes(), &body, provided) {
            tracing::warn!(
                "webhook rejected: bad signature (provided_len={}, body_len={})",
                provided.len(),
                body.len()
            );
            return Err(AppError::BadRequest("invalid signature".into()));
        }
    }

    let payload: Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("invalid json body: {}", e)))?;

    // P2：先处理 GeWe 控制事件（在解析 appId / 进限流之前）。控制事件不喂 Agent，
    // 立刻 200 返回，避免占用 per-account 限流配额，也保证 MCP 那边 5s timeout
    // 内一定收到 ack。
    //
    // 三类 short-circuit：
    // (a) `testMsg` 探活：GeWe 控制台「测试回调」按钮使用，直接 ack。
    // (b) `TypeName=Offline`：账号离线事件，本期版本不在 webhook 入口处理（账号在线
    //     状态走 SSE `account_status`），直接 ack。
    // (c) MCP envelope `_mcp.event` 非 wechat.message.created 的事件（如未来扩展），
    //     谨慎放行：除显式消息事件外一律 ack ignored。
    if let Some(test_msg) = find_string(&payload, &["testMsg", "TestMsg"]) {
        return Ok(Json(serde_json::json!({
            "ok": true,
            "ignored": "callback_test",
            "echo": test_msg
        })));
    }
    if let Some(type_name) = find_string(&payload, &["TypeName", "typeName"]) {
        let lower = type_name.to_ascii_lowercase();
        if lower == "offline" {
            return Ok(Json(serde_json::json!({
                "ok": true,
                "ignored": "offline_event",
                "type": type_name
            })));
        }
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
    let (workspace_id, account_id) =
        match resolve_account_context(&state, app_id.as_deref()).await {
            Ok(pair) => pair,
            Err(AppError::BadRequest(msg)) => {
                // P1：未知 appId 不再静默回退到 default account_id；写一条 admin-visible
                // 事件后明确 400，让运维侧能看到「webhook 入站但无对应 account」。
                let _ = emit_unknown_app_id_event(&state, app_id.as_deref()).await;
                return Err(AppError::BadRequest(msg));
            }
            Err(other) => return Err(other),
        };

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

    let from_wxid = find_string(
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
    .ok_or_else(|| AppError::BadRequest("webhook missing sender wxid".to_string()))?;
    let content = find_string(
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
    .unwrap_or_default();
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
    let inbound = ConversationMessage {
        id: None,
        workspace_id: workspace_id.clone(),
        account_id: account_id.clone(),
        contact_wxid: from_wxid.clone(),
        message_id: effective_message_id.clone(),
        dedupe_key: Some(dedupe_key.clone()),
        direction: MessageDirection::Inbound,
        content,
        raw,
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
    if managed {
        let key = contact_key(&workspace_id, &account_id, &from_wxid);
        let window_ms = state.config.message_debounce_window_ms;
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

    Ok(Json(serde_json::json!({
        "ok": true,
        "managed": managed,
        "queued": managed
    })))
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

async fn resolve_account_context(
    state: &AppState,
    app_id: Option<&str>,
) -> AppResult<(String, String)> {
    if let Some(app_id) = app_id {
        if let Some(account) = state
            .db
            .accounts()
            .find_one(doc! { "app_id": app_id }, None)
            .await?
        {
            return Ok((account.workspace_id, account.account_id));
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

/// P0-E：HMAC-SHA256(body, MCP_API_KEY) 常时间比对。
///
/// `provided_hex` 是 header `X-MCP-Signature` 的 hex 字符串（大小写不敏感）。
/// 任一为空 / hex 解码失败 / 长度不匹配 → 直接 false（不泄露具体原因）。
/// 用 [`hmac::Mac::verify_slice`] 做常时间比对，避免 timing attack。
fn verify_hmac_sha256(key: &[u8], body: &[u8], provided_hex: &str) -> bool {
    if provided_hex.is_empty() || key.is_empty() {
        return false;
    }
    let expected_bytes = match hex::decode(provided_hex.trim()) {
        Ok(b) => b,
        Err(_) => return false,
    };
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = match HmacSha256::new_from_slice(key) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&expected_bytes).is_ok()
}

#[cfg(test)]
mod hmac_tests {
    use super::*;

    fn sign(key: &[u8], body: &[u8]) -> String {
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn verify_accepts_correct_signature() {
        let key = b"secret_key";
        let body = b"{\"appId\":\"a\"}";
        let sig = sign(key, body);
        assert!(verify_hmac_sha256(key, body, &sig));
    }

    #[test]
    fn verify_accepts_uppercase_hex() {
        let key = b"secret_key";
        let body = b"x";
        let sig = sign(key, body).to_uppercase();
        assert!(verify_hmac_sha256(key, body, &sig));
    }

    #[test]
    fn verify_rejects_wrong_signature() {
        let key = b"secret_key";
        let body = b"x";
        let bad = "0".repeat(64);
        assert!(!verify_hmac_sha256(key, body, &bad));
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let key = b"secret_key";
        let sig = sign(key, b"original");
        assert!(!verify_hmac_sha256(key, b"tampered", &sig));
    }

    #[test]
    fn verify_rejects_empty_signature() {
        assert!(!verify_hmac_sha256(b"k", b"x", ""));
    }

    #[test]
    fn verify_rejects_non_hex() {
        assert!(!verify_hmac_sha256(b"k", b"x", "not-a-hex-string!"));
    }

    #[test]
    fn verify_rejects_empty_key() {
        assert!(!verify_hmac_sha256(b"", b"x", &"00".repeat(32)));
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
            raw: None,
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

use std::num::NonZeroU32;
use std::sync::{Arc, LazyLock};

use axum::{extract::State, Json};
use dashmap::DashMap;
use governor::{
    clock::{Clock, DefaultClock},
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use mongodb::{
    bson::{doc, to_document, DateTime},
    options::UpdateOptions,
};
use serde_json::Value;

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

pub async fn wechat_webhook(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
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

    if state
        .db
        .messages()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": &account_id,
                "dedupe_key": &dedupe_key
            },
            None,
        )
        .await?
        .is_some()
    {
        return Ok(Json(serde_json::json!({ "ok": true, "duplicate": true })));
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

    let raw = to_document(&payload).ok();
    let inbound = ConversationMessage {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: from_wxid.clone(),
        message_id: effective_message_id,
        dedupe_key: Some(dedupe_key),
        direction: MessageDirection::Inbound,
        content,
        raw,
        created_at: DateTime::now(),
    };
    state.db.messages().insert_one(&inbound, None).await?;
    let now = DateTime::now();
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

    // P2：MCP（GeWe-agent）那一侧 fetch(messageWebhookUrl) 用了 5s AbortController
    // timeout 且失败不重试。Agent 决策 + Review 流水线一次约 10–15s，远超
     // 5s，必须把它挪到后台 spawn，主请求落库后立即 ack。
    let managed = contact.agent_status == AgentStatus::Managed;
    if managed {
        let bg_state = state.clone();
        let bg_contact = contact.clone();
        let bg_inbound = inbound.clone();
        let bg_account_id = account_id.clone();
        let bg_from_wxid = from_wxid.clone();
        let bg_app_id = app_id.clone();
        tokio::spawn(async move {
            if let Err(error) =
                agent::record_user_reaction(&bg_state, &bg_contact, &bg_inbound).await
            {
                let _ = agent::write_event_for_account(
                    &bg_state,
                    &bg_account_id,
                    Some(&bg_from_wxid),
                    "agent_error",
                    "failed",
                    &format!("record_user_reaction failed: {error}"),
                    bg_app_id.clone().map(|v| doc! { "app_id": v }),
                )
                .await;
                return;
            }
            if let Err(error) =
                agent::handle_managed_message(&bg_state, bg_contact, &bg_inbound).await
            {
                let _ = agent::write_event_for_account(
                    &bg_state,
                    &bg_account_id,
                    Some(&bg_from_wxid),
                    "agent_error",
                    "failed",
                    &error.to_string(),
                    bg_app_id.map(|v| doc! { "app_id": v }),
                )
                .await;
            }
        });
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

/// LP-14 / Task 20：限流命中时按 account 当日去重写一条 agent_event，避免事件爆量。
async fn maybe_emit_rate_limit_event(state: &AppState, account_id: &str) -> AppResult<()> {
    let day_start_ms = {
        let now = DateTime::now().timestamp_millis();
        let day_ms: i64 = 24 * 60 * 60 * 1000;
        now - (now.rem_euclid(day_ms))
    };
    let exists = state
        .db
        .events()
        .find_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "account_id": account_id,
                "kind": "webhook_rate_limited",
                "created_at": { "$gte": DateTime::from_millis(day_start_ms) }
            },
            None,
        )
        .await?;
    if exists.is_some() {
        return Ok(());
    }
    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: account_id.to_string(),
                contact_wxid: None,
                kind: "webhook_rate_limited".to_string(),
                status: "blocked".to_string(),
                summary: "webhook 入口触发 per-account 限流".to_string(),
                details: None,
                created_at: DateTime::now(),
            },
            None,
        )
        .await;
    Ok(())
}

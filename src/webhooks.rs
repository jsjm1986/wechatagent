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
    let app_id = find_string(&payload, &["appId", "app_id", "appid"]);
    let (workspace_id, account_id) = resolve_account_context(&state, app_id.as_deref()).await?;

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
            "fromWxid",
            "from_wxid",
            "fromUserName",
            "from_user_name",
            "fromusername",
            "from",
        ],
    )
    .ok_or_else(|| AppError::BadRequest("webhook missing sender wxid".to_string()))?;
    let content = find_string(
        &payload,
        &[
            "content",
            "text",
            "msgContent",
            "msg_content",
            "message",
            "messageContent",
        ],
    )
    .unwrap_or_default();
    let message_id = find_string(
        &payload,
        &[
            "newMsgId",
            "new_msg_id",
            "msgId",
            "msg_id",
            "messageId",
            "id",
        ],
    );
    let dedupe_key = message_id
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
        message_id,
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

    let managed = contact.agent_status == AgentStatus::Managed;
    if managed {
        agent::record_user_reaction(&state, &contact, &inbound).await?;
        if let Err(error) = agent::handle_managed_message(&state, contact, &inbound).await {
            agent::write_event_for_account(
                &state,
                &account_id,
                Some(&from_wxid),
                "agent_error",
                "failed",
                &error.to_string(),
                app_id.map(|value| doc! { "app_id": value }),
            )
            .await?;
            return Err(error);
        }
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "managed": managed
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
    }
    Ok((
        state.config.default_workspace_id.clone(),
        state.config.default_account_id.clone(),
    ))
}

async fn upsert_webhook_contact(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
    payload: &Value,
) -> AppResult<Option<Contact>> {
    let nickname = find_string(payload, &["nickName", "nickname", "fromNickName"]);
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

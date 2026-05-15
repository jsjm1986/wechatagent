use axum::{extract::State, Json};
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

pub async fn wechat_webhook(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let app_id = find_string(&payload, &["appId", "app_id", "appid"]);
    let (workspace_id, account_id) = resolve_account_context(&state, app_id.as_deref()).await?;
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

    if let Some(message_id) = &message_id {
        if state
            .db
            .messages()
            .find_one(doc! { "message_id": message_id }, None)
            .await?
            .is_some()
        {
            return Ok(Json(serde_json::json!({ "ok": true, "duplicate": true })));
        }
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
        direction: MessageDirection::Inbound,
        content,
        raw,
        created_at: DateTime::now(),
    };
    state.db.messages().insert_one(&inbound, None).await?;
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact.id },
            doc! {
                "$set": {
                    "last_message_at": DateTime::now(),
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;

    let managed = contact.agent_status == AgentStatus::Managed;
    if managed {
        if let Err(error) = agent::handle_managed_message(&state, contact, &inbound).await {
            agent::write_event(
                &state,
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

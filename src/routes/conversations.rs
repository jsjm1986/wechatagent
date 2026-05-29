//! 会话消息路由：根据联系人查询历史对话。

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{bson::doc, options::FindOptions};
use serde_json::{json, Value};

use crate::auth::AuthenticatedAdmin;
use crate::error::AppResult;

use super::shared::*;
use super::AppState;

pub(super) async fn list_messages(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(contact_id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &contact_id).await?;
    let mut cursor = state
        .db
        .messages()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(message) = cursor.try_next().await? {
        items.push(json!({
            "id": message.id.map(|id| id.to_hex()).unwrap_or_default(),
            "direction": message.direction,
            "content": message.content,
            "messageId": message.message_id,
            "createdAt": crate::models::dt_to_string(message.created_at)
        }));
    }
    Ok(Json(json!({ "items": items })))
}

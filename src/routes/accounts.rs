//! 微信账号路由：管理 `WechatAccount` 记录及 MCP key 同步。

use axum::{
    extract::{Path, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::{FindOptions, UpdateOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    mcp::{self},
    models::WechatAccount,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateAccountMcpKeyRequest {
    mcp_api_key: String,
    mcp_base_url: Option<String>,
}

pub(super) async fn list_accounts(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .accounts()
        .find(
            doc! {},
            FindOptions::builder().sort(doc! { "alias": 1 }).build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(account) = cursor.try_next().await? {
        items.push(json!({
            "id": account.id.map(|id| id.to_hex()).unwrap_or_default(),
            "workspaceId": account.workspace_id,
            "accountId": account.account_id,
            "alias": account.alias,
            "displayName": account.display_name,
            "appId": account.app_id,
            "wxid": account.wxid,
            "nickName": account.nick_name,
            "mcpBaseUrl": account.mcp_base_url,
            "mcpKeyConfigured": account.mcp_api_key.as_ref().map(|key| !key.is_empty()).unwrap_or(false) || !state.config.mcp_api_key.is_empty(),
            "online": account.online
        }));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn sync_accounts(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let result = mcp::logged_call(&state, "account_list", json!({})).await?;
    let items = result
        .get("items")
        .and_then(|value| value.as_array())
        .ok_or_else(|| AppError::External("account_list returned no items".to_string()))?;

    let mut synced = 0usize;
    for item in items {
        let account_id = item.get("id").map(|v| v.to_string()).unwrap_or_else(|| {
            item.get("alias")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string()
        });
        let alias = item
            .get("alias")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();
        let account = WechatAccount {
            id: None,
            workspace_id: state.config.default_workspace_id.clone(),
            account_id: account_id.clone(),
            alias,
            display_name: item
                .get("display_name")
                .or_else(|| item.get("displayName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            app_id: item
                .get("app_id")
                .or_else(|| item.get("appId"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            wxid: item
                .get("wxid")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            nick_name: item
                .get("nick_name")
                .or_else(|| item.get("nickName"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            mcp_base_url: Some(state.config.mcp_base_url.clone()),
            mcp_api_key: Some(state.config.mcp_api_key.clone()),
            online: item
                .get("online")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            last_sync_at: DateTime::now(),
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        state
            .db
            .accounts()
            .update_one(
                doc! {
                    "workspace_id": &account.workspace_id,
                    "account_id": &account.account_id
                },
                doc! {
                    "$set": {
                        "alias": &account.alias,
                        "display_name": &account.display_name,
                        "app_id": &account.app_id,
                        "wxid": &account.wxid,
                        "nick_name": &account.nick_name,
                        "mcp_base_url": &account.mcp_base_url,
                        "online": account.online,
                        "last_sync_at": account.last_sync_at,
                        "updated_at": account.updated_at,
                    },
                    "$setOnInsert": {
                        "workspace_id": &account.workspace_id,
                        "account_id": &account.account_id,
                        "mcp_api_key": &account.mcp_api_key,
                        "created_at": account.created_at,
                    }
                },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;
        synced += 1;
    }
    Ok(Json(json!({ "synced": synced })))
}

pub(super) async fn update_account_mcp_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateAccountMcpKeyRequest>,
) -> AppResult<Json<Value>> {
    if payload.mcp_api_key.trim().is_empty() {
        return Err(AppError::BadRequest("mcpApiKey is required".to_string()));
    }
    let object_id = parse_object_id(&id)?;
    state
        .db
        .accounts()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "mcp_api_key": payload.mcp_api_key,
                    "mcp_base_url": payload.mcp_base_url.unwrap_or_else(|| state.config.mcp_base_url.clone()),
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

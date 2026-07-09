//! 微信账号路由：管理 `WechatAccount` 记录及 MCP key 同步。

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::{FindOptions, UpdateOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    mcp::{self},
    models::WechatAccount,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccountMcpKeyRequest {
    mcp_api_key: String,
    mcp_base_url: Option<String>,
}

pub async fn list_accounts(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .accounts()
        .find(
            doc! { "workspace_id": &admin.current_workspace },
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
            "online": account.online,
            "status": account.status
        }));
    }
    Ok(Json(json!({ "items": items })))
}

pub async fn sync_accounts(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
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
            workspace_id: admin.current_workspace.clone(),
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
            // Task 3 会回填真实头像（sync_accounts 时从 MCP 拉取），此处先 None。
            avatar_url: None,
            mcp_base_url: Some(state.config.mcp_base_url.clone()),
            mcp_api_key: Some(state.config.mcp_api_key.clone()),
            webhook_secret: None,
            online: item
                .get("online")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            status: item
                .get("status")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            last_sync_at: DateTime::now(),
            capacity: 0,
            persona_tag: None,
            off_hours: Vec::new(),
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
                        "status": &account.status,
                        "last_sync_at": account.last_sync_at,
                        "updated_at": account.updated_at,
                        // 确保所有 WechatAccount 必填字段都在 $set 或 $setOnInsert 中，
                        // 避免部分更新留下反序列化失败的不完整记录（capacity 有 serde default=0 可省略）。
                        "workspace_id": &account.workspace_id,
                        "account_id": &account.account_id,
                    },
                    "$setOnInsert": {
                        "mcp_api_key": &account.mcp_api_key,
                        "created_at": account.created_at,
                        "capacity": 0,
                    }
                },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;
        synced += 1;
    }
    Ok(Json(json!({ "synced": synced })))
}

pub async fn update_account_mcp_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<UpdateAccountMcpKeyRequest>,
) -> AppResult<Json<Value>> {
    if payload.mcp_api_key.trim().is_empty() {
        return Err(AppError::BadRequest("mcpApiKey is required".to_string()));
    }
    let object_id = parse_object_id(&id)?;
    let result = state
        .db
        .accounts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
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
    if result.matched_count == 0 {
        return Err(AppError::NotFound("account not found".to_string()));
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginBeginRequest {
    account_alias: Option<String>,
    #[serde(default)]
    login_type: LoginType,
    #[serde(default)]
    login_flow: LoginFlow,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LoginType {
    Mac,
    Ipad,
}

impl Default for LoginType {
    fn default() -> Self {
        Self::Mac
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LoginFlow {
    Auto,
    Manual,
}

impl Default for LoginFlow {
    fn default() -> Self {
        Self::Auto
    }
}

/// POST /api/accounts/login/begin - 发起微信账号登录，获取二维码
///
/// 调用 MCP `login_begin` 工具，返回：
/// - `qr_data_url`: 二维码图片的 data URL（base64）
/// - `login_page_url`: MCP server 提供的登录页面 URL（推荐优先使用）
/// - `session_id`: 轮询会话 ID（传给 `login_poll`）
pub async fn login_begin(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<LoginBeginRequest>,
) -> AppResult<Json<Value>> {
    // Workspace Key 必须传 account_alias；Account Key 可省略
    let arguments = json!({
        "login_type": format!("{:?}", payload.login_type).to_lowercase(),
        "login_flow": format!("{:?}", payload.login_flow).to_lowercase(),
    });

    let result = if let Some(alias) = payload.account_alias {
        // 通过 alias 找到对应账号的 credentials
        let account = state
            .db
            .accounts()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "alias": &alias
                },
                None,
            )
            .await?
            .ok_or_else(|| AppError::NotFound(format!("account alias {} not found", alias)))?;

        mcp::logged_call_for_account(&state, &account.account_id, "login_begin", arguments).await?
    } else {
        // Account Key 模式：使用默认 credentials
        mcp::logged_call(&state, "login_begin", arguments).await?
    };

    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginPollQuery {
    login_session_id: String,
    account_alias: Option<String>,
}

/// GET /api/accounts/login/poll?loginSessionId=xxx&accountAlias=yyy - 轮询登录状态
///
/// 调用 MCP `login_poll` 工具（参数 `login_session_id` 来自 `login_begin` 返回值），返回：
/// - `status`: `pending` / `success` / `expired` / `canceled`
/// - 登录成功后的微信身份字段（`wxid` / `nick_name` 等，以 server 回包为准）
///
/// 前端应每 2-3 秒轮询一次，直到 status 不是 `pending`。
/// 登录成功后建议立即调用 `POST /api/accounts/sync` 同步账号信息。
pub async fn login_poll(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    axum::extract::Query(query): axum::extract::Query<LoginPollQuery>,
) -> AppResult<Json<Value>> {
    let arguments = json!({
        "login_session_id": query.login_session_id,
    });

    let result = if let Some(alias) = query.account_alias {
        let account = state
            .db
            .accounts()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "alias": &alias
                },
                None,
            )
            .await?
            .ok_or_else(|| AppError::NotFound(format!("account alias {} not found", alias)))?;

        mcp::logged_call_for_account(&state, &account.account_id, "login_poll", arguments).await?
    } else {
        mcp::logged_call(&state, "login_poll", arguments).await?
    };

    Ok(Json(result))
}

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, to_bson, DateTime, Regex},
    options::{FindOptions, UpdateOptions},
};
use serde_json::{json, Value};

use crate::{
    agent,
    config::AppConfig,
    db::Database,
    error::{AppError, AppResult},
    llm::LlmClient,
    mcp::{self, McpClient},
    models::{
        ApiContact, Contact, ContactQuery, EnableAgentRequest, ProfileNoteRequest,
        SearchImportRequest, WechatAccount,
    },
};

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub mcp: McpClient,
    pub llm: LlmClient,
    pub config: AppConfig,
}

pub fn api_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/accounts", get(list_accounts))
        .route("/accounts/sync", post(sync_accounts))
        .route("/contacts", get(list_contacts))
        .route("/contacts/search-import", post(search_import_contacts))
        .route("/contacts/:id", get(get_contact))
        .route("/contacts/:id/enable-agent", post(enable_agent))
        .route("/contacts/:id/disable-agent", post(disable_agent))
        .route("/contacts/:id/profile-note", put(update_profile_note))
        .route("/conversations/:contact_id/messages", get(list_messages))
        .route("/events", get(list_events))
        .route("/tasks", get(list_tasks))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "appBaseUrl": state.config.app_base_url
    }))
}

async fn list_accounts(State(state): State<AppState>) -> AppResult<Json<Value>> {
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
            "online": account.online
        }));
    }
    Ok(Json(json!({ "items": items })))
}

async fn sync_accounts(State(state): State<AppState>) -> AppResult<Json<Value>> {
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
                        "online": account.online,
                        "last_sync_at": account.last_sync_at,
                        "updated_at": account.updated_at,
                    },
                    "$setOnInsert": {
                        "workspace_id": &account.workspace_id,
                        "account_id": &account.account_id,
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

async fn list_contacts(
    State(state): State<AppState>,
    Query(query): Query<ContactQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! {};
    if let Some(status) = query.status {
        if !status.is_empty() {
            filter.insert("agent_status", status);
        }
    }
    if let Some(q) = query.q {
        if !q.is_empty() {
            filter.insert(
                "$or",
                vec![
                    doc! { "nickname": Regex { pattern: q.clone(), options: "i".to_string() } },
                    doc! { "remark": Regex { pattern: q.clone(), options: "i".to_string() } },
                    doc! { "wxid": Regex { pattern: q.clone(), options: "i".to_string() } },
                    doc! { "alias": Regex { pattern: q, options: "i".to_string() } },
                ],
            );
        }
    }
    let mut cursor = state
        .db
        .contacts()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(contact) = cursor.try_next().await? {
        items.push(ApiContact::from(contact));
    }
    Ok(Json(json!({ "items": items })))
}

async fn search_import_contacts(
    State(state): State<AppState>,
    Json(payload): Json<SearchImportRequest>,
) -> AppResult<Json<Value>> {
    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("query is required".to_string()));
    }
    let result = mcp::logged_call(
        &state,
        "contacts_search",
        json!({
            "query": payload.query,
            "limit": 20
        }),
    )
    .await?;
    let items = result
        .get("items")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut imported = Vec::new();
    for item in items {
        if let Some(contact_value) = item.get("contact") {
            if let Some(contact) = upsert_contact_from_value(&state, contact_value).await? {
                imported.push(ApiContact::from(contact));
            }
        }
    }
    Ok(Json(json!({ "items": imported })))
}

async fn get_contact(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

async fn enable_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<EnableAgentRequest>,
) -> AppResult<Json<Value>> {
    if payload.human_profile_note.trim().is_empty() {
        return Err(AppError::BadRequest(
            "humanProfileNote is required".to_string(),
        ));
    }
    let object_id = parse_object_id(&id)?;
    let profile = agent::build_initial_profile(&state, &payload.human_profile_note).await?;
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "agent_status": "managed",
                    "human_profile_note": payload.human_profile_note,
                    "agent_profile": to_bson(&profile)?,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

async fn disable_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "agent_status": "normal",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

async fn update_profile_note(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<ProfileNoteRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let profile = agent::build_initial_profile(&state, &payload.human_profile_note).await?;
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "human_profile_note": payload.human_profile_note,
                    "agent_profile": to_bson(&profile)?,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

async fn list_messages(
    State(state): State<AppState>,
    Path(contact_id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &contact_id).await?;
    let mut cursor = state
        .db
        .messages()
        .find(
            doc! { "contact_wxid": &contact.wxid },
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

async fn list_events(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .events()
        .find(
            doc! {},
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(event) = cursor.try_next().await? {
        items.push(json!({
            "id": event.id.map(|id| id.to_hex()).unwrap_or_default(),
            "contactWxid": event.contact_wxid,
            "kind": event.kind,
            "status": event.status,
            "summary": event.summary,
            "createdAt": crate::models::dt_to_string(event.created_at)
        }));
    }
    Ok(Json(json!({ "items": items })))
}

async fn list_tasks(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {},
            FindOptions::builder()
                .sort(doc! { "run_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(task) = cursor.try_next().await? {
        items.push(json!({
            "id": task.id.map(|id| id.to_hex()).unwrap_or_default(),
            "contactWxid": task.contact_wxid,
            "kind": task.kind,
            "runAt": crate::models::dt_to_string(task.run_at),
            "content": task.content,
            "status": task.status,
            "error": task.error
        }));
    }
    Ok(Json(json!({ "items": items })))
}

pub async fn upsert_contact_from_value(
    state: &AppState,
    contact_value: &Value,
) -> AppResult<Option<Contact>> {
    let wxid = contact_value
        .get("userName")
        .or_else(|| contact_value.get("username"))
        .or_else(|| contact_value.get("wxid"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let Some(wxid) = wxid else {
        return Ok(None);
    };
    let nickname = contact_value
        .get("nickName")
        .or_else(|| contact_value.get("nickname"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let remark = contact_value
        .get("remark")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let alias = contact_value
        .get("alias")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "account_id": &state.config.default_account_id,
                "wxid": &wxid
            },
            doc! {
                "$set": {
                    "nickname": &nickname,
                    "remark": &remark,
                    "alias": &alias,
                    "updated_at": DateTime::now()
                },
                "$setOnInsert": {
                    "workspace_id": &state.config.default_workspace_id,
                    "account_id": &state.config.default_account_id,
                    "wxid": &wxid,
                    "agent_status": "normal",
                    "created_at": DateTime::now()
                }
            },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "account_id": &state.config.default_account_id,
                "wxid": &wxid
            },
            None,
        )
        .await?;
    Ok(contact)
}

async fn find_contact_by_id(state: &AppState, id: &str) -> AppResult<Contact> {
    let object_id = parse_object_id(id)?;
    state
        .db
        .contacts()
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("contact not found".to_string()))
}

fn parse_object_id(id: &str) -> AppResult<ObjectId> {
    ObjectId::parse_str(id).map_err(|_| AppError::BadRequest("invalid object id".to_string()))
}

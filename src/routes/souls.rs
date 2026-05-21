//! Agent 灵魂提示路由：管理各 Agent 的人格 prompt。

use axum::{
    extract::{Path, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::{FindOneOptions, FindOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    models::AgentSoul,
    prompts,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentSoulRequest {
    agent_kind: String,
    name: String,
    content: String,
}

pub(super) async fn list_agent_souls(State(state): State<AppState>) -> AppResult<Json<Value>> {
    ensure_default_souls(&state).await?;
    let mut cursor = state
        .db
        .agent_souls()
        .find(
            doc! { "workspace_id": &state.config.default_workspace_id },
            FindOptions::builder()
                .sort(doc! { "agent_kind": 1, "version": -1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(soul) = cursor.try_next().await? {
        items.push(json!({
            "id": soul.id.map(|id| id.to_hex()).unwrap_or_default(),
            "agentKind": soul.agent_kind,
            "name": soul.name,
            "content": soul.content,
            "status": soul.status,
            "version": soul.version,
            "updatedAt": crate::models::dt_to_string(soul.updated_at)
        }));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_agent_soul(
    State(state): State<AppState>,
    Json(payload): Json<AgentSoulRequest>,
) -> AppResult<Json<Value>> {
    if payload.agent_kind.trim().is_empty() || payload.content.trim().is_empty() {
        return Err(AppError::BadRequest(
            "agentKind and content are required".to_string(),
        ));
    }
    let latest = state
        .db
        .agent_souls()
        .find_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "agent_kind": &payload.agent_kind
            },
            FindOneOptions::builder()
                .sort(doc! { "version": -1 })
                .build(),
        )
        .await?;
    let version = latest.map(|item| item.version + 1).unwrap_or(1);
    let soul = AgentSoul {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        agent_kind: payload.agent_kind,
        name: payload.name,
        content: payload.content,
        status: "draft".to_string(),
        version,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state.db.agent_souls().insert_one(soul, None).await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

pub(super) async fn update_agent_soul(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<AgentSoulRequest>,
) -> AppResult<Json<Value>> {
    if payload.agent_kind.trim().is_empty()
        || payload.name.trim().is_empty()
        || payload.content.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "agentKind, name and content are required".to_string(),
        ));
    }
    let object_id = parse_object_id(&id)?;
    state
        .db
        .agent_souls()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
            },
            doc! {
                "$set": {
                    "agent_kind": payload.agent_kind,
                    "name": payload.name,
                    "content": payload.content,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn publish_agent_soul(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let soul = state
        .db
        .agent_souls()
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("agent soul not found".to_string()))?;
    state
        .db
        .agent_souls()
        .delete_many(
            doc! {
                "workspace_id": &soul.workspace_id,
                "agent_kind": &soul.agent_kind,
                "_id": { "$ne": object_id }
            },
            None,
        )
        .await?;
    state
        .db
        .agent_souls()
        .update_one(
            doc! { "_id": object_id },
            doc! { "$set": { "status": "published", "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn ensure_default_souls(state: &AppState) -> AppResult<()> {
    prompts::ensure_prompt_pack_v2(
        &state.db,
        &state.config.default_workspace_id,
        &state.config.default_account_id,
    )
    .await
}

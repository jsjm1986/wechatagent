//! Agent 灵魂提示路由：管理各 Agent 的人格 prompt。

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{bson::doc, options::FindOptions};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    prompts,
    soul_versions::{self, NewSoulVersion},
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

pub(super) async fn list_agent_souls(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    ensure_default_souls(&state, &admin.current_workspace).await?;
    let mut cursor = state
        .db
        .agent_souls()
        .find(
            doc! { "workspace_id": &admin.current_workspace },
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
            "previousVersion": soul.previous_version,
            "seededBy": soul.seeded_by,
            "publishedAt": soul.published_at.and_then(crate::models::dt_to_string),
            "publishedBy": soul.published_by,
            "createdAt": crate::models::dt_to_string(soul.created_at),
            "updatedAt": crate::models::dt_to_string(soul.updated_at),
        }));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_agent_soul(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<AgentSoulRequest>,
) -> AppResult<Json<Value>> {
    let soul = soul_versions::append_version(
        &state.db,
        &admin.current_workspace,
        NewSoulVersion {
            agent_kind: &payload.agent_kind,
            name: &payload.name,
            content: &payload.content,
            seeded_by: "manual",
            previous_version: None,
        },
    )
    .await?;
    Ok(Json(json!({
        "id": soul.id.map(|id| id.to_hex()),
        "version": soul.version
    })))
}

pub(super) async fn update_agent_soul(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
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
    let soul = soul_versions::append_edited_draft(
        &state.db,
        &admin.current_workspace,
        object_id,
        &payload.agent_kind,
        &payload.name,
        &payload.content,
        "manual",
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "id": soul.id.map(|id| id.to_hex()),
        "version": soul.version
    })))
}

pub(super) async fn publish_agent_soul(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let soul = soul_versions::publish_version(
        &state.db,
        &admin.current_workspace,
        object_id,
        &admin.user_id,
    )
    .await?;
    state
        .prompt_pack_version
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Ok(Json(json!({
        "ok": true,
        "id": soul.id.map(|id| id.to_hex()),
        "version": soul.version
    })))
}

pub(super) async fn ensure_default_souls(state: &AppState, workspace_id: &str) -> AppResult<()> {
    let wrote =
        prompts::ensure_prompt_pack_v2(&state.db, workspace_id, &state.config.default_account_id)
            .await?;
    if wrote {
        state
            .prompt_pack_version
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

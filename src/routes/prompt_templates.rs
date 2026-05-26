//! Prompt 模板路由：分层 prompt 的发布与回滚。

use axum::{
    extract::{Path, Query, State},
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
    models::PromptTemplate,
    prompts,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PromptTemplateQuery {
    agent_kind: Option<String>,
    layer: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PromptTemplateRequest {
    prompt_key: String,
    agent_kind: String,
    layer: String,
    title: String,
    description: Option<String>,
    content: String,
    /// Phase E / E3：可选 locale（BCP-47），未提供时落到 [`prompts::DEFAULT_LOCALE`]。
    #[serde(default)]
    locale: Option<String>,
}

pub(super) async fn list_prompt_templates(
    State(state): State<AppState>,
    Query(query): Query<PromptTemplateQuery>,
) -> AppResult<Json<Value>> {
    prompts::ensure_prompt_pack_v2(
        &state.db,
        &state.config.default_workspace_id,
        &state.config.default_account_id,
    )
    .await?;
    let mut filter = doc! { "workspace_id": &state.config.default_workspace_id };
    if let Some(agent_kind) = normalize_optional(query.agent_kind) {
        filter.insert("agent_kind", agent_kind);
    }
    if let Some(layer) = normalize_optional(query.layer) {
        filter.insert("layer", layer);
    }
    let mut cursor = state
        .db
        .prompt_templates()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "agent_kind": 1, "layer": 1, "prompt_key": 1, "version": -1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(template) = cursor.try_next().await? {
        items.push(prompt_template_json(template));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_prompt_template(
    State(state): State<AppState>,
    Json(payload): Json<PromptTemplateRequest>,
) -> AppResult<Json<Value>> {
    validate_prompt_template_input(&payload)?;
    let latest = state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "prompt_key": &payload.prompt_key
            },
            FindOneOptions::builder()
                .sort(doc! { "version": -1 })
                .build(),
        )
        .await?;
    let version = latest.map(|item| item.version + 1).unwrap_or(1);
    let template = PromptTemplate {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        prompt_key: payload.prompt_key,
        agent_kind: payload.agent_kind,
        layer: payload.layer,
        title: payload.title,
        description: normalize_optional(payload.description),
        content: payload.content,
        status: "draft".to_string(),
        version,
        prompt_pack_version: "custom".to_string(),
        created_by: "manual".to_string(),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
        // 后台手工创建时尚未发布；publish_prompt_template 会接管 current_version 切换。
        current_version: false,
        previous_version: None,
        seeded_by: Some("manual".to_string()),
        locale: normalize_optional(payload.locale)
            .or_else(|| Some(prompts::DEFAULT_LOCALE.to_string())),
    };
    let result = state
        .db
        .prompt_templates()
        .insert_one(template, None)
        .await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

pub(super) async fn update_prompt_template(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<PromptTemplateRequest>,
) -> AppResult<Json<Value>> {
    validate_prompt_template_input(&payload)?;
    let object_id = parse_object_id(&id)?;
    state
        .db
        .prompt_templates()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
            },
            doc! {
                "$set": {
                    "prompt_key": payload.prompt_key,
                    "agent_kind": payload.agent_kind,
                    "layer": payload.layer,
                    "title": payload.title,
                    "description": normalize_optional(payload.description),
                    "content": payload.content,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn publish_prompt_template(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let template = state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("prompt template not found".to_string()))?;
    state
        .db
        .prompt_templates()
        .delete_many(
            doc! {
                "workspace_id": &template.workspace_id,
                "prompt_key": &template.prompt_key,
                "_id": { "$ne": object_id }
            },
            None,
        )
        .await?;
    state
        .db
        .prompt_templates()
        .update_one(
            doc! { "_id": object_id },
            doc! { "$set": { "status": "active", "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    // 旧的 product_claim_markers 缓存随 sales 守卫一起删除，commit 3 wiki
    // 化以后再决定要不要在这里集中失效新的缓存层。
    let _ = template;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn reset_system_prompt_pack(
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    prompts::reset_prompt_pack_v2(
        &state.db,
        &state.config.default_workspace_id,
        &state.config.default_account_id,
    )
    .await?;
    // M4 W4 Task 5.3：reset 是显式销毁性 reseed，必须 bump 让 LRU cache 失效。
    state
        .prompt_pack_version
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Ok(Json(
        json!({ "ok": true, "promptPackVersion": prompts::PROMPT_PACK_VERSION }),
    ))
}

pub(super) fn prompt_template_json(template: PromptTemplate) -> Value {
    json!({
        "id": template.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": template.workspace_id,
        "promptKey": template.prompt_key,
        "agentKind": template.agent_kind,
        "layer": template.layer,
        "title": template.title,
        "description": template.description,
        "content": template.content,
        "status": template.status,
        "version": template.version,
        "promptPackVersion": template.prompt_pack_version,
        "createdBy": template.created_by,
        "updatedAt": crate::models::dt_to_string(template.updated_at)
    })
}

pub(super) fn validate_prompt_template_input(payload: &PromptTemplateRequest) -> AppResult<()> {
    if payload.prompt_key.trim().is_empty()
        || payload.agent_kind.trim().is_empty()
        || payload.layer.trim().is_empty()
        || payload.title.trim().is_empty()
        || payload.content.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "promptKey, agentKind, layer, title and content are required".to_string(),
        ));
    }
    Ok(())
}

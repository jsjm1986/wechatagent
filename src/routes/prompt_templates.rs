//! Prompt 模板路由：分层 prompt 的发布与回滚。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::{FindOneOptions, FindOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
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
    /// Task 6.6 第三闸：管理者在收到 needs_human_confirm 后逐字核对无误，带 force=true 重提以覆盖语义审查。
    #[serde(default)]
    force: Option<bool>,
}

pub(super) async fn list_prompt_templates(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<PromptTemplateQuery>,
) -> AppResult<Json<Value>> {
    let wrote = prompts::ensure_prompt_pack_v2(
        &state.db,
        &admin.current_workspace,
        &state.config.default_account_id,
    )
    .await?;
    if wrote {
        state
            .prompt_pack_version
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<PromptTemplateRequest>,
) -> AppResult<Json<Value>> {
    validate_prompt_template_input(&payload)?;
    let latest = state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
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
        workspace_id: admin.current_workspace.clone(),
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
        // 后台手工创建时尚未发布；publish_prompt_template 负责 current_version 切换。
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<PromptTemplateRequest>,
) -> AppResult<Json<Value>> {
    validate_prompt_template_input(&payload)?;
    // 自然语言编辑硬门（fail-closed）：三层分级 + 字面双闸（禁用词 + 锚完整性）。
    // 单点拦截——无论走管理 agent 工具还是管理员直接 PUT，命中即拒、不落库。
    crate::routes::management_prompt_edit::validate_prompt_edit(&payload.prompt_key, &payload.content)
        .map_err(AppError::BadRequest)?;
    let object_id = parse_object_id(&id)?;
    // Task 6.6 第三闸：LLM 红线语义审查（审 diff 增量）。force=true 跳过（管理者已逐字核对）。
    let force = payload.force.unwrap_or(false);
    if !force {
        let old_content = state
            .db
            .prompt_templates()
            .find_one(
                doc! {
                    "_id": object_id,
                    "workspace_id": &admin.current_workspace
                },
                None,
            )
            .await?
            .map(|t| t.content)
            .unwrap_or_default();
        match crate::routes::management_prompt_edit::review_prompt_edit(
            &state,
            &admin.current_workspace,
            &payload.prompt_key,
            &old_content,
            &payload.content,
        )
        .await
        {
            crate::routes::management_prompt_edit::PromptEditVerdict::Pass => {}
            crate::routes::management_prompt_edit::PromptEditVerdict::Reject(reason) => {
                return Err(AppError::BadRequest(format!(
                    "红线语义审查拒绝：{reason}（确认无误可带 force 覆盖）"
                )));
            }
            crate::routes::management_prompt_edit::PromptEditVerdict::NeedsHumanConfirm {
                diff,
                reason,
            } => {
                // 路径B：返回需二次确认（非错误），前端弹框显示 diff+reason，勾选后带 force=true 重提。
                return Ok(Json(json!({
                    "status": "needs_human_confirm",
                    "reason": reason,
                    "diff": diff
                })));
            }
        }
    }
    state
        .db
        .prompt_templates()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            doc! {
                "$set": {
                    "prompt_key": payload.prompt_key,
                    "agent_kind": payload.agent_kind,
                    "layer": payload.layer,
                    "title": payload.title,
                    "description": normalize_optional(payload.description),
                    "content": payload.content,
                    // 防 PR#42 启动对齐 align_prompt_specs 把被编辑的系统种子行
                    // （seeded_by="system" 且内容≠DEFAULT）归档重种回 DEFAULT。
                    // 置 "manual" 让 align 跳过，保住管理者编辑活过重启。
                    "seeded_by": "manual",
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let template = state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
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
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    prompts::reset_prompt_pack_v2(
        &state.db,
        &admin.current_workspace,
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

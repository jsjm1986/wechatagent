//! Prompt 模板路由：分层 prompt 的发布与回滚。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{bson::doc, options::FindOptions};
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

/// publish 端点可选 body：force=true 时跳过 LLM 第三闸（管理者已逐字核对），
/// 但仍过字面双闸（禁词/锚完整性是确定性硬闸，force 不可绕）。无 body 时落 default（force=None）。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRequest {
    #[serde(default)]
    force: Option<bool>,
}

pub const RESET_SYSTEM_PROMPT_PACK_CONFIRMATION: &str = "RESET PROMPT PACK";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ResetSystemPromptPackRequest {
    confirmation: String,
}

fn validate_reset_confirmation(value: &str) -> AppResult<()> {
    if value != RESET_SYSTEM_PROMPT_PACK_CONFIRMATION {
        return Err(AppError::BadRequest(
            "prompt_pack_reset_confirmation_mismatch".to_string(),
        ));
    }
    Ok(())
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
    // #2 修复：create 与 update 对齐，过字面双闸（禁用词 + 锚完整性）。
    // create 是写入全新整篇内容，对整篇过双闸语义正确；不加 LLM 第三闸
    //（无 old 基线做 diff，且该 draft 最终须经 publish，publish 关口兜 LLM 闸）。
    crate::routes::management_prompt_edit::validate_prompt_edit(
        &payload.prompt_key,
        &payload.content,
    )
    .map_err(AppError::BadRequest)?;
    let description = normalize_optional(payload.description);
    let locale =
        normalize_optional(payload.locale).or_else(|| Some(prompts::DEFAULT_LOCALE.to_string()));
    let draft = crate::prompt_template_versions::append_version(
        &state.db,
        &admin.current_workspace,
        crate::prompt_template_versions::NewPromptTemplateVersion {
            prompt_key: &payload.prompt_key,
            agent_kind: &payload.agent_kind,
            layer: &payload.layer,
            title: &payload.title,
            description: description.as_deref(),
            content: &payload.content,
            prompt_pack_version: "custom",
            actor: &admin.user_id,
            seeded_by: "manual",
            locale: locale.as_deref(),
            previous_version: None,
            source_proposal_id: None,
        },
    )
    .await?;
    Ok(Json(json!({
        "id": draft.id.map(|id| id.to_hex()).unwrap_or_default(),
        "version": draft.version,
        "status": draft.status,
    })))
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
    crate::routes::management_prompt_edit::validate_prompt_edit(
        &payload.prompt_key,
        &payload.content,
    )
    .map_err(AppError::BadRequest)?;
    let object_id = parse_object_id(&id)?;
    let source = state
        .db
        .prompt_templates()
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("prompt template not found".to_string()))?;
    if source.prompt_key != payload.prompt_key {
        return Err(AppError::Conflict("prompt_key_is_immutable".to_string()));
    }
    // Task 6.6 第三闸：LLM 红线语义审查（审 diff 增量）。force=true 跳过（管理者已逐字核对）。
    let force = payload.force.unwrap_or(false);
    if !force {
        match crate::routes::management_prompt_edit::review_prompt_edit(
            &state,
            &admin.current_workspace,
            &payload.prompt_key,
            &source.content,
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
    let description = normalize_optional(payload.description);
    let locale = normalize_optional(payload.locale)
        .or(source.locale.clone())
        .or_else(|| Some(prompts::DEFAULT_LOCALE.to_string()));
    let draft = crate::prompt_template_versions::append_edited_draft(
        &state.db,
        &admin.current_workspace,
        object_id,
        crate::prompt_template_versions::NewPromptTemplateVersion {
            prompt_key: &payload.prompt_key,
            agent_kind: &payload.agent_kind,
            layer: &payload.layer,
            title: &payload.title,
            description: description.as_deref(),
            content: &payload.content,
            prompt_pack_version: "custom",
            actor: &admin.user_id,
            seeded_by: "manual",
            locale: locale.as_deref(),
            previous_version: Some(source.version),
            source_proposal_id: None,
        },
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "id": draft.id.map(|id| id.to_hex()).unwrap_or_default(),
        "version": draft.version,
        "status": draft.status,
    })))
}

pub async fn publish_prompt_template(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    body: Option<Json<PublishRequest>>,
) -> AppResult<Json<Value>> {
    let force = body.and_then(|b| b.0.force).unwrap_or(false);
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

    // #2 修复：publish 是 draft→active 的最终生效点，红线最该把守的关口。
    // 闸 1+2 字面双闸（禁词 + 锚完整性），force 不可绕。
    crate::routes::management_prompt_edit::validate_prompt_edit(
        &template.prompt_key,
        &template.content,
    )
    .map_err(AppError::BadRequest)?;

    // 闸 3 LLM 语义审查（审 diff 增量）。force=true 跳过（管理者已逐字核对）。
    if !force {
        let old_content = crate::prompt_template_versions::load_current_for_publish(
            &state.db,
            &template.workspace_id,
            &template.prompt_key,
        )
        .await?
        .map(|row| row.content)
        .unwrap_or_default();
        match crate::routes::management_prompt_edit::review_prompt_edit(
            &state,
            &admin.current_workspace,
            &template.prompt_key,
            &old_content,
            &template.content,
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
                return Ok(Json(json!({
                    "status": "needs_human_confirm",
                    "reason": reason,
                    "diff": diff
                })));
            }
        }
    }

    let published = crate::prompt_template_versions::publish_version(
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
        "id": published.id.map(|id| id.to_hex()).unwrap_or_default(),
        "version": published.version,
    })))
}

pub(super) async fn reset_system_prompt_pack(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<ResetSystemPromptPackRequest>,
) -> AppResult<Json<Value>> {
    // Validate before the first database write. The UI confirmation is a UX
    // guard; this server-side token is the authoritative bypass-resistant gate.
    validate_reset_confirmation(&payload.confirmation)?;
    prompts::reset_prompt_pack_v2_as_actor(
        &state.db,
        &admin.current_workspace,
        &state.config.default_account_id,
        &admin.user_id,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约快照:prompt_template_json。PromptTemplate 18 字段全量构造(description/previous_version/
    /// seeded_by/locale 给 Some);id→Option.map(to_hex).unwrap_or_default();updatedAt→dt_to_string。
    /// 投影下发 13 顶层键(漏发 createdAt/currentVersion/previousVersion/seededBy/locale)。
    #[test]
    fn prompt_template_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let template = PromptTemplate {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            prompt_key: "reply_agent_main".to_string(),
            agent_kind: "reply".to_string(),
            layer: "policy".to_string(),
            title: "回复 Agent 主提示词".to_string(),
            description: Some("主决策层提示词".to_string()),
            content: "你是私域运营助手……".to_string(),
            status: "active".to_string(),
            version: 11,
            prompt_pack_version: "v2".to_string(),
            created_by: "system".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            current_version: true,
            previous_version: Some(10),
            seeded_by: Some("system".to_string()),
            locale: Some("zh-CN".to_string()),
            source_proposal_id: None,
        };
        let value = prompt_template_json(template);
        crate::routes::contract_snapshot::assert_contract_fixture("prompt_template", value);
    }

    #[test]
    fn reset_confirmation_is_exact_and_unknown_fields_are_rejected() {
        assert!(validate_reset_confirmation(RESET_SYSTEM_PROMPT_PACK_CONFIRMATION).is_ok());
        assert!(validate_reset_confirmation("").is_err());
        assert!(validate_reset_confirmation("reset prompt pack").is_err());

        let parsed = serde_json::from_value::<ResetSystemPromptPackRequest>(json!({
            "confirmation": RESET_SYSTEM_PROMPT_PACK_CONFIRMATION
        }));
        assert!(parsed.is_ok());
        let unknown = serde_json::from_value::<ResetSystemPromptPackRequest>(json!({
            "confirmation": RESET_SYSTEM_PROMPT_PACK_CONFIRMATION,
            "force": true
        }));
        assert!(unknown.is_err());
    }
}

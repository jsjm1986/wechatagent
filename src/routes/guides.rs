//! 用户运营引导路由：自然语言指令转配置预览与确认应用。

use axum::{extract::State, Json};
use mongodb::bson::{doc, DateTime, Document};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    error::{AppError, AppResult},
    models::{ApiContact, UserOperationGuidePreview},
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GuidePreviewRequest {
    account_id: String,
    contact_id: String,
    instruction: String,
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GuideApplyRequest {
    preview_id: String,
}

pub(super) async fn preview_user_operation_guide(
    State(state): State<AppState>,
    Json(payload): Json<GuidePreviewRequest>,
) -> AppResult<Json<Value>> {
    if payload.instruction.trim().is_empty() {
        return Err(AppError::BadRequest("instruction is required".to_string()));
    }
    validate_account(&state, &payload.account_id).await?;
    let contact = find_contact_by_id(&state, &payload.contact_id).await?;
    if contact.account_id != payload.account_id {
        return Err(AppError::BadRequest(
            "contact does not belong to account".to_string(),
        ));
    }
    let memory = ensure_operating_memory(&state, &contact).await?;
    let latest_review = latest_decision_review(&state, &contact).await?;
    let playbook = agent::load_operation_playbook_for_contact(&state, &contact).await?;
    let health = operation_health_json(&contact, &memory, latest_review.as_ref());
    let system = "你是微信私域用户运营产品里的 AI 引导助手。你的职责不是直接写聊天回复，而是根据运营人员的自然语言指令，生成一份可确认的配置修改预览。必须输出严格 JSON。";
    let user = build_guide_preview_prompt(
        &payload.instruction,
        payload.mode.as_deref().unwrap_or("smart"),
        &contact,
        &memory,
        playbook.as_ref(),
        latest_review.as_ref(),
        &health,
    );
    let generated = agent::generate_agent_json(
        &state,
        Some(&payload.account_id),
        Some(&contact.wxid),
        None,
        "user.guide.preview",
        system,
        &user,
    )
    .await?;
    let summary = json_string_any(&generated, &["summary"])
        .unwrap_or_else(|| "已生成运营优化预览。".to_string());
    let impact_scope = json_string_any(&generated, &["impactScope", "impact_scope"])
        .unwrap_or_else(|| "current_contact".to_string());
    let scope_reason = json_string_any(&generated, &["scopeReason", "scope_reason"])
        .unwrap_or_else(|| "默认只影响当前好友，确认后不会改动其他用户。".to_string());
    let health_scores = json_document_any(&generated, &["healthScores", "health_scores"])
        .unwrap_or_else(|| health_scores_document(&contact, &memory, latest_review.as_ref()));
    let suggested_changes =
        json_document_any(&generated, &["suggestedChanges", "suggested_changes"])
            .unwrap_or_else(Document::new);
    let readable_changes =
        json_string_vec_any(&generated, &["readableChanges", "readable_changes"]);
    let risk_warnings = json_string_vec_any(&generated, &["riskWarnings", "risk_warnings"]);
    let preview = UserOperationGuidePreview {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: payload.account_id,
        contact_id: contact
            .id
            .ok_or_else(|| AppError::External("contact id missing".to_string()))?,
        contact_wxid: contact.wxid,
        instruction: payload.instruction,
        mode: payload.mode.unwrap_or_else(|| "smart".to_string()),
        status: "pending".to_string(),
        summary,
        impact_scope,
        scope_reason,
        readable_changes,
        health_scores,
        suggested_changes,
        risk_warnings,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state
        .db
        .user_operation_guide_previews()
        .insert_one(preview, None)
        .await?;
    let id = result
        .inserted_id
        .as_object_id()
        .ok_or_else(|| AppError::External("guide preview id missing".to_string()))?;
    let stored = state
        .db
        .user_operation_guide_previews()
        .find_one(doc! { "_id": id }, None)
        .await?
        .ok_or_else(|| AppError::External("guide preview missing after insert".to_string()))?;
    Ok(Json(json!({ "item": guide_preview_json(stored) })))
}

pub(super) async fn apply_user_operation_guide(
    State(state): State<AppState>,
    Json(payload): Json<GuideApplyRequest>,
) -> AppResult<Json<Value>> {
    let preview_id = parse_object_id(&payload.preview_id)?;
    let preview = state
        .db
        .user_operation_guide_previews()
        .find_one(doc! { "_id": preview_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("guide preview not found".to_string()))?;
    if preview.status != "pending" {
        return Err(AppError::BadRequest(
            "guide preview is not pending".to_string(),
        ));
    }
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "_id": preview.contact_id,
                "workspace_id": &preview.workspace_id,
                "account_id": &preview.account_id
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("contact not found".to_string()))?;
    apply_contact_changes(&state, &contact, &preview.suggested_changes).await?;
    apply_memory_changes(&state, &contact, &preview.suggested_changes).await?;
    apply_playbook_changes(&state, &contact, &preview.suggested_changes).await?;
    apply_domain_changes(&state, &preview.suggested_changes).await?;
    state
        .db
        .user_operation_guide_previews()
        .update_one(
            doc! { "_id": preview_id },
            doc! { "$set": { "status": "applied", "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: preview.workspace_id.clone(),
                account_id: preview.account_id.clone(),
                contact_wxid: Some(preview.contact_wxid.clone()),
                kind: "user_operation_guide_applied".to_string(),
                status: "succeeded".to_string(),
                summary: preview.summary.clone(),
                details: Some(doc! {
                    "previewId": payload.preview_id,
                    "instruction": preview.instruction,
                    "impactScope": preview.impact_scope,
                    "scopeReason": preview.scope_reason,
                    "readableChanges": preview.readable_changes,
                    "suggestedChanges": preview.suggested_changes
                }),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    let updated_contact = state
        .db
        .contacts()
        .find_one(doc! { "_id": preview.contact_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("contact not found after guide apply".to_string()))?;
    let memory = ensure_operating_memory(&state, &updated_contact).await?;
    let latest_review = latest_decision_review(&state, &updated_contact).await?;
    let health = operation_health_json(&updated_contact, &memory, latest_review.as_ref());
    Ok(Json(json!({
        "item": {
            "contact": ApiContact::from(updated_contact),
            "operatingMemory": operating_memory_json(memory),
            "health": health
        }
    })))
}

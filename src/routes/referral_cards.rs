//! 专属顾问名片库路由：名片的列表、新增、审核与启停。
//!
//! 红线：AI 不自我核验。新建名片默认 `review_status="draft"` + `enabled=false`，
//! 必须管理员显式审核（approved）并启用后，AI 才能在引荐场景选用该名片。

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::ReferralCard,
};

use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReferralCardRequest {
    account_id: Option<String>,
    target_wxid: String,
    display_name: String,
    #[serde(default)]
    send_trigger_hint: String,
    #[serde(default)]
    target_stages: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReferralCardReviewRequest {
    status: String,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReferralCardToggleRequest {
    enabled: bool,
}

/// 新增名片：始终落 `review_status="draft"` + `enabled=false`（AI 不自我核验红线）。
pub(super) async fn create_referral_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<ReferralCardRequest>,
) -> AppResult<Json<Value>> {
    if payload.target_wxid.trim().is_empty() || payload.display_name.trim().is_empty() {
        return Err(AppError::BadRequest(
            "targetWxid and displayName are required".to_string(),
        ));
    }
    // 缺口 6：归一 target_stages 到 canonical（与 contact.customer_stage 同空间），
    // 越界即 400。account_id 缺失 → 空串走 global scope。
    let scope = payload.account_id.as_deref().unwrap_or("");
    let target_stages = crate::agent::dimension_registry::normalize_target_stages(
        &state.db,
        scope,
        &payload.target_stages,
    )
    .await
    .map_err(|reason| AppError::BadRequest(format!("target_stages 校验未通过：{reason}")))?;
    let card = ReferralCard {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: payload.account_id,
        target_wxid: payload.target_wxid,
        display_name: payload.display_name,
        send_trigger_hint: payload.send_trigger_hint,
        target_stages,
        // 红线：创建即草稿且禁用，必须管理员审核+启用后 AI 才可引荐。
        enabled: false,
        review_status: "draft".to_string(),
        review_note: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state.db.referral_cards().insert_one(card, None).await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

/// 列出当前 workspace 下的全部名片。
pub(super) async fn list_referral_cards(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let filter = doc! { "workspace_id": &admin.current_workspace };
    let mut cursor = state.db.referral_cards().find(filter, None).await?;
    let mut items = Vec::new();
    while let Some(card) = cursor.try_next().await? {
        items.push(json!({
            "id": card.id.map(|id| id.to_hex()).unwrap_or_default(),
            "workspaceId": card.workspace_id,
            "accountId": card.account_id,
            "targetWxid": card.target_wxid,
            "displayName": card.display_name,
            "sendTriggerHint": card.send_trigger_hint,
            "targetStages": card.target_stages,
            "enabled": card.enabled,
            "reviewStatus": card.review_status,
            "reviewNote": card.review_note,
            "createdAt": crate::models::dt_to_string(card.created_at),
            "updatedAt": crate::models::dt_to_string(card.updated_at)
        }));
    }
    Ok(Json(json!({ "items": items })))
}

/// 审核名片：status ∈ {"approved","draft"}。filter 带 workspace_id 防越权。
pub(super) async fn review_referral_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ReferralCardReviewRequest>,
) -> AppResult<Json<Value>> {
    if payload.status != "approved" && payload.status != "draft" {
        return Err(AppError::BadRequest(
            "status must be 'approved' or 'draft'".to_string(),
        ));
    }
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("invalid card id".to_string()))?;
    let result = state
        .db
        .referral_cards()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": {
                "review_status": &payload.status,
                "review_note": payload.note.clone(),
                "updated_at": DateTime::now(),
            }},
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::BadRequest("card not found".to_string()));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 启停名片。filter 带 workspace_id 防越权。
pub(super) async fn toggle_referral_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ReferralCardToggleRequest>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("invalid card id".to_string()))?;
    let result = state
        .db
        .referral_cards()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": {
                "enabled": payload.enabled,
                "updated_at": DateTime::now(),
            }},
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::BadRequest("card not found".to_string()));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 删除名片。filter 带 workspace_id 防越权。
pub(super) async fn delete_referral_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("invalid card id".to_string()))?;
    let result = state
        .db
        .referral_cards()
        .delete_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?;
    if result.deleted_count == 0 {
        return Err(AppError::BadRequest("card not found".to_string()));
    }
    Ok(Json(json!({ "ok": true })))
}

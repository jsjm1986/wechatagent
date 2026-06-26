//! 数字分身建议链 T8：relationship_type 建议的运营审核路由（admin）。
//!
//! 职责：列表 / approve / reject `relationship_type_suggestions` 记录——LLM
//! 决策时识别出「某 contact 实际是 customer/peer/friend」会 upsert 一条
//! `status=pending` 建议（T6），但**不直接生效**；本路由是「保守闭环」的人在
//! 环路确认环节：运营 approve 才回写 contact 的 `domain_attributes.relationship_type`，
//! LLM 误判不会直接切换运营范式。
//!
//! - `GET /api/admin/relationship-type-suggestions?status=pending`
//!     列待审建议；按 status 过滤（默认 pending），**强制 workspace 隔离**
//!     （relationship_type_suggestions 是 contact 级、带 workspace_id 字段）。
//! - `POST /api/admin/relationship-type-suggestions/:id/approve`
//!     行为：
//!       1. 读 suggestion（必须 `status=pending`，且属当前 workspace）。
//!       2. `validate_dimension_value(relationship_type, AdminWrite)` 校验
//!          `suggested_value`——approve 是运营权威确认动作=AdminWrite，越界恒
//!          Reject → 返 400，不写 contact。取 Accept 的 canonical 值。
//!       3. `find_contact_by_id`（workspace 隔离）→ 写
//!          `domain_attributes.relationship_type = canonical`。
//!       4. mark suggestion `status="approved"` + reviewed_at + reviewed_by。
//! - `POST /api/admin/relationship-type-suggestions/:id/reject`
//!     body: `{ reason }` —— 写 `rejection_reason` 并 `status="rejected"`。
//!
//! 注意：approve 涉及「写 contact + 改 suggestion 状态」两步，MongoDB 单机部署
//! 不支持事务，这里采用「先写 contact → 再改建议状态」的最佳努力顺序写。先写
//! contact 是因为它才是业务生效点；若改建议状态失败，建议仍为 pending，下次
//! approve 会重新校验并幂等地把 contact 写成同一 canonical 值（$set 幂等）。

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};
use mongodb::options::FindOptions;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent::dimension_registry::{validate_dimension_value, DimValidation, WriteIntent},
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::RelationshipTypeSuggestion,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListSuggestionsQuery {
    /// 默认只看 `pending`；前端可显式传 `approved` / `rejected` / `all` 看历史。
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApproveSuggestionRequest {
    /// 可选：操作人标识（一般是 admin email / id），落入 `reviewed_by`。
    #[serde(default)]
    reviewed_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RejectSuggestionRequest {
    reason: String,
    #[serde(default)]
    reviewed_by: Option<String>,
}

pub(super) async fn list_relationship_suggestions(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ListSuggestionsQuery>,
) -> AppResult<Json<Value>> {
    // workspace 隔离：suggestion 是 contact 级、带 workspace_id，必须按当前
    // 登录态 workspace 过滤，绝不跨 workspace 暴露他人建议。
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("pending");
    if status != "all" {
        filter.insert("status", status);
    }

    let mut cursor = state
        .db
        .collection_relationship_type_suggestions()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "last_seen_at": -1 })
                .limit(500)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(relationship_suggestion_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn approve_relationship_suggestion(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ApproveSuggestionRequest>,
) -> Result<Response, AppError> {
    // 抽内部 fn 后 REST handler 只负责包 Response，业务逻辑全在 inner——
    // 与管理 Agent 工具分支共用同一份校验 + 写 contact + 改状态流程，行为等价。
    let value =
        approve_relationship_suggestion_inner(&state, &admin.current_workspace, &id, payload).await?;
    Ok(Json(value).into_response())
}

/// approve_relationship_suggestion 的内部核心：workspace 隔离读 suggestion →
/// AdminWrite 校验 suggested_value → 写 contact 的 domain_attributes.relationship_type
/// → mark 建议 approved → 返回 `{"item": <suggestion json>}`。
/// 跨 workspace / 不存在的 _id 返 NotFound（不泄漏存在性）。
pub(in crate::routes) async fn approve_relationship_suggestion_inner(
    state: &AppState,
    workspace_id: &str,
    id: &str,
    payload: ApproveSuggestionRequest,
) -> AppResult<Value> {
    let object_id = parse_object_id(id)?;
    let suggestions = state.db.collection_relationship_type_suggestions();
    // 查询带 workspace 过滤：跨 workspace 的 _id 返回 NotFound（不泄漏存在性）。
    let suggestion = suggestions
        .find_one(
            doc! { "_id": object_id, "workspace_id": workspace_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("suggestion not found".to_string()))?;
    if suggestion.status != "pending" {
        return Err(AppError::BadRequest(format!(
            "建议状态 = {}，仅 status=pending 可 approve",
            suggestion.status
        )));
    }

    // approve = 运营权威确认 = AdminWrite：suggested_value 越界恒 Reject 当场报错。
    let canonical = match validate_dimension_value(
        &state.db,
        "relationship_type",
        &suggestion.suggested_value,
        &suggestion.account_id,
        WriteIntent::AdminWrite,
    )
    .await
    {
        DimValidation::Accept(canonical) => canonical,
        DimValidation::Reject(reason) => {
            return Err(AppError::BadRequest(format!(
                "suggested_value {:?} 校验未通过：{reason}",
                suggestion.suggested_value
            )));
        }
        DimValidation::DropSilently => {
            return Err(AppError::BadRequest(
                "suggested_value 为空，无法 approve".to_string(),
            ));
        }
    };

    // workspace 隔离取 contact——确认建议指向的 contact 仍在当前 workspace
    // （跨 workspace / 不存在均 404，不写 contact）。
    find_contact_by_id(state, workspace_id, &suggestion.contact_id).await?;
    let contact_oid = parse_object_id(&suggestion.contact_id)?;

    let now = DateTime::now();
    // 第 1 步（业务生效点）：写 contact 的 domain_attributes.relationship_type。
    // 点路径 $set 只覆盖该一个键，不动其它 domain_attributes。
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact_oid, "workspace_id": workspace_id },
            doc! {
                "$set": {
                    "domain_attributes.relationship_type": &canonical,
                    "updated_at": now
                }
            },
            None,
        )
        .await?;

    // 第 2 步：mark 建议 approved。失败时 contact 已生效，建议仍 pending，下次
    // approve 会重新校验并幂等写同值，业务不丢。
    suggestions
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "status": "approved",
                    "reviewed_at": now,
                    "reviewed_by": payload.reviewed_by.as_deref().unwrap_or("admin")
                }
            },
            None,
        )
        .await?;

    let updated = suggestions
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("suggestion not found".to_string()))?;
    Ok(json!({ "item": relationship_suggestion_json(updated) }))
}

pub(super) async fn reject_relationship_suggestion(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<RejectSuggestionRequest>,
) -> AppResult<Json<Value>> {
    if payload.reason.trim().is_empty() {
        return Err(AppError::BadRequest("reason 不能为空".to_string()));
    }
    let object_id = parse_object_id(&id)?;
    let suggestions = state.db.collection_relationship_type_suggestions();
    let now = DateTime::now();
    let result = suggestions
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "status": "pending"
            },
            doc! {
                "$set": {
                    "status": "rejected",
                    "reviewed_at": now,
                    "reviewed_by": payload.reviewed_by.as_deref().unwrap_or("admin"),
                    "rejection_reason": payload.reason.trim()
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "suggestion not found or not pending".to_string(),
        ));
    }
    let updated = suggestions
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("suggestion not found".to_string()))?;
    Ok(Json(json!({ "item": relationship_suggestion_json(updated) })))
}

pub(super) fn relationship_suggestion_json(item: RelationshipTypeSuggestion) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "contactId": item.contact_id,
        "suggestedValue": item.suggested_value,
        "evidence": item.evidence,
        "confidence": item.confidence,
        "occurrences": item.occurrences,
        "status": item.status,
        "firstSeenAt": crate::models::dt_to_string(item.first_seen_at),
        "lastSeenAt": crate::models::dt_to_string(item.last_seen_at),
        "reviewedAt": item.reviewed_at.and_then(crate::models::dt_to_string),
        "reviewedBy": item.reviewed_by
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;

    fn sample_suggestion(status: &str) -> RelationshipTypeSuggestion {
        RelationshipTypeSuggestion {
            id: Some(ObjectId::new()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_id: "507f1f77bcf86cd799439011".to_string(),
            suggested_value: "peer".to_string(),
            evidence: Some("用户自称同行".to_string()),
            confidence: 7,
            status: status.to_string(),
            occurrences: 2,
            first_seen_at: DateTime::now(),
            last_seen_at: DateTime::now(),
            reviewed_at: None,
            reviewed_by: None,
        }
    }

    /// T8：建议 JSON 形状稳定。
    #[test]
    fn suggestion_json_shape_is_stable() {
        let s = sample_suggestion("pending");
        let id_hex = s.id.unwrap().to_hex();
        let value = relationship_suggestion_json(s);
        assert_eq!(value["id"], id_hex);
        assert_eq!(value["workspaceId"], "ws-1");
        assert_eq!(value["accountId"], "acc-1");
        assert_eq!(value["contactId"], "507f1f77bcf86cd799439011");
        assert_eq!(value["suggestedValue"], "peer");
        assert_eq!(value["evidence"], "用户自称同行");
        assert_eq!(value["confidence"], 7);
        assert_eq!(value["occurrences"], 2);
        assert_eq!(value["status"], "pending");
        assert!(value["firstSeenAt"].is_string());
        assert!(value["lastSeenAt"].is_string());
        assert!(value["reviewedAt"].is_null());
    }

    /// T8：默认 list query 不传 status 时 handler 内部解析为 "pending"。
    #[test]
    fn list_query_defaults_to_pending() {
        let q: ListSuggestionsQuery = serde_json::from_value(json!({})).unwrap();
        let resolved = q
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("pending");
        assert_eq!(resolved, "pending");
    }

    /// T8：approve 请求 reviewed_by 可缺省（serde default）。
    #[test]
    fn approve_request_reviewed_by_optional() {
        let req: ApproveSuggestionRequest = serde_json::from_value(json!({})).unwrap();
        assert!(req.reviewed_by.is_none());
        let req2: ApproveSuggestionRequest =
            serde_json::from_value(json!({ "reviewedBy": "alice@corp" })).unwrap();
        assert_eq!(req2.reviewed_by.as_deref(), Some("alice@corp"));
    }

    /// T8：reject 请求要求 `reason` 字段（serde 默认 missing 报错）。
    #[test]
    fn reject_request_requires_reason() {
        let parsed: Result<RejectSuggestionRequest, _> = serde_json::from_value(json!({}));
        assert!(parsed.is_err(), "缺少 reason 应该被 serde 拒绝");
        let ok: RejectSuggestionRequest =
            serde_json::from_value(json!({ "reason": "误判，实际是普通客户" })).unwrap();
        assert_eq!(ok.reason, "误判，实际是普通客户");
    }
}

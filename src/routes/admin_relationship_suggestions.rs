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
//!     行为（validate-first + transaction）：
//!       1. 读 suggestion（必须 `status=pending`，且属当前 workspace）。
//!       2. `validate_dimension_value(relationship_type, AdminWrite)` 校验
//!          `suggested_value`——approve 是运营权威确认动作=AdminWrite，越界恒
//!          Reject → 返 400，不写 contact。取 Accept 的 canonical 值。
//!       3. 事务内以完整 suggestion 快照 + `status=pending` CAS，同时写 contact 的
//!          `domain_attributes.relationship_type = canonical` 并终结 approved。
//! - `POST /api/admin/relationship-type-suggestions/:id/reject`
//!     body: `{ reason }` —— 写 `rejection_reason` 并 `status="rejected"`。
//!
//! 该路径要求 MongoDB replica set。建议 CAS、联系人写入或 commit 任一步失败均不留下
//! 「联系人已生效但建议仍 pending」或「建议 approved 但联系人未更新」的分裂状态。

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};
use mongodb::options::{FindOptions, TransactionOptions};
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
pub(super) struct ApproveSuggestionRequest {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RejectSuggestionRequest {
    reason: String,
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
    Json(_payload): Json<ApproveSuggestionRequest>,
) -> Result<Response, AppError> {
    // 抽内部 fn 后 REST handler 只负责包 Response，业务逻辑全在 inner——
    // 与管理 Agent 工具分支共用同一份校验 + 写 contact + 改状态流程，行为等价。
    let actor = ReviewActor::from_admin(&admin)?;
    let value =
        approve_relationship_suggestion_inner(&state, &admin.current_workspace, &id, actor).await?;
    Ok(Json(value).into_response())
}

/// approve_relationship_suggestion 的内部核心：workspace 隔离读 suggestion →
/// AdminWrite 校验 suggested_value → 事务内 snapshot CAS + contact 写入 + approved 终态
/// → 返回 `{"item": <suggestion json>}`。
/// 跨 workspace / 不存在的 _id 返 NotFound（不泄漏存在性）。
pub(in crate::routes) async fn approve_relationship_suggestion_inner(
    state: &AppState,
    workspace_id: &str,
    id: &str,
    actor: ReviewActor,
) -> AppResult<Value> {
    let object_id = parse_object_id(id)?;
    let suggestions = state.db.collection_relationship_type_suggestions();
    // 查询带 workspace 过滤：跨 workspace 的 _id 返回 NotFound（不泄漏存在性）。
    let mut suggestion = suggestions
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
        workspace_id,
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
    let contact = find_contact_by_id(state, workspace_id, &suggestion.contact_id).await?;
    if contact.account_id != suggestion.account_id {
        return Err(AppError::Conflict(
            "relationship_suggestion_contact_identity_changed".to_string(),
        ));
    }
    let contact_oid = parse_object_id(&suggestion.contact_id)?;

    let now = DateTime::now();
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let transaction_result: AppResult<()> = async {
        // Bind approval to the exact object that was validated. A concurrent gateway refresh
        // changing value/contact/account makes this CAS fail instead of approving stale evidence.
        let claimed = suggestions
            .update_one_with_session(
                doc! {
                    "_id": object_id,
                    "workspace_id": workspace_id,
                    "account_id": &suggestion.account_id,
                    "contact_id": &suggestion.contact_id,
                    "suggested_value": &suggestion.suggested_value,
                    "last_seen_at": suggestion.last_seen_at,
                    "status": "pending",
                },
                doc! {
                    "$set": {
                        "status": "approved",
                        "reviewed_at": now,
                        "reviewed_by": actor.as_str(),
                    }
                },
                None,
                &mut session,
            )
            .await?;
        if claimed.modified_count != 1 {
            return Err(AppError::Conflict(
                "relationship_suggestion_not_pending_or_changed".to_string(),
            ));
        }

        // Merge the one reviewed field into the container. `$ifNull` supports legacy rows with
        // an absent or explicit-null domain_attributes value; malformed non-document values fail
        // the transaction instead of being silently replaced.
        let contact_update = state
            .db
            .contacts()
            .update_one_with_session(
                doc! {
                    "_id": contact_oid,
                    "workspace_id": workspace_id,
                    "account_id": &suggestion.account_id,
                },
                vec![doc! {
                    "$set": {
                        "domain_attributes": {
                            "$mergeObjects": [
                                { "$ifNull": ["$domain_attributes", {}] },
                                { "relationship_type": &canonical },
                            ]
                        },
                        "domain_attributes_updated_at": now,
                        "updated_at": now,
                    }
                }],
                None,
                &mut session,
            )
            .await?;
        if contact_update.matched_count != 1 {
            return Err(AppError::Conflict(
                "relationship_suggestion_contact_changed".to_string(),
            ));
        }
        Ok(())
    }
    .await;
    if let Err(error) = transaction_result {
        let _ = session.abort_transaction().await;
        return Err(error);
    }
    loop {
        match session.commit_transaction().await {
            Ok(()) => break,
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => return Err(error.into()),
        }
    }

    suggestion.status = "approved".to_string();
    suggestion.reviewed_at = Some(now);
    suggestion.reviewed_by = Some(actor.as_str().to_string());
    Ok(json!({ "item": relationship_suggestion_json(suggestion) }))
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
    let actor = ReviewActor::from_admin(&admin)?;
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
                    "reviewed_by": actor.as_str(),
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
    Ok(Json(
        json!({ "item": relationship_suggestion_json(updated) }),
    ))
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

    /// SR-058：旧客户端即使提交 reviewedBy，也不会进入请求模型或覆盖可信 actor。
    #[test]
    fn approve_request_ignores_untrusted_reviewed_by() {
        let _: ApproveSuggestionRequest = serde_json::from_value(json!({})).unwrap();
        let _: ApproveSuggestionRequest =
            serde_json::from_value(json!({ "reviewedBy": "spoofed@corp" })).unwrap();
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

    /// 契约快照:relationship_suggestion_json。RelationshipTypeSuggestion 13 字段全量构造
    /// (evidence/reviewed_at/reviewed_by 三个 Option 给 Some);id→hex;
    /// first_seen_at/last_seen_at→RFC3339;reviewed_at→Some 后 RFC3339。投影下发全部 13 键。
    #[test]
    fn relationship_suggestion_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let item = RelationshipTypeSuggestion {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_id: "507f1f77bcf86cd799439012".to_string(),
            suggested_value: "peer".to_string(),
            evidence: Some("用户自称同行".to_string()),
            confidence: 7,
            status: "pending".to_string(),
            occurrences: 2,
            first_seen_at: DateTime::from_millis(1_700_000_000_000),
            last_seen_at: DateTime::from_millis(1_700_000_100_000),
            reviewed_at: Some(DateTime::from_millis(1_700_000_200_000)),
            reviewed_by: Some("admin-1".to_string()),
        };
        let value = relationship_suggestion_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("relationship_suggestion", value);
    }
}

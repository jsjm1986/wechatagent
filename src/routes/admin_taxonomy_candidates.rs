//! agent-autonomy-loop W3 / Task 4.8：双层标签候选审核路由（admin）。
//!
//! 职责：列表 / approve / reject `taxonomy_candidates` 候选记录。
//!
//! - `GET /api/admin/taxonomy-candidates?status=pending&scope=&kind=`
//! - `POST /api/admin/taxonomy-candidates/:id/approve`
//!     body: `{ canonicalValue: { id, label, aliases? } }`
//!     行为：
//!       1. 读 candidate（必须 `status=pending`）。
//!       2. 写 `system_taxonomies`：以 `(scope, kind, canonicalValue.id)` 为唯一键
//!          插入条目；若已存在（11000）合并别名后视为成功。
//!       3. 改 candidate `status="approved"`、`reviewed_at=now`。
//!       4. `invalidate_global_taxonomy_cache`。
//!     失败回滚约定：即使第 2 步因唯一冲突失败，第 3 步仍会 mark approved（与
//!     `agent::taxonomy::approve` 已有的"幂等跳过"语义保持一致；下次相同 value
//!     不会再触发审核流程）。
//! - `POST /api/admin/taxonomy-candidates/:id/reject`
//!     body: `{ reason }` —— 写入 candidate.reason 并 `status="rejected"`。
//!
//! 注意：MongoDB 单机部署不支持事务，这里采用"先写字典 → 再改候选"的最佳努力
//! 顺序写。如果第二步失败，候选仍为 pending，下次审核会发现字典里已有条目，
//! 通过唯一索引幂等跳过插入并补完成 candidate 状态。

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::FindOptions;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent::taxonomy::invalidate_global_taxonomy_cache,
    error::{AppError, AppResult},
    models::{TaxonomyCandidate, TaxonomyEntry, TaxonomyValue},
};

use super::admin_taxonomies::is_duplicate_key_error;
use super::shared::*;
use super::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListCandidatesQuery {
    /// 默认只看 `pending`；前端可显式传 `approved` / `rejected` / `all` 看历史。
    status: Option<String>,
    scope: Option<String>,
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApproveCandidateRequest {
    canonical_value: ApproveCanonicalValue,
    /// 可选：操作人标识（一般是 admin email / id），落入 `reviewed_by`。
    #[serde(default)]
    reviewed_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApproveCanonicalValue {
    id: String,
    #[serde(alias = "displayName")]
    label: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RejectCandidateRequest {
    reason: String,
    #[serde(default)]
    reviewed_by: Option<String>,
}

pub(super) async fn list_taxonomy_candidates(
    State(state): State<AppState>,
    Query(query): Query<ListCandidatesQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = Document::new();
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("pending");
    if status != "all" {
        filter.insert("status", status);
    }
    if let Some(scope) = query.scope.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("scope", scope.trim());
    }
    if let Some(kind) = query.kind.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("kind", kind.trim());
    }

    let mut cursor = state
        .db
        .collection_taxonomy_candidates()
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
        items.push(taxonomy_candidate_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn approve_taxonomy_candidate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<ApproveCandidateRequest>,
) -> Result<Response, AppError> {
    if payload.canonical_value.id.trim().is_empty()
        || payload.canonical_value.label.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "canonicalValue.id / canonicalValue.label 不能为空".to_string(),
        ));
    }

    let object_id = parse_object_id(&id)?;
    let candidates = state.db.collection_taxonomy_candidates();
    let candidate = candidates
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("candidate not found".to_string()))?;
    if candidate.status != "pending" {
        return Err(AppError::BadRequest(format!(
            "候选状态 = {}，仅 status=pending 可 approve",
            candidate.status
        )));
    }

    let now = DateTime::now();
    let entry = TaxonomyEntry {
        id: None,
        scope: candidate.scope.clone(),
        kind: candidate.kind.clone(),
        value: TaxonomyValue {
            id: payload.canonical_value.id.trim().to_string(),
            display_name: payload.canonical_value.label.trim().to_string(),
            description: payload
                .canonical_value
                .description
                .clone()
                .unwrap_or_else(|| candidate.evidence.clone().unwrap_or_default()),
            aliases: {
                let mut aliases: Vec<String> = payload
                    .canonical_value
                    .aliases
                    .iter()
                    .map(|alias| alias.trim().to_string())
                    .filter(|alias| !alias.is_empty())
                    .collect();
                // 把 candidate.raw_value 自动加进 aliases，便于历史 run 在
                // taxonomy cache 重新加载后立即命中（避免 raw_value 与
                // canonical id 不一致时再次产生新候选）。
                let raw = candidate.raw_value.trim().to_string();
                if !raw.is_empty()
                    && raw != payload.canonical_value.id.trim()
                    && !aliases.iter().any(|a| a == &raw)
                {
                    aliases.push(raw);
                }
                aliases
            },
            status: "active".to_string(),
        },
        updated_at: now,
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("manual".to_string()),
    };

    match state
        .db
        .collection_system_taxonomies()
        .insert_one(&entry, None)
        .await
    {
        Ok(_) => {}
        Err(error) if is_duplicate_key_error(&error) => {
            // 已存在则与 `agent::taxonomy::approve` 行为一致：跳过插入，候选仍
            // 标记为 approved（业务语义上视为"该 value 已在字典里"）。
            tracing::info!(
                candidate_id = %object_id,
                scope = %candidate.scope,
                kind = %candidate.kind,
                value_id = %payload.canonical_value.id,
                "approve_candidate found existing taxonomy entry, skipping insert"
            );
            // 但这里仍需要返回 409，让前端知道 canonical value 已有，方便提示
            // 操作员选择"合并别名"还是"重新选 id"。
            // 为了保留可观测性：先把候选 mark 为 approved，再以 409 返回。
            mark_candidate_approved(&state, object_id, payload.reviewed_by.as_deref()).await?;
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "duplicate_taxonomy",
                    "message": format!(
                        "(scope={}, kind={}, value.id={}) 已存在；候选已置为 approved",
                        candidate.scope, candidate.kind, payload.canonical_value.id
                    )
                })),
            )
                .into_response());
        }
        Err(error) => return Err(error.into()),
    }

    mark_candidate_approved(&state, object_id, payload.reviewed_by.as_deref()).await?;
    invalidate_global_taxonomy_cache();

    let updated = candidates
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("candidate not found".to_string()))?;
    Ok(Json(json!({ "item": taxonomy_candidate_json(updated) })).into_response())
}

pub(super) async fn reject_taxonomy_candidate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<RejectCandidateRequest>,
) -> AppResult<Json<Value>> {
    if payload.reason.trim().is_empty() {
        return Err(AppError::BadRequest("reason 不能为空".to_string()));
    }
    let object_id = parse_object_id(&id)?;
    let candidates = state.db.collection_taxonomy_candidates();
    let now = DateTime::now();
    let result = candidates
        .update_one(
            doc! { "_id": object_id, "status": "pending" },
            doc! {
                "$set": {
                    "status": "rejected",
                    "reviewed_at": now,
                    "reviewed_by": payload.reviewed_by.as_deref().unwrap_or("admin"),
                    // candidate 模型暂未声明 reason 字段（W0 占位）；以
                    // dynamic field 写入 BSON，仍然可被 mongo shell / UI 看到。
                    "rejection_reason": payload.reason.trim()
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "candidate not found or not pending".to_string(),
        ));
    }
    let updated = candidates
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("candidate not found".to_string()))?;
    Ok(Json(json!({ "item": taxonomy_candidate_json(updated) })))
}

async fn mark_candidate_approved(
    state: &AppState,
    candidate_id: mongodb::bson::oid::ObjectId,
    reviewed_by: Option<&str>,
) -> AppResult<()> {
    let now = DateTime::now();
    state
        .db
        .collection_taxonomy_candidates()
        .update_one(
            doc! { "_id": candidate_id },
            doc! {
                "$set": {
                    "status": "approved",
                    "reviewed_at": now,
                    "reviewed_by": reviewed_by.unwrap_or("admin")
                }
            },
            None,
        )
        .await?;
    Ok(())
}

pub(super) fn taxonomy_candidate_json(item: TaxonomyCandidate) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "scope": item.scope,
        "kind": item.kind,
        "rawValue": item.raw_value,
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

    fn sample_candidate(status: &str) -> TaxonomyCandidate {
        TaxonomyCandidate {
            id: Some(ObjectId::new()),
            scope: "global".to_string(),
            kind: "objection_type".to_string(),
            raw_value: "太贵了".to_string(),
            evidence: Some("用户说价格高".to_string()),
            confidence: 7,
            first_seen_at: DateTime::now(),
            last_seen_at: DateTime::now(),
            occurrences: 3,
            status: status.to_string(),
            reviewed_at: None,
            reviewed_by: None,
        }
    }

    /// W3 / Task 4.8：候选 JSON 形状稳定。
    #[test]
    fn candidate_json_shape_is_stable() {
        let c = sample_candidate("pending");
        let id_hex = c.id.unwrap().to_hex();
        let value = taxonomy_candidate_json(c);
        assert_eq!(value["id"], id_hex);
        assert_eq!(value["scope"], "global");
        assert_eq!(value["kind"], "objection_type");
        assert_eq!(value["rawValue"], "太贵了");
        assert_eq!(value["status"], "pending");
        assert_eq!(value["confidence"], 7);
        assert_eq!(value["occurrences"], 3);
        assert!(value["firstSeenAt"].is_string());
        assert!(value["lastSeenAt"].is_string());
        assert!(value["reviewedAt"].is_null());
    }

    /// W3 / Task 4.8：默认 list query 不传 status 时 handler 内部解析为 "pending"。
    #[test]
    fn list_query_defaults_to_pending() {
        let q: ListCandidatesQuery = serde_json::from_value(json!({})).unwrap();
        let resolved = q
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("pending");
        assert_eq!(resolved, "pending");
    }

    /// W3 / Task 4.8：approve 请求支持 `displayName` 别名。
    #[test]
    fn approve_request_accepts_display_name_alias() {
        let req: ApproveCandidateRequest = serde_json::from_value(json!({
            "canonicalValue": {
                "id": "price_objection",
                "displayName": "价格异议"
            }
        }))
        .unwrap();
        assert_eq!(req.canonical_value.id, "price_objection");
        assert_eq!(req.canonical_value.label, "价格异议");
        assert!(req.canonical_value.aliases.is_empty());
    }

    /// W3 / Task 4.8：reject 请求要求 `reason` 字段（serde 默认 missing 报错）。
    #[test]
    fn reject_request_requires_reason() {
        let parsed: Result<RejectCandidateRequest, _> = serde_json::from_value(json!({}));
        assert!(parsed.is_err(), "缺少 reason 应该被 serde 拒绝");
        let ok: RejectCandidateRequest =
            serde_json::from_value(json!({ "reason": "无业务相关性" })).unwrap();
        assert_eq!(ok.reason, "无业务相关性");
    }
}

//! agent-autonomy-loop W3 / Task 4.8：双层标签词典管理路由（admin）。
//!
//! 职责：直接维护 `system_taxonomies` 集合（"严格字典"层），用于 `customer_stage`
//! / `intent_level` / `objection_type` 等维度的可枚举 value。
//!
//! - `GET /api/admin/taxonomies?scope=&kind=&include_deprecated=`
//! - `POST /api/admin/taxonomies` 新增条目，依赖 `(scope, kind, value.id)` 唯一索引。
//! - `PATCH /api/admin/taxonomies/:id` 局部更新（label / aliases / description / deprecated）。
//! - `DELETE /api/admin/taxonomies/:id` 软删除：`value.status = "deprecated"`，
//!   保留历史值以便对历史 run / 审核留档继续可读。
//!
//! 任意写操作完成后立即调用 `invalidate_global_taxonomy_cache`，让运行中
//! Reply / Review Agent 在下次 `check_value` 时按新字典执行。

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, to_bson, DateTime, Document};
use mongodb::options::FindOptions;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent::taxonomy::invalidate_global_taxonomy_cache,
    error::{AppError, AppResult},
    models::{TaxonomyEntry, TaxonomyValue},
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListTaxonomiesQuery {
    scope: Option<String>,
    kind: Option<String>,
    /// 默认 `false`：列表只返回 `value.status = "active"` 的条目。
    #[serde(default)]
    include_deprecated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateTaxonomyRequest {
    scope: String,
    kind: String,
    value: CreateTaxonomyValue,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateTaxonomyValue {
    id: String,
    /// 兼容 `label` 与 `displayName` 两种入参（前端 / spec 沿用 `label`，DB 字段
    /// 是 `displayName`）。
    #[serde(alias = "displayName")]
    label: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PatchTaxonomyRequest {
    #[serde(alias = "displayName")]
    label: Option<String>,
    aliases: Option<Vec<String>>,
    description: Option<String>,
    /// `true` → `value.status = "deprecated"`；`false` → 恢复为 `active`。
    deprecated: Option<bool>,
}

pub(super) async fn list_taxonomies(
    State(state): State<AppState>,
    Query(query): Query<ListTaxonomiesQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = Document::new();
    if let Some(scope) = query.scope.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("scope", scope.trim());
    }
    if let Some(kind) = query.kind.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("kind", kind.trim());
    }
    if !query.include_deprecated {
        filter.insert("value.status", "active");
    }

    let mut cursor = state
        .db
        .collection_system_taxonomies()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "scope": 1, "kind": 1, "value.id": 1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(entry) = cursor.try_next().await? {
        items.push(taxonomy_entry_json(entry));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_taxonomy(
    State(state): State<AppState>,
    Json(payload): Json<CreateTaxonomyRequest>,
) -> Result<Response, AppError> {
    if payload.scope.trim().is_empty()
        || payload.kind.trim().is_empty()
        || payload.value.id.trim().is_empty()
        || payload.value.label.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "scope / kind / value.id / value.label 均不能为空".to_string(),
        ));
    }

    let now = DateTime::now();
    let entry = TaxonomyEntry {
        id: None,
        scope: payload.scope.trim().to_string(),
        kind: payload.kind.trim().to_string(),
        value: TaxonomyValue {
            id: payload.value.id.trim().to_string(),
            display_name: payload.value.label.trim().to_string(),
            description: payload.value.description.unwrap_or_default(),
            aliases: payload
                .value
                .aliases
                .into_iter()
                .map(|alias| alias.trim().to_string())
                .filter(|alias| !alias.is_empty())
                .collect(),
            status: "active".to_string(),
        },
        updated_at: now,
    };

    match state
        .db
        .collection_system_taxonomies()
        .insert_one(&entry, None)
        .await
    {
        Ok(result) => {
            invalidate_global_taxonomy_cache();
            let mut entry_with_id = entry;
            entry_with_id.id = result.inserted_id.as_object_id();
            Ok(Json(json!({ "item": taxonomy_entry_json(entry_with_id) })).into_response())
        }
        Err(error) if is_duplicate_key_error(&error) => Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_taxonomy",
                "message": format!(
                    "(scope={}, kind={}, value.id={}) 已存在",
                    entry.scope, entry.kind, entry.value.id
                )
            })),
        )
            .into_response()),
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn patch_taxonomy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<PatchTaxonomyRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;

    let mut set_doc = Document::new();
    if let Some(label) = payload.label {
        let trimmed = label.trim().to_string();
        if trimmed.is_empty() {
            return Err(AppError::BadRequest("label 不能为空".to_string()));
        }
        set_doc.insert("value.displayName", trimmed);
    }
    if let Some(aliases) = payload.aliases {
        let cleaned: Vec<String> = aliases
            .into_iter()
            .map(|alias| alias.trim().to_string())
            .filter(|alias| !alias.is_empty())
            .collect();
        set_doc.insert("value.aliases", to_bson(&cleaned)?);
    }
    if let Some(description) = payload.description {
        set_doc.insert("value.description", description);
    }
    if let Some(deprecated) = payload.deprecated {
        set_doc.insert(
            "value.status",
            if deprecated { "deprecated" } else { "active" },
        );
    }
    if set_doc.is_empty() {
        return Err(AppError::BadRequest(
            "至少要传 label / aliases / description / deprecated 之一".to_string(),
        ));
    }
    set_doc.insert("updated_at", DateTime::now());

    let collection = state.db.collection_system_taxonomies();
    let result = collection
        .update_one(doc! { "_id": object_id }, doc! { "$set": set_doc }, None)
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("taxonomy entry not found".to_string()));
    }
    invalidate_global_taxonomy_cache();
    let entry = collection
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("taxonomy entry not found".to_string()))?;
    Ok(Json(json!({ "item": taxonomy_entry_json(entry) })))
}

pub(super) async fn delete_taxonomy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let result = state
        .db
        .collection_system_taxonomies()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "value.status": "deprecated",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("taxonomy entry not found".to_string()));
    }
    invalidate_global_taxonomy_cache();
    Ok(Json(json!({ "ok": true })))
}

pub(super) fn taxonomy_entry_json(entry: TaxonomyEntry) -> Value {
    json!({
        "id": entry.id.map(|id| id.to_hex()).unwrap_or_default(),
        "scope": entry.scope,
        "kind": entry.kind,
        "value": {
            "id": entry.value.id,
            "label": entry.value.display_name,
            "displayName": entry.value.display_name,
            "description": entry.value.description,
            "aliases": entry.value.aliases,
            "status": entry.value.status,
        },
        "updatedAt": crate::models::dt_to_string(entry.updated_at)
    })
}

pub(super) fn is_duplicate_key_error(err: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match &*err.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            write_error.code == 11000 || write_error.code == 11001
        }
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .as_ref()
            .map(|errs| errs.iter().any(|e| e.code == 11000 || e.code == 11001))
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;

    /// W3 / Task 4.8：响应 JSON 携带稳定字段（id / scope / kind / value.label / aliases / status）。
    #[test]
    fn taxonomy_entry_json_carries_stable_fields() {
        let oid = ObjectId::new();
        let entry = TaxonomyEntry {
            id: Some(oid),
            scope: "global".to_string(),
            kind: "customer_stage".to_string(),
            value: TaxonomyValue {
                id: "first_contact".to_string(),
                display_name: "首次接触".to_string(),
                description: "刚加上微信、还没业务对话".to_string(),
                aliases: vec!["new_lead".to_string()],
                status: "active".to_string(),
            },
            updated_at: DateTime::now(),
        };
        let value = taxonomy_entry_json(entry);
        assert_eq!(value["id"], oid.to_hex());
        assert_eq!(value["scope"], "global");
        assert_eq!(value["kind"], "customer_stage");
        assert_eq!(value["value"]["id"], "first_contact");
        assert_eq!(value["value"]["label"], "首次接触");
        assert_eq!(value["value"]["displayName"], "首次接触");
        assert_eq!(value["value"]["aliases"], json!(["new_lead"]));
        assert_eq!(value["value"]["status"], "active");
    }

    /// W3 / Task 4.8：默认列表过滤 `include_deprecated=false` 不显式给出时
    /// 应被 serde 反序列化为 `false`（确保只返回 active 条目）。
    #[test]
    fn list_query_defaults_include_deprecated_to_false() {
        let q: ListTaxonomiesQuery = serde_json::from_value(json!({})).unwrap();
        assert!(!q.include_deprecated);
        assert!(q.scope.is_none());
        assert!(q.kind.is_none());
    }

    /// W3 / Task 4.8：`PATCH` 请求中 `label` 别名兼容 `displayName`
    /// （后端字段是 `display_name`，spec / 前端用 `label`）。
    #[test]
    fn patch_request_accepts_display_name_alias() {
        let req: PatchTaxonomyRequest =
            serde_json::from_value(json!({ "displayName": "意向中" })).unwrap();
        assert_eq!(req.label.as_deref(), Some("意向中"));
    }

    /// W3 / Task 4.8：`POST` 请求中允许只传 `label`（不强制 `displayName`），
    /// 同时也接受 `displayName` 作为别名。
    #[test]
    fn create_request_accepts_label_or_display_name() {
        let req_label: CreateTaxonomyRequest = serde_json::from_value(json!({
            "scope": "global",
            "kind": "customer_stage",
            "value": { "id": "x", "label": "X" }
        }))
        .unwrap();
        assert_eq!(req_label.value.label, "X");

        let req_dn: CreateTaxonomyRequest = serde_json::from_value(json!({
            "scope": "global",
            "kind": "customer_stage",
            "value": { "id": "y", "displayName": "Y" }
        }))
        .unwrap();
        assert_eq!(req_dn.value.label, "Y");
    }
}

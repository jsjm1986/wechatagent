//! 行业可配 schema admin 路由（knowledge-wiki Phase G）。
//!
//! `domain_schemas` 让产品在不同行业（销售 / 教培 / 医疗 / SaaS / 招聘 ...）下
//! 用同一份 chunk 主表，把行业差异下沉到 `chunks.domain_attributes` JSON 子文档。
//! 同 workspace 同时只能 1 条 `is_active=true`，写入侧按 active schema 校验
//! `domain_attributes`：required 字段缺失 reject、enum 值非法 reject、命中
//! `alias_dict` key 透明 rewrite。
//!
//! 路由（全部挂在 `/api/admin/domain-schemas` 下）：
//!
//! - `GET    /admin/domain-schemas`              列表（按 workspace_id）
//! - `POST   /admin/domain-schemas`              新建（自动 version=既有 max+1）
//! - `PUT    /admin/domain-schemas/:id`          更新（id 是 schema_id slug）
//! - `DELETE /admin/domain-schemas/:id`          删除（不允许删 active 那条）
//! - `POST   /admin/domain-schemas/:id/activate` 切换 active：把同 workspace 其它
//!   active 全部置 false，再把目标置 true。
//!
//! 校验红线（在 [`validate_schema_payload`] 内集中处理）：
//! - `fields.len() <= 64`
//! - `name` 不能与 chunk 主表既有字段名冲突（`base_field_blacklist`）
//! - `alias_dict` 的所有 value 必须存在于 `fields[].name`
//! - `kind ∈ {string, enum, number, date, reference}`
//! - `kind == "enum"` 时必须提供 `allowed_values` 且非空

use axum::{
    extract::{Path, Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, to_bson, DateTime, Document};
use mongodb::options::{FindOneOptions, FindOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    models::{DomainField, DomainSchema},
};

use super::AppState;

/// chunk 主表既有字段名黑名单：domain_schema 自定义字段不可与之冲突。
/// 与 `OperationKnowledgeChunk` 的字段对齐（蛇形命名）。
const BASE_FIELD_BLACKLIST: &[&str] = &[
    "id",
    "_id",
    "chunk_id",
    "workspace_id",
    "document_id",
    "wiki_type",
    "domain_attributes",
    "provenance",
    "valid_from",
    "valid_to",
    "superseded_by",
    "previous_version_id",
    "related_chunks",
    "usage_stats",
    "dynamic_confidence",
    "integrity_score",
    "locked_fields",
    "tags",
    "search_terms",
    "sources",
    "applicable_scenes",
    "answer",
    "explanation",
    "summary",
    "title",
    "status",
    "integrity_status",
    "created_at",
    "updated_at",
    "verified_at",
    "verified_by",
    "approved_at",
    "source_anchor",
    "routing_card",
];

const ALLOWED_KINDS: &[&str] = &["string", "enum", "number", "date", "reference"];

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListQuery {
    workspace_id: Option<String>,
    /// 仅返 active：默认 false（admin 看历史版本）。
    #[serde(default)]
    active_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpsertRequest {
    pub workspace_id: Option<String>,
    pub schema_id: String,
    pub name: String,
    #[serde(default)]
    pub fields: Vec<DomainFieldPayload>,
    #[serde(default)]
    pub alias_dict: serde_json::Value,
    #[serde(default)]
    pub guard_dsl: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DomainFieldPayload {
    pub name: String,
    pub label: String,
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub allowed_values: Option<Vec<String>>,
    #[serde(default)]
    pub alias_of: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DomainSchemaView {
    schema_id: String,
    workspace_id: String,
    name: String,
    version: i32,
    fields: Vec<DomainFieldPayload>,
    alias_dict: Value,
    guard_dsl: Option<String>,
    is_active: bool,
    created_at: i64,
    updated_at: i64,
}

impl From<&DomainSchema> for DomainSchemaView {
    fn from(s: &DomainSchema) -> Self {
        let fields = s
            .fields
            .iter()
            .map(|f| DomainFieldPayload {
                name: f.name.clone(),
                label: f.label.clone(),
                kind: f.kind.clone(),
                required: f.required,
                allowed_values: f.allowed_values.clone(),
                alias_of: f.alias_of.clone(),
            })
            .collect();
        let alias_dict = mongodb::bson::Bson::Document(s.alias_dict.clone())
            .into_relaxed_extjson();
        Self {
            schema_id: s.schema_id.clone(),
            workspace_id: s.workspace_id.clone(),
            name: s.name.clone(),
            version: s.version,
            fields,
            alias_dict,
            guard_dsl: s.guard_dsl.clone(),
            is_active: s.is_active,
            created_at: s.created_at.timestamp_millis(),
            updated_at: s.updated_at.timestamp_millis(),
        }
    }
}

pub(super) async fn list_domain_schemas(
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = params
        .workspace_id
        .clone()
        .unwrap_or_else(|| state.config.default_workspace_id.clone());
    let mut filter = doc! { "workspaceId": &workspace_id };
    if params.active_only {
        filter.insert("isActive", true);
    }
    let mut cursor = state
        .db
        .domain_schemas()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "schema_id": 1, "version": -1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(s) = cursor.try_next().await? {
        items.push(DomainSchemaView::from(&s));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_domain_schema(
    State(state): State<AppState>,
    Json(body): Json<UpsertRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id = body
        .workspace_id
        .clone()
        .unwrap_or_else(|| state.config.default_workspace_id.clone());
    if body.schema_id.trim().is_empty() {
        return Err(AppError::BadRequest("schemaId 不能为空".to_string()));
    }
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("name 不能为空".to_string()));
    }
    let (fields, alias_dict_doc) = validate_schema_payload(&body.fields, &body.alias_dict)?;
    // 同 workspace + 同 schema_id 取 max version + 1（同名升级）
    let next_version = next_version_for(&state, &workspace_id, &body.schema_id).await?;
    let now = DateTime::now();
    let cfg = DomainSchema {
        id: None,
        schema_id: body.schema_id.clone(),
        workspace_id: workspace_id.clone(),
        name: body.name.clone(),
        version: next_version,
        fields,
        alias_dict: alias_dict_doc,
        guard_dsl: body.guard_dsl.clone(),
        is_active: false,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .domain_schemas()
        .insert_one(&cfg, None)
        .await
        .map_err(|err| AppError::BadRequest(format!("创建失败: {err}")))?;
    Ok(Json(json!({ "item": DomainSchemaView::from(&cfg) })))
}

pub(super) async fn update_domain_schema(
    State(state): State<AppState>,
    Path(schema_id): Path<String>,
    Json(body): Json<UpsertRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id = body
        .workspace_id
        .clone()
        .unwrap_or_else(|| state.config.default_workspace_id.clone());
    let existing = state
        .db
        .domain_schemas()
        .find_one(
            doc! { "workspaceId": &workspace_id, "schema_id": &schema_id },
            FindOneOptions::builder()
                .sort(doc! { "version": -1 })
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("schema {schema_id} not found")))?;
    let (fields, alias_dict_doc) = validate_schema_payload(&body.fields, &body.alias_dict)?;
    let fields_bson = to_bson(&fields)?;
    let now = DateTime::now();
    let mut update = doc! {
        "name": &body.name,
        "fields": fields_bson,
        "alias_dict": alias_dict_doc,
        "updatedAt": now,
    };
    if let Some(g) = &body.guard_dsl {
        update.insert("guard_dsl", g);
    } else {
        update.insert("guard_dsl", mongodb::bson::Bson::Null);
    }
    state
        .db
        .domain_schemas()
        .update_one(
            doc! { "workspaceId": &workspace_id, "schema_id": &schema_id, "version": existing.version },
            doc! { "$set": update },
            None,
        )
        .await?;
    let refreshed = state
        .db
        .domain_schemas()
        .find_one(
            doc! { "workspaceId": &workspace_id, "schema_id": &schema_id, "version": existing.version },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("schema 更新后未找到".to_string()))?;
    Ok(Json(json!({ "item": DomainSchemaView::from(&refreshed) })))
}

pub(super) async fn delete_domain_schema(
    State(state): State<AppState>,
    Path(schema_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = params
        .workspace_id
        .clone()
        .unwrap_or_else(|| state.config.default_workspace_id.clone());
    let existing = state
        .db
        .domain_schemas()
        .find_one(
            doc! { "workspaceId": &workspace_id, "schema_id": &schema_id },
            FindOneOptions::builder()
                .sort(doc! { "version": -1 })
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("schema {schema_id} not found")))?;
    if existing.is_active {
        return Err(AppError::BadRequest(
            "请先启用其它 schema 再删除当前激活的版本".to_string(),
        ));
    }
    state
        .db
        .domain_schemas()
        .delete_many(
            doc! { "workspaceId": &workspace_id, "schema_id": &schema_id },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn activate_domain_schema(
    State(state): State<AppState>,
    Path(schema_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = params
        .workspace_id
        .clone()
        .unwrap_or_else(|| state.config.default_workspace_id.clone());
    let target = state
        .db
        .domain_schemas()
        .find_one(
            doc! { "workspaceId": &workspace_id, "schema_id": &schema_id },
            FindOneOptions::builder()
                .sort(doc! { "version": -1 })
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("schema {schema_id} not found")))?;
    let now = DateTime::now();
    state
        .db
        .domain_schemas()
        .update_many(
            doc! {
                "workspaceId": &workspace_id,
                "isActive": true,
            },
            doc! { "$set": { "isActive": false, "updatedAt": now } },
            None,
        )
        .await?;
    state
        .db
        .domain_schemas()
        .update_one(
            doc! {
                "workspaceId": &workspace_id,
                "schema_id": &schema_id,
                "version": target.version,
            },
            doc! { "$set": { "isActive": true, "updatedAt": now } },
            None,
        )
        .await?;
    let refreshed = state
        .db
        .domain_schemas()
        .find_one(
            doc! {
                "workspaceId": &workspace_id,
                "schema_id": &schema_id,
                "version": target.version,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("schema 激活后未找到".to_string()))?;
    Ok(Json(json!({
        "ok": true,
        "item": DomainSchemaView::from(&refreshed),
    })))
}

async fn next_version_for(
    state: &AppState,
    workspace_id: &str,
    schema_id: &str,
) -> AppResult<i32> {
    let latest = state
        .db
        .domain_schemas()
        .find_one(
            doc! { "workspaceId": workspace_id, "schema_id": schema_id },
            FindOneOptions::builder()
                .sort(doc! { "version": -1 })
                .build(),
        )
        .await?;
    Ok(match latest {
        Some(s) => s.version + 1,
        None => 1,
    })
}

/// 校验 schema payload，返回规范化后的 (fields, alias_dict)。
///
/// 规则：
/// - `fields.len() <= 64`
/// - 每个 field 的 `name` 不在 [`BASE_FIELD_BLACKLIST`] 中
/// - 每个 field 的 `name` 全 schema 内唯一
/// - 每个 field 的 `kind` 属于 [`ALLOWED_KINDS`]
/// - `kind=="enum"` 时必须提供非空 `allowed_values`
/// - `alias_dict` 必须是 JSON object；每个 value 必须是 string 且存在于 fields[].name
fn validate_schema_payload(
    incoming_fields: &[DomainFieldPayload],
    alias_dict_value: &Value,
) -> AppResult<(Vec<DomainField>, Document)> {
    if incoming_fields.len() > 64 {
        return Err(AppError::BadRequest(
            "fields 数量不得超过 64".to_string(),
        ));
    }
    let mut seen_names = std::collections::HashSet::new();
    let mut fields = Vec::with_capacity(incoming_fields.len());
    for f in incoming_fields {
        let name = f.name.trim();
        if name.is_empty() {
            return Err(AppError::BadRequest(
                "field.name 不能为空".to_string(),
            ));
        }
        if BASE_FIELD_BLACKLIST.contains(&name) {
            return Err(AppError::BadRequest(format!(
                "字段名 {name} 与 chunk 主表既有字段冲突，请改用其它名字"
            )));
        }
        if !seen_names.insert(name.to_string()) {
            return Err(AppError::BadRequest(format!(
                "字段 {name} 在 fields 中重复定义"
            )));
        }
        if !ALLOWED_KINDS.contains(&f.kind.as_str()) {
            return Err(AppError::BadRequest(format!(
                "字段 {name} 的 kind={} 非法（合法值：string/enum/number/date/reference）",
                f.kind
            )));
        }
        if f.kind == "enum" {
            match &f.allowed_values {
                Some(v) if !v.is_empty() => {}
                _ => {
                    return Err(AppError::BadRequest(format!(
                        "字段 {name} 是 enum 类型，必须提供非空 allowedValues"
                    )));
                }
            }
        }
        fields.push(DomainField {
            name: name.to_string(),
            label: f.label.clone(),
            kind: f.kind.clone(),
            required: f.required,
            allowed_values: f.allowed_values.clone(),
            alias_of: f.alias_of.clone(),
        });
    }

    // alias_dict：JSON object → Document，每个 value 必须存在于 fields[].name
    let alias_doc = match alias_dict_value {
        Value::Null => Document::new(),
        Value::Object(map) => {
            let mut d = Document::new();
            for (k, v) in map {
                let target = v.as_str().ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "aliasDict[{k}] 必须是字符串（指向某个 field.name）"
                    ))
                })?;
                if !seen_names.contains(target) {
                    return Err(AppError::BadRequest(format!(
                        "aliasDict[{k}] = {target}，但 fields 中不存在该字段名"
                    )));
                }
                d.insert(k, target);
            }
            d
        }
        _ => {
            return Err(AppError::BadRequest(
                "aliasDict 必须是 JSON object（{中文别名: canonical字段名}）".to_string(),
            ));
        }
    };

    Ok((fields, alias_doc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn enum_field() -> DomainFieldPayload {
        DomainFieldPayload {
            name: "customer_stage".to_string(),
            label: "客户阶段".to_string(),
            kind: "enum".to_string(),
            required: true,
            allowed_values: Some(vec!["lead".into(), "decision".into()]),
            alias_of: None,
        }
    }

    #[test]
    fn validate_payload_accepts_legal_enum_field() {
        let (fields, alias) =
            validate_schema_payload(&[enum_field()], &json!({"客户阶段": "customer_stage"}))
                .expect("ok");
        assert_eq!(fields.len(), 1);
        assert_eq!(alias.get_str("客户阶段").unwrap(), "customer_stage");
    }

    #[test]
    fn validate_payload_rejects_blacklisted_name() {
        let mut f = enum_field();
        f.name = "wiki_type".to_string();
        let err = validate_schema_payload(&[f], &json!({})).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_payload_rejects_enum_without_allowed_values() {
        let mut f = enum_field();
        f.allowed_values = None;
        let err = validate_schema_payload(&[f], &json!({})).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_payload_rejects_invalid_kind() {
        let mut f = enum_field();
        f.kind = "json".to_string();
        f.allowed_values = None;
        let err = validate_schema_payload(&[f], &json!({})).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_payload_rejects_duplicate_field_names() {
        let f1 = enum_field();
        let mut f2 = enum_field();
        f2.label = "客户阶段-2".to_string();
        let err = validate_schema_payload(&[f1, f2], &json!({})).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_payload_rejects_alias_pointing_to_unknown_field() {
        let err = validate_schema_payload(
            &[enum_field()],
            &json!({"客户阶段": "non_existent_field"}),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_payload_rejects_too_many_fields() {
        let too_many: Vec<_> = (0..65)
            .map(|i| DomainFieldPayload {
                name: format!("f{i}"),
                label: format!("l{i}"),
                kind: "string".to_string(),
                required: false,
                allowed_values: None,
                alias_of: None,
            })
            .collect();
        let err = validate_schema_payload(&too_many, &json!({})).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_payload_rejects_non_string_alias_value() {
        let err =
            validate_schema_payload(&[enum_field()], &json!({"客户阶段": 123})).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}

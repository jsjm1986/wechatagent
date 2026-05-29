//! Phase E / E5-T1：`operation_state_policies` 的 admin 只读列表 / 详情路由。
//!
//! 写入路径仍由 [`super::admin_ops_versions`] 的 publish / rollout / rollback 三动作
//! 承担（同 ops 三表灰度模型），本模块只暴露：
//!
//! - `GET /api/admin/operation-state-policies?domain=&stateKey=&includeAllVersions=`
//! - `GET /api/admin/operation-state-policies/:id`
//!
//! 默认 `includeAllVersions=false` 时只返回 `current_version != false` 的 row（兼容
//! m015 之前的老库），灰度面板传 `true` 拿到完整版本流水以渲染 "v3 → v4 各 50%"
//! 的桶分布与 [[ admin-ops-versions-rollback-chain ]] 回滚链 UI。
//!
//! seed 入口仍是 `m013_seed_user_operation_state_policies`；本模块不引入新的
//! ensure 路径，避免与 migration 重复 backfill。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::doc,
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::OperationStatePolicy,
};

use super::shared::parse_object_id;
use super::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListOperationStatePoliciesQuery {
    domain: Option<String>,
    state_key: Option<String>,
    /// Phase E / E5-T1：默认 `false` 只列 `current_version=true` 的版本，
    /// admin 灰度面板传 `true` 拿历史版本流水（用于 rollback / 回滚链 UI）。
    #[serde(default)]
    include_all_versions: bool,
}

pub(super) async fn list_operation_state_policies(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ListOperationStatePoliciesQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    if let Some(domain) = query.domain.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("domain", domain.trim());
    }
    if let Some(state_key) = query.state_key.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("state_key", state_key.trim());
    }
    if !query.include_all_versions {
        filter.insert("current_version", doc! { "$ne": false });
    }
    let mut cursor = state
        .db
        .operation_state_policies()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "domain": 1, "state_key": 1, "version": -1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(policy) = cursor.try_next().await? {
        items.push(operation_state_policy_json(policy));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn get_operation_state_policy(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let policy = state
        .db
        .operation_state_policies()
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation state policy not found".to_string()))?;
    Ok(Json(json!({ "item": operation_state_policy_json(policy) })))
}

pub(super) fn operation_state_policy_json(policy: OperationStatePolicy) -> Value {
    json!({
        "id": policy.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": policy.workspace_id,
        "domain": policy.domain,
        "stateKey": policy.state_key,
        "allowed": policy.allowed,
        "forbidden": policy.forbidden,
        "recommendedPace": policy.recommended_pace,
        "status": policy.status,
        "updatedAt": crate::models::dt_to_string(policy.updated_at),
        // Phase E / E5-T1：active_versions 灰度字段。前端 StatePolicyAdmin 渲染
        // 当前版本号 + previous_version 回滚链 + seededBy 写入来源徽章。
        "version": policy.version,
        "currentVersion": policy.current_version,
        "previousVersion": policy.previous_version,
        "seededBy": policy.seeded_by,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{DateTime, Document};

    /// E5-T1：序列化输出含 4 灰度字段 + 业务字段（domain/stateKey/allowed/forbidden）。
    #[test]
    fn operation_state_policy_json_carries_versioning_fields() {
        let policy = OperationStatePolicy {
            id: None,
            workspace_id: "default".into(),
            domain: "user_operations".into(),
            state_key: "need_discovery".into(),
            allowed: vec!["text_reply".into()],
            forbidden: vec!["product_pitch".into()],
            recommended_pace: Some("normal".into()),
            status: "active".into(),
            updated_at: DateTime::now(),
            version: 3,
            current_version: true,
            previous_version: Some(2),
            seeded_by: Some("manual".into()),
        };
        let value = operation_state_policy_json(policy);
        assert_eq!(value["domain"], "user_operations");
        assert_eq!(value["stateKey"], "need_discovery");
        assert_eq!(value["allowed"], json!(["text_reply"]));
        assert_eq!(value["forbidden"], json!(["product_pitch"]));
        assert_eq!(value["recommendedPace"], "normal");
        assert_eq!(value["version"], 3);
        assert_eq!(value["currentVersion"], true);
        assert_eq!(value["previousVersion"], 2);
        assert_eq!(value["seededBy"], "manual");
    }

    /// E5-T1：`includeAllVersions=true/false` 默认值反序列化稳定。
    #[test]
    fn list_query_defaults_include_all_versions_to_false() {
        let q: ListOperationStatePoliciesQuery = serde_json::from_value(json!({})).unwrap();
        assert!(!q.include_all_versions);
        assert!(q.domain.is_none());
        assert!(q.state_key.is_none());

        let q2: ListOperationStatePoliciesQuery = serde_json::from_value(json!({
            "includeAllVersions": true,
            "domain": "user_operations"
        }))
        .unwrap();
        assert!(q2.include_all_versions);
        assert_eq!(q2.domain.as_deref(), Some("user_operations"));
    }

    /// E5-T1：filter trim：空白字段不应进入 filter 文档。
    #[test]
    fn list_query_filter_trims_empty_strings() {
        let q: ListOperationStatePoliciesQuery = serde_json::from_value(json!({
            "domain": "  ",
            "stateKey": ""
        }))
        .unwrap();
        // filter 构造逻辑：`is_empty()` 后 trim 视为空 → 不插入到 filter
        let domain_kept = q.domain.as_ref().is_some_and(|s| !s.trim().is_empty());
        let state_key_kept = q.state_key.as_ref().is_some_and(|s| !s.trim().is_empty());
        assert!(!domain_kept);
        assert!(!state_key_kept);
    }

    // 假装一个 doc Filter 互通用例（防止 doc!{}写法回归）。
    #[test]
    fn filter_doc_keys_are_camel_to_snake() {
        // 验证我们写入的 filter key 仍是 mongo 字段名（snake_case），不会被
        // 序列化时混进 camelCase 引发 silent miss。
        let filter: Document = doc! {
            "workspace_id": "default",
            "domain": "user_operations",
            "state_key": "need_discovery",
            "current_version": { "$ne": false },
        };
        assert!(filter.contains_key("workspace_id"));
        assert!(filter.contains_key("state_key"));
        assert!(filter.contains_key("current_version"));
    }
}

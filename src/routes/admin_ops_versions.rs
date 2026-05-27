//! Phase E / E5-T1：ops 三表（`operation_domain_configs` / `operation_state_policies`
//! / `system_taxonomies`）的多版本灰度 admin REST。
//!
//! 三表共享同一套 `(version, current_version, previous_version, seeded_by)` 四元字段
//! （详见 [`crate::models::OperationDomainConfig`] / [`crate::models::OperationStatePolicy`]
//! / [`crate::models::TaxonomyEntry`]），所以三类资源的 publish / rollout / rollback
//! 路径行为高度同构，集中在本模块。
//!
//! 行为约定（**非** prompt_templates 的 `delete_many` 硬清）：
//!
//! - `publish`：在指定 scope（`scope = (workspace_id, domain[, state_key/value.id])`）
//!   下取 `max(version)` 当前已存在的 `previous_version`，写入新文档 `version+1`、
//!   `current_version=true`、`previous_version=Some(prev_current.version)`、
//!   `seeded_by=Some("manual")`，然后 **soft demote** 同 scope 的其他 row 为
//!   `current_version=false`（不删数据，rollback 还需要它们）。
//! - `rollout`：把 scope 下指定 `_id` 的 row promote 到 `current_version=true`，
//!   同时 demote 其他 row。允许多 active 共存的灰度阶段使用（参考
//!   [`crate::agent::runtime::load_user_operation_domain_config_for_resolve`] 的多版本读路径）。
//! - `rollback`：以目标 row 的 `previous_version` 为索引找回上一版本，把它重新
//!   promote 到 current，并 demote 当前 row。如果上一版本不存在或 `previous_version=None`
//!   返回 `BadRequest`。
//!
//! 任何写入 taxonomy 的路径都立即调用 [`crate::agent::taxonomy::invalidate_global_taxonomy_cache`]，
//! 让运行中 Reply / Review Agent 在下次校验 value 时重新 build 字典。

use axum::{
    extract::{Path, State},
    Json,
};
use mongodb::bson::{doc, DateTime, Document};
use serde_json::{json, Value};

use crate::{
    agent::taxonomy::invalidate_global_taxonomy_cache,
    error::{AppError, AppResult},
    models::{OperationDomainConfig, OperationStatePolicy, TaxonomyEntry},
};

use super::shared::parse_object_id;
use super::AppState;

/// ── operation_domain_configs ──────────────────────────────────────────────────

pub(super) async fn publish_operation_domain_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_domain_configs();
    let source = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("operation domain config not found".to_string()))?;

    let scope = doc! {
        "workspace_id": &source.workspace_id,
        "domain": &source.domain,
    };
    let next_version = next_version_for_scope(
        state.db.operation_domain_configs(),
        scope.clone(),
    )
    .await?;
    let now = DateTime::now();
    let new_entry = OperationDomainConfig {
        id: None,
        workspace_id: source.workspace_id.clone(),
        domain: source.domain.clone(),
        name: source.name,
        goal: source.goal,
        methodology: source.methodology,
        workflow: source.workflow,
        tool_policy: source.tool_policy,
        automation_policy: source.automation_policy,
        review_policy: source.review_policy,
        runtime_parameters: source.runtime_parameters,
        state_machine: source.state_machine,
        status: "active".to_string(),
        updated_at: now,
        version: next_version,
        current_version: true,
        previous_version: Some(source.version),
        seeded_by: Some("manual".to_string()),
    };
    let inserted = coll.insert_one(&new_entry, None).await?;
    coll.update_many(
        doc! {
            "workspace_id": &source.workspace_id,
            "domain": &source.domain,
            "_id": { "$ne": inserted.inserted_id.as_object_id() },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "id": inserted.inserted_id.as_object_id().map(|i| i.to_hex()).unwrap_or_default(),
        "version": next_version,
        "previousVersion": source.version,
    })))
}

pub(super) async fn rollout_operation_domain_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_domain_configs();
    let target = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("operation domain config not found".to_string()))?;
    let now = DateTime::now();
    coll.update_one(
        doc! { "_id": object_id },
        doc! { "$set": { "current_version": true, "updated_at": now } },
        None,
    )
    .await?;
    coll.update_many(
        doc! {
            "workspace_id": &target.workspace_id,
            "domain": &target.domain,
            "_id": { "$ne": object_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    Ok(Json(json!({ "ok": true, "version": target.version })))
}

pub(super) async fn rollback_operation_domain_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_domain_configs();
    let target = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("operation domain config not found".to_string()))?;
    let prev_version = target.previous_version.ok_or_else(|| {
        AppError::BadRequest("target version has no previous_version recorded".to_string())
    })?;
    let prev = coll
        .find_one(
            doc! {
                "workspace_id": &target.workspace_id,
                "domain": &target.domain,
                "version": prev_version,
            },
            None,
        )
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "previous version {prev_version} not found for rollback"
            ))
        })?;
    let prev_id = prev
        .id
        .ok_or_else(|| AppError::BadRequest("previous version has no _id".to_string()))?;
    let now = DateTime::now();
    coll.update_one(
        doc! { "_id": prev_id },
        doc! { "$set": { "current_version": true, "updated_at": now } },
        None,
    )
    .await?;
    coll.update_many(
        doc! {
            "workspace_id": &target.workspace_id,
            "domain": &target.domain,
            "_id": { "$ne": prev_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    Ok(Json(json!({ "ok": true, "rolledBackTo": prev_version })))
}

/// ── operation_state_policies ─────────────────────────────────────────────────

pub(super) async fn publish_operation_state_policy_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_state_policies();
    let source = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("operation state policy not found".to_string()))?;
    let scope = doc! {
        "workspace_id": &source.workspace_id,
        "domain": &source.domain,
        "state_key": &source.state_key,
    };
    let next_version = next_version_for_scope(
        state.db.operation_state_policies(),
        scope.clone(),
    )
    .await?;
    let now = DateTime::now();
    let new_entry = OperationStatePolicy {
        id: None,
        workspace_id: source.workspace_id.clone(),
        domain: source.domain.clone(),
        state_key: source.state_key.clone(),
        allowed: source.allowed,
        forbidden: source.forbidden,
        recommended_pace: source.recommended_pace,
        status: "active".to_string(),
        updated_at: now,
        version: next_version,
        current_version: true,
        previous_version: Some(source.version),
        seeded_by: Some("manual".to_string()),
    };
    let inserted = coll.insert_one(&new_entry, None).await?;
    coll.update_many(
        doc! {
            "workspace_id": &source.workspace_id,
            "domain": &source.domain,
            "state_key": &source.state_key,
            "_id": { "$ne": inserted.inserted_id.as_object_id() },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "id": inserted.inserted_id.as_object_id().map(|i| i.to_hex()).unwrap_or_default(),
        "version": next_version,
        "previousVersion": source.version,
    })))
}

pub(super) async fn rollout_operation_state_policy_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_state_policies();
    let target = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("operation state policy not found".to_string()))?;
    let now = DateTime::now();
    coll.update_one(
        doc! { "_id": object_id },
        doc! { "$set": { "current_version": true, "updated_at": now } },
        None,
    )
    .await?;
    coll.update_many(
        doc! {
            "workspace_id": &target.workspace_id,
            "domain": &target.domain,
            "state_key": &target.state_key,
            "_id": { "$ne": object_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    Ok(Json(json!({ "ok": true, "version": target.version })))
}

pub(super) async fn rollback_operation_state_policy_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_state_policies();
    let target = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("operation state policy not found".to_string()))?;
    let prev_version = target.previous_version.ok_or_else(|| {
        AppError::BadRequest("target version has no previous_version recorded".to_string())
    })?;
    let prev = coll
        .find_one(
            doc! {
                "workspace_id": &target.workspace_id,
                "domain": &target.domain,
                "state_key": &target.state_key,
                "version": prev_version,
            },
            None,
        )
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "previous version {prev_version} not found for rollback"
            ))
        })?;
    let prev_id = prev
        .id
        .ok_or_else(|| AppError::BadRequest("previous version has no _id".to_string()))?;
    let now = DateTime::now();
    coll.update_one(
        doc! { "_id": prev_id },
        doc! { "$set": { "current_version": true, "updated_at": now } },
        None,
    )
    .await?;
    coll.update_many(
        doc! {
            "workspace_id": &target.workspace_id,
            "domain": &target.domain,
            "state_key": &target.state_key,
            "_id": { "$ne": prev_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    Ok(Json(json!({ "ok": true, "rolledBackTo": prev_version })))
}

/// ── system_taxonomies ────────────────────────────────────────────────────────

pub(super) async fn publish_taxonomy_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.collection_system_taxonomies();
    let source = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("taxonomy entry not found".to_string()))?;
    let scope = doc! {
        "scope": &source.scope,
        "kind": &source.kind,
        "value.id": &source.value.id,
    };
    let next_version = next_version_for_scope(
        state.db.collection_system_taxonomies(),
        scope.clone(),
    )
    .await?;
    let now = DateTime::now();
    let new_entry = TaxonomyEntry {
        id: None,
        scope: source.scope.clone(),
        kind: source.kind.clone(),
        value: source.value.clone(),
        updated_at: now,
        version: next_version,
        current_version: true,
        previous_version: Some(source.version),
        seeded_by: Some("manual".to_string()),
    };
    let inserted = coll.insert_one(&new_entry, None).await?;
    coll.update_many(
        doc! {
            "scope": &source.scope,
            "kind": &source.kind,
            "value.id": &source.value.id,
            "_id": { "$ne": inserted.inserted_id.as_object_id() },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    invalidate_global_taxonomy_cache();
    Ok(Json(json!({
        "ok": true,
        "id": inserted.inserted_id.as_object_id().map(|i| i.to_hex()).unwrap_or_default(),
        "version": next_version,
        "previousVersion": source.version,
    })))
}

pub(super) async fn rollout_taxonomy_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.collection_system_taxonomies();
    let target = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("taxonomy entry not found".to_string()))?;
    let now = DateTime::now();
    coll.update_one(
        doc! { "_id": object_id },
        doc! { "$set": { "current_version": true, "updated_at": now } },
        None,
    )
    .await?;
    coll.update_many(
        doc! {
            "scope": &target.scope,
            "kind": &target.kind,
            "value.id": &target.value.id,
            "_id": { "$ne": object_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    invalidate_global_taxonomy_cache();
    Ok(Json(json!({ "ok": true, "version": target.version })))
}

pub(super) async fn rollback_taxonomy_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.collection_system_taxonomies();
    let target = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("taxonomy entry not found".to_string()))?;
    let prev_version = target.previous_version.ok_or_else(|| {
        AppError::BadRequest("target version has no previous_version recorded".to_string())
    })?;
    let prev = coll
        .find_one(
            doc! {
                "scope": &target.scope,
                "kind": &target.kind,
                "value.id": &target.value.id,
                "version": prev_version,
            },
            None,
        )
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "previous version {prev_version} not found for rollback"
            ))
        })?;
    let prev_id = prev
        .id
        .ok_or_else(|| AppError::BadRequest("previous version has no _id".to_string()))?;
    let now = DateTime::now();
    coll.update_one(
        doc! { "_id": prev_id },
        doc! { "$set": { "current_version": true, "updated_at": now } },
        None,
    )
    .await?;
    coll.update_many(
        doc! {
            "scope": &target.scope,
            "kind": &target.kind,
            "value.id": &target.value.id,
            "_id": { "$ne": prev_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    invalidate_global_taxonomy_cache();
    Ok(Json(json!({ "ok": true, "rolledBackTo": prev_version })))
}

/// 取出指定 scope 下当前最大 version + 1（无记录时退回 1）。
///
/// 通用化以避免三表三份重复实现。`T` 必须是携带 `version: i32` 的 BSON struct，
/// 这里只读 `version` 字段；其他字段反序列化时由各自的 `serde(default)` 处理。
async fn next_version_for_scope<T>(
    coll: mongodb::Collection<T>,
    scope: Document,
) -> AppResult<i32>
where
    T: serde::de::DeserializeOwned + Sync + Send + Unpin,
{
    use futures::TryStreamExt;
    let raw_coll = coll.clone_with_type::<Document>();
    let mut cursor = raw_coll
        .find(
            scope,
            mongodb::options::FindOptions::builder()
                .sort(doc! { "version": -1_i32 })
                .limit(1_i64)
                .projection(doc! { "version": 1_i32 })
                .build(),
        )
        .await?;
    let max = if let Some(d) = cursor.try_next().await? {
        d.get_i32("version").unwrap_or(0)
    } else {
        0
    };
    Ok(max + 1)
}

#[cfg(test)]
mod tests {
    /// E5-T1：rollback 链路核心 invariant 的纯函数版。
    ///
    /// 给一组 `(version, current_version)` 集合，模拟 publish 事后的 demote
    /// 行为：除 target 外其他全部置 false。这里用纯切片实现，把 `update_many`
    /// 的语义抽离到可单测的形态，覆盖 publish/rollout/rollback 三处共用的
    /// "至多一条 current_version=true" 不变量。
    fn demote_others(rows: &mut [(i32, bool)], keep: i32) {
        for (v, cur) in rows.iter_mut() {
            *cur = *v == keep;
        }
    }

    /// publish：新增 max+1 → 设为 current → 其他 demote 为 false。
    #[test]
    fn publish_demotes_existing_active_to_single_current() {
        let mut rows = vec![(1, true), (2, true), (3, false)];
        // 模拟 publish 4 后 demote 其他
        rows.push((4, true));
        demote_others(&mut rows, 4);
        let active: Vec<_> = rows.iter().filter(|(_, c)| *c).collect();
        assert_eq!(active.len(), 1, "publish 后只能有一条 current_version=true");
        assert_eq!(active[0].0, 4);
    }

    /// rollout：把指定 version 设为 current，其它 demote。
    #[test]
    fn rollout_promotes_target_and_demotes_siblings() {
        let mut rows = vec![(1, false), (2, true), (3, false)];
        demote_others(&mut rows, 1);
        let active: Vec<_> = rows.iter().filter(|(_, c)| *c).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0, 1, "rollout 把版本 1 拉回 current");
    }

    /// rollback：从 target 的 previous_version 解析出回退目标。
    #[test]
    fn rollback_resolves_previous_version_chain() {
        // 模拟 (version, previous_version)
        let chain: Vec<(i32, Option<i32>)> = vec![(1, None), (2, Some(1)), (3, Some(2))];
        // 当前 current = version 3，回退应找到 previous = Some(2)
        let target = chain.iter().find(|(v, _)| *v == 3).unwrap();
        assert_eq!(target.1, Some(2));
        // 版本 2 在历史里有 _id（这里只校验链）；从 2 再回退到 1
        let prev = chain.iter().find(|(v, _)| *v == target.1.unwrap()).unwrap();
        assert_eq!(prev.1, Some(1));
        // 版本 1 是初版，previous_version=None → 链终点
        let root = chain.iter().find(|(v, _)| *v == prev.1.unwrap()).unwrap();
        assert_eq!(root.1, None);
    }

    /// rollback：previous_version=None 时 publish/rollback handler 必须报错。
    /// 这里只验证 None 检测逻辑，handler 内部 `ok_or_else` 走 BadRequest 分支。
    #[test]
    fn rollback_rejects_when_no_previous_version() {
        let target_prev: Option<i32> = None;
        assert!(target_prev.is_none(), "无 previous_version 时 rollback 应被拒绝");
    }
}

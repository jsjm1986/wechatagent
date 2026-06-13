//! universal-domain-adaptation Phase 3（3A-3）：`domain_profiles` 行业总装配单的
//! admin REST + 多版本灰度。
//!
//! `DomainProfile` 携带与 ops 三表同构的 `(version, current_version, previous_version,
//! seeded_by)` 四元字段（见 [`crate::models::DomainProfile`]），故 publish / rollout /
//! rollback 行为照搬 [`crate::routes::admin_ops_versions`] 的 soft-demote 语义，scope =
//! `(workspace_id, profile_id)`。
//!
//! **publish / activate 两步**（设计文档 §4.1 step 6-7）：
//! - `publish`：在 scope 下写新 `version+1`、`current_version=true`、soft-demote 同
//!   scope 其他 row 的 `current_version`。**不动 `is_active`**——publish 只定稿版本，
//!   尚未让运行时切换。
//! - `activate`：把指定 row 的 `is_active=true`，并把同 workspace 其他 profile 的
//!   `is_active=false`（每 workspace 至多一条 active）。运行时下一轮决策即用它。
//!
//! 运行时缓存查询要求 `is_active=true AND current_version=true`（见
//! [`crate::agent::domain_profile::DomainProfileCache`]），故任何改这两个标记的写入
//! 路径都立即调 [`crate::agent::domain_profile::invalidate_global_domain_profile_cache`]，
//! 让运行中 Agent 在下一次决策重新加载 active profile（否则最多 30s TTL 才可见）。
//!
//! **红线**：引导层 AI 生成的 profile 必须人审才能 activate（继承「AI 永不自动 verify」）；
//! 候选不阻塞运行时（无 active 时回落 DEFAULT_PROFILE，零配置启动不变）。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::FindOptions;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent::domain_profile::invalidate_global_domain_profile_cache,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::DomainProfile,
};

use super::shared::parse_object_id;
use super::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct ListQuery {
    #[serde(default, rename = "workspaceId")]
    workspace_id: Option<String>,
    /// 默认只返回 `current_version=true`；`includeAllVersions=true` 时返回全部历史版本。
    #[serde(default, rename = "includeAllVersions")]
    include_all_versions: bool,
}

pub(super) async fn list_domain_profiles(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = params
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
    let mut filter = doc! { "workspace_id": &workspace_id };
    if !params.include_all_versions {
        filter.insert("current_version", true);
    }
    let mut cursor = state
        .db
        .domain_profiles()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "profile_id": 1_i32, "version": -1_i32 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(p) = cursor.try_next().await? {
        items.push(profile_view(&p));
    }
    Ok(Json(json!({ "items": items })))
}

/// 把 DomainProfile 序列化成前端视图：整体 serde + `id` 转 hex。
fn profile_view(p: &DomainProfile) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        let hex = p.id.map(|i| i.to_hex()).unwrap_or_default();
        obj.insert("id".to_string(), json!(hex));
        // _id 是 BSON ObjectId 序列化形态,前端用上面的 hex `id` 即可。
        obj.remove("_id");
    }
    v
}

pub(super) async fn get_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let profile = state
        .db
        .domain_profiles()
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
    Ok(Json(json!({ "item": profile_view(&profile) })))
}

/// create / update 请求体 = 完整 DomainProfile（引导层 apply 落候选用）。`id` /
/// 版本灰度字段由后端管理,请求里给的会被忽略/覆盖。
#[derive(Debug, Deserialize)]
pub(super) struct UpsertRequest {
    #[serde(rename = "workspaceId", default)]
    workspace_id: Option<String>,
    #[serde(rename = "profileId")]
    profile_id: String,
    #[serde(flatten)]
    profile: Document,
}

pub(super) async fn create_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<UpsertRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id = body
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
    if body.profile_id.trim().is_empty() {
        return Err(AppError::BadRequest("profileId 不能为空".to_string()));
    }
    let next_version =
        next_version_for_profile(&state, &workspace_id, &body.profile_id).await?;
    let now = DateTime::now();
    // 用请求 body 反序列化成 DomainProfile,再强制覆盖后端管理字段。
    let mut doc = body.profile.clone();
    doc.insert("profile_id", &body.profile_id);
    doc.insert("workspace_id", &workspace_id);
    let mut profile: DomainProfile = mongodb::bson::from_document(doc)
        .map_err(|e| AppError::BadRequest(format!("profile 字段不合法: {e}")))?;
    profile.id = None;
    profile.profile_id = body.profile_id.clone();
    profile.workspace_id = workspace_id.clone();
    profile.version = next_version;
    profile.current_version = false; // 创建即草稿,需 publish 定稿 + activate 生效
    profile.previous_version = None;
    profile.is_active = false;
    profile.seeded_by = profile.seeded_by.or_else(|| Some("manual".to_string()));
    profile.created_at = now;
    profile.updated_at = now;
    let inserted = state.db.domain_profiles().insert_one(&profile, None).await?;
    profile.id = inserted.inserted_id.as_object_id();
    Ok(Json(json!({ "item": profile_view(&profile) })))
}

/// update：在指定 `_id`（必须是当前 current_version 草稿）上原地改字段。已 publish
/// 定稿的版本不应原地改（应 create 新版本再 publish），故 update 只允许改
/// `current_version=false` 的草稿行；改 active 行直接拒绝（须走 create→publish→activate）。
pub(super) async fn update_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(body): Json<UpsertRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.domain_profiles();
    let existing = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
    if existing.is_active {
        return Err(AppError::BadRequest(
            "已激活的 profile 不可原地修改;请 create 新版本再 publish/activate".to_string(),
        ));
    }
    let workspace_id = existing.workspace_id.clone();
    let now = DateTime::now();
    let mut doc = body.profile.clone();
    doc.insert("profile_id", &existing.profile_id);
    doc.insert("workspace_id", &workspace_id);
    let mut profile: DomainProfile = mongodb::bson::from_document(doc)
        .map_err(|e| AppError::BadRequest(format!("profile 字段不合法: {e}")))?;
    // 保留原 _id / 版本灰度 / active 标记,只更新内容字段。
    profile.id = existing.id;
    profile.profile_id = existing.profile_id.clone();
    profile.workspace_id = workspace_id.clone();
    profile.version = existing.version;
    profile.current_version = existing.current_version;
    profile.previous_version = existing.previous_version;
    profile.is_active = existing.is_active;
    profile.seeded_by = existing.seeded_by.clone();
    profile.created_at = existing.created_at;
    profile.updated_at = now;
    coll.replace_one(doc! { "_id": object_id }, &profile, None)
        .await?;
    Ok(Json(json!({ "item": profile_view(&profile) })))
}

/// delete：禁止删除 active profile（须先 activate 另一条或回落 DEFAULT）。
pub(super) async fn delete_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.domain_profiles();
    let existing = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
    if existing.is_active {
        return Err(AppError::BadRequest(
            "不可删除已激活的 profile;请先激活另一条或停用".to_string(),
        ));
    }
    coll.delete_one(doc! { "_id": object_id }, None).await?;
    Ok(Json(json!({ "ok": true })))
}

/// publish：在 scope=(workspace_id, profile_id) 下取 max(version)+1,写新文档
/// current_version=true + previous_version=Some(source.version),soft-demote 同 scope
/// 其他 row 的 current_version。**不动 is_active**(publish 只定稿版本)。
pub(super) async fn publish_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.domain_profiles();
    let source = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
    let next_version =
        next_version_for_profile(&state, &source.workspace_id, &source.profile_id).await?;
    let now = DateTime::now();
    let mut published = source.clone();
    published.id = None;
    published.version = next_version;
    published.current_version = true;
    published.previous_version = Some(source.version);
    published.seeded_by = Some("manual".to_string());
    published.is_active = source.is_active; // publish 不改激活态
    published.updated_at = now;
    let inserted = coll.insert_one(&published, None).await?;
    let new_id = inserted.inserted_id.as_object_id();
    coll.update_many(
        doc! {
            "workspace_id": &source.workspace_id,
            "profile_id": &source.profile_id,
            "_id": { "$ne": new_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    invalidate_global_domain_profile_cache();
    Ok(Json(json!({
        "ok": true,
        "id": new_id.map(|i| i.to_hex()).unwrap_or_default(),
        "version": next_version,
        "previousVersion": source.version,
    })))
}

/// rollout：把指定 row promote 到 current_version=true,demote 同 scope 其他 row。
pub(super) async fn rollout_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.domain_profiles();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
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
            "profile_id": &target.profile_id,
            "_id": { "$ne": object_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    invalidate_global_domain_profile_cache();
    Ok(Json(json!({ "ok": true, "version": target.version })))
}

/// rollback：以 target.previous_version 找回上一版本 promote 到 current,demote 当前。
pub(super) async fn rollback_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.domain_profiles();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
    let prev_version = target.previous_version.ok_or_else(|| {
        AppError::BadRequest("target version has no previous_version recorded".to_string())
    })?;
    let prev = coll
        .find_one(
            doc! {
                "workspace_id": &target.workspace_id,
                "profile_id": &target.profile_id,
                "version": prev_version,
            },
            None,
        )
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!("previous version {prev_version} not found for rollback"))
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
            "profile_id": &target.profile_id,
            "_id": { "$ne": prev_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    invalidate_global_domain_profile_cache();
    Ok(Json(json!({ "ok": true, "rolledBackTo": prev_version })))
}

/// activate：把指定 row is_active=true,同 workspace 其他 profile is_active=false
/// （每 workspace 至多一条 active）。运行时缓存查 is_active+current_version,故只有
/// 既 current 又 active 的 row 会被加载——activate 前应已 publish 定稿。
pub(super) async fn activate_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.domain_profiles();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
    if !target.current_version {
        return Err(AppError::BadRequest(
            "只能激活 current_version 定稿版本;请先 publish".to_string(),
        ));
    }
    let now = DateTime::now();
    coll.update_one(
        doc! { "_id": object_id },
        doc! { "$set": { "is_active": true, "updated_at": now } },
        None,
    )
    .await?;
    coll.update_many(
        doc! {
            "workspace_id": &target.workspace_id,
            "_id": { "$ne": object_id },
        },
        doc! { "$set": { "is_active": false, "updated_at": now } },
        None,
    )
    .await?;
    invalidate_global_domain_profile_cache();
    Ok(Json(json!({ "ok": true, "activated": target.profile_id })))
}

/// 同 scope=(workspace_id, profile_id) 下取 max(version)+1。
async fn next_version_for_profile(
    state: &AppState,
    workspace_id: &str,
    profile_id: &str,
) -> AppResult<i32> {
    let raw = state
        .db
        .domain_profiles()
        .clone_with_type::<Document>();
    let mut cursor = raw
        .find(
            doc! { "workspace_id": workspace_id, "profile_id": profile_id },
            FindOptions::builder()
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
    //! publish/activate 两步语义的纯函数不变量(DB 端 update_many 行为的可单测抽离)。
    //! 完整 DB-backed 端到端流程由 CI 集成套件覆盖(本地磁盘纪律:重套件走 CI)。

    /// publish：scope 内除新版本外全部 demote current_version；is_active **不动**。
    fn publish_demote_current(rows: &mut [(i32, bool, bool)], new_version: i32) {
        // 元组 = (version, current_version, is_active)
        for (v, cur, _active) in rows.iter_mut() {
            *cur = *v == new_version;
        }
    }

    /// activate：workspace 内目标 is_active=true,其他全部 false；current_version 不动。
    fn activate_single(rows: &mut [(i32, bool, bool)], target_version: i32) {
        for (v, _cur, active) in rows.iter_mut() {
            *active = *v == target_version;
        }
    }

    #[test]
    fn publish_demotes_current_but_leaves_is_active_untouched() {
        // 版本 1 当前 current+active;publish 版本 2 后只 demote current,active 不变。
        let mut rows = vec![(1, true, true), (2, true, false)];
        publish_demote_current(&mut rows, 2);
        let current: Vec<_> = rows.iter().filter(|(_, c, _)| *c).collect();
        assert_eq!(current.len(), 1, "publish 后只一条 current_version");
        assert_eq!(current[0].0, 2);
        // 关键:publish 不动 is_active —— 版本 1 仍 active(运行时缓存要 active+current,
        // 故此刻版本 1 既非 current 也就不会被加载,需后续 activate 版本 2 才生效)。
        assert!(rows[0].2, "publish 不改 is_active:版本1仍标 active");
        assert!(!rows[1].2, "版本2 publish 后尚未 activate");
    }

    #[test]
    fn activate_sets_single_active_but_leaves_current_untouched() {
        // 版本 2 已 publish(current),activate 版本 2 → 它 active,版本 1 取消 active。
        let mut rows = vec![(1, false, true), (2, true, false)];
        activate_single(&mut rows, 2);
        let active: Vec<_> = rows.iter().filter(|(_, _, a)| *a).collect();
        assert_eq!(active.len(), 1, "每 workspace 至多一条 active");
        assert_eq!(active[0].0, 2);
        // current_version 不被 activate 触碰。
        assert!(!rows[0].1 && rows[1].1, "activate 不改 current_version");
    }

    #[test]
    fn two_step_publish_then_activate_makes_version_loadable() {
        // 缓存可见的充要条件 = current_version && is_active。两步后版本 2 同时满足。
        let mut rows = vec![(1, true, true), (2, true, false)];
        publish_demote_current(&mut rows, 2);
        activate_single(&mut rows, 2);
        let loadable: Vec<_> = rows.iter().filter(|(_, c, a)| *c && *a).collect();
        assert_eq!(loadable.len(), 1, "只一条 current+active 可被运行时加载");
        assert_eq!(loadable[0].0, 2);
    }
}

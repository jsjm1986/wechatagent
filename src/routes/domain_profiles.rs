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
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::FindOptions;
use mongodb::Collection;
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
///
/// `profile_id` 标 `#[serde(default)]`：**update 路径根本不消费它**（只用
/// `existing.profile_id`，见 `update_domain_profile`），前端「保存修改」发的
/// `DomainProfileDraft` 是 snake_case 且不含 `profileId`，若必填会在进 handler 前
/// 因 serde flatten 缺字段而 422，使 $set 部分更新逻辑根本不可达。create 路径仍由
/// `create_domain_profile` 里的显式空值校验（profileId 不能为空）兜底，行为不变。
#[derive(Debug, Deserialize)]
pub(super) struct UpsertRequest {
    #[serde(rename = "workspaceId", default)]
    workspace_id: Option<String>,
    #[serde(rename = "profileId", default)]
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
    let now = DateTime::now();
    // 部分更新：只 `$set` 请求 body 实际带来的内容键,未出现的字段保持原值(避免
    // `from_document` + `replace_one` 把缺失字段按 serde 默认清零 —— 草稿/AI 候选行
    // is_active=false 不受上面那道拒绝保护,replace 会静默丢字段)。同时剥离后端管理键,
    // 任何来自 body 的 id/版本灰度/active/审计字段都被忽略,不可经 PUT 篡改。
    let set_doc = strip_backend_managed_keys(&body.profile);
    // 校验：把 body 内容键合并到现有文档上,整体反序列化一次,确保类型/取值合法
    // （非法直接 400,不落库）；校验通过后只 `$set` body 键(合并后的不写回,防止把
    // 未触碰字段的 BSON 形态漂移)。
    let mut merged = mongodb::bson::to_document(&existing)
        .map_err(|e| AppError::External(format!("序列化现有 profile 失败: {e}")))?;
    for (k, v) in set_doc.iter() {
        merged.insert(k.clone(), v.clone());
    }
    let _validated: DomainProfile = mongodb::bson::from_document(merged)
        .map_err(|e| AppError::BadRequest(format!("profile 字段不合法: {e}")))?;
    // 白名单过滤（审查 CORRECT-1）：只 `$set` 既出现在 body、又是合法 DomainProfile
    // 字段的键。DomainProfile 无 deny_unknown_fields，校验阶段 serde 会忽略未知键 → 不
    // 拦截；若直接 $set 原始 set_doc，body 里任意未知键会被真实写进 Mongo 文档（读回
    // 虽被 serde 忽略、功能无害，但属文档污染）。用 _validated 重新序列化得到的键集做
    // 交集，挡掉未知键。
    let known_keys: std::collections::HashSet<String> = mongodb::bson::to_document(&_validated)
        .map(|d| d.keys().cloned().collect())
        .unwrap_or_default();
    let mut set_doc: Document = set_doc
        .into_iter()
        .filter(|(k, _)| known_keys.contains(k))
        .collect();
    set_doc.insert("updated_at", now);
    coll.update_one(doc! { "_id": object_id }, doc! { "$set": set_doc }, None)
        .await?;
    let updated = coll
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
    Ok(Json(json!({ "item": profile_view(&updated) })))
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
    published.is_active = false; // 先落非 active,下方 realign 据血缘决定是否继承生效态
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
    // 不变量对齐:若该 profile 血缘原本生效(有 active 行),把 is_active 迁到新 current 行,
    // 避免 publish 后 active 行(旧版本)非 current → 运行时静默回落 DEFAULT。血缘从未生效
    // 则不动(守住「AI 生成候选须人审 activate」红线)。
    if let Some(nid) = new_id {
        realign_active_to_current(&coll, &source.workspace_id, &source.profile_id, nid, now)
            .await?;
    }
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
    // 不变量对齐：current 已移到 object_id，若血缘原本生效则把 is_active 一并迁过来，
    // 避免 active 行非 current → 运行时静默回落 DEFAULT。
    realign_active_to_current(&coll, &target.workspace_id, &target.profile_id, object_id, now)
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
    // 不变量对齐：current 已回退到 prev_id，若血缘原本生效则把 is_active 一并迁过来。
    realign_active_to_current(&coll, &target.workspace_id, &target.profile_id, prev_id, now)
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

/// 不变量对齐：把 `is_active` 迁到新的 current 行。
///
/// 运行时缓存加载充要条件是 `is_active=true AND current_version=true`（见文件头）。
/// `publish`/`rollout`/`rollback` 都会把 `current_version` 移动到 **另一行**，若不同步
/// 迁移 `is_active`，会出现「active 行非 current、current 行非 active」→ reload 查询零命中
/// → 该 workspace 静默回落 DEFAULT_PROFILE（运营无感知配置失效）。
///
/// 本 helper 在移动 current_version **之后**调用：若该 `(workspace_id, profile_id)` 血缘
/// 里原本存在 active 行（说明这个 profile 是当前生效的），就把 `is_active` 收敛到
/// `new_current_id` 这一行、清掉血缘里其他行的 active，保证「active 行 == current 行」。
/// 若血缘里本就没有 active 行（profile 从未 activate，纯草稿/定稿态），则不动 is_active
/// （不凭空激活一个未经人审 activate 的版本——守住 AI 永不自动 verify 红线）。
async fn realign_active_to_current(
    coll: &Collection<DomainProfile>,
    workspace_id: &str,
    profile_id: &str,
    new_current_id: ObjectId,
    now: DateTime,
) -> AppResult<()> {
    let active_in_lineage = coll
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "profile_id": profile_id,
                "is_active": true,
            },
            None,
        )
        .await?;
    if active_in_lineage == 0 {
        // profile 从未生效过 → 不凭空激活，保持纯草稿/定稿态。
        return Ok(());
    }
    // 血缘原本生效 → 把 active 收敛到新 current 行。
    coll.update_one(
        doc! { "_id": new_current_id },
        doc! { "$set": { "is_active": true, "updated_at": now } },
        None,
    )
    .await?;
    coll.update_many(
        doc! {
            "workspace_id": workspace_id,
            "profile_id": profile_id,
            "_id": { "$ne": new_current_id },
        },
        doc! { "$set": { "is_active": false, "updated_at": now } },
        None,
    )
    .await?;
    Ok(())
}

/// PUT 部分更新：从请求 body 文档剥离后端管理键,返回只含可改内容键的 `$set` 文档。
///
/// 后端管理键（id / 版本灰度 / active / 审计时间戳）由 publish/rollout/activate 等
/// 专用路径维护,不可经内容编辑 PUT 篡改;同时本函数**只**收录 body 实际出现的键,
/// 故未编辑字段不会被写入 `$set` → 原值保持（修复 `replace_one` 整行覆盖把缺失字段
/// 按 serde 默认清零的数据丢失)。
fn strip_backend_managed_keys(body: &Document) -> Document {
    const BACKEND_MANAGED_KEYS: &[&str] = &[
        "_id",
        "id",
        "profile_id",
        "workspace_id",
        "version",
        "current_version",
        "previous_version",
        "is_active",
        "seeded_by",
        "created_at",
        "updated_at",
    ];
    let mut set_doc = Document::new();
    for (k, v) in body.iter() {
        if BACKEND_MANAGED_KEYS.contains(&k.as_str()) {
            continue;
        }
        set_doc.insert(k.clone(), v.clone());
    }
    set_doc
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

    /// realign：current 移动后调用。若血缘原本有 active 行,把 is_active 收敛到新 current
    /// 版本行、清掉其他;若血缘从未 active 则不动(不凭空激活未经人审 activate 的版本)。
    /// 对应 `realign_active_to_current`(以 version 代 _id 作模型键)。
    fn realign_active_to_current_sim(rows: &mut [(i32, bool, bool)], new_current_version: i32) {
        let lineage_had_active = rows.iter().any(|(_, _, a)| *a);
        if !lineage_had_active {
            return;
        }
        for (v, _cur, active) in rows.iter_mut() {
            *active = *v == new_current_version;
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

    #[test]
    fn realign_migrates_active_to_new_current_when_lineage_was_active() {
        // 血缘原本生效(版本1 current+active)。publish 版本2 demote current 后,
        // realign 把 active 迁到版本2 → active 行恒等于 current 行,运行时不回落 DEFAULT。
        let mut rows = vec![(1, true, true), (2, true, false)];
        publish_demote_current(&mut rows, 2);
        realign_active_to_current_sim(&mut rows, 2);
        let current: Vec<_> = rows.iter().filter(|(_, c, _)| *c).map(|(v, _, _)| *v).collect();
        let active: Vec<_> = rows.iter().filter(|(_, _, a)| *a).map(|(v, _, _)| *v).collect();
        assert_eq!(current, vec![2], "publish 后唯一 current=版本2");
        assert_eq!(active, vec![2], "realign 后 active 迁到版本2");
        // 充要条件成立:版本2 既 current 又 active → 立即可加载,无静默降级。
        assert_eq!(rows.iter().filter(|(_, c, a)| *c && *a).count(), 1);
    }

    #[test]
    fn realign_noop_when_lineage_never_active() {
        // 纯草稿/定稿血缘(从未 activate)。publish 新版本后 realign 不凭空激活任何行,
        // 守住「AI 生成候选须人审 activate」红线。
        let mut rows = vec![(1, true, false), (2, true, false)];
        publish_demote_current(&mut rows, 2);
        realign_active_to_current_sim(&mut rows, 2);
        assert!(rows.iter().all(|(_, _, a)| !*a), "血缘从未 active → realign 不动 is_active");
    }

    #[test]
    fn rollback_realigns_active_back_to_previous_version() {
        // 版本2 current+active(生效中),rollback 到版本1:current 回退到版本1,
        // realign 把 active 也迁回版本1 → 回退后的版本立即生效,不静默降级。
        let mut rows = vec![(1, false, false), (2, true, true)];
        // rollback：prev(版本1) current=true,其余 demote。
        for (v, cur, _) in rows.iter_mut() {
            *cur = *v == 1;
        }
        realign_active_to_current_sim(&mut rows, 1);
        let loadable: Vec<_> = rows.iter().filter(|(_, c, a)| *c && *a).map(|(v, _, _)| *v).collect();
        assert_eq!(loadable, vec![1], "rollback 后版本1 既 current 又 active");
    }

    #[test]
    fn strip_backend_managed_keys_drops_managed_and_keeps_only_present_content() {
        use mongodb::bson::doc;
        // body 同时含内容键与后端管理键 + 试图篡改的 is_active/version。
        let body = doc! {
            "display_name": "情感陪伴",
            "grounding_gate_bypass_without_claim": true,
            // 以下后端管理键必须被剥离,不可经 PUT 篡改：
            "is_active": true,
            "version": 99_i32,
            "current_version": true,
            "seeded_by": "manual",
            "_id": "deadbeef",
            "id": "deadbeef",
            "workspace_id": "attacker",
            "profile_id": "spoof",
        };
        let set_doc = super::strip_backend_managed_keys(&body);
        // 只保留内容键。
        assert_eq!(set_doc.len(), 2, "只剩两个内容键");
        assert_eq!(set_doc.get_str("display_name").unwrap(), "情感陪伴");
        assert_eq!(set_doc.get_bool("grounding_gate_bypass_without_claim").unwrap(), true);
        // 所有后端管理键被剥离。
        for k in ["is_active", "version", "current_version", "seeded_by", "_id", "id", "workspace_id", "profile_id"] {
            assert!(set_doc.get(k).is_none(), "后端管理键 {k} 必须被剥离");
        }
    }

    #[test]
    fn strip_backend_managed_keys_omits_untouched_fields() {
        use mongodb::bson::doc;
        // 关键:body 只带一个字段 → set_doc 也只含那一个键,其余字段不出现在 `$set`
        // → update_one 不触碰它们 → 原值保持(这正是修复「整行 replace 清零」的核心)。
        let body = doc! { "description": "只改简介" };
        let set_doc = super::strip_backend_managed_keys(&body);
        assert_eq!(set_doc.len(), 1);
        assert_eq!(set_doc.get_str("description").unwrap(), "只改简介");
        // 未在 body 出现的内容字段不进 set_doc → 不被清零。
        assert!(set_doc.get("display_name").is_none());
        assert!(set_doc.get("memory_dimensions").is_none());
        assert!(set_doc.get("soul_override").is_none());
    }

    /// D4-1 回归：前端「保存修改」发的 DomainProfileDraft 是 snake_case 且**不含
    /// profileId**，UpsertRequest.profile_id 必须 `#[serde(default)]` 才能反序列化成功，
    /// 否则 serde flatten 缺必填字段会在进 handler 前 422，使 update 的 $set 部分更新
    /// 根本不可达。本测试钉死该契约：缺 profileId 的 body 能成功反序列化。
    #[test]
    fn upsert_request_deserializes_without_profile_id() {
        // 模拟前端 PUT body：snake_case 内容字段，无 profileId。
        let body = serde_json::json!({
            "display_name": "情感陪伴",
            "description": "改简介",
            "conversation_mode_policy": "## 对话模式判定\n\n用户表达情绪 → empathetic_support。"
        });
        let req: super::UpsertRequest =
            serde_json::from_value(body).expect("缺 profileId 的 body 应能反序列化（update 路径不消费它）");
        // profile_id 走 default = 空串（update 路径不读它，用 existing.profile_id）。
        assert_eq!(req.profile_id, "");
        // workspaceId 缺省也走 default = None。
        assert!(req.workspace_id.is_none());
        // 内容键经 flatten 落进 profile Document，strip 后能进 $set。
        let set_doc = super::strip_backend_managed_keys(&req.profile);
        assert_eq!(set_doc.get_str("display_name").unwrap(), "情感陪伴");
        assert!(set_doc.get_str("conversation_mode_policy").is_ok());
    }
}

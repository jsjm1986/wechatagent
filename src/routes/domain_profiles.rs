//! universal-domain-adaptation Phase 3（3A-3）：`domain_profiles` 行业总装配单的
//! admin REST + 多版本灰度。
//!
//! `DomainProfile` 携带与 ops 三表同构的 `(version, current_version, previous_version,
//! seeded_by)` 四元字段（见 [`crate::models::DomainProfile`]），故 publish / rollout /
//! rollback 行为照搬 [`crate::routes::admin_ops_versions`] 的 soft-demote 语义，scope =
//! `(workspace_id, profile_id)`。
//!
//! **publish / activate 语义**（设计文档 §4.1 step 6-7，+ #1 不变量对齐修订）：
//! - `publish`：在 scope 下写新 `version+1`、`current_version=true`、soft-demote 同
//!   scope 其他 row 的 `current_version`；随后调 `realign_active_to_current`：
//!   - 若该 `(workspace_id, profile_id)` 血缘**原本有 active 行**（已被人审 activate 过、
//!     正在生效）→ 把 `is_active` 一并迁到新版本行 → 新版本**即时生效**（运营编辑已生效
//!     配置后无需再 activate；与 ops 三表 publish 即生效一致）。
//!   - 若血缘**从未 active**（纯草稿/AI 生成候选，`is_active=false`）→ realign **noop** →
//!     新版本仍 `is_active=false`，**必须经人审 `activate` 才生效**（守住「AI 生成候选须
//!     人审」红线）。
//! - `activate`：把指定 row 的 `is_active=true`，并把同 workspace 其他 profile 的
//!   `is_active=false`（每 workspace 至多一条 active）。运行时下一轮决策即用它。
//!
//! > 注：未来若要对「危险开关变更」加二次确认（即便已生效血缘也强制走 publish→activate
//! > 两步），在 publish 的 realign 调用前按字段 diff 分级即可——本文件头描述的是当前
//! > 「已生效血缘 publish 即时生效」基线行为。
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
pub struct UpsertRequest {
    #[serde(rename = "workspaceId", default)]
    workspace_id: Option<String>,
    #[serde(rename = "profileId", default)]
    profile_id: String,
    #[serde(flatten)]
    profile: Document,
}

pub async fn create_domain_profile(
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
pub async fn update_domain_profile(
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

/// publish：在 scope=(workspace_id, profile_id) 下取 max(version)+1,写新文档。
///
/// **分级生效**（危险开关二次确认）：先取该血缘当前 active 版本作 diff 基准，比对
/// [`risky_fields_changed`] 的 10 个危险字段——
/// - **危险分支**（血缘已生效 **且** 危险字段有变更）：新版本落「旁路稿」
///   `current_version=false`、`is_active=false`，**不 demote、不 realign、不动旧 active
///   版本**——旧版继续 current+active 生效（零窗口期回落 DEFAULT）。返回 `pendingActivation:true`
///   + `riskyFields`，前端二次确认后经 `rollout`（推 current+demote+realign）才真正生效。
/// - **普通分支**（无危险变更，或血缘从未 active：纯草稿/AI 候选/DEFAULT）：与改造前
///   逐字节等价——新版本 `current_version=true`、soft-demote 其他、`realign_active_to_current`
///   据血缘决定是否继承生效态。
pub async fn publish_domain_profile(
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
    // diff 基准 = 该血缘当前正在生效（is_active）的版本。无 active（纯草稿/AI 候选/
    // DEFAULT 血缘）时为 None → 危险分支不触发，走与改造前等价的普通分支。
    let active_base = coll
        .find_one(
            doc! {
                "workspace_id": &source.workspace_id,
                "profile_id": &source.profile_id,
                "is_active": true,
            },
            None,
        )
        .await?;
    let risky_changed: Vec<&'static str> = active_base
        .as_ref()
        .map(|base| risky_fields_changed(base, &source))
        .unwrap_or_default();
    let next_version =
        next_version_for_profile(&state, &source.workspace_id, &source.profile_id).await?;
    let now = DateTime::now();
    let mut published = source.clone();
    published.id = None;
    published.version = next_version;
    published.previous_version = Some(source.version);
    published.seeded_by = Some("manual".to_string());
    published.is_active = false;
    published.updated_at = now;

    if active_base.is_some() && !risky_changed.is_empty() {
        // 危险分支：落旁路稿（非 current），不动正在生效的旧版本。等运营二次确认后
        // 经 rollout 才推 current+生效——给手滑改错风控/人格/自学习方向留缓冲。
        published.current_version = false;
        let inserted = coll.insert_one(&published, None).await?;
        let new_id = inserted.inserted_id.as_object_id();
        invalidate_global_domain_profile_cache();
        return Ok(Json(json!({
            "ok": true,
            "pendingActivation": true,
            "riskyFields": risky_changed,
            "id": new_id.map(|i| i.to_hex()).unwrap_or_default(),
            "version": next_version,
            "previousVersion": source.version,
        })));
    }

    // 普通分支（无危险变更，或血缘从未 active）——与改造前逐字节等价。
    published.current_version = true;
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
pub async fn rollout_domain_profile(
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
pub async fn rollback_domain_profile(
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
pub async fn activate_domain_profile(
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

/// 「危险开关」字段集：直接左右 AI 能否瞎编产品 / 自学习方向 / 人格本体 / 风控阈值
/// 的 10 个字段。运营手动编辑**已生效**血缘并 publish 时，这些字段一旦相对当前 active
/// 版本发生变化，就不即时生效（落旁路稿等二次确认），避免手滑改错立即污染线上。
/// 黑名单外字段（display_name/description/profile_dimensions/coverage_dimensions/
/// business_formulas/memory_dimensions/chunk_roles/prompt_fragment/stagnation_dimension/
/// domain_schema_id/methodology_generator_preamble）视为普通字段，照旧即时生效。
const RISKY_FIELD_NAMES: [&str; 10] = [
    "soul_override",
    "methodology_override",
    "conversation_mode_policy",
    "commitment_markers",
    "conversation_modes",
    "operation_mode",
    "grounding_gate_bypass_without_claim",
    "distrust_self_reported_low_risk",
    "outcome_polarity",
    "threshold_overrides",
];

/// 比对两份 profile 的 10 个危险字段，返回**发生变化**的字段名列表（顺序与
/// [`RISKY_FIELD_NAMES`] 一致）。整体相等比较（逐字段 `!=`，偏保守：宁可多一次确认也
/// 不漏判）。`commitment_markers` / `operation_mode` / `outcome_polarity` /
/// `threshold_overrides` 依赖各自类型的 `PartialEq`（见 `models.rs`）。
///
/// 纯函数、无 IO，供 `publish_domain_profile` 分级判定 + 单测共用。空 Vec = 无危险变更。
pub fn risky_fields_changed(old: &DomainProfile, new: &DomainProfile) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if old.soul_override != new.soul_override {
        changed.push(RISKY_FIELD_NAMES[0]);
    }
    if old.methodology_override != new.methodology_override {
        changed.push(RISKY_FIELD_NAMES[1]);
    }
    if old.conversation_mode_policy != new.conversation_mode_policy {
        changed.push(RISKY_FIELD_NAMES[2]);
    }
    if old.commitment_markers != new.commitment_markers {
        changed.push(RISKY_FIELD_NAMES[3]);
    }
    if old.conversation_modes != new.conversation_modes {
        changed.push(RISKY_FIELD_NAMES[4]);
    }
    if old.operation_mode != new.operation_mode {
        changed.push(RISKY_FIELD_NAMES[5]);
    }
    if old.grounding_gate_bypass_without_claim != new.grounding_gate_bypass_without_claim {
        changed.push(RISKY_FIELD_NAMES[6]);
    }
    if old.distrust_self_reported_low_risk != new.distrust_self_reported_low_risk {
        changed.push(RISKY_FIELD_NAMES[7]);
    }
    if old.outcome_polarity != new.outcome_polarity {
        changed.push(RISKY_FIELD_NAMES[8]);
    }
    if old.threshold_overrides != new.threshold_overrides {
        changed.push(RISKY_FIELD_NAMES[9]);
    }
    changed
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

    /// publish 的 **demote 子步**：scope 内除新版本外全部 demote current_version；
    /// is_active 不动。**完整 publish = 本子步 + realign 子步**（见 handler line296-299）：
    /// realign 才负责把 is_active 迁到新版本（对已生效血缘）。单测把两子步分开锁，便于
    /// 各自验证；勿把本子步当成完整 publish 语义（历史上这里的注释曾误导）。
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
    fn publish_demote_current_substep_leaves_is_active_untouched() {
        // 仅测 publish 的 **demote 子步**（`publish_demote_current`）：它只把 current_version
        // 收敛到新版本，不碰 is_active。**这不是完整 publish 语义**——真 `publish_domain_profile`
        // 在 demote 之后还会调 `realign_active_to_current`（见 `realign_*_when_lineage_was_active`
        // 测试 + handler line296-299），对「已生效血缘」会把 is_active 迁到新版本（即时生效）。
        // 本测试只锁 demote 子步自身的不变量：demote 不应有副作用地改动 is_active。
        let mut rows = vec![(1, true, true), (2, true, false)];
        publish_demote_current(&mut rows, 2);
        let current: Vec<_> = rows.iter().filter(|(_, c, _)| *c).collect();
        assert_eq!(current.len(), 1, "demote 后只一条 current_version");
        assert_eq!(current[0].0, 2);
        // demote 子步本身不动 is_active（is_active 的迁移由后续 realign 子步负责）。
        assert!(rows[0].2, "demote 子步不改 is_active：版本1 此刻仍标 active");
        assert!(!rows[1].2, "demote 子步不激活版本2（realign 才迁移）");
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

    /// 红线钉死：AI 生成候选（`guide_profile::generate_domain_profile_candidate` 落库
    /// 强制 `is_active=false`+`current_version=false`，见该文件 :248-249）的整个生命周期，
    /// 在被人审 `activate` 之前**始终 is_active=false**——publish 定稿子步 + realign 子步
    /// 都不会让它生效。这是「AI 永不自动 verify / AI 生成候选须人审」红线在版本机制层的
    /// 护栏：blocked 场景下哪怕有人 publish 了 AI 草稿，realign 命中「血缘从未 active」→
    /// noop → 仍须显式 activate。
    #[test]
    fn ai_candidate_stays_inactive_until_human_activate() {
        // AI 生成候选落库态：单条草稿，既非 current 也非 active。
        let mut rows = vec![(1, false, false)];
        // 有人对它 publish（定稿一个新版本 v2）：demote 子步把 current 收敛到 v2。
        rows.push((2, true, false));
        publish_demote_current(&mut rows, 2);
        // realign 子步：血缘从未 active → noop。
        realign_active_to_current_sim(&mut rows, 2);
        assert!(
            rows.iter().all(|(_, _, a)| !*a),
            "AI 候选血缘从未 active → publish 后仍无任何 active 行（守住人审红线）"
        );
        // 必须显式 activate v2 才生效。
        activate_single(&mut rows, 2);
        let active: Vec<_> = rows.iter().filter(|(_, _, a)| *a).map(|(v, _, _)| *v).collect();
        assert_eq!(active, vec![2], "人审 activate 后 v2 才生效");
        // 此刻 v2 既 current 又 active → 运行时可加载。
        assert_eq!(rows.iter().filter(|(_, c, a)| *c && *a).count(), 1);
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

    // ───────────────────────────────────────────────────────────────
    // 分级二次确认：risky_fields_changed 纯函数 + 旁路稿/确认 sim。
    // 真 DomainProfile 夹具复用 lib 侧单一真相源（销售 DEFAULT vs 情感陪伴 example），
    // 直接验证真函数逻辑而非 sim 复刻。
    // ───────────────────────────────────────────────────────────────

    use crate::agent::{default_domain_profile, example_emotional_companion_profile};
    use crate::routes::domain_profiles::risky_fields_changed;

    #[test]
    fn risky_fields_changed_all_equal_empty() {
        // 同一份 profile 自比 → 无危险变更。
        let p = default_domain_profile("ws");
        assert!(risky_fields_changed(&p, &p).is_empty(), "完全相同 → 空");
        let q = p.clone();
        assert!(risky_fields_changed(&p, &q).is_empty(), "clone 自比 → 空");
    }

    #[test]
    fn risky_fields_changed_nonrisky_field_returns_empty() {
        // 只改黑名单外字段（display_name / description / prompt_fragment）→ 不算危险。
        let base = default_domain_profile("ws");
        let mut edited = base.clone();
        edited.display_name = "改个名字".to_string();
        edited.description = "改个简介".to_string();
        edited.prompt_fragment = Some("叠加业务上下文，不换人格本体".to_string());
        assert!(
            risky_fields_changed(&base, &edited).is_empty(),
            "纯普通字段变更不触发分级"
        );
    }

    #[test]
    fn risky_fields_changed_detects_emotional_companion_diff() {
        // 销售 DEFAULT → 情感陪伴 example：example 相对 default 恰好改了 4 个危险字段
        // （conversation_modes / operation_mode / grounding_gate_bypass_without_claim /
        // distrust_self_reported_low_risk，见 example_emotional_companion_profile）。
        let base = default_domain_profile("ws");
        let edited = example_emotional_companion_profile("ws");
        let changed = risky_fields_changed(&base, &edited);
        // 返回顺序与 RISKY_FIELD_NAMES 声明序一致。
        assert_eq!(
            changed,
            vec![
                "conversation_modes",
                "operation_mode",
                "grounding_gate_bypass_without_claim",
                "distrust_self_reported_low_risk",
            ],
            "恰好这 4 个危险字段（profile_id/display_name/prompt_fragment 等普通字段不计）"
        );
    }

    #[test]
    fn risky_fields_changed_single_diff_each_field() {
        // 逐个危险字段单改，确认每个都被独立检出（覆盖 10 字段比较分支）。
        let base = default_domain_profile("ws");

        let mut p = base.clone();
        p.soul_override = Some("换人格本体".to_string());
        assert_eq!(risky_fields_changed(&base, &p), vec!["soul_override"]);

        let mut p = base.clone();
        p.methodology_override = Some("换方法论".to_string());
        assert_eq!(risky_fields_changed(&base, &p), vec!["methodology_override"]);

        let mut p = base.clone();
        p.conversation_mode_policy = Some("## 对话模式判定\n换判定规则".to_string());
        assert_eq!(risky_fields_changed(&base, &p), vec!["conversation_mode_policy"]);

        let mut p = base.clone();
        p.grounding_gate_bypass_without_claim = !base.grounding_gate_bypass_without_claim;
        assert_eq!(
            risky_fields_changed(&base, &p),
            vec!["grounding_gate_bypass_without_claim"]
        );

        let mut p = base.clone();
        p.distrust_self_reported_low_risk = !base.distrust_self_reported_low_risk;
        assert_eq!(
            risky_fields_changed(&base, &p),
            vec!["distrust_self_reported_low_risk"]
        );

        let mut p = base.clone();
        p.threshold_overrides = Some(crate::models::ProfileThresholds {
            pressure_risk_block_at: Some(9),
            ..Default::default()
        });
        assert_eq!(risky_fields_changed(&base, &p), vec!["threshold_overrides"]);
    }

    #[test]
    fn risky_fields_changed_commitment_markers_uses_partial_eq() {
        // 钉死 CommitmentMarkers 的 PartialEq derive 生效：改其内 Vec 即被检出。
        let base = default_domain_profile("ws");
        let mut edited = base.clone();
        edited
            .commitment_markers
            .product_effect
            .push("根治率".to_string());
        assert_eq!(
            risky_fields_changed(&base, &edited),
            vec!["commitment_markers"],
            "CommitmentMarkers 内层 Vec 变更经 PartialEq 检出"
        );
    }

    #[test]
    fn risky_fields_changed_outcome_polarity_diff() {
        // outcome_polarity 经 PartialEq 检出（H11 自学习极性，高危）。
        let base = default_domain_profile("ws");
        let mut edited = base.clone();
        edited
            .outcome_polarity
            .positive
            .push("emotional_disclosure".to_string());
        assert_eq!(risky_fields_changed(&base, &edited), vec!["outcome_polarity"]);
    }

    /// publish 危险分支：落「旁路稿」(current=false)，不动正在生效的旧版本。
    /// 旧 v1 保持唯一 current+active（零窗口期回落 DEFAULT）。
    /// 对应 handler 危险分支：insert published(current=false)，不 demote / 不 realign。
    #[test]
    fn risky_publish_keeps_v1_current_active_via_sideline_draft() {
        // 元组 = (version, current_version, is_active)。v1 当前 current+active 生效。
        let mut rows = vec![(1, true, true)];
        // 危险 publish：插入 v2 旁路稿 current=false、is_active=false；**不** demote v1。
        rows.push((2, false, false));
        // 危险分支既不 demote 也不 realign：rows 保持原样。
        let current: Vec<_> = rows.iter().filter(|(_, c, _)| *c).map(|(v, _, _)| *v).collect();
        let active: Vec<_> = rows.iter().filter(|(_, _, a)| *a).map(|(v, _, _)| *v).collect();
        assert_eq!(current, vec![1], "旁路稿不占 current，v1 仍唯一 current（零窗口）");
        assert_eq!(active, vec![1], "v1 仍唯一 active，运行时不回落 DEFAULT");
        // 充要条件：v1 既 current 又 active，运行时继续加载旧版本。
        assert_eq!(rows.iter().filter(|(_, c, a)| *c && *a).count(), 1);
    }

    /// 二次确认经 rollout 把旁路稿推成 current+active（rollout = 推 current + demote +
    /// realign）。对应 confirm-path 复用 `rollout_domain_profile`。
    #[test]
    fn confirm_via_rollout_migrates_current_and_active_to_sideline_draft() {
        // v1 current+active 生效中，v2 是危险 publish 落的旁路稿。
        let mut rows = vec![(1, true, true), (2, false, false)];
        // rollout(v2)：推 v2 current=true，demote 其他 current。
        for (v, cur, _) in rows.iter_mut() {
            *cur = *v == 2;
        }
        // realign：血缘原本有 active（v1）→ 把 is_active 迁到新 current（v2）。
        realign_active_to_current_sim(&mut rows, 2);
        let loadable: Vec<_> = rows
            .iter()
            .filter(|(_, c, a)| *c && *a)
            .map(|(v, _, _)| *v)
            .collect();
        assert_eq!(loadable, vec![2], "确认后 v2 既 current 又 active，唯一可加载");
        // 单 current 不变量保持。
        assert_eq!(rows.iter().filter(|(_, c, _)| *c).count(), 1);
    }
}

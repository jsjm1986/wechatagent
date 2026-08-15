//! universal-domain-adaptation Phase 3（3A-3）：`domain_profiles` 行业总装配单的
//! admin REST + 不可变草稿、发布与显式激活。
//!
//! `DomainProfile` 携带与 ops 三表同构的 `(version, current_version, previous_version,
//! seeded_by)` 四元字段（见 [`crate::models::DomainProfile`]），故 publish / rollout /
//! rollback 行为照搬 [`crate::routes::admin_ops_versions`] 的 soft-demote 语义，scope =
//! `(workspace_id, profile_id)`。
//!
//! **publish / activate 语义**：
//! - `publish`：把不可变 draft 原位标为 `release_status=published` 并切成该血缘唯一
//!   `current_version=true`；只移动发布指针，绝不改任何行的 `is_active`。
//! - `rollout` / `rollback`：只在已发布历史中移动 `current_version`，同样不改运行时。
//! - `activate`：仅允许选中 published current；事务内把它设为 workspace 唯一
//!   `is_active=true`。这一步才改变运行时，并在核心指针提交后执行可重试附属同步。
//!
//! 因此发布后允许「旧版本 active + 新版本 current」并存；运行时只按 workspace 的唯一
//! `is_active=true` 读取，不要求 active 同时也是 current。此分离保证任何内容（包括普通字段）
//! 都必须经过管理员明确激活才生效。
//!
//! **红线**：引导层 AI 生成的 profile 必须人审才能 activate（继承「AI 永不自动 verify」）；
//! 候选不阻塞运行时（无 active 时回落 DEFAULT_PROFILE，零配置启动不变）。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
use mongodb::options::{FindOneOptions, FindOptions, TransactionOptions};
use mongodb::ClientSession;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

use crate::{
    auth::AuthenticatedAdmin,
    db::Database,
    error::{AppError, AppResult},
    models::{dt_to_string, DomainProfile},
};

use super::shared::{parse_object_id, resolve_authorized_workspace};
use super::AppState;

const DOMAIN_PROFILE_RELEASE_CONFLICT: &str = "domain_profile_release_conflict";
const DOMAIN_PROFILE_COMMIT_MAX_ATTEMPTS: usize = 3;
const DOMAIN_PROFILE_COMMIT_MAX_TIME: Duration = Duration::from_secs(5);

fn domain_profile_transaction_options() -> TransactionOptions {
    TransactionOptions::builder()
        .max_commit_time(DOMAIN_PROFILE_COMMIT_MAX_TIME)
        .build()
}

fn domain_profile_transaction_error(error: AppError) -> AppError {
    match error {
        AppError::Db(db_error) => {
            tracing::warn!(error = %db_error, "domain profile transaction conflicted");
            AppError::Conflict(DOMAIN_PROFILE_RELEASE_CONFLICT.to_string())
        }
        other => other,
    }
}

async fn commit_domain_profile_transaction(
    session: &mut ClientSession,
    db: &Database,
    committed_filter: Document,
) -> AppResult<()> {
    for attempt in 1..=DOMAIN_PROFILE_COMMIT_MAX_ATTEMPTS {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(error)
                if error.contains_label("UnknownTransactionCommitResult")
                    && attempt < DOMAIN_PROFILE_COMMIT_MAX_ATTEMPTS =>
            {
                tracing::warn!(
                    attempt,
                    "domain profile commit result unknown; retrying bounded commit"
                );
            }
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => {
                tracing::warn!(attempt, %error, "domain profile commit result remained unknown; reading authoritative row");
                return if db
                    .domain_profiles()
                    .find_one(committed_filter, None)
                    .await?
                    .is_some()
                {
                    Ok(())
                } else {
                    Err(AppError::Conflict(
                        "domain_profile_commit_result_unknown".to_string(),
                    ))
                };
            }
            Err(error) => {
                let _ = session.abort_transaction().await;
                tracing::warn!(error = %error, "domain profile transaction commit failed");
                return Err(AppError::Conflict(
                    DOMAIN_PROFILE_RELEASE_CONFLICT.to_string(),
                ));
            }
        }
    }
    unreachable!("finite domain profile commit loop always returns")
}

/// Validate every permanent property of an activation target before the
/// transaction changes the workspace runtime pointer. Transient writes may
/// still fail after activation and return `partial`, but malformed persisted
/// content must fail with zero writes because retrying cannot repair it.
fn validate_domain_profile_activation_target(target: &DomainProfile) -> AppResult<()> {
    crate::models::validate_domain_profile_dimensions(target).map_err(AppError::BadRequest)?;
    if let Some(machine) = target.generated_state_machine.as_ref() {
        let states = machine.get_array("states").map_err(|_| {
            AppError::BadRequest(
                "generatedStateMachine.states must be a non-empty array".to_string(),
            )
        })?;
        if states.is_empty() {
            return Err(AppError::BadRequest(
                "generatedStateMachine.states must be a non-empty array".to_string(),
            ));
        }
        crate::routes::domains::validate_state_machine(machine)?;
    }
    Ok(())
}

/// Append one immutable draft and allocate the lineage version inside the same
/// transaction. Concurrent editors may both prepare drafts, but at most one
/// can claim a particular version; the loser receives a stable 409.
pub(crate) async fn append_domain_profile_draft(
    db: &Database,
    mut draft: DomainProfile,
) -> AppResult<DomainProfile> {
    crate::models::validate_domain_profile_dimensions(&draft).map_err(AppError::BadRequest)?;
    let collection = db.domain_profiles();
    let mut session = db.client().start_session(None).await?;
    session
        .start_transaction(domain_profile_transaction_options())
        .await?;
    let result: AppResult<DomainProfile> = async {
        let latest = collection
            .find_one_with_session(
                doc! {
                    "workspace_id": &draft.workspace_id,
                    "profile_id": &draft.profile_id,
                },
                FindOneOptions::builder()
                    .sort(doc! { "version": -1_i32 })
                    .build(),
                &mut session,
            )
            .await?;
        draft.version = match latest {
            Some(ref row) => row.version.checked_add(1).ok_or_else(|| {
                AppError::BadRequest("domain profile version overflow".to_string())
            })?,
            None => 1,
        };
        let current = collection
            .find_one_with_session(
                doc! {
                    "workspace_id": &draft.workspace_id,
                    "profile_id": &draft.profile_id,
                    "current_version": true,
                },
                None,
                &mut session,
            )
            .await?;
        if current
            .as_ref()
            .is_some_and(|row| row.release_status != "published")
        {
            return Err(AppError::Conflict(
                "domain_profile_current_not_published".to_string(),
            ));
        }
        draft.id = None;
        draft.current_version = false;
        draft.previous_version = current.map(|row| row.version);
        draft.is_active = false;
        draft.release_status = "draft".to_string();
        let inserted = collection
            .insert_one_with_session(&draft, None, &mut session)
            .await?;
        draft.id = inserted.inserted_id.as_object_id();
        Ok(draft)
    }
    .await;
    let draft = match result {
        Ok(draft) => draft,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(domain_profile_transaction_error(error));
        }
    };
    let draft_id = draft.id.ok_or_else(|| {
        AppError::External("inserted domain profile draft missing _id".to_string())
    })?;
    commit_domain_profile_transaction(
        &mut session,
        db,
        doc! {
            "_id": draft_id,
            "workspace_id": &draft.workspace_id,
            "profile_id": &draft.profile_id,
            "version": draft.version,
            "release_status": "draft",
            "current_version": false,
            "is_active": false,
        },
    )
    .await?;
    Ok(draft)
}

/// Move the published current pointer for one lineage. `expected_status` is
/// `draft` for first publication and `published` for rollout/rollback.
/// Runtime activation is intentionally untouched.
async fn switch_domain_profile_current(
    db: &Database,
    workspace_id: &str,
    profile_id: &str,
    target_id: ObjectId,
    expected_status: &str,
) -> AppResult<DomainProfile> {
    let collection = db.domain_profiles();
    let mut session = db.client().start_session(None).await?;
    session
        .start_transaction(domain_profile_transaction_options())
        .await?;
    let result: AppResult<DomainProfile> = async {
        let target = collection
            .find_one_with_session(
                doc! {
                    "_id": target_id,
                    "workspace_id": workspace_id,
                    "profile_id": profile_id,
                    "release_status": expected_status,
                },
                None,
                &mut session,
            )
            .await?
            .ok_or_else(|| AppError::Conflict("domain_profile_target_changed".to_string()))?;
        crate::models::validate_domain_profile_dimensions(&target).map_err(AppError::BadRequest)?;
        if expected_status == "draft" && (target.current_version || target.is_active) {
            return Err(AppError::Conflict(
                "domain_profile_draft_state_invalid".to_string(),
            ));
        }

        let mut cursor = collection
            .find_with_session(
                doc! {
                    "workspace_id": workspace_id,
                    "profile_id": profile_id,
                    "current_version": true,
                },
                FindOptions::builder().limit(2).build(),
                &mut session,
            )
            .await?;
        let current = cursor.next(&mut session).await.transpose()?;
        if cursor.next(&mut session).await.transpose()?.is_some() {
            return Err(AppError::Conflict(
                "multiple_current_domain_profiles".to_string(),
            ));
        }
        if current.as_ref().and_then(|row| row.id) == Some(target_id) {
            return Ok(target);
        }

        let now = DateTime::now();
        if let Some(current) = current {
            let current_id = current.id.ok_or_else(|| {
                AppError::External("current domain profile missing _id".to_string())
            })?;
            let demoted = collection
                .update_one_with_session(
                    doc! {
                        "_id": current_id,
                        "workspace_id": workspace_id,
                        "profile_id": profile_id,
                        "current_version": true,
                    },
                    doc! { "$set": { "current_version": false, "updated_at": now } },
                    None,
                    &mut session,
                )
                .await?;
            if demoted.modified_count != 1 {
                return Err(AppError::Conflict(
                    "domain_profile_current_changed".to_string(),
                ));
            }
        }

        let promoted = collection
            .update_one_with_session(
                doc! {
                    "_id": target_id,
                    "workspace_id": workspace_id,
                    "profile_id": profile_id,
                    "release_status": expected_status,
                    "current_version": false,
                },
                doc! {
                    "$set": {
                        "release_status": "published",
                        "current_version": true,
                        "updated_at": now,
                    }
                },
                None,
                &mut session,
            )
            .await?;
        if promoted.modified_count != 1 {
            return Err(AppError::Conflict(
                "domain_profile_target_changed".to_string(),
            ));
        }
        let mut target = target;
        target.release_status = "published".to_string();
        target.current_version = true;
        target.updated_at = now;
        Ok(target)
    }
    .await;
    let target = match result {
        Ok(target) => target,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(domain_profile_transaction_error(error));
        }
    };
    commit_domain_profile_transaction(
        &mut session,
        db,
        doc! {
            "_id": target_id,
            "workspace_id": workspace_id,
            "profile_id": profile_id,
            "release_status": "published",
            "current_version": true,
            "updated_at": target.updated_at,
        },
    )
    .await?;
    Ok(target)
}

/// Atomically move the workspace runtime pointer to a published current row.
/// Repeating activation of the same target is a no-op so failed side effects
/// can be retried safely.
async fn switch_domain_profile_active(
    db: &Database,
    workspace_id: &str,
    target_id: ObjectId,
) -> AppResult<DomainProfile> {
    let collection = db.domain_profiles();
    let mut session = db.client().start_session(None).await?;
    session
        .start_transaction(domain_profile_transaction_options())
        .await?;
    let result: AppResult<DomainProfile> = async {
        let target = collection
            .find_one_with_session(
                doc! {
                    "_id": target_id,
                    "workspace_id": workspace_id,
                    "release_status": "published",
                    "current_version": true,
                },
                None,
                &mut session,
            )
            .await?
            .ok_or_else(|| AppError::Conflict("domain_profile_target_not_current".to_string()))?;
        validate_domain_profile_activation_target(&target)?;

        let mut cursor = collection
            .find_with_session(
                doc! { "workspace_id": workspace_id, "is_active": true },
                FindOptions::builder().limit(2).build(),
                &mut session,
            )
            .await?;
        let active = cursor.next(&mut session).await.transpose()?;
        if cursor.next(&mut session).await.transpose()?.is_some() {
            return Err(AppError::Conflict(
                "multiple_active_domain_profiles".to_string(),
            ));
        }
        if active.as_ref().and_then(|row| row.id) == Some(target_id) {
            return Ok(target);
        }

        let now = DateTime::now();
        if let Some(active) = active {
            let active_id = active.id.ok_or_else(|| {
                AppError::External("active domain profile missing _id".to_string())
            })?;
            let demoted = collection
                .update_one_with_session(
                    doc! {
                        "_id": active_id,
                        "workspace_id": workspace_id,
                        "is_active": true,
                    },
                    doc! { "$set": { "is_active": false, "updated_at": now } },
                    None,
                    &mut session,
                )
                .await?;
            if demoted.modified_count != 1 {
                return Err(AppError::Conflict(
                    "domain_profile_active_changed".to_string(),
                ));
            }
        }
        let promoted = collection
            .update_one_with_session(
                doc! {
                    "_id": target_id,
                    "workspace_id": workspace_id,
                    "release_status": "published",
                    "current_version": true,
                    "is_active": false,
                },
                doc! { "$set": { "is_active": true, "updated_at": now } },
                None,
                &mut session,
            )
            .await?;
        if promoted.modified_count != 1 {
            return Err(AppError::Conflict(
                "domain_profile_target_changed".to_string(),
            ));
        }
        let mut target = target;
        target.is_active = true;
        target.updated_at = now;
        crate::db::config_generation::bump_generation_with_session(
            db,
            crate::db::config_generation::DOMAIN_PROFILE_NAMESPACE,
            workspace_id,
            &mut session,
        )
        .await?;
        Ok(target)
    }
    .await;
    let target = match result {
        Ok(target) => target,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(domain_profile_transaction_error(error));
        }
    };
    commit_domain_profile_transaction(
        &mut session,
        db,
        doc! {
            "_id": target_id,
            "workspace_id": workspace_id,
            "release_status": "published",
            "current_version": true,
            "is_active": true,
            "updated_at": target.updated_at,
        },
    )
    .await?;
    Ok(target)
}

#[derive(Debug, Deserialize)]
pub(super) struct ListQuery {
    #[serde(default, rename = "workspaceId")]
    workspace_id: Option<String>,
    /// Compatibility flag. The default view already includes drafts, current
    /// published rows, and the active runtime row so reviewable drafts cannot
    /// disappear from the workflow.
    #[serde(default, rename = "includeAllVersions")]
    include_all_versions: bool,
}

pub(super) async fn list_domain_profiles(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id =
        resolve_authorized_workspace(&state, &admin, params.workspace_id.clone()).await?;
    let filter = doc! { "workspace_id": &workspace_id };
    let _ = params.include_all_versions;
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

/// 把 DomainProfile 序列化成前端视图：整体 serde + `id` 转 hex + 时间字段转 RFC3339。
///
/// `created_at` / `updated_at` 在 [`DomainProfile`] 上是裸 `bson::DateTime`，整体
/// `serde_json::to_value` 会把它们序列化成扩展 JSON 对象
/// `{"$date":{"$numberLong":"…"}}`（见 bson 的 `impl Serialize for DateTime`），
/// 而前端 TS 契约声明的是 `created_at?: string`，并直接把该值当 React child 渲染
/// （`{profile.updated_at}`）。对象作为 child 会让 React 抛
/// "Objects are not valid as a React child (found: object with keys {$date})"，
/// 整个「行业配置」tab 白屏。故此处统一经 `dt_to_string` 脱壳成 RFC3339 字符串，
/// 与 `ApiConfirmedTag`（D4-F1）/ 其余 routes 的 wire 形态一致。
fn profile_view(p: &DomainProfile) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        let hex = p.id.map(|i| i.to_hex()).unwrap_or_default();
        obj.insert("id".to_string(), json!(hex));
        // _id 是 BSON ObjectId 序列化形态,前端用上面的 hex `id` 即可。
        obj.remove("_id");
        obj.insert("created_at".to_string(), json!(dt_to_string(p.created_at)));
        obj.insert("updated_at".to_string(), json!(dt_to_string(p.updated_at)));
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

/// 取当前 workspace 运行时生效的 active profile（只读）。
///
/// Query the same unique `is_active=true` runtime row used by the cache. A
/// successful zero-row query is the only legal default-profile fallback;
/// multiple rows fail closed even before the unique index is available.
pub(super) async fn active_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .domain_profiles()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "is_active": true,
            },
            FindOptions::builder().limit(2).build(),
        )
        .await?;
    let first = cursor.try_next().await?;
    if cursor.try_next().await?.is_some() {
        return Err(AppError::Conflict(
            "multiple_active_domain_profiles".to_string(),
        ));
    }
    Ok(Json(json!({ "item": first.map(|p| profile_view(&p)) })))
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
    let workspace_id =
        resolve_authorized_workspace(&state, &admin, body.workspace_id.clone()).await?;
    if body.profile_id.is_empty() || body.profile_id.trim() != body.profile_id {
        return Err(AppError::BadRequest(
            "profileId must be non-empty and canonical".to_string(),
        ));
    }
    let now = DateTime::now();
    let mut doc = body.profile.clone();
    doc.insert("profile_id", &body.profile_id);
    doc.insert("workspace_id", &workspace_id);
    let mut profile: DomainProfile = mongodb::bson::from_document(doc)
        .map_err(|e| AppError::BadRequest(format!("profile 字段不合法: {e}")))?;
    profile.id = None;
    profile.profile_id = body.profile_id.clone();
    profile.workspace_id = workspace_id.clone();
    profile.version = 0;
    profile.current_version = false;
    profile.previous_version = None;
    profile.release_status = "draft".to_string();
    profile.is_active = false;
    profile.seeded_by = profile.seeded_by.or_else(|| Some("manual".to_string()));
    profile.created_at = now;
    profile.updated_at = now;
    let profile = append_domain_profile_draft(&state.db, profile).await?;
    Ok(Json(json!({ "item": profile_view(&profile) })))
}

/// Save edits by appending a new immutable draft derived from the selected
/// version. Published and active rows are never modified in place.
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
    let now = DateTime::now();
    let set_doc = strip_backend_managed_keys(&body.profile);
    let mut merged = mongodb::bson::to_document(&existing)
        .map_err(|e| AppError::External(format!("序列化现有 profile 失败: {e}")))?;
    for (k, v) in set_doc.iter() {
        merged.insert(k.clone(), v.clone());
    }
    let mut draft: DomainProfile = mongodb::bson::from_document(merged)
        .map_err(|e| AppError::BadRequest(format!("profile 字段不合法: {e}")))?;
    draft.id = None;
    draft.version = 0;
    draft.current_version = false;
    draft.previous_version = Some(existing.version);
    draft.release_status = "draft".to_string();
    draft.is_active = false;
    draft.seeded_by = Some("manual".to_string());
    draft.created_at = now;
    draft.updated_at = now;
    let draft = append_domain_profile_draft(&state.db, draft).await?;
    Ok(Json(json!({ "item": profile_view(&draft) })))
}

/// delete：禁止删除 active profile（须先 activate 另一条或回落 DEFAULT）。
/// `pub`（同 [`publish_domain_profile`] 先例）：集成测试 `tests/domain_profile_e2e.rs`
/// 直调本 handler 守护"删 active 被拒 / 删 draft 放行"的业务规则。
pub async fn delete_domain_profile(
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
    if existing.release_status != "draft" || existing.current_version || existing.is_active {
        return Err(AppError::BadRequest(
            "only an unpublished domain profile draft may be deleted".to_string(),
        ));
    }
    let deleted = coll
        .delete_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "release_status": "draft",
                "current_version": false,
                "is_active": false,
            },
            None,
        )
        .await?;
    if deleted.deleted_count != 1 {
        return Err(AppError::Conflict(
            "domain_profile_draft_changed".to_string(),
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

/// Publish an immutable draft as the lineage's unique current artifact.
/// Runtime activation is a separate explicit action: an old active version
/// keeps serving until `activate` succeeds, even after this pointer moves.
pub async fn publish_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let source = state
        .db
        .domain_profiles()
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("domain profile not found".to_string()))?;
    if source.release_status != "draft" || source.current_version || source.is_active {
        return Err(AppError::BadRequest(
            "only an unpublished domain profile draft may be published".to_string(),
        ));
    }
    let active_base = state
        .db
        .domain_profiles()
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
    let published = switch_domain_profile_current(
        &state.db,
        &source.workspace_id,
        &source.profile_id,
        object_id,
        "draft",
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "status": "published",
        "requiresActivation": true,
        "riskyFields": risky_changed,
        "id": object_id.to_hex(),
        "version": published.version,
    })))
}

/// Roll an already-published historical artifact forward to unique current.
/// This never changes the runtime active pointer.
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
    if target.release_status != "published" {
        return Err(AppError::BadRequest(
            "only a published domain profile may be rolled out".to_string(),
        ));
    }
    let target = switch_domain_profile_current(
        &state.db,
        &target.workspace_id,
        &target.profile_id,
        object_id,
        "published",
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "status": "published",
        "requiresActivation": true,
        "id": object_id.to_hex(),
        "version": target.version,
    })))
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
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "release_status": "published",
                "current_version": true,
            },
            None,
        )
        .await?
        .ok_or_else(|| {
            AppError::Conflict("domain_profile_rollback_source_not_current".to_string())
        })?;
    let prev_version = target.previous_version.ok_or_else(|| {
        AppError::BadRequest("target version has no previous_version recorded".to_string())
    })?;
    let prev = coll
        .find_one(
            doc! {
                "workspace_id": &target.workspace_id,
                "profile_id": &target.profile_id,
                "version": prev_version,
                "release_status": "published",
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
    switch_domain_profile_current(
        &state.db,
        &target.workspace_id,
        &target.profile_id,
        prev_id,
        "published",
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "status": "published",
        "requiresActivation": true,
        "id": prev_id.to_hex(),
        "rolledBackTo": prev_version,
    })))
}

/// Atomically activate the selected published current row. The core pointer
/// switch commits first; state-machine publication and contact realignment are
/// explicit retryable follow-up steps whose outcome is returned to the caller.
pub async fn activate_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let target =
        switch_domain_profile_active(&state.db, &admin.current_workspace, object_id).await?;
    // The runtime pointer is already committed. Invalidate before any optional
    // follow-up so the next decision observes the new profile even if a later
    // step fails and the response becomes partial.
    crate::agent::domain_profile::global_domain_profile_cache(&state.db)
        .invalidate_workspace(&admin.current_workspace);

    let now = DateTime::now();
    let state_machine_status;
    let mut state_policies_status = "skipped";
    let mut state_policies_report = json!(null);
    let mut contacts_status = "skipped";
    let mut contacts_matched = 0_u64;
    let mut contacts_modified = 0_u64;
    let mut errors: Vec<Value> = Vec::new();
    // Resolve one effective machine for every activation. A profile without an embedded
    // machine means "use the system default", not "keep whichever industry's workspace-global
    // machine happened to be active before". Publishing the explicit default is a no-op when the
    // workspace already uses it, and safely restores it after switching back from a custom domain.
    let effective_machine = target
        .generated_state_machine
        .clone()
        .unwrap_or_else(crate::prompts::default_user_operation_state_machine);
    let machine = &effective_machine;
    {
        match super::admin_ops_versions::publish_state_machine_version(
            &state.db,
            &target.workspace_id,
            crate::agent::domain::USER_OPS_DOMAIN_ID,
            machine.clone(),
            format!("profile:{}", target.profile_id),
        )
        .await
        {
            Err(err) => {
                state_machine_status = "failed";
                errors.push(json!({
                    "step": "stateMachine",
                    "code": "state_machine_publish_failed",
                    "message": err.to_string(),
                }));
                tracing::warn!(
                    profile_id = %target.profile_id,
                    workspace_id = %target.workspace_id,
                    error = %err,
                    "activate：状态机本体 publish 失败，profile 已激活，运行时保留原状态机（best-effort，不阻断激活）"
                );
            }
            Ok(report) => {
                state_machine_status = "completed";
                state_policies_status = if report.policies.is_complete() {
                    "completed"
                } else {
                    "partial"
                };
                if !report.policies.is_complete() {
                    errors.push(json!({
                        "step": "statePolicies",
                        "code": "state_policy_reconcile_partial",
                        "message": "one or more operation state policies were not reconciled",
                    }));
                }
                state_policies_report = json!(report.policies);
                // universal/H13 幻影态修复：状态机已成功切到新行业机器，但本 workspace 的
                // **存量** contact 的 `operation_state` 仍是旧机器的 key（如 sales 的
                // `"negotiating"`）。这些 key 在新机器里不存在 → 运行时 `check_state_transition`
                // 找不到 `from` 态 → 该 contact 的状态机 fail-soft 静默冻结永不推进（幻影态）。
                // 新建 contact 不受影响（从新机器 initial 起步）；只有切域前已存在的 contact 受困。
                //
                // 修复：把「`operation_state` 已设且不在新机器 key 集合里」的存量 contact
                // 批量重置到新机器声明的 initial 态（T10 保证 publish 通过的机器必有 initial:true，
                // 这里用 guards 同一抽取逻辑取 initial key，不引第二份事实源）。
                // 精确 scope（红线：这是一次破坏性批量改客户数据）：
                //   - 只本 workspace；
                //   - 只 `operation_state` 已设且 `$nin` 新 key 集（真正非法态）；
                //   - `None`/未设的 contact 被 `$exists:true,$ne:null` 排除 → 首次运行时再补 initial；
                //   - 仍合法的态被 `$nin` 排除 → 不被误重置。
                let new_keys: std::collections::HashSet<String> = machine
                    .get_array("states")
                    .map(|states| {
                        states
                            .iter()
                            .filter_map(|item| item.as_document())
                            .filter_map(|state| state.get_str("key").ok().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                // 防御：new_keys 为空时 `$nin:[]` 会匹配所有 contact → 误重置全量。T10 后
                // 不应发生（机器必有 state），但仍硬守：空集时跳过迁移。
                if new_keys.is_empty() {
                    contacts_status = "failed";
                    errors.push(json!({
                        "step": "contacts",
                        "code": "state_machine_has_no_states",
                        "message": "generated state machine has no state keys",
                    }));
                    tracing::warn!(
                        profile_id = %target.profile_id,
                        workspace_id = %target.workspace_id,
                        "activate：新状态机 key 集为空，跳过存量 contact 幻影态迁移（防止 $nin:[] 误重置全量）"
                    );
                } else {
                    let initial_key =
                        crate::agent::initial_operation_state_key_in_machine(Some(machine));
                    let nin: Vec<Bson> = new_keys.iter().cloned().map(Bson::String).collect();
                    match state
                            .db
                            .contacts()
                            .update_many(
                                doc! {
                                    "workspace_id": &target.workspace_id,
                                    "operation_state": {
                                        "$exists": true,
                                        "$ne": Bson::Null,
                                        "$nin": nin,
                                    },
                                },
                                doc! {
                                    "$set": {
                                        "operation_state": &initial_key,
                                        // 同步重写姊妹元数据：否则 reason/confidence/state_updated_at
                                        // 仍指向旧机器已删除的态，admin/审计视图会显示「态=新 initial
                                        // 但 reason=旧态描述」的错位三元组（运行时无害，下一轮 gateway
                                        // 覆盖，但审计期间不一致）。reason 打迁移标记、清掉旧 confidence。
                                        "operation_state_reason": "h13_phantom_state_migration: 换域激活，旧态在新状态机不存在，重置到 initial",
                                        "operation_state_updated_at": now,
                                        "updated_at": now,
                                    },
                                    "$unset": { "operation_state_confidence": "" },
                                },
                                None,
                            )
                            .await
                        {
                            Ok(result) => {
                                contacts_status = "completed";
                                contacts_matched = result.matched_count;
                                contacts_modified = result.modified_count;
                                tracing::info!(
                                    profile_id = %target.profile_id,
                                    workspace_id = %target.workspace_id,
                                    initial_key = %initial_key,
                                    matched = result.matched_count,
                                    modified = result.modified_count,
                                    "activate：存量 contact 幻影态已迁移到新状态机 initial 态"
                                );
                            }
                            Err(err) => {
                                contacts_status = "failed";
                                errors.push(json!({
                                    "step": "contacts",
                                    "code": "contact_state_migration_failed",
                                    "message": err.to_string(),
                                }));
                                tracing::warn!(
                                    profile_id = %target.profile_id,
                                    workspace_id = %target.workspace_id,
                                    error = %err,
                                    "activate：存量 contact 幻影态迁移失败（best-effort，状态机已切换，不阻断激活；受困 contact 将在下次运行时补 initial）"
                                );
                            }
                        }
                }
            }
        }
    }
    let partial = !errors.is_empty();
    Ok(Json(json!({
        "ok": true,
        "status": if partial { "partial" } else { "completed" },
        "retryable": partial,
        "activated": target.profile_id,
        "id": object_id.to_hex(),
        "version": target.version,
        "steps": {
            "profileActive": { "status": "completed" },
            "stateMachine": { "status": state_machine_status },
            "statePolicies": {
                "status": state_policies_status,
                "report": state_policies_report,
            },
            "contacts": {
                "status": contacts_status,
                "matched": contacts_matched,
                "modified": contacts_modified,
            },
        },
        "errors": errors,
    })))
}

/// 「危险开关」字段集：直接左右 AI 能否瞎编产品 / 自学习方向 / 人格本体 / 风控阈值 /
/// 交易事实注入 / 评审取向 / 模式-闸说明的 12 个字段。publish 对所有字段都不改变运行时；
/// 此列表用于响应中标出相对当前 active 的高风险差异，帮助管理员在显式 activate 前重点审阅。
/// `reviewer_orientation`（评审重点取向 / 转化平衡 / few-shot 打分锚）与
/// `mode_gate_policy_override`（模式与 5 闸说明散文）直接改写喂给 Review/Reply Agent 的
/// 取向 prompt，G31 起一并纳入风险提示。黑名单外字段仍不列入风险提示，但也必须经过
/// 独立 activate 才进入运行时。
const RISKY_FIELD_NAMES: [&str; 12] = [
    "soul_override",
    "methodology_override",
    "conversation_mode_policy",
    "conversation_modes",
    "operation_mode",
    "grounding_gate_bypass_without_claim",
    "distrust_self_reported_low_risk",
    "outcome_polarity",
    "threshold_overrides",
    "transaction_facts_enabled",
    "reviewer_orientation",
    "mode_gate_policy_override",
];

/// 比对两份 profile 的 12 个危险字段，返回**发生变化**的字段名列表（顺序与
/// [`RISKY_FIELD_NAMES`] 一致）。整体相等比较（逐字段 `!=`，偏保守：宁可多一次确认也
/// 不漏判）。历史 `commitment_markers` 不在审计清单内：它既不再写入，也不参与运行时语义判定；
/// `operation_mode` / `outcome_polarity` /
/// `threshold_overrides` / `reviewer_orientation` 依赖各自类型的 `PartialEq`（见 `models.rs`）。
///
/// 纯函数、无 IO，供 `publish_domain_profile` 生成审阅提示 + 单测共用。空 Vec = 无危险变更。
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
    if old.conversation_modes != new.conversation_modes {
        changed.push(RISKY_FIELD_NAMES[3]);
    }
    if old.operation_mode != new.operation_mode {
        changed.push(RISKY_FIELD_NAMES[4]);
    }
    if old.grounding_gate_bypass_without_claim != new.grounding_gate_bypass_without_claim {
        changed.push(RISKY_FIELD_NAMES[5]);
    }
    if old.distrust_self_reported_low_risk != new.distrust_self_reported_low_risk {
        changed.push(RISKY_FIELD_NAMES[6]);
    }
    if old.outcome_polarity != new.outcome_polarity {
        changed.push(RISKY_FIELD_NAMES[7]);
    }
    if old.threshold_overrides != new.threshold_overrides {
        changed.push(RISKY_FIELD_NAMES[8]);
    }
    if old.transaction_facts_enabled != new.transaction_facts_enabled {
        changed.push(RISKY_FIELD_NAMES[9]);
    }
    if old.reviewer_orientation != new.reviewer_orientation {
        changed.push(RISKY_FIELD_NAMES[10]);
    }
    if old.mode_gate_policy_override != new.mode_gate_policy_override {
        changed.push(RISKY_FIELD_NAMES[11]);
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
        "release_status",
        "is_active",
        "seeded_by",
        "created_at",
        "updated_at",
    ];
    // Deprecated semantic word-list field. It remains deserializable for old rows, but new
    // writes must discard it so the legacy classifier cannot be resurrected through the API.
    const RETIRED_CONTENT_KEYS: &[&str] = &["commitment_markers"];
    let mut set_doc = Document::new();
    for (k, v) in body.iter() {
        if BACKEND_MANAGED_KEYS.contains(&k.as_str()) || RETIRED_CONTENT_KEYS.contains(&k.as_str())
        {
            continue;
        }
        set_doc.insert(k.clone(), v.clone());
    }
    set_doc
}

#[cfg(test)]
mod tests {
    //! publish/activate 两步语义的纯函数不变量(DB 端 update_many 行为的可单测抽离)。
    //! 完整 DB-backed 端到端流程由 CI 集成套件覆盖(本地磁盘纪律:重套件走 CI)。

    /// publish / rollout / rollback 只移动 lineage 的 published-current 指针；
    /// `is_active` 是独立的 workspace 运行时指针，只有 activate 可以修改。
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
    fn publish_moves_current_but_preserves_old_active() {
        let mut rows = vec![(1, true, true), (2, false, false)];
        publish_demote_current(&mut rows, 2);
        let current: Vec<_> = rows.iter().filter(|(_, c, _)| *c).collect();
        assert_eq!(current.len(), 1, "demote 后只一条 current_version");
        assert_eq!(current[0].0, 2);
        assert!(rows[0].2, "旧版本继续作为运行时 active");
        assert!(!rows[1].2, "publish 不得隐式激活新版本");
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
        let mut rows = vec![(1, true, true), (2, false, false)];
        publish_demote_current(&mut rows, 2);
        assert_eq!(
            rows.iter().find(|(_, _, active)| *active).map(|row| row.0),
            Some(1),
            "publish 后运行时仍使用旧 active"
        );
        activate_single(&mut rows, 2);
        let active: Vec<_> = rows.iter().filter(|(_, _, active)| *active).collect();
        assert_eq!(active.len(), 1, "只一条 active 可被运行时加载");
        assert_eq!(active[0].0, 2);
        assert!(active[0].1, "激活目标仍是 published current");
    }

    #[test]
    fn ai_candidate_stays_inactive_until_human_activate() {
        let mut rows = vec![(1, false, false)];
        publish_demote_current(&mut rows, 1);
        assert!(
            rows.iter().all(|(_, _, a)| !*a),
            "AI 候选 publish 后仍无 active 行"
        );
        activate_single(&mut rows, 1);
        assert!(rows[0].1 && rows[0].2, "人审 activate 后才生效");
    }

    #[test]
    fn rollback_moves_current_but_preserves_runtime_active() {
        let mut rows = vec![(1, false, false), (2, true, true)];
        publish_demote_current(&mut rows, 1);
        assert!(rows[0].1 && !rows[0].2, "版本1成为发布 current，但未激活");
        assert!(!rows[1].1 && rows[1].2, "版本2继续服务运行时");
    }

    #[test]
    fn activation_target_rejects_permanently_invalid_generated_machine() {
        use mongodb::bson::doc;

        let mut profile = crate::agent::domain_profile::default_domain_profile("ws");
        profile.generated_state_machine = None;
        assert!(super::validate_domain_profile_activation_target(&profile).is_ok());

        profile.generated_state_machine = Some(doc! {
            "states": [{
                "key": "ready",
                "initial": true,
                "allowedFrom": ["ready"],
            }]
        });
        assert!(super::validate_domain_profile_activation_target(&profile).is_ok());

        profile.generated_state_machine = Some(doc! { "states": [] });
        assert!(matches!(
            super::validate_domain_profile_activation_target(&profile),
            Err(crate::error::AppError::BadRequest(_))
        ));

        profile.generated_state_machine = Some(doc! {
            "states": [{
                "key": "ready",
                "initial": true,
                "allowedFrom": ["missing"],
            }]
        });
        assert!(matches!(
            super::validate_domain_profile_activation_target(&profile),
            Err(crate::error::AppError::BadRequest(_))
        ));
    }

    #[test]
    fn strip_backend_managed_keys_drops_managed_and_keeps_only_present_content() {
        use mongodb::bson::doc;
        // body 同时含内容键与后端管理键 + 试图篡改的 is_active/version。
        let body = doc! {
            "display_name": "情感陪伴",
            "grounding_gate_bypass_without_claim": true,
            "commitment_markers": {
                "product_effect": ["不要再写入"],
                "tone_only": ["不要再写入"]
            },
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
        assert_eq!(
            set_doc
                .get_bool("grounding_gate_bypass_without_claim")
                .unwrap(),
            true
        );
        // 所有后端管理键被剥离。
        for k in [
            "is_active",
            "version",
            "current_version",
            "seeded_by",
            "_id",
            "id",
            "workspace_id",
            "profile_id",
            "commitment_markers",
        ] {
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
        let req: super::UpsertRequest = serde_json::from_value(body)
            .expect("缺 profileId 的 body 应能反序列化（update 路径不消费它）");
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
    // 激活前审阅提示：risky_fields_changed 纯函数。
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
        // 销售 DEFAULT → 情感陪伴 example：example 相对 default 恰好改了 5 个危险字段
        // （conversation_modes / operation_mode / grounding_gate_bypass_without_claim /
        // distrust_self_reported_low_risk / transaction_facts_enabled，见
        // example_emotional_companion_profile：交易域 true→非交易域 false）。
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
                "transaction_facts_enabled",
            ],
            "恰好这 5 个危险字段（profile_id/display_name/prompt_fragment 等普通字段不计）"
        );
    }

    #[test]
    fn risky_fields_changed_single_diff_each_field() {
        // 逐个危险字段单改，确认每个都被独立检出（覆盖 11 字段比较分支）。
        let base = default_domain_profile("ws");

        let mut p = base.clone();
        p.soul_override = Some("换人格本体".to_string());
        assert_eq!(risky_fields_changed(&base, &p), vec!["soul_override"]);

        let mut p = base.clone();
        p.methodology_override = Some("换方法论".to_string());
        assert_eq!(
            risky_fields_changed(&base, &p),
            vec!["methodology_override"]
        );

        let mut p = base.clone();
        p.conversation_mode_policy = Some("## 对话模式判定\n换判定规则".to_string());
        assert_eq!(
            risky_fields_changed(&base, &p),
            vec!["conversation_mode_policy"]
        );

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

        let mut p = base.clone();
        p.transaction_facts_enabled = !base.transaction_facts_enabled;
        assert_eq!(
            risky_fields_changed(&base, &p),
            vec!["transaction_facts_enabled"]
        );
    }

    #[test]
    fn risky_fields_detects_reviewer_orientation_and_mode_gate() {
        // reviewer 取向 / 模式-闸说明会直接改写喂给 Review/Reply Agent 的取向 prompt，
        // 因此必须出现在发布结果的 riskyFields 中，供激活前审阅。
        let base = default_domain_profile("ws");

        let mut changed_ro = base.clone();
        changed_ro.reviewer_orientation = Some(crate::models::ReviewerOrientation {
            review_focus: Some("真诚陪伴、尊重边界、不越界承诺。".to_string()),
            ..Default::default()
        });
        assert_eq!(
            risky_fields_changed(&base, &changed_ro),
            vec!["reviewer_orientation"],
            "reviewer_orientation 变更须被列为危险字段"
        );

        let mut changed_mg = base.clone();
        changed_mg.mode_gate_policy_override = Some("本域模式-闸说明".to_string());
        assert_eq!(
            risky_fields_changed(&base, &changed_mg),
            vec!["mode_gate_policy_override"],
            "mode_gate_policy_override 变更须被列为危险字段"
        );
    }

    #[test]
    fn risky_fields_changed_ignores_retired_commitment_markers() {
        // 旧词表字段不再是运行时策略，也不应触发激活前风险提示。
        let base = default_domain_profile("ws");
        let mut edited = base.clone();
        edited
            .commitment_markers
            .product_effect
            .push("根治率".to_string());
        assert!(risky_fields_changed(&base, &edited).is_empty());
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
        assert_eq!(
            risky_fields_changed(&base, &edited),
            vec!["outcome_polarity"]
        );
    }

    /// riskyFields 只改变审阅提示，不改变统一的发布语义：新版本成为 published current，
    /// 旧版本仍是 runtime active，直到显式 activate。
    #[test]
    fn risky_publish_moves_current_but_keeps_runtime_active() {
        let mut rows = vec![(1, true, true), (2, false, false)];
        publish_demote_current(&mut rows, 2);
        let current: Vec<_> = rows
            .iter()
            .filter(|(_, c, _)| *c)
            .map(|(v, _, _)| *v)
            .collect();
        let active: Vec<_> = rows
            .iter()
            .filter(|(_, _, a)| *a)
            .map(|(v, _, _)| *v)
            .collect();
        assert_eq!(current, vec![2], "新发布版本成为唯一 current");
        assert_eq!(active, vec![1], "旧版本继续作为唯一 runtime active");
    }

    /// rollout 也只移动 published current；不得借版本操作绕过显式 activate。
    #[test]
    fn rollout_preserves_runtime_active_until_activate() {
        let mut rows = vec![(1, true, true), (2, false, false)];
        publish_demote_current(&mut rows, 2);
        assert!(rows[0].2 && !rows[1].2, "rollout 不改变 active");
        activate_single(&mut rows, 2);
        assert!(!rows[0].2 && rows[1].2, "显式 activate 才切换运行时");
    }

    /// wire 契约护栏：`profile_view` 的 `created_at` / `updated_at` 必须是 RFC3339
    /// **字符串**，不能是 bson 扩展 JSON 对象 `{"$date":{"$numberLong":"…"}}`。
    ///
    /// 回归背景：`profile_view` 用 `serde_json::to_value(p)` 整体序列化，而
    /// `DomainProfile::{created_at,updated_at}` 是裸 `bson::DateTime`——bson 的
    /// `impl Serialize for DateTime` 无条件写 `serialize_struct("$date")`，于是
    /// wire 上变成对象。前端 TS 声明 `updated_at?: string` 且直接
    /// `{profile.updated_at}` 当 React child 渲染，对象会让 React 抛
    /// "Objects are not valid as a React child (found: object with keys {$date})"，
    /// 「行业配置」tab 整页白屏（无 ErrorBoundary 兜底 → root 卸载）。
    #[test]
    fn profile_view_serializes_timestamps_as_rfc3339_strings() {
        use crate::agent::domain_profile::default_domain_profile;

        let profile = default_domain_profile("default");
        let v = super::profile_view(&profile);

        for key in ["created_at", "updated_at"] {
            let field = v.get(key).unwrap_or_else(|| panic!("{key} 应存在于 wire"));
            assert!(
                field.is_string(),
                "{key} 必须是 RFC3339 字符串（前端直接渲染），实际={field}"
            );
            let s = field.as_str().unwrap();
            assert!(
                s.contains('T') && (s.ends_with('Z') || s.contains('+')),
                "{key} 应形如 RFC3339，实际={s}"
            );
            assert!(
                field.get("$date").is_none(),
                "{key} 不得是 bson 扩展 JSON 对象"
            );
        }

        // 整个 wire 里不得残留任何 $date / _id 键（嵌套字段一并守）。
        let dumped = serde_json::to_string(&v).expect("serialize wire");
        assert!(
            !dumped.contains("$date"),
            "wire 不得含 $date 扩展 JSON：{dumped}"
        );
        assert!(v.get("_id").is_none(), "_id 应被移除，前端用 hex id");
        assert!(v.get("id").is_some(), "id（hex）应存在");
    }

    /// 契约快照：`profile_view`。此投影此前完全在 `every_projection_has_contract_test`
    /// 扫描集之外（守卫只认 `_json` 后缀），是 `$date` 漏成对象、前端白屏的直接原因。
    /// 守卫已扩到 `_view`，这里补上真正的键集/形状对账。
    ///
    /// 时间戳必须钉死：`default_domain_profile` 内部用 `DateTime::now()`，
    /// 不覆盖会让 fixture 每次运行都漂移。
    #[test]
    fn profile_view_matches_contract_fixture() {
        use crate::agent::domain_profile::default_domain_profile;
        use mongodb::bson::{oid::ObjectId, DateTime};

        let mut profile = default_domain_profile("ws-1");
        profile.id = Some(ObjectId::parse_str("64a1f2c3e4b5a697889d0001").unwrap());
        profile.created_at = DateTime::from_millis(1_700_000_000_000);
        profile.updated_at = DateTime::from_millis(1_700_000_100_000);

        let projected = super::profile_view(&profile);
        crate::routes::contract_snapshot::assert_contract_fixture("domain_profile", projected);
    }
}

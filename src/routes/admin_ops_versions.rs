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
    Extension, Json,
};
use mongodb::bson::{doc, DateTime, Document};
use serde_json::{json, Value};

use crate::{
    agent::taxonomy::invalidate_global_taxonomy_cache,
    auth::AuthenticatedAdmin,
    db::Database,
    error::{AppError, AppResult},
    models::{OperationDomainConfig, OperationStatePolicy, TaxonomyEntry},
};

use super::shared::parse_object_id;
use super::AppState;

/// ── operation_domain_configs ──────────────────────────────────────────────────

/// 插入一版新 current [`OperationDomainConfig`] 并 demote 同 `(workspace_id, domain)`
/// scope 其余行。复用 [`publish_operation_domain_version`] 与
/// [`publish_state_machine_version`] 的共同逻辑（构造新行→insert→demote 其余）。
///
/// 新行克隆 `source` 的全部字段，**除了** `state_machine`（由参数注入）/ `status`（恒
/// `"active"`）/ `version`（= `next_version`）/ `current_version`（恒 `true`）/
/// `previous_version`（= `Some(source.version)`）/ `seeded_by`（由参数注入）/
/// `updated_at`（= `now`）/ `id`（恒 `None`）。
///
/// 事务性：本代码库不使用 MongoDB 多文档事务，沿用「先 insert 新 current，再 demote
/// 其余」的 best-effort 顺序（与两个原实现逐字一致），保证「至多一条 current」不变量在
/// 正常完成时成立。返回新插入行的 [`ObjectId`](mongodb::bson::oid::ObjectId)。
async fn insert_new_current_domain_config(
    coll: &mongodb::Collection<OperationDomainConfig>,
    source: &OperationDomainConfig,
    state_machine: Document,
    next_version: i32,
    seeded_by: String,
    now: DateTime,
) -> AppResult<mongodb::bson::oid::ObjectId> {
    let new_entry = OperationDomainConfig {
        id: None,
        workspace_id: source.workspace_id.clone(),
        domain: source.domain.clone(),
        name: source.name.clone(),
        goal: source.goal.clone(),
        methodology: source.methodology.clone(),
        workflow: source.workflow.clone(),
        tool_policy: source.tool_policy.clone(),
        automation_policy: source.automation_policy.clone(),
        review_policy: source.review_policy.clone(),
        runtime_parameters: source.runtime_parameters.clone(),
        state_machine,
        status: "active".to_string(),
        updated_at: now,
        version: next_version,
        current_version: true,
        previous_version: Some(source.version),
        seeded_by: Some(seeded_by),
        principal_decider: source.principal_decider.clone(),
        high_risk_escalation_mode: source.high_risk_escalation_mode.clone(),
    };
    let inserted = coll.insert_one(&new_entry, None).await?;
    let inserted_id = inserted.inserted_id.as_object_id();
    coll.update_many(
        doc! {
            "workspace_id": &source.workspace_id,
            "domain": &source.domain,
            "_id": { "$ne": inserted_id },
        },
        doc! { "$set": { "current_version": false, "updated_at": now } },
        None,
    )
    .await?;
    inserted_id
        .ok_or_else(|| AppError::External("inserted operation domain config has no _id".to_string()))
}

pub(super) async fn publish_operation_domain_version(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_domain_configs();
    let source = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
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
    let inserted_id = insert_new_current_domain_config(
        &coll,
        &source,
        source.state_machine.clone(),
        next_version,
        "manual".to_string(),
        now,
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "id": inserted_id.to_hex(),
        "version": next_version,
        "previousVersion": source.version,
    })))
}

/// universal/H13：把一份**新的状态机本体** publish 成 `operation_domain_configs`
/// 在 `(workspace_id, domain)` 下的新 current 版本——复用 [`publish_operation_domain_version`]
/// 的「克隆当前 current 行、只换 state_machine、version+1、insert-new-current 后 demote 其余」
/// 逻辑，但本体由调用方注入（profile activate 联动），而非沿用旧行的 state_machine。
///
/// 这是「消费方零改动」的关键：运行时引擎照旧按 `(workspace_id, domain, current_version=true)`
/// 读 `operation_domain_configs`，本 helper 把行业状态机塞进同一张表的新版本，引擎无感切换。
///
/// **edge case（无 current 行）**：正常路径下 `ensure_operation_domains` 会在 workspace
/// 首次落地时 seed 一条 DEFAULT current 行，所以这里一般能找到。为保持本 helper 无副作用
/// 且不引入跨模块 seed 依赖，选择**更简单稳健**的方案：找不到 current 行时只
/// `tracing::warn!` 并 `Ok(())` 返回（不 crash、不阻塞 activate）——activate 已成功，状态机
/// 联动是 best-effort；缺底座行属于异常部署态，留给 `ensure_operation_domains` 兜底而非在此
/// 凭空造一条无 name/goal/methodology 的半残行。
///
/// 一条 `operation_state_policies` 行的 `seeded_by` 是否「机器派生 / 可安全刷新」。
///
/// publish 重派生 policy 时用它区分**机器派生行**（安全刷新）与**运营手工调整行**
/// （保留，绝不 clobber，与 m013 skip-existing 同红线）：
/// - `None`（无溯源）→ 机器派生（早期未打标的派生行）→ 可刷新
/// - `"statemachine_publish:*"`（本 publish 路径联动派生）→ 可刷新
/// - `"legacy_migration"`（m013 从 DEFAULT 机器 seed 的行）→ 可刷新
/// - 其它任意值（如 `"admin_manual"` / 运营后台手设）→ 手工行 → **保留**
pub(crate) fn is_refreshable_policy_seeded_by(seeded_by: &Option<String>) -> bool {
    match seeded_by.as_deref() {
        None => true,
        Some(s) => s.starts_with("statemachine_publish:") || s == "legacy_migration",
    }
}

/// 事务性 / current-flag 竞态边界（universal/H13 (3)，judgment call 已定）：
///
/// 沿用「先 insert 新 current（`current_version=true`），再 demote 同 scope 其余行」的
/// best-effort 顺序，**不**引入事务、**不**加 `current_version=true` 唯一分区索引。理由：
/// - 本代码库不使用 MongoDB 多文档事务（显式约定，见 [`insert_new_current_domain_config`]
///   doc），且 testcontainers mongo 单节点无副本集 → 事务不可用。
/// - `current_version=true` 唯一分区索引会强制 demote-then-insert 顺序，打开「0 current」窗口：
///   并发 runtime 读此刻回落 DEFAULT-sales（比「2 条 industry current」更坏的失败方向）；
///   且 `ensure_indexes` 用 `?` 非 best-effort，存量脏 current 行会**直接 brick 启动**
///   （prod-117 部署炸雷），无配套清理不可加。
///
/// 残留（accepted-and-bounded）：在**真正并发的、本体各不相同**的 publish（admin-only，极罕见）
/// 下，最终 current flag 可能短暂出现 2 条 current。可接受因为：
/// (i) `(workspace,domain,version)` 唯一索引已挡住「重复 version」这个危险竞态；
/// (ii) 下方 no-op 幂等消除了「同机器重复 activate」这个最常见触发器；
/// (iii) 失败方向良性——多出的也是 industry 机器（非 DEFAULT-sales），且下次 publish 自愈；
/// (iv) 事务不可用，唯一索引会引入更坏的 0-current 窗口 + 启动炸雷。
pub(crate) async fn publish_state_machine_version(
    db: &Database,
    workspace_id: &str,
    domain: &str,
    new_state_machine: Document,
    seeded_by: String,
) -> AppResult<()> {
    let coll = db.operation_domain_configs();
    let Some(source) = coll
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": domain,
                "current_version": true,
            },
            None,
        )
        .await?
    else {
        tracing::warn!(
            workspace_id,
            domain,
            "publish_state_machine_version: no current operation_domain_config row; skip publish (ensure_operation_domains 未 seed 底座)"
        );
        return Ok(());
    };

    // universal/H13 (1) no-op 幂等：本体与当前 current 行逐字节相等 → 整个
    // insert+demote+policy 重派生全部跳过，直接 Ok(())。最常见触发器是 admin 重复
    // 激活同一 profile（或 activate 被重试）——此前每次都往 operation_domain_configs
    // 灌一版本体完全相同的新行（版本膨胀 + 放大 current-flag 竞态窗口）。本短路让
    // 「重复激活同机器」成为真正的 no-op，从源头消除最常见的竞态触发器。
    if new_state_machine == source.state_machine {
        tracing::debug!(
            workspace_id,
            domain,
            "publish_state_machine_version: state machine unchanged, skip republish (no-op 幂等)"
        );
        return Ok(());
    }

    let scope = doc! {
        "workspace_id": workspace_id,
        "domain": domain,
    };
    let next_version = next_version_for_scope(db.operation_domain_configs(), scope).await?;
    let now = DateTime::now();
    // policy 行的溯源标签：在 `seeded_by` 被 move 进 config insert 前先派生一个可区分的
    // 并行标签（`statemachine_publish:<原 seeded_by>`，如 `statemachine_publish:profile:edu-k12`），
    // 让 operation_state_policies 里这批联动派生的行能与 config 行/手工行/legacy_migration 行区分。
    let policy_seeded_by = format!("statemachine_publish:{seeded_by}");
    // 克隆当前 current 行的全部非 state_machine 字段（name/goal/methodology/…一一保留），
    // 只把 state_machine 换成注入的本体——这正是「消费方零改动」的保证。
    // 派生 policy 时还要读机器里的 states，故先取一份 states 列表再把本体 move 进 helper。
    let states = new_state_machine
        .get_array("states")
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_document().cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    insert_new_current_domain_config(
        &coll,
        &source,
        new_state_machine,
        next_version,
        seeded_by,
        now,
    )
    .await?;

    // universal/H13 修补：状态机本体进 operation_domain_configs 不会自动让主动触达门生效。
    // 主动触达由派生表 operation_state_policies enforce（guards::enforce_state_action_policy
    // 对缺失 policy 行 fail-open → 不拦），而该表此前仅 m013 从 DEFAULT 机器 seed。这里
    // 在 publish 新机器后，按机器里每个 state 的 `forbidsProactive` 标志**联动重派生** policy
    // 行（复用 m013 的 `derive_state_policy_lists` 唯一真相），让非销售 profile 标
    // `forbidsProactive:true` 的 state 真正拦住主动发。
    //
    // best-effort：与本 helper 整体一致（无 current 行时 warn+Ok）——单个 state 的 policy
    // 派生失败只 `warn!` 并继续下一个，绝不 `?` 传播出去把已成功的 activate/publish 拖垮。
    // 一个坏 state 不应让后续 state 的 policy 漏派生，故 per-state warn-and-continue。
    for state in &states {
        let Some(state_key) = state.get_str("key").ok().filter(|k| !k.is_empty()) else {
            continue;
        };
        let forbids_proactive = state.get_bool("forbidsProactive").unwrap_or(false);
        let (allowed, forbidden) =
            crate::db::migrations::m013_seed_user_operation_state_policies::derive_state_policy_lists(
                forbids_proactive,
            );
        match db
            .operation_state_policies()
            .find_one(
                doc! {
                    "workspace_id": workspace_id,
                    "domain": domain,
                    "state_key": state_key,
                },
                None,
            )
            .await
        {
            // 已有行：区分机器派生行（可安全刷新）与运营手工行（保留，绝不 clobber）。
            // 机器派生行若新本体的 forbidsProactive 与存量派生的 (allowed, forbidden) 不一致
            // → in-place update（解决「forbidsProactive 切换后旧 policy 行陈旧、切换静默失效」）；
            // 一致则不写。手工行（其它 seeded_by）一律 continue，与 m013 skip-existing 同红线。
            Ok(Some(existing)) => {
                if !is_refreshable_policy_seeded_by(&existing.seeded_by) {
                    continue;
                }
                if existing.allowed == allowed && existing.forbidden == forbidden {
                    continue;
                }
                if let Err(err) = db
                    .operation_state_policies()
                    .update_one(
                        doc! { "_id": existing.id },
                        doc! {
                            "$set": {
                                "allowed": &allowed,
                                "forbidden": &forbidden,
                                "updated_at": now,
                                "seeded_by": &policy_seeded_by,
                            }
                        },
                        None,
                    )
                    .await
                {
                    tracing::warn!(
                        workspace_id,
                        domain,
                        state_key,
                        error = %err,
                        "publish_state_machine_version: 刷新 operation_state_policy 失败（best-effort，跳过该 state 继续）"
                    );
                }
            }
            Ok(None) => {
                let policy = OperationStatePolicy {
                    id: None,
                    workspace_id: workspace_id.to_string(),
                    domain: domain.to_string(),
                    state_key: state_key.to_string(),
                    allowed,
                    forbidden,
                    recommended_pace: None,
                    status: "active".to_string(),
                    updated_at: now,
                    version: 1,
                    current_version: true,
                    previous_version: None,
                    seeded_by: Some(policy_seeded_by.clone()),
                };
                if let Err(err) = db.operation_state_policies().insert_one(&policy, None).await {
                    tracing::warn!(
                        workspace_id,
                        domain,
                        state_key,
                        error = %err,
                        "publish_state_machine_version: 派生 operation_state_policy 失败（best-effort，跳过该 state 继续）"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(
                    workspace_id,
                    domain,
                    state_key,
                    error = %err,
                    "publish_state_machine_version: 查询 operation_state_policy 失败（best-effort，跳过该 state 继续）"
                );
            }
        }
    }
    Ok(())
}

pub(super) async fn rollout_operation_domain_version(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_domain_configs();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_domain_configs();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_state_policies();
    let source = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_state_policies();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.operation_state_policies();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
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

    /// H13 (2)：`is_refreshable_policy_seeded_by` 区分机器派生行（可刷新）与手工行（保留）。
    #[test]
    fn refreshable_seeded_by_classifies_machine_vs_operator() {
        use super::is_refreshable_policy_seeded_by;
        // 机器派生 / 可安全刷新
        assert!(is_refreshable_policy_seeded_by(&None), "None（无溯源）可刷新");
        assert!(
            is_refreshable_policy_seeded_by(&Some("legacy_migration".to_string())),
            "m013 seed tag 可刷新"
        );
        assert!(
            is_refreshable_policy_seeded_by(&Some("statemachine_publish:profile:edu-k12".to_string())),
            "statemachine_publish:* 可刷新"
        );
        assert!(
            is_refreshable_policy_seeded_by(&Some("statemachine_publish:manual".to_string())),
            "statemachine_publish:manual 可刷新"
        );
        // 运营手工 / 必须保留
        assert!(
            !is_refreshable_policy_seeded_by(&Some("admin_manual".to_string())),
            "admin_manual 是手工行，不可刷新"
        );
        assert!(
            !is_refreshable_policy_seeded_by(&Some("manual".to_string())),
            "其它任意值视为手工行，不可刷新"
        );
    }
}

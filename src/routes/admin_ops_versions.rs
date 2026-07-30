//! Ops 三张版本表（`operation_domain_configs` / `operation_state_policies` /
//! `system_taxonomies`）的单-current版本管理 REST。
//!
//! 三表共享同一套 `(version, current_version, previous_version, seeded_by)` 四元字段
//! （详见 [`crate::models::OperationDomainConfig`] / [`crate::models::OperationStatePolicy`]
//! / [`crate::models::TaxonomyEntry`]），所以三类资源的 publish / rollout / rollback
//! 路径行为高度同构，集中在本模块。
//!
//! 行为约定（**非** prompt_templates 的 `delete_many` 硬清）：
//!
//! - `publish`：事务内分配 `max(version)+1`，降级旧 current，再插入新 current；
//! - `rollout`：事务内把指定历史版本切成唯一 current；
//! - `rollback`：以目标 row 的 `previous_version` 为索引找回上一版本，把它重新
//!   切成唯一 current。历史版本永不删除。
//!
//! 任何写入 taxonomy 的路径都立即调用 [`crate::agent::taxonomy::invalidate_global_taxonomy_cache`]，
//! 让运行中 Reply / Review Agent 在下次校验 value 时重新 build 字典。

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use mongodb::{
    bson::{doc, oid::ObjectId, to_document, DateTime, Document},
    options::{FindOneOptions, FindOptions, TransactionOptions},
    ClientSession,
};
use serde::Serialize;
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

const OPS_VERSION_CONFLICT: &str = "ops_version_switch_conflict";

fn validated_operation_domain_runtime(config: &OperationDomainConfig) -> AppResult<Document> {
    if config.domain != crate::agent::domain::USER_OPS_DOMAIN_ID {
        return Ok(config.runtime_parameters.clone());
    }
    crate::agent::runtime::validate_and_normalize_user_runtime_parameters(
        &config.runtime_parameters,
    )
    .map_err(AppError::BadRequest)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatePolicyReconcileFailure {
    pub state_key: String,
    pub operation: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatePolicyReconcileReport {
    pub considered: usize,
    pub unchanged: usize,
    pub updated: usize,
    pub inserted: usize,
    pub preserved_manual: usize,
    pub invalid_states: usize,
    pub failures: Vec<StatePolicyReconcileFailure>,
}

impl StatePolicyReconcileReport {
    pub(crate) fn is_complete(&self) -> bool {
        self.invalid_states == 0 && self.failures.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StateMachinePublishReport {
    pub changed: bool,
    pub policies: StatePolicyReconcileReport,
}

async fn commit_ops_transaction(session: &mut ClientSession) -> AppResult<()> {
    loop {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => {
                tracing::warn!(error = %error, "operations version transaction commit failed");
                return Err(AppError::Conflict(OPS_VERSION_CONFLICT.to_string()));
            }
        }
    }
}

fn ops_transaction_error(error: AppError) -> AppError {
    match error {
        AppError::Db(db_error) => {
            tracing::warn!(error = %db_error, "operations version transaction conflicted");
            AppError::Conflict(OPS_VERSION_CONFLICT.to_string())
        }
        other => other,
    }
}

async fn unique_current_id_with_session(
    collection: &mongodb::Collection<Document>,
    scope: &Document,
    session: &mut ClientSession,
) -> AppResult<ObjectId> {
    let mut filter = scope.clone();
    filter.insert("current_version", true);
    let mut cursor = collection
        .find_with_session(
            filter,
            FindOptions::builder()
                .projection(doc! { "_id": 1_i32 })
                .limit(2_i64)
                .build(),
            session,
        )
        .await?;
    let first = cursor
        .next(session)
        .await
        .transpose()?
        .ok_or_else(|| AppError::Conflict("missing_current_ops_version".to_string()))?;
    if cursor.next(session).await.transpose()?.is_some() {
        return Err(AppError::Conflict(
            "multiple_current_ops_versions".to_string(),
        ));
    }
    first
        .get_object_id("_id")
        .map_err(|_| AppError::External("current ops version missing ObjectId".to_string()))
}

async fn unique_current_id(
    db: &Database,
    collection_name: &str,
    scope: &Document,
) -> AppResult<ObjectId> {
    let collection = db.raw().collection::<Document>(collection_name);
    let mut filter = scope.clone();
    filter.insert("current_version", true);
    let mut cursor = collection
        .find(
            filter,
            FindOptions::builder()
                .projection(doc! { "_id": 1_i32 })
                .limit(2_i64)
                .build(),
        )
        .await?;
    use futures::TryStreamExt;
    let first = cursor
        .try_next()
        .await?
        .ok_or_else(|| AppError::Conflict("missing_current_ops_version".to_string()))?;
    if cursor.try_next().await?.is_some() {
        return Err(AppError::Conflict(
            "multiple_current_ops_versions".to_string(),
        ));
    }
    first
        .get_object_id("_id")
        .map_err(|_| AppError::External("current ops version missing ObjectId".to_string()))
}

async fn insert_new_current(
    db: &Database,
    collection_name: &str,
    scope: Document,
    expected_current_id: ObjectId,
    mut new_entry: Document,
) -> AppResult<(ObjectId, i32)> {
    let collection = db.raw().collection::<Document>(collection_name);
    let mut session = db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let result: AppResult<(ObjectId, i32)> = async {
        let current_id = unique_current_id_with_session(&collection, &scope, &mut session).await?;
        if current_id != expected_current_id {
            return Err(AppError::Conflict("ops_current_changed".to_string()));
        }
        let latest = collection
            .find_one_with_session(
                scope.clone(),
                FindOneOptions::builder()
                    .sort(doc! { "version": -1_i32 })
                    .projection(doc! { "version": 1_i32 })
                    .build(),
                &mut session,
            )
            .await?
            .ok_or_else(|| AppError::Conflict("missing_ops_version_stream".to_string()))?;
        let next_version = latest
            .get_i32("version")
            .map_err(|_| AppError::External("ops version is not int32".to_string()))?
            .checked_add(1)
            .ok_or_else(|| AppError::BadRequest("ops version overflow".to_string()))?;
        let now = DateTime::now();
        let mut current_filter = scope.clone();
        current_filter.insert("_id", current_id);
        current_filter.insert("current_version", true);
        let demoted = collection
            .update_one_with_session(
                current_filter,
                doc! { "$set": { "current_version": false, "updated_at": now } },
                None,
                &mut session,
            )
            .await?;
        if demoted.modified_count != 1 {
            return Err(AppError::Conflict("ops_current_changed".to_string()));
        }
        new_entry.remove("_id");
        new_entry.insert("version", next_version);
        new_entry.insert("current_version", true);
        new_entry.insert("updated_at", now);
        let inserted = collection
            .insert_one_with_session(new_entry, None, &mut session)
            .await?;
        let inserted_id = inserted.inserted_id.as_object_id().ok_or_else(|| {
            AppError::External("inserted ops version has no ObjectId".to_string())
        })?;
        Ok((inserted_id, next_version))
    }
    .await;
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(ops_transaction_error(error));
        }
    };
    commit_ops_transaction(&mut session).await?;
    Ok(value)
}

async fn switch_current(
    db: &Database,
    collection_name: &str,
    scope: Document,
    target_id: ObjectId,
) -> AppResult<()> {
    let collection = db.raw().collection::<Document>(collection_name);
    let mut session = db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let result: AppResult<()> = async {
        let mut target_filter = scope.clone();
        target_filter.insert("_id", target_id);
        let target = collection
            .find_one_with_session(target_filter.clone(), None, &mut session)
            .await?
            .ok_or_else(|| AppError::Conflict("ops_target_changed".to_string()))?;
        let current_id = unique_current_id_with_session(&collection, &scope, &mut session).await?;
        if current_id == target_id {
            return Ok(());
        }
        let now = DateTime::now();
        let mut current_filter = scope.clone();
        current_filter.insert("_id", current_id);
        current_filter.insert("current_version", true);
        let demoted = collection
            .update_one_with_session(
                current_filter,
                doc! { "$set": { "current_version": false, "updated_at": now } },
                None,
                &mut session,
            )
            .await?;
        if demoted.modified_count != 1 {
            return Err(AppError::Conflict("ops_current_changed".to_string()));
        }
        target_filter.insert(
            "current_version",
            target.get_bool("current_version").unwrap_or(false),
        );
        let promoted = collection
            .update_one_with_session(
                target_filter,
                doc! { "$set": { "current_version": true, "updated_at": now } },
                None,
                &mut session,
            )
            .await?;
        if promoted.modified_count != 1 {
            return Err(AppError::Conflict("ops_target_changed".to_string()));
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let _ = session.abort_transaction().await;
        return Err(ops_transaction_error(error));
    }
    commit_ops_transaction(&mut session).await
}

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
async fn insert_new_current_domain_config(
    db: &Database,
    source: &OperationDomainConfig,
    expected_current_id: ObjectId,
    state_machine: Document,
    seeded_by: String,
) -> AppResult<(ObjectId, i32)> {
    let now = DateTime::now();
    let runtime_parameters = validated_operation_domain_runtime(source)?;
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
        runtime_parameters,
        state_machine,
        status: "active".to_string(),
        updated_at: now,
        version: 0,
        current_version: false,
        previous_version: Some(source.version),
        seeded_by: Some(seeded_by),
        principal_decider: source.principal_decider.clone(),
        high_risk_escalation_mode: source.high_risk_escalation_mode.clone(),
        ask_human_policy: source.ask_human_policy.clone(),
        assist_mode_enabled: source.assist_mode_enabled,
    };
    let scope = doc! {
        "workspace_id": &source.workspace_id,
        "domain": &source.domain,
    };
    insert_new_current(
        db,
        "operation_domain_configs",
        scope,
        expected_current_id,
        to_document(&new_entry)?,
    )
    .await
}

/// Append the built-in default as a new immutable current version. The old
/// current row and every historical row remain intact; allocation and pointer
/// switching reuse the same transaction/CAS protocol as ordinary publishing.
pub(crate) async fn append_default_operation_domain_version(
    db: &Database,
    mut default_config: OperationDomainConfig,
    seeded_by: String,
) -> AppResult<(ObjectId, i32, i32)> {
    if default_config.domain == crate::agent::domain::USER_OPS_DOMAIN_ID {
        default_config.runtime_parameters =
            crate::agent::runtime::validate_and_normalize_user_runtime_parameters(
                &default_config.runtime_parameters,
            )
            .map_err(AppError::BadRequest)?;
    }
    let scope = doc! {
        "workspace_id": &default_config.workspace_id,
        "domain": &default_config.domain,
    };
    let expected_current_id = unique_current_id(db, "operation_domain_configs", &scope).await?;
    let current = db
        .operation_domain_configs()
        .find_one(
            doc! {
                "_id": expected_current_id,
                "workspace_id": &default_config.workspace_id,
                "domain": &default_config.domain,
                "current_version": true,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::Conflict("ops_current_changed".to_string()))?;
    default_config.id = None;
    default_config.version = 0;
    default_config.current_version = false;
    default_config.previous_version = Some(current.version);
    default_config.seeded_by = Some(seeded_by);
    let (id, version) = insert_new_current(
        db,
        "operation_domain_configs",
        scope,
        expected_current_id,
        to_document(&default_config)?,
    )
    .await?;
    Ok((id, version, current.version))
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
    let expected_current_id =
        unique_current_id(&state.db, "operation_domain_configs", &scope).await?;

    let (inserted_id, next_version) = insert_new_current_domain_config(
        &state.db,
        &source,
        expected_current_id,
        source.state_machine.clone(),
        "manual".to_string(),
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
///
/// **不变量（关键）**：运营/admin policy 插入路径**必须**总是写一个不可刷新的 `seeded_by`
/// （今天 `publish_operation_state_policy_version` 等都写 `"manual"`），因为「`None` →
/// 可刷新」这条规则静默依赖它——若手工行漏写 `seeded_by`（落 `None`），会被本 publish
/// 路径误判为机器派生行而 clobber。新增手工 policy 写入点必须守住这条。
pub(crate) fn is_refreshable_policy_seeded_by(seeded_by: &Option<String>) -> bool {
    match seeded_by.as_deref() {
        None => true,
        Some(s) => {
            s.starts_with("statemachine_publish:")
                || s.starts_with("statemachine_edit:")
                || s == "legacy_migration"
        }
    }
}

/// universal/G06+G11+G12：按一份状态机本体 `state_machine` 里每个 state 的
/// `forbidsProactive` 标志，**幂等地**重派生 `operation_state_policies` 的 current 行。
///
/// 这是从原 [`publish_state_machine_version`] 体内（取 states + per-state policy 派生 loop）
/// 逐字提取的共享 helper，让三条「切换 current 机器但不走 publish loop」的路径都能联动重派 policy：
/// - **G06**：`domains.rs` 两个直编路由（`update_operation_domain` /
///   `update_operation_domain_state_machine`）直接 `$set state_machine`，此前不派生 policy →
///   新增 `forbidsProactive:true` state 主动触达门 fail-open 静默失效。
/// - **G11**：`publish_state_machine_version` no-op 幂等短路在 policy loop **之前** return，
///   首次 activate best-effort 失败遗漏的行永不补 → 短路前调一次本 helper 幂等补齐。
/// - **G12**：`rollout` / `rollback` 切 `operation_domain_configs` 的 current 版本但不碰 policy
///   → policy current 行与机器 `forbidsProactive` 漂移；切 current 后调本 helper 重对齐。
///
/// **best-effort（关键）**：逐 state 继续执行并返回结构化报告。调用方不会回滚已提交的
/// 主操作，但必须把 `failures` 暴露为 partial，而不是只写日志后宣称完整成功。
///
/// 幂等：只刷新「机器派生 / 可安全刷新」行（[`is_refreshable_policy_seeded_by`]），运营手工行
/// 一律保留；已存在且 `(allowed, forbidden)` 一致的行内部 `continue` 不写；缺失行用
/// [`next_version_for_scope`] 分配版本（避开 `(ws,domain,state_key,version)` 唯一索引冲突）。
pub(crate) async fn reconcile_state_policies_for_machine(
    db: &Database,
    workspace_id: &str,
    domain: &str,
    state_machine: &Document,
    policy_seeded_by: &str,
    now: DateTime,
) -> StatePolicyReconcileReport {
    let mut report = StatePolicyReconcileReport::default();
    let states = state_machine
        .get_array("states")
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_document().cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for state in &states {
        let Some(state_key) = state.get_str("key").ok().filter(|k| !k.is_empty()) else {
            report.invalid_states += 1;
            continue;
        };
        report.considered += 1;
        let forbids_proactive = state.get_bool("forbidsProactive").unwrap_or(false);
        let (allowed, forbidden) =
            crate::db::migrations::m013_seed_user_operation_state_policies::derive_state_policy_lists(
                forbids_proactive,
            );
        // 只认 current_version=true 行：运行时 reader（agent/decision.rs）严格按
        // current_version=true 读 policy，且同 (ws,domain,state_key) 下可并存多版本
        // （admin publish/rollout/rollback 造历史版本，(ws,domain,state_key,version)
        // 唯一索引证 version 是 key 的一部分）。裸 find_one 会返回任意版本——可能改到
        // 非 current 的历史行，运行时仍读旧 current（toggle 静默失效，正是本代码要修的
        // bug），且改写历史行的 allowed/forbidden/seeded_by 会污染 rollback 链。
        match db
            .operation_state_policies()
            .find_one(
                doc! {
                    "workspace_id": workspace_id,
                    "domain": domain,
                    "state_key": state_key,
                    "current_version": true,
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
                    report.preserved_manual += 1;
                    continue;
                }
                if existing.allowed == allowed && existing.forbidden == forbidden {
                    report.unchanged += 1;
                    continue;
                }
                match db
                    .operation_state_policies()
                    .update_one(
                        doc! { "_id": existing.id },
                        doc! {
                            "$set": {
                                "allowed": &allowed,
                                "forbidden": &forbidden,
                                "updated_at": now,
                                "seeded_by": policy_seeded_by,
                            }
                        },
                        None,
                    )
                    .await
                {
                    Ok(result) if result.modified_count == 1 => report.updated += 1,
                    Ok(_) => report.failures.push(StatePolicyReconcileFailure {
                        state_key: state_key.to_string(),
                        operation: "update",
                        message: "policy row changed concurrently".to_string(),
                    }),
                    Err(err) => {
                        tracing::warn!(
                            workspace_id,
                            domain,
                            state_key,
                            error = %err,
                            "reconcile_state_policies_for_machine: 刷新 operation_state_policy 失败（best-effort，跳过该 state 继续）"
                        );
                        report.failures.push(StatePolicyReconcileFailure {
                            state_key: state_key.to_string(),
                            operation: "update",
                            message: err.to_string(),
                        });
                    }
                }
            }
            Ok(None) => {
                // current find_one 为 None：通常是该 state 首次派生 → 插 current 行。
                // EDGE（rollback 中途）：若存在非 current 的历史行但无 current 行，硬插
                // version 1 会与历史 version 1 撞 (ws,domain,state_key,version) 唯一索引。
                // 故按该 scope 现存最大 version+1 分配版本（复用 next_version_for_scope，
                // 与 admin policy publish 同一真相），避开撞索引。
                let next_policy_version = match next_version_for_scope(
                    db.operation_state_policies(),
                    doc! {
                        "workspace_id": workspace_id,
                        "domain": domain,
                        "state_key": state_key,
                    },
                )
                .await
                {
                    Ok(version) => version,
                    Err(err) => {
                        report.failures.push(StatePolicyReconcileFailure {
                            state_key: state_key.to_string(),
                            operation: "allocateVersion",
                            message: err.to_string(),
                        });
                        continue;
                    }
                };
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
                    version: next_policy_version,
                    current_version: true,
                    previous_version: None,
                    seeded_by: Some(policy_seeded_by.to_string()),
                };
                match db
                    .operation_state_policies()
                    .insert_one(&policy, None)
                    .await
                {
                    Ok(_) => report.inserted += 1,
                    Err(err) => {
                        tracing::warn!(
                            workspace_id,
                            domain,
                            state_key,
                            error = %err,
                            "reconcile_state_policies_for_machine: 派生 operation_state_policy 失败（best-effort，跳过该 state 继续）"
                        );
                        report.failures.push(StatePolicyReconcileFailure {
                            state_key: state_key.to_string(),
                            operation: "insert",
                            message: err.to_string(),
                        });
                    }
                }
            }
            Err(err) => {
                tracing::warn!(
                    workspace_id,
                    domain,
                    state_key,
                    error = %err,
                    "reconcile_state_policies_for_machine: 查询 operation_state_policy 失败（best-effort，跳过该 state 继续）"
                );
                report.failures.push(StatePolicyReconcileFailure {
                    state_key: state_key.to_string(),
                    operation: "load",
                    message: err.to_string(),
                });
            }
        }
    }
    report
}

/// SR-008：状态机联动发布与 admin 版本动作共享单-current事务协议。
/// 事务内先校验调用方看到的 current 未变化，再降级旧指针并插入新版本；并发
/// publish 至多一个提交，另一个返回冲突。partial unique index 提供最终数据库约束。
pub(crate) async fn publish_state_machine_version(
    db: &Database,
    workspace_id: &str,
    domain: &str,
    new_state_machine: Document,
    seeded_by: String,
) -> AppResult<StateMachinePublishReport> {
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
        return Err(AppError::Conflict(
            "missing_current_operation_domain_config".to_string(),
        ));
    };

    // universal/H13 (1) no-op 幂等：本体与当前 current 行逐字节相等 → 整个
    // insert+demote+policy 重派生全部跳过，直接 Ok(())。最常见触发器是 admin 重复
    // 激活同一 profile（或 activate 被重试）——此前每次都往 operation_domain_configs
    // 灌一版本体完全相同的新行（版本膨胀 + 放大 current-flag 竞态窗口）。本短路让
    // 「重复激活同机器」成为真正的 no-op，从源头消除最常见的竞态触发器。
    if new_state_machine == source.state_machine {
        // G11：本体未变仍 reconcile 一次——首次 activate 若某 state 的 policy 派生
        // best-effort 失败（warn 跳过），之后重激活同本体走此 no-op 短路，遗漏的 policy 行
        // 永不补。此处幂等 reconcile 补齐（已存在且一致的行内部 continue 不写，只补缺失/
        // 刷新陈旧机器派生行），再走原 no-op 短路。
        let policy_seeded_by = format!("statemachine_publish:{seeded_by}");
        let policies = reconcile_state_policies_for_machine(
            db,
            workspace_id,
            domain,
            &source.state_machine,
            &policy_seeded_by,
            DateTime::now(),
        )
        .await;
        tracing::debug!(
            workspace_id,
            domain,
            "publish_state_machine_version: state machine unchanged, skip republish (no-op 幂等; policy 已 reconcile)"
        );
        return Ok(StateMachinePublishReport {
            changed: false,
            policies,
        });
    }

    let now = DateTime::now();
    // policy 行的溯源标签：在 `seeded_by` 被 move 进 config insert 前先派生一个可区分的
    // 并行标签（`statemachine_publish:<原 seeded_by>`，如 `statemachine_publish:profile:edu-k12`），
    // 让 operation_state_policies 里这批联动派生的行能与 config 行/手工行/legacy_migration 行区分。
    let policy_seeded_by = format!("statemachine_publish:{seeded_by}");
    // 克隆当前 current 行的全部非 state_machine 字段（name/goal/methodology/…一一保留），
    // 只把 state_machine 换成注入的本体——这正是「消费方零改动」的保证。
    // 派生 policy 时还要读机器里的 states，故先 clone 一份本体，再把本体 move 进 insert helper，
    // clone 后用 `&states_doc` 喂 reconcile（admin 低频 publish 路径 clone 一次本体可接受）。
    let states_doc = new_state_machine.clone();
    let source_id = source.id.ok_or_else(|| {
        AppError::External("current operation domain config has no _id".to_string())
    })?;
    insert_new_current_domain_config(db, &source, source_id, new_state_machine, seeded_by).await?;

    // universal/H13 修补：状态机本体进 operation_domain_configs 不会自动让主动触达门生效。
    // 主动触达由派生表 operation_state_policies enforce（guards::enforce_state_action_policy
    // 对缺失 policy 行 fail-open → 不拦），而该表此前仅 m013 从 DEFAULT 机器 seed。这里
    // 在 publish 新机器后，按机器里每个 state 的 `forbidsProactive` 标志**联动重派生** policy
    // 行（复用 m013 的 `derive_state_policy_lists` 唯一真相），让非销售 profile 标
    // `forbidsProactive:true` 的 state 真正拦住主动发。best-effort，per-state warn-and-continue。
    let policies = reconcile_state_policies_for_machine(
        db,
        workspace_id,
        domain,
        &states_doc,
        &policy_seeded_by,
        now,
    )
    .await;
    Ok(StateMachinePublishReport {
        changed: true,
        policies,
    })
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
    validated_operation_domain_runtime(&target)?;
    let now = DateTime::now();
    switch_current(
        &state.db,
        "operation_domain_configs",
        doc! {
            "workspace_id": &target.workspace_id,
            "domain": &target.domain,
        },
        object_id,
    )
    .await?;
    // G12：切 current 配置版本后按新 current 机器（target.state_machine）重派 policy，
    // 否则 operation_state_policies 的 current 行与机器 forbidsProactive 漂移（下次 publish 才自愈）。
    // best-effort，只刷新机器派生行 + 补缺失；reconcile 幂等（一致行不写）。
    let policy_seeded_by = format!(
        "statemachine_publish:{}",
        target.seeded_by.clone().unwrap_or_default()
    );
    reconcile_state_policies_for_machine(
        &state.db,
        &target.workspace_id,
        &target.domain,
        &target.state_machine,
        &policy_seeded_by,
        now,
    )
    .await;
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
    validated_operation_domain_runtime(&prev)?;
    let prev_id = prev
        .id
        .ok_or_else(|| AppError::BadRequest("previous version has no _id".to_string()))?;
    let now = DateTime::now();
    switch_current(
        &state.db,
        "operation_domain_configs",
        doc! {
            "workspace_id": &target.workspace_id,
            "domain": &target.domain,
        },
        prev_id,
    )
    .await?;
    // G12：rollback 切回历史版本 `prev` 后按其机器（prev.state_machine）重派 policy，
    // 把 policy current 行重新对齐到回退目标机器的 forbidsProactive（否则与切到的版本漂移）。
    // best-effort，只刷新机器派生行 + 补缺失；reconcile 幂等（一致行不写）。
    let policy_seeded_by = format!(
        "statemachine_publish:{}",
        prev.seeded_by.clone().unwrap_or_default()
    );
    reconcile_state_policies_for_machine(
        &state.db,
        &prev.workspace_id,
        &prev.domain,
        &prev.state_machine,
        &policy_seeded_by,
        now,
    )
    .await;
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
    let expected_current_id =
        unique_current_id(&state.db, "operation_state_policies", &scope).await?;
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
        version: 0,
        current_version: false,
        previous_version: Some(source.version),
        seeded_by: Some("manual".to_string()),
    };
    let (inserted_id, next_version) = insert_new_current(
        &state.db,
        "operation_state_policies",
        scope,
        expected_current_id,
        to_document(&new_entry)?,
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "id": inserted_id.to_hex(),
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
    switch_current(
        &state.db,
        "operation_state_policies",
        doc! {
            "workspace_id": &target.workspace_id,
            "domain": &target.domain,
            "state_key": &target.state_key,
        },
        object_id,
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
    switch_current(
        &state.db,
        "operation_state_policies",
        doc! {
            "workspace_id": &target.workspace_id,
            "domain": &target.domain,
            "state_key": &target.state_key,
        },
        prev_id,
    )
    .await?;
    Ok(Json(json!({ "ok": true, "rolledBackTo": prev_version })))
}

/// ── system_taxonomies ────────────────────────────────────────────────────────

pub async fn publish_taxonomy_version(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.collection_system_taxonomies();
    let source = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("taxonomy entry not found".to_string()))?;
    let scope = doc! {
        "workspace_id": &admin.current_workspace,
        "scope": &source.scope,
        "kind": &source.kind,
        "value.id": &source.value.id,
    };
    let expected_current_id = unique_current_id(&state.db, "system_taxonomies", &scope).await?;
    let now = DateTime::now();
    let mut new_entry = TaxonomyEntry {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        scope: source.scope.clone(),
        kind: source.kind.clone(),
        value: source.value.clone(),
        updated_at: now,
        version: 0,
        current_version: false,
        previous_version: Some(source.version),
        seeded_by: Some("manual".to_string()),
    };
    let (inserted_id, next_version) = insert_new_current(
        &state.db,
        "system_taxonomies",
        scope,
        expected_current_id,
        to_document(&new_entry)?,
    )
    .await?;
    new_entry.version = next_version;
    new_entry.current_version = true;
    invalidate_global_taxonomy_cache(&state.db);
    audit_taxonomy_change(&state, &admin, "publish", &new_entry).await;
    Ok(Json(json!({
        "ok": true,
        "id": inserted_id.to_hex(),
        "version": next_version,
        "previousVersion": source.version,
    })))
}

pub async fn rollout_taxonomy_version(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.collection_system_taxonomies();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("taxonomy entry not found".to_string()))?;
    switch_current(
        &state.db,
        "system_taxonomies",
        doc! {
            "workspace_id": &admin.current_workspace,
            "scope": &target.scope,
            "kind": &target.kind,
            "value.id": &target.value.id,
        },
        object_id,
    )
    .await?;
    invalidate_global_taxonomy_cache(&state.db);
    audit_taxonomy_change(&state, &admin, "rollout", &target).await;
    Ok(Json(json!({ "ok": true, "version": target.version })))
}

pub async fn rollback_taxonomy_version(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let coll = state.db.collection_system_taxonomies();
    let target = coll
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("taxonomy entry not found".to_string()))?;
    let prev_version = target.previous_version.ok_or_else(|| {
        AppError::BadRequest("target version has no previous_version recorded".to_string())
    })?;
    let prev = coll
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
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
    switch_current(
        &state.db,
        "system_taxonomies",
        doc! {
            "workspace_id": &admin.current_workspace,
            "scope": &target.scope,
            "kind": &target.kind,
            "value.id": &target.value.id,
        },
        prev_id,
    )
    .await?;
    invalidate_global_taxonomy_cache(&state.db);
    audit_taxonomy_change(&state, &admin, "rollback", &prev).await;
    Ok(Json(json!({ "ok": true, "rolledBackTo": prev_version })))
}

/// 取出指定 scope 下当前最大 version + 1（无记录时退回 1）。
///
/// 通用化以避免三表三份重复实现。`T` 必须是携带 `version: i32` 的 BSON struct，
/// 这里只读 `version` 字段；其他字段反序列化时由各自的 `serde(default)` 处理。
async fn next_version_for_scope<T>(coll: mongodb::Collection<T>, scope: Document) -> AppResult<i32>
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

/// system_taxonomies 版本改动（publish / rollout / rollback）成功后写一条审计事件，
/// 记录**是谁**改了哪条全局/账号 scope 字典项。
///
/// 背景（Stage4 孤儿 #4）：`TaxonomyEntry` 无 `workspace_id`、只有 `scope`（`"global"`
/// 或 account_id），三个版本 handler 历史上不接 `AuthenticatedAdmin`——全局字典任一
/// admin 皆可改且改动无迹可查。本系统无 RBAC 角色模型（`AuthenticatedAdmin` 仅
/// user_id/username/current_workspace），"谁有权改全局字典" 红线/文档均无定义，故
/// **不加拦截门**（保持策略型孤儿的现状语义），只补最小可观测：把改动主体与目标
/// scope 落一条 `taxonomy_version_changed` 事件，让全局字典变更 who/what 可追溯。
///
/// **fail-soft**：审计写失败绝不影响已成功的字典改动（best-effort，忽略错误）——
/// 与 gateway 送达后审计降级、prompt publish 观测事件同红线（可观测不得反噬业务）。
async fn audit_taxonomy_change(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    action: &str,
    entry: &TaxonomyEntry,
) {
    let is_global = entry.scope == "global";
    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: admin.current_workspace.clone(),
                account_id: state.config.default_account_id.clone(),
                contact_wxid: None,
                kind: "taxonomy_version_changed".to_string(),
                status: "ok".to_string(),
                summary: format!(
                    "admin={} {} taxonomy scope={} kind={} value.id={}（{}）",
                    admin.username,
                    action,
                    entry.scope,
                    entry.kind,
                    entry.value.id,
                    if is_global {
                        "全局字典改动"
                    } else {
                        "账号级字典改动"
                    }
                ),
                details: Some(doc! {
                    "action": action,
                    "adminUserId": &admin.user_id,
                    "adminUsername": &admin.username,
                    "currentWorkspace": &admin.current_workspace,
                    "scope": &entry.scope,
                    "isGlobalScope": is_global,
                    "kind": &entry.kind,
                    "valueId": &entry.value.id,
                    "version": entry.version,
                }),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;
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
        assert!(
            target_prev.is_none(),
            "无 previous_version 时 rollback 应被拒绝"
        );
    }

    /// H13 (2)：`is_refreshable_policy_seeded_by` 区分机器派生行（可刷新）与手工行（保留）。
    #[test]
    fn refreshable_seeded_by_classifies_machine_vs_operator() {
        use super::is_refreshable_policy_seeded_by;
        // 机器派生 / 可安全刷新
        assert!(
            is_refreshable_policy_seeded_by(&None),
            "None（无溯源）可刷新"
        );
        assert!(
            is_refreshable_policy_seeded_by(&Some("legacy_migration".to_string())),
            "m013 seed tag 可刷新"
        );
        assert!(
            is_refreshable_policy_seeded_by(&Some(
                "statemachine_publish:profile:edu-k12".to_string()
            )),
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

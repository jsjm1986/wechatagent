//! 运营领域配置路由：领域目标、方法论与状态机。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::OperationDomainConfig,
    prompts,
};

use super::AppState;

/// Phase E / E5-T1：list 路径默认只返回 `current_version=true` 的 row，
/// admin 灰度面板传 `?includeAllVersions=true` 拿到完整版本流水以渲染
/// "v3 → v4 各 50%" 的桶分布与回滚链。老库无该字段时 `m015` 已 backfill。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListOperationDomainsQuery {
    #[serde(default)]
    include_all_versions: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationDomainRequest {
    name: String,
    goal: String,
    methodology: String,
    workflow: String,
    tool_policy: String,
    automation_policy: String,
    review_policy: String,
    #[serde(default)]
    runtime_parameters: Document,
    #[serde(default)]
    state_machine: Document,
    #[serde(default)]
    assist_mode_enabled: Option<bool>,
}

pub(super) async fn list_operation_domains(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ListOperationDomainsQuery>,
) -> AppResult<Json<Value>> {
    ensure_operation_domains(&state, &admin.current_workspace).await?;
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    if !query.include_all_versions {
        filter.insert("current_version", doc! { "$ne": false });
    }
    let mut cursor = state
        .db
        .operation_domain_configs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "domain": 1, "version": -1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(config) = cursor.try_next().await? {
        items.push(operation_domain_json(config));
    }
    Ok(Json(json!({ "items": items })))
}

pub async fn get_operation_domain(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(domain): Path<String>,
) -> AppResult<Json<Value>> {
    ensure_operation_domains(&state, &admin.current_workspace).await?;
    let config = find_operation_domain(&state, &admin.current_workspace, &domain).await?;
    Ok(Json(json!({ "item": operation_domain_json(config) })))
}

pub(super) async fn update_operation_domain(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(domain): Path<String>,
    Json(mut payload): Json<OperationDomainRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_domain_input(&payload)?;
    validate_state_machine(&payload.state_machine)?;
    normalize_state_machine_allow_from_any(&mut payload.state_machine);
    ensure_operation_domains(&state, &admin.current_workspace).await?;
    // G06：直编路由改状态机本体后联动重派 policy（否则 forbidsProactive 新增 state 主动触达门
    // fail-open 静默失效）。$set 会 move payload.state_machine，故先 clone 出本体喂 reconcile。
    let state_machine_for_policy = payload.state_machine.clone();
    let mut set_doc = doc! {
        "name": payload.name,
        "goal": payload.goal,
        "methodology": payload.methodology,
        "workflow": payload.workflow,
        "tool_policy": payload.tool_policy,
        "automation_policy": payload.automation_policy,
        "review_policy": payload.review_policy,
        "runtime_parameters": payload.runtime_parameters,
        "state_machine": payload.state_machine,
        "status": "active",
        "updated_at": DateTime::now(),
    };
    // 辅助模式账号级总开关：None 时不写入（保留既有值，避免误覆盖）。
    if let Some(v) = payload.assist_mode_enabled {
        set_doc.insert("assist_mode_enabled", v);
    }
    state
        .db
        .operation_domain_configs()
        .update_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": &domain,
                // Phase E / E5-T1：PATCH 只更新当前生效版本，避免在多版本灰度时
                // 非确定性写到任意 row。`$ne: false` 让 m015 之前未 backfill 的
                // 老 row（无 current_version 字段）继续被命中。
                "current_version": { "$ne": false },
            },
            doc! { "$set": set_doc },
            None,
        )
        .await?;
    // G06：$set 成功后按新本体重派 policy current 行（statemachine_edit: 溯源标，
    // 通过 is_refreshable_policy_seeded_by 判定为机器派生行，下次 publish 可刷新）。
    let policy_seeded_by = format!("statemachine_edit:{}", &domain);
    crate::routes::admin_ops_versions::reconcile_state_policies_for_machine(
        &state.db,
        &admin.current_workspace,
        &domain,
        &state_machine_for_policy,
        &policy_seeded_by,
        DateTime::now(),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn get_operation_domain_state_machine(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(domain): Path<String>,
) -> AppResult<Json<Value>> {
    ensure_operation_domains(&state, &admin.current_workspace).await?;
    let config = find_operation_domain(&state, &admin.current_workspace, &domain).await?;
    Ok(Json(json!({ "item": config.state_machine })))
}

pub async fn update_operation_domain_state_machine(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(domain): Path<String>,
    Json(mut payload): Json<Document>,
) -> AppResult<Json<Value>> {
    ensure_operation_domains(&state, &admin.current_workspace).await?;
    validate_state_machine(&payload)?;
    normalize_state_machine_allow_from_any(&mut payload);
    // G06：$set 会 move payload（payload 本身即 state_machine Document），先 clone 喂 reconcile。
    let state_machine_for_policy = payload.clone();
    state
        .db
        .operation_domain_configs()
        .update_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": &domain,
                "current_version": { "$ne": false },
            },
            doc! {
                "$set": {
                    "state_machine": payload,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    // G06：直编状态机本体后联动重派 policy（否则 forbidsProactive 新增 state 主动触达门
    // fail-open 静默失效）。statemachine_edit: 溯源标可被 is_refreshable_policy_seeded_by 识别。
    let policy_seeded_by = format!("statemachine_edit:{}", &domain);
    crate::routes::admin_ops_versions::reconcile_state_policies_for_machine(
        &state.db,
        &admin.current_workspace,
        &domain,
        &state_machine_for_policy,
        &policy_seeded_by,
        DateTime::now(),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

/// PUT /api/admin/operation-domains/:domain/ask-human-policy
/// $set ask_human_policy 到 current_version 行（不 bump 版本，贴生产 admin 编辑语义）。
pub async fn put_ask_human_policy(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(domain): Path<String>,
    Json(policy): Json<crate::models::AskHumanPolicy>,
) -> AppResult<Json<Value>> {
    // 校验：decider_chain wxid 非空；quiet_hours 小时范围。
    for d in &policy.decider_chain {
        if d.wxid.trim().is_empty() {
            return Err(AppError::BadRequest("decider_chain wxid 不能为空".into()));
        }
    }
    if let Some(qh) = &policy.quiet_hours {
        if qh.start_hour > 23 || qh.end_hour > 23 {
            return Err(AppError::BadRequest("quiet_hours 小时须 0-23".into()));
        }
    }
    let policy_bson = mongodb::bson::to_bson(&policy)?;
    let res = state
        .db
        .operation_domain_configs()
        .update_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": &domain,
                "current_version": true,
            },
            doc! { "$set": { "ask_human_policy": policy_bson, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound("operation domain 当前版本不存在".into()));
    }
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn reset_operation_domain(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(domain): Path<String>,
) -> AppResult<Json<Value>> {
    let Some(default_config) = prompts::default_domain_configs(&admin.current_workspace)
        .into_iter()
        .find(|item| item.domain == domain)
    else {
        return Err(AppError::NotFound("operation domain not found".to_string()));
    };
    state
        .db
        .operation_domain_configs()
        .delete_many(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": &domain
            },
            None,
        )
        .await?;
    state
        .db
        .operation_domain_configs()
        .insert_one(default_config, None)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) fn operation_domain_json(config: OperationDomainConfig) -> Value {
    json!({
        "id": config.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": config.workspace_id,
        "domain": config.domain,
        "name": config.name,
        "goal": config.goal,
        "methodology": config.methodology,
        "workflow": config.workflow,
        "toolPolicy": config.tool_policy,
        "automationPolicy": config.automation_policy,
        "reviewPolicy": config.review_policy,
        "runtimeParameters": config.runtime_parameters,
        "stateMachine": config.state_machine,
        "assistModeEnabled": config.assist_mode_enabled,
        "status": config.status,
        "updatedAt": crate::models::dt_to_string(config.updated_at),
        "version": config.version,
        "currentVersion": config.current_version,
        "previousVersion": config.previous_version,
        "seededBy": config.seeded_by,
        "askHumanPolicy": config.ask_human_policy,
    })
}

pub(super) fn validate_operation_domain_input(payload: &OperationDomainRequest) -> AppResult<()> {
    if payload.name.trim().is_empty()
        || payload.goal.trim().is_empty()
        || payload.methodology.trim().is_empty()
        || payload.workflow.trim().is_empty()
        || payload.tool_policy.trim().is_empty()
        || payload.automation_policy.trim().is_empty()
        || payload.review_policy.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "name, goal, methodology, workflow, toolPolicy, automationPolicy and reviewPolicy are required".to_string(),
        ));
    }
    Ok(())
}

// pub(crate)（非 pub(super)）：H13 引导层 `guide_profile.rs` 复用本校验，对 LLM 生成的
// 候选状态机本体做合法性检查（states 是对象数组 / key 非空且唯一 / allowedFrom 只引已知
// 态）后再落 draft。私有 `mod domains` 下的 pub(crate) 项对 crate 内 sibling 仍可见。
pub(crate) fn validate_state_machine(machine: &Document) -> AppResult<()> {
    let Ok(states) = machine.get_array("states") else {
        return Ok(());
    };
    let mut keys = Vec::new();
    for state in states {
        let Some(doc) = state.as_document() else {
            return Err(AppError::BadRequest(
                "stateMachine.states must contain objects".to_string(),
            ));
        };
        let key = doc
            .get_str("key")
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if key.is_empty() {
            return Err(AppError::BadRequest(
                "stateMachine.states[].key is required".to_string(),
            ));
        }
        if keys.iter().any(|existing| existing == &key) {
            return Err(AppError::BadRequest(format!(
                "duplicate stateMachine state key: {key}"
            )));
        }
        keys.push(key);
    }
    // H13：非空 states 必须至少有一个 initial:true 态。否则运行时新联系人（from 为空）
    // 找不到唯一合法迁入目标 → 每次新接触迁移都被 check_state_transition fail-soft 拒绝，
    // 状态机静默冻结、无报错。满足 spec H13① "缺 initial reject"。空/缺 states 不在此约束
    // （上方 get_array 缺失即 Ok 早返，空数组是退化机交由 publish/runtime 层处理，逐字保持旧行为）。
    if !states.is_empty()
        && !states
            .iter()
            .filter_map(|state| state.as_document())
            .any(|doc| doc.get_bool("initial").unwrap_or(false))
    {
        return Err(AppError::BadRequest(
            "stateMachine must declare at least one initial state (initial:true)".to_string(),
        ));
    }
    for state in states {
        let Some(doc) = state.as_document() else {
            continue;
        };
        let key = doc.get_str("key").unwrap_or_default();
        if let Ok(allowed_from) = doc.get_array("allowedFrom") {
            for item in allowed_from {
                let Some(from) = item
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return Err(AppError::BadRequest(format!(
                        "stateMachine {key}.allowedFrom must contain state keys"
                    )));
                };
                if !keys.iter().any(|existing| existing == from) {
                    return Err(AppError::BadRequest(format!(
                        "stateMachine {key}.allowedFrom references unknown state: {from}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// 波 C1：把 `allowFromAny=true` 的 state 的 `allowedFrom` 归一化为空数组。
pub(super) fn normalize_state_machine_allow_from_any(machine: &mut Document) {
    let Ok(states) = machine.get_array_mut("states") else {
        return;
    };
    for item in states.iter_mut() {
        let Some(state) = item.as_document_mut() else {
            continue;
        };
        if state.get_bool("allowFromAny").unwrap_or(false) {
            state.insert("allowedFrom", mongodb::bson::Bson::Array(Vec::new()));
        }
    }
}

pub(super) async fn ensure_operation_domains(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<()> {
    for config in prompts::default_domain_configs(workspace_id) {
        let existing = state
            .db
            .operation_domain_configs()
            .find_one(
                doc! {
                    "workspace_id": workspace_id,
                    "domain": &config.domain
                },
                None,
            )
            .await?;
        if let Some(existing) = existing {
            if existing.domain == "user_operations" && existing.state_machine.is_empty() {
                state
                    .db
                    .operation_domain_configs()
                    .update_one(
                        doc! {
                            "workspace_id": workspace_id,
                            "domain": "user_operations"
                        },
                        doc! {
                            "$set": {
                                "state_machine": prompts::default_user_operation_state_machine(),
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await?;
            }
        } else {
            state
                .db
                .operation_domain_configs()
                .insert_one(config, None)
                .await?;
        }
    }
    Ok(())
}

pub(super) async fn find_operation_domain(
    state: &AppState,
    workspace_id: &str,
    domain: &str,
) -> AppResult<OperationDomainConfig> {
    let coll = state.db.operation_domain_configs();
    if let Some(active) = coll
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": domain,
                "current_version": true,
            },
            None,
        )
        .await?
    {
        return Ok(active);
    }
    coll.find_one(
        doc! {
            "workspace_id": workspace_id,
            "domain": domain,
            "current_version": { "$exists": false },
        },
        None,
    )
    .await?
    .ok_or_else(|| AppError::NotFound("operation domain not found".to_string()))
}


#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{doc, Bson};

    #[test]
    fn normalize_clears_allowed_from_when_allow_from_any() {
        let mut machine = doc! {
            "states": [
                {
                    "key": "cooldown",
                    "allowFromAny": true,
                    "allowedFrom": ["foo", "bar"]
                },
                {
                    "key": "new_contact",
                    "allowedFrom": ["new_contact"]
                }
            ]
        };
        normalize_state_machine_allow_from_any(&mut machine);
        let states = machine.get_array("states").unwrap();
        let cooldown = states[0].as_document().unwrap();
        let cooldown_allowed = cooldown.get_array("allowedFrom").unwrap();
        assert!(
            cooldown_allowed.is_empty(),
            "allowFromAny=true 时 allowedFrom 应为空，实际：{:?}",
            cooldown_allowed
        );
        let new_contact = states[1].as_document().unwrap();
        let new_contact_allowed = new_contact.get_array("allowedFrom").unwrap();
        assert_eq!(
            new_contact_allowed
                .iter()
                .filter_map(Bson::as_str)
                .collect::<Vec<_>>(),
            vec!["new_contact"],
            "allowFromAny=false 时不动 allowedFrom"
        );
    }

    #[test]
    fn normalize_keeps_allowed_from_when_allow_from_any_missing() {
        let mut machine = doc! {
            "states": [
                { "key": "need_discovery", "allowedFrom": ["new_contact"] }
            ]
        };
        normalize_state_machine_allow_from_any(&mut machine);
        let arr = machine.get_array("states").unwrap()[0]
            .as_document()
            .unwrap()
            .get_array("allowedFrom")
            .unwrap();
        assert_eq!(
            arr.iter().filter_map(Bson::as_str).collect::<Vec<_>>(),
            vec!["new_contact"]
        );
    }

    #[test]
    fn validate_state_machine_rejects_duplicate_keys() {
        let machine = doc! {
            "states": [
                { "key": "alpha", "allowedFrom": [] },
                { "key": "alpha", "allowedFrom": [] }
            ]
        };
        let err = validate_state_machine(&machine).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_state_machine_rejects_unknown_allowed_from() {
        let machine = doc! {
            "states": [
                { "key": "alpha", "allowedFrom": ["beta"] }
            ]
        };
        let err = validate_state_machine(&machine).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    // H13：非空 states 缺 initial:true → reject（spec H13① "缺 initial reject"）。
    #[test]
    fn validate_rejects_state_machine_without_initial() {
        let machine = doc! {
            "states": [
                { "key": "alpha", "allowedFrom": ["alpha"] },
                { "key": "beta", "allowedFrom": ["alpha", "beta"] }
            ]
        };
        let err = validate_state_machine(&machine).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    // H13：同样结构但其中一态标 initial:true → 过校验。
    #[test]
    fn validate_accepts_state_machine_with_initial() {
        let machine = doc! {
            "states": [
                { "key": "alpha", "initial": true, "allowedFrom": ["alpha"] },
                { "key": "beta", "allowedFrom": ["alpha", "beta"] }
            ]
        };
        assert!(validate_state_machine(&machine).is_ok());
    }

    // H13 字节等价红线：DEFAULT 销售状态机（new_contact initial:true）必须仍过校验。
    #[test]
    fn validate_accepts_default_sales_machine() {
        assert!(
            validate_state_machine(&crate::prompts::default_user_operation_state_machine()).is_ok()
        );
    }
}

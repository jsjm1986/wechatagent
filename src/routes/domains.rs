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

pub(super) async fn get_operation_domain(
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
            doc! {
                "$set": {
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
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
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

pub(super) async fn update_operation_domain_state_machine(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(domain): Path<String>,
    Json(mut payload): Json<Document>,
) -> AppResult<Json<Value>> {
    ensure_operation_domains(&state, &admin.current_workspace).await?;
    validate_state_machine(&payload)?;
    normalize_state_machine_allow_from_any(&mut payload);
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
        "status": config.status,
        "updatedAt": crate::models::dt_to_string(config.updated_at),
        "version": config.version,
        "currentVersion": config.current_version,
        "previousVersion": config.previous_version,
        "seededBy": config.seeded_by,
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

pub(super) fn validate_state_machine(machine: &Document) -> AppResult<()> {
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
}

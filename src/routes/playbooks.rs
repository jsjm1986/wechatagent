//! 运营 Playbook 路由：方法论模板的增删改查及自动生成。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime},
    options::{FindOneOptions, FindOptions, TransactionOptions},
    ClientSession,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::OperationPlaybook,
    prompts,
};

use super::shared::*;
use super::AppState;

const PLAYBOOK_DEFAULT_CONFLICT: &str = "playbook_default_conflict";

async fn commit_playbook_transaction(session: &mut ClientSession) -> AppResult<()> {
    loop {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => {
                let _ = session.abort_transaction().await;
                tracing::warn!(error = %error, "playbook transaction commit failed");
                return Err(AppError::Conflict(PLAYBOOK_DEFAULT_CONFLICT.to_string()));
            }
        }
    }
}

fn playbook_transaction_error(error: AppError) -> AppError {
    match error {
        AppError::Db(db_error) => {
            tracing::warn!(error = %db_error, "playbook transaction conflicted");
            AppError::Conflict(PLAYBOOK_DEFAULT_CONFLICT.to_string())
        }
        other => other,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationPlaybookQuery {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationPlaybookRequest {
    pub account_id: Option<String>,
    pub expected_version: Option<i32>,
    pub name: String,
    pub description: Option<String>,
    pub method_prompt: String,
    pub profile_method: Option<String>,
    pub tag_method: Option<String>,
    pub stage_method: Option<String>,
    pub intent_method: Option<String>,
    pub follow_up_method: Option<String>,
    pub reply_style: Option<String>,
    pub forbidden_rules: Option<String>,
    pub success_criteria: Option<String>,
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeneratePlaybookRequest {
    account_id: String,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OptimizePlaybookRequest {
    account_id: String,
    expected_version: i32,
    instruction: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookMutationIdentity {
    pub account_id: String,
    pub expected_version: i32,
}

pub(super) async fn list_operation_playbooks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<OperationPlaybookQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    ensure_default_playbook(&state, &admin.current_workspace, &account_id).await?;
    let mut cursor = state
        .db
        .operation_playbooks()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id
            },
            FindOptions::builder()
                .sort(doc! { "is_default": -1, "updated_at": -1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(playbook) = cursor.try_next().await? {
        items.push(playbook_json(playbook));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_operation_playbook(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<OperationPlaybookRequest>,
) -> AppResult<Json<Value>> {
    let account_id = payload
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    validate_playbook_input(&payload.name, &payload.method_prompt)?;
    let make_default = payload.is_default.unwrap_or(false)
        || state
            .db
            .operation_playbooks()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &account_id,
                    "release_status": "published",
                    "is_default": true,
                },
                None,
            )
            .await?
            .is_none();
    let mut playbook = OperationPlaybook {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id,
        name: payload.name,
        description: normalize_optional(payload.description),
        method_prompt: payload.method_prompt,
        profile_method: normalize_optional(payload.profile_method),
        tag_method: normalize_optional(payload.tag_method),
        stage_method: normalize_optional(payload.stage_method),
        intent_method: normalize_optional(payload.intent_method),
        follow_up_method: normalize_optional(payload.follow_up_method),
        reply_style: normalize_optional(payload.reply_style),
        forbidden_rules: normalize_optional(payload.forbidden_rules),
        success_criteria: normalize_optional(payload.success_criteria),
        created_by: "manual".to_string(),
        release_status: "published".to_string(),
        // Default ownership is assigned only by the transaction below. A
        // failed insert can therefore never leave the account with no default.
        is_default: false,
        version: 1,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    playbook = insert_playbook(&state, playbook, make_default).await?;
    Ok(Json(json!({
        "id": playbook.id.map(|id| id.to_hex()),
        "item": playbook_json(playbook),
    })))
}

pub async fn update_operation_playbook(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OperationPlaybookRequest>,
) -> AppResult<Json<Value>> {
    validate_playbook_input(&payload.name, &payload.method_prompt)?;
    let account_id = payload
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::BadRequest("accountId is required".to_string()))?
        .to_string();
    let expected_version = payload
        .expected_version
        .filter(|version| *version > 0)
        .ok_or_else(|| AppError::BadRequest("positive expectedVersion is required".to_string()))?;
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    let object_id = parse_object_id(&id)?;
    let existing = state
        .db
        .operation_playbooks()
        .find_one(
            playbook_mutation_filter(
                object_id,
                &admin.current_workspace,
                &account_id,
                expected_version,
            ),
            None,
        )
        .await?
        .ok_or_else(|| AppError::Conflict("playbook_identity_or_version_conflict".to_string()))?;
    if payload
        .is_default
        .is_some_and(|requested| requested != existing.is_default)
    {
        return Err(AppError::BadRequest(
            "isDefault is managed by the dedicated set-default endpoint".to_string(),
        ));
    }
    let updated = state
        .db
        .operation_playbooks()
        .update_one(
            playbook_mutation_filter(
                object_id,
                &admin.current_workspace,
                &account_id,
                expected_version,
            ),
            doc! {
                "$set": {
                    "name": payload.name,
                    "description": normalize_optional(payload.description),
                    "method_prompt": payload.method_prompt,
                    "profile_method": normalize_optional(payload.profile_method),
                    "tag_method": normalize_optional(payload.tag_method),
                    "stage_method": normalize_optional(payload.stage_method),
                    "intent_method": normalize_optional(payload.intent_method),
                    "follow_up_method": normalize_optional(payload.follow_up_method),
                    "reply_style": normalize_optional(payload.reply_style),
                    "forbidden_rules": normalize_optional(payload.forbidden_rules),
                    "success_criteria": normalize_optional(payload.success_criteria),
                    "release_status": "published",
                    "is_default": existing.is_default,
                    "version": expected_version + 1,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    if updated.matched_count != 1 {
        return Err(AppError::Conflict(
            "playbook_identity_or_version_conflict".to_string(),
        ));
    }
    Ok(Json(json!({ "ok": true, "version": expected_version + 1 })))
}

pub async fn set_default_operation_playbook(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<PlaybookMutationIdentity>,
) -> AppResult<Json<Value>> {
    let account_id = payload.account_id.trim();
    if account_id.is_empty() || payload.expected_version < 1 {
        return Err(AppError::BadRequest(
            "accountId and positive expectedVersion are required".to_string(),
        ));
    }
    validate_account(&state, &admin.current_workspace, account_id).await?;
    let object_id = parse_object_id(&id)?;
    let playbook = switch_default_playbook(
        &state,
        &admin.current_workspace,
        account_id,
        object_id,
        payload.expected_version,
    )
    .await?;
    Ok(Json(json!({ "ok": true, "version": playbook.version })))
}

pub(super) async fn generate_operation_playbook(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<GeneratePlaybookRequest>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &admin.current_workspace, &payload.account_id).await?;
    if payload.description.trim().is_empty() {
        return Err(AppError::BadRequest("description is required".to_string()));
    }
    let system = prompts::load_prompt(
        &state.db,
        &admin.current_workspace,
        "playbook.generator.system",
    )
    .await?;
    // C3：active profile 可声明行业专属生成器引导语,覆盖领域中性 DEFAULT(去销售偏见)。
    let active_profile =
        agent::domain_profile::load_active_domain_profile(&state.db, &admin.current_workspace)
            .await?;
    let system = match active_profile
        .methodology_generator_preamble
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(p) => p.to_string(),
        None => system,
    };
    let user = build_playbook_generation_prompt(&payload.description);
    let generated = agent::generate_agent_json(
        &state,
        &admin.current_workspace,
        Some(&payload.account_id),
        None,
        None,
        "playbook.generator",
        &system,
        &user,
    )
    .await?;
    // AI generation only creates a reviewable candidate. It never acquires the
    // production default pointer, including on a previously empty account.
    let mut playbook = OperationPlaybook {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: payload.account_id,
        name: json_string_any(&generated, &["name"])
            .unwrap_or_else(|| "AI 生成运营方法".to_string()),
        description: json_string_any(&generated, &["description"]),
        method_prompt: json_string_any(&generated, &["methodPrompt", "method_prompt"])
            .unwrap_or_else(|| payload.description.clone()),
        profile_method: json_string_any(&generated, &["profileMethod", "profile_method"]),
        tag_method: json_string_any(&generated, &["tagMethod", "tag_method"]),
        stage_method: json_string_any(&generated, &["stageMethod", "stage_method"]),
        intent_method: json_string_any(&generated, &["intentMethod", "intent_method"]),
        follow_up_method: json_string_any(&generated, &["followUpMethod", "follow_up_method"]),
        reply_style: json_string_any(&generated, &["replyStyle", "reply_style"]),
        forbidden_rules: json_string_any(&generated, &["forbiddenRules", "forbidden_rules"]),
        success_criteria: json_string_any(&generated, &["successCriteria", "success_criteria"]),
        created_by: "agent".to_string(),
        release_status: "draft".to_string(),
        is_default: false,
        version: 1,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state
        .db
        .operation_playbooks()
        .insert_one(&playbook, None)
        .await?;
    playbook.id = result.inserted_id.as_object_id();
    Ok(Json(json!({
        "id": playbook.id.map(|id| id.to_hex()),
        "item": playbook_json(playbook),
    })))
}

pub(super) async fn optimize_operation_playbook(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OptimizePlaybookRequest>,
) -> AppResult<Json<Value>> {
    if payload.instruction.trim().is_empty() {
        return Err(AppError::BadRequest("instruction is required".to_string()));
    }
    let account_id = payload.account_id.trim();
    if account_id.is_empty() || payload.expected_version < 1 {
        return Err(AppError::BadRequest(
            "accountId and positive expectedVersion are required".to_string(),
        ));
    }
    validate_account(&state, &admin.current_workspace, account_id).await?;
    let object_id = parse_object_id(&id)?;
    let existing = state
        .db
        .operation_playbooks()
        .find_one(
            playbook_mutation_filter(
                object_id,
                &admin.current_workspace,
                account_id,
                payload.expected_version,
            ),
            None,
        )
        .await?
        .ok_or_else(|| AppError::Conflict("playbook_identity_or_version_conflict".to_string()))?;
    let system = prompts::load_prompt(
        &state.db,
        &admin.current_workspace,
        "playbook.generator.system",
    )
    .await?;
    // C3：active profile 可声明行业专属生成器引导语,覆盖领域中性 DEFAULT(去销售偏见)。
    let active_profile =
        agent::domain_profile::load_active_domain_profile(&state.db, &admin.current_workspace)
            .await?;
    let system = match active_profile
        .methodology_generator_preamble
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(p) => p.to_string(),
        None => system,
    };
    let user = build_playbook_optimization_prompt(&existing, &payload.instruction);
    let generated = agent::generate_agent_json(
        &state,
        &admin.current_workspace,
        Some(account_id),
        None,
        None,
        "playbook.optimizer",
        &system,
        &user,
    )
    .await?;
    let name = json_string_any(&generated, &["name"]).unwrap_or(existing.name);
    let description = json_string_any(&generated, &["description"]).or(existing.description);
    let method_prompt = json_string_any(&generated, &["methodPrompt", "method_prompt"])
        .unwrap_or(existing.method_prompt);
    validate_playbook_input(&name, &method_prompt)?;
    let profile_method = json_string_any(&generated, &["profileMethod", "profile_method"])
        .or(existing.profile_method);
    let tag_method =
        json_string_any(&generated, &["tagMethod", "tag_method"]).or(existing.tag_method);
    let stage_method =
        json_string_any(&generated, &["stageMethod", "stage_method"]).or(existing.stage_method);
    let intent_method =
        json_string_any(&generated, &["intentMethod", "intent_method"]).or(existing.intent_method);
    let follow_up_method = json_string_any(&generated, &["followUpMethod", "follow_up_method"])
        .or(existing.follow_up_method);
    let reply_style =
        json_string_any(&generated, &["replyStyle", "reply_style"]).or(existing.reply_style);
    let forbidden_rules = json_string_any(&generated, &["forbiddenRules", "forbidden_rules"])
        .or(existing.forbidden_rules);
    let success_criteria = json_string_any(&generated, &["successCriteria", "success_criteria"])
        .or(existing.success_criteria);
    let version = payload
        .expected_version
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("playbook_version_exhausted".to_string()))?;
    let now = DateTime::now();
    let mut candidate = OperationPlaybook {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: account_id.to_string(),
        name,
        description,
        method_prompt,
        profile_method,
        tag_method,
        stage_method,
        intent_method,
        follow_up_method,
        reply_style,
        forbidden_rules,
        success_criteria,
        created_by: "agent_optimized".to_string(),
        release_status: "draft".to_string(),
        is_default: false,
        version,
        created_at: now,
        updated_at: now,
    };
    let inserted = state
        .db
        .operation_playbooks()
        .insert_one(&candidate, None)
        .await?;
    candidate.id = inserted.inserted_id.as_object_id();
    Ok(Json(json!({
        "sourceId": object_id.to_hex(),
        "item": playbook_json(candidate),
    })))
}

pub(super) fn playbook_json(playbook: OperationPlaybook) -> Value {
    json!({
        "id": playbook.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": playbook.workspace_id,
        "accountId": playbook.account_id,
        "name": playbook.name,
        "description": playbook.description,
        "methodPrompt": playbook.method_prompt,
        "profileMethod": playbook.profile_method,
        "tagMethod": playbook.tag_method,
        "stageMethod": playbook.stage_method,
        "intentMethod": playbook.intent_method,
        "followUpMethod": playbook.follow_up_method,
        "replyStyle": playbook.reply_style,
        "forbiddenRules": playbook.forbidden_rules,
        "successCriteria": playbook.success_criteria,
        "createdBy": playbook.created_by,
        "releaseStatus": playbook.release_status,
        "isDefault": playbook.is_default,
        "version": playbook.version,
        "updatedAt": crate::models::dt_to_string(playbook.updated_at)
    })
}

pub(super) fn validate_playbook_input(name: &str, method_prompt: &str) -> AppResult<()> {
    if name.trim().is_empty() || method_prompt.trim().is_empty() {
        return Err(AppError::BadRequest(
            "name and methodPrompt are required".to_string(),
        ));
    }
    Ok(())
}

fn playbook_mutation_filter(
    object_id: ObjectId,
    workspace_id: &str,
    account_id: &str,
    expected_version: i32,
) -> mongodb::bson::Document {
    doc! {
        "_id": object_id,
        "workspace_id": workspace_id,
        "account_id": account_id,
        "version": expected_version,
    }
}

async fn insert_playbook(
    state: &AppState,
    mut playbook: OperationPlaybook,
    make_default: bool,
) -> AppResult<OperationPlaybook> {
    if !make_default {
        let inserted = state
            .db
            .operation_playbooks()
            .insert_one(&playbook, None)
            .await?;
        playbook.id = inserted.inserted_id.as_object_id();
        return Ok(playbook);
    }

    playbook.release_status = "published".to_string();
    playbook.is_default = true;
    if find_default_playbook(state, &playbook.workspace_id, &playbook.account_id)
        .await?
        .is_none()
    {
        return match state
            .db
            .operation_playbooks()
            .insert_one(&playbook, None)
            .await
        {
            Ok(inserted) => {
                playbook.id = inserted.inserted_id.as_object_id();
                Ok(playbook)
            }
            Err(error) if is_duplicate_key_error(&error) => {
                Err(AppError::Conflict(PLAYBOOK_DEFAULT_CONFLICT.to_string()))
            }
            Err(error) => Err(error.into()),
        };
    }

    // Replacing an existing default is the only path that needs a transaction:
    // demotion and promotion must become visible atomically.
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let result: AppResult<OperationPlaybook> = async {
        let mut cursor = state
            .db
            .operation_playbooks()
            .find_with_session(
                doc! {
                    "workspace_id": &playbook.workspace_id,
                    "account_id": &playbook.account_id,
                    "release_status": "published",
                    "is_default": true,
                },
                FindOptions::builder().limit(2).build(),
                &mut session,
            )
            .await?;
        let current = cursor.next(&mut session).await.transpose()?;
        if cursor.next(&mut session).await.transpose()?.is_some() {
            return Err(AppError::Conflict("multiple_default_playbooks".to_string()));
        }
        drop(cursor);
        if let Some(current) = current {
            let current_id = current
                .id
                .ok_or_else(|| AppError::External("default playbook missing _id".to_string()))?;
            let demoted = state
                .db
                .operation_playbooks()
                .update_one_with_session(
                    doc! {
                        "_id": current_id,
                        "workspace_id": &playbook.workspace_id,
                        "account_id": &playbook.account_id,
                        "is_default": true,
                    },
                    doc! { "$set": { "is_default": false, "updated_at": DateTime::now() } },
                    None,
                    &mut session,
                )
                .await?;
            if demoted.modified_count != 1 {
                return Err(AppError::Conflict(PLAYBOOK_DEFAULT_CONFLICT.to_string()));
            }
        }
        let inserted = state
            .db
            .operation_playbooks()
            .insert_one_with_session(&playbook, None, &mut session)
            .await?;
        playbook.id = inserted.inserted_id.as_object_id();
        Ok(playbook)
    }
    .await;
    let playbook = match result {
        Ok(playbook) => playbook,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(playbook_transaction_error(error));
        }
    };
    commit_playbook_transaction(&mut session).await?;
    Ok(playbook)
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};

    matches!(
        error.kind.as_ref(),
        ErrorKind::Write(WriteFailure::WriteError(write_error)) if write_error.code == 11000
    )
}

async fn switch_default_playbook(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    target_id: ObjectId,
    expected_version: i32,
) -> AppResult<OperationPlaybook> {
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let result: AppResult<OperationPlaybook> = async {
        let mut target = state
            .db
            .operation_playbooks()
            .find_one_with_session(
                playbook_mutation_filter(target_id, workspace_id, account_id, expected_version),
                None,
                &mut session,
            )
            .await?
            .ok_or_else(|| {
                AppError::Conflict("playbook_identity_or_version_conflict".to_string())
            })?;
        if !matches!(target.release_status.as_str(), "draft" | "published") {
            return Err(AppError::Conflict(
                "playbook_release_status_invalid".to_string(),
            ));
        }
        let mut cursor = state
            .db
            .operation_playbooks()
            .find_with_session(
                doc! {
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "release_status": "published",
                    "is_default": true,
                },
                FindOptions::builder().limit(2).build(),
                &mut session,
            )
            .await?;
        let current = cursor.next(&mut session).await.transpose()?;
        if cursor.next(&mut session).await.transpose()?.is_some() {
            return Err(AppError::Conflict("multiple_default_playbooks".to_string()));
        }
        drop(cursor);
        if current.as_ref().and_then(|row| row.id) == Some(target_id) {
            return Ok(target);
        }
        if let Some(current) = current {
            let current_id = current
                .id
                .ok_or_else(|| AppError::External("default playbook missing _id".to_string()))?;
            let demoted = state
                .db
                .operation_playbooks()
                .update_one_with_session(
                    doc! {
                        "_id": current_id,
                        "workspace_id": workspace_id,
                        "account_id": account_id,
                        "is_default": true,
                    },
                    doc! { "$set": { "is_default": false, "updated_at": DateTime::now() } },
                    None,
                    &mut session,
                )
                .await?;
            if demoted.modified_count != 1 {
                return Err(AppError::Conflict(PLAYBOOK_DEFAULT_CONFLICT.to_string()));
            }
        }
        let now = DateTime::now();
        let promoted = state
            .db
            .operation_playbooks()
            .update_one_with_session(
                doc! {
                    "_id": target_id,
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "version": expected_version,
                    "is_default": false,
                },
                doc! {
                    "$set": {
                        "release_status": "published",
                        "is_default": true,
                        "updated_at": now,
                    }
                },
                None,
                &mut session,
            )
            .await?;
        if promoted.modified_count != 1 {
            return Err(AppError::Conflict(PLAYBOOK_DEFAULT_CONFLICT.to_string()));
        }
        target.is_default = true;
        target.release_status = "published".to_string();
        target.updated_at = now;
        Ok(target)
    }
    .await;
    let target = match result {
        Ok(target) => target,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(playbook_transaction_error(error));
        }
    };
    commit_playbook_transaction(&mut session).await?;
    Ok(target)
}

pub(super) async fn ensure_default_playbook(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<OperationPlaybook> {
    if let Some(playbook) = find_default_playbook(state, workspace_id, account_id).await? {
        return Ok(playbook);
    }
    let playbook = prompts::default_playbook(workspace_id, account_id);
    match insert_playbook(state, playbook, true).await {
        Ok(playbook) => Ok(playbook),
        Err(AppError::Conflict(_)) => {
            // Two first reads may race on an empty account. The partial unique
            // index chooses one committed default; the loser converges by
            // reading that winner instead of failing the list request. The
            // winner may still be finishing its commit, so use a short bounded
            // convergence window rather than an unbounded retry loop.
            for _ in 0..4 {
                if let Some(playbook) =
                    find_default_playbook(state, workspace_id, account_id).await?
                {
                    return Ok(playbook);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(AppError::Conflict(PLAYBOOK_DEFAULT_CONFLICT.to_string()))
        }
        Err(error) => Err(error),
    }
}

async fn find_default_playbook(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Option<OperationPlaybook>> {
    state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "release_status": "published",
                "is_default": true,
            },
            FindOneOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .build(),
        )
        .await
        .map_err(AppError::from)
}

pub(super) fn build_playbook_generation_prompt(description: &str) -> String {
    format!(
        r#"请根据业务描述生成一套账号级微信运营方法论。输出字段必须完整：
{{
  "name": "运营方法名称",
  "description": "一句话说明这套方法适合什么业务和人群",
  "methodPrompt": "方法论总纲：说明 Agent 如何长期理解用户、建立信任、提供情绪价值并自然推进业务。必须包含这些公式的中文解释：信任 = 专业可信 + 稳定可靠 + 亲近感 - 自我推销感；成交准备度 = 动机 × 产品匹配 × 时机 × 信任 ÷ 阻力；下一步动作评分 = 关系增益 + 转化进展 + 情绪价值 + 产品匹配 - 压迫感 - 事实风险。",
  "profileMethod": "如何理解用户：用业务用户能懂的语言说明要观察哪些信息、如何从聊天中更新画像、哪些信息未知时不能乱猜。",
  "tagMethod": "用户识别规则：说明标签如何生成、合并、删除，标签必须来自真实行为或明确表达，不能凭感觉贴标签。",
  "stageMethod": "关系阶段判断：说明从陌生、初步信任、明确需求、方案评估、成交推进、老客户维护等阶段如何判断和迁移。",
  "intentMethod": "意向和时机判断：说明高/中/低意向的可观察信号，以及什么时候推进、什么时候降频、什么时候只提供帮助。",
  "followUpMethod": "跟进节奏和下一步动作：说明不同阶段、不同温度、不同沉默时间下应该怎么做，必须有低压、自然、像真人的动作。",
  "replyStyle": "微信表达风格：用业务人员能直接理解的表达规范说明语气、长度、称呼、共情、提问、推进方式。",
  "forbiddenRules": "禁止行为：列出不能做的事情，包括虚假承诺、过度催促、连续追问、强行成交、编造产品能力、忽视用户情绪等。",
  "successCriteria": "复盘和优化标准：说明每次运营如何判断好坏，包含信任、情绪价值、产品准确性、自然度、推进有效性、风险等评分口径。"
}}

写法要求：
- 每个字段都要让前端用户读起来像“运营制度/方法论”，不是机器提示词。
- 不要写空泛口号，要给可观察信号和可执行动作。
- 保持克制、专业、长期主义。

业务描述：
{}"#,
        description
    )
}

pub(super) fn build_playbook_optimization_prompt(
    playbook: &OperationPlaybook,
    instruction: &str,
) -> String {
    format!(
        r#"请根据优化要求，重写并升级当前微信运营方法论。输出字段必须完整，字段名保持不变：
{{
  "name": "运营方法名称",
  "description": "一句话说明这套方法适合什么业务和人群",
  "methodPrompt": "方法论总纲",
  "profileMethod": "如何理解用户",
  "tagMethod": "用户识别规则",
  "stageMethod": "关系阶段判断",
  "intentMethod": "意向和时机判断",
  "followUpMethod": "跟进节奏和下一步动作",
  "replyStyle": "微信表达风格",
  "forbiddenRules": "禁止行为",
  "successCriteria": "复盘和优化标准"
}}

优化要求：
{}

当前方法：
名称：{}
描述：{}
方法论总纲：{}
如何理解用户：{}
用户识别规则：{}
关系阶段判断：{}
意向和时机判断：{}
跟进节奏和下一步动作：{}
微信表达风格：{}
禁止行为：{}
复盘和优化标准：{}

升级原则：
- 让方法更适合业务用户阅读和修改，避免工程提示词腔。
- 补强消费心理学、用户研究、长期关系运营和顾问式成交。
- 每条规则尽量写成“观察到什么 -> 如何判断 -> 采取什么动作 -> 避免什么风险”。
- 保持真实、克制、有人味，不要让用户感觉被机器人营销。"#,
        instruction,
        playbook.name,
        playbook.description.as_deref().unwrap_or(""),
        playbook.method_prompt,
        playbook.profile_method.as_deref().unwrap_or(""),
        playbook.tag_method.as_deref().unwrap_or(""),
        playbook.stage_method.as_deref().unwrap_or(""),
        playbook.intent_method.as_deref().unwrap_or(""),
        playbook.follow_up_method.as_deref().unwrap_or(""),
        playbook.reply_style.as_deref().unwrap_or(""),
        playbook.forbidden_rules.as_deref().unwrap_or(""),
        playbook.success_criteria.as_deref().unwrap_or("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约快照:playbook_json。OperationPlaybook 19 字段全量构造(7 个 Option<String> 给 Some);
    /// id→Option.map(to_hex).unwrap_or_default();updatedAt→dt_to_string。投影下发 18 顶层键(漏发 createdAt)。
    #[test]
    fn playbook_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let playbook = OperationPlaybook {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            name: "默认销售剧本".to_string(),
            description: Some("用于高意向客户跟进".to_string()),
            method_prompt: "先共情再给方案".to_string(),
            profile_method: Some("三段式画像".to_string()),
            tag_method: Some("意向分级打标".to_string()),
            stage_method: Some("AIDA 阶段推进".to_string()),
            intent_method: Some("显式信号优先".to_string()),
            follow_up_method: Some("三天未回主动跟进".to_string()),
            reply_style: Some("简洁口语".to_string()),
            forbidden_rules: Some("不承诺无依据效果".to_string()),
            success_criteria: Some("客户主动询价".to_string()),
            created_by: "admin-1".to_string(),
            release_status: "published".to_string(),
            is_default: true,
            version: 3,
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = playbook_json(playbook);
        crate::routes::contract_snapshot::assert_contract_fixture("playbook", value);
    }
}

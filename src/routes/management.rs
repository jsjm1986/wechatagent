//! 管理 Agent 路由：管理对话 session、计划生成与工具执行。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, to_bson, to_document, DateTime, Document},
    options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{
    agent,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    mcp::{self},
    models::{
        AgentCommandRun, AgentToolCall, ApiContact, Contact, ManagementAgentMessage,
        ManagementAgentSession,
    },
    prompts,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateSessionRequest {
    account_id: String,
    title: Option<String>,
    /// S-20 / Task 19：创建 session 时的默认 dry-run 模式。
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManagementMessageRequest {
    account_id: String,
    content: String,
    /// S-20 / Task 19：单条消息级别的 dry-run 覆盖；缺省时取 session 默认值。
    #[serde(default)]
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfirmManagementCommandRequest {
    account_id: String,
    plan_hash: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManagementPlan {
    #[serde(default)]
    intent: String,
    #[serde(default)]
    risk_level: String,
    #[serde(default)]
    requires_confirmation: bool,
    #[serde(default)]
    missing_information: Vec<String>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    tool_calls: Vec<PlannedToolCall>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlannedToolCall {
    tool_name: String,
    #[serde(default)]
    arguments: Value,
}

/// 一组 plan tool_calls 执行后的产物，供调用方拼装响应/汇报。
/// `calls`：每个工具调用的前端 JSON；`outcomes`：喂 build_execution_summary；
/// `failed`：业务 Failed 或 RPC Err 时的失败原因（设置即"失败即止"）。
pub(super) struct PlanExecution {
    pub calls: Vec<Value>,
    pub outcomes: Vec<(String, ToolOutcome)>,
    pub failed: Option<String>,
    pub execution_unknown: bool,
}

const MANAGEMENT_EXECUTION_LEASE_MILLIS: i64 = 5 * 60 * 1000;
const MANAGEMENT_EXECUTION_HEARTBEAT_SECONDS: u64 = 60;
const MAX_MANAGEMENT_TOOL_CALLS: usize = 12;

#[derive(Debug, Clone)]
struct ManagementExecutionLease {
    run_id: mongodb::bson::oid::ObjectId,
    workspace_id: String,
    account_id: String,
    token: String,
}

impl ManagementExecutionLease {
    fn owner_filter(&self) -> Document {
        doc! {
            "_id": self.run_id,
            "workspace_id": &self.workspace_id,
            "account_id": &self.account_id,
            "status": "running",
            "execution_token": &self.token,
        }
    }
}

async fn renew_management_execution_lease(
    state: &AppState,
    lease: &ManagementExecutionLease,
) -> mongodb::error::Result<bool> {
    let now = DateTime::now();
    let result = state
        .db
        .command_runs()
        .update_one(
            lease.owner_filter(),
            doc! { "$set": { "execution_started_at": now, "updated_at": now } },
            None,
        )
        .await?;
    Ok(result.matched_count == 1)
}

fn spawn_management_execution_heartbeat(
    state: AppState,
    lease: ManagementExecutionLease,
    cancelled: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(Duration::from_secs(MANAGEMENT_EXECUTION_HEARTBEAT_SECONDS));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match renew_management_execution_lease(&state, &lease).await {
                Ok(true) => {}
                Ok(false) => {
                    cancelled.store(true, Ordering::SeqCst);
                    return;
                }
                Err(error) => {
                    cancelled.store(true, Ordering::SeqCst);
                    tracing::warn!(
                        command_run_id = %lease.run_id.to_hex(),
                        error = %error,
                        "management execution heartbeat failed; ownership is unproven"
                    );
                    return;
                }
            }
        }
    })
}

async fn ensure_management_execution_owned(
    state: &AppState,
    lease: &ManagementExecutionLease,
    cancelled: &AtomicBool,
) -> AppResult<()> {
    if cancelled.load(Ordering::SeqCst) {
        return Err(AppError::Conflict(
            "management_command_lease_lost".to_string(),
        ));
    }
    match renew_management_execution_lease(state, lease).await {
        Ok(true) => Ok(()),
        Ok(false) => {
            cancelled.store(true, Ordering::SeqCst);
            Err(AppError::Conflict(
                "management_command_lease_lost".to_string(),
            ))
        }
        Err(error) => {
            cancelled.store(true, Ordering::SeqCst);
            Err(error.into())
        }
    }
}

fn validate_command_status(status: &str) -> AppResult<()> {
    crate::models::validate_agent_command_run_status(status).map_err(AppError::External)
}

fn validate_management_plan(plan: &ManagementPlan) -> AppResult<()> {
    if plan.tool_calls.len() > MAX_MANAGEMENT_TOOL_CALLS {
        return Err(AppError::BadRequest(format!(
            "management plan has {} tool calls; maximum is {MAX_MANAGEMENT_TOOL_CALLS}",
            plan.tool_calls.len()
        )));
    }
    Ok(())
}

fn management_plan_hash(plan: &ManagementPlan) -> AppResult<String> {
    let bytes = serde_json::to_vec(plan)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn management_tool_intent_key(
    command_run_id: mongodb::bson::oid::ObjectId,
    plan_hash: &str,
    call_index: usize,
) -> String {
    format!("management-tool:v1:{command_run_id}:{plan_hash}:{call_index}")
}

fn tool_call_json(call: &AgentToolCall) -> Value {
    json!({
        "id": call.id.map(|id| id.to_hex()).unwrap_or_default(),
        "toolName": call.tool_name,
        "arguments": call.arguments,
        "status": call.status,
        "response": call.response,
        "error": call.error,
    })
}

async fn load_management_command_calls(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    command_run_id: mongodb::bson::oid::ObjectId,
) -> AppResult<Vec<Value>> {
    let mut cursor = state
        .db
        .tool_calls()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "command_run_id": command_run_id,
            },
            FindOptions::builder()
                .sort(doc! { "call_index": 1, "created_at": 1 })
                .build(),
        )
        .await?;
    let mut calls = Vec::new();
    while let Some(call) = cursor.try_next().await? {
        calls.push(tool_call_json(&call));
    }
    Ok(calls)
}

fn persisted_tool_outcome(call: &AgentToolCall) -> Option<ToolOutcome> {
    match call.status.as_str() {
        "succeeded" | "dry_run" => {
            if call.tool_name == "wechatagent.send_contact_message" {
                call.response
                    .as_ref()
                    .and_then(|response| {
                        mongodb::bson::from_document::<Value>(response.clone()).ok()
                    })
                    .map(|response| assert_tool_outcome(&call.tool_name, &response))
                    .or(Some(ToolOutcome::Succeeded))
            } else {
                Some(ToolOutcome::Succeeded)
            }
        }
        // 恢复既有 `accepted`：重放 response 拿回原始受理原因；response 缺失/不可解析
        // 时保持 Accepted 语义，绝不降级成 Succeeded（会把「已受理」读成「已送达」）。
        "accepted" => call
            .response
            .as_ref()
            .and_then(|response| mongodb::bson::from_document::<Value>(response.clone()).ok())
            .map(|response| assert_tool_outcome(&call.tool_name, &response))
            .or_else(|| {
                Some(ToolOutcome::Accepted(
                    "发送意图已持久受理，等待异步送达回执".to_string(),
                ))
            }),
        "failed" => Some(ToolOutcome::Failed(
            call.error
                .clone()
                .unwrap_or_else(|| "tool execution failed".to_string()),
        )),
        "executed_unverified" => {
            Some(ToolOutcome::Unverified(call.error.clone().unwrap_or_else(
                || "tool result requires verification".to_string(),
            )))
        }
        "execution_unknown" => Some(ToolOutcome::ExecutionUnknown(
            call.error
                .clone()
                .unwrap_or_else(|| "tool execution outcome is unknown".to_string()),
        )),
        _ => None,
    }
}

async fn load_tool_call_by_intent(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    intent_key: &str,
) -> AppResult<AgentToolCall> {
    state
        .db
        .tool_calls()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "intent_key": intent_key,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::External("management tool intent missing".to_string()))
}

/// Execute a frozen plan through durable per-call intents. A terminal intent is
/// reused. A prepared intent may be claimed exactly once. An intent left in
/// executing by a crashed process is never replayed: it converges to
/// execution_unknown and stops the remaining plan.
pub(super) async fn execute_plan_tool_calls(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    tool_calls: &[PlannedToolCall],
    command_run_id: mongodb::bson::oid::ObjectId,
    plan_hash: &str,
    dry_run: bool,
    advertised: &HashSet<String>,
    confirmed_admin: Option<&AuthenticatedAdmin>,
    execution_token: Option<&str>,
) -> AppResult<PlanExecution> {
    if !tool_calls.is_empty() && execution_token.is_none() {
        return Err(AppError::Conflict(
            "management_command_execution_token_missing".to_string(),
        ));
    }
    let lease = execution_token.map(|token| ManagementExecutionLease {
        run_id: command_run_id,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        token: token.to_string(),
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let heartbeat = lease.as_ref().map(|lease| {
        spawn_management_execution_heartbeat(state.clone(), lease.clone(), cancelled.clone())
    });
    let result = execute_plan_tool_calls_owned(
        state,
        workspace_id,
        account_id,
        tool_calls,
        command_run_id,
        plan_hash,
        dry_run,
        advertised,
        confirmed_admin,
        lease.as_ref(),
        cancelled.as_ref(),
    )
    .await;
    if let Some(handle) = heartbeat {
        handle.abort();
    }
    result
}

async fn execute_plan_tool_calls_owned(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    tool_calls: &[PlannedToolCall],
    command_run_id: mongodb::bson::oid::ObjectId,
    plan_hash: &str,
    dry_run: bool,
    advertised: &HashSet<String>,
    confirmed_admin: Option<&AuthenticatedAdmin>,
    lease: Option<&ManagementExecutionLease>,
    cancelled: &AtomicBool,
) -> AppResult<PlanExecution> {
    if tool_calls.len() > MAX_MANAGEMENT_TOOL_CALLS {
        return Err(AppError::BadRequest(format!(
            "management plan has {} tool calls; maximum is {MAX_MANAGEMENT_TOOL_CALLS}",
            tool_calls.len()
        )));
    }
    let mut calls = Vec::new();
    let mut outcomes: Vec<(String, ToolOutcome)> = Vec::new();
    let mut failed = None;
    let mut execution_unknown = false;
    for (call_index, planned) in tool_calls.iter().enumerate() {
        if let Some(lease) = lease {
            ensure_management_execution_owned(state, lease, cancelled).await?;
        }
        let intent_key = management_tool_intent_key(command_run_id, plan_hash, call_index);
        let arguments_doc = to_document(&planned.arguments).unwrap_or_else(|_| Document::new());
        let now = DateTime::now();
        let call_start = AgentToolCall {
            id: None,
            workspace_id: workspace_id.to_string(),
            account_id: account_id.to_string(),
            command_run_id,
            intent_key: Some(intent_key.clone()),
            call_index: call_index as i32,
            tool_name: planned.tool_name.clone(),
            arguments: arguments_doc.clone(),
            status: "prepared".to_string(),
            response: None,
            error: None,
            execution_started_at: None,
            finalized_at: None,
            created_at: now,
            updated_at: now,
        };
        crate::models::assert_tool_call_status_valid(&call_start.status);
        match state.db.tool_calls().insert_one(&call_start, None).await {
            Ok(_) => {}
            Err(error) if crate::routes::admin_taxonomies::is_duplicate_key_error(&error) => {}
            Err(error) => return Err(error.into()),
        }
        let mut stored =
            load_tool_call_by_intent(state, workspace_id, account_id, &intent_key).await?;

        if let Some(outcome) = persisted_tool_outcome(&stored) {
            calls.push(tool_call_json(&stored));
            outcomes.push((planned.tool_name.clone(), outcome.clone()));
            match outcome {
                ToolOutcome::Failed(reason) => {
                    failed = Some(reason);
                    break;
                }
                ToolOutcome::ExecutionUnknown(reason) => {
                    failed = Some(reason);
                    execution_unknown = true;
                    break;
                }
                ToolOutcome::Succeeded | ToolOutcome::Accepted(_) | ToolOutcome::Unverified(_) => {
                    continue
                }
            }
        }

        if stored.status == "executing" {
            let reason = "previous process stopped after execution began; outcome is unknown and the tool will not be replayed";
            crate::models::assert_tool_call_status_valid("execution_unknown");
            state
                .db
                .tool_calls()
                .update_one(
                    doc! {
                        "_id": stored.id,
                        "workspace_id": workspace_id,
                        "account_id": account_id,
                        "status": "executing",
                    },
                    doc! {
                        "$set": {
                            "status": "execution_unknown",
                            "error": reason,
                            "finalized_at": DateTime::now(),
                            "updated_at": DateTime::now(),
                        }
                    },
                    None,
                )
                .await?;
            stored = load_tool_call_by_intent(state, workspace_id, account_id, &intent_key).await?;
            calls.push(tool_call_json(&stored));
            outcomes.push((
                planned.tool_name.clone(),
                ToolOutcome::ExecutionUnknown(reason.to_string()),
            ));
            failed = Some(reason.to_string());
            execution_unknown = true;
            break;
        }
        if stored.status != "prepared" {
            return Err(AppError::Conflict(format!(
                "management_tool_intent_invalid_status:{}",
                stored.status
            )));
        }

        let call_id = stored
            .id
            .ok_or_else(|| AppError::External("tool call id missing".to_string()))?;
        let execution_started_at = DateTime::now();
        let claimed = state
            .db
            .tool_calls()
            .update_one(
                doc! {
                    "_id": call_id,
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "status": "prepared",
                },
                doc! {
                    "$set": {
                        "status": "executing",
                        "execution_started_at": execution_started_at,
                        "updated_at": execution_started_at,
                    }
                },
                None,
            )
            .await?;
        if claimed.modified_count != 1 {
            return Err(AppError::Conflict(
                "management_tool_intent_claim_conflict".to_string(),
            ));
        }
        let result = execute_management_tool(
            state,
            workspace_id,
            account_id,
            planned,
            dry_run,
            advertised,
            confirmed_admin,
        )
        .await;
        if let Some(lease) = lease {
            // Do not finalize the per-tool intent or continue the plan unless this
            // process still owns the parent command after the external call.
            ensure_management_execution_owned(state, lease, cancelled).await?;
        }
        let is_dry_run = should_dry_run_tool(&planned.tool_name, dry_run);
        match result {
            Ok(response) => {
                // RPC 返 Ok 不等于业务成功：核实真实结果（dry_run 不核实，视为 Succeeded）。
                let outcome = if is_dry_run {
                    ToolOutcome::Succeeded
                } else {
                    assert_tool_outcome(&planned.tool_name, &response)
                };
                let status_str = tool_call_status_for_outcome(&outcome, is_dry_run);
                let response_doc = to_document(&response).ok();
                crate::models::assert_tool_call_status_valid(status_str);
                let finalized = state
                    .db
                    .tool_calls()
                    .update_one(
                        doc! {
                            "_id": call_id,
                            "workspace_id": workspace_id,
                            "account_id": account_id,
                            "status": "executing",
                        },
                        doc! {
                            "$set": {
                                "status": status_str,
                                "response": response_doc,
                                "finalized_at": DateTime::now(),
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await?;
                if finalized.modified_count != 1 {
                    return Err(AppError::Conflict(
                        "management_tool_finalize_conflict".to_string(),
                    ));
                }
                stored =
                    load_tool_call_by_intent(state, workspace_id, account_id, &intent_key).await?;
                calls.push(tool_call_json(&stored));
                outcomes.push((planned.tool_name.clone(), outcome.clone()));
                // 业务 Failed 走的是 Ok(response) 分支（RPC 成功但结果失败），
                // 与原 Err 分支"失败即止"语义对齐：设 failed 并 break。
                // Unverified 不算失败（已执行，仅结果待核实）→ 不设 failed、不 break。
                if let ToolOutcome::Failed(why) = &outcome {
                    failed = Some(why.clone());
                    break;
                }
            }
            Err(error) => {
                let message = error.to_string();
                crate::models::assert_tool_call_status_valid("failed");
                let finalized = state
                    .db
                    .tool_calls()
                    .update_one(
                        doc! {
                            "_id": call_id,
                            "workspace_id": workspace_id,
                            "account_id": account_id,
                            "status": "executing",
                        },
                        doc! {
                            "$set": {
                                "status": "failed",
                                "error": &message,
                                "finalized_at": DateTime::now(),
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await?;
                if finalized.modified_count != 1 {
                    return Err(AppError::Conflict(
                        "management_tool_finalize_conflict".to_string(),
                    ));
                }
                stored =
                    load_tool_call_by_intent(state, workspace_id, account_id, &intent_key).await?;
                calls.push(tool_call_json(&stored));
                failed = Some(message);
                break;
            }
        }
    }
    Ok(PlanExecution {
        calls,
        outcomes,
        failed,
        execution_unknown,
    })
}

pub(super) fn build_confirm_filter(
    workspace_id: &str,
    run_id: &mongodb::bson::oid::ObjectId,
    account_id: &str,
    plan_hash: &str,
) -> Document {
    doc! {
        "_id": run_id,
        "workspace_id": workspace_id,
        "account_id": account_id,
        "plan_hash": plan_hash,
        "status": "pending_confirmation",
    }
}

pub(super) async fn create_management_session(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<CreateSessionRequest>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &admin.current_workspace, &payload.account_id).await?;
    let session = ManagementAgentSession {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: payload.account_id,
        title: payload
            .title
            .unwrap_or_else(|| "New command session".to_string()),
        dry_run: payload.dry_run,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state
        .db
        .management_sessions()
        .insert_one(&session, None)
        .await?;
    Ok(Json(json!({
        "id": result.inserted_id.as_object_id().map(|id| id.to_hex()),
        "dryRun": session.dry_run
    })))
}

pub(super) async fn post_management_message(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ManagementMessageRequest>,
) -> AppResult<Json<Value>> {
    if payload.content.trim().is_empty() {
        return Err(AppError::BadRequest("content is required".to_string()));
    }
    validate_account(&state, &admin.current_workspace, &payload.account_id).await?;
    let session_id = parse_object_id(&id)?;
    let session = state
        .db
        .management_sessions()
        .find_one(
            doc! {
                "_id": session_id,
                "workspace_id": &admin.current_workspace,
                "account_id": &payload.account_id,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("management session not found".to_string()))?;
    state
        .db
        .management_messages()
        .insert_one(
            ManagementAgentMessage {
                id: None,
                workspace_id: admin.current_workspace.clone(),
                account_id: payload.account_id.clone(),
                session_id,
                role: "user".to_string(),
                content: payload.content.clone(),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;

    let tools =
        mcp::list_tools_for_account(&state, &admin.current_workspace, &payload.account_id).await?;
    let tools = merge_product_tools(tools);
    let advertised_tools = advertised_tool_names(&tools);
    let context = management_context(&state, &admin.current_workspace, &payload.account_id).await?;
    let effective_dry_run = payload.dry_run.unwrap_or(session.dry_run);
    let mut plan = build_management_plan(
        &state,
        &admin.current_workspace,
        &payload.account_id,
        &payload.content,
        &tools,
        &context,
    )
    .await?;
    apply_locked_send_content(&mut plan, &payload.content, effective_dry_run)?;
    let plan_hash = management_plan_hash(&plan)?;
    let plan_doc = to_document(&plan)?;
    let tool_names: Vec<&str> = plan
        .tool_calls
        .iter()
        .map(|call| call.tool_name.as_str())
        .collect();
    let requires_confirmation = !effective_dry_run
        && (plan.requires_confirmation
            || plan.risk_level.eq_ignore_ascii_case("dangerous")
            || plan_requires_confirmation(&tool_names));
    let execution_token = (!requires_confirmation).then(|| uuid::Uuid::new_v4().to_string());
    let execution_started_at = execution_token.as_ref().map(|_| DateTime::now());
    let initial_status = if requires_confirmation {
        "pending_confirmation"
    } else {
        "running"
    };
    validate_command_status(initial_status)?;
    let prompt_versions = prompts::prompt_versions(
        &state.db,
        &admin.current_workspace,
        &["management.plan.system", "management.plan.policy"],
        Some("management"),
        None,
    )
    .await?;
    let run = AgentCommandRun {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: payload.account_id.clone(),
        session_id,
        operator_message: payload.content.clone(),
        status: initial_status.to_string(),
        plan: Some(plan_doc.clone()),
        plan_hash: Some(plan_hash.clone()),
        summary: plan.summary.clone(),
        error: None,
        execution_token: execution_token.clone(),
        execution_started_at,
        confirmed_by: None,
        confirmed_at: None,
        prompt_versions: prompt_versions.clone(),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let run_result = state.db.command_runs().insert_one(run, None).await?;
    let run_id = run_result
        .inserted_id
        .as_object_id()
        .ok_or_else(|| AppError::External("command run id missing".to_string()))?;
    let exec = execute_plan_tool_calls(
        &state,
        &admin.current_workspace,
        &payload.account_id,
        if requires_confirmation {
            &[]
        } else {
            plan.tool_calls.as_slice()
        },
        run_id,
        &plan_hash,
        effective_dry_run,
        &advertised_tools,
        None,
        execution_token.as_deref(),
    )
    .await?;
    let calls = exec.calls;
    let outcomes = exec.outcomes;
    let failed = exec.failed;

    let final_status = if requires_confirmation {
        "pending_confirmation"
    } else if failed.is_some() {
        "failed"
    } else if effective_dry_run
        && plan
            .tool_calls
            .iter()
            .any(|c| !tool_effect(&c.tool_name).read_only)
    {
        "dry_run"
    } else {
        "succeeded"
    };
    validate_command_status(final_status)?;
    let finalized = if requires_confirmation {
        state
            .db
            .command_runs()
            .update_one(
                doc! {
                    "_id": run_id,
                    "workspace_id": &admin.current_workspace,
                    "account_id": &payload.account_id,
                    "plan_hash": &plan_hash,
                    "status": "pending_confirmation",
                },
                doc! { "$set": { "updated_at": DateTime::now() } },
                None,
            )
            .await?
    } else {
        state
            .db
            .command_runs()
            .update_one(
                doc! {
                    "_id": run_id,
                    "workspace_id": &admin.current_workspace,
                    "account_id": &payload.account_id,
                    "plan_hash": &plan_hash,
                    "status": "running",
                    "execution_token": execution_token.as_deref(),
                },
                doc! {
                    "$set": {
                        "status": final_status,
                        "error": &failed,
                        "updated_at": DateTime::now(),
                    },
                    "$unset": {
                        "execution_token": "",
                        "execution_started_at": "",
                    }
                },
                None,
            )
            .await?
    };
    if finalized.matched_count != 1 {
        return Err(AppError::Conflict(
            "management_command_finalize_conflict".to_string(),
        ));
    }
    let assistant_text = if requires_confirmation {
        if plan.summary.trim().is_empty() {
            "该指令涉及高风险或需要确认的动作，已生成计划但未执行。".to_string()
        } else {
            format!("待确认：{}", plan.summary)
        }
    } else if let Some(error) = failed {
        format!("执行失败：{error}")
    } else if !outcomes.is_empty() {
        // spec §3.2：基于真实 outcome 汇报，区分"打算做"与"做成了什么"，不回放 plan.summary。
        build_execution_summary(&outcomes)
    } else if plan.summary.trim().is_empty() {
        "执行完成".to_string()
    } else {
        plan.summary.clone()
    };
    state
        .db
        .management_messages()
        .insert_one(
            ManagementAgentMessage {
                id: None,
                workspace_id: session.workspace_id,
                account_id: payload.account_id.clone(),
                session_id,
                role: "assistant".to_string(),
                content: assistant_text.clone(),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    Ok(Json(json!({
        "command": {
            "id": run_id.to_hex(),
            "accountId": payload.account_id,
            "planHash": plan_hash,
            "status": final_status,
            "summary": assistant_text,
            "plan": plan,
            "promptVersions": prompt_versions,
            "toolCalls": calls
        }
    })))
}

/// Confirm and execute a frozen command that is pending explicit approval.
/// 乐观锁仿 escalation/ledger.rs::resolve_escalation：find_one_and_update 仅命中
/// pending_confirmation 原子改 running，二次确认/并发只一个命中，其余拿 None 幂等返回。
/// filter 带 workspace_id 防 IDOR（不能跨 workspace 确认他人命令）。
pub(super) async fn confirm_management_command(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ConfirmManagementCommandRequest>,
) -> AppResult<Json<Value>> {
    let account_id = payload.account_id.trim();
    let requested_plan_hash = payload.plan_hash.trim();
    if account_id.is_empty() || requested_plan_hash.is_empty() {
        return Err(AppError::BadRequest(
            "accountId and planHash are required".to_string(),
        ));
    }
    validate_account(&state, &admin.current_workspace, account_id).await?;
    let run_id = parse_object_id(&id)?;

    // Validate the frozen plan before taking the execution lease. Legacy
    // commands without a plan hash and commands whose plan was changed after
    // rendering are intentionally not executable.
    let candidate = state
        .db
        .command_runs()
        .find_one(
            doc! {
                "_id": run_id,
                "workspace_id": &admin.current_workspace,
                "account_id": account_id,
                "plan_hash": requested_plan_hash,
            },
            None,
        )
        .await?;
    let Some(candidate) = candidate else {
        return Err(AppError::Conflict(
            "management_command_binding_mismatch_or_legacy".to_string(),
        ));
    };
    let plan: ManagementPlan = candidate
        .plan
        .as_ref()
        .cloned()
        .ok_or_else(|| AppError::Conflict("management_plan_missing".to_string()))
        .and_then(|document| {
            mongodb::bson::from_document(document)
                .map_err(|_| AppError::Conflict("management_plan_invalid".to_string()))
        })?;
    if management_plan_hash(&plan)? != requested_plan_hash {
        return Err(AppError::Conflict(
            "management_plan_hash_mismatch".to_string(),
        ));
    }
    validate_management_plan(&plan)?;

    if !matches!(
        candidate.status.as_str(),
        "pending_confirmation" | "running"
    ) {
        let calls =
            load_management_command_calls(&state, &admin.current_workspace, account_id, run_id)
                .await?;
        return Ok(Json(json!({
            "status": candidate.status,
            "summary": candidate.summary,
            "toolCalls": calls,
        })));
    }

    let now = DateTime::now();
    let stale_before =
        DateTime::from_millis(now.timestamp_millis() - MANAGEMENT_EXECUTION_LEASE_MILLIS);
    let execution_token = uuid::Uuid::new_v4().to_string();
    let run = state
        .db
        .command_runs()
        .find_one_and_update(
            doc! {
                "_id": run_id,
                "workspace_id": &admin.current_workspace,
                "account_id": account_id,
                "plan_hash": requested_plan_hash,
                "$or": [
                    { "status": "pending_confirmation" },
                    {
                        "status": "running",
                        "execution_started_at": { "$lte": stale_before },
                    },
                ],
            },
            doc! {
                "$set": {
                    "status": "running",
                    "execution_token": &execution_token,
                    "execution_started_at": now,
                    "confirmed_by": admin.username.trim(),
                    "confirmed_at": now,
                    "updated_at": now,
                }
            },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let Some(run) = run else {
        let current = state
            .db
            .command_runs()
            .find_one(
                doc! {
                    "_id": run_id,
                    "workspace_id": &admin.current_workspace,
                    "account_id": account_id,
                    "plan_hash": requested_plan_hash,
                },
                None,
            )
            .await?;
        let Some(current) = current else {
            return Ok(Json(json!({ "status": "already_processed_or_not_found" })));
        };
        let calls =
            load_management_command_calls(&state, &admin.current_workspace, account_id, run_id)
                .await?;
        return Ok(Json(json!({
            "status": current.status,
            "summary": current.summary,
            "toolCalls": calls,
        })));
    };
    let tools = merge_product_tools(
        mcp::list_tools_for_account(&state, &admin.current_workspace, &run.account_id).await?,
    );
    let advertised = advertised_tool_names(&tools);
    // 确认后真执行（非 dry_run），全执行已确认的 tool_calls（与 post_message 共用执行函数）。
    let exec = execute_plan_tool_calls(
        &state,
        &admin.current_workspace,
        &run.account_id,
        &plan.tool_calls,
        run_id,
        requested_plan_hash,
        false,
        &advertised,
        Some(&admin),
        Some(&execution_token),
    )
    .await?;
    let summary = build_execution_summary(&exec.outcomes);
    let final_status = if exec.execution_unknown {
        "execution_unknown"
    } else if exec.failed.is_some() {
        "failed"
    } else {
        "succeeded"
    };
    validate_command_status(final_status)?;
    let finalized = state
        .db
        .command_runs()
        .update_one(
            doc! {
                "_id": run_id,
                "workspace_id": &admin.current_workspace,
                "account_id": account_id,
                "plan_hash": requested_plan_hash,
                "status": "running",
                "execution_token": &execution_token,
            },
            doc! {
                "$set": {
                    "status": final_status,
                    "summary": &summary,
                    "error": &exec.failed,
                    "updated_at": DateTime::now(),
                },
                "$unset": {
                    "execution_token": "",
                    "execution_started_at": "",
                }
            },
            None,
        )
        .await?;
    if finalized.matched_count != 1 {
        return Err(AppError::Conflict(
            "management_command_finalize_conflict".to_string(),
        ));
    }
    state
        .db
        .management_messages()
        .insert_one(
            ManagementAgentMessage {
                id: None,
                workspace_id: admin.current_workspace.clone(),
                account_id: run.account_id.clone(),
                session_id: run.session_id,
                role: "assistant".to_string(),
                content: summary.clone(),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    Ok(Json(json!({
        "status": final_status,
        "summary": summary,
        "toolCalls": exec.calls
    })))
}

/// 驳回此前因高风险被暂存的命令：乐观锁同 confirm filter（仅 pending_confirmation），
/// 原子改 canceled，落一条 assistant message 说明未执行。filter 带 workspace_id 防 IDOR。
pub(super) async fn reject_management_command(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ConfirmManagementCommandRequest>,
) -> AppResult<Json<Value>> {
    let account_id = payload.account_id.trim();
    let plan_hash = payload.plan_hash.trim();
    if account_id.is_empty() || plan_hash.is_empty() {
        return Err(AppError::BadRequest(
            "accountId and planHash are required".to_string(),
        ));
    }
    validate_account(&state, &admin.current_workspace, account_id).await?;
    let run_id = parse_object_id(&id)?;
    validate_command_status("canceled")?;
    let run = state
        .db
        .command_runs()
        .find_one_and_update(
            build_confirm_filter(&admin.current_workspace, &run_id, account_id, plan_hash),
            doc! { "$set": { "status": "canceled", "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    let Some(run) = run else {
        return Ok(Json(json!({ "status": "already_processed_or_not_found" })));
    };
    state
        .db
        .management_messages()
        .insert_one(
            ManagementAgentMessage {
                id: None,
                workspace_id: admin.current_workspace.clone(),
                account_id: run.account_id.clone(),
                session_id: run.session_id,
                role: "assistant".to_string(),
                content: "已取消该计划，未执行。".to_string(),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "status": "canceled" })))
}

pub(super) async fn get_management_command(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let run_id = parse_object_id(&id)?;
    let run = state
        .db
        .command_runs()
        .find_one(
            doc! { "_id": run_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("command run not found".to_string()))?;
    let mut cursor = state
        .db
        .tool_calls()
        .find(
            doc! {
                "command_run_id": run_id,
                "workspace_id": &admin.current_workspace,
                "account_id": &run.account_id,
            },
            FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .build(),
        )
        .await?;
    let mut calls = Vec::new();
    while let Some(call) = cursor.try_next().await? {
        calls.push(json!({
            "id": call.id.map(|id| id.to_hex()).unwrap_or_default(),
            "toolName": call.tool_name,
            "arguments": call.arguments,
            "status": call.status,
            "response": call.response,
            "error": call.error
        }));
    }
    Ok(Json(json!({
        "item": {
            "id": run_id.to_hex(),
            "accountId": run.account_id,
            "planHash": run.plan_hash,
            "status": run.status,
            "summary": run.summary,
            "error": run.error,
            "plan": run.plan,
            "promptVersions": run.prompt_versions,
            "toolCalls": calls
        }
    })))
}

pub(super) async fn get_tool_catalog(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    let tools = merge_product_tools(
        mcp::list_tools_for_account(&state, &admin.current_workspace, &account_id).await?,
    );
    Ok(Json(json!({ "tools": tools })))
}

fn is_forbidden_raw_send_tool(tool_name: &str) -> bool {
    tool_name.starts_with("message_send_")
}

fn remove_forbidden_raw_send_tools(value: &mut Value) {
    match value {
        Value::Array(items) => {
            items.retain(|item| {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str());
                !name.is_some_and(is_forbidden_raw_send_tool)
            });
            for item in items {
                remove_forbidden_raw_send_tools(item);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                remove_forbidden_raw_send_tools(item);
            }
        }
        _ => {}
    }
}

pub(super) fn merge_product_tools(mut tools: Value) -> Value {
    // Raw MCP send tools bypass the product gateway's contact eligibility,
    // content lock, review, ledger, and idempotency controls. Never advertise
    // them to the planner; the execution fallback rejects them independently.
    remove_forbidden_raw_send_tools(&mut tools);
    let product_tools = vec![
        json!({
            "name": "wechatagent.search_contacts",
            "description": "只搜索当前账号的微信好友，不写入系统。参数：query"
        }),
        json!({
            "name": "wechatagent.import_contacts",
            "description": "搜索并导入当前账号的微信好友。参数：query。该工具会写入联系人，dry-run 下只返回 would_execute。"
        }),
        json!({
            "name": "wechatagent.enable_contact_agent",
            "description": "把已导入好友加入 Agent 运营。参数：contactId 或 wxid，humanProfileNote"
        }),
        json!({
            "name": "wechatagent.disable_contact_agent",
            "description": "把好友移出 Agent 运营。参数：contactId 或 wxid"
        }),
        json!({
            "name": "wechatagent.create_follow_up_task",
            "description": "创建私聊跟进任务。参数：contactId 或 wxid，content，runAt(RFC3339)"
        }),
        json!({
            "name": "wechatagent.send_contact_message",
            "description": "通过生产发送网关给已导入且已纳入运营的好友发送私聊文本。参数：contactId 或 wxid 或 recipient，content。content 必须只包含最终发给好友的微信正文，不能混入操作说明、确认说明、测试说明或内部流程说明。"
        }),
        json!({
            "name": "wechatagent.update_contact_profile",
            "description": "更新好友运营画像字段。参数：contactId 或 wxid，tags，customerStage，intentLevel，lastCommitment，followUpPolicy，profileAttributes"
        }),
        // ── 批 1：观测查询类（只读，不写入）──
        json!({
            "name": "wechatagent.query_runs",
            "description": "查询最近的 Agent 运行日志（只读）。可选参数：accountId、contactWxid、limit。用于排查某客户最近几轮自动回复/跟进的运行情况。"
        }),
        json!({
            "name": "wechatagent.query_metrics",
            "description": "查询 Agent 运营成效指标（只读）。可选参数：accountId、horizon、fromDate、toDate、limit。用于看一段时间内的运营效果汇总。"
        }),
        json!({
            "name": "wechatagent.query_health",
            "description": "查询指定好友的运营健康度（只读）。参数：contactId。用于查看某客户的运营记忆、最近评审与状态。"
        }),
        json!({
            "name": "wechatagent.query_inbox",
            "description": "查询 AI 决策请示收件箱（只读）。可选参数：source。用于查看等待幕后决策源处理的请示条目。"
        }),
        json!({
            "name": "wechatagent.query_send_ledger",
            "description": "查询发送台账统计（只读）。必填参数：accountId；可选参数：kind。用于查看指定业务号各类发送的聚合统计。"
        }),
        // ── 批 1：版本与灰度类（publish=出草稿低风险；rollout/rollback/activate/灰度=高风险）──
        json!({
            "name": "wechatagent.publish_operation_domain_version",
            "description": "发布指定运营域配置为新版本（出草稿、不放量）。参数：id（运营域配置 id）"
        }),
        json!({
            "name": "wechatagent.rollout_operation_domain_version",
            "description": "把指定运营域配置版本放量生效（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.rollback_operation_domain_version",
            "description": "回滚运营域配置到指定版本（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.publish_state_policy_version",
            "description": "发布指定状态机策略为新版本（出草稿、不放量）。参数：id"
        }),
        json!({
            "name": "wechatagent.rollout_state_policy_version",
            "description": "把指定状态机策略版本放量生效（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.rollback_state_policy_version",
            "description": "回滚状态机策略到指定版本（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.publish_taxonomy_version",
            "description": "发布指定标签体系为新版本（出草稿、不放量）。参数：id"
        }),
        json!({
            "name": "wechatagent.rollout_taxonomy_version",
            "description": "把指定标签体系版本放量生效（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.rollback_taxonomy_version",
            "description": "回滚标签体系到指定版本（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.publish_domain_profile",
            "description": "发布指定 domain profile 为新版本（出草稿、不放量）。参数：id（profile id）"
        }),
        json!({
            "name": "wechatagent.rollout_domain_profile",
            "description": "把指定 domain profile 版本放量生效（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.rollback_domain_profile",
            "description": "回滚 domain profile 到指定版本（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.activate_domain_profile",
            "description": "把指定 domain profile 直接置为生效（立即影响线上）。参数：id"
        }),
        json!({
            "name": "wechatagent.release_evolution_proposal",
            "description": "发布（release）指定演进提案，使其阈值/提示词改动生效（立即影响线上，需确认串 RELEASE）。参数：id"
        }),
        json!({
            "name": "wechatagent.rollback_evolution_proposal",
            "description": "回滚指定演进提案的改动（立即影响线上，需确认串 ROLLBACK）。参数：id"
        }),
        json!({
            "name": "wechatagent.provider_activate",
            "description": "把指定 LLM provider 配置置为当前生效（立即切换线上模型）。参数：providerId，可选 workspaceId"
        }),
        json!({
            "name": "wechatagent.provider_test",
            "description": "测试指定 LLM provider 连通性（不改生效配置）。可选参数：providerId 或 inline 的 format/baseUrl/apiKey/model/timeoutSeconds，workspaceId"
        }),
        // ── 批 2：运营态（单对象写，low）──
        json!({
            "name": "wechatagent.update_assist_override",
            "description": "设置单个好友的辅助模式覆盖开关。参数：contactId，mode（default|force_on|force_off）。default 回落账号级总开关。"
        }),
        json!({
            "name": "wechatagent.update_custom_instructions",
            "description": "设置单个好友的运营特别指令（最高优先级 Operator Instruction 层，下一轮回复注入）。参数：contactId，instructions（上限 1000 字，trim 后空=清空）。"
        }),
        json!({
            "name": "wechatagent.update_manual_tags",
            "description": "更新好友的手动标签（manual，运营权威层；AI 永不覆盖本字段）。参数：contactId，tags（字符串数组，自动去空白去重）。"
        }),
        json!({
            "name": "wechatagent.write_deal_events",
            "description": "为好友登记一条成效/成交事件（append-only 正例）。参数：contactId，可选 amount（分）、currency、eventKind（deal|reversal）、productId、quantity、note、verification、occurredAtMs。"
        }),
        json!({
            "name": "wechatagent.analyze_profile",
            "description": "对指定好友重新生成运营画像（内部跑 LLM，老客户保留 stage/状态/承诺）。参数：contactId。"
        }),
        json!({
            "name": "wechatagent.review_task_now",
            "description": "立即触发执行指定跟进任务（不等调度间隔）。参数：taskId。"
        }),
        json!({
            "name": "wechatagent.cancel_task",
            "description": "取消指定的待执行跟进任务。参数：taskId。"
        }),
        json!({
            "name": "wechatagent.resolve_principal_escalation",
            "description": "对指定 AI 决策请示条目登记结构化裁决（跳过 LLM 解读直走转述）。参数：shortCode（请示短码），verdict；approved/conditional 必填 substance；可选 constraints、authorizationWindowHours（仅本次转述有效期，0-8760 小时）、exemptionType（none|customer_only|knowledge，客户豁免长期有效直至显式撤销）。"
        }),
        // ── 批 2：运行时调参（update_operation_domain=low；update_ask_human_policy=dangerous 立即改全量在跑 agent 行为）──
        json!({
            "name": "wechatagent.update_operation_domain",
            "description": "更新指定运营域配置本体（目标/方法论/工作流/工具策略/自动化策略/评审策略/运行参数/状态机），直接写当前生效版本。参数：domain（运营域标识），body（OperationDomainRequest 各字段）。"
        }),
        json!({
            "name": "wechatagent.update_ask_human_policy",
            "description": "更新指定运营域的请示决策链策略（decider_chain、安静时段等），立即改变全量在跑 agent 的请示行为。参数：domain，body（AskHumanPolicy：deciderChain 等）。"
        }),
        // ── 批 3：策略编辑类 ──
        json!({
            "name": "wechatagent.edit_soul",
            "description": "编辑指定 Agent 灵魂（人格底座）内容草稿。参数：soulId，body（AgentSoulRequest：agentKind、name、content）。仅改草稿不发布生效。"
        }),
        json!({
            "name": "wechatagent.publish_soul",
            "description": "发布指定 Agent 灵魂为当前生效版本（立即影响全量在跑 agent 的人格底座）。参数：soulId。"
        }),
        json!({
            "name": "wechatagent.edit_playbook",
            "description": "编辑指定运营 playbook（立即改生产方法论）。参数：playbookId、accountId、expectedVersion，以及 name、methodPrompt 等完整正文。accountId/expectedVersion 必须复制当前 Playbook 上下文。"
        }),
        json!({
            "name": "wechatagent.set_default_playbook",
            "description": "把指定运营 playbook 设为账号默认（立即生效）。参数：playbookId、accountId、expectedVersion，三者必须复制当前 Playbook 上下文。"
        }),
        json!({
            "name": "wechatagent.generate_playbook",
            "description": "用自然语言描述生成一份新运营 playbook（内部跑 LLM）。参数：body（GeneratePlaybookRequest：accountId、description）。"
        }),
        json!({
            "name": "wechatagent.optimize_playbook",
            "description": "用自然语言指令优化指定运营 playbook（内部跑 LLM，只生成新的非默认候选，不修改当前生产方法论）。参数：playbookId、accountId、expectedVersion、instruction；身份和版本必须复制当前 Playbook 上下文。"
        }),
        json!({
            "name": "wechatagent.edit_state_machine",
            "description": "直接编辑指定运营域的状态机本体（states/transitions/initial 等），立即改全局状态流转。参数：domain，body（裸状态机 Document）。"
        }),
        json!({
            "name": "wechatagent.promote_lesson",
            "description": "把指定经验沉淀（lesson）提升为同行案例库条目。参数：lessonId，body（PromoteLessonRequest：title、body 等）。"
        }),
        // ── 批 3：知识维护类 ──
        json!({
            "name": "wechatagent.verify_knowledge_chunk",
            "description": "把指定知识切片核验为 verified（写 source=Human，恒需人确认）。参数：chunkId、expectedUpdatedAt（管理员看到的 RFC3339 版本令牌），可选 verifiedClaims（字符串数组）。核验前须有 sourceQuote 且能锚定父文档。"
        }),
        json!({
            "name": "wechatagent.reject_knowledge_chunk",
            "description": "把指定知识切片标为 rejected（不可用）。参数：chunkId。"
        }),
        json!({
            "name": "wechatagent.archive_knowledge_chunk",
            "description": "归档指定知识切片（软下线，可逆）。参数：chunkId，body（ChunkArchiveRequest）。"
        }),
        json!({
            "name": "wechatagent.patch_knowledge_chunk",
            "description": "局部修改指定知识切片内容并退回待审（留修订历史）。参数：chunkId、patch（可编辑内容字段对象），可选 reason。"
        }),
        json!({
            "name": "wechatagent.split_knowledge_chunk",
            "description": "按 Unicode 字符位置把一条知识切片拆成两条待审草稿。参数：chunkId、offset（正整数），可选 reason。"
        }),
        json!({
            "name": "wechatagent.merge_knowledge_chunk",
            "description": "把指定知识切片合并到同作用域目标并退回待审。参数：chunkId、targetId，可选 reason。"
        }),
        json!({
            "name": "wechatagent.relate_knowledge_chunk",
            "description": "为指定知识切片建立关联。参数：chunkId、targetId、kind（references/requires/contradicts/clarifies/refines/superseded_by），可选 note/reason。"
        }),
        json!({
            "name": "wechatagent.batch_verify_chunks",
            "description": "批量核验多条知识切片（写 source=Human，恒需人确认）。参数：items（每项含 id、expectedUpdatedAt），可选 note。"
        }),
        json!({
            "name": "wechatagent.apply_gap_signal",
            "description": "采纳指定知识缺口信号并登记处置。参数：signalId，body（GapSignalResolutionRequest：note 等）。"
        }),
        json!({
            "name": "wechatagent.dismiss_gap_signal",
            "description": "忽略指定知识缺口信号。参数：signalId，body（GapSignalResolutionRequest：note 等）。"
        }),
        json!({
            "name": "wechatagent.import_knowledge_text",
            "description": "提交当前确认管理员已生成的知识导入预览（落库 status=draft，需后续人核验）。参数：previewId、previewHash、chunks（每项含 candidateId 与可选 patch）；不能绕过 import-preview。"
        }),
        json!({
            "name": "wechatagent.import_knowledge_image",
            "description": "从图片（视觉理解）导入运营知识切片（落库 status=draft，需后续人核验）。参数：body（ImportApplyImageRequest：imageBase64 等）。"
        }),
        // ── 批 4：需小重构才接入的工具（抽内部 fn / scope 校验 / 字节 helper）──
        json!({
            "name": "wechatagent.cancel_outbox",
            "description": "取消当前命令账号下指定的待发送 outbox 条目（仅 pending/in_flight 可取消，可逆单对象）。参数：id（outbox 条目 id），cancelReason（取消原因，上限 200 字）。"
        }),
        json!({
            "name": "wechatagent.approve_relationship_suggestion",
            "description": "审核通过指定的 relationship_type 建议，回写好友的关系类型（经维度字典校验）。参数：id（建议 id）。审核 actor 固定记录为 management_agent。"
        }),
        json!({
            "name": "wechatagent.approve_taxonomy_candidate",
            "description": "审核通过指定的标签候选并写入标签字典（出草稿性质）。仅可审 scope=global 或本账号的候选。参数：id（候选 id），canonicalValue（id、label、可选 aliases/description）。审核 actor 固定记录为 management_agent。"
        }),
        json!({
            "name": "wechatagent.import_knowledge_pdf",
            "description": "把 PDF（base64 编码字节）导入为运营知识切片（落库 status=draft，需后续人核验）。参数：sourceName（来源名），pdfBase64（base64 编码的 PDF 字节）。"
        }),
        json!({
            "name": "wechatagent.preview_campaign",
            "description": "创建一条活动草稿并做纯计算圈人预览（不会发送）。返回 campaignId、specVersion、specHash、命中人数和抽样；后续 dispatch 必须原样携带 specVersion/specHash。参数：title(活动名)，intentText(活动意图要点)，segmentFilter(圈人条件对象：productIds、aftercare、valueTier、customerStage)。"
        }),
        json!({
            "name": "wechatagent.dispatch_campaign",
            "description": "确认扇出活动推送：按已确认的不可变规格动态重圈一次并冻结本批受众，再可靠物化逐人跟进任务。高风险动作，执行前必须确认。参数：campaignId、specVersion、specHash（三者必须来自同一次 preview_campaign 返回）。"
        }),
    ];
    match &mut tools {
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get_mut("tools") {
                items.extend(product_tools);
            } else if let Some(Value::Array(items)) = map.get_mut("allowed_tools") {
                items.extend(product_tools.iter().map(|tool| {
                    tool.get("name")
                        .and_then(Value::as_str)
                        .map(|name| Value::String(name.to_string()))
                        .unwrap_or(Value::Null)
                }));
                map.insert("product_tools".to_string(), Value::Array(product_tools));
            } else if let Some(Value::Object(auth)) = map.get_mut("auth") {
                if let Some(Value::Array(items)) = auth.get_mut("allowed_tools") {
                    items.extend(product_tools.iter().map(|tool| {
                        tool.get("name")
                            .and_then(Value::as_str)
                            .map(|name| Value::String(name.to_string()))
                            .unwrap_or(Value::Null)
                    }));
                }
                map.insert("product_tools".to_string(), Value::Array(product_tools));
            } else {
                map.insert("product_tools".to_string(), Value::Array(product_tools));
            }
        }
        _ => {
            tools = json!({
                "mcp": tools,
                "product_tools": product_tools
            });
        }
    }
    tools
}

/// 从合并后的工具目录中收集所有"已被 tools/list 公布 + 已注册的产品工具"名称白名单。
/// 用于在 `execute_management_tool` 的兜底分支拦截 LLM 幻觉/注入出来、
/// MCP 服务端从未公布过的工具名，避免裸 `tools/call` 打到生产 MCP。
pub(super) fn advertised_tool_names(tools: &Value) -> HashSet<String> {
    fn collect(value: &Value, names: &mut HashSet<String>) {
        match value {
            Value::Object(map) => {
                // tools / product_tools：对象数组，取每项的 name
                for key in ["tools", "product_tools"] {
                    if let Some(Value::Array(items)) = map.get(key) {
                        for item in items {
                            if let Some(name) = item.get("name").and_then(Value::as_str) {
                                names.insert(name.to_string());
                            } else if let Some(name) = item.as_str() {
                                names.insert(name.to_string());
                            }
                        }
                    }
                }
                // allowed_tools：字符串数组
                if let Some(Value::Array(items)) = map.get("allowed_tools") {
                    for item in items {
                        if let Some(name) = item.as_str() {
                            names.insert(name.to_string());
                        }
                    }
                }
                // auth.allowed_tools / mcp.*：嵌套结构递归
                if let Some(auth) = map.get("auth") {
                    collect(auth, names);
                }
                if let Some(inner) = map.get("mcp") {
                    collect(inner, names);
                }
            }
            Value::Array(items) => {
                for item in items {
                    if let Some(name) = item.get("name").and_then(Value::as_str) {
                        names.insert(name.to_string());
                    } else if let Some(name) = item.as_str() {
                        names.insert(name.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    let mut names = HashSet::new();
    collect(tools, &mut names);
    names
}

pub(super) fn apply_locked_send_content(
    plan: &mut ManagementPlan,
    instruction: &str,
    dry_run: bool,
) -> AppResult<()> {
    let locked = extract_locked_send_content(instruction)?;
    for call in plan
        .tool_calls
        .iter_mut()
        .filter(|call| call.tool_name == "wechatagent.send_contact_message")
    {
        if let Some(locked) = &locked {
            let Some(map) = call.arguments.as_object_mut() else {
                if dry_run {
                    call.arguments = json!({
                        "content": "<extraction_failed: send_contact_message arguments must be an object>",
                        "originalContentLocked": true,
                        "lockedContentError": "send_contact_message arguments must be an object"
                    });
                    continue;
                }
                return Err(AppError::BadRequest(
                    "send_contact_message arguments must be an object".to_string(),
                ));
            };
            map.insert("content".to_string(), Value::String(locked.clone()));
            map.insert("originalContentLocked".to_string(), Value::Bool(true));
        }
    }
    Ok(())
}

pub(super) fn extract_locked_send_content(instruction: &str) -> AppResult<Option<String>> {
    let markers = [
        "内容必须完全等于：",
        "内容必须完全等于:",
        "内容必须等于：",
        "内容必须等于:",
        "内容完全等于：",
        "内容完全等于:",
        "内容为：",
        "内容为:",
        "发送内容：",
        "发送内容:",
        "发送：",
        "发送:",
    ];
    let Some((index, marker)) = markers
        .iter()
        .filter_map(|marker| instruction.find(marker).map(|index| (index, *marker)))
        .min_by_key(|(index, _)| *index)
    else {
        return Ok(None);
    };
    let text = instruction[index + marker.len()..].trim();
    if text.is_empty() {
        return Err(AppError::BadRequest(
            "locked send content is empty".to_string(),
        ));
    }
    if matches!(text.chars().next(), Some('“' | '"' | '「' | '『' | '\'')) {
        return extract_quoted_text(text)
            .map(|value| Some(value.trim().to_string()))
            .ok_or_else(|| {
                AppError::BadRequest("locked send content has an unclosed quote".to_string())
            });
    }
    // Unquoted text is accepted only when no operator-instruction separator is
    // present. Never truncate a user's message based on words that may belong
    // to the message itself; ambiguous requests must use explicit quotes.
    let ambiguous = [
        "。这是",
        "。不需要",
        "。不要",
        "。请不要",
        "；这是",
        "；不需要",
        "；不要",
        "\n",
    ]
    .iter()
    .any(|separator| text.contains(separator));
    if ambiguous {
        return Err(AppError::BadRequest(
            "ambiguous locked send content; wrap the exact message in quotes".to_string(),
        ));
    }
    let value = trim_wrapping_quotes(text).trim();
    if value.is_empty() {
        Err(AppError::BadRequest(
            "locked send content is empty".to_string(),
        ))
    } else {
        Ok(Some(value.to_string()))
    }
}

pub(super) fn extract_quoted_text(text: &str) -> Option<String> {
    let pairs = [
        ('“', '”'),
        ('"', '"'),
        ('「', '」'),
        ('『', '』'),
        ('\'', '\''),
    ];
    let first = text.chars().next()?;
    let end = pairs
        .iter()
        .find_map(|(open, close)| if *open == first { Some(*close) } else { None })?;
    let start_len = first.len_utf8();
    let rest = &text[start_len..];
    let end_index = rest.find(end)?;
    Some(rest[..end_index].to_string())
}

pub(super) fn trim_wrapping_quotes(text: &str) -> &str {
    let pairs = [
        ('“', '”'),
        ('"', '"'),
        ('「', '」'),
        ('『', '』'),
        ('\'', '\''),
    ];
    for (open, close) in pairs {
        if text.starts_with(open) && text.ends_with(close) {
            return &text[open.len_utf8()..text.len() - close.len_utf8()];
        }
    }
    text
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolRisk {
    Readonly,
    Low,
    Dangerous,
    Irreversible,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ToolEffect {
    pub read_only: bool,
    pub risk: ToolRisk,
    /// `false` means the tool reached the fail-closed fallback rather than an
    /// explicitly reviewed policy entry.
    pub explicitly_classified: bool,
}

pub(super) fn tool_effect(tool_name: &str) -> ToolEffect {
    use ToolRisk::*;
    let (risk, explicitly_classified) = match tool_name {
        // agent-policy.md explicitly classifies these raw MCP queries as read-only.
        "auth_whoami"
        | "account_list"
        | "account_get_status"
        | "contacts_search"
        | "contact_get_detail"
        | "schedule_list"
        | "knowledge.search"
        | "knowledge.list_catalog"
        | "knowledge.open_slice"
        | "knowledge.open_document"
        | "wechatagent.search_contacts"
        | "wechatagent.query_runs"
        | "wechatagent.query_metrics"
        | "wechatagent.query_health"
        | "wechatagent.query_inbox"
        | "wechatagent.query_send_ledger" => (Readonly, true),
        // 低风险可逆写
        // agent-policy.md allows these raw MCP actions to run automatically
        // under the current phase-one policy. Keep that behavior explicit.
        "media_get"
        | "schedule_create"
        | "schedule_cancel"
        | "wechatagent.import_contacts"
        | "wechatagent.enable_contact_agent"
        | "wechatagent.disable_contact_agent"
        | "wechatagent.create_follow_up_task"
        | "wechatagent.update_contact_profile"
        | "wechatagent.update_operation_domain"
        | "wechatagent.set_assist_mode"
        // 批 1：publish_*（出草稿、不放量）+ provider_test（连通测试不改生效配置）
        | "wechatagent.publish_operation_domain_version"
        | "wechatagent.publish_state_policy_version"
        | "wechatagent.publish_taxonomy_version"
        | "wechatagent.publish_domain_profile"
        | "wechatagent.provider_test"
        // 批 2：运营态单对象写 + update_operation_domain（改配置本体不放量）= Low
        | "wechatagent.update_assist_override"
        | "wechatagent.update_custom_instructions"
        | "wechatagent.update_manual_tags"
        | "wechatagent.write_deal_events"
        | "wechatagent.analyze_profile"
        | "wechatagent.review_task_now"
        | "wechatagent.cancel_task"
        | "wechatagent.resolve_principal_escalation"
        // 批 3：Soul 编辑只改草稿；Playbook 相关写会直接改变运行时方法论，见 Dangerous 分支。
        | "wechatagent.edit_soul"
        | "wechatagent.promote_lesson"
        // 批 3：知识维护（可逆单对象写 / 落 draft 待核验）= Low
        | "wechatagent.archive_knowledge_chunk"
        | "wechatagent.patch_knowledge_chunk"
        | "wechatagent.split_knowledge_chunk"
        | "wechatagent.merge_knowledge_chunk"
        | "wechatagent.relate_knowledge_chunk"
        | "wechatagent.apply_gap_signal"
        | "wechatagent.dismiss_gap_signal"
        | "wechatagent.import_knowledge_text"
        | "wechatagent.import_knowledge_image"
        // 批 4：cancel_outbox（取消可逆单对象）/ approve_relationship_suggestion（单 contact 回写）
        // / approve_taxonomy_candidate（出草稿性质）/ import_knowledge_pdf（导入落 draft 待核验）= Low
        | "wechatagent.cancel_outbox"
        | "wechatagent.approve_relationship_suggestion"
        | "wechatagent.approve_taxonomy_candidate"
        | "wechatagent.import_knowledge_pdf"
        | "wechatagent.preview_campaign" => (Low, true),
        // 高风险/宽影响（立即全量/改全局）
        "wechatagent.send_contact_message"
        | "wechatagent.edit_playbook"
        | "wechatagent.set_default_playbook"
        | "wechatagent.generate_playbook"
        | "wechatagent.optimize_playbook"
        | "wechatagent.publish_prompt_template"
        | "wechatagent.edit_state_machine"
        | "wechatagent.provider_activate"
        | "wechatagent.verify_knowledge_chunk"
        | "wechatagent.reject_knowledge_chunk"
        // 批 3：publish_soul 发布=生效全局人格底座；batch_verify 同 verify 类（写 source=Human）→ Dangerous
        | "wechatagent.publish_soul"
        | "wechatagent.batch_verify_chunks"
        // 批 1：rollout/rollback/activate/灰度立即生效 → 高风险
        | "wechatagent.rollout_operation_domain_version"
        | "wechatagent.rollback_operation_domain_version"
        | "wechatagent.rollout_state_policy_version"
        | "wechatagent.rollback_state_policy_version"
        | "wechatagent.rollout_taxonomy_version"
        | "wechatagent.rollback_taxonomy_version"
        | "wechatagent.rollout_domain_profile"
        | "wechatagent.rollback_domain_profile"
        | "wechatagent.activate_domain_profile"
        | "wechatagent.release_evolution_proposal"
        | "wechatagent.rollback_evolution_proposal"
        | "wechatagent.dispatch_campaign"
        // 批 2：update_ask_human_policy 立即改全量在跑 agent 的请示行为（spec §4.1）→ Dangerous
        | "wechatagent.update_ask_human_policy" => (Dangerous, true),
        // 不可逆（reset/delete/物理销毁）：档位高于 dangerous，第一期即便放权也保留确认
        "wechatagent.reset_domain"
        | "wechatagent.delete_knowledge_chunk"
        | "wechatagent.reset_system_pack" => (Irreversible, true),
        // Any tool not reviewed above fails closed. It may still be shown in
        // the dynamic catalog and executed after explicit confirmation.
        _ => (Dangerous, false),
    };
    let read_only = matches!(risk, Readonly);
    ToolEffect {
        read_only,
        risk,
        explicitly_classified,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ToolOutcome {
    Succeeded,
    /// Durable gateway accepted the intent; delivery is asynchronous.
    Accepted(String),
    Failed(String),
    Unverified(String),
    ExecutionUnknown(String),
}

/// 核实工具调用的"业务结果"——区别于"调用返回 Ok"。返回 Ok 不等于业务成功
/// （如 MCP send 返 Ok 但 success=false=账号离线）。无法判定的诚实标 Unverified，
/// 绝不假报成功（spec §3）。
pub(super) fn assert_tool_outcome(tool_name: &str, response: &Value) -> ToolOutcome {
    // Product gateway returns ContactSendResult, not a direct MCP receipt.
    // outbox_enqueued/skipped_duplicate mean durable acceptance, never delivery.
    if tool_name == "wechatagent.send_contact_message" {
        let gateway_status = response
            .get("gatewayStatus")
            .or_else(|| response.get("gateway_status"))
            .and_then(Value::as_str);
        let reason = response
            .get("gatewayReason")
            .or_else(|| response.get("gateway_reason"))
            .and_then(Value::as_str)
            .unwrap_or("发送网关未提供原因");
        return match gateway_status {
            Some("outbox_enqueued" | "skipped_duplicate") => {
                ToolOutcome::Accepted("发送意图已持久受理，等待异步送达回执".to_string())
            }
            Some("sent") => ToolOutcome::Succeeded,
            Some(_) => ToolOutcome::Failed(reason.to_string()),
            None => ToolOutcome::Unverified(
                "发送网关响应无 gatewayStatus，无法确认是否已受理".to_string(),
            ),
        };
    }
    // 写库类：核实 matched/modified
    if let Some(matched) = response.get("matched").and_then(Value::as_i64) {
        if matched == 0 {
            return ToolOutcome::Failed("未命中任何记录，实际没有改动".to_string());
        }
        return ToolOutcome::Succeeded;
    }
    // 显式 ok:true 的产品工具
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        return ToolOutcome::Succeeded;
    }
    // 只读查询：返回了结构即视为成功
    if matches!(tool_effect(tool_name).risk, ToolRisk::Readonly) {
        return ToolOutcome::Succeeded;
    }
    // 兜底：无法判定 → 诚实标 Unverified
    ToolOutcome::Unverified(format!(
        "工具 '{tool_name}' 已执行，但响应结构无法确认业务结果，请核对"
    ))
}

/// `ToolOutcome` → `agent_tool_calls.status` 的落库映射（纯函数便于单测）。
///
/// 返回值必须全部落在 [`crate::models::ALLOWED_TOOL_CALL_STATUS`] 闭集内。
///
/// 关键纪律：`Accepted`（网关持久受理、异步送达）**绝不**映射成 `succeeded`。
/// 否则 status 字段说「成功」而同一次执行的 summary 说「已受理」，管理端同屏
/// 自相矛盾，且把「已入队」误报成「已送达」——与 `executed_unverified` 必须显
/// 「待核实」是同一条「诚实优于好看」纪律（spec §3.3）。
pub(super) fn tool_call_status_for_outcome(
    outcome: &ToolOutcome,
    is_dry_run: bool,
) -> &'static str {
    if is_dry_run {
        return "dry_run";
    }
    match outcome {
        ToolOutcome::Succeeded => "succeeded",
        ToolOutcome::Accepted(_) => "accepted",
        ToolOutcome::Failed(_) => "failed",
        ToolOutcome::Unverified(_) => "executed_unverified",
        ToolOutcome::ExecutionUnknown(_) => "execution_unknown",
    }
}

/// 基于真实执行结果生成汇报（spec §3.2：不回放 plan.summary，区分打算做与做成了什么）。
pub(super) fn build_execution_summary(results: &[(String, ToolOutcome)]) -> String {
    if results.is_empty() {
        return "没有需要执行的操作。".to_string();
    }
    let mut lines = Vec::new();
    for (tool, outcome) in results {
        match outcome {
            ToolOutcome::Succeeded => lines.push(format!("✅ {tool}：已完成")),
            ToolOutcome::Accepted(why) => lines.push(format!("📨 {tool}：已受理——{why}")),
            ToolOutcome::Failed(why) => lines.push(format!("❌ {tool}：失败——{why}")),
            ToolOutcome::Unverified(why) => lines.push(format!("⚠️ {tool}：已执行待核实——{why}")),
            ToolOutcome::ExecutionUnknown(why) => {
                lines.push(format!("⛔ {tool}：执行结果未知，已停止自动重放——{why}"))
            }
        }
    }
    lines.join("\n")
}

/// verify 类工具：把 chunk 推向 verified 的动作。它写 source=Human（verify.rs:101），
/// 包成 AI 工具会"AI 调用被记成人确认"——故恒强制确认，不随第一期开关放行（spec §4.3）。
/// batch_verify_chunks 同属 verify 类，一并恒确认；未进入显式风险表的动态工具也
/// fail closed，避免 `tools/list` 新增能力在第一阶段 dangerous 总闸关闭时自动执行。
/// 所有真实副作用默认要求确认。仅只读工具以及经明确审查、无持久副作用的
/// `media_get` / provider 连通性探测可直接执行。未分类工具始终 fail closed。
pub(super) fn plan_requires_confirmation(tool_names: &[&str]) -> bool {
    tool_names.iter().any(|name| {
        let effect = tool_effect(name);
        !effect.explicitly_classified
            || (!effect.read_only && !matches!(*name, "media_get" | "wechatagent.provider_test"))
    })
}

/// S-20 / Task 19：判断一个工具是否属于"read 类"豁免列表。
/// 这些工具不会修改业务数据，dry-run 模式下仍正常执行以便操作员能看到查询结果。
pub(super) fn is_read_tool(tool_name: &str) -> bool {
    tool_effect(tool_name).read_only
}

pub(super) fn should_dry_run_tool(tool_name: &str, dry_run: bool) -> bool {
    dry_run && !is_read_tool(tool_name)
}

pub(super) async fn execute_management_tool(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    planned: &PlannedToolCall,
    dry_run: bool,
    advertised: &HashSet<String>,
    confirmed_admin: Option<&AuthenticatedAdmin>,
) -> AppResult<Value> {
    // S-20 / Task 19：dry-run 模式下，所有非 read 类工具直接返回
    // would_execute 计划，不实际调用底层 MCP 或写库。
    if should_dry_run_tool(&planned.tool_name, dry_run) {
        let error = planned
            .arguments
            .get("lockedContentError")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        return Ok(json!({
            "dry_run": true,
            "would_execute": {
                "toolName": planned.tool_name,
                "arguments": planned.arguments,
                "error": error
            }
        }));
    }
    match planned.tool_name.as_str() {
        "wechatagent.search_contacts" => {
            let query = string_arg(&planned.arguments, "query")?;
            mcp::logged_call_for_account(
                state,
                workspace_id,
                account_id,
                "contacts_search",
                json!({ "query": query, "limit": 20 }),
            )
            .await
        }
        "wechatagent.import_contacts" => {
            let query = string_arg(&planned.arguments, "query")?;
            let result = mcp::logged_call_for_account(
                state,
                workspace_id,
                account_id,
                "contacts_search",
                json!({ "query": query, "limit": 20 }),
            )
            .await?;
            let items = result
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut imported = Vec::new();
            for item in items {
                if let Some(contact_value) = item.get("contact") {
                    if let Some(contact) =
                        upsert_contact_from_value(state, workspace_id, account_id, contact_value)
                            .await?
                    {
                        imported.push(ApiContact::from(contact));
                    }
                }
            }
            Ok(json!({ "items": imported }))
        }
        "wechatagent.enable_contact_agent" => {
            let note = string_arg(&planned.arguments, "humanProfileNote")
                .or_else(|_| string_arg(&planned.arguments, "note"))?;
            let contact =
                resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            // 账号不能运营自己。
            let self_wxid = state
                .db
                .accounts()
                .find_one(
                    doc! { "workspace_id": workspace_id, "account_id": account_id },
                    None,
                )
                .await?
                .and_then(|a| a.wxid);
            if crate::webhooks::is_self_account(&contact.wxid, self_wxid.as_deref()) {
                let _ = agent::write_event_for_account(
                    state,
                    workspace_id,
                    account_id,
                    Some(&contact.wxid),
                    "contact.enable_rejected_self",
                    "rejected",
                    "目标命中账号自身 wxid，拒绝纳入 AI 运营",
                    Some(doc! { "actor": "management_tool", "source": "enable_contact_agent" }),
                )
                .await;
                return Err(AppError::BadRequest(
                    "不能对账号自身 wxid 启用 Agent 运营".to_string(),
                ));
            }
            let playbook_id = planned.arguments.get("playbookId").and_then(Value::as_str);
            let playbook =
                resolve_playbook_for_contact(state, workspace_id, account_id, playbook_id).await?;
            let generated = agent::build_initial_operation_profile(
                state,
                workspace_id,
                account_id,
                &note,
                Some(&playbook),
            )
            .await?;
            let commitments_bson = commitments_with_optional_text(
                &contact.commitments,
                generated.last_commitment.as_deref(),
            );
            // #72：曾运营过的老客户重新启用保留 stage / operation_state / commitments。
            let mut set_doc = doc! {
                "agent_status": "managed",
                "human_profile_note": note,
                "agent_profile": to_bson(&generated.agent_profile)?,
                "playbook_id": playbook.id,
                "playbook_version": playbook.version,
                // T8：裸 `tags` 字段已废弃（Contact 不再有该字段），不再写孤儿键。
                // AI 画像标签归 confirmed_tags（子计划2），此处只保留非标签画像字段。
                "profile_attributes": generated.profile_attributes,
                "profile_updated_at": DateTime::now(),
                "updated_at": DateTime::now(),
            };
            let mut unset_doc = Document::new();
            if !is_previously_operated(&contact) {
                // H13：初始 operation_state 从 active 状态机的 initial 态取（替代写死 "new_contact"）。
                let domain_config =
                    agent::load_user_operation_domain_config(state, workspace_id).await?;
                let initial_state = agent::initial_operation_state_key(domain_config.as_ref());
                // I1：AI 生成的初始画像 stage/intent 经 dimension_registry 校验。AI 产出 →
                // WriteIntent::MachineWrite：越界值 drop（不阻断建档），不像 admin 那样 reject。
                let gen_stage = match generated.customer_stage.as_deref() {
                    Some(v) => apply_admin_dim_validation(
                        agent::dimension_registry::validate_dimension_value(
                            &state.db,
                            workspace_id,
                            "customer_stage",
                            v,
                            &contact.account_id,
                            agent::dimension_registry::WriteIntent::MachineWrite,
                        )
                        .await,
                    )?,
                    None => None,
                };
                let gen_intent = match generated.intent_level.as_deref() {
                    Some(v) => apply_admin_dim_validation(
                        agent::dimension_registry::validate_dimension_value(
                            &state.db,
                            workspace_id,
                            "intent_level",
                            v,
                            &contact.account_id,
                            agent::dimension_registry::WriteIntent::MachineWrite,
                        )
                        .await,
                    )?,
                    None => None,
                };
                // 建档初始 stage_changed=true（语义正确）；若 gen_stage 被 drop 成 None，
                // 第②项内核守卫（signals 无 customer_stage 即不刷时间戳）会兜住。
                insert_domain_stage_fields(
                    &mut set_doc,
                    gen_stage.as_deref(),
                    gen_intent.as_deref(),
                    true,
                );
                set_doc.insert("commitments", commitments_bson);
                set_doc.insert("follow_up_policy", generated.follow_up_policy);
                set_doc.insert("operation_state", initial_state);
                set_doc.insert(
                    "operation_state_reason",
                    "后台管理 Agent 纳入运营，等待后续互动确认阶段",
                );
                set_doc.insert("operation_state_confidence", 6);
                set_doc.insert("operation_state_updated_at", DateTime::now());
                unset_doc.insert("last_commitment", "");
            }
            let mut update_doc = doc! { "$set": set_doc };
            update_doc.insert("$inc", doc! { "profile_revision": 1i64 });
            if !unset_doc.is_empty() {
                update_doc.insert("$unset", unset_doc);
            }
            state
                .db
                .contacts()
                .update_one(doc! { "_id": contact.id }, update_doc, None)
                .await?;
            let _ = agent::write_event_for_account(
                state,
                workspace_id,
                account_id,
                Some(&contact.wxid),
                "contact.enabled_for_ops",
                "ok",
                "经管理工具纳入 AI 运营",
                Some(doc! { "actor": "management_tool", "source": "enable_contact_agent" }),
            )
            .await;
            let updated =
                find_contact_by_id(state, workspace_id, &contact.id.unwrap().to_hex()).await?;
            Ok(json!({ "item": ApiContact::from(updated) }))
        }
        "wechatagent.disable_contact_agent" => {
            let contact =
                resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            state
                .db
                .contacts()
                .update_one(
                    doc! { "_id": contact.id },
                    doc! { "$set": { "agent_status": "normal", "updated_at": DateTime::now() } },
                    None,
                )
                .await?;
            let _ = agent::write_event_for_account(
                state,
                workspace_id,
                account_id,
                Some(&contact.wxid),
                "contact.removed_from_ops",
                "ok",
                "经管理工具移出 AI 运营",
                Some(doc! { "actor": "management_tool", "source": "disable_contact_agent" }),
            )
            .await;
            Ok(json!({ "ok": true }))
        }
        "wechatagent.create_follow_up_task" => {
            let contact =
                resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            let content = string_arg(&planned.arguments, "content")?;
            let run_at = string_arg(&planned.arguments, "runAt")
                .ok()
                .and_then(|value| DateTime::parse_rfc3339_str(&value).ok())
                .unwrap_or_else(DateTime::now);
            state
                .db
                .tasks()
                .insert_one(
                    crate::models::AgentTask {
                        id: None,
                        workspace_id: workspace_id.to_string(),
                        account_id: account_id.to_string(),
                        contact_wxid: contact.wxid,
                        kind: "follow_up".to_string(),
                        run_at,
                        expires_at: Some(DateTime::from_millis(
                            run_at.timestamp_millis() + 48 * 60 * 60 * 1000,
                        )),
                        content,
                        status: "pending".to_string(),
                        source_decision_id: None,
                        review_required: true,
                        attempt_count: 0,
                        max_attempts: 3,
                        next_retry_at: None,
                        gateway_status: None,
                        cancel_reason: None,
                        error: None,
                        claimed_at: None,
                        claim_recovery_count: 0,
                        created_at: DateTime::now(),
                        updated_at: DateTime::now(),
                    },
                    None,
                )
                .await?;
            Ok(json!({ "ok": true }))
        }
        "wechatagent.send_contact_message" => {
            let content = string_arg(&planned.arguments, "content")?;
            let contact =
                resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            let response = agent::send_contact_message_gateway(
                state,
                contact,
                agent::ManualContactSend {
                    content,
                    source: doc! {
                        "toolName": "wechatagent.send_contact_message",
                        "arguments": to_document(&planned.arguments).unwrap_or_default()
                    },
                    original_content_locked: planned
                        .arguments
                        .get("originalContentLocked")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
            )
            .await?;
            Ok(json!(response))
        }
        "wechatagent.update_contact_profile" => {
            let contact =
                resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            // T8（子计划2 衔接）：旧裸 `tags` 字段已废弃（Contact 不再有该字段），这里
            // 写 "tags" 是孤儿键、反序列化即丢弃。management Agent 产出的是 AI 画像层标签，
            // 应进 confirmed_tags（带证据），由子计划2 落地，本任务不顺势改写 AI 标签语义。
            let profile_attributes = planned
                .arguments
                .get("profileAttributes")
                .or_else(|| planned.arguments.get("profile_attributes"))
                .and_then(|value| to_document(value).ok())
                .unwrap_or_default();
            // T8：management Agent 是 AI/MCP 侧工具（非 admin 直写 UI），stage/intent 写入前
            // 过 dimension_registry 校验——此前直落 set_doc 是旁路，绕开了维度闸门。
            // WriteIntent::MachineWrite：越界值 DropSilently（不阻断、不报错，与 LlmSignals
            // 主通道容错一致）；alias 归一到 canonical；字典未配置回退信任原值。
            let raw_stage = optional_value_arg(&planned.arguments, "customerStage")
                .or_else(|| optional_value_arg(&planned.arguments, "customer_stage"));
            let new_stage = match raw_stage.as_deref() {
                Some(v) => apply_admin_dim_validation(
                    agent::dimension_registry::validate_dimension_value(
                        &state.db,
                        workspace_id,
                        "customer_stage",
                        v,
                        &contact.account_id,
                        agent::dimension_registry::WriteIntent::MachineWrite,
                    )
                    .await,
                )?,
                None => None,
            };
            let prev_stage = contact
                .domain_attributes
                .as_ref()
                .and_then(|d| d.get_str("customer_stage").ok().map(|s| s.to_string()));
            // stage 实际未写入（new_stage=None：缺省 / 空串 / 越界被 drop）时绝不算变更——
            // 否则 insert_domain_stage_fields(stage_changed=true) 会错误重置 stagnation 计时器。
            let stage_changed =
                new_stage.is_some() && prev_stage.as_deref() != new_stage.as_deref();
            let new_commitment_text = optional_value_arg(&planned.arguments, "lastCommitment")
                .or_else(|| optional_value_arg(&planned.arguments, "last_commitment"));
            let commitments_bson = commitments_with_optional_text(
                &contact.commitments,
                new_commitment_text.as_deref(),
            );
            let mut set_doc = doc! {
                "commitments": commitments_bson,
                "follow_up_policy": optional_value_arg(&planned.arguments, "followUpPolicy")
                    .or_else(|| optional_value_arg(&planned.arguments, "follow_up_policy")),
                "profile_attributes": profile_attributes,
                "profile_updated_at": DateTime::now(),
                "updated_at": DateTime::now(),
            };
            let raw_intent = optional_value_arg(&planned.arguments, "intentLevel")
                .or_else(|| optional_value_arg(&planned.arguments, "intent_level"));
            let new_intent = match raw_intent.as_deref() {
                Some(v) => apply_admin_dim_validation(
                    agent::dimension_registry::validate_dimension_value(
                        &state.db,
                        workspace_id,
                        "intent_level",
                        v,
                        &contact.account_id,
                        agent::dimension_registry::WriteIntent::MachineWrite,
                    )
                    .await,
                )?,
                None => None,
            };
            insert_domain_stage_fields(
                &mut set_doc,
                new_stage.as_deref(),
                new_intent.as_deref(),
                stage_changed,
            );
            state
                .db
                .contacts()
                .update_one(
                    doc! { "_id": contact.id },
                    doc! {
                        "$set": set_doc,
                        "$unset": { "last_commitment": "" },
                        "$inc": { "profile_revision": 1i64 }
                    },
                    None,
                )
                .await?;
            Ok(json!({ "ok": true }))
        }
        // ── 批 1：版本与灰度类（17 个）+ 观测查询类（5 个）。复用 crate::routes
        // 兄弟模块已有 REST handler——构造提取器 newtype 调用，拆 .0 取 Value。
        // 管理 agent 发起：username 标 "management-agent" 便于审计，user_id 占位，
        // current_workspace 用传入 workspace_id（多租户隔离关键）。──
        "wechatagent.query_runs" => {
            let q = serde_json::from_value(planned.arguments.clone())
                .or_else(|_| serde_json::from_value(json!({})))
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::tasks::list_agent_runs(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Query(q),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.query_metrics" => {
            let q = serde_json::from_value(planned.arguments.clone())
                .or_else(|_| serde_json::from_value(json!({})))
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::outcome_metrics::list_agent_outcome_metrics(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Query(q),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.query_health" => {
            let contact_id = string_arg(&planned.arguments, "contactId")?;
            let resp = crate::routes::contacts::get_operation_health(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(contact_id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.query_inbox" => {
            let q = serde_json::from_value(planned.arguments.clone())
                .or_else(|_| serde_json::from_value(json!({})))
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::ask_human_inbox::ask_human_inbox(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Query(q),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.query_send_ledger" => {
            let q = serde_json::from_value(planned.arguments.clone())
                .or_else(|_| serde_json::from_value(json!({})))
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::send_ledger::send_ledger_stats(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Query(q),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.publish_operation_domain_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::publish_operation_domain_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollout_operation_domain_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::rollout_operation_domain_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollback_operation_domain_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::rollback_operation_domain_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.publish_state_policy_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::publish_operation_state_policy_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollout_state_policy_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::rollout_operation_state_policy_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollback_state_policy_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::rollback_operation_state_policy_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.publish_taxonomy_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::publish_taxonomy_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollout_taxonomy_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::rollout_taxonomy_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollback_taxonomy_version" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::admin_ops_versions::rollback_taxonomy_version(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.publish_domain_profile" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::domain_profiles::publish_domain_profile(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollout_domain_profile" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::domain_profiles::rollout_domain_profile(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollback_domain_profile" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::domain_profiles::rollback_domain_profile(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.activate_domain_profile" => {
            let id = string_arg(&planned.arguments, "id")?;
            let resp = crate::routes::domain_profiles::activate_domain_profile(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.release_evolution_proposal" => {
            let id = string_arg(&planned.arguments, "id")?;
            // ConfirmationRequest 需精确确认串 "RELEASE"（evolution.rs:45）。管理 agent 经
            // plan 确认门后执行此动作，确认串由本分支补足以满足 handler 的硬校验。
            let body = serde_json::from_value(json!({ "confirmation": "RELEASE" }))
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::evolution::release_evolution_proposal(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.rollback_evolution_proposal" => {
            let id = string_arg(&planned.arguments, "id")?;
            let body = serde_json::from_value(json!({ "confirmation": "ROLLBACK" }))
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::evolution::rollback_evolution_proposal(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.provider_activate" => {
            let provider_id = string_arg(&planned.arguments, "providerId")?;
            // activate_provider 的 workspace 从 Query.workspaceId 取，缺省回落 admin.current_workspace；
            // 这里显式塞入传入 workspace_id 保证多租户隔离一致。
            let q = serde_json::from_value(json!({ "workspaceId": workspace_id }))
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::llm_providers::activate_provider(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(provider_id),
                Query(q),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.provider_test" => {
            // test_provider 无 Path，workspace 从 body.workspaceId 取；强制覆盖为可信的
            // 传入 workspace_id（同 provider_activate），丢弃 LLM arguments 里可能注入的
            // workspaceId，防跨租户读他人 provider 配置 apiKey 发起连通测试。
            let mut args = planned.arguments.clone();
            if let Some(map) = args.as_object_mut() {
                map.insert("workspaceId".to_string(), json!(workspace_id));
            } else {
                args = json!({ "workspaceId": workspace_id });
            }
            let body = serde_json::from_value(args)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::llm_providers::test_provider(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        // ── 批 2：运营态（单对象写）──
        "wechatagent.update_assist_override" => {
            let id = string_arg(&planned.arguments, "contactId")?;
            let arguments =
                management_expected_account_bound_arguments(&planned.arguments, account_id)?;
            let body = serde_json::from_value(arguments)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::contacts::update_assist_override(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.update_custom_instructions" => {
            let id = string_arg(&planned.arguments, "contactId")?;
            let arguments =
                management_expected_account_bound_arguments(&planned.arguments, account_id)?;
            let body = serde_json::from_value(arguments)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::contacts::update_custom_agent_instructions(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.update_manual_tags" => {
            let id = string_arg(&planned.arguments, "contactId")?;
            let arguments =
                management_expected_account_bound_arguments(&planned.arguments, account_id)?;
            let body = serde_json::from_value(arguments)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::contacts::update_manual_tags(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.write_deal_events" => {
            let admin = confirmed_admin.ok_or_else(|| {
                AppError::Conflict("deal_event_requires_confirmed_admin".to_string())
            })?;
            if admin.current_workspace != workspace_id || admin.username.trim().is_empty() {
                return Err(AppError::Conflict(
                    "deal_event_confirmed_admin_scope_mismatch".to_string(),
                ));
            }
            let contact =
                resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            let amount = planned.arguments.get("amount").and_then(Value::as_i64);
            let quantity = planned
                .arguments
                .get("quantity")
                .and_then(Value::as_u64)
                .map(u32::try_from)
                .transpose()
                .map_err(|_| AppError::BadRequest("quantity exceeds u32".to_string()))?;
            let occurred_at_ms = planned
                .arguments
                .get("occurredAtMs")
                .or_else(|| planned.arguments.get("occurred_at_ms"))
                .and_then(Value::as_i64);
            let outcome = add_outcome_event_inner(
                state,
                &contact,
                OutcomeEventInput {
                    source: "manual".to_string(),
                    marked_by: admin.username.trim().to_string(),
                    audit_summary: "management agent 经管理员确认登记成效事件".to_string(),
                    amount,
                    currency: optional_value_arg(&planned.arguments, "currency"),
                    verification: Some("staff_confirmed".to_string()),
                    event_kind: optional_value_arg(&planned.arguments, "eventKind")
                        .or_else(|| optional_value_arg(&planned.arguments, "event_kind")),
                    product_id: optional_value_arg(&planned.arguments, "productId")
                        .or_else(|| optional_value_arg(&planned.arguments, "product_id")),
                    quantity,
                    note: optional_value_arg(&planned.arguments, "note"),
                    occurred_at_ms,
                },
            )
            .await?;
            Ok(json!({ "ok": true, "event": outcome }))
        }
        "wechatagent.analyze_profile" => {
            let id = string_arg(&planned.arguments, "contactId")?;
            let resp = crate::routes::contacts::analyze_contact_profile(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.review_task_now" => {
            // 注：tasks handler 提取器顺序为 State+Path+Extension（与多数 Ext 在前的不同）。
            let id = string_arg(&planned.arguments, "taskId")?;
            let resp = crate::routes::tasks::review_task_now(
                State(state.clone()),
                Path(id),
                Extension(management_admin(workspace_id)),
                Json(crate::routes::tasks::TaskActionRequest::for_account(
                    account_id,
                )),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.cancel_task" => {
            let id = string_arg(&planned.arguments, "taskId")?;
            let resp = crate::routes::tasks::cancel_agent_task(
                State(state.clone()),
                Path(id),
                Extension(management_admin(workspace_id)),
                Json(crate::routes::tasks::TaskActionRequest::for_account(
                    account_id,
                )),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.resolve_principal_escalation" => {
            // shortCode 只用于 Path；严格 DTO 启用 deny_unknown_fields，不能把路由参数混进 body。
            let short_code = string_arg(&planned.arguments, "shortCode")?;
            let mut body_value = planned.arguments.clone();
            if let Some(object) = body_value.as_object_mut() {
                object.remove("shortCode");
            }
            let body = serde_json::from_value(body_value)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::principal_escalations::resolve_principal_escalation(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(short_code),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        // ── 批 2：运行时调参 ──
        "wechatagent.update_operation_domain" => {
            let domain = string_arg(&planned.arguments, "domain")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::domains::update_operation_domain(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(domain),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.update_ask_human_policy" => {
            let domain = string_arg(&planned.arguments, "domain")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::domains::put_ask_human_policy(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(domain),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        // ── 批 3：策略编辑类 ──
        "wechatagent.edit_soul" => {
            let id = string_arg(&planned.arguments, "soulId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::souls::update_agent_soul(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.publish_soul" => {
            // 注：publish_agent_soul 提取器顺序为 State+Path+Extension（Path 在 Ext 前）。
            let id = string_arg(&planned.arguments, "soulId")?;
            let resp = crate::routes::souls::publish_agent_soul(
                State(state.clone()),
                Path(id),
                Extension(management_admin(workspace_id)),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.edit_playbook" => {
            let id = string_arg(&planned.arguments, "playbookId")?;
            let arguments = management_account_bound_arguments(&planned.arguments, account_id)?;
            let body = serde_json::from_value(arguments)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::playbooks::update_operation_playbook(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.set_default_playbook" => {
            let id = string_arg(&planned.arguments, "playbookId")?;
            let arguments = management_account_bound_arguments(&planned.arguments, account_id)?;
            let body = serde_json::from_value(arguments)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::playbooks::set_default_operation_playbook(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.generate_playbook" => {
            let arguments = management_account_bound_arguments(&planned.arguments, account_id)?;
            let body = serde_json::from_value(arguments)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::playbooks::generate_operation_playbook(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.optimize_playbook" => {
            let id = string_arg(&planned.arguments, "playbookId")?;
            let arguments = management_account_bound_arguments(&planned.arguments, account_id)?;
            let body = serde_json::from_value(arguments)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::playbooks::optimize_operation_playbook(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.edit_state_machine" => {
            // body 是裸状态机 Document（mongodb::bson::Document），从 arguments.body 取。
            let domain = string_arg(&planned.arguments, "domain")?;
            let body_value = planned
                .arguments
                .get("body")
                .cloned()
                .unwrap_or_else(|| planned.arguments.clone());
            let body: Document = serde_json::from_value(body_value)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::domains::update_operation_domain_state_machine(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(domain),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.promote_lesson" => {
            let id = string_arg(&planned.arguments, "lessonId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::lessons_learned::promote_lesson_to_peer_case(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        // ── 批 3：知识维护类 ──
        "wechatagent.verify_knowledge_chunk" => {
            let id = string_arg(&planned.arguments, "chunkId")?;
            let mut body_value = planned.arguments.clone();
            if let Some(object) = body_value.as_object_mut() {
                object.remove("chunkId");
            }
            let body = serde_json::from_value(body_value)
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::verify_operation_knowledge_chunk(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.reject_knowledge_chunk" => {
            // 注：reject 无 Json body（State+Ext+Path）。
            let id = string_arg(&planned.arguments, "chunkId")?;
            let resp = crate::routes::knowledge::reject_operation_knowledge_chunk(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.archive_knowledge_chunk" => {
            let id = string_arg(&planned.arguments, "chunkId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::archive_operation_knowledge_chunk(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.patch_knowledge_chunk" => {
            let id = string_arg(&planned.arguments, "chunkId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::patch_operation_knowledge_chunk(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.split_knowledge_chunk" => {
            let id = string_arg(&planned.arguments, "chunkId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::split_operation_knowledge_chunk(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.merge_knowledge_chunk" => {
            let id = string_arg(&planned.arguments, "chunkId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::merge_operation_knowledge_chunk(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.relate_knowledge_chunk" => {
            let id = string_arg(&planned.arguments, "chunkId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::relate_operation_knowledge_chunk(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.batch_verify_chunks" => {
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::batch_verify_chunks(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.apply_gap_signal" => {
            let id = string_arg(&planned.arguments, "signalId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::apply_knowledge_gap_signal(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.dismiss_gap_signal" => {
            let id = string_arg(&planned.arguments, "signalId")?;
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::dismiss_knowledge_gap_signal(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(id),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.import_knowledge_text" => {
            let admin = confirmed_admin.ok_or_else(|| {
                AppError::Conflict("knowledge_import_requires_confirmed_admin".to_string())
            })?;
            if admin.current_workspace != workspace_id || admin.user_id.trim().is_empty() {
                return Err(AppError::Conflict(
                    "knowledge_import_confirmed_admin_scope_mismatch".to_string(),
                ));
            }
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::import_operation_knowledge_apply(
                State(state.clone()),
                Extension(admin.clone()),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.import_knowledge_image" => {
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let resp = crate::routes::knowledge::import_operation_knowledge_apply_image(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Json(body),
            )
            .await?;
            Ok(resp.0)
        }
        // ── 批 4：需小重构才接入的工具 ──
        "wechatagent.cancel_outbox" => {
            // 复用抽出的 cancel_outbox_inner（带 workspace 过滤）；不可取消时 inner
            // 返 AppError::Conflict，由 execute_plan_tool_calls 当 Err「失败即止」处理。
            let id = string_arg(&planned.arguments, "id")?;
            let reason = string_arg(&planned.arguments, "cancelReason")
                .or_else(|_| string_arg(&planned.arguments, "cancel_reason"))?;
            crate::routes::admin_outbox::cancel_outbox_inner(
                state,
                workspace_id,
                account_id,
                &id,
                &reason,
            )
            .await
        }
        "wechatagent.approve_relationship_suggestion" => {
            let id = string_arg(&planned.arguments, "id")?;
            let actor = crate::routes::shared::ReviewActor::system(
                crate::routes::shared::SystemReviewActor::ManagementAgent,
            );
            crate::routes::admin_relationship_suggestions::approve_relationship_suggestion_inner(
                state,
                workspace_id,
                &id,
                actor,
            )
            .await
        }
        "wechatagent.approve_taxonomy_candidate" => {
            // scope 隔离用 account_id（候选无 workspace_id 字段，隔离边界是 scope）：
            // 管理者只能 approve scope=global 或本 account_id 的候选（inner 内校验）。
            let id = string_arg(&planned.arguments, "id")?;
            let payload = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let actor = crate::routes::shared::ReviewActor::system(
                crate::routes::shared::SystemReviewActor::ManagementAgent,
            );
            crate::routes::admin_taxonomy_candidates::approve_taxonomy_candidate_inner(
                state,
                workspace_id,
                account_id,
                &id,
                payload,
                actor,
            )
            .await
        }
        "wechatagent.import_knowledge_pdf" => {
            // PDF 字节无法走 multipart，从 arguments 取 base64 解码后喂 import_pdf_bytes。
            // base64 解码方式与 multimodal.rs / media_send.rs 一致（STANDARD engine）。
            use base64::Engine;
            let source_name = string_arg(&planned.arguments, "sourceName")
                .or_else(|_| string_arg(&planned.arguments, "source_name"))?;
            let pdf_base64 = string_arg(&planned.arguments, "pdfBase64")
                .or_else(|_| string_arg(&planned.arguments, "pdf_base64"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(pdf_base64.trim())
                .map_err(|e| AppError::BadRequest(format!("pdfBase64 解码失败: {e}")))?;
            let outcome = crate::routes::knowledge::import_pdf_bytes(
                state,
                workspace_id,
                Some(account_id),
                &source_name,
                bytes,
            )
            .await?;
            Ok(json!({
                "documentId": outcome.document_id,
                "chunkIds": outcome.chunk_ids,
                "parseWarnings": outcome.parse_warnings,
                "fallbackBlob": outcome.fallback_blob,
            }))
        }
        "wechatagent.preview_campaign" => {
            // 先创建活动，再预览。create + preview 两步，返回 preview 结果。
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let created = crate::routes::campaigns::create_campaign(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Json(body),
            )
            .await?;
            let campaign_id = created
                .0
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::External("campaign id missing".to_string()))?
                .to_string();
            let resp = crate::routes::campaigns::preview_campaign(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(campaign_id),
            )
            .await?;
            Ok(resp.0)
        }
        "wechatagent.dispatch_campaign" => {
            let campaign_id = string_arg(&planned.arguments, "campaignId")?;
            let spec_hash = string_arg(&planned.arguments, "specHash")?;
            let spec_version = planned
                .arguments
                .get("specVersion")
                .and_then(Value::as_i64)
                .ok_or_else(|| AppError::BadRequest("specVersion is required".to_string()))?;
            let resp = crate::routes::campaigns::dispatch_campaign(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(campaign_id),
                Json(crate::routes::campaigns::DispatchCampaignRequest {
                    spec_hash,
                    spec_version,
                }),
            )
            .await?;
            Ok(resp.0)
        }
        _ => {
            if is_forbidden_raw_send_tool(&planned.tool_name) {
                return Err(AppError::BadRequest(format!(
                    "raw send tool '{}' is forbidden; use wechatagent.send_contact_message",
                    planned.tool_name
                )));
            }
            // 兜底分支：只允许把 tools/list 真实公布过的工具名透传给生产 MCP。
            if !advertised.contains(planned.tool_name.as_str()) {
                return Err(AppError::BadRequest(format!(
                    "tool '{}' is not advertised by the MCP server and is not a known product tool",
                    planned.tool_name
                )));
            }
            mcp::logged_call_for_account(
                state,
                workspace_id,
                account_id,
                &planned.tool_name,
                planned.arguments.clone(),
            )
            .await
        }
    }
}

/// 构造管理 Agent 复用 REST handler 时的 AuthenticatedAdmin 上下文。
/// current_workspace 用传入 workspace_id（多租户隔离关键）；username 标 "management-agent"
/// 便于审计能看出是管理 Agent 发起；user_id 占位（这些 handler 不依赖 user_id 做业务）。
pub(super) fn management_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: String::new(),
        username: "management-agent".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

pub(super) fn string_arg(arguments: &Value, key: &str) -> AppResult<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| AppError::BadRequest(format!("{key} is required")))
}

pub(super) fn optional_value_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn management_account_bound_arguments(arguments: &Value, account_id: &str) -> AppResult<Value> {
    let mut scoped = arguments.clone();
    let map = scoped
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("tool arguments must be an object".to_string()))?;
    if let Some(requested) = map.get("accountId") {
        if requested.as_str().map(str::trim) != Some(account_id) {
            return Err(AppError::Conflict(
                "management_tool_account_binding_mismatch".to_string(),
            ));
        }
    }
    map.insert("accountId".to_string(), json!(account_id));
    Ok(scoped)
}

fn management_expected_account_bound_arguments(
    arguments: &Value,
    account_id: &str,
) -> AppResult<Value> {
    let mut scoped = arguments.clone();
    let map = scoped
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("tool arguments must be an object".to_string()))?;
    if let Some(requested) = map.get("expectedAccountId") {
        if requested.as_str().map(str::trim) != Some(account_id) {
            return Err(AppError::Conflict(
                "management_tool_account_binding_mismatch".to_string(),
            ));
        }
    }
    map.insert("expectedAccountId".to_string(), json!(account_id));
    Ok(scoped)
}

pub(super) async fn resolve_contact_arg(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    arguments: &Value,
) -> AppResult<Contact> {
    if let Some(contact_id) = arguments.get("contactId").and_then(Value::as_str) {
        let contact = find_contact_by_id(state, workspace_id, contact_id).await?;
        if contact.account_id == account_id {
            return Ok(contact);
        }
    }
    let wxid = arguments
        .get("wxid")
        .or_else(|| arguments.get("recipient"))
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("contactId or wxid is required".to_string()))?;
    state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": wxid
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("contact not found".to_string()))
}

pub(super) async fn management_context(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<String> {
    let mut contacts = state
        .db
        .contacts()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(30)
                .build(),
        )
        .await?;
    let mut contact_lines = Vec::new();
    while let Some(contact) = contacts.try_next().await? {
        contact_lines.push(format!(
            "- id={} wxid={} name={} alias={} status={:?}",
            contact.id.map(|id| id.to_hex()).unwrap_or_default(),
            contact.wxid,
            contact
                .remark
                .or(contact.nickname)
                .unwrap_or_else(|| "-".to_string()),
            contact.alias.unwrap_or_else(|| "-".to_string()),
            contact.agent_status
        ));
    }
    let mut assets = state
        .db
        .content_assets()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "$or": [
                    { "account_id": null },
                    { "account_id": account_id }
                ]
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(20)
                .build(),
        )
        .await?;
    let mut asset_lines = Vec::new();
    while let Some(asset) = assets.try_next().await? {
        asset_lines.push(format!("- [{}] {}", asset.kind, asset.title));
    }
    let mut playbooks = state
        .db
        .operation_playbooks()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            FindOptions::builder()
                .sort(doc! { "is_default": -1, "updated_at": -1 })
                .limit(30)
                .build(),
        )
        .await?;
    let mut playbook_lines = Vec::new();
    while let Some(playbook) = playbooks.try_next().await? {
        playbook_lines.push(format!(
            "- id={} accountId={} version={} isDefault={} name={}",
            playbook.id.map(|id| id.to_hex()).unwrap_or_default(),
            playbook.account_id,
            playbook.version,
            playbook.is_default,
            playbook.name,
        ));
    }
    let mut campaigns = state
        .db
        .campaigns()
        .find(
            doc! {
                "workspaceId": workspace_id,
                "accountId": account_id,
                "status": { "$in": ["draft", "previewed"] },
            },
            FindOptions::builder()
                .sort(doc! { "updatedAt": -1 })
                .limit(20)
                .build(),
        )
        .await?;
    let mut campaign_lines = Vec::new();
    while let Some(campaign) = campaigns.try_next().await? {
        let Some(id) = campaign.id else { continue };
        let spec_hash = crate::routes::campaigns::campaign_spec_hash_for_view(&campaign)?;
        campaign_lines.push(format!(
            "- id={} title={} specVersion={} specHash={}",
            id.to_hex(),
            campaign.title,
            campaign.spec_version,
            spec_hash
        ));
    }
    Ok(format!(
        "当前账号: {}\n最近联系人:\n{}\n内容资产:\n{}\n当前 Playbook（写操作必须复制 id/accountId/version）:\n{}\n待派发活动（dispatch 必须复制 specVersion/specHash）:\n{}",
        account_id,
        contact_lines.join("\n"),
        asset_lines.join("\n"),
        playbook_lines.join("\n"),
        campaign_lines.join("\n")
    ))
}

pub(super) async fn build_management_plan(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    instruction: &str,
    tools: &Value,
    context: &str,
) -> AppResult<ManagementPlan> {
    let system = format!(
        "{}\n\n{}",
        prompts::load_prompt(&state.db, workspace_id, "management.plan.system",).await?,
        prompts::load_prompt(&state.db, workspace_id, "management.plan.policy",).await?
    );
    let user = format!(
        "操作员指令:\n{}\n\n当前系统上下文:\n{}\n\nMCP 工具目录:\n{}",
        instruction, context, tools
    );
    let value = agent::generate_agent_json(
        state,
        workspace_id,
        Some(account_id),
        None,
        None,
        "management.plan",
        &system,
        &user,
    )
    .await?;
    let mut plan: ManagementPlan = serde_json::from_value(value)?;
    plan.tool_calls
        .retain(|call| !call.tool_name.trim().is_empty());
    validate_management_plan(&plan)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_json_matches_contract_fixture() {
        let call = AgentToolCall {
            id: Some(mongodb::bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            command_run_id: mongodb::bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439012")
                .unwrap(),
            intent_key: Some("management-tool:v1:fixture".to_string()),
            call_index: 0,
            tool_name: "wechatagent.search_contacts".to_string(),
            arguments: doc! { "query": "Alice" },
            status: "succeeded".to_string(),
            response: Some(doc! { "count": 1 }),
            error: None,
            execution_started_at: Some(DateTime::from_millis(1_700_000_000_000)),
            finalized_at: Some(DateTime::from_millis(1_700_000_000_100)),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_000_100),
        };
        crate::routes::contract_snapshot::assert_contract_fixture(
            "tool_call",
            tool_call_json(&call),
        );
    }

    #[test]
    fn tool_effect_classifies_risk() {
        assert_eq!(
            tool_effect("wechatagent.search_contacts").risk,
            ToolRisk::Readonly
        );
        assert_eq!(
            tool_effect("wechatagent.create_follow_up_task").risk,
            ToolRisk::Low
        );
        assert_eq!(
            tool_effect("wechatagent.send_contact_message").risk,
            ToolRisk::Dangerous
        );
        // 批 1：publish_* 出草稿不放量 → Low（rollout/rollback 才 Dangerous）
        assert_eq!(
            tool_effect("wechatagent.publish_domain_profile").risk,
            ToolRisk::Low
        );
        assert_eq!(
            tool_effect("wechatagent.reset_domain").risk,
            ToolRisk::Irreversible
        );
        // 只读工具同时 read_only=true（与既有 dry-run 逻辑兼容）
        assert!(tool_effect("wechatagent.search_contacts").read_only);
    }

    #[test]
    fn campaign_tools_risk_and_confirmation() {
        // preview 会先创建活动草稿，因此不是只读，也必须确认。
        assert_eq!(
            tool_effect("wechatagent.preview_campaign").risk,
            ToolRisk::Low
        );
        assert!(!tool_effect("wechatagent.preview_campaign").read_only);
        assert!(plan_requires_confirmation(&[
            "wechatagent.preview_campaign"
        ]));
        assert!(
            plan_requires_confirmation(&["wechatagent.dispatch_campaign"]),
            "dispatch 必须恒走确认门"
        );
    }

    #[test]
    fn campaign_tools_in_catalog() {
        let merged = merge_product_tools(json!({ "tools": [] }));
        let names = advertised_tool_names(&merged);
        assert!(names.contains("wechatagent.preview_campaign"));
        assert!(names.contains("wechatagent.dispatch_campaign"));
    }

    #[test]
    fn merged_catalog_includes_batch1_tools() {
        let merged = merge_product_tools(json!({ "tools": [] }));
        let names = advertised_tool_names(&merged);
        for t in [
            // 观测查询类（5 个里取 2 个代表）
            "wechatagent.query_runs",
            "wechatagent.query_health",
            // 版本与灰度类代表
            "wechatagent.publish_domain_profile",
            "wechatagent.rollout_domain_profile",
            "wechatagent.activate_domain_profile",
            "wechatagent.release_evolution_proposal",
            "wechatagent.provider_activate",
        ] {
            assert!(names.contains(t), "catalog 缺工具 {t}");
        }
    }

    #[test]
    fn tool_effect_classifies_batch1_risk() {
        // query_* = Readonly
        assert_eq!(
            tool_effect("wechatagent.query_runs").risk,
            ToolRisk::Readonly
        );
        assert!(tool_effect("wechatagent.query_send_ledger").read_only);
        // publish_* = Low（出草稿不放量）
        assert_eq!(
            tool_effect("wechatagent.publish_domain_profile").risk,
            ToolRisk::Low
        );
        assert_eq!(
            tool_effect("wechatagent.publish_operation_domain_version").risk,
            ToolRisk::Low
        );
        // rollout/rollback/activate/provider/evolution = Dangerous
        assert_eq!(
            tool_effect("wechatagent.rollout_domain_profile").risk,
            ToolRisk::Dangerous
        );
        assert_eq!(
            tool_effect("wechatagent.rollback_taxonomy_version").risk,
            ToolRisk::Dangerous
        );
        assert_eq!(
            tool_effect("wechatagent.activate_domain_profile").risk,
            ToolRisk::Dangerous
        );
        assert_eq!(
            tool_effect("wechatagent.provider_activate").risk,
            ToolRisk::Dangerous
        );
        assert_eq!(
            tool_effect("wechatagent.release_evolution_proposal").risk,
            ToolRisk::Dangerous
        );
    }

    #[test]
    fn merged_catalog_includes_batch2_tools() {
        let merged = merge_product_tools(json!({ "tools": [] }));
        let names = advertised_tool_names(&merged);
        for t in [
            // 运营态类代表
            "wechatagent.update_manual_tags",
            "wechatagent.analyze_profile",
            "wechatagent.cancel_task",
            // 运行时调参类代表
            "wechatagent.update_operation_domain",
            "wechatagent.update_ask_human_policy",
        ] {
            assert!(names.contains(t), "catalog 缺工具 {t}");
        }
    }

    #[test]
    fn tool_effect_classifies_batch2_risk() {
        // 运营态单对象写 = Low
        assert_eq!(
            tool_effect("wechatagent.update_manual_tags").risk,
            ToolRisk::Low
        );
        assert_eq!(
            tool_effect("wechatagent.update_assist_override").risk,
            ToolRisk::Low
        );
        assert_eq!(tool_effect("wechatagent.cancel_task").risk, ToolRisk::Low);
        // 运行时调参：update_operation_domain = Low
        assert_eq!(
            tool_effect("wechatagent.update_operation_domain").risk,
            ToolRisk::Low
        );
        // ask_human_policy 立即改全量在跑 agent 行为 → Dangerous（spec §4.1）
        assert_eq!(
            tool_effect("wechatagent.update_ask_human_policy").risk,
            ToolRisk::Dangerous
        );
    }

    #[test]
    fn merged_catalog_includes_batch3_tools() {
        let merged = merge_product_tools(json!({ "tools": [] }));
        let names = advertised_tool_names(&merged);
        for t in [
            // 策略编辑类代表
            "wechatagent.edit_soul",
            "wechatagent.edit_playbook",
            "wechatagent.edit_state_machine",
            // 知识维护类代表
            "wechatagent.verify_knowledge_chunk",
            "wechatagent.patch_knowledge_chunk",
            "wechatagent.import_knowledge_text",
        ] {
            assert!(names.contains(t), "catalog 缺工具 {t}");
        }
    }

    #[test]
    fn tool_effect_classifies_batch3_risk() {
        // 策略编辑：soul/playbook 编辑 = Low（出草稿不放量）
        assert_eq!(tool_effect("wechatagent.edit_soul").risk, ToolRisk::Low);
        // 知识维护：patch 等可逆单对象写 = Low
        assert_eq!(
            tool_effect("wechatagent.patch_knowledge_chunk").risk,
            ToolRisk::Low
        );
        // 改全局状态机 = Dangerous（spec §4.1）
        assert_eq!(
            tool_effect("wechatagent.edit_state_machine").risk,
            ToolRisk::Dangerous
        );
        // verify 类 = Dangerous（推 chunk 到 verified，写 source=Human）
        assert_eq!(
            tool_effect("wechatagent.verify_knowledge_chunk").risk,
            ToolRisk::Dangerous
        );
    }

    #[test]
    fn merged_catalog_includes_batch4_tools() {
        let merged = merge_product_tools(json!({ "tools": [] }));
        let names = advertised_tool_names(&merged);
        for t in [
            "wechatagent.cancel_outbox",
            "wechatagent.approve_relationship_suggestion",
            "wechatagent.approve_taxonomy_candidate",
            "wechatagent.import_knowledge_pdf",
        ] {
            assert!(names.contains(t), "catalog 缺工具 {t}");
        }
    }

    #[test]
    fn tool_effect_classifies_batch4_risk() {
        // 批 4 四工具均归 Low（取消可逆 / 单 contact 回写 / 出草稿 / 导入落 draft）
        assert_eq!(tool_effect("wechatagent.cancel_outbox").risk, ToolRisk::Low);
        assert_eq!(
            tool_effect("wechatagent.approve_relationship_suggestion").risk,
            ToolRisk::Low
        );
        assert_eq!(
            tool_effect("wechatagent.approve_taxonomy_candidate").risk,
            ToolRisk::Low
        );
        assert_eq!(
            tool_effect("wechatagent.import_knowledge_pdf").risk,
            ToolRisk::Low
        );
        // Low 工具非只读：dry-run 下应被拦截不实际执行
        assert!(!tool_effect("wechatagent.cancel_outbox").read_only);
    }

    #[test]
    fn taxonomy_scope_check_allows_global_and_own_account() {
        use crate::routes::admin_taxonomy_candidates::taxonomy_scope_allows;
        // global 候选任何账号可审
        assert!(taxonomy_scope_allows("global", "acc_a"));
        // 同 account 候选本账号可审
        assert!(taxonomy_scope_allows("acc_a", "acc_a"));
        // 他 account 候选拒绝（跨 account scope 隔离）
        assert!(!taxonomy_scope_allows("acc_b", "acc_a"));
    }

    #[test]
    fn batch_verify_always_requires_confirmation() {
        // batch_verify 与单条 verify 同属 verify 类，恒强制确认无视第一期 dangerous 开关
        // （spec §4.3：守"AI 永不自动 verify"，AI 调 verify 会落 source=Human）。
        assert!(plan_requires_confirmation(&[
            "wechatagent.batch_verify_chunks"
        ]));
    }

    #[test]
    fn all_real_side_effects_require_confirmation() {
        assert!(plan_requires_confirmation(&[
            "wechatagent.send_contact_message"
        ]));
        assert!(plan_requires_confirmation(&[
            "wechatagent.update_contact_profile"
        ]));
        assert!(!plan_requires_confirmation(&[
            "wechatagent.search_contacts"
        ]));
        assert!(!plan_requires_confirmation(&["media_get"]));
        assert!(!plan_requires_confirmation(&["wechatagent.provider_test"]));
        assert!(plan_requires_confirmation(&["wechatagent.reset_domain"]));
        assert!(plan_requires_confirmation(&[
            "wechatagent.verify_knowledge_chunk"
        ]));
    }

    #[test]
    fn documented_raw_mcp_read_tools_are_classified_readonly() {
        for tool in [
            "auth_whoami",
            "account_list",
            "account_get_status",
            "contacts_search",
            "contact_get_detail",
            "schedule_list",
        ] {
            assert!(tool_effect(tool).read_only, "{tool} should be readonly");
            assert!(!plan_requires_confirmation(&[tool]));
        }
    }

    #[test]
    fn reviewed_raw_mcp_tools_follow_side_effect_policy() {
        assert!(!plan_requires_confirmation(&["media_get"]));
        for tool in ["schedule_create", "schedule_cancel"] {
            assert_eq!(tool_effect(tool).risk, ToolRisk::Low);
            assert!(tool_effect(tool).explicitly_classified);
            assert!(plan_requires_confirmation(&[tool]));
        }
        assert!(!tool_effect("message_send_text").explicitly_classified);
        assert!(plan_requires_confirmation(&["message_send_text"]));
    }

    #[test]
    fn raw_mcp_risk_policy_requires_confirmation_for_unclassified_tools() {
        for tool in [
            "friend_delete",
            "account_logout",
            "group_create",
            "moment_post_text",
            "personal_update_name",
            "gewe_execute_raw",
            "future_unclassified_mcp_write",
            "knowledge.open_mutating",
            "wechatagent.future_unclassified_product_tool",
        ] {
            assert_eq!(tool_effect(tool).risk, ToolRisk::Dangerous);
            assert!(
                plan_requires_confirmation(&[tool]),
                "{tool} should always require confirmation"
            );
        }
    }

    #[test]
    fn every_advertised_product_tool_has_an_explicit_risk_classification() {
        let tools = merge_product_tools(json!({ "tools": [] }));
        let names = advertised_tool_names(&tools);
        let product_names: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|name| name.starts_with("wechatagent."))
            .collect();
        assert!(
            !product_names.is_empty(),
            "product catalog should not be empty"
        );
        for name in product_names {
            assert!(
                tool_effect(name).explicitly_classified,
                "advertised product tool {name} is missing a reviewed risk classification"
            );
        }
    }

    #[test]
    fn outcome_assertion_detects_business_failure() {
        use serde_json::json;
        // Product gateway acceptance is queued, not a delivery receipt.
        let r = json!({
            "gatewayStatus": "outbox_enqueued",
            "gatewayReason": "queued",
            "messageId": null
        });
        assert!(matches!(
            assert_tool_outcome("wechatagent.send_contact_message", &r),
            ToolOutcome::Accepted(_)
        ));
        let r = json!({
            "gatewayStatus": "blocked_by_safety_guard",
            "gatewayReason": "review rejected"
        });
        assert!(matches!(
            assert_tool_outcome("wechatagent.send_contact_message", &r),
            ToolOutcome::Failed(_)
        ));
        // update: matched=0 → Failed（未命中、实际没改）
        let r = json!({"matched": 0, "modified": 0});
        assert!(matches!(
            assert_tool_outcome("wechatagent.update_contact_profile", &r),
            ToolOutcome::Failed(_)
        ));
        // update: modified>=1 → Succeeded
        let r = json!({"matched": 1, "modified": 1});
        assert!(matches!(
            assert_tool_outcome("wechatagent.update_contact_profile", &r),
            ToolOutcome::Succeeded
        ));
        // 无断言规则的工具 + response 无明显信号 → Unverified（诚实暴露）
        let r = json!({"weird": "shape"});
        assert!(matches!(
            assert_tool_outcome("wechatagent.some_unknown_tool", &r),
            ToolOutcome::Unverified(_)
        ));
        // readonly 查询：有数据即 Succeeded
        let r = json!({"items": []});
        assert!(matches!(
            assert_tool_outcome("wechatagent.query_runs", &r),
            ToolOutcome::Succeeded
        ));
    }

    /// 回归守卫：`Accepted`（持久受理、异步送达）绝不落 `succeeded`。
    ///
    /// 此前该映射内联在 match 里且无任何测试，导致 `Accepted` 被写成 `succeeded`：
    /// 管理端徽章显示「✅ 成功」而同屏 summary 显示「📨 已受理」，把「已入发件队列」
    /// 误报为「已送达」。同时锁住每个返回值都在闭集内。
    #[test]
    fn accepted_outcome_never_persists_as_succeeded() {
        use crate::models::ALLOWED_TOOL_CALL_STATUS;

        let accepted = ToolOutcome::Accepted("queued".to_string());
        let status = tool_call_status_for_outcome(&accepted, false);
        assert_eq!(
            status, "accepted",
            "Accepted 必须落 `accepted`；落 `succeeded` 会把已受理误报成已送达"
        );
        assert_ne!(status, "succeeded");

        // 真正成功仍是 succeeded，未被这次修正带偏。
        assert_eq!(
            tool_call_status_for_outcome(&ToolOutcome::Succeeded, false),
            "succeeded"
        );
        // dry_run 优先级最高：演练不核实业务结果。
        assert_eq!(tool_call_status_for_outcome(&accepted, true), "dry_run");

        // 每个映射结果都必须在闭集内，否则 assert_tool_call_status_valid 会告警。
        for outcome in [
            ToolOutcome::Succeeded,
            ToolOutcome::Accepted("why".to_string()),
            ToolOutcome::Failed("why".to_string()),
            ToolOutcome::Unverified("why".to_string()),
            ToolOutcome::ExecutionUnknown("why".to_string()),
        ] {
            for dry in [false, true] {
                let mapped = tool_call_status_for_outcome(&outcome, dry);
                assert!(
                    ALLOWED_TOOL_CALL_STATUS.contains(&mapped),
                    "映射结果 {mapped} 不在 ALLOWED_TOOL_CALL_STATUS 闭集内"
                );
            }
        }
    }

    /// `accepted` 落库后重放（进程重启 / plan 续跑）必须仍是 `Accepted`，
    /// 不得降级成 `Succeeded`——否则恢复路径会把「已受理」读成「已送达」。
    #[test]
    fn persisted_accepted_status_round_trips_without_downgrade() {
        use crate::models::AgentToolCall;
        use mongodb::bson::DateTime;

        let call = AgentToolCall {
            id: None,
            workspace_id: "ws".to_string(),
            account_id: "acc".to_string(),
            command_run_id: mongodb::bson::oid::ObjectId::new(),
            intent_key: Some("k".to_string()),
            call_index: 0,
            tool_name: "wechatagent.send_contact_message".to_string(),
            arguments: mongodb::bson::Document::new(),
            status: "accepted".to_string(),
            response: None,
            error: None,
            execution_started_at: None,
            finalized_at: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        assert!(
            matches!(
                persisted_tool_outcome(&call),
                Some(ToolOutcome::Accepted(_))
            ),
            "response 缺失时也必须保持 Accepted，绝不降级为 Succeeded"
        );
    }

    #[test]
    fn execution_summary_reports_real_outcomes() {
        let results = vec![
            (
                "wechatagent.update_contact_profile".to_string(),
                ToolOutcome::Succeeded,
            ),
            (
                "wechatagent.send_contact_message".to_string(),
                ToolOutcome::Failed("账号离线".to_string()),
            ),
        ];
        let s = build_execution_summary(&results);
        assert!(s.contains("update_contact_profile"));
        assert!(s.contains("失败") || s.contains("账号离线"));
        // 不假报全部成功
        assert!(!s.contains("全部成功"));

        let unv = vec![(
            "wechatagent.x".to_string(),
            ToolOutcome::Unverified("无法确认".to_string()),
        )];
        let s2 = build_execution_summary(&unv);
        assert!(s2.contains("待核实") || s2.contains("无法确认"));
    }

    #[test]
    fn management_machine_write_drops_out_of_dict_stage_not_reject() {
        // T8 旁路修复守护：management update_contact_profile 走 AI/MCP 通道 → MachineWrite。
        // customer_stage 主通道是 LlmSignals（机器容错）：字典有条目但此值越界(Miss) →
        // classify_validation 判 DropSilently（不阻断、不报错），apply_admin_dim_validation
        // 再把 DropSilently 映成 None → 该键不写入（越界值不落库脏值，且不像 admin 那样 400）。
        use crate::agent::dimension_registry::{
            classify_validation, spec_for, DictLookup, WriteIntent,
        };
        let stage = spec_for("customer_stage").unwrap();
        let v = classify_validation(stage, DictLookup::Miss, "臆造态", WriteIntent::MachineWrite);
        // 旁路修复前：值原样落库；修复后：MachineWrite 越界 → DropSilently → 不写。
        assert!(matches!(apply_admin_dim_validation(v), Ok(None)));

        // intent_level 同为 LlmSignals 机器通道，越界同样 drop（不写）而非报错。
        let intent = spec_for("intent_level").unwrap();
        let vi = classify_validation(
            intent,
            DictLookup::Miss,
            "瞎填意向",
            WriteIntent::MachineWrite,
        );
        assert!(matches!(apply_admin_dim_validation(vi), Ok(None)));

        // 合法值（alias 归一后 Accept）仍写入 canonical，证明校验不误杀正常路径。
        let ok = classify_validation(
            stage,
            DictLookup::Alias("need_discovery".into()),
            "需求挖掘",
            WriteIntent::MachineWrite,
        );
        assert!(matches!(apply_admin_dim_validation(ok), Ok(Some(ref c)) if c == "need_discovery"));
    }

    #[test]
    fn locked_send_content_rejects_ambiguous_unquoted_instruction() {
        let instruction = "请给 Jsjm 发送一条真实微信文本消息，内容必须完全等于：Jsjm，测试一下发送链路。收到不用回复。。这是链路验收，不需要二次确认。";
        assert!(matches!(
            extract_locked_send_content(instruction),
            Err(AppError::BadRequest(message)) if message.contains("ambiguous locked send content")
        ));
    }

    #[test]
    fn locked_send_content_prefers_quoted_body() {
        let instruction = "发送内容：\"只发送这一句。\" 不要创建跟进任务。";
        assert_eq!(
            extract_locked_send_content(instruction).unwrap().as_deref(),
            Some("只发送这一句。")
        );
    }

    #[test]
    fn apply_locked_send_content_overrides_llm_content() {
        let mut plan = ManagementPlan {
            tool_calls: vec![PlannedToolCall {
                tool_name: "wechatagent.send_contact_message".to_string(),
                arguments: json!({
                    "contactId": "abc",
                    "content": "污染后的内容"
                }),
            }],
            ..Default::default()
        };
        apply_locked_send_content(
            &mut plan,
            "内容必须完全等于：“原文消息。不要追加说明”",
            false,
        )
        .unwrap();
        let args = plan.tool_calls[0].arguments.as_object().unwrap();
        assert_eq!(
            args.get("content").and_then(Value::as_str),
            Some("原文消息。不要追加说明")
        );
        assert_eq!(
            args.get("originalContentLocked").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn dry_run_keeps_read_tools_live_and_blocks_write_tools() {
        assert!(!should_dry_run_tool("wechatagent.search_contacts", true));
        assert!(!should_dry_run_tool("knowledge.open_slice", true));
        assert!(!should_dry_run_tool("knowledge.open_document", true));
        assert!(should_dry_run_tool("knowledge.open_mutating", true));
        assert!(should_dry_run_tool("wechatagent.import_contacts", true));
        assert!(should_dry_run_tool(
            "wechatagent.send_contact_message",
            true
        ));
        assert!(!should_dry_run_tool(
            "wechatagent.send_contact_message",
            false
        ));
    }

    #[test]
    fn dry_run_locked_content_error_is_visible_in_arguments() {
        let mut plan = ManagementPlan {
            tool_calls: vec![PlannedToolCall {
                tool_name: "wechatagent.send_contact_message".to_string(),
                arguments: json!("bad arguments"),
            }],
            ..Default::default()
        };
        apply_locked_send_content(&mut plan, "内容必须完全等于：原文消息", true).unwrap();
        let args = plan.tool_calls[0].arguments.as_object().unwrap();
        assert_eq!(
            args.get("content").and_then(Value::as_str),
            Some("<extraction_failed: send_contact_message arguments must be an object>")
        );
        assert_eq!(
            args.get("lockedContentError").and_then(Value::as_str),
            Some("send_contact_message arguments must be an object")
        );
    }

    #[test]
    fn advertised_tool_names_collects_from_tools_array() {
        // tools/list 公布的对象数组 + merge_product_tools 追加的产品工具
        let merged = merge_product_tools(json!({
            "tools": [
                { "name": "account_list", "description": "list accounts" },
                { "name": "contacts_search", "description": "search" }
            ]
        }));
        let names = advertised_tool_names(&merged);
        assert!(names.contains("account_list"));
        assert!(names.contains("contacts_search"));
        // 产品工具也应进入白名单
        assert!(names.contains("wechatagent.send_contact_message"));
        assert!(names.contains("wechatagent.import_contacts"));
    }

    #[test]
    fn advertised_tool_names_collects_from_allowed_tools_and_auth() {
        // allowed_tools 字符串数组形态
        let merged = merge_product_tools(json!({
            "allowed_tools": ["account_list", "message_send_text"]
        }));
        let names = advertised_tool_names(&merged);
        assert!(names.contains("account_list"));
        assert!(!names.contains("message_send_text"));
        assert!(names.contains("wechatagent.search_contacts"));

        // auth.allowed_tools 嵌套形态
        let merged_auth = merge_product_tools(json!({
            "auth": { "allowed_tools": ["contacts_search"] }
        }));
        let names_auth = advertised_tool_names(&merged_auth);
        assert!(names_auth.contains("contacts_search"));
    }

    #[test]
    fn advertised_tool_names_collects_from_non_object_catalog() {
        // tools/list 返回非对象（数组）时 merge_product_tools 包成 { mcp, product_tools }
        let merged = merge_product_tools(json!([
            { "name": "account_list" },
            { "name": "message_send_text" }
        ]));
        let names = advertised_tool_names(&merged);
        assert!(names.contains("account_list"));
        assert!(!names.contains("message_send_text"));
        assert!(names.contains("wechatagent.update_contact_profile"));
    }

    #[test]
    fn unadvertised_tool_name_is_not_in_whitelist() {
        // LLM 幻觉/注入产生的、tools/list 从未公布的工具名不得进入白名单
        let merged = merge_product_tools(json!({
            "tools": [{ "name": "account_list" }]
        }));
        let names = advertised_tool_names(&merged);
        assert!(!names.contains("os.exec"));
        assert!(!names.contains("message_send_text"));
        assert!(!names.contains("admin.delete_workspace"));
    }

    #[test]
    fn confirm_filter_only_targets_pending_confirmation() {
        let filter = build_confirm_filter(
            "workspace1",
            &mongodb::bson::oid::ObjectId::new(),
            "account1",
            "plan-hash-1",
        );
        // filter 必须含 status: pending_confirmation（防二次确认 / 防确认非待确认命令）
        // 且带 workspace_id（IDOR：不能跨 workspace 确认他人命令）
        assert_eq!(filter.get_str("workspace_id").unwrap(), "workspace1");
        assert_eq!(filter.get_str("account_id").unwrap(), "account1");
        assert_eq!(filter.get_str("plan_hash").unwrap(), "plan-hash-1");
        assert_eq!(filter.get_str("status").unwrap(), "pending_confirmation");
    }

    #[test]
    fn tool_call_status_closed_set() {
        use crate::models::ALLOWED_TOOL_CALL_STATUS;
        const EXPECTED: &[&str] = &[
            "prepared",
            "executing",
            "dry_run",
            "succeeded",
            "accepted",
            "failed",
            "executed_unverified",
            "execution_unknown",
        ];
        for s in EXPECTED {
            assert!(ALLOWED_TOOL_CALL_STATUS.contains(s), "缺少状态 {s}");
        }
        assert_eq!(ALLOWED_TOOL_CALL_STATUS.len(), 8);
    }

    #[test]
    fn assert_tool_call_status_accepts_all_valid() {
        for s in [
            "prepared",
            "executing",
            "dry_run",
            "succeeded",
            "accepted",
            "failed",
            "executed_unverified",
            "execution_unknown",
        ] {
            crate::models::assert_tool_call_status_valid(s); // 不 panic 即通过
        }
    }
}

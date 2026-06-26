//! 管理 Agent 路由：管理对话 session、计划生成与工具执行。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, to_bson, to_document, DateTime, Document},
    options::FindOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

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
}

/// 执行一组 plan tool_calls：对每个工具 insert tool_call → 调 execute_management_tool
/// → 核实 outcome（assert_tool_outcome）→ 写终态（过闭集断言）→ 收集 calls/outcomes。
/// 业务 Failed 与 Err 都"失败即止"（set failed + break）；Unverified 不算失败继续。
/// post_message 与 confirm 共用，避免两份执行逻辑漂移（项目历史踩过 dual-path drift）。
///
/// 调用方决定要执行哪些 tool_calls：post_message 在 requires_confirmation 时传空切片
/// （等价原 take(0)，不执行只暂存），否则传 plan.tool_calls；confirm 已确认全执行。
/// 函数内部对传入切片做 `.take(12)` 上限（防一个 plan 塞超量工具）。
pub(super) async fn execute_plan_tool_calls(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    tool_calls: &[PlannedToolCall],
    command_run_id: mongodb::bson::oid::ObjectId,
    dry_run: bool,
    advertised: &HashSet<String>,
) -> AppResult<PlanExecution> {
    let mut calls = Vec::new();
    let mut outcomes: Vec<(String, ToolOutcome)> = Vec::new();
    let mut failed = None;
    for planned in tool_calls.iter().take(12) {
        let arguments_doc = to_document(&planned.arguments).unwrap_or_else(|_| Document::new());
        let call_start = AgentToolCall {
            id: None,
            workspace_id: workspace_id.to_string(),
            account_id: account_id.to_string(),
            command_run_id,
            tool_name: planned.tool_name.clone(),
            arguments: arguments_doc.clone(),
            status: if should_dry_run_tool(&planned.tool_name, dry_run) {
                "dry_run".to_string()
            } else {
                "running".to_string()
            },
            response: None,
            error: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        crate::models::assert_tool_call_status_valid(&call_start.status);
        let call_result = state.db.tool_calls().insert_one(call_start, None).await?;
        let call_id = call_result
            .inserted_id
            .as_object_id()
            .ok_or_else(|| AppError::External("tool call id missing".to_string()))?;
        let result =
            execute_management_tool(state, workspace_id, account_id, planned, dry_run, advertised).await;
        let is_dry_run = should_dry_run_tool(&planned.tool_name, dry_run);
        match result {
            Ok(response) => {
                // RPC 返 Ok 不等于业务成功：核实真实结果（dry_run 不核实，视为 Succeeded）。
                let outcome = if is_dry_run {
                    ToolOutcome::Succeeded
                } else {
                    assert_tool_outcome(&planned.tool_name, &response)
                };
                let status_str = match (&outcome, is_dry_run) {
                    (_, true) => "dry_run",
                    (ToolOutcome::Succeeded, _) => "succeeded",
                    (ToolOutcome::Failed(_), _) => "failed",
                    (ToolOutcome::Unverified(_), _) => "executed_unverified",
                };
                let response_doc = to_document(&response).ok();
                crate::models::assert_tool_call_status_valid(status_str);
                state
                    .db
                    .tool_calls()
                    .update_one(
                        doc! { "_id": call_id },
                        doc! {
                            "$set": {
                                "status": status_str,
                                "response": response_doc,
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await?;
                calls.push(json!({
                    "id": call_id.to_hex(),
                    "toolName": planned.tool_name,
                    "arguments": planned.arguments,
                    "status": status_str,
                    "response": response
                }));
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
                state
                    .db
                    .tool_calls()
                    .update_one(
                        doc! { "_id": call_id },
                        doc! {
                            "$set": {
                                "status": "failed",
                                "error": &message,
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await?;
                calls.push(json!({
                    "id": call_id.to_hex(),
                    "toolName": planned.tool_name,
                    "arguments": planned.arguments,
                    "status": "failed",
                    "error": message
                }));
                failed = Some(message);
                break;
            }
        }
    }
    Ok(PlanExecution {
        calls,
        outcomes,
        failed,
    })
}

/// 乐观锁过滤条件：仅命中本 workspace 下、状态为 pending_confirmation 的命令。
/// 带 workspace_id 防 IDOR（不能跨 workspace 确认他人命令）；带 status 防二次确认
/// / 防确认非待确认命令（confirm 与 reject 共用同一条件）。
pub(super) fn build_confirm_filter(
    workspace_id: &str,
    run_id: &mongodb::bson::oid::ObjectId,
) -> Document {
    doc! { "_id": run_id, "workspace_id": workspace_id, "status": "pending_confirmation" }
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

    let tools = mcp::list_tools_for_account(&state, &payload.account_id).await?;
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
    let plan_doc = to_document(&plan)?;
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
        status: "running".to_string(),
        plan: Some(plan_doc.clone()),
        summary: plan.summary.clone(),
        error: None,
        prompt_versions: prompt_versions.clone(),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let run_result = state.db.command_runs().insert_one(run, None).await?;
    let run_id = run_result
        .inserted_id
        .as_object_id()
        .ok_or_else(|| AppError::External("command run id missing".to_string()))?;
    let requires_confirmation =
        plan.requires_confirmation || plan.risk_level.eq_ignore_ascii_case("dangerous");
    // 抽公共执行函数后，"0 还是全部"由调用方传切片控制：requires_confirmation 时传空切片
    // （等价原 take(0)，只暂存不执行），否则传全部 tool_calls（函数内 take(12) 上限保留）。
    // confirm 与 post_message 共用同一执行函数，避免 dual-path drift。
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
        effective_dry_run,
        &advertised_tools,
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
    state
        .db
        .command_runs()
        .update_one(
            doc! { "_id": run_id },
            doc! {
                "$set": {
                    "status": final_status,
                    "error": &failed,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
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
            "status": final_status,
            "summary": assistant_text,
            "plan": plan,
            "promptVersions": prompt_versions,
            "toolCalls": calls
        }
    })))
}

/// 确认并执行此前因高风险被暂存（pending_confirmation）的命令。
/// 乐观锁仿 escalation/ledger.rs::resolve_escalation：find_one_and_update 仅命中
/// pending_confirmation 原子改 running，二次确认/并发只一个命中，其余拿 None 幂等返回。
/// filter 带 workspace_id 防 IDOR（不能跨 workspace 确认他人命令）。
pub(super) async fn confirm_management_command(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let run_id = parse_object_id(&id)?;
    let run = state
        .db
        .command_runs()
        .find_one_and_update(
            build_confirm_filter(&admin.current_workspace, &run_id),
            doc! { "$set": { "status": "running", "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    let Some(run) = run else {
        return Ok(Json(json!({ "status": "already_processed_or_not_found" })));
    };
    let plan: ManagementPlan = run
        .plan
        .as_ref()
        .and_then(|d| mongodb::bson::from_document(d.clone()).ok())
        .unwrap_or_default();
    let tools = merge_product_tools(mcp::list_tools_for_account(&state, &run.account_id).await?);
    let advertised = advertised_tool_names(&tools);
    // 确认后真执行（非 dry_run），全执行已确认的 tool_calls（与 post_message 共用执行函数）。
    let exec = execute_plan_tool_calls(
        &state,
        &admin.current_workspace,
        &run.account_id,
        &plan.tool_calls,
        run_id,
        false,
        &advertised,
    )
    .await?;
    let summary = build_execution_summary(&exec.outcomes);
    let final_status = if exec.failed.is_some() {
        "failed"
    } else {
        "succeeded"
    };
    state
        .db
        .command_runs()
        .update_one(
            doc! { "_id": run_id },
            doc! { "$set": { "status": final_status, "summary": &summary, "error": &exec.failed, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
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
) -> AppResult<Json<Value>> {
    let run_id = parse_object_id(&id)?;
    let run = state
        .db
        .command_runs()
        .find_one_and_update(
            build_confirm_filter(&admin.current_workspace, &run_id),
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
            doc! { "command_run_id": run_id, "workspace_id": &admin.current_workspace },
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
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let tools = merge_product_tools(mcp::list_tools_for_account(&state, &account_id).await?);
    Ok(Json(json!({ "tools": tools })))
}

pub(super) fn merge_product_tools(mut tools: Value) -> Value {
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
    let locked = extract_locked_send_content(instruction);
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

pub(super) fn extract_locked_send_content(instruction: &str) -> Option<String> {
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
    let (_, marker) = markers
        .iter()
        .filter_map(|marker| instruction.find(marker).map(|index| (index, *marker)))
        .min_by_key(|(index, _)| *index)?;
    let start = instruction.find(marker)? + marker.len();
    let mut text = instruction[start..].trim().to_string();
    if text.is_empty() {
        return None;
    }
    if let Some(quoted) = extract_quoted_text(&text) {
        text = quoted;
    } else {
        let stops = [
            "。这是",
            "。不需要",
            "。不要",
            "。请不要",
            "；这是",
            "；不需要",
            "；不要",
            "\n",
        ];
        if let Some(stop_index) = stops.iter().filter_map(|stop| text.find(stop)).min() {
            text.truncate(stop_index);
        }
    }
    let text = trim_wrapping_quotes(text.trim()).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
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
}

pub(super) fn tool_effect(tool_name: &str) -> ToolEffect {
    use ToolRisk::*;
    let risk = match tool_name {
        // 只读查询
        "account_list"
        | "contacts_search"
        | "knowledge.search"
        | "knowledge.list_catalog"
        | "wechatagent.search_contacts"
        | "wechatagent.query_runs"
        | "wechatagent.query_metrics"
        | "wechatagent.query_health"
        | "wechatagent.query_inbox" => Readonly,
        // 低风险可逆写
        "wechatagent.import_contacts"
        | "wechatagent.enable_contact_agent"
        | "wechatagent.disable_contact_agent"
        | "wechatagent.create_follow_up_task"
        | "wechatagent.update_contact_profile"
        | "wechatagent.update_operation_domain"
        | "wechatagent.set_assist_mode" => Low,
        // 高风险/宽影响（立即全量/改全局）
        "wechatagent.send_contact_message"
        | "wechatagent.publish_domain_profile"
        | "wechatagent.activate_domain_profile"
        | "wechatagent.publish_prompt_template"
        | "wechatagent.edit_state_machine"
        | "wechatagent.provider_activate"
        | "wechatagent.rollout_evolution_proposal"
        | "wechatagent.verify_knowledge_chunk"
        | "wechatagent.reject_knowledge_chunk" => Dangerous,
        // 不可逆（reset/delete/物理销毁）：档位高于 dangerous，第一期即便放权也保留确认
        "wechatagent.reset_domain"
        | "wechatagent.delete_knowledge_chunk"
        | "wechatagent.reset_system_pack" => Irreversible,
        // 只读前缀工具（knowledge.open*）
        other if other.starts_with("knowledge.open") => Readonly,
        // 未知（含 MCP 透传工具）：保守按 Low，read_only=false
        _ => Low,
    };
    let read_only = matches!(risk, Readonly);
    ToolEffect { read_only, risk }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ToolOutcome {
    Succeeded,
    Failed(String),
    Unverified(String),
}

/// 核实工具调用的"业务结果"——区别于"调用返回 Ok"。返回 Ok 不等于业务成功
/// （如 MCP send 返 Ok 但 success=false=账号离线）。无法判定的诚实标 Unverified，
/// 绝不假报成功（spec §3）。
pub(super) fn assert_tool_outcome(tool_name: &str, response: &Value) -> ToolOutcome {
    // MCP 发送类：核实 success + msgId
    if tool_name == "wechatagent.send_contact_message" {
        let success = response.get("success").and_then(Value::as_bool);
        match success {
            Some(true) => return ToolOutcome::Succeeded,
            Some(false) => {
                let err = response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP 返回 success=false");
                return ToolOutcome::Failed(err.to_string());
            }
            None => {
                return ToolOutcome::Unverified(
                    "MCP 响应无 success 字段，无法确认是否送达".to_string(),
                )
            }
        }
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

/// 基于真实执行结果生成汇报（spec §3.2：不回放 plan.summary，区分打算做与做成了什么）。
pub(super) fn build_execution_summary(results: &[(String, ToolOutcome)]) -> String {
    if results.is_empty() {
        return "没有需要执行的操作。".to_string();
    }
    let mut lines = Vec::new();
    for (tool, outcome) in results {
        match outcome {
            ToolOutcome::Succeeded => lines.push(format!("✅ {tool}：已完成")),
            ToolOutcome::Failed(why) => lines.push(format!("❌ {tool}：失败——{why}")),
            ToolOutcome::Unverified(why) => lines.push(format!("⚠️ {tool}：已执行待核实——{why}")),
        }
    }
    lines.join("\n")
}

/// verify 类工具：把 chunk 推向 verified 的动作。它写 source=Human（verify.rs:101），
/// 包成 AI 工具会"AI 调用被记成人确认"——故恒强制确认，不随第一期开关放行（spec §4.3）。
pub(super) fn tool_always_requires_confirmation(tool_name: &str) -> bool {
    matches!(tool_name, "wechatagent.verify_knowledge_chunk")
}

/// 第一期权限放大：dangerous_confirm_enabled 默认 false（见 spec §1.2），
/// 此时即便有 dangerous 工具也不强制确认，先跑通功能。开关为后续收紧预留。
/// 但 irreversible（reset/delete/销毁）+ verify 类（AI 永不自动 verify）无视开关
/// 恒需确认——第一期即便放权也保留（spec §4.2/§4.3）。
pub(super) fn plan_requires_confirmation(
    tool_names: &[&str],
    dangerous_confirm_enabled: bool,
) -> bool {
    tool_names.iter().any(|name| {
        let risk = tool_effect(name).risk;
        risk == ToolRisk::Irreversible
            || tool_always_requires_confirmation(name)
            || (dangerous_confirm_enabled && risk == ToolRisk::Dangerous)
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
                        upsert_contact_from_value(state, workspace_id, account_id, contact_value).await?
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
            let contact = resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            let playbook_id = planned.arguments.get("playbookId").and_then(Value::as_str);
            let playbook = resolve_playbook_for_contact(state, workspace_id, account_id, playbook_id).await?;
            let generated =
                agent::build_initial_operation_profile(state, workspace_id, &note, Some(&playbook)).await?;
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
            if !unset_doc.is_empty() {
                update_doc.insert("$unset", unset_doc);
            }
            state
                .db
                .contacts()
                .update_one(doc! { "_id": contact.id }, update_doc, None)
                .await?;
            let updated = find_contact_by_id(state, workspace_id, &contact.id.unwrap().to_hex()).await?;
            Ok(json!({ "item": ApiContact::from(updated) }))
        }
        "wechatagent.disable_contact_agent" => {
            let contact = resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            state
                .db
                .contacts()
                .update_one(
                    doc! { "_id": contact.id },
                    doc! { "$set": { "agent_status": "normal", "updated_at": DateTime::now() } },
                    None,
                )
                .await?;
            Ok(json!({ "ok": true }))
        }
        "wechatagent.create_follow_up_task" => {
            let contact = resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
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
            let contact = resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
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
            let contact = resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
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
                        "$unset": { "last_commitment": "" }
                    },
                    None,
                )
                .await?;
            Ok(json!({ "ok": true }))
        }
        _ => {
            // 兜底分支：只允许把 tools/list 真实公布过的工具名透传给生产 MCP。
            // 拦截 LLM 幻觉或提示注入产生的、服务端从未声明的工具名。
            if !advertised.contains(planned.tool_name.as_str()) {
                return Err(AppError::BadRequest(format!(
                    "tool '{}' is not advertised by the MCP server and is not a known product tool",
                    planned.tool_name
                )));
            }
            mcp::logged_call_for_account(
                state,
                account_id,
                &planned.tool_name,
                planned.arguments.clone(),
            )
            .await
        }
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

pub(super) async fn management_context(state: &AppState, workspace_id: &str, account_id: &str) -> AppResult<String> {
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
    Ok(format!(
        "当前账号: {}\n最近联系人:\n{}\n内容资产:\n{}",
        account_id,
        contact_lines.join("\n"),
        asset_lines.join("\n")
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
        prompts::load_prompt(
            &state.db,
            workspace_id,
            "management.plan.system",
        )
        .await?,
        prompts::load_prompt(
            &state.db,
            workspace_id,
            "management.plan.policy",
        )
        .await?
    );
    let user = format!(
        "操作员指令:\n{}\n\n当前系统上下文:\n{}\n\nMCP 工具目录:\n{}",
        instruction, context, tools
    );
    let value = agent::generate_agent_json(
        state,
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
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_effect_classifies_risk() {
        assert_eq!(tool_effect("wechatagent.search_contacts").risk, ToolRisk::Readonly);
        assert_eq!(tool_effect("wechatagent.create_follow_up_task").risk, ToolRisk::Low);
        assert_eq!(tool_effect("wechatagent.send_contact_message").risk, ToolRisk::Dangerous);
        assert_eq!(tool_effect("wechatagent.publish_domain_profile").risk, ToolRisk::Dangerous);
        assert_eq!(tool_effect("wechatagent.reset_domain").risk, ToolRisk::Irreversible);
        // 只读工具同时 read_only=true（与既有 dry-run 逻辑兼容）
        assert!(tool_effect("wechatagent.search_contacts").read_only);
    }

    #[test]
    fn confirmation_gate_off_by_default_phase1() {
        // 第一期权限放大：dangerous_confirm_enabled=false 时即便有 dangerous 工具也不强制确认
        assert!(!plan_requires_confirmation(&["wechatagent.send_contact_message"], false));
        // 开关打开后 dangerous 触发确认（为后续阶段预留）
        assert!(plan_requires_confirmation(&["wechatagent.send_contact_message"], true));
        // 全 readonly 永不需确认
        assert!(!plan_requires_confirmation(&["wechatagent.search_contacts"], true));
        // irreversible 无视开关恒需确认（第一期即便放权也保留，spec §4.2）
        assert!(plan_requires_confirmation(&["wechatagent.reset_domain"], false));
        // verify 类无视开关恒需确认（spec §4.3：AI 调 verify 会落 source=Human，
        // 守"AI 永不自动 verify"——确认门不随第一期 dangerous 开关放行）
        assert!(plan_requires_confirmation(&["wechatagent.verify_knowledge_chunk"], false));
    }

    #[test]
    fn outcome_assertion_detects_business_failure() {
        use serde_json::json;
        // send: MCP RPC 返 Ok 但 success=false（账号离线）→ Failed
        let r = json!({"success": false, "error": "account offline"});
        assert!(matches!(assert_tool_outcome("wechatagent.send_contact_message", &r), ToolOutcome::Failed(_)));
        // send: success=true 且有 msgId → Succeeded
        let r = json!({"success": true, "msgId": "m123"});
        assert!(matches!(assert_tool_outcome("wechatagent.send_contact_message", &r), ToolOutcome::Succeeded));
        // update: matched=0 → Failed（未命中、实际没改）
        let r = json!({"matched": 0, "modified": 0});
        assert!(matches!(assert_tool_outcome("wechatagent.update_contact_profile", &r), ToolOutcome::Failed(_)));
        // update: modified>=1 → Succeeded
        let r = json!({"matched": 1, "modified": 1});
        assert!(matches!(assert_tool_outcome("wechatagent.update_contact_profile", &r), ToolOutcome::Succeeded));
        // 无断言规则的工具 + response 无明显信号 → Unverified（诚实暴露）
        let r = json!({"weird": "shape"});
        assert!(matches!(assert_tool_outcome("wechatagent.some_unknown_tool", &r), ToolOutcome::Unverified(_)));
        // readonly 查询：有数据即 Succeeded
        let r = json!({"items": []});
        assert!(matches!(assert_tool_outcome("wechatagent.query_runs", &r), ToolOutcome::Succeeded));
    }

    #[test]
    fn execution_summary_reports_real_outcomes() {
        let results = vec![
            ("wechatagent.update_contact_profile".to_string(), ToolOutcome::Succeeded),
            ("wechatagent.send_contact_message".to_string(), ToolOutcome::Failed("账号离线".to_string())),
        ];
        let s = build_execution_summary(&results);
        assert!(s.contains("update_contact_profile"));
        assert!(s.contains("失败") || s.contains("账号离线"));
        // 不假报全部成功
        assert!(!s.contains("全部成功"));

        let unv = vec![("wechatagent.x".to_string(), ToolOutcome::Unverified("无法确认".to_string()))];
        let s2 = build_execution_summary(&unv);
        assert!(s2.contains("待核实") || s2.contains("无法确认"));
    }

    #[test]
    fn management_machine_write_drops_out_of_dict_stage_not_reject() {
        // T8 旁路修复守护：management update_contact_profile 走 AI/MCP 通道 → MachineWrite。
        // customer_stage 主通道是 LlmSignals（机器容错）：字典有条目但此值越界(Miss) →
        // classify_validation 判 DropSilently（不阻断、不报错），apply_admin_dim_validation
        // 再把 DropSilently 映成 None → 该键不写入（越界值不落库脏值，且不像 admin 那样 400）。
        use crate::agent::dimension_registry::{classify_validation, spec_for, DictLookup, WriteIntent};
        let stage = spec_for("customer_stage").unwrap();
        let v = classify_validation(stage, DictLookup::Miss, "臆造态", WriteIntent::MachineWrite);
        // 旁路修复前：值原样落库；修复后：MachineWrite 越界 → DropSilently → 不写。
        assert!(matches!(apply_admin_dim_validation(v), Ok(None)));

        // intent_level 同为 LlmSignals 机器通道，越界同样 drop（不写）而非报错。
        let intent = spec_for("intent_level").unwrap();
        let vi = classify_validation(intent, DictLookup::Miss, "瞎填意向", WriteIntent::MachineWrite);
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
    fn locked_send_content_stops_before_operator_instruction() {
        let instruction = "请给 Jsjm 发送一条真实微信文本消息，内容必须完全等于：Jsjm，测试一下 kefu-b 的用户运营 Agent 真实发送链路。收到不用回复。。这是 kefu-b 的发送链路验收，不需要二次确认。";
        assert_eq!(
            extract_locked_send_content(instruction).as_deref(),
            Some("Jsjm，测试一下 kefu-b 的用户运营 Agent 真实发送链路。收到不用回复。")
        );
    }

    #[test]
    fn locked_send_content_prefers_quoted_body() {
        let instruction = "发送内容：\"只发送这一句。\" 不要创建跟进任务。";
        assert_eq!(
            extract_locked_send_content(instruction).as_deref(),
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
        apply_locked_send_content(&mut plan, "内容必须完全等于：原文消息。不要追加说明", false)
            .unwrap();
        let args = plan.tool_calls[0].arguments.as_object().unwrap();
        assert_eq!(
            args.get("content").and_then(Value::as_str),
            Some("原文消息")
        );
        assert_eq!(
            args.get("originalContentLocked").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn dry_run_keeps_read_tools_live_and_blocks_write_tools() {
        assert!(!should_dry_run_tool("wechatagent.search_contacts", true));
        assert!(!should_dry_run_tool("knowledge.open", true));
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
        assert!(names.contains("message_send_text"));
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
        assert!(names.contains("message_send_text"));
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
        let filter = build_confirm_filter("workspace1", &mongodb::bson::oid::ObjectId::new());
        // filter 必须含 status: pending_confirmation（防二次确认 / 防确认非待确认命令）
        // 且带 workspace_id（IDOR：不能跨 workspace 确认他人命令）
        assert_eq!(filter.get_str("workspace_id").unwrap(), "workspace1");
        assert_eq!(filter.get_str("status").unwrap(), "pending_confirmation");
    }

    #[test]
    fn tool_call_status_closed_set() {
        use crate::models::ALLOWED_TOOL_CALL_STATUS;
        const EXPECTED: &[&str] = &["running", "dry_run", "succeeded", "failed", "executed_unverified"];
        for s in EXPECTED {
            assert!(ALLOWED_TOOL_CALL_STATUS.contains(s), "缺少状态 {s}");
        }
        assert_eq!(ALLOWED_TOOL_CALL_STATUS.len(), 5);
    }

    #[test]
    fn assert_tool_call_status_accepts_all_valid() {
        for s in ["running", "dry_run", "succeeded", "failed", "executed_unverified"] {
            crate::models::assert_tool_call_status_valid(s); // 不 panic 即通过
        }
    }
}

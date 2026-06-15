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
            doc! { "_id": session_id, "account_id": &payload.account_id },
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
    let mut calls = Vec::new();
    let mut failed = None;
    let requires_confirmation =
        plan.requires_confirmation || plan.risk_level.eq_ignore_ascii_case("dangerous");
    for planned in plan
        .tool_calls
        .iter()
        .take(if requires_confirmation { 0 } else { 12 })
    {
        let arguments_doc = to_document(&planned.arguments).unwrap_or_else(|_| Document::new());
        let call_start = AgentToolCall {
            id: None,
            workspace_id: admin.current_workspace.clone(),
            account_id: payload.account_id.clone(),
            command_run_id: run_id,
            tool_name: planned.tool_name.clone(),
            arguments: arguments_doc.clone(),
            status: if should_dry_run_tool(&planned.tool_name, effective_dry_run) {
                "dry_run".to_string()
            } else {
                "running".to_string()
            },
            response: None,
            error: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        let call_result = state.db.tool_calls().insert_one(call_start, None).await?;
        let call_id = call_result
            .inserted_id
            .as_object_id()
            .ok_or_else(|| AppError::External("tool call id missing".to_string()))?;
        let result =
            execute_management_tool(&state, &admin.current_workspace, &payload.account_id, planned, effective_dry_run, &advertised_tools).await;
        let succeeded_status = if should_dry_run_tool(&planned.tool_name, effective_dry_run) {
            "dry_run"
        } else {
            "succeeded"
        };
        match result {
            Ok(response) => {
                let response_doc = to_document(&response).ok();
                state
                    .db
                    .tool_calls()
                    .update_one(
                        doc! { "_id": call_id },
                        doc! {
                            "$set": {
                                "status": succeeded_status,
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
                    "status": succeeded_status,
                    "response": response
                }));
            }
            Err(error) => {
                let message = error.to_string();
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

#[derive(Debug, Clone, Copy)]
pub(super) struct ToolEffect {
    read_only: bool,
}

pub(super) fn tool_effect(tool_name: &str) -> ToolEffect {
    let read_only = matches!(
        tool_name,
        "account_list"
            | "contacts_search"
            | "knowledge.search"
            | "knowledge.list_catalog"
            | "wechatagent.search_contacts"
    ) || tool_name.starts_with("knowledge.open");
    ToolEffect { read_only }
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
                agent::build_initial_operation_profile(state, &note, Some(&playbook)).await?;
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
                "tags": generated.tags,
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
                insert_domain_stage_fields(
                    &mut set_doc,
                    generated.customer_stage.as_deref(),
                    generated.intent_level.as_deref(),
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
            let tags = planned
                .arguments
                .get("tags")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let profile_attributes = planned
                .arguments
                .get("profileAttributes")
                .or_else(|| planned.arguments.get("profile_attributes"))
                .and_then(|value| to_document(value).ok())
                .unwrap_or_default();
            let new_stage = optional_value_arg(&planned.arguments, "customerStage")
                .or_else(|| optional_value_arg(&planned.arguments, "customer_stage"));
            let prev_stage = contact
                .domain_attributes
                .as_ref()
                .and_then(|d| d.get_str("customer_stage").ok().map(|s| s.to_string()));
            let stage_changed = prev_stage.as_deref() != new_stage.as_deref();
            let new_commitment_text = optional_value_arg(&planned.arguments, "lastCommitment")
                .or_else(|| optional_value_arg(&planned.arguments, "last_commitment"));
            let commitments_bson = commitments_with_optional_text(
                &contact.commitments,
                new_commitment_text.as_deref(),
            );
            let mut set_doc = doc! {
                "tags": tags,
                "commitments": commitments_bson,
                "follow_up_policy": optional_value_arg(&planned.arguments, "followUpPolicy")
                    .or_else(|| optional_value_arg(&planned.arguments, "follow_up_policy")),
                "profile_attributes": profile_attributes,
                "profile_updated_at": DateTime::now(),
                "updated_at": DateTime::now(),
            };
            let new_intent = optional_value_arg(&planned.arguments, "intentLevel")
                .or_else(|| optional_value_arg(&planned.arguments, "intent_level"));
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
}

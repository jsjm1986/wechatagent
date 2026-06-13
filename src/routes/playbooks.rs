//! 运营 Playbook 路由：方法论模板的增删改查及自动生成。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::{FindOneOptions, FindOptions},
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationPlaybookQuery {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationPlaybookRequest {
    account_id: Option<String>,
    name: String,
    description: Option<String>,
    method_prompt: String,
    profile_method: Option<String>,
    tag_method: Option<String>,
    stage_method: Option<String>,
    intent_method: Option<String>,
    follow_up_method: Option<String>,
    reply_style: Option<String>,
    forbidden_rules: Option<String>,
    success_criteria: Option<String>,
    is_default: Option<bool>,
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
    instruction: String,
}

pub(super) async fn list_operation_playbooks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<OperationPlaybookQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
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
    validate_playbook_input(&payload.name, &payload.method_prompt)?;
    let is_default = payload.is_default.unwrap_or(false)
        || state
            .db
            .operation_playbooks()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &account_id
                },
                None,
            )
            .await?
            .is_none();
    if is_default {
        unset_default_playbooks(&state, &admin.current_workspace, &account_id).await?;
    }
    let playbook = OperationPlaybook {
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
        is_default,
        version: 1,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state
        .db
        .operation_playbooks()
        .insert_one(playbook, None)
        .await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

pub(super) async fn update_operation_playbook(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OperationPlaybookRequest>,
) -> AppResult<Json<Value>> {
    validate_playbook_input(&payload.name, &payload.method_prompt)?;
    let object_id = parse_object_id(&id)?;
    let existing = state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation playbook not found".to_string()))?;
    if payload.is_default.unwrap_or(existing.is_default) {
        unset_default_playbooks(&state, &admin.current_workspace, &existing.account_id).await?;
    }
    state
        .db
        .operation_playbooks()
        .update_one(
            doc! { "_id": object_id },
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
                    "is_default": payload.is_default.unwrap_or(existing.is_default),
                    "version": existing.version + 1,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn set_default_operation_playbook(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let playbook = state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation playbook not found".to_string()))?;
    unset_default_playbooks(&state, &admin.current_workspace, &playbook.account_id).await?;
    state
        .db
        .operation_playbooks()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "is_default": true,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
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
            .await;
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
        Some(&payload.account_id),
        None,
        None,
        "playbook.generator",
        &system,
        &user,
    )
    .await?;
    let exists = state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &payload.account_id
            },
            None,
        )
        .await?
        .is_some();
    let is_default = !exists;
    if is_default {
        unset_default_playbooks(&state, &admin.current_workspace, &payload.account_id).await?;
    }
    let playbook = OperationPlaybook {
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
        is_default,
        version: 1,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state
        .db
        .operation_playbooks()
        .insert_one(playbook, None)
        .await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
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
    let object_id = parse_object_id(&id)?;
    let existing = state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation playbook not found".to_string()))?;
    let system = prompts::load_prompt(
        &state.db,
        &admin.current_workspace,
        "playbook.generator.system",
    )
    .await?;
    // C3：active profile 可声明行业专属生成器引导语,覆盖领域中性 DEFAULT(去销售偏见)。
    let active_profile =
        agent::domain_profile::load_active_domain_profile(&state.db, &admin.current_workspace)
            .await;
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
        Some(&existing.account_id),
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
    state
        .db
        .operation_playbooks()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "name": name,
                    "description": description,
                    "method_prompt": method_prompt,
                    "profile_method": profile_method,
                    "tag_method": tag_method,
                    "stage_method": stage_method,
                    "intent_method": intent_method,
                    "follow_up_method": follow_up_method,
                    "reply_style": reply_style,
                    "forbidden_rules": forbidden_rules,
                    "success_criteria": success_criteria,
                    "created_by": "agent_optimized",
                    "version": existing.version + 1,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    let updated = state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation playbook not found".to_string()))?;
    Ok(Json(json!({ "item": playbook_json(updated) })))
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

pub(super) async fn ensure_default_playbook(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<OperationPlaybook> {
    if let Some(playbook) = state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "is_default": true
            },
            FindOneOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .build(),
        )
        .await?
    {
        return Ok(playbook);
    }
    let playbook = prompts::default_playbook(workspace_id, account_id);
    let result = state
        .db
        .operation_playbooks()
        .insert_one(playbook, None)
        .await?;
    let id = result
        .inserted_id
        .as_object_id()
        .ok_or_else(|| AppError::External("operation playbook id missing".to_string()))?;
    state
        .db
        .operation_playbooks()
        .find_one(doc! { "_id": id }, None)
        .await?
        .ok_or_else(|| AppError::External("operation playbook not found after insert".to_string()))
}

pub(super) async fn unset_default_playbooks(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<()> {
    state
        .db
        .operation_playbooks()
        .update_many(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "is_default": true
            },
            doc! { "$set": { "is_default": false, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    Ok(())
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

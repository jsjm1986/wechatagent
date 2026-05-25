//! Reply Agent 主决策入口 (`decide_reply`)。
//!
//! 该模块负责构造 `user.reply.task` prompt，注入运营方法、状态机、
//! 知识切片、长期记忆、最近聊天等上下文，调用 LLM 生成 [`AgentDecision`]。
//! 同时承载 [`build_initial_operation_profile`]：根据运营人员的人工备注
//! 给联系人生成初始运营画像。
//!
//! 所有 prompt 加载、上下文格式化、调用 LLM 都集中在这里；其它子模块
//! 通过 `pub(crate)` 调用 `decide_reply` 复用同一份 prompt 渲染逻辑。

use mongodb::bson::{to_document, Document};

use crate::error::{AppError, AppResult};
use crate::models::{
    AgentProfile, Contact, ConversationMessage, MessageDirection, OperatingMemory,
    OperationDomainConfig, OperationKnowledgeChunk, OperationPlaybook,
};
use crate::prompts;
use crate::routes::AppState;

use super::generate_agent_json;
use super::knowledge_router::format_operation_knowledge_for_prompt;
use super::runtime::UserRuntimeParameters;
use super::types::{
    optional_string, string_array, AgentDecision, GeneratedOperationProfile, KnowledgeRouteResult,
    RawAgentDecision,
};
use crate::models::AgentTask;

pub async fn build_initial_operation_profile(
    state: &AppState,
    note: &str,
    playbook: Option<&OperationPlaybook>,
) -> AppResult<GeneratedOperationProfile> {
    let playbook_text = playbook.map(format_playbook_for_prompt).unwrap_or_else(|| {
        "未配置运营方法。请根据运营备注自由生成克制、真实、可执行的运营画像。".to_string()
    });
    let domain_config =
        load_user_operation_domain_config(state, &state.config.default_workspace_id).await?;
    let domain_text = domain_config
        .as_ref()
        .map(format_operation_domain_config_for_prompt)
        .unwrap_or_default();
    let system = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.initial_profile.system",
    )
    .await?;
    let task_template = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.initial_profile.task",
    )
    .await?;
    let user = format!(
        r#"{}

运营方法：
{}

用户运营域策略：
{}

运营人员描述：
{}"#,
        task_template, playbook_text, domain_text, note
    );
    let value = generate_agent_json(
        state,
        None,
        None,
        None,
        "user.initial_profile.task",
        &system,
        &user,
    )
    .await?;
    let profile_value = value
        .get("agentProfile")
        .or_else(|| value.get("agent_profile"))
        .cloned()
        .unwrap_or_else(|| value.clone());
    let agent_profile = AgentProfile {
        summary: profile_value
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or(note)
            .to_string(),
        interests: profile_value
            .get("interests")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        communication_style: profile_value
            .get("communicationStyle")
            .or_else(|| profile_value.get("communication_style"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        operation_goal: profile_value
            .get("operationGoal")
            .or_else(|| profile_value.get("operation_goal"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    };
    Ok(GeneratedOperationProfile {
        agent_profile,
        tags: string_array(&value, "tags"),
        customer_stage: optional_string(&value, "customerStage")
            .or_else(|| optional_string(&value, "customer_stage")),
        intent_level: optional_string(&value, "intentLevel")
            .or_else(|| optional_string(&value, "intent_level")),
        last_commitment: optional_string(&value, "lastCommitment")
            .or_else(|| optional_string(&value, "last_commitment")),
        follow_up_policy: optional_string(&value, "followUpPolicy")
            .or_else(|| optional_string(&value, "follow_up_policy")),
        profile_attributes: value
            .get("profileAttributes")
            .or_else(|| value.get("profile_attributes"))
            .and_then(|item| to_document(item).ok())
            .unwrap_or_default(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn decide_reply(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    recent_messages: &[ConversationMessage],
    pending_tasks: &[AgentTask],
    playbook: Option<&OperationPlaybook>,
    domain_config: Option<&OperationDomainConfig>,
    runtime: &UserRuntimeParameters,
    memory: &OperatingMemory,
    context_pack: &Document,
    knowledge_chunks: &[OperationKnowledgeChunk],
    knowledge_route: &KnowledgeRouteResult,
    rewrite_instruction: Option<&str>,
    run_id: Option<&str>,
) -> AppResult<AgentDecision> {
    let (decision, _risks) = decide_reply_with_promote(
        state,
        contact,
        inbound,
        recent_messages,
        pending_tasks,
        playbook,
        domain_config,
        runtime,
        memory,
        context_pack,
        knowledge_chunks,
        knowledge_route,
        rewrite_instruction,
        run_id,
    )
    .await?;
    Ok(decision)
}

/// agent-autonomy-loop W2 / Task 3.4：与 [`decide_reply`] 相同上下文与 prompt，
/// 但额外返回 [`RawAgentDecision::validate_and_promote`] 聚合的协议违规标签
/// （`promote_risks`），供 gateway 主路径在 `finalize_review_for_send` 阶段
/// 把"missing_required_field / invalid_enum_value / invalid_type /
/// decision_phase_invalid / insufficient_detail_in_critical_turn"等等聚合进
/// `review.risks` 并按 R3.5 / R3.6 走 blocked_by_required_field 路径。
///
/// 单纯 `decide_reply` 把 promote_risks 默默丢掉以保持 simulation /
/// management_send 等老入口的二元接口；新链路（task 3.4 之后）SHALL 直接调
/// 本函数把 risks 透传给 gateway 主流程。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn decide_reply_with_promote(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    recent_messages: &[ConversationMessage],
    pending_tasks: &[AgentTask],
    playbook: Option<&OperationPlaybook>,
    domain_config: Option<&OperationDomainConfig>,
    runtime: &UserRuntimeParameters,
    memory: &OperatingMemory,
    context_pack: &Document,
    knowledge_chunks: &[OperationKnowledgeChunk],
    knowledge_route: &KnowledgeRouteResult,
    rewrite_instruction: Option<&str>,
    run_id: Option<&str>,
) -> AppResult<(AgentDecision, Vec<String>)> {
    let soul = load_published_soul(state, "user").await?.unwrap_or_else(|| {
        "你是长期运行的微信私域运营 AI Agent。你只为已纳管好友服务，目标是自然、克制、持续推进关系和业务目标。".to_string()
    });
    let assets = load_context_assets(state, &contact.account_id).await?;
    let playbook_text = playbook.map(format_playbook_for_prompt).unwrap_or_else(|| {
        "未配置运营方法。按用户备注、聊天上下文和内容资产自由判断。".to_string()
    });
    let domain_text = domain_config
        .map(format_operation_domain_config_for_prompt)
        .unwrap_or_default();
    let state_machine_text = domain_config
        .map(format_operation_state_machine_for_prompt)
        .unwrap_or_default();
    let runtime_text = serde_json::to_string(&runtime.as_document()).unwrap_or_default();
    let knowledge_text =
        format_operation_knowledge_for_prompt(knowledge_chunks);
    let knowledge_route_text = serde_json::to_string(knowledge_route).unwrap_or_default();
    // agent-autonomy-loop W5 / Task 6.5：注入最近 K=5 条 deprecated_facts，
    // 让 Reply Agent 知道哪些事实已过期，避免再次引用。仅传 id / text /
    // deprecation_reason / deprecated_at，按 deprecated_at 降序。
    let deprecated_facts_recent: Vec<serde_json::Value> = {
        let mut entries: Vec<&crate::models::MemoryFact> = memory
            .memory_card
            .deprecated_facts
            .iter()
            .filter_map(|repr| match repr {
                crate::models::MemoryFactRepr::Structured(f) => Some(f),
                _ => None,
            })
            .collect();
        entries.sort_by(|a, b| {
            let a_at = a.deprecated_at.map(|d| d.timestamp_millis()).unwrap_or(0);
            let b_at = b.deprecated_at.map(|d| d.timestamp_millis()).unwrap_or(0);
            b_at.cmp(&a_at)
        });
        entries
            .into_iter()
            .take(5)
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "text": f.text,
                    "deprecation_reason": f.deprecation_reason,
                    "deprecated_at": f.deprecated_at.map(|d| d.timestamp_millis()),
                })
            })
            .collect()
    };
    let memory_text = serde_json::to_string(&mongodb::bson::doc! {
        "memoryCard": context_pack.clone(),
        "userUnderstanding": memory.user_understanding.clone(),
        "relationshipState": memory.relationship_state.clone(),
        "productFit": memory.product_fit.clone(),
        "nextAction": memory.next_action.clone()
    })
    .unwrap_or_default();
    let memory_card_text = serde_json::to_string(context_pack).unwrap_or_default();
    let rewrite_text = rewrite_instruction.unwrap_or("");
    let system_contract = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.reply.system",
    )
    .await?;
    let policy = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.reply.policy",
    )
    .await?;
    let task_template = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.reply.task",
    )
    .await?;
    // R-prompt-v3：Operator Instruction 层（最高优先级）。运营人员可在后台对
    // 单个联系人写一段 ≤ 1000 字的特别指令，覆盖 Soul + Policy 的默认人格判定
    // （如"老客户已签约，不要主动推销"、"这个客户技术背景，可以多用术语"）。
    // 末位注入是为了利用 LLM 的近端注意力优势（recency bias）—— 系统消息越靠后
    // 的指令权重越高。
    let operator_instruction = contact
        .custom_agent_instructions
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            format!(
                "\n\n# 运营关于本联系人的特别指令（最高优先级，覆盖 Soul + Policy）\n{}\n\n上述指令来自运营，必须遵守；与 Soul / Policy 冲突时以本指令为准。",
                s
            )
        })
        .unwrap_or_default();
    let system = format!(
        "{}\n\n{}\n\n{}{}",
        soul, system_contract, policy, operator_instruction
    );
    let history = recent_messages
        .iter()
        .rev()
        .map(|message| {
            let speaker = match message.direction {
                MessageDirection::Inbound => "客户",
                MessageDirection::Outbound => "我方",
            };
            format!("{speaker}: {}", message.content)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let task_text = pending_tasks
        .iter()
        .map(|task| format!("{} @ {:?}", task.content, task.run_at))
        .collect::<Vec<_>>()
        .join("\n");
    let user = format!(
        r#"{}

当前运营方法:
{}

用户运营域策略:
{}

运营状态机:
{}

硬运行参数:
{}

长期运营记忆:
{}

长期记忆卡片:
{}

最近 5 条已弃用记忆（不要再引用，仅供识别变化）:
{}

产品知识:
{}

知识路由:
{}

改写要求:
{}

客户 wxid: {}
客户昵称: {}
人工描述: {}
当前画像: {}
长期记忆: {}
标签: {}
客户阶段: {}
意向等级: {}
最近承诺: {}
跟进策略: {}
自由画像字段: {}
可引用内容资产:
{}
未完成跟进:
{}

最近聊天:
{}

最新消息:
{}"#,
        task_template,
        playbook_text,
        domain_text,
        state_machine_text,
        runtime_text,
        memory_text,
        memory_card_text,
        serde_json::to_string(&deprecated_facts_recent).unwrap_or_default(),
        knowledge_text,
        knowledge_route_text,
        rewrite_text,
        contact.wxid,
        contact.nickname.clone().unwrap_or_default(),
        contact.human_profile_note.clone().unwrap_or_default(),
        serde_json::to_string(&contact.agent_profile).unwrap_or_default(),
        contact.memory_summary.clone().unwrap_or_default(),
        contact.tags.join(", "),
        contact
            .domain_attributes
            .as_ref()
            .and_then(|doc| doc.get_str("customer_stage").ok().map(|s| s.to_string()))
            .unwrap_or_default(),
        contact
            .domain_attributes
            .as_ref()
            .and_then(|doc| doc.get_str("intent_level").ok().map(|s| s.to_string()))
            .unwrap_or_default(),
        contact
            .commitments
            .last()
            .map(|c| c.text().to_string())
            .unwrap_or_default(),
        contact.follow_up_policy.clone().unwrap_or_default(),
        serde_json::to_string(&contact.profile_attributes).unwrap_or_default(),
        assets,
        task_text,
        history,
        inbound.content
    );

    let value = generate_agent_json(
        state,
        Some(&contact.account_id),
        Some(&contact.wxid),
        run_id,
        "user.reply.task",
        &system,
        &user,
    )
    .await?;
    // agent-autonomy-loop W1 task 2.3 / W2 task 3.4：先反序列化为
    // [`RawAgentDecision`]（Option<T> 边界结构），再调
    // `validate_and_promote(runtime)` 落到业务结构 [`AgentDecision`] 并
    // 聚合协议违规标签（`missing_required_field:* / invalid_enum_value:* /
    // invalid_type:* / decision_phase_invalid:* /
    // insufficient_detail_in_critical_turn:*`）。risks 由调用方在
    // `finalize_review_for_send` 阶段消费。
    let raw: RawAgentDecision = serde_json::from_value(value).map_err(AppError::from)?;
    let (decision, promote_risks) = raw.validate_and_promote(runtime);
    Ok((decision, promote_risks))
}

pub async fn load_operation_playbook_for_contact(
    state: &AppState,
    contact: &Contact,
) -> AppResult<Option<OperationPlaybook>> {
    use mongodb::bson::doc;
    use mongodb::options::FindOneOptions;
    if let Some(id) = contact.playbook_id {
        if let Some(playbook) = state
            .db
            .operation_playbooks()
            .find_one(
                doc! {
                    "_id": id,
                    "workspace_id": &contact.workspace_id,
                    "account_id": &contact.account_id
                },
                None,
            )
            .await?
        {
            return Ok(Some(playbook));
        }
    }
    state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "is_default": true
            },
            FindOneOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .build(),
        )
        .await
        .map_err(AppError::from)
}

pub(crate) async fn load_user_operation_domain_config(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<Option<OperationDomainConfig>> {
    use mongodb::bson::doc;
    state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations"
            },
            None,
        )
        .await
        .map_err(AppError::from)
}

pub(crate) fn format_operation_domain_config_for_prompt(config: &OperationDomainConfig) -> String {
    format!(
        r#"名称: {}
目标: {}
方法论: {}
工作流: {}
工具边界: {}
自动化策略: {}
复盘规则: {}
运行参数: {}"#,
        config.name,
        config.goal,
        config.methodology,
        config.workflow,
        config.tool_policy,
        config.automation_policy,
        config.review_policy,
        serde_json::to_string(&config.runtime_parameters).unwrap_or_default()
    )
}

pub(crate) fn format_operation_state_machine_for_prompt(config: &OperationDomainConfig) -> String {
    serde_json::to_string(&config.state_machine).unwrap_or_default()
}

pub(crate) fn format_playbook_for_prompt(playbook: &OperationPlaybook) -> String {
    format!(
        r#"名称: {}
描述: {}
总方法: {}
画像方法: {}
标签方法: {}
阶段方法: {}
意向方法: {}
跟进方法: {}
回复风格: {}
禁用规则: {}
成功标准: {}
版本: {}"#,
        playbook.name,
        playbook.description.clone().unwrap_or_default(),
        playbook.method_prompt,
        playbook.profile_method.clone().unwrap_or_default(),
        playbook.tag_method.clone().unwrap_or_default(),
        playbook.stage_method.clone().unwrap_or_default(),
        playbook.intent_method.clone().unwrap_or_default(),
        playbook.follow_up_method.clone().unwrap_or_default(),
        playbook.reply_style.clone().unwrap_or_default(),
        playbook.forbidden_rules.clone().unwrap_or_default(),
        playbook.success_criteria.clone().unwrap_or_default(),
        playbook.version
    )
}

pub(crate) async fn load_published_soul(
    state: &AppState,
    agent_kind: &str,
) -> AppResult<Option<String>> {
    use mongodb::bson::doc;
    use mongodb::options::FindOneOptions;
    let soul = state
        .db
        .agent_souls()
        .find_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "agent_kind": agent_kind,
                "status": "published"
            },
            FindOneOptions::builder()
                .sort(doc! { "version": -1, "updated_at": -1 })
                .build(),
        )
        .await?;
    Ok(soul.map(|item| item.content))
}

pub(crate) async fn load_context_assets(state: &AppState, account_id: &str) -> AppResult<String> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use mongodb::options::FindOptions;
    let mut cursor = state
        .db
        .content_assets()
        .find(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "$or": [
                    { "account_id": null },
                    { "account_id": account_id }
                ],
                "kind": { "$in": ["text", "faq", "script", "brand_voice", "forbidden_expression"] }
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(12)
                .build(),
        )
        .await?;
    let mut lines = Vec::new();
    while let Some(asset) = cursor.try_next().await? {
        lines.push(format!(
            "- [{}] {}: {}",
            asset.kind,
            asset.title,
            asset.body.unwrap_or_default()
        ));
    }
    Ok(lines.join("\n"))
}

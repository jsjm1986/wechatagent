use futures::TryStreamExt;
use mongodb::{
    bson::{doc, to_document, DateTime, Document},
    options::FindOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    error::{AppError, AppResult},
    mcp,
    models::{
        AgentEvent, AgentProfile, AgentStatus, AgentTask, Contact, ConversationMessage,
        MessageDirection,
    },
    routes::AppState,
};

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentDecision {
    #[serde(default)]
    pub should_reply: bool,
    #[serde(default)]
    pub reply_text: String,
    #[serde(default)]
    pub profile_update: Option<AgentProfile>,
    #[serde(default)]
    pub memory_update: String,
    #[serde(default)]
    pub follow_up: Option<FollowUpDecision>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FollowUpDecision {
    #[serde(default)]
    pub needed: bool,
    #[serde(default)]
    pub run_at: String,
    #[serde(default)]
    pub content: String,
}

pub async fn build_initial_profile(state: &AppState, note: &str) -> AppResult<AgentProfile> {
    let system = "你是微信私域运营画像助手。只输出 JSON，不输出 markdown。";
    let user = format!(
        r#"根据运营人员描述，生成一个客户运营画像 JSON。
字段必须是：
{{
  "summary": "一句话客户画像",
  "interests": ["兴趣1"],
  "communicationStyle": "沟通风格",
  "operationGoal": "运营目标"
}}

运营人员描述：
{}"#,
        note
    );
    let value = state.llm.generate_json(system, &user).await?;
    Ok(AgentProfile {
        summary: value
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or(note)
            .to_string(),
        interests: value
            .get("interests")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        communication_style: value
            .get("communicationStyle")
            .or_else(|| value.get("communication_style"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        operation_goal: value
            .get("operationGoal")
            .or_else(|| value.get("operation_goal"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

pub async fn handle_managed_message(
    state: &AppState,
    contact: Contact,
    inbound: &ConversationMessage,
) -> AppResult<()> {
    if contact.agent_status != AgentStatus::Managed {
        return Ok(());
    }

    if recently_replied(state, &contact).await? {
        write_event(
            state,
            Some(&contact.wxid),
            "agent_skipped",
            "skipped",
            "短时间内已回复，跳过本次自动回复",
            None,
        )
        .await?;
        return Ok(());
    }

    let recent_messages = load_recent_messages(state, &contact).await?;
    let pending_tasks = load_pending_tasks(state, &contact).await?;
    let decision = decide_reply(state, &contact, inbound, &recent_messages, &pending_tasks).await?;

    if decision.should_reply && !decision.reply_text.trim().is_empty() {
        let response = mcp::logged_call(
            state,
            "message_send_text",
            json!({
                "recipient": contact.wxid,
                "content": decision.reply_text
            }),
        )
        .await?;
        state
            .db
            .messages()
            .insert_one(
                ConversationMessage {
                    id: None,
                    workspace_id: contact.workspace_id.clone(),
                    account_id: contact.account_id.clone(),
                    contact_wxid: contact.wxid.clone(),
                    message_id: response
                        .get("newMsgId")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    direction: MessageDirection::Outbound,
                    content: decision.reply_text.clone(),
                    raw: to_document(&response).ok(),
                    created_at: DateTime::now(),
                },
                None,
            )
            .await?;
    }

    apply_agent_updates(state, &contact, &decision).await?;
    write_event(
        state,
        Some(&contact.wxid),
        "agent_reply",
        "success",
        if decision.should_reply {
            "Agent 已生成并发送回复"
        } else {
            "Agent 判断无需回复"
        },
        to_document(&decision).ok(),
    )
    .await?;
    Ok(())
}

async fn decide_reply(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    recent_messages: &[ConversationMessage],
    pending_tasks: &[AgentTask],
) -> AppResult<AgentDecision> {
    let system = "你是长期运行的微信私域运营 AI Agent。只输出严格 JSON，不输出 markdown。你只为已纳管好友服务，目标是自然、克制、持续推进关系和业务目标。";
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
        r#"请基于以下上下文生成运营决策 JSON：
{{
  "shouldReply": true,
  "replyText": "要发送给客户的微信文本，口吻自然，不要暴露系统或AI",
  "profileUpdate": {{
    "summary": "更新后的一句话客户画像",
    "interests": ["兴趣"],
    "communicationStyle": "沟通风格",
    "operationGoal": "运营目标"
  }},
  "memoryUpdate": "需要写入长期记忆的摘要",
  "followUp": {{
    "needed": false,
    "runAt": "",
    "content": ""
  }}
}}

规则：
- 不要编造已成交、价格、承诺、身份等事实。
- 回复要短，像真人微信，不要营销腔。
- 如果客户只是表情、寒暄或无需回复，可以 shouldReply=false。
- followUp.runAt 如需要，使用 RFC3339 UTC 时间。

客户 wxid: {}
客户昵称: {}
人工描述: {}
当前画像: {}
长期记忆: {}
未完成跟进:
{}

最近聊天:
{}

最新消息:
{}"#,
        contact.wxid,
        contact.nickname.clone().unwrap_or_default(),
        contact.human_profile_note.clone().unwrap_or_default(),
        serde_json::to_string(&contact.agent_profile).unwrap_or_default(),
        contact.memory_summary.clone().unwrap_or_default(),
        task_text,
        history,
        inbound.content
    );

    let value = state.llm.generate_json(system, &user).await?;
    serde_json::from_value(value).map_err(AppError::from)
}

async fn apply_agent_updates(
    state: &AppState,
    contact: &Contact,
    decision: &AgentDecision,
) -> AppResult<()> {
    let mut set_doc = doc! {
        "updated_at": DateTime::now(),
        "last_agent_run_at": DateTime::now(),
    };

    if let Some(profile) = &decision.profile_update {
        set_doc.insert("agent_profile", to_document(profile)?);
    }
    if !decision.memory_update.trim().is_empty() {
        let existing = contact.memory_summary.clone().unwrap_or_default();
        let merged = if existing.is_empty() {
            decision.memory_update.clone()
        } else {
            format!("{}\n{}", existing, decision.memory_update)
        };
        set_doc.insert("memory_summary", merged);
    }

    state
        .db
        .contacts()
        .update_one(doc! { "_id": contact.id }, doc! { "$set": set_doc }, None)
        .await?;

    if let Some(follow_up) = &decision.follow_up {
        if follow_up.needed && !follow_up.content.trim().is_empty() {
            if let Some(run_at) = parse_rfc3339_to_bson(&follow_up.run_at) {
                state
                    .db
                    .tasks()
                    .insert_one(
                        AgentTask {
                            id: None,
                            workspace_id: contact.workspace_id.clone(),
                            account_id: contact.account_id.clone(),
                            contact_wxid: contact.wxid.clone(),
                            kind: "follow_up".to_string(),
                            run_at,
                            content: follow_up.content.clone(),
                            status: "pending".to_string(),
                            error: None,
                            created_at: DateTime::now(),
                            updated_at: DateTime::now(),
                        },
                        None,
                    )
                    .await?;
            }
        }
    }
    Ok(())
}

async fn recently_replied(state: &AppState, contact: &Contact) -> AppResult<bool> {
    if let Some(last_run) = contact.last_agent_run_at {
        let elapsed = DateTime::now().timestamp_millis() - last_run.timestamp_millis();
        return Ok(elapsed < state.config.agent_min_reply_interval_seconds * 1000);
    }
    Ok(false)
}

async fn load_recent_messages(
    state: &AppState,
    contact: &Contact,
) -> AppResult<Vec<ConversationMessage>> {
    let options = FindOptions::builder()
        .sort(doc! { "created_at": -1 })
        .limit(state.config.agent_recent_message_limit)
        .build();
    let mut cursor = state
        .db
        .messages()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            options,
        )
        .await?;
    let mut messages = Vec::new();
    while let Some(message) = cursor.try_next().await? {
        messages.push(message);
    }
    Ok(messages)
}

async fn load_pending_tasks(state: &AppState, contact: &Contact) -> AppResult<Vec<AgentTask>> {
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "status": "pending"
            },
            FindOptions::builder()
                .sort(doc! { "run_at": 1 })
                .limit(5)
                .build(),
        )
        .await?;
    let mut tasks = Vec::new();
    while let Some(task) = cursor.try_next().await? {
        tasks.push(task);
    }
    Ok(tasks)
}

pub async fn write_event(
    state: &AppState,
    contact_wxid: Option<&str>,
    kind: &str,
    status: &str,
    summary: &str,
    details: Option<Document>,
) -> AppResult<()> {
    state
        .db
        .events()
        .insert_one(
            AgentEvent {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: state.config.default_account_id.clone(),
                contact_wxid: contact_wxid.map(ToString::to_string),
                kind: kind.to_string(),
                status: status.to_string(),
                summary: summary.to_string(),
                details,
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    Ok(())
}

fn parse_rfc3339_to_bson(value: &str) -> Option<DateTime> {
    DateTime::parse_rfc3339_str(value).ok()
}

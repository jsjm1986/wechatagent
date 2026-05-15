use mongodb::bson::{oid::ObjectId, DateTime, Document};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Normal,
    Managed,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub interests: Vec<String>,
    #[serde(default)]
    pub communication_style: String,
    #[serde(default)]
    pub operation_goal: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WechatAccount {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub alias: String,
    pub display_name: String,
    pub app_id: Option<String>,
    pub wxid: Option<String>,
    pub nick_name: Option<String>,
    pub online: bool,
    pub last_sync_at: DateTime,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub alias: Option<String>,
    #[serde(default)]
    pub agent_status: AgentStatus,
    pub human_profile_note: Option<String>,
    pub agent_profile: Option<AgentProfile>,
    pub memory_summary: Option<String>,
    pub last_message_at: Option<DateTime>,
    pub last_agent_run_at: Option<DateTime>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub message_id: Option<String>,
    pub direction: MessageDirection,
    pub content: String,
    pub raw: Option<Document>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub kind: String,
    pub run_at: DateTime,
    pub content: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub details: Option<Document>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub tool_name: String,
    pub request: Document,
    pub response: Option<Document>,
    pub error: Option<String>,
    pub created_at: DateTime,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnableAgentRequest {
    pub human_profile_note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileNoteRequest {
    pub human_profile_note: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchImportRequest {
    pub query: String,
}

#[derive(Debug, Deserialize)]
pub struct ContactQuery {
    pub status: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiContact {
    pub id: String,
    pub workspace_id: String,
    pub account_id: String,
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub alias: Option<String>,
    pub agent_status: AgentStatus,
    pub human_profile_note: Option<String>,
    pub agent_profile: Option<AgentProfile>,
    pub memory_summary: Option<String>,
    pub last_message_at: Option<String>,
    pub last_agent_run_at: Option<String>,
    pub updated_at: String,
}

impl From<Contact> for ApiContact {
    fn from(contact: Contact) -> Self {
        Self {
            id: contact.id.map(|id| id.to_hex()).unwrap_or_default(),
            workspace_id: contact.workspace_id,
            account_id: contact.account_id,
            wxid: contact.wxid,
            nickname: contact.nickname,
            remark: contact.remark,
            alias: contact.alias,
            agent_status: contact.agent_status,
            human_profile_note: contact.human_profile_note,
            agent_profile: contact.agent_profile,
            memory_summary: contact.memory_summary,
            last_message_at: contact.last_message_at.and_then(dt_to_string),
            last_agent_run_at: contact.last_agent_run_at.and_then(dt_to_string),
            updated_at: dt_to_string(contact.updated_at).unwrap_or_default(),
        }
    }
}

pub fn dt_to_string(dt: DateTime) -> Option<String> {
    dt.try_to_rfc3339_string().ok()
}

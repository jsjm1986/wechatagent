//! 数据库连接与集合访问层。
//!
//! [`Database`] 仅持有 mongodb client + database 引用，提供按集合命名的 typed
//! `Collection<T>` accessor。索引创建拆到 [`indexes`]，迁移框架拆到 [`migrations`]，
//! 调用方在 [`Database::connect`] 之后需自行决定何时调用 [`migrations::run`] 与
//! [`Database::ensure_indexes`]。
//!
//! 这种"connect 不副作用"的设计便于测试和迁移：迁移必须在索引创建之前运行
//! （部分迁移会改 schema 甚至重建集合），而老逻辑把索引塞进 `connect` 会让
//! 顺序无法控制。

mod indexes;
pub mod migrations;

use mongodb::{options::ClientOptions, Client, Collection, Database as MongoDatabase};

use crate::models::{
    AgentCommandRun, AgentDecisionReview, AgentEvent, AgentOutcomeMetric, AgentRunLog, AgentSoul,
    AgentTask, AgentToolCall, Contact, ContentAsset, ConversationMessage, EvaluationScenario,
    KnowledgeUsageLog, LlmCallLog, ManagementAgentMessage, ManagementAgentSession, McpCallLog,
    MemoryCandidate, MigrationRecord, OperatingMemory, OperationDomainConfig,
    OperationKnowledgeChunk, OperationKnowledgeDocument, OperationKnowledgeItem, OperationPlaybook,
    OutboxEntry, PromptTemplate, TaxonomyCandidate, TaxonomyEntry, UserOperationGuidePreview,
    WechatAccount,
};

#[derive(Clone)]
pub struct Database {
    db: MongoDatabase,
}

impl Database {
    pub async fn connect(uri: &str, database: &str) -> anyhow::Result<Self> {
        let mut options = ClientOptions::parse(uri).await?;
        options.app_name = Some("wechatagent".to_string());
        let client = Client::with_options(options)?;
        let db = client.database(database);
        Ok(Self { db })
    }

    /// 创建/确保所有索引。调用方需在 [`migrations::run`] 之后调用，避免迁移
    /// 与索引创建在 schema 不一致时互相干扰。
    pub async fn ensure_indexes(&self) -> anyhow::Result<()> {
        indexes::ensure_all(self).await
    }

    pub fn accounts(&self) -> Collection<WechatAccount> {
        self.db.collection("wechat_accounts")
    }

    /// 暴露底层 `MongoDatabase`，便于集成测试直接 `collection::<Document>(name)`
    /// 写入未走 typed `Collection<T>` 的原始 BSON（避免重复构造 30+ 字段）。
    pub fn raw(&self) -> &MongoDatabase {
        &self.db
    }

    pub fn contacts(&self) -> Collection<Contact> {
        self.db.collection("contacts")
    }

    pub fn messages(&self) -> Collection<ConversationMessage> {
        self.db.collection("conversation_messages")
    }

    pub fn tasks(&self) -> Collection<AgentTask> {
        self.db.collection("agent_tasks")
    }

    pub fn events(&self) -> Collection<AgentEvent> {
        self.db.collection("agent_events")
    }

    pub fn mcp_logs(&self) -> Collection<McpCallLog> {
        self.db.collection("mcp_call_logs")
    }

    pub fn content_assets(&self) -> Collection<ContentAsset> {
        self.db.collection("content_assets")
    }

    pub fn agent_souls(&self) -> Collection<AgentSoul> {
        self.db.collection("agent_souls")
    }

    pub fn operation_playbooks(&self) -> Collection<OperationPlaybook> {
        self.db.collection("operation_playbooks")
    }

    pub fn operation_domain_configs(&self) -> Collection<OperationDomainConfig> {
        self.db.collection("operation_domain_configs")
    }

    pub fn prompt_templates(&self) -> Collection<PromptTemplate> {
        self.db.collection("prompt_templates")
    }

    pub fn operating_memories(&self) -> Collection<OperatingMemory> {
        self.db.collection("operating_memories")
    }

    pub fn operation_knowledge_items(&self) -> Collection<OperationKnowledgeItem> {
        self.db.collection("operation_knowledge_items")
    }

    pub fn operation_knowledge_documents(&self) -> Collection<OperationKnowledgeDocument> {
        self.db.collection("operation_knowledge_documents")
    }

    pub fn operation_knowledge_chunks(&self) -> Collection<OperationKnowledgeChunk> {
        self.db.collection("operation_knowledge_chunks")
    }

    pub fn knowledge_usage_logs(&self) -> Collection<KnowledgeUsageLog> {
        self.db.collection("knowledge_usage_logs")
    }

    pub fn decision_reviews(&self) -> Collection<AgentDecisionReview> {
        self.db.collection("agent_decision_reviews")
    }

    pub fn agent_run_logs(&self) -> Collection<AgentRunLog> {
        self.db.collection("agent_run_logs")
    }

    pub fn llm_call_logs(&self) -> Collection<LlmCallLog> {
        self.db.collection("llm_call_logs")
    }

    pub fn memory_candidates(&self) -> Collection<MemoryCandidate> {
        self.db.collection("memory_candidates")
    }

    pub fn user_operation_guide_previews(&self) -> Collection<UserOperationGuidePreview> {
        self.db.collection("user_operation_guide_previews")
    }

    pub fn management_sessions(&self) -> Collection<ManagementAgentSession> {
        self.db.collection("management_agent_sessions")
    }

    pub fn management_messages(&self) -> Collection<ManagementAgentMessage> {
        self.db.collection("management_agent_messages")
    }

    pub fn command_runs(&self) -> Collection<AgentCommandRun> {
        self.db.collection("agent_command_runs")
    }

    pub fn tool_calls(&self) -> Collection<AgentToolCall> {
        self.db.collection("agent_tool_calls")
    }

    pub fn outcome_metrics(&self) -> Collection<AgentOutcomeMetric> {
        self.db.collection("agent_outcome_metrics")
    }

    pub fn evaluation_scenarios(&self) -> Collection<EvaluationScenario> {
        self.db.collection("evaluation_scenarios")
    }

    pub fn migrations(&self) -> Collection<MigrationRecord> {
        self.db.collection("migrations")
    }

    // ── agent-autonomy-loop W0 (Task 1.1) ──
    //
    // 三个新增 collection 的 typed accessor。索引创建（含 `idempotency_key` 唯一
    // 与 `(scope, kind, value.id)` 唯一）见 W0 task 1.2；具体业务字段语义在
    // W3 / W4 落地（参考 design.md §3.2 / §3.3 / §3.4）。

    /// agent-autonomy-loop W0：`agent_send_outbox` 集合 typed accessor（Requirements 13.1）。
    pub fn collection_agent_send_outbox(&self) -> Collection<OutboxEntry> {
        self.db.collection("agent_send_outbox")
    }

    /// agent-autonomy-loop W0：`system_taxonomies` 集合 typed accessor（Requirements 8.1）。
    pub fn collection_system_taxonomies(&self) -> Collection<TaxonomyEntry> {
        self.db.collection("system_taxonomies")
    }

    /// agent-autonomy-loop W0：`taxonomy_candidates` 集合 typed accessor（Requirements 8.3）。
    pub fn collection_taxonomy_candidates(&self) -> Collection<TaxonomyCandidate> {
        self.db.collection("taxonomy_candidates")
    }
}

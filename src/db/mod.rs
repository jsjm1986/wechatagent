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
    Experiment, KnowledgeUsageLog, LlmCallLog, ManagementAgentMessage, ManagementAgentSession,
    McpCallLog, MemoryCandidate, MigrationRecord, OperatingMemory, OperationDomainConfig,
    OperationKnowledgeChunk, OperationKnowledgeDocument, OperationKnowledgeItem, OperationPlaybook,
    OutboxEntry, PostReleaseReview, PromptTemplate, Proposal, ShadowReplay, TaxonomyCandidate,
    TaxonomyEntry, ThresholdOverride, UserOperationGuidePreview, WechatAccount,
};

#[derive(Clone)]
pub struct Database {
    db: MongoDatabase,
    client: Client,
}

impl Database {
    pub async fn connect(uri: &str, database: &str) -> anyhow::Result<Self> {
        let mut options = ClientOptions::parse(uri).await?;
        options.app_name = Some("wechatagent".to_string());
        let client = Client::with_options(options)?;
        let db = client.database(database);
        Ok(Self {
            db,
            client,
        })
    }

    /// 创建/确保所有索引。调用方需在 [`migrations::run`] 之后调用，避免迁移
    /// 与索引创建在 schema 不一致时互相干扰。
    pub async fn ensure_indexes(&self) -> anyhow::Result<()> {
        indexes::ensure_all(self).await
    }

    /// agent-self-evolution M4 W4 Task 5.2：暴露底层 `Client`，让 release.rs 能
    /// 跨 collection 起 mongo session 实现 transaction（threshold_overrides +
    /// proposals 必须 atomic 落地，否则会出现 release 已写但 proposal status
    /// 还是 eligible_for_release 的污染状态）。
    pub fn client(&self) -> Client {
        self.client.clone()
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

    // ── agent-self-evolution W0 (Task 1.1) ──
    //
    // 5 个新增 collection 的 typed accessor。索引创建（`(workspace_id, account_id,
    // started_at desc)` / `(experiment_id)` 唯一 / `(proposal_id)` 等）见 W0 task 1.2。
    // 业务字段最终值在 W2/W3/W4 落定（参考 design.md §3.x）。

    /// agent-self-evolution W0：`experiments` 集合 typed accessor（Requirements 1.3 / 8.1）。
    pub fn experiments(&self) -> Collection<Experiment> {
        self.db.collection("experiments")
    }

    /// agent-self-evolution W0：`proposals` 集合 typed accessor（Requirements 3.x / 4.x / 8.1）。
    pub fn proposals(&self) -> Collection<Proposal> {
        self.db.collection("proposals")
    }

    /// agent-self-evolution W0：`shadow_replays` 集合 typed accessor（Requirements 5.x / 8.1）。
    pub fn shadow_replays(&self) -> Collection<ShadowReplay> {
        self.db.collection("shadow_replays")
    }

    /// agent-self-evolution W0：`threshold_overrides` 集合 typed accessor（Requirements 6.x / 8.1）。
    pub fn threshold_overrides(&self) -> Collection<ThresholdOverride> {
        self.db.collection("threshold_overrides")
    }

    /// agent-self-evolution W4 (Task 5.6)：`post_release_reviews` 集合 typed accessor
    /// （Requirements 9.7）。+24h 对比窗口评测在 release 完成后由 evolution worker
    /// 末尾扫描；不参与 release 决策本身。
    pub fn post_release_reviews(&self) -> Collection<PostReleaseReview> {
        self.db.collection("post_release_reviews")
    }
}

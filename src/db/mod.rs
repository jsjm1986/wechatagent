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
    AgentCommandRun, AgentDecisionReview, AgentEvent, AgentOutcomeMetric, AgentPrincipalEscalation,
    AgentRunLog, AgentSoul,
    AgentTask, AgentToolCall, BehaviorSignal, BehaviorSignalMetric, CatalogRebuildJob,
    ChunkRevision, Contact,
    ContentAsset, ConversationMessage, DomainProfile, DomainSchema, EvaluationScenario, Experiment, IngestSource,
    KnowledgeChatTask, KnowledgeChatTurn, KnowledgeDailyReport, KnowledgeGapSignal,
    KnowledgeOperatorMemory, KnowledgeUsageLog, LlmCallLog, LlmProviderConfig,
    ManagementAgentMessage, ManagementAgentSession, McpCallLog, MemoryCandidate, MigrationRecord,
    OperatingMemory, OperationDomainConfig, OperationKnowledgeChunk, OperationKnowledgeDocument,
    OperationPlaybook, OutboxEntry, PostReleaseReview, PromptTemplate,
    Proposal, ShadowReplay, TaxonomyCandidate, TaxonomyEntry, ThresholdOverride,
    ThresholdOverrideAudit, UserOperationGuidePreview, WechatAccount,
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

    /// 自学习采集管道 S1–S3：`behavior_signals` append-only 事件日志 typed
    /// accessor。只存系统观察到的客观行为量（reply_latency / reply_length /
    /// reactivation / silence），不含任何 LLM 解释。索引（含
    /// `(workspace_id, dedupe_key)` partial unique 幂等约束）见 `db/indexes.rs`。
    pub fn behavior_signals(&self) -> Collection<BehaviorSignal> {
        self.db.collection("behavior_signals")
    }

    /// P3 采集健康度：`behavior_signal_metrics` 每日每 workspace 三态计数聚合
    /// （`_id="{workspace_id}:{date}"`）。索引见 `db/indexes.rs`。
    pub fn behavior_signal_metrics(&self) -> Collection<BehaviorSignalMetric> {
        self.db.collection("behavior_signal_metrics")
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

    /// Phase B / B4：`operation_state_policies` typed accessor。
    /// 行结构 [`crate::models::OperationStatePolicy`]，每行对应单 (workspace, domain, state_key)
    /// 的"允许/禁止动作 + 推荐节奏" policy。enforce 路径在 `agent::guards`。
    pub fn operation_state_policies(&self) -> Collection<crate::models::OperationStatePolicy> {
        self.db.collection("operation_state_policies")
    }

    pub fn prompt_templates(&self) -> Collection<PromptTemplate> {
        self.db.collection("prompt_templates")
    }

    pub fn operating_memories(&self) -> Collection<OperatingMemory> {
        self.db.collection("operating_memories")
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

    pub fn knowledge_chat_turns(&self) -> Collection<KnowledgeChatTurn> {
        self.db.collection("knowledge_chat_turns")
    }

    /// P1-7：每 session 一个 `{ _id: "{workspace_id}|{session_id}", seq: i64 }`
    /// 行；`findOneAndUpdate $inc seq +1 upsert returnDocument=After` 得到原子
    /// 自增后的新 turn_index。避免「读 last + 1 → 写」之间出现并发写者读到
    /// 同一 last 制造重复 turn_index、命中 unique 索引报错的窗口。
    pub fn knowledge_chat_session_seqs(&self) -> Collection<mongodb::bson::Document> {
        self.db.collection("knowledge_chat_session_seqs")
    }

    pub fn knowledge_daily_reports(&self) -> Collection<KnowledgeDailyReport> {
        self.db.collection("knowledge_daily_reports")
    }

    pub fn knowledge_chat_tasks(&self) -> Collection<KnowledgeChatTask> {
        self.db.collection("knowledge_chat_tasks")
    }

    pub fn knowledge_operator_memory(&self) -> Collection<KnowledgeOperatorMemory> {
        self.db.collection("knowledge_operator_memory")
    }

    pub fn decision_reviews(&self) -> Collection<AgentDecisionReview> {
        self.db.collection("agent_decision_reviews")
    }

    pub fn agent_run_logs(&self) -> Collection<AgentRunLog> {
        self.db.collection("agent_run_logs")
    }

    pub fn agent_principal_escalations(&self) -> Collection<AgentPrincipalEscalation> {
        self.db.collection("agent_principal_escalations")
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

    /// Phase C / C5：`threshold_overrides_audit` 不可变审计表。每次 release /
    /// rollback / auto-release 都追加一行；与 `threshold_overrides` 当前生效层
    /// 解耦，方便事后追因（"为什么 X 阈值在 YYYY-MM 跳到 Z"）。
    pub fn threshold_overrides_audit(&self) -> Collection<ThresholdOverrideAudit> {
        self.db.collection("threshold_overrides_audit")
    }

    /// agent-self-evolution W4 (Task 5.6)：`post_release_reviews` 集合 typed accessor
    /// （Requirements 9.7）。+24h 对比窗口评测在 release 完成后由 evolution worker
    /// 末尾扫描；不参与 release 决策本身。
    pub fn post_release_reviews(&self) -> Collection<PostReleaseReview> {
        self.db.collection("post_release_reviews")
    }

    /// Phase C / C3：`evolution_runtime_flags` 集合 typed accessor。
    /// 运行时演化器开关 + 灰度比例（0..=100）。`evolution::is_evolution_enabled_for`
    /// 读这条文档计算 `hash(contact_id) % 100 < rollout_percent`。同一 workspace
    /// 单条文档；不存在时按"关停态"处理。
    pub fn evolution_runtime_flags(&self) -> Collection<crate::models::EvolutionRuntimeFlag> {
        self.db.collection("evolution_runtime_flags")
    }

    /// LLM 服务商配置集合：把原本只读的 `.env`（`OPENAI_BASE_URL` /
    /// `OPENAI_API_KEY` / `OPENAI_MODEL`）抬升为可在前端运行时编辑的 DB 数据，
    /// 并支持 `format=openai|anthropic` 双形态。`is_active=true` 的那条会被
    /// 启动时与切换时由 `LlmRegistry` 加载，作为 `AppState.llm` 的实际后端。
    pub fn llm_provider_configs(&self) -> Collection<LlmProviderConfig> {
        self.db.collection("llm_provider_configs")
    }

    // ── knowledge-wiki Phase A：4 个新 collection 的 typed accessor ──
    //
    // 落地"质量 / 可检索 / 可修改 / 可优化"四件事的存储层：
    //   - `chunk_revisions` 是不可变编辑历史，apply_chunk_revision 与 chunks 双写；
    //   - `knowledge_gap_signals` 是 structural + semantic lint 的待办队列；
    //   - `domain_schemas` 是行业可配 schema（active 一条 / workspace）；
    //   - `catalog_rebuild_jobs` 是 catalog 落库的异步重写队列。
    //
    // 索引创建见 `db/indexes.rs`。

    /// chunk 编辑历史（patch / split / merge / rollback / archive / restore /
    /// verify / unverify）。每次 apply_chunk_revision 都会在 chunks 表更新前先写一行。
    pub fn chunk_revisions(&self) -> Collection<ChunkRevision> {
        self.db.collection("chunk_revisions")
    }

    /// gap signals：orphan / broken_link / no_outlinks / contradiction / stale /
    /// missing_chunk / suggestion / low_confidence。两阶段 sweep 后状态流转
    /// pending → auto_resolved | llm_resolved | applied | dismissed。
    pub fn knowledge_gap_signals(&self) -> Collection<KnowledgeGapSignal> {
        self.db.collection("knowledge_gap_signals")
    }

    /// 行业可配 schema：每 workspace 同时只能 1 条 is_active=true。
    pub fn domain_schemas(&self) -> Collection<DomainSchema> {
        self.db.collection("domain_schemas")
    }

    /// universal-domain-adaptation Phase 0：行业「总装配单」。每 workspace 同时
    /// 1 条 is_active=true；运行时按 active 加载（无则 fallback DEFAULT_PROFILE）。
    /// 详见 `src/agent/domain_profile.rs`。索引见 `db/indexes.rs`。
    pub fn domain_profiles(&self) -> Collection<DomainProfile> {
        self.db.collection("domain_profiles")
    }

    /// catalog 重建队列：apply_chunk_revision 写完即 enqueue；catalog_rebuild_worker
    /// 每 200ms 取一批 status=queued 落库 `documents.catalog_summary_persisted`。
    pub fn catalog_rebuild_jobs(&self) -> Collection<CatalogRebuildJob> {
        self.db.collection("catalog_rebuild_jobs")
    }

    /// P1-6：自动 ingest 数据源 typed accessor。`ingest_worker_loop` 逐 workspace
    /// 扫 `status="active"` 的 source，按 `schedule_minutes` 节流后 GET → 解析 →
    /// `ingest_chunked_text` 落 chunks（`integrity_status="needs_review"`）。
    pub fn ingest_sources(&self) -> Collection<IngestSource> {
        self.db.collection("ingest_sources")
    }
}

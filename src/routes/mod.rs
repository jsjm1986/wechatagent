//! Routes 模块入口：组装 `AppState` 与 `api_router`，并通过 `pub use` 暴露子模块。
//!
//! 业务路由按职责切分到子模块；本入口只负责拼装 axum Router、共享状态和外部
//! 依赖（main.rs / agent.rs / mcp.rs / tasks.rs / webhooks.rs / 集成测试）需要
//! 看到的最小公开 API。

use axum::{
    routing::{get, patch, post, put},
    Router,
};
use std::sync::Arc;

use crate::{
    config::AppConfig,
    db::Database,
    llm::{LlmProvider, LlmRegistry},
    mcp::McpClient,
};

mod accounts;
mod admin_ops_versions;
mod admin_outbox;
mod admin_state_policies;
mod admin_taxonomies;
mod admin_taxonomy_candidates;
mod assets;
mod auth;
mod behavior_signal_metrics;
pub mod chunk_locks;
mod contacts;
mod conversations;
mod domain_schemas;
mod domains;
mod evaluations;
mod events;
mod evolution;
mod guides;
mod health;
pub(crate) mod knowledge;
mod lessons_learned;
mod llm_providers;
mod management;
mod observability;
mod outcome_metrics;
mod outcomes_autonomy;
mod playbooks;
mod prompt_templates;
mod reviews;
mod shared;
mod simulations;
mod souls;
mod tasks;

pub use outcomes_autonomy::{
    get_autonomy_outcomes, list_autonomy_revisions, AutonomyMetricsQuery, AutonomyRevisionsQuery,
};
pub use knowledge::{
    ChunkBatchArchiveRequest, ChunkBatchVerifyRequest, ChunkReferrersQuery,
};
// G3：批量动作 + 反向查询的处理函数。集成测试直接调用绕过 axum HTTP 层。
pub mod ext_knowledge {
    pub use super::knowledge::{batch_archive_chunks, batch_verify_chunks, list_chunk_referrers};
    // P1-5 / #574：multimodal 导入 + ingest 核心，供集成测试绕过 axum extractor
    // （Multipart 无法在测试里手工构造）直接驱动字节 / 图片导入路径。
    pub use super::knowledge::{
        import_operation_knowledge_apply_image, import_pdf_bytes, ingest_chunked_text,
        ImportApplyImageRequest, IngestOutcome,
    };
    // real-LLM 知识库全能力 smoke（real_llm_knowledge.rs K5–K9）：把 LLM 驱动的
    // 抽取 / 自动审定 / AI 修复 / 标签抽取 handler + 其请求体类型暴露给测试 crate，
    // 让真模型直接驱动这些「mock 测不到」的链路。请求体字段为私有但派生
    // `Deserialize`，测试侧用 `serde_json::from_value` 构造，无需放开字段可见性。
    // `decide_auto_verify_status` 暴露用于直接对 provenance 闸门做单元级红线断言。
    pub use super::knowledge::{
        auto_verify_operation_knowledge_chunks, decide_auto_verify_status,
        extract_operation_knowledge_tags, import_operation_knowledge_preview,
        propose_chunk_repair, verify_operation_knowledge_chunk, ExtractKnowledgeTagsRequest,
        KnowledgeAutoVerifyRequest, KnowledgeVerifyRequest, OperationKnowledgeImportRequest,
    };
}
pub use shared::upsert_contact_from_value;

use accounts::{list_accounts, sync_accounts, update_account_mcp_key};
use admin_ops_versions::{
    publish_operation_domain_version, publish_operation_state_policy_version,
    publish_taxonomy_version, rollback_operation_domain_version,
    rollback_operation_state_policy_version, rollback_taxonomy_version,
    rollout_operation_domain_version, rollout_operation_state_policy_version,
    rollout_taxonomy_version,
};
use admin_outbox::{cancel_outbox, list_outbox};
use admin_state_policies::{get_operation_state_policy, list_operation_state_policies};
use admin_taxonomies::{
    create_taxonomy, delete_taxonomy, list_taxonomies, patch_taxonomy,
};
use admin_taxonomy_candidates::{
    approve_taxonomy_candidate, list_taxonomy_candidates, reject_taxonomy_candidate,
};
use assets::{create_content_asset, list_content_assets};
use contacts::{
    analyze_contact_profile, add_deal_event, disable_agent, enable_agent, get_contact,
    get_contact_memory_card, get_operating_memory, get_operation_health, import_contacts_endpoint,
    list_contact_memory_candidates, list_contacts, run_contact_memory_consolidation,
    search_contacts_endpoint, search_import_contacts, update_operating_memory,
    update_operation_profile, update_profile_note, update_custom_agent_instructions,
};
use conversations::list_messages;
use domain_schemas::{
    activate_domain_schema, create_domain_schema, delete_domain_schema, list_domain_schemas,
    update_domain_schema,
};
use domains::{
    get_operation_domain, get_operation_domain_state_machine, list_operation_domains,
    reset_operation_domain, update_operation_domain, update_operation_domain_state_machine,
};
use evaluations::{
    create_evaluation_scenario, delete_evaluation_scenario, list_evaluation_scenarios,
    run_formula_adherence_evaluation, update_evaluation_scenario,
};
use events::list_events;
use evolution::{
    get_evolution_proposal_detail, get_evolution_runtime_flag, list_evolution_experiments,
    list_threshold_override_audit, put_evolution_runtime_flag, release_evolution_proposal,
    rollback_evolution_proposal,
};
use guides::{apply_user_operation_guide, preview_user_operation_guide};
use health::health;
use llm_providers::{
    activate_provider, create_provider, delete_provider, list_providers, set_vision_active,
    test_provider,
    update_provider,
};
use knowledge::{
    analyze_operation_knowledge_logs, answer_chunk_repair,
    apply_knowledge_gap_signal,
    archive_operation_knowledge_chunk, ask_knowledge, ask_knowledge_stream, knowledge_metrics,
    auto_verify_operation_knowledge_chunks,
    batch_archive_chunks, batch_verify_chunks,
    chat_apply, chat_discard,
    chat_history, chat_session_stream, chat_task_cancel, chat_task_create, chat_task_get,
    chat_turn, create_ingest_source, create_operation_knowledge,
    create_operation_knowledge_chunk, create_operation_knowledge_document,
    delete_ingest_source, delete_operation_knowledge, delete_operation_knowledge_chunk,
    delete_operation_knowledge_document, digest_dismiss_card, digest_regenerate, digest_today,
    dismiss_knowledge_gap_signal,
    get_operation_knowledge_catalog, get_operation_knowledge_catalog_persisted,
    get_operation_knowledge_chunk_source, get_operation_knowledge_completeness,
    get_operation_knowledge_document, get_operation_knowledge_integrity_report,
    extract_operation_knowledge_tags, import_operation_knowledge_apply,
    import_operation_knowledge_apply_image, import_operation_knowledge_apply_pdf,
    import_operation_knowledge_preview, knowledge_aggregate_metadata, knowledge_inbox,
    list_chunk_referrers,
    list_ingest_sources,
    list_knowledge_gap_signals,
    list_knowledge_usage,
    list_operation_knowledge, list_operation_knowledge_chunk_revisions,
    list_operation_knowledge_chunks,
    list_operation_knowledge_document_chunks, list_operation_knowledge_documents,
    list_operator_memory,
    merge_operation_knowledge_chunk,
    open_operation_knowledge_slices, patch_operation_knowledge_chunk,
    propose_chunk_repair, propose_pack_repair,
    record_repair_apply, refresh_operation_knowledge_completeness,
    reject_operation_knowledge_chunk, relate_operation_knowledge_chunk,
    restore_operation_knowledge_chunk, rollback_operation_knowledge_chunk,
    search_operation_knowledge_tool, split_operation_knowledge_chunk,
    sweep_knowledge_gap_signals,
    test_operation_knowledge_match, unrelate_operation_knowledge_chunk,
    update_operation_knowledge,
    update_operation_knowledge_chunk, update_operation_knowledge_document,
    update_ingest_source,
    verify_operation_knowledge_chunk,
};
use management::{
    create_management_session, get_management_command, get_tool_catalog, post_management_message,
};
use behavior_signal_metrics::list_behavior_signal_metrics;
use lessons_learned::{list_lessons_learned, promote_lesson_to_peer_case};
use observability::{phase_rollup, worker_health};
use outcome_metrics::list_agent_outcome_metrics;
use playbooks::{
    create_operation_playbook, generate_operation_playbook, list_operation_playbooks,
    optimize_operation_playbook, set_default_operation_playbook, update_operation_playbook,
};
use prompt_templates::{
    create_prompt_template, list_prompt_templates, publish_prompt_template,
    reset_system_prompt_pack, update_prompt_template,
};
use reviews::{get_decision_review, list_decision_reviews};
use simulations::{run_user_operation_evaluation, simulate_user_operation_dialogue};
use souls::{create_agent_soul, list_agent_souls, publish_agent_soul, update_agent_soul};
use tasks::{cancel_agent_task, list_agent_runs, list_llm_usage, list_tasks, review_task_now};

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub mcp: McpClient,
    pub llm: Arc<dyn LlmProvider>,
    /// 当前激活 provider 的热替换 wrapper。生产路径 `main.rs` 让 `llm` 与
    /// 这里指向同一个 [`LlmRegistry`] 实例；前端「启用 provider」走
    /// `routes/llm_providers` 时取这个字段进行原子 swap，写一次新 client
    /// 后整个进程的 LLM 调用就切换到新配置。测试可填 `None`，使用 mock。
    pub llm_registry: Option<Arc<LlmRegistry>>,
    pub config: AppConfig,
    /// agent-self-evolution M4 W4 Task 5.3：prompt 包版本号。
    ///
    /// `prompts::ensure_prompt_pack_v2` / `ensure_evolution_prompt_pack_v1`
    /// 在末尾各 fetch_add 一次；`evolution::release::release_prompt`
    /// commit 后 fetch_add 一次。`agent::generate_agent_json` 把当前值折进
    /// LRU cache key，让 release/seed/rollback 任一动作 atomic 触发缓存失效，
    /// 不需要重启进程。
    pub prompt_pack_version: Arc<std::sync::atomic::AtomicU64>,
    /// knowledge-digest-workstation Phase 4：chat 进度总线。
    /// 由 `KnowledgeTaskWorker` 写 turn 后 `bump` 通知；
    /// `chat_session_stream` SSE handler 订阅 watch::Receiver 推送 turn id。
    pub chat_progress_bus: Arc<crate::knowledge_task::ChatProgressBus>,
    /// Phase E / E2：reviewer 双脑并行的第二 provider。
    ///
    /// `None` 时 review_decision 仅跑主 reviewer (`self.llm`)，行为与本字段
    /// 引入前完全一致；`Some` 时 reviewer 走 `tokio::join!` 并行，分歧触发
    /// single-shot revision（[`crate::agent::review::detect_dual_reviewer_disagreement`]）。
    /// 由 `main.rs` 在 `config.reviewer_dual_enabled=true` 且第二 provider 4
    /// 件套环境变量齐备时构建一次，进程生命周期内不替换；运行时切换需重启
    /// （与 `LlmRegistry` 的运行时热替换不同——双 reviewer 故意不热切以保持
    /// epistemic 对照稳定）。
    pub second_reviewer_llm: Option<Arc<dyn LlmProvider>>,
    /// Phase G P1-4：知识 chunk 软锁表。进程内 DashMap，重启清空。
    /// key=chunk_id；value 含 owner / 过期时间，TTL 见 [`chunk_locks::CHUNK_LOCK_TTL_SECONDS`]。
    pub chunk_locks: chunk_locks::ChunkLockMap,
    /// Phase G P1-4：知识 chunk 事件总线。WebSocket handler 订阅；
    /// patch/archive/restore/... handler 写入 `Revised` 事件。
    /// 多副本部署需 Redis pub/sub —— 留 P2。
    pub chunk_event_bus: tokio::sync::broadcast::Sender<chunk_locks::ChunkEvent>,
    /// Phase G P1-7：RS256 JWT keypair。`jwt_enabled=false` → None；
    /// `true` 时 main.rs 启动期 `JwtKeys::from_config` 解码 PEM 失败直接 panic。
    pub jwt_keys: Option<Arc<crate::auth::jwt::JwtKeys>>,
}

pub fn api_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/workspace", post(auth::switch_workspace))
        .route("/auth/token", post(auth::issue_token))
        .route("/accounts", get(list_accounts))
        .route("/accounts/sync", post(sync_accounts))
        .route("/accounts/:id/mcp-key", put(update_account_mcp_key))
        .route("/contacts", get(list_contacts))
        .route("/contacts/search", post(search_contacts_endpoint))
        .route("/contacts/import", post(import_contacts_endpoint))
        .route("/contacts/search-import", post(search_import_contacts))
        .route("/contacts/:id", get(get_contact))
        .route("/contacts/:id/enable-agent", post(enable_agent))
        .route("/contacts/:id/disable-agent", post(disable_agent))
        .route("/contacts/:id/profile-note", put(update_profile_note))
        .route(
            "/contacts/:id/custom-agent-instructions",
            put(update_custom_agent_instructions),
        )
        .route(
            "/contacts/:id/operation-profile",
            put(update_operation_profile),
        )
        .route("/contacts/:id/deal-events", post(add_deal_event))
        .route(
            "/contacts/:id/analyze-profile",
            post(analyze_contact_profile),
        )
        .route(
            "/contacts/:id/operating-memory",
            get(get_operating_memory).put(update_operating_memory),
        )
        .route("/contacts/:id/memory-card", get(get_contact_memory_card))
        .route(
            "/contacts/:id/memory-candidates",
            get(list_contact_memory_candidates),
        )
        .route(
            "/contacts/:id/memory-consolidation/run",
            post(run_contact_memory_consolidation),
        )
        .route("/contacts/:id/operation-health", get(get_operation_health))
        .route(
            "/user-operations/guide/preview",
            post(preview_user_operation_guide),
        )
        .route(
            "/user-operations/guide/apply",
            post(apply_user_operation_guide),
        )
        .route(
            "/user-operations/simulations/dialogue",
            post(simulate_user_operation_dialogue),
        )
        .route(
            "/user-operations/evaluations/run",
            post(run_user_operation_evaluation),
        )
        .route("/conversations/:contact_id/messages", get(list_messages))
        .route("/events", get(list_events))
        .route("/tasks", get(list_tasks))
        .route("/agent-runs", get(list_agent_runs))
        .route("/llm-usage", get(list_llm_usage))
        .route("/agent-tasks/:id/review-now", post(review_task_now))
        .route("/agent-tasks/:id/cancel", post(cancel_agent_task))
        .route(
            "/content-assets",
            get(list_content_assets).post(create_content_asset),
        )
        .route(
            "/operation-knowledge",
            get(list_operation_knowledge).post(create_operation_knowledge),
        )
        .route(
            "/operation-knowledge/documents",
            get(list_operation_knowledge_documents).post(create_operation_knowledge_document),
        )
        .route(
            "/operation-knowledge/documents/:id",
            get(get_operation_knowledge_document)
                .put(update_operation_knowledge_document)
                .delete(delete_operation_knowledge_document),
        )
        .route(
            "/operation-knowledge/documents/:id/chunks",
            get(list_operation_knowledge_document_chunks),
        )
        .route(
            "/operation-knowledge/chunks",
            get(list_operation_knowledge_chunks).post(create_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id",
            put(update_operation_knowledge_chunk).delete(delete_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/source",
            get(get_operation_knowledge_chunk_source),
        )
        .route(
            "/operation-knowledge/chunks/:id/verify",
            post(verify_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/reject",
            post(reject_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/repair",
            post(propose_chunk_repair),
        )
        .route(
            "/operation-knowledge/chunks/:id/repair/answer",
            post(answer_chunk_repair),
        )
        // ── knowledge-wiki Phase C：7 个编辑路由 + 2 个关系路由 ────────────────
        .route(
            "/operation-knowledge/chunks/:id/patch",
            post(patch_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/archive",
            post(archive_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/restore",
            post(restore_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/rollback/:revision_id",
            post(rollback_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/revisions",
            get(list_operation_knowledge_chunk_revisions),
        )
        .route(
            "/operation-knowledge/chunks/:id/split",
            post(split_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/merge",
            post(merge_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/relate",
            post(relate_operation_knowledge_chunk),
        )
        .route(
            "/operation-knowledge/chunks/:id/relate/:target_id",
            axum::routing::delete(unrelate_operation_knowledge_chunk),
        )
        // ── Phase G P1-4 · 软锁 + WebSocket 事件总线 ───────────────────────────
        .route(
            "/operation-knowledge/chunks/:id/lock",
            post(chunk_locks::acquire_chunk_lock)
                .delete(chunk_locks::release_chunk_lock),
        )
        .route("/ws/chunks", get(chunk_locks::chunk_event_websocket))
        // ── G3 · 反向查询 + 批量动作（admin 手工触发，非 AI 自动） ─────────────
        .route(
            "/operation-knowledge/chunks/referrers",
            get(list_chunk_referrers),
        )
        .route(
            "/operation-knowledge/chunks/batch-verify",
            post(batch_verify_chunks),
        )
        .route(
            "/operation-knowledge/chunks/batch-archive",
            post(batch_archive_chunks),
        )
        .route(
            "/operation-knowledge/catalog",
            get(get_operation_knowledge_catalog),
        )
        .route(
            "/operation-knowledge/catalog/persisted",
            get(get_operation_knowledge_catalog_persisted),
        )
        .route(
            "/operation-knowledge/completeness",
            get(get_operation_knowledge_completeness)
                .post(refresh_operation_knowledge_completeness),
        )
        .route(
            "/operation-knowledge/integrity-report",
            get(get_operation_knowledge_integrity_report),
        )
        .route(
            "/operation-knowledge/tools/search",
            post(search_operation_knowledge_tool),
        )
        .route(
            "/operation-knowledge/auto-verify",
            post(auto_verify_operation_knowledge_chunks),
        )
        .route(
            "/knowledge/gap-signals",
            get(list_knowledge_gap_signals),
        )
        .route(
            "/knowledge/gap-signals/:id/dismiss",
            post(dismiss_knowledge_gap_signal),
        )
        .route(
            "/knowledge/gap-signals/:id/apply",
            post(apply_knowledge_gap_signal),
        )
        .route(
            "/knowledge/gap-signals/sweep",
            post(sweep_knowledge_gap_signals),
        )
        .route("/knowledge/ask", post(ask_knowledge))
        .route("/knowledge/ask/stream", get(ask_knowledge_stream))
        .route("/knowledge/metrics", get(knowledge_metrics))
        .route("/knowledge/operator-memory", get(list_operator_memory))
        .route(
            "/operation-knowledge/tools/open-slice",
            post(open_operation_knowledge_slices),
        )
        .route(
            "/operation-knowledge/tools/open-evidence",
            post(open_operation_knowledge_slices),
        )
        .route(
            "/operation-knowledge/import-preview",
            post(import_operation_knowledge_preview),
        )
        .route(
            "/operation-knowledge/import-apply",
            post(import_operation_knowledge_apply),
        )
        .route(
            "/operation-knowledge/import-apply-pdf",
            post(import_operation_knowledge_apply_pdf),
        )
        .route(
            "/operation-knowledge/import-apply-image",
            post(import_operation_knowledge_apply_image),
        )
        .route(
            "/operation-knowledge/extract-tags",
            post(extract_operation_knowledge_tags),
        )
        .route(
            "/operation-knowledge/test-match",
            post(test_operation_knowledge_match),
        )
        .route("/operation-knowledge/usage", get(list_knowledge_usage))
        .route(
            "/operation-knowledge/logs/analyze",
            get(analyze_operation_knowledge_logs),
        )
        .route(
            "/operation-knowledge/items/:id/repair",
            post(propose_pack_repair),
        )
        .route(
            "/operation-knowledge/repair/applied",
            post(record_repair_apply),
        )
        .route("/operation-knowledge/chat", post(chat_turn))
        .route(
            "/operation-knowledge/inbox",
            get(knowledge_inbox),
        )
        .route(
            "/operation-knowledge/metadata",
            get(knowledge_aggregate_metadata),
        )
        .route(
            "/operation-knowledge/chat/:session_id",
            get(chat_history),
        )
        .route(
            "/operation-knowledge/chat/:session_id/apply",
            post(chat_apply),
        )
        .route(
            "/operation-knowledge/chat/:session_id/discard",
            post(chat_discard),
        )
        .route("/knowledge/digest/today", get(digest_today))
        .route("/knowledge/digest/regenerate", post(digest_regenerate))
        .route(
            "/knowledge/digest/cards/:id/dismiss",
            post(digest_dismiss_card),
        )
        .route("/knowledge/chat/tasks", post(chat_task_create))
        .route(
            "/knowledge/chat/tasks/:id",
            get(chat_task_get),
        )
        .route(
            "/knowledge/chat/tasks/:id/cancel",
            post(chat_task_cancel),
        )
        .route(
            "/knowledge/chat/sessions/:sid/stream",
            get(chat_session_stream),
        )
        .route(
            "/knowledge/ingest-sources",
            get(list_ingest_sources).post(create_ingest_source),
        )
        .route(
            "/knowledge/ingest-sources/:id",
            patch(update_ingest_source).delete(delete_ingest_source),
        )
        .route(
            "/operation-knowledge/:id",
            put(update_operation_knowledge).delete(delete_operation_knowledge),
        )
        .route("/decision-reviews", get(list_decision_reviews))
        .route("/decision-reviews/:id", get(get_decision_review))
        .route("/agent-outcome-metrics", get(list_agent_outcome_metrics))
        .route(
            "/behavior-signal-metrics",
            get(list_behavior_signal_metrics),
        )
        .route("/outcomes/autonomy", get(get_autonomy_outcomes))
        .route(
            "/outcomes/autonomy/revisions",
            get(list_autonomy_revisions),
        )
        .route(
            "/evaluation-scenarios",
            get(list_evaluation_scenarios).post(create_evaluation_scenario),
        )
        .route(
            "/evaluation-scenarios/:id",
            put(update_evaluation_scenario).delete(delete_evaluation_scenario),
        )
        .route(
            "/user-operations/evaluations/formula-adherence",
            post(run_formula_adherence_evaluation),
        )
        .route(
            "/agent-souls",
            get(list_agent_souls).post(create_agent_soul),
        )
        .route("/agent-souls/:id", put(update_agent_soul))
        .route("/agent-souls/:id/publish", post(publish_agent_soul))
        .route("/operation-domains", get(list_operation_domains))
        .route(
            "/operation-domains/:domain",
            get(get_operation_domain).put(update_operation_domain),
        )
        .route(
            "/operation-domains/:domain/state-machine",
            get(get_operation_domain_state_machine).put(update_operation_domain_state_machine),
        )
        .route(
            "/operation-domains/:domain/reset",
            post(reset_operation_domain),
        )
        .route(
            "/prompt-templates",
            get(list_prompt_templates).post(create_prompt_template),
        )
        .route("/prompt-templates/:id", put(update_prompt_template))
        .route(
            "/prompt-templates/:id/publish",
            post(publish_prompt_template),
        )
        .route(
            "/prompt-templates/reset-system-pack",
            post(reset_system_prompt_pack),
        )
        .route(
            "/operation-playbooks",
            get(list_operation_playbooks).post(create_operation_playbook),
        )
        .route(
            "/operation-playbooks/generate",
            post(generate_operation_playbook),
        )
        .route(
            "/operation-playbooks/:id/optimize",
            post(optimize_operation_playbook),
        )
        .route("/operation-playbooks/:id", put(update_operation_playbook))
        .route(
            "/operation-playbooks/:id/set-default",
            post(set_default_operation_playbook),
        )
        .route(
            "/management-agent/sessions",
            post(create_management_session),
        )
        .route(
            "/management-agent/sessions/:id/messages",
            post(post_management_message),
        )
        .route(
            "/management-agent/commands/:id",
            get(get_management_command),
        )
        .route("/management-agent/tool-catalog", get(get_tool_catalog))
        // ── agent-autonomy-loop W3 / Task 4.8：双层标签 admin 路由 ─────────────
        .route(
            "/admin/taxonomies",
            get(list_taxonomies).post(create_taxonomy),
        )
        .route(
            "/admin/taxonomies/:id",
            patch(patch_taxonomy).delete(delete_taxonomy),
        )
        .route(
            "/admin/taxonomy-candidates",
            get(list_taxonomy_candidates),
        )
        .route(
            "/admin/taxonomy-candidates/:id/approve",
            post(approve_taxonomy_candidate),
        )
        .route(
            "/admin/taxonomy-candidates/:id/reject",
            post(reject_taxonomy_candidate),
        )
        // ── Phase E / E5-T1：ops 三表多版本灰度 admin 路由 ──────────────────────
        // 同一套 publish/rollout/rollback 三动作分别覆盖
        // operation_domain_configs / operation_state_policies / system_taxonomies。
        // state-policies 没有独立 CRUD（写路径走 publish），但前端面板需要列表 + 详情
        // 来渲染 active_versions 流水与回滚链。
        .route(
            "/admin/operation-state-policies",
            get(list_operation_state_policies),
        )
        .route(
            "/admin/operation-state-policies/:id",
            get(get_operation_state_policy),
        )
        .route(
            "/admin/operation-domains/:id/publish",
            post(publish_operation_domain_version),
        )
        .route(
            "/admin/operation-domains/:id/rollout",
            post(rollout_operation_domain_version),
        )
        .route(
            "/admin/operation-domains/:id/rollback",
            post(rollback_operation_domain_version),
        )
        .route(
            "/admin/operation-state-policies/:id/publish",
            post(publish_operation_state_policy_version),
        )
        .route(
            "/admin/operation-state-policies/:id/rollout",
            post(rollout_operation_state_policy_version),
        )
        .route(
            "/admin/operation-state-policies/:id/rollback",
            post(rollback_operation_state_policy_version),
        )
        .route(
            "/admin/taxonomies/:id/publish",
            post(publish_taxonomy_version),
        )
        .route(
            "/admin/taxonomies/:id/rollout",
            post(rollout_taxonomy_version),
        )
        .route(
            "/admin/taxonomies/:id/rollback",
            post(rollback_taxonomy_version),
        )
        // ── agent-autonomy-loop W4 / Task 5.6：outbox admin 路由 ─────────────
        .route("/admin/outbox", get(list_outbox))
        .route("/admin/outbox/:id/cancel", post(cancel_outbox))
        // ── Phase D / D5：lessons_learned admin 只读列表 ──────────────────────
        .route("/admin/lessons-learned", get(list_lessons_learned))
        // ── Phase D 收尾：lesson 一键晋升为 peer_case 候选 chunk（仍走 chunk
        //    review queue 二次确认；admin 手工触发，AI 永不自动 promote） ────
        .route(
            "/admin/lessons-learned/:lesson_id/promote-to-peer-case",
            post(promote_lesson_to_peer_case),
        )
        // ── Phase 0-D 自治信号 admin 聚合（lifecycle / revision_reason /
        //    reviewer_misjudge_signal / negative_example pending）只读 ────────
        .route("/admin/observability/phase-rollup", get(phase_rollup))
        // ── G-后续Ⅱ/2：worker 健康聚合（chat_tasks 状态 / gap_signals sweep 命中率 /
        //    lessons_learned 14d pattern × review_status）一次 RTT 拉齐 ──────
        .route("/admin/observability/worker-health", get(worker_health))
        // ── LLM provider 配置 admin 路由：前端 UI 编辑 / 测试 / 热切换 ────
        .route(
            "/admin/llm-providers",
            get(list_providers).post(create_provider),
        )
        .route(
            "/admin/llm-providers/:id",
            put(update_provider).delete(delete_provider),
        )
        .route(
            "/admin/llm-providers/:id/activate",
            post(activate_provider),
        )
        .route(
            "/admin/llm-providers/:id/vision",
            post(set_vision_active),
        )
        .route("/admin/llm-providers/test", post(test_provider))
        // ── knowledge-wiki Phase G：行业可配 schema admin 路由 ─────────────────
        .route(
            "/admin/domain-schemas",
            get(list_domain_schemas).post(create_domain_schema),
        )
        .route(
            "/admin/domain-schemas/:id",
            put(update_domain_schema).delete(delete_domain_schema),
        )
        .route(
            "/admin/domain-schemas/:id/activate",
            post(activate_domain_schema),
        )
        // ── agent-self-evolution M4 W4 / Task 5.5：evolution admin 路由 ──────
        .route("/evolution/experiments", get(list_evolution_experiments))
        .route(
            "/evolution/proposals/:id",
            get(get_evolution_proposal_detail),
        )
        .route(
            "/evolution/proposals/:id/release",
            post(release_evolution_proposal),
        )
        .route(
            "/evolution/proposals/:id/rollback",
            post(rollback_evolution_proposal),
        )
        // Phase C / C5：threshold 变更不可变审计日志（release/rollback/auto-release）。
        .route(
            "/evolution/threshold-overrides/audit",
            get(list_threshold_override_audit),
        )
        // Phase C / C3：evolution 灰度运行时开关。env `EVOLUTION_ENABLED=true`
        // 仍是最外层熔断；mongo flag 决定 contact 维度是否落桶。
        .route(
            "/evolution/runtime-flag",
            get(get_evolution_runtime_flag).put(put_evolution_runtime_flag),
        )
        // P0-D：session middleware 挂在所有 /api 路由上；白名单 /health + /auth/login。
        // 注意 layer 顺序——middleware 包住所有上面的 route，路径已剥 /api 前缀。
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::middleware::require_session,
        ))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    /// P2-6：routes/* 死路由 tripwire。
    ///
    /// 把 `pub async fn` 路由 handler 名单读出来，再与 `api_router()` 当前
    /// 静态文本中实际 mount 的字符串做比对——任何新增 `pub async fn` 但忘了
    /// 接进 router / 任何被取消 mount 但忘了删函数，都会让本测试 fail，
    /// 防止"实现了但不连"的死路由再次出现。
    ///
    /// 已显式列入 [`KNOWN_NON_ROUTE_HANDLERS`] 的属于"框架/复用 helper"，
    /// 不计入 mount 校验。
    #[test]
    fn no_orphan_pub_async_route_handlers() {
        let mod_src = include_str!("mod.rs");
        let route_files = [
            include_str!("accounts.rs"),
            include_str!("admin_ops_versions.rs"),
            include_str!("admin_outbox.rs"),
            include_str!("admin_state_policies.rs"),
            include_str!("admin_taxonomies.rs"),
            include_str!("admin_taxonomy_candidates.rs"),
            include_str!("assets.rs"),
            include_str!("auth.rs"),
            include_str!("behavior_signal_metrics.rs"),
            include_str!("chunk_locks.rs"),
            include_str!("contacts.rs"),
            include_str!("conversations.rs"),
            include_str!("domain_schemas.rs"),
            include_str!("domains.rs"),
            include_str!("evaluations.rs"),
            include_str!("events.rs"),
            include_str!("evolution.rs"),
            include_str!("guides.rs"),
            include_str!("health.rs"),
            include_str!("knowledge.rs"),
            include_str!("lessons_learned.rs"),
            include_str!("llm_providers.rs"),
            include_str!("management.rs"),
            include_str!("observability.rs"),
            include_str!("outcome_metrics.rs"),
            include_str!("outcomes_autonomy.rs"),
            include_str!("playbooks.rs"),
            include_str!("prompt_templates.rs"),
            include_str!("reviews.rs"),
            include_str!("shared.rs"),
            include_str!("simulations.rs"),
            include_str!("souls.rs"),
            include_str!("tasks.rs"),
        ];

        // 已知不是 axum handler 的 `pub async fn`：integration helper / WS handler。
        const KNOWN_NON_ROUTE_HANDLERS: &[&str] = &[
            // shared.rs：webhooks.rs / 集成测试通过 `pub use` 直接调用。
            "upsert_contact_from_value",
            // knowledge.rs：lib 内部复用的导入流水（不绑 HTTP）。
            "ingest_chunked_text",
            // knowledge.rs：PDF multipart handler 委托的字节级 helper（集成测试直调）。
            "import_pdf_bytes",
        ];

        let mut handlers: Vec<&str> = Vec::new();
        for src in &route_files {
            for line in src.lines() {
                let trimmed = line.trim_start();
                let prefix = if trimmed.starts_with("pub async fn ") {
                    "pub async fn "
                } else if trimmed.starts_with("pub(crate) async fn ") {
                    "pub(crate) async fn "
                } else {
                    continue;
                };
                let rest = &trimmed[prefix.len()..];
                if let Some(end) = rest.find('(') {
                    handlers.push(&rest[..end]);
                }
            }
        }

        let mut orphans: Vec<&str> = Vec::new();
        for h in &handlers {
            if KNOWN_NON_ROUTE_HANDLERS.contains(h) {
                continue;
            }
            // 在 mod.rs 中查 `(<name>)` 或 `::<name>)`（chunk_locks 命名空间）。
            let direct = format!("({h})");
            let nested = format!("::{h})");
            if !mod_src.contains(&direct) && !mod_src.contains(&nested) {
                orphans.push(h);
            }
        }

        assert!(
            orphans.is_empty(),
            "发现未挂载的 pub async fn 路由 handler（请在 api_router 中 .route 或加入 KNOWN_NON_ROUTE_HANDLERS）：{orphans:?}"
        );
    }
}

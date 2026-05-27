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
    llm::{LlmGenerator, LlmRegistry},
    mcp::McpClient,
};

mod accounts;
mod admin_outbox;
mod admin_taxonomies;
mod admin_taxonomy_candidates;
mod assets;
mod contacts;
mod conversations;
mod domain_schemas;
mod domains;
mod evaluations;
mod events;
mod evolution;
mod guides;
mod health;
mod knowledge;
mod llm_providers;
mod management;
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
pub use shared::upsert_contact_from_value;

use accounts::{list_accounts, sync_accounts, update_account_mcp_key};
use admin_outbox::{cancel_outbox, list_outbox};
use admin_taxonomies::{
    create_taxonomy, delete_taxonomy, list_taxonomies, patch_taxonomy,
};
use admin_taxonomy_candidates::{
    approve_taxonomy_candidate, list_taxonomy_candidates, reject_taxonomy_candidate,
};
use assets::{create_content_asset, list_content_assets};
use contacts::{
    analyze_contact_profile, disable_agent, enable_agent, get_contact, get_contact_memory_card,
    get_operating_memory, get_operation_health, import_contacts_endpoint,
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
    put_evolution_runtime_flag, release_evolution_proposal, rollback_evolution_proposal,
};
use guides::{apply_user_operation_guide, preview_user_operation_guide};
use health::health;
use llm_providers::{
    activate_provider, create_provider, delete_provider, list_providers, test_provider,
    update_provider,
};
use knowledge::{
    analyze_operation_knowledge_logs, answer_chunk_repair,
    apply_knowledge_gap_signal,
    archive_operation_knowledge_chunk, ask_knowledge,
    auto_verify_operation_knowledge_chunks,
    chat_apply, chat_discard,
    chat_history, chat_session_stream, chat_task_cancel, chat_task_create, chat_task_get,
    chat_turn, create_operation_knowledge,
    create_operation_knowledge_chunk, create_operation_knowledge_document,
    delete_operation_knowledge, delete_operation_knowledge_chunk,
    delete_operation_knowledge_document, digest_dismiss_card, digest_regenerate, digest_today,
    dismiss_knowledge_gap_signal,
    get_operation_knowledge_catalog, get_operation_knowledge_catalog_persisted,
    get_operation_knowledge_chunk_source, get_operation_knowledge_completeness,
    get_operation_knowledge_document, get_operation_knowledge_integrity_report,
    extract_operation_knowledge_tags, import_operation_knowledge_apply,
    import_operation_knowledge_preview, knowledge_inbox,
    list_knowledge_gap_signals,
    list_knowledge_usage,
    list_operation_knowledge, list_operation_knowledge_chunk_revisions,
    list_operation_knowledge_chunks,
    list_operation_knowledge_document_chunks, list_operation_knowledge_documents,
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
    verify_operation_knowledge_chunk,
};
use management::{
    create_management_session, get_management_command, get_tool_catalog, post_management_message,
};
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
    pub llm: Arc<dyn LlmGenerator>,
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
    pub second_reviewer_llm: Option<Arc<dyn LlmGenerator>>,
}

pub fn api_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
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
            "/operation-knowledge/:id",
            put(update_operation_knowledge).delete(delete_operation_knowledge),
        )
        .route("/decision-reviews", get(list_decision_reviews))
        .route("/decision-reviews/:id", get(get_decision_review))
        .route("/agent-outcome-metrics", get(list_agent_outcome_metrics))
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
        // ── agent-autonomy-loop W4 / Task 5.6：outbox admin 路由 ─────────────
        .route("/admin/outbox", get(list_outbox))
        .route("/admin/outbox/:id/cancel", post(cancel_outbox))
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
        // Phase C / C3：evolution 灰度运行时开关。env `EVOLUTION_ENABLED=true`
        // 仍是最外层熔断；mongo flag 决定 contact 维度是否落桶。
        .route(
            "/evolution/runtime-flag",
            get(get_evolution_runtime_flag).put(put_evolution_runtime_flag),
        )
        .with_state(state)
}

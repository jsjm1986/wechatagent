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

use crate::{config::AppConfig, db::Database, llm::LlmGenerator, mcp::McpClient};

mod accounts;
mod admin_outbox;
mod admin_taxonomies;
mod admin_taxonomy_candidates;
mod assets;
mod contacts;
mod conversations;
mod domains;
mod evaluations;
mod events;
mod evolution;
mod guides;
mod health;
mod knowledge;
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
    get_evolution_proposal_detail, list_evolution_experiments, release_evolution_proposal,
    rollback_evolution_proposal,
};
use guides::{apply_user_operation_guide, preview_user_operation_guide};
use health::health;
use knowledge::{
    answer_chunk_repair, auto_verify_operation_knowledge_chunks, chat_apply, chat_discard,
    chat_history, chat_turn, create_operation_knowledge,
    create_operation_knowledge_chunk, create_operation_knowledge_document,
    delete_operation_knowledge, delete_operation_knowledge_chunk,
    delete_operation_knowledge_document, digest_today, get_operation_knowledge_catalog,
    get_operation_knowledge_chunk_source, get_operation_knowledge_completeness,
    get_operation_knowledge_document, get_operation_knowledge_integrity_report,
    extract_operation_knowledge_tags, import_operation_knowledge_apply,
    import_operation_knowledge_preview, list_knowledge_usage,
    list_operation_knowledge, list_operation_knowledge_chunks,
    list_operation_knowledge_document_chunks, list_operation_knowledge_documents,
    open_operation_knowledge_slices, propose_chunk_repair, propose_pack_repair,
    record_repair_apply, refresh_operation_knowledge_completeness,
    reject_operation_knowledge_chunk, search_operation_knowledge_tool,
    test_operation_knowledge_match, update_operation_knowledge,
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
    pub config: AppConfig,
    /// agent-self-evolution M4 W4 Task 5.3：prompt 包版本号。
    ///
    /// `prompts::ensure_prompt_pack_v2` / `ensure_evolution_prompt_pack_v1`
    /// 在末尾各 fetch_add 一次；`evolution::release::release_prompt`
    /// commit 后 fetch_add 一次。`agent::generate_agent_json` 把当前值折进
    /// LRU cache key，让 release/seed/rollback 任一动作 atomic 触发缓存失效，
    /// 不需要重启进程。
    pub prompt_pack_version: Arc<std::sync::atomic::AtomicU64>,
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
        .route(
            "/operation-knowledge/catalog",
            get(get_operation_knowledge_catalog),
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
            "/operation-knowledge/items/:id/repair",
            post(propose_pack_repair),
        )
        .route(
            "/operation-knowledge/repair/applied",
            post(record_repair_apply),
        )
        .route("/operation-knowledge/chat", post(chat_turn))
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
        .with_state(state)
}

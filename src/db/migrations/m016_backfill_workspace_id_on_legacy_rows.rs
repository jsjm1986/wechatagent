//! 2026_05_G1_001（Phase G P1-1）：legacy 行 `workspace_id` 字段缺失回填。
//!
//! P1-1 多租户联邦把 handler 路径上 `state.config.default_workspace_id` 全部
//! 换成 `admin.current_workspace`。`AppState` 不再兜底 ws_id；从此每条业务行
//! 必须自带 `workspace_id`。在那之前用 single-tenant 起步的环境，旧行多数
//! 直接没写 `workspace_id` 字段，迁移后会被多租户过滤无差别黑掉。
//!
//! 这条 migration 扫所有自带 `workspace_id` 的业务集合（来自 `Database` 的
//! typed accessors），把 `workspace_id: { $exists: false }` 全部 `$set` 为
//! `DEFAULT_WORKSPACE_ID`（默认 `"default"`），同时兼容 camelCase 写法
//! `workspaceId`（早期 P0 鉴权 / LLM 服务商等少数集合用了 BSON camelCase）。
//!
//! 生产守卫：`APP_ENV=production` 时 noop 返回（不自动 backfill）——P1-1 在生产
//! 打开多租户前，运维必须显式 backfill。与 m014 同款 warn+Ok 形态：返回 Err 会在
//! `mod.rs::run_with` 记录迁移前中断，迁移永不入账，每次启动重试重错（boot-brick），
//! 且运维手工 backfill 后仍因未入账而再次砖机，无干净恢复路径。
//!
//! 幂等：仅修改 `$exists: false` 的文档；二次执行 matched=0 即可。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

/// 业务侧 BSON 用 snake_case `workspace_id` 的集合（绝大多数）。
const SNAKE_CASE_COLLECTIONS: &[&str] = &[
    "accounts",
    "contacts",
    "conversation_messages",
    "agent_tasks",
    "agent_events",
    "content_assets",
    "agent_souls",
    "operation_playbooks",
    "operation_domain_configs",
    "operation_state_policies",
    "prompt_templates",
    "operating_memories",
    "operation_knowledge_documents",
    "operation_knowledge_chunks",
    "knowledge_usage_logs",
    "knowledge_chat_turns",
    "knowledge_daily_reports",
    "knowledge_operator_memory",
    "decision_reviews",
    "agent_run_logs",
    "llm_call_logs",
    "memory_candidates",
    "user_operation_guide_previews",
    "management_sessions",
    "management_messages",
    "command_runs",
    "tool_calls",
    "outcome_metrics",
    "evaluation_scenarios",
    "experiments",
    "proposals",
    "shadow_replays",
    "threshold_overrides",
    "threshold_overrides_audit",
    "post_release_reviews",
    "evolution_runtime_flags",
    "chunk_revisions",
    "knowledge_gap_signals",
    "domain_schemas",
    "catalog_rebuild_jobs",
];

/// 用 camelCase `workspaceId` 的集合（P0 鉴权 / LLM 服务商等）。
const CAMEL_CASE_COLLECTIONS: &[&str] = &[
    "llm_provider_configs",
    "admin_users",
];

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    if std::env::var("APP_ENV").unwrap_or_default() == "production" {
        tracing::warn!(
            migration_id = "2026_05_X1_001_backfill_workspace_id_on_legacy_rows",
            "production guard: skipped workspace_id backfill; run manually before enabling multi-tenant filtering"
        );
        return Ok(());
    }
    let default_ws = std::env::var("DEFAULT_WORKSPACE_ID").unwrap_or_else(|_| "default".into());
    let raw = db.raw();

    for name in SNAKE_CASE_COLLECTIONS {
        let coll = raw.collection::<Document>(name);
        let result = coll
            .update_many(
                doc! { "workspace_id": { "$exists": false } },
                doc! { "$set": { "workspace_id": &default_ws } },
                None,
            )
            .await?;
        tracing::info!(
            migration_id = "2026_05_X1_001_backfill_workspace_id_on_legacy_rows",
            collection = *name,
            modified = result.modified_count,
            field = "workspace_id",
            "backfilled missing workspace_id"
        );
    }

    for name in CAMEL_CASE_COLLECTIONS {
        let coll = raw.collection::<Document>(name);
        let result = coll
            .update_many(
                doc! { "workspaceId": { "$exists": false } },
                doc! { "$set": { "workspaceId": &default_ws } },
                None,
            )
            .await?;
        tracing::info!(
            migration_id = "2026_05_X1_001_backfill_workspace_id_on_legacy_rows",
            collection = *name,
            modified = result.modified_count,
            field = "workspaceId",
            "backfilled missing workspaceId"
        );
    }

    Ok(())
}

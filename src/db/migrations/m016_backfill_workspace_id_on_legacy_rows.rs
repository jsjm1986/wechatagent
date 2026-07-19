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

/// 业务侧 BSON 用 snake_case `workspace_id` 的集合(绝大多数)。
/// 名字须与 `src/db/mod.rs` 的 accessor 集合名字面量逐字一致;每个集合的 model
/// 都有 `pub workspace_id: String` 且 struct 头无 `#[serde(rename_all)]`(见
/// tests::KNOWN_SNAKE_TENANT_COLLECTIONS 审计基准)。
const SNAKE_CASE_COLLECTIONS: &[&str] = &[
    "wechat_accounts",
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
    "agent_decision_reviews",
    "agent_run_logs",
    "llm_call_logs",
    "memory_candidates",
    "user_operation_guide_previews",
    "management_agent_sessions",
    "management_agent_messages",
    "agent_command_runs",
    "agent_tool_calls",
    "agent_outcome_metrics",
    "evaluation_scenarios",
    "experiments",
    "proposals",
    "shadow_replays",
    "threshold_overrides",
    "threshold_overrides_audit",
    "post_release_reviews",
    "evolution_runtime_flags",
    "knowledge_gap_signals",
    "domain_schemas",
    "catalog_rebuild_jobs",
    "behavior_signals",
    "behavior_signal_metrics",
    "mcp_call_logs",
    "referral_cards",
    "agent_send_ledger",
    "ingest_sources",
    "domain_profiles",
    "agent_send_outbox",
    "relationship_type_suggestions",
    "suspected_deal_signals",
    "agent_principal_escalations",
    "knowledge_chat_tasks",
    "products",
];

/// 用 camelCase `workspaceId` 的集合——对应 model struct 头带
/// `#[serde(rename_all="camelCase")]`,故 `workspace_id` 序列化成 `workspaceId`
/// (见 tests::KNOWN_CAMEL_TENANT_COLLECTIONS 审计基准)。
/// 注意:`admin_users` 不在此表——AdminUser 用 `workspaces:Vec<String>` 而非单值
/// workspaceId(auth/mod.rs:28-39),不符合单值回填契约。
const CAMEL_CASE_COLLECTIONS: &[&str] = &["llm_provider_configs", "campaigns", "campaign_sends"];

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

#[cfg(test)]
mod tests {
    use super::{CAMEL_CASE_COLLECTIONS, SNAKE_CASE_COLLECTIONS};
    use std::collections::HashSet;

    /// 审计定稿:真实携带**单值 snake_case `workspace_id`** 字段的 Mongo 集合名全集。
    /// 来源=对 `src/models.rs` 各 struct 的逐一核对(spec §2,每条附 file:line):
    /// struct 有 `pub workspace_id: String` 且 struct 头无 `#[serde(rename_all)]`。
    /// **不是**从下方 `SNAKE_CASE_COLLECTIONS` 抄来的——这是"允许被回填 snake workspace_id
    /// 的集合宇宙",下方表是"m016 实际回填目标",目标必须是本宇宙的子集。
    /// 新增真实 snake 租户集合时,先在这里登记(带 file:line),再决定是否加入回填表。
    const KNOWN_SNAKE_TENANT_COLLECTIONS: &[&str] = &[
        "wechat_accounts",
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
        "agent_decision_reviews",
        "agent_run_logs",
        "llm_call_logs",
        "memory_candidates",
        "user_operation_guide_previews",
        "management_agent_sessions",
        "management_agent_messages",
        "agent_command_runs",
        "agent_tool_calls",
        "agent_outcome_metrics",
        "evaluation_scenarios",
        "experiments",
        "proposals",
        "shadow_replays",
        "threshold_overrides",
        "threshold_overrides_audit",
        "post_release_reviews",
        "evolution_runtime_flags",
        "knowledge_gap_signals",
        "domain_schemas",
        "catalog_rebuild_jobs",
        "behavior_signals",
        "behavior_signal_metrics",
        "mcp_call_logs",
        "referral_cards",
        "agent_send_ledger",
        "ingest_sources",
        "domain_profiles",
        "agent_send_outbox",
        "relationship_type_suggestions",
        "suspected_deal_signals",
        "agent_principal_escalations",
        "knowledge_chat_tasks",
        "products",
    ];

    /// 审计定稿:真实携带**单值 camelCase `workspaceId`** 字段的集合。
    /// 判据=对应 struct 头带 `#[serde(rename_all="camelCase")]`(spec §2):
    /// LlmProviderConfig(models.rs:4732)/ Campaign(552)/ CampaignSend(596)。
    const KNOWN_CAMEL_TENANT_COLLECTIONS: &[&str] =
        &["llm_provider_configs", "campaigns", "campaign_sends"];

    /// 无单值 workspace_id 字段、绝不该进任一回填表(防回退,spec §2.C):
    /// `admin_users` 用 `workspaces:Vec<String>`(auth/mod.rs:28-39);
    /// `chunk_revisions` 无 ws 字段、靠 chunk_id 反查租户(models.rs:1613-1632)。
    const MUST_NOT_BACKFILL: &[&str] = &["admin_users", "chunk_revisions"];

    /// 挡拼错 + 挡 snake/camel 归错类:SNAKE 表每个名字都必须 ∈ snake 审计全集。
    /// (拼错真实名 → 不在全集;或该集合其实是 camelCase → 也不在 snake 全集而在 camel 全集。)
    #[test]
    fn snake_table_names_are_all_known_snake_tenant_collections() {
        let known: HashSet<&str> = KNOWN_SNAKE_TENANT_COLLECTIONS.iter().copied().collect();
        for name in SNAKE_CASE_COLLECTIONS {
            assert!(
                known.contains(name),
                "SNAKE_CASE_COLLECTIONS 含 `{name}`,但它不在 KNOWN_SNAKE_TENANT_COLLECTIONS 审计全集内\
                 (要么拼错了真实集合名,要么该集合其实是 camelCase workspaceId 应进 CAMEL 表)"
            );
        }
    }

    /// 挡拼错 + 挡 camel/snake 归错类:CAMEL 表每个名字都必须 ∈ camel 审计全集。
    #[test]
    fn camel_table_names_are_all_known_camel_tenant_collections() {
        let known: HashSet<&str> = KNOWN_CAMEL_TENANT_COLLECTIONS.iter().copied().collect();
        for name in CAMEL_CASE_COLLECTIONS {
            assert!(
                known.contains(name),
                "CAMEL_CASE_COLLECTIONS 含 `{name}`,但它不在 KNOWN_CAMEL_TENANT_COLLECTIONS 审计全集内\
                 (要么拼错,要么该集合其实是 snake workspace_id 应进 SNAKE 表)"
            );
        }
    }

    /// 无空串、无重复、两表不相交(同一集合不会被回填两次/写两种字段名)。
    #[test]
    fn tables_have_no_empty_no_duplicates_and_are_disjoint() {
        let mut seen: HashSet<&str> = HashSet::new();
        for name in SNAKE_CASE_COLLECTIONS
            .iter()
            .chain(CAMEL_CASE_COLLECTIONS.iter())
        {
            assert!(!name.is_empty(), "集合名表含空串");
            assert!(
                seen.insert(name),
                "集合名 `{name}` 在 m016 两张表里重复出现(跨表或表内)"
            );
        }
    }

    /// 防回退:无单值 workspace_id 的集合绝不该出现在任一回填表里。
    #[test]
    fn collections_without_single_workspace_id_are_never_backfilled() {
        let snake: HashSet<&str> = SNAKE_CASE_COLLECTIONS.iter().copied().collect();
        let camel: HashSet<&str> = CAMEL_CASE_COLLECTIONS.iter().copied().collect();
        for name in MUST_NOT_BACKFILL {
            assert!(
                !snake.contains(name),
                "`{name}` 无单值 workspace_id,不该在 SNAKE_CASE_COLLECTIONS"
            );
            assert!(
                !camel.contains(name),
                "`{name}` 无单值 workspace_id,不该在 CAMEL_CASE_COLLECTIONS"
            );
        }
    }
}

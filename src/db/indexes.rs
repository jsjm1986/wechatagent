//! 索引创建集合。
//!
//! 所有索引创建语句集中在 [`ensure_all`]，由 [`super::Database::ensure_indexes`]
//! 调用。运行时其它路径不应该再调用 `create_index`。

use mongodb::{bson::doc, options::IndexOptions, IndexModel};

use super::Database;

pub(super) async fn ensure_all(db: &Database) -> anyhow::Result<()> {
    db.accounts()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.accounts()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "app_id": 1 })
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
            None,
        )
        .await?;
    db.contacts()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "wxid": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "message_id": 1 })
                .options(IndexOptions::builder().sparse(true).unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "dedupe_key": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "dedupe_key": { "$type": "string" } })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "run_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "contact_wxid": 1,
                    "kind": 1,
                    "status": 1
                })
                .build(),
            None,
        )
        .await?;
    db.events()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "contact_wxid": 1,
                    "created_at": -1
                })
                .build(),
            None,
        )
        .await?;
    db.content_assets()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "kind": 1, "updated_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.agent_souls()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "agent_kind": 1, "status": 1, "version": -1 })
                .build(),
            None,
        )
        .await?;
    db.operation_playbooks()
        .create_index(
            IndexModel::builder()
                .keys(
                    doc! { "workspace_id": 1, "account_id": 1, "is_default": 1, "updated_at": -1 },
                )
                .build(),
            None,
        )
        .await?;
    db.prompt_templates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "prompt_key": 1, "status": 1, "version": -1 })
                .build(),
            None,
        )
        .await?;
    db.operation_domain_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.operating_memories()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_items()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "domain": 1, "category": 1, "status": 1, "priority": -1, "updated_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_documents()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "domain": 1, "status": 1, "updated_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "domain": 1, "status": 1, "priority": -1, "updated_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "document_id": 1, "item_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    // 知识标签快路径：inbound 子串匹配触发关键词时直接召回 chunk，
    // 跳过/优先于 LLM Planner 选择。sparse 避免老文档无 trigger_keywords 时占索引空间。
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "trigger_keywords": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kchunks_trigger_keywords_idx".to_string())
                        .sparse(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_usage_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.knowledge_chat_turns()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "session_id": 1, "turn_index": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kchat_turns_session_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_chat_turns()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("kchat_turns_recent_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.decision_reviews()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.decision_reviews()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "contact_wxid": 1,
                    "status": 1,
                    "outcome_status": 1
                })
                .options(
                    IndexOptions::builder()
                        .partial_filter_expression(
                            doc! { "outcome_status": { "$in": ["pending", "analyzing"] } },
                        )
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "run_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    // ── agent-autonomy-loop W0 (Task 1.2) / W6 (Task 7.1) ──
    //
    // R0.8 / R9.5 监控查询索引。BSON key 使用 snake_case，与 `AgentRunLog`
    // 字段未加 `#[serde(rename = ...)]` 的 snake_case 约定一致。
    //
    // W6 修订（Task 7.1）：W0 设计稿规划了 `started_at`，但 W1 落地后
    // `AgentRunLog` 顶层只写 `created_at`（`planner.started_at` 是嵌套 Document
    // 字段，不能作为顶层索引 key 使用）。`outcomes_autonomy::build_horizon_filter`
    // 实际过滤的就是 `created_at`，因此索引在此对齐到 `created_at`，避免
    // 监控聚合走 collection scan。
    //
    // 已部署集群可能仍残留 W0 创建的同形 `started_at` 索引（不会命中任何
    // 文档，是空索引）；它不阻塞写入，可在维护窗口手工 dropIndex。
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "lifecycle": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "final_review_status": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "autonomy_mode": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.llm_call_logs()
        .create_index(
            IndexModel::builder()
                .keys(
                    doc! { "workspace_id": 1, "account_id": 1, "prompt_key": 1, "created_at": -1 },
                )
                .build(),
            None,
        )
        .await?;
    db.llm_call_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "run_id": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.memory_candidates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "status": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.user_operation_guide_previews()
        .create_index(
            IndexModel::builder()
                .keys(
                    doc! { "workspace_id": 1, "account_id": 1, "contact_id": 1, "created_at": -1 },
                )
                .build(),
            None,
        )
        .await?;
    db.management_messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "created_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.command_runs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.tool_calls()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "command_run_id": 1, "created_at": 1 })
                .build(),
            None,
        )
        .await?;
    // S-19 / Task 17：outcome metrics TTL 索引（默认 90 天）。
    let ttl_days: u64 = std::env::var("OUTCOME_METRICS_TTL_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    db.outcome_metrics()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::from_secs(ttl_days * 24 * 60 * 60))
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.outcome_metrics()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "horizon": 1, "date": -1 })
                .build(),
            None,
        )
        .await?;
    // S-18 / Task 18：evaluation_scenarios 唯一索引（scenario_id 在 workspace 内唯一）。
    db.evaluation_scenarios()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "scenario_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    // ── agent-autonomy-loop W0 (Task 1.2) ──
    //
    // 三个新增 collection 的索引集中在专属 helper：保持 ensure_all 主流程精简，
    // 同时方便 W3 / W4 在落地业务字段时按需新增索引（如 outbox 的 ttl / 字典的
    // alias 命中索引）。
    ensure_agent_send_outbox_indexes(db).await?;
    ensure_system_taxonomies_indexes(db).await?;
    ensure_taxonomy_candidates_indexes(db).await?;
    // ── agent-self-evolution W0 (Task 1.2) ──
    ensure_evolution_indexes(db).await?;
    Ok(())
}

/// agent-autonomy-loop W0 / R13.1：`agent_send_outbox` 索引。
///
/// - `(account_id, status, next_retry_at)`：dispatcher worker 扫描待发送条目。
/// - `idempotency_key` 唯一：强幂等门，DuplicateKey 视为 `IdempotentSkip`。
/// - `(status, locked_until)`：崩溃恢复扫描过期 lease。
/// - `(source_event_id, contact_wxid)`：按入站事件追溯发送链路。
async fn ensure_agent_send_outbox_indexes(db: &Database) -> anyhow::Result<()> {
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "status": 1, "next_retry_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "idempotency_key": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "locked_until": 1 })
                .build(),
            None,
        )
        .await?;
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "source_event_id": 1, "contact_wxid": 1 })
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// agent-autonomy-loop W0 / R8.1：`system_taxonomies` 索引。
///
/// `(scope, kind, value.id)` 唯一：保证同一 scope+kind 下 value.id 唯一，支持
/// `2026_05_006_taxonomy_seed` 迁移幂等以及后台 API approve 的 upsert 写入。
async fn ensure_system_taxonomies_indexes(db: &Database) -> anyhow::Result<()> {
    db.collection_system_taxonomies()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "scope": 1, "kind": 1, "value.id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// agent-autonomy-loop W0 / R8.3：`taxonomy_candidates` 索引。
///
/// - `(scope, kind, status)`：后台列表 `?status=pending` 查询。
/// - `(scope, kind, raw_value)` 唯一：`upsert_candidate` 幂等键，重复值仅累加
///   `occurrences` / 更新 `last_seen_at`。
async fn ensure_taxonomy_candidates_indexes(db: &Database) -> anyhow::Result<()> {
    db.collection_taxonomy_candidates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "scope": 1, "kind": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    db.collection_taxonomy_candidates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "scope": 1, "kind": 1, "raw_value": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// agent-self-evolution W0 (Task 1.2)：5 张新 collection + prompt_templates
/// 多版本辅助索引。
///
/// - `experiments`：`(workspace_id, account_id, started_at desc)` 列表查询；
///   `(experiment_id)` 唯一保证 envelope 不重复 insert（Requirements 1.3）。
/// - `proposals`：`(workspace_id, account_id, status, created_at desc)` 后台
///   按状态分页；`(experiment_id)` 反查 cohort 下所有 proposal（Requirements 5.x）。
/// - `shadow_replays`：`(proposal_id)` 聚合；`(workspace_id, account_id,
///   started_at desc)` 后台监控（Requirements 5.x）。
/// - `threshold_overrides`：`(workspace_id, account_id, gate_key, released_at
///   desc)` 是 `resolve_thresholds` 取最新有效值的核心查询路径（Requirements 6.2）。
/// - `prompt_templates` 多版本支持：`(workspace_id, prompt_key, current_version)`
///   过滤 current 那条；`(workspace_id, prompt_key, version)` 唯一保证同 key
///   下版本号不冲突（Requirements 6.4 / 6.5）。
async fn ensure_evolution_indexes(db: &Database) -> anyhow::Result<()> {
    // experiments
    db.experiments()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "started_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.experiments()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "experiment_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;

    // proposals
    db.proposals()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "status": 1,
                    "created_at": -1,
                })
                .build(),
            None,
        )
        .await?;
    db.proposals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "experiment_id": 1 })
                .build(),
            None,
        )
        .await?;

    // shadow_replays
    db.shadow_replays()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "proposal_id": 1 })
                .build(),
            None,
        )
        .await?;
    db.shadow_replays()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "started_at": -1,
                })
                .build(),
            None,
        )
        .await?;

    // threshold_overrides
    db.threshold_overrides()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "gate_key": 1,
                    "released_at": -1,
                })
                .build(),
            None,
        )
        .await?;

    // post_release_reviews（W4 Task 5.6 一并加，避免 W4 再补一波索引）
    db.post_release_reviews()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "scheduled_at": 1,
                    "completed": 1,
                })
                .build(),
            None,
        )
        .await?;
    db.post_release_reviews()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "proposal_id": 1 })
                .build(),
            None,
        )
        .await?;

    // prompt_templates 多版本辅助：(workspace_id, prompt_key, current_version)
    // 用于 ensure_prompt_pack_v2 + release_prompt 在同 key 下定位 current 那条；
    // (workspace_id, prompt_key, version) 唯一保证多版本不冲突。
    db.prompt_templates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "prompt_key": 1, "current_version": 1 })
                .build(),
            None,
        )
        .await?;
    db.prompt_templates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "prompt_key": 1, "version": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;

    Ok(())
}

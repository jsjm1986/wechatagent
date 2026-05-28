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
    // Phase B / B4：operation_state_policies 唯一索引——
    //   (workspace_id, domain, state_key) 复合 unique。
    // enforce 路径单次 find_one，索引保命中。
    db.operation_state_policies()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1, "state_key": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    // Phase E5-T1：ops 三表 active_versions 灰度——
    //   把 (workspace_id, domain[, state_key/value.id]) 旧 unique 索引下线，
    //   换成包含 `version` 的 4-tuple unique，让多版本可同时驻留 collection。
    //   `(..., current_version=true)` 部分索引快路径，给读路径筛 active 集合。
    ensure_ops_versioned_indexes(db).await?;    db.operating_memories()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1 })
                .options(IndexOptions::builder().unique(true).build())
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
    // LLM 服务商配置：(workspace_id, provider_id) 唯一；is_active 部分索引便于
    // 启动时快速取出当前 active 记录。
    ensure_llm_provider_indexes(db).await?;
    Ok(())
}

async fn ensure_llm_provider_indexes(db: &Database) -> anyhow::Result<()> {
    // 历史遗留：早期版本错误地用 snake_case 字段建过 unique 索引，
    // 但模型 BSON 层是 camelCase → 旧索引把所有真实文档当成
    // (workspace_id=null, provider_id=null) 重复键。开机时 best-effort drop。
    let _ = db
        .llm_provider_configs()
        .drop_index("workspace_id_1_provider_id_1", None)
        .await;
    let _ = db
        .llm_provider_configs()
        .drop_index("workspace_id_1_is_active_1", None)
        .await;
    db.llm_provider_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspaceId": 1, "providerId": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.llm_provider_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspaceId": 1, "isActive": 1 })
                .build(),
            None,
        )
        .await?;
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
/// 历史上 `(scope, kind, value.id)` 直接走 unique，保证 seed migration 与 admin
/// approve upsert 幂等。Phase E5-T1 引入 active_versions 灰度后，唯一性维度变成
/// `(scope, kind, value.id, version)`，由 [`ensure_ops_versioned_indexes`] 创建；
/// 这里只保留非唯一辅助索引（按 (scope, kind, status) 列字典），列表查询命中。
async fn ensure_system_taxonomies_indexes(db: &Database) -> anyhow::Result<()> {
    db.collection_system_taxonomies()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "scope": 1, "kind": 1, "value.status": 1 })
                .options(
                    IndexOptions::builder()
                        .name("sys_tax_scope_kind_status_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// Phase E5-T1：ops 三表 active_versions 灰度索引切换。
///
/// 旧形态：(workspace_id, domain) / (workspace_id, domain, state_key) /
/// (scope, kind, value.id) 三个 unique 索引一一对应一行；同 key 不能同时存在
/// 多个版本，无法做灰度。
///
/// 新形态：用 `version: i32` 把 unique 索引扩到 4-tuple，多版本同时驻留；读
/// 路径用 `(workspace_id, domain[, state_key/value.id], current_version=true)`
/// 部分索引筛 active 集合。配合 `ab_bucket_for_contact(contact_id)` 选 active
/// 集合中的某一个。
///
/// 二次启动安全：`drop_index` 用 best-effort（旧索引可能在升级前已被运维手工
/// 清理，也可能本就不存在），失败不阻塞 ensure_all 主流程；新 unique 索引
/// 由 MongoDB 在已存在时静默 noop。
async fn ensure_ops_versioned_indexes(db: &Database) -> anyhow::Result<()> {
    // ── operation_domain_configs ──
    let _ = db
        .operation_domain_configs()
        .drop_index("workspace_id_1_domain_1", None)
        .await;
    db.operation_domain_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1, "version": 1 })
                .options(
                    IndexOptions::builder()
                        .name("op_domain_ws_domain_version_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_domain_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1, "current_version": 1 })
                .options(
                    IndexOptions::builder()
                        .name("op_domain_ws_domain_current_idx".to_string())
                        .partial_filter_expression(doc! { "current_version": true })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── operation_state_policies ──
    let _ = db
        .operation_state_policies()
        .drop_index("workspace_id_1_domain_1_state_key_1", None)
        .await;
    db.operation_state_policies()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "domain": 1,
                    "state_key": 1,
                    "version": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("op_state_policy_ws_domain_state_version_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_state_policies()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "domain": 1,
                    "state_key": 1,
                    "current_version": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("op_state_policy_ws_domain_state_current_idx".to_string())
                        .partial_filter_expression(doc! { "current_version": true })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── system_taxonomies ──
    //
    // 旧 (scope, kind, value.id) unique 索引由 ensure_system_taxonomies_indexes
    // 继续创建（兼容旧路径），这里只补 4-tuple 与 current_version 部分索引。
    // 多版本驻留时旧 unique 会冲突 —— 改为非唯一时机由 W5/W6 一并迁移。
    let _ = db
        .collection_system_taxonomies()
        .drop_index("scope_1_kind_1_value.id_1", None)
        .await;
    db.collection_system_taxonomies()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "scope": 1,
                    "kind": 1,
                    "value.id": 1,
                    "version": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("sys_tax_scope_kind_value_version_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.collection_system_taxonomies()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "scope": 1,
                    "kind": 1,
                    "value.id": 1,
                    "current_version": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("sys_tax_scope_kind_value_current_idx".to_string())
                        .partial_filter_expression(doc! { "current_version": true })
                        .build(),
                )
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

    // ── knowledge-digest-workstation ──
    //
    // knowledge_daily_reports：(workspace_id, account_id, report_date) 三元组
    // 复合 unique，保证一天一份；同时支持按 (account_id, report_date desc) 拉
    // 当日 / 最近 N 天日报。
    db.knowledge_daily_reports()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "report_date": -1,
                })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;

    // knowledge_chat_tasks：worker 取 pending 用 (status, created_at)；
    // chat 面板按 sessionId 拉历史用 (session_id, status)。
    db.knowledge_chat_tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "created_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.knowledge_chat_tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;

    // knowledge_operator_memory：chat 注入按
    // (account_id, operator_id, last_used_at desc) 拉 top N。
    db.knowledge_operator_memory()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "operator_id": 1,
                    "last_used_at": -1,
                })
                .build(),
            None,
        )
        .await?;

    // P1-9：knowledge_operator_memory.expires_at 上挂 TTL 索引（expireAfterSeconds=0）。
    // MongoDB 后台进程会在 `expires_at < now()` 时把对应文档自动删除——长期跑下
    // 来运营 memory 不会无界堆积；`expires_at == None` 的文档不会被 TTL 命中
    // （MongoDB TTL 只清理 BSON Date 字段，缺失字段会被忽略）。
    // 名字 `kop_memory_expires_ttl` 显式标记，避免与上面的 last_used_at 索引误并。
    db.knowledge_operator_memory()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kop_memory_expires_ttl".to_string())
                        .expire_after(std::time::Duration::from_secs(0))
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── knowledge-wiki Phase A：4 个新 collection 的索引 + chunks 新字段索引 ──
    //
    // 这一组索引服务"四件事"的检索面：
    //   * chunk_revisions：按 chunk_id 时间倒序读 timeline；按 created_at 全局
    //     扫"最近 N 条"；
    //   * knowledge_gap_signals：worker 拉 pending 任务、admin 看 timeline；
    //   * domain_schemas：workspace+schema_id+version 唯一标识，加 is_active
    //     快路径；
    //   * catalog_rebuild_jobs：workspace+status+queued_at 决定 worker 取哪批；
    //   * operation_knowledge_chunks 三条新查询路径：按 wiki_type 分组、按
    //     valid_to 找 stale、按 dynamic_confidence 取 top。
    //
    // 旧 chunks 索引（document_id+item_id+status / status+priority）
    // 仍然保留，召回算法零改动。
    db.chunk_revisions()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "chunk_id": 1, "revision_id": -1 })
                .options(
                    IndexOptions::builder()
                        .name("chunk_revisions_chunk_rev_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.chunk_revisions()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("chunk_revisions_created_at_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_gap_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "status": 1, "kind": 1 })
                .options(
                    IndexOptions::builder()
                        .name("gap_signals_status_kind_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_gap_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("gap_signals_created_at_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_gap_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "signal_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("gap_signals_signal_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // LintView dashboard：按 (kind, status) 分组的时间线视图。
    // 与 gap_signals_status_kind_idx 的差异是字段顺序与排序键 —— 前端
    // /api/knowledge/gap-signals?kind=X 直接走这条避免 in-memory sort。
    db.knowledge_gap_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "kind": 1, "status": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("gap_signals_kind_status_created_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.domain_schemas()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "schema_id": 1, "version": -1 })
                .options(
                    IndexOptions::builder()
                        .name("domain_schemas_ws_id_version_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.domain_schemas()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "is_active": 1 })
                .options(
                    IndexOptions::builder()
                        .name("domain_schemas_ws_active_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.catalog_rebuild_jobs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "status": 1, "queued_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name("catalog_jobs_status_queued_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.catalog_rebuild_jobs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "job_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("catalog_jobs_job_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "wiki_type": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kchunks_wiki_type_idx".to_string())
                        .sparse(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "valid_to": 1, "status": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kchunks_valid_to_idx".to_string())
                        .sparse(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "dynamic_confidence": -1 })
                .options(
                    IndexOptions::builder()
                        .name("kchunks_dynamic_confidence_idx".to_string())
                        .sparse(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── P0 鉴权 / Session ─────────────────────────────────────────────────
    // admin_users.username unique：登录路径按 username 查；同名禁止。
    db.raw()
        .collection::<mongodb::bson::Document>("admin_users")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "username": 1 })
                .options(
                    IndexOptions::builder()
                        .name("admin_users_username_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // admin_sessions.session_id unique：cookie 唯一定位 session。
    db.raw()
        .collection::<mongodb::bson::Document>("admin_sessions")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "session_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("admin_sessions_session_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // admin_sessions.expires_at TTL：mongo 自动清理过期 session。
    // expireAfterSeconds=0 表示「字段时间到达即过期」（不是字段时间 + N 秒）。
    db.raw()
        .collection::<mongodb::bson::Document>("admin_sessions")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name("admin_sessions_ttl".to_string())
                        .expire_after(std::time::Duration::from_secs(0))
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    Ok(())
}

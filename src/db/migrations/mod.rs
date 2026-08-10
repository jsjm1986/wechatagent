//! 版本化数据迁移：启动时幂等执行未应用的迁移。
//!
//! 每条迁移在 `migrations` 集合留下 [`MigrationRecord`]，下次启动跳过已应用项。
//! 迁移本身必须幂等（即使标记丢失，重跑也不破坏数据），以便支持回滚后重跑。
//!
//! 使用方式：
//! ```text
//! let db = Database::connect(...).await?;
//! db::migrations::run(&db).await?;   // 先迁移
//! db.ensure_indexes().await?;        // 再建索引
//! ```
//!
//! 模块布局：每条迁移单独一个 `mNNN_*` 文件，每个文件导出
//! `pub(super) async fn run_step(db: &Database) -> AppResult<()>`；
//! 跨 step 共享的纯函数 helper 集中在 [`helpers`] 子模块，便于直接单测。

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use mongodb::{
    bson::{doc, DateTime},
    options::{TransactionOptions, UpdateOptions},
};

use super::Database;
use crate::error::{AppError, AppResult};

mod helpers;

mod m001_split_last_message_at;
mod m002_split_active_facts;
mod m003_state_machine_allowed_from;
mod m004_outcome_metrics_id;
mod m005_memory_facts_to_structured;
/// `pub`：集成测试需在 `migrations::run` 后直接调用 `m006::run_step` 重新 seed 销售域
/// 字典——`m012_drop_legacy_taxonomy_seed` 在非 production 环境会删掉 customer_stage /
/// intent_level / objection_type 的 m006 seed（生产靠 `APP_ENV=production` 守卫跳过），
/// 测试 DB 跑完全部迁移后这三 kind 字典为空，经 `validate_dimension_value(Taxonomy)`
/// 的维度会被判越界 drop。测试复用 m006 的 upsert seed（与生产同源，不抄数据）补回。
pub mod m006_taxonomy_seed;
mod m007_outbox_indexes;
mod m008_contact_commitments_reshape;
mod m009_prompt_template_versioned;
mod m010_contact_custom_instructions_and_knowledge_tags;
mod m011_drop_legacy_sales_collections;
mod m012_drop_legacy_taxonomy_seed;
/// `pub(crate)`：H13 把 `derive_state_policy_lists` 派生纯函数留在本迁移里作唯一真相，
/// `routes::admin_ops_versions::publish_state_machine_version` 联动重派生 policies 时
/// 跨模块复用同一份逻辑（杜绝 m013 与 publish 路径漂移）。
pub(crate) mod m013_seed_user_operation_state_policies;
mod m014_drop_trigger_keywords;
mod m015_ops_tables_active_versions;
/// `pub`: workspace isolation integration test inserts a legacy unscoped row after startup
/// and directly exercises this idempotent step without bypassing the production approval gate.
pub mod m016_backfill_workspace_id_on_legacy_rows;
mod m017_dedupe_outcome_aggregation;
/// `pub`:集成测试需直接调用 `m018::run_step` 对预置顶层残留验证回填语义(详见模块内注释)。
pub mod m018_backfill_domain_stage_from_legacy_top;
mod m019_state_machine_state_flags;
mod m020_seed_purchase_lifecycle;
mod m021_seed_churn_reason;
mod m022_backfill_dormant_allow_from_any;
mod m023_seed_value_tier;
mod m024_seed_relationship_type;
mod m025_backfill_ask_human_policy;
mod m026_seed_sales_with_relationships;
mod m027_contact_trust_fields;
mod m028_seed_conversation_mode;
/// `pub`：集成测试 `tests/m029_cleanup_contact_identity.rs` 需在 `TestApp::start()`
/// 跑完迁移后手动插入受污染 contacts + roster 快照，再**直接调用** `m029::run_step`
/// 验证清理语义（删非真人 normal / roster 回填 / 清 Demi / managed 保留 / 幂等）。
/// 跨 crate 调用要求 `pub`（同 m018 先例）。
pub mod m029_cleanup_contact_identity;
/// `pub`：集成测试 `tests/campaign_segment_coverage.rs` 需在 `TestApp::start()` 跑完
/// 迁移后手动插入缺 verification/eventKind 的老成交 contacts，再**直接调用**
/// `m030::run_step` 验回填语义（同 m018/m029 先例：为集成测暴露而用 `pub mod`）。
pub mod m030_backfill_outcome_event_defaults;
/// `pub`：集成测试需在 `TestApp::start()` 跑完迁移后手动插入缺 last_pushed_at_ms 的旧
/// escalation 行，再**直接调用** `m031::run_step` 验回填语义（同 m018/m029/m030 先例：
/// 为集成测暴露而用 `pub mod`）。
pub mod m031_backfill_escalation_last_pushed_at;
pub mod m032_backfill_taxonomy_workspace;
mod m033_task_commit_indexes;
mod m034_reconcile_review_fixes;
mod m035_reconcile_legacy_cleanup;
mod m036_reconcile_workspace_backfill;
mod m037_materialize_admin_acl;
mod m038_scope_outbox_idempotency;
/// `pub`：集成测试会在启动迁移完成后插入 legacy revision/signal，再直接重跑本步，
/// 验证精确回填、幂等和歧义 fail-closed。
pub mod m039_scope_revision_and_behavior_identity;
/// `pub`: integration tests rerun this corrective migration after seeding
/// legacy threshold rows to prove election, revision backfill, and idempotency.
pub mod m040_evolution_release_protocol;
/// `pub`: integration tests seed legacy duplicate anchors after normal startup,
/// then rerun the read-only audit to prove fail-closed/no-rewrite behavior.
pub mod m041_audit_send_ledger_anchors;
/// `pub`: integration tests seed legacy Soul rows after normal startup and
/// rerun this reconciliation to prove fail-closed and idempotent behavior.
pub mod m042_agent_soul_versions;
/// `pub`: integration tests seed legacy split pointer/status rows after normal
/// startup and rerun this reconciliation to prove validation-before-write.
pub mod m043_prompt_single_current;
/// `pub`: integration tests seed legacy DomainSchema rows after startup and
/// rerun this pre-index audit to prove ambiguous data fails without rewrites.
pub mod m044_domain_schema_single_active;
/// `pub`: integration tests seed legacy relationship review indexes/rows after startup and
/// rerun this migration to prove exact index retirement and fail-closed pending audits.
pub mod m045_relationship_review_cycles;
/// `pub`: integration tests can seed the retired principal-escalation index after startup and
/// rerun this audit to prove exact retirement and full account-scoped pending identity.
pub mod m046_scope_principal_escalation_pending;
/// `pub`: integration tests may rerun the idempotent owner backfill after seeding
/// legacy pending/new-protocol relay rows and active legacy relay tasks.
pub mod m047_backfill_principal_awaiting_owners;
/// `pub`: integration tests may seed zero/multiple current rows after startup
/// and rerun this deterministic reconciliation before rebuilding unique indexes.
pub mod m048_ops_single_current;
/// Corrective rerun for upgraded databases whose m043 marker predates the
/// planning-only prompt invariant. Integration tests exercise the real marker
/// transition and startup alignment after this step.
pub mod m049_reconcile_prompt_planning_currents;
/// `pub`: integration tests may seed legacy canonical/alias ambiguity and rerun
/// the validation-before-write identity-claims backfill.
pub mod m050_taxonomy_identity_claims;
/// Audit DomainProfile version/current/active identities before unique indexes.
pub mod m051_domain_profile_release_invariants;
/// Upgrade legacy catalog work to durable generations with recoverable leases.
/// Integration tests rerun this idempotent step after seeding legacy jobs.
pub mod m052_catalog_rebuild_leases;
/// Upgrade auto-ingest sources to configuration generations and leased claims.
pub mod m053_ingest_source_claims;
/// Audit account-scoped Playbook default ownership before the partial unique index.
pub mod m054_playbook_single_default;
/// Audit and backfill the one-to-one Lesson promotion identity before unique indexes.
pub mod m055_lesson_promotion_identity;
/// Initialize legacy asynchronous import jobs for generation-fenced claims.
pub mod m056_import_job_claims;
/// Materialize acknowledgement in every state-policy version before removing the runtime bypass.
pub mod m057_explicit_acknowledgement_action;
/// Audit legacy text-provider active pointers before creating the unique index.
pub mod m058_llm_provider_active_invariant;

/// Seed the built-in taxonomy template into one workspace without overwriting
/// any operator-owned row. This is used lazily when an existing/new workspace
/// is first accessed; migrations only seed DEFAULT_WORKSPACE_ID and therefore
/// cannot cover workspaces added after process startup.
pub(crate) async fn ensure_builtin_taxonomies_for_workspace(
    db: &Database,
    workspace_id: &str,
) -> AppResult<bool> {
    const MAX_ATTEMPTS: usize = 5;
    const MAX_COMMIT_TIME: Duration = Duration::from_secs(5);

    let marker_id = format!("workspace_taxonomy_template_v1:{workspace_id}");
    let markers = db.raw().collection::<mongodb::bson::Document>("migrations");
    if markers
        .find_one(doc! { "_id": &marker_id }, None)
        .await?
        .is_some()
    {
        return Ok(false);
    }

    let now = DateTime::now();
    let mut entries = m006_taxonomy_seed::default_taxonomy_seed_entries(now);
    entries.extend(m020_seed_purchase_lifecycle::purchase_lifecycle_seed_entries(now));
    entries.extend(m021_seed_churn_reason::churn_reason_seed_entries(now));
    entries.extend(m023_seed_value_tier::value_tier_seed_entries(now));
    entries.extend(m024_seed_relationship_type::relationship_type_seed_entries(
        now,
    ));
    entries.extend(m028_seed_conversation_mode::conversation_mode_seed_entries(
        now,
    ));

    'attempts: for attempt in 1..=MAX_ATTEMPTS {
        let mut session = db.client().start_session(None).await?;
        session
            .start_transaction(
                TransactionOptions::builder()
                    .max_commit_time(MAX_COMMIT_TIME)
                    .build(),
            )
            .await?;
        let result: AppResult<Option<bool>> = async {
            if markers
                .find_one_with_session(doc! { "_id": &marker_id }, None, &mut session)
                .await?
                .is_some()
            {
                return Ok(None);
            }

            let collection = db.collection_system_taxonomies();
            let mut inserted = false;
            for template in &entries {
                let mut entry = template.clone();
                entry.workspace_id = workspace_id.to_string();
                entry.seeded_by = Some("workspace_template".to_string());
                let filter = doc! {
                    "workspace_id": workspace_id,
                    "scope": &entry.scope,
                    "kind": &entry.kind,
                    "value.id": &entry.value.id,
                };
                let mut insert_doc = mongodb::bson::to_document(&entry)?;
                insert_doc.remove("_id");
                let result = collection
                    .update_one_with_session(
                        filter,
                        doc! { "$setOnInsert": insert_doc },
                        UpdateOptions::builder().upsert(true).build(),
                        &mut session,
                    )
                    .await?;
                inserted |= result.upserted_id.is_some();
            }
            markers
                .update_one_with_session(
                    doc! { "_id": &marker_id },
                    doc! { "$setOnInsert": { "applied_at": DateTime::now() } },
                    UpdateOptions::builder().upsert(true).build(),
                    &mut session,
                )
                .await?;
            crate::db::config_generation::bump_generation_with_session(
                db,
                crate::db::config_generation::TAXONOMY_NAMESPACE,
                workspace_id,
                &mut session,
            )
            .await?;
            Ok(Some(inserted))
        }
        .await;

        let inserted = match result {
            Ok(Some(inserted)) => inserted,
            Ok(None) => {
                let _ = session.abort_transaction().await;
                return Ok(false);
            }
            Err(error) => {
                let retryable = matches!(
                    &error,
                    AppError::Db(db_error)
                        if db_error.contains_label("TransientTransactionError")
                );
                let _ = session.abort_transaction().await;
                if retryable && attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(20 * attempt as u64)).await;
                    continue 'attempts;
                }
                return Err(error);
            }
        };

        for commit_attempt in 1..=MAX_ATTEMPTS {
            match session.commit_transaction().await {
                Ok(()) => return Ok(inserted),
                Err(error)
                    if error.contains_label("UnknownTransactionCommitResult")
                        && commit_attempt < MAX_ATTEMPTS => {}
                Err(error) if error.contains_label("UnknownTransactionCommitResult") => {
                    if markers
                        .find_one(doc! { "_id": &marker_id }, None)
                        .await?
                        .is_some()
                    {
                        return Ok(inserted);
                    }
                    return Err(error.into());
                }
                Err(error)
                    if error.contains_label("TransientTransactionError")
                        && attempt < MAX_ATTEMPTS =>
                {
                    let _ = session.abort_transaction().await;
                    tokio::time::sleep(Duration::from_millis(20 * attempt as u64)).await;
                    continue 'attempts;
                }
                Err(error) => {
                    let _ = session.abort_transaction().await;
                    return Err(error.into());
                }
            }
        }
    }
    unreachable!("bounded taxonomy seed transaction always returns")
}

type MigrationFuture<'a> = Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>>;
pub type MigrationFn = for<'a> fn(&'a Database) -> MigrationFuture<'a>;

/// 单条迁移定义：`id` 必须 chronologically sortable（建议 `YYYY_MM_NNN_*` 命名）。
pub struct Migration {
    pub id: &'static str,
    pub run: MigrationFn,
}

/// 全局迁移列表。新增迁移时：先在 `mNNN_*.rs` 实现 `run_step`，再追加到此列表。
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        id: "2026_05_001_split_last_message_at",
        run: |db| Box::pin(m001_split_last_message_at::run_step(db)),
    },
    Migration {
        id: "2026_05_002_split_active_facts",
        run: |db| Box::pin(m002_split_active_facts::run_step(db)),
    },
    Migration {
        id: "2026_05_003_state_machine_allowed_from",
        run: |db| Box::pin(m003_state_machine_allowed_from::run_step(db)),
    },
    Migration {
        id: "2026_05_004_outcome_metrics_workspace_in_id",
        run: |db| Box::pin(m004_outcome_metrics_id::run_step(db)),
    },
    Migration {
        id: "2026_05_005_memory_facts_to_structured",
        run: |db| Box::pin(m005_memory_facts_to_structured::run_step(db)),
    },
    Migration {
        id: "2026_05_006_taxonomy_seed",
        run: |db| Box::pin(m006_taxonomy_seed::run_step(db)),
    },
    Migration {
        id: "2026_05_007_outbox_indexes",
        run: |db| Box::pin(m007_outbox_indexes::run_step(db)),
    },
    Migration {
        id: "2026_05_008_contact_commitments_reshape",
        run: |db| Box::pin(m008_contact_commitments_reshape::run_step(db)),
    },
    Migration {
        id: "2026_05_M4_001_prompt_template_versioned",
        run: |db| Box::pin(m009_prompt_template_versioned::run_step(db)),
    },
    Migration {
        id: "2026_05_V3_001_contact_custom_instructions_and_knowledge_tags",
        run: |db| Box::pin(m010_contact_custom_instructions_and_knowledge_tags::run_step(db)),
    },
    Migration {
        id: "2026_05_V3_002_drop_legacy_sales_collections",
        run: |db| Box::pin(m011_drop_legacy_sales_collections::run_step(db)),
    },
    Migration {
        id: "2026_05_V3_003_drop_legacy_taxonomy_seed",
        run: |db| Box::pin(m012_drop_legacy_taxonomy_seed::run_step(db)),
    },
    Migration {
        id: "2026_05_W4_001_seed_user_operation_state_policies",
        run: |db| Box::pin(m013_seed_user_operation_state_policies::run_step(db)),
    },
    Migration {
        id: "2026_05_W4_002_drop_trigger_keywords",
        run: |db| Box::pin(m014_drop_trigger_keywords::run_step(db)),
    },
    Migration {
        id: "2026_05_W4_003_ops_tables_active_versions",
        run: |db| Box::pin(m015_ops_tables_active_versions::run_step(db)),
    },
    Migration {
        id: "2026_05_X1_001_backfill_workspace_id_on_legacy_rows",
        run: |db| Box::pin(m016_backfill_workspace_id_on_legacy_rows::run_step(db)),
    },
    Migration {
        id: "2026_05_X1_002_dedupe_outcome_aggregation_tasks",
        run: |db| Box::pin(m017_dedupe_outcome_aggregation::run_step(db)),
    },
    Migration {
        id: "2026_06_X2_001_backfill_domain_stage_from_legacy_top",
        run: |db| Box::pin(m018_backfill_domain_stage_from_legacy_top::run_step(db)),
    },
    Migration {
        id: "2026_06_X3_001_state_machine_state_flags",
        run: |db| Box::pin(m019_state_machine_state_flags::run_step(db)),
    },
    Migration {
        id: "2026_06_X4_001_seed_purchase_lifecycle",
        run: |db| Box::pin(m020_seed_purchase_lifecycle::run_step(db)),
    },
    Migration {
        id: "2026_06_X5_001_seed_churn_reason",
        run: |db| Box::pin(m021_seed_churn_reason::run_step(db)),
    },
    Migration {
        id: "2026_06_X6_001_backfill_dormant_allow_from_any",
        run: |db| Box::pin(m022_backfill_dormant_allow_from_any::run_step(db)),
    },
    Migration {
        id: "2026_06_X7_001_seed_value_tier",
        run: |db| Box::pin(m023_seed_value_tier::run_step(db)),
    },
    Migration {
        id: "2026_06_X8_001_seed_relationship_type",
        run: |db| Box::pin(m024_seed_relationship_type::run_step(db)),
    },
    Migration {
        id: "2026_06_X9_001_backfill_ask_human_policy",
        run: |db| Box::pin(m025_backfill_ask_human_policy::run_step(db)),
    },
    Migration {
        id: "2026_06_Y0_001_seed_sales_with_relationships",
        run: |db| Box::pin(m026_seed_sales_with_relationships::run_step(db)),
    },
    Migration {
        id: "2026_06_Y1_001_contact_trust_fields",
        run: |db| Box::pin(m027_contact_trust_fields::run_step(db)),
    },
    Migration {
        id: "2026_06_Y2_001_seed_conversation_mode",
        run: |db| Box::pin(m028_seed_conversation_mode::run_step(db)),
    },
    Migration {
        id: "2026_07_029_cleanup_contact_identity",
        run: |db| Box::pin(m029_cleanup_contact_identity::run_step(db)),
    },
    Migration {
        id: "2026_07_030_backfill_outcome_event_defaults",
        run: |db| Box::pin(m030_backfill_outcome_event_defaults::run_step(db)),
    },
    Migration {
        id: "2026_07_031_backfill_escalation_last_pushed_at",
        run: |db| Box::pin(m031_backfill_escalation_last_pushed_at::run_step(db)),
    },
    Migration {
        id: "2026_07_032_backfill_taxonomy_workspace",
        run: |db| Box::pin(m032_backfill_taxonomy_workspace::run_step(db)),
    },
    Migration {
        id: "2026_07_033_task_commit_indexes",
        run: |db| Box::pin(m033_task_commit_indexes::run_step(db)),
    },
    Migration {
        id: "2026_07_034_reconcile_review_fixes",
        run: |db| Box::pin(m034_reconcile_review_fixes::run_step(db)),
    },
    Migration {
        id: "2026_07_035_reconcile_legacy_cleanup",
        run: |db| Box::pin(m035_reconcile_legacy_cleanup::run_step(db)),
    },
    Migration {
        id: "2026_07_036_reconcile_workspace_backfill",
        run: |db| Box::pin(m036_reconcile_workspace_backfill::run_step(db)),
    },
    Migration {
        id: "2026_07_037_materialize_admin_acl",
        run: |db| Box::pin(m037_materialize_admin_acl::run_step(db)),
    },
    Migration {
        id: "2026_07_038_scope_outbox_idempotency",
        run: |db| Box::pin(m038_scope_outbox_idempotency::run_step(db)),
    },
    Migration {
        id: "2026_07_039_scope_revision_and_behavior_identity",
        run: |db| Box::pin(m039_scope_revision_and_behavior_identity::run_step(db)),
    },
    Migration {
        id: "2026_07_040_evolution_release_protocol",
        run: |db| Box::pin(m040_evolution_release_protocol::run_step(db)),
    },
    Migration {
        id: "2026_07_041_audit_send_ledger_anchors",
        run: |db| Box::pin(m041_audit_send_ledger_anchors::run_step(db)),
    },
    Migration {
        id: "2026_07_042_agent_soul_versions",
        run: |db| Box::pin(m042_agent_soul_versions::run_step(db)),
    },
    Migration {
        id: "2026_07_043_prompt_single_current",
        run: |db| Box::pin(m043_prompt_single_current::run_step(db)),
    },
    Migration {
        id: "2026_07_044_domain_schema_single_active",
        run: |db| Box::pin(m044_domain_schema_single_active::run_step(db)),
    },
    Migration {
        id: "2026_07_045_relationship_review_cycles",
        run: |db| Box::pin(m045_relationship_review_cycles::run_step(db)),
    },
    Migration {
        id: "2026_07_046_scope_principal_escalation_pending",
        run: |db| Box::pin(m046_scope_principal_escalation_pending::run_step(db)),
    },
    Migration {
        id: "2026_07_047_backfill_principal_awaiting_owners",
        run: |db| Box::pin(m047_backfill_principal_awaiting_owners::run_step(db)),
    },
    Migration {
        id: "2026_07_048_ops_single_current",
        run: |db| Box::pin(m048_ops_single_current::run_step(db)),
    },
    Migration {
        id: "2026_07_049_reconcile_prompt_planning_currents",
        run: |db| Box::pin(m049_reconcile_prompt_planning_currents::run_step(db)),
    },
    Migration {
        id: "2026_07_050_taxonomy_identity_claims",
        run: |db| Box::pin(m050_taxonomy_identity_claims::run_step(db)),
    },
    Migration {
        id: "2026_07_051_domain_profile_release_invariants",
        run: |db| Box::pin(m051_domain_profile_release_invariants::run_step(db)),
    },
    Migration {
        id: "2026_07_052_catalog_rebuild_leases",
        run: |db| Box::pin(m052_catalog_rebuild_leases::run_step(db)),
    },
    Migration {
        id: "2026_07_053_ingest_source_claims",
        run: |db| Box::pin(m053_ingest_source_claims::run_step(db)),
    },
    Migration {
        id: "2026_07_054_playbook_single_default",
        run: |db| Box::pin(m054_playbook_single_default::run_step(db)),
    },
    Migration {
        id: "2026_07_055_lesson_promotion_identity",
        run: |db| Box::pin(m055_lesson_promotion_identity::run_step(db)),
    },
    Migration {
        id: "2026_07_056_import_job_claims",
        run: |db| Box::pin(m056_import_job_claims::run_step(db)),
    },
    Migration {
        id: "2026_08_057_explicit_acknowledgement_action",
        run: |db| Box::pin(m057_explicit_acknowledgement_action::run_step(db)),
    },
    Migration {
        id: "2026_08_058_llm_provider_active_invariant",
        run: |db| Box::pin(m058_llm_provider_active_invariant::run_step(db)),
    },
];

const LEGACY_CLEANUP_APPROVAL: &str = "2026_07_035_reconcile_legacy_cleanup";
const WORKSPACE_BACKFILL_APPROVAL: &str = "2026_07_036_reconcile_workspace_backfill";

/// Return the exact corrective migration id that must be present in
/// `APPROVED_MIGRATIONS` before a production process may execute this step.
fn production_approval_gate(migration_id: &str) -> Option<&'static str> {
    match migration_id {
        "2026_05_V3_002_drop_legacy_sales_collections"
        | "2026_05_V3_003_drop_legacy_taxonomy_seed"
        | "2026_05_W4_002_drop_trigger_keywords"
        | LEGACY_CLEANUP_APPROVAL => Some(LEGACY_CLEANUP_APPROVAL),
        "2026_05_X1_001_backfill_workspace_id_on_legacy_rows" | WORKSPACE_BACKFILL_APPROVAL => {
            Some(WORKSPACE_BACKFILL_APPROVAL)
        }
        _ => None,
    }
}

fn approved_migrations_from_env() -> HashSet<String> {
    std::env::var("APPROVED_MIGRATIONS")
        .unwrap_or_default()
        .split([',', ';', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Returns whether destructive migrations must require explicit approval.
/// Only an explicit local/development/test environment opts out. Missing,
/// blank, staging, and unknown values all fail closed as production-like.
fn destructive_migrations_require_approval(app_env: Option<&str>) -> bool {
    !matches!(
        app_env
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("development" | "dev" | "test" | "local")
    )
}

/// 入口函数：扫描 `migrations` 集合，按顺序执行未应用的迁移。
pub async fn run(db: &Database) -> AppResult<()> {
    let app_env = std::env::var("APP_ENV").ok();
    let protected = destructive_migrations_require_approval(app_env.as_deref());
    let approvals = approved_migrations_from_env();
    run_with_policy(db, MIGRATIONS, protected, &approvals).await
}

/// 测试友好的内部入口：允许传入自定义迁移列表，用于单元测试和快照重放。
pub async fn run_with(db: &Database, migrations: &[Migration]) -> AppResult<()> {
    run_with_policy(db, migrations, false, &HashSet::new()).await
}

/// Execute migrations with an explicit environment policy. Exposed so
/// integration tests can prove the production blocked/approved transition
/// without mutating process-global environment variables.
pub async fn run_with_policy(
    db: &Database,
    migrations: &[Migration],
    production: bool,
    approvals: &HashSet<String>,
) -> AppResult<()> {
    let collection = db.migrations();
    for migration in migrations {
        let existing = collection
            .find_one(doc! { "_id": migration.id }, None)
            .await?;
        if existing
            .as_ref()
            .is_some_and(|record| record.status.as_deref() != Some("blocked"))
        {
            tracing::debug!(
                migration_id = migration.id,
                "migration already applied, skipping"
            );
            continue;
        }

        if production {
            if let Some(gate_id) = production_approval_gate(migration.id) {
                if !approvals.contains(gate_id) {
                    let reason = format!(
                        "production approval required: add {gate_id} to APPROVED_MIGRATIONS after backup and ownership verification"
                    );
                    collection
                        .update_one(
                            doc! { "_id": migration.id },
                            doc! {
                                "$set": {
                                    "status": "blocked",
                                    "reason": &reason,
                                    "blocked_at": DateTime::now(),
                                },
                                "$unset": { "applied_at": "" },
                            },
                            UpdateOptions::builder().upsert(true).build(),
                        )
                        .await?;
                    tracing::warn!(
                        migration_id = migration.id,
                        approval_gate = gate_id,
                        "migration blocked pending explicit production approval"
                    );
                    continue;
                }
            }
        }

        tracing::info!(migration_id = migration.id, "applying migration");
        (migration.run)(db).await?;
        collection
            .update_one(
                doc! { "_id": migration.id },
                doc! {
                    "$set": {
                        "status": "applied",
                        "applied_at": DateTime::now(),
                    },
                    "$unset": { "reason": "", "blocked_at": "" },
                },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;
        tracing::info!(migration_id = migration.id, "migration applied");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_migrations_fail_closed_when_environment_is_missing_or_unknown() {
        for value in [
            None,
            Some(""),
            Some("production"),
            Some("staging"),
            Some("qa"),
        ] {
            assert!(destructive_migrations_require_approval(value), "{value:?}");
        }
    }

    #[test]
    fn destructive_migrations_only_open_for_explicit_local_environments() {
        for value in ["development", "DEV", "test", " local "] {
            assert!(
                !destructive_migrations_require_approval(Some(value)),
                "{value}"
            );
        }
    }

    #[test]
    fn migration_ids_are_unique() {
        let mut ids: Vec<&str> = MIGRATIONS.iter().map(|m| m.id).collect();
        let original_len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            original_len,
            "migration ids must be unique; duplicates: {:?}",
            ids
        );
    }

    #[test]
    fn migration_ids_are_chronologically_ordered() {
        for window in MIGRATIONS.windows(2) {
            assert!(
                window[0].id < window[1].id,
                "migrations must be in id order: {} should come before {}",
                window[0].id,
                window[1].id
            );
        }
    }
}

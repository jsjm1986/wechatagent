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

use std::future::Future;
use std::pin::Pin;

use mongodb::bson::{doc, DateTime};

use crate::error::AppResult;
use crate::models::MigrationRecord;

use super::Database;

mod helpers;

mod m001_split_last_message_at;
mod m002_split_active_facts;
mod m003_state_machine_allowed_from;
mod m004_outcome_metrics_id;
mod m005_memory_facts_to_structured;
mod m006_taxonomy_seed;
mod m007_outbox_indexes;
mod m008_contact_commitments_reshape;
mod m009_prompt_template_versioned;
mod m010_contact_custom_instructions_and_knowledge_tags;
mod m011_drop_legacy_sales_collections;
mod m012_drop_legacy_taxonomy_seed;
mod m013_seed_user_operation_state_policies;
mod m014_drop_trigger_keywords;
mod m015_ops_tables_active_versions;
mod m016_backfill_workspace_id_on_legacy_rows;
mod m017_dedupe_outcome_aggregation;
/// `pub`:集成测试需直接调用 `m018::run_step` 对预置顶层残留验证回填语义(详见模块内注释)。
pub mod m018_backfill_domain_stage_from_legacy_top;

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
];

/// 入口函数：扫描 `migrations` 集合，按顺序执行未应用的迁移。
pub async fn run(db: &Database) -> AppResult<()> {
    run_with(db, MIGRATIONS).await
}

/// 测试友好的内部入口：允许传入自定义迁移列表，用于单元测试和快照重放。
pub async fn run_with(db: &Database, migrations: &[Migration]) -> AppResult<()> {
    let collection = db.migrations();
    for migration in migrations {
        let existing = collection
            .find_one(doc! { "_id": migration.id }, None)
            .await?;
        if existing.is_some() {
            tracing::debug!(
                migration_id = migration.id,
                "migration already applied, skipping"
            );
            continue;
        }
        tracing::info!(migration_id = migration.id, "applying migration");
        (migration.run)(db).await?;
        let record = MigrationRecord {
            id: migration.id.to_string(),
            applied_at: DateTime::now(),
        };
        collection.insert_one(record, None).await?;
        tracing::info!(migration_id = migration.id, "migration applied");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

//! 2026_05_V3_003（knowledge-base cleanup）：清空 `system_taxonomies` 中的销售域三 kind seed
//! （`customer_stage` / `intent_level` / `objection_type`），让用户在 admin 通过
//! DomainSchema + 自定义 taxonomy 自配。
//!
//! 集合本身保留，仅删销售域 seed。其它 kind（如 evolution-related）不受影响。
//!
//! 生产环境由迁移 runner 的 `APPROVED_MIGRATIONS` 精确审批闸保护；未批准时
//! 记录为 `blocked` 并在后续启动重试。
//!
//! 幂等：filter 命中即删，二次执行 matched=0。

use mongodb::bson::doc;

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let result = db
        .collection_system_taxonomies()
        .delete_many(
            doc! { "kind": { "$in": ["customer_stage", "intent_level", "objection_type"] } },
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_05_V3_003_drop_legacy_taxonomy_seed",
        deleted = result.deleted_count,
        "cleared legacy sales-domain taxonomy seeds"
    );
    Ok(())
}

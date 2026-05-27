//! 2026_05_V3_003（knowledge-base cleanup）：清空 `system_taxonomies` 中的销售域三 kind seed
//! （`customer_stage` / `intent_level` / `objection_type`），让用户在 admin 通过
//! DomainSchema + 自定义 taxonomy 自配。
//!
//! 集合本身保留，仅删销售域 seed。其它 kind（如 evolution-related）不受影响。
//!
//! 生产环境守卫：`APP_ENV=production` 时直接报错阻断，避免误删。
//!
//! 幂等：filter 命中即删，二次执行 matched=0。

use mongodb::bson::doc;

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    if std::env::var("APP_ENV").unwrap_or_default() == "production" {
        return Err(crate::error::AppError::External(
            "禁止在 production 环境执行 cleanup migration: drop_legacy_taxonomy_seed".into(),
        ));
    }
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

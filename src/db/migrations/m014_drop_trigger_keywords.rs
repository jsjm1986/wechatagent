//! 2026_05_W4_002（agent-first 渐进式披露）：从 operation_knowledge_chunks /
//! _items / _documents 三集合 `$unset` `trigger_keywords` 字段。
//! Agent-first 渐进式披露形态接管之后，关键词快路径与 `trigger_keywords` 索引
//! 一并下线。
//!
//! 生产环境守卫：`APP_ENV=production` 时 noop 返回，避免误删；
//! 运维需在确认所有副本与备份不再依赖该字段后再手工 unset。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    if std::env::var("APP_ENV").unwrap_or_default() == "production" {
        tracing::warn!(
            migration_id = "2026_05_W4_002_drop_trigger_keywords",
            "production guard: skipped trigger_keywords unset; run manually after backup verification"
        );
        return Ok(());
    }
    let collections: [&str; 3] = [
        "operation_knowledge_documents",
        "operation_knowledge_items",
        "operation_knowledge_chunks",
    ];
    for coll_name in collections {
        let coll = db.raw().collection::<Document>(coll_name);
        let result = coll
            .update_many(
                doc! { "trigger_keywords": { "$exists": true } },
                doc! { "$unset": { "trigger_keywords": "" } },
                None,
            )
            .await?;
        tracing::info!(
            migration_id = "2026_05_W4_002_drop_trigger_keywords",
            collection = coll_name,
            modified = result.modified_count,
            "unset trigger_keywords"
        );
    }
    Ok(())
}

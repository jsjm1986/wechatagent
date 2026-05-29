//! 2026_05_V3_002（knowledge-base cleanup）：清空旧三层集合的存量数据。
//!
//! 开发期数据无价值，无需兼容；本迁移仅清空文档，集合本身保留。
//! 集合 `operation_knowledge_items` 在 commit 2 移除 typed accessor；这里同步 delete_many
//! 让所有环境的存量销售域文档归零。
//!
//! 生产环境守卫：`APP_ENV=production` 时 noop 返回（不删数据），避免误删；
//! 运维需在确认无误后手工执行清理。与 m014 同款 warn+Ok 形态——返回 Err 会在
//! `mod.rs::run_with` 的 `(migration.run)(db).await?` 处于 `insert_one` 记录前
//! 中断，迁移永不入账，每次启动重试重错，生产端无干净恢复路径（boot-brick）。
//!
//! 幂等：所有 delete_many 都是按 `{}` 全量删；二次执行 matched=0 即可。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    if std::env::var("APP_ENV").unwrap_or_default() == "production" {
        tracing::warn!(
            migration_id = "2026_05_V3_002_drop_legacy_sales_collections",
            "production guard: skipped legacy sales-collection cleanup; run manually after backup verification"
        );
        return Ok(());
    }
    let raw = db.raw();
    for name in [
        "operation_knowledge_items",
        "operation_knowledge_documents",
        "operation_knowledge_chunks",
    ] {
        let coll = raw.collection::<Document>(name);
        let result = coll.delete_many(doc! {}, None).await?;
        tracing::info!(
            migration_id = "2026_05_V3_002_drop_legacy_sales_collections",
            collection = name,
            deleted = result.deleted_count,
            "cleared legacy knowledge collection"
        );
    }
    Ok(())
}

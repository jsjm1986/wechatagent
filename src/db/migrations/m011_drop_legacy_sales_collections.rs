//! 2026_05_V3_002（knowledge-base cleanup）：清空旧三层集合的存量数据。
//!
//! 开发期数据无价值，无需兼容；本迁移仅清空文档，集合本身保留。
//! 集合 `operation_knowledge_items` 在 commit 2 移除 typed accessor；这里同步 delete_many
//! 让所有环境的存量销售域文档归零。
//!
//! 生产环境由迁移 runner 的 `APPROVED_MIGRATIONS` 精确审批闸保护；未批准时
//! 记录为 `blocked` 并在后续启动重试，不会把未执行动作伪记为完成。
//!
//! 幂等：所有 delete_many 都是按 `{}` 全量删；二次执行 matched=0 即可。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
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

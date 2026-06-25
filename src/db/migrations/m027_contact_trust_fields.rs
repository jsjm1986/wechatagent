//! 2026_06_Y1_001：标签可信度改造 · 回填存量 contacts 的新字段默认值。
//!
//! Task 3 在 Contact 加了 7 个新字段（manual_tags / confirmed_tags / bayesian_signals
//! / personality_profile / tags_version / manual_tags_updated_at / manual_tags_by），
//! 全部带 `#[serde(default)]` 向后兼容反序列化。但存量文档物理上缺字段，某些操作
//! （如 `$push` 到不存在的数组）可能报错。本迁移物理写入空默认值，确保查询/索引一致性。
//!
//! 幂等：仅 `$set` 到 `manual_tags: { $exists: false }` 的文档；二次执行 matched=0。
//! personality_profile 是 `Option`（缺字段 → None），无需回填。

use mongodb::bson::doc;

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let result = db
        .contacts()
        .update_many(
            doc! { "manual_tags": { "$exists": false } },
            doc! {
                "$set": {
                    "manual_tags": [],
                    "confirmed_tags": [],
                    "bayesian_signals": [],
                    "tags_version": 0_i64,
                }
            },
            None,
        )
        .await?;

    tracing::info!(
        migration_id = "2026_06_Y1_001_contact_trust_fields",
        modified = result.modified_count,
        "backfilled trust fields on contacts"
    );

    Ok(())
}

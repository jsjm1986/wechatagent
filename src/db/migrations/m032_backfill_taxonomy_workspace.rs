//! 2026_07_032：为 taxonomy 两张表补齐 workspace 隔离键。
//!
//! 旧模型只用 `scope = global | account_id`，而 `account_id` 只在 workspace 内唯一，
//! 多租户下会让同名账号共享字典和候选。迁移把所有旧行明确归入
//! `DEFAULT_WORKSPACE_ID`；后续所有读写与唯一索引都包含 `workspace_id`。
//!
//! 这是语义保持型、幂等回填：仅命中字段缺失行，生产环境也必须执行。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) fn missing_workspace_filter() -> Document {
    doc! { "workspace_id": { "$exists": false } }
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    let workspace_id =
        std::env::var("DEFAULT_WORKSPACE_ID").unwrap_or_else(|_| "default".to_string());

    for collection_name in ["system_taxonomies", "taxonomy_candidates"] {
        let result = db
            .raw()
            .collection::<Document>(collection_name)
            .update_many(
                missing_workspace_filter(),
                doc! { "$set": { "workspace_id": &workspace_id } },
                None,
            )
            .await?;
        tracing::info!(
            migration_id = "2026_07_032_backfill_taxonomy_workspace",
            collection = collection_name,
            matched = result.matched_count,
            modified = result.modified_count,
            workspace_id,
            "backfilled taxonomy workspace isolation key"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_only_matches_legacy_rows() {
        assert_eq!(
            missing_workspace_filter(),
            doc! { "workspace_id": { "$exists": false } }
        );
    }
}

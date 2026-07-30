//! 2026_05_M4_001（agent-self-evolution / W0 Task 1.5）：把 `prompt_templates`
//! 升级为多版本形态。给所有缺字段的旧文档填：
//!   - `current_version`：同 `(workspace_id, prompt_key)` 下 `version` 最大且
//!     `status="active"` 的一条置 `true`，其余置 `false`；同 prompt_key 下
//!     若没有 active，按 version 最大兜底（`status="draft"` 也算）。
//!   - `previous_version: null`（rollback 不能跨迁移点）。
//!   - `seeded_by: "legacy_migration"`（来源标记）。
//!
//! 幂等：filter 命中"`current_version` 字段不存在"的文档；二次启动时所有文档
//! 都已具备字段，filter 不再命中。`migrations` 集合的版本记录是双保险。

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let coll = db.raw().collection::<Document>("prompt_templates");
    let stage1 = coll
        .update_many(
            doc! { "current_version": { "$exists": false } },
            doc! {
                "$set": {
                    "current_version": false,
                    "previous_version": Bson::Null,
                    "seeded_by": "legacy_migration",
                }
            },
            None,
        )
        .await?;

    let mut cursor = coll
        .aggregate(
            vec![
                doc! {
                    "$match": { "status": { "$ne": "archived" } }
                },
                doc! {
                    "$addFields": {
                        "_migration_active_priority": {
                            "$cond": [{ "$eq": ["$status", "active"] }, 1, 0]
                        }
                    }
                },
                doc! {
                    "$sort": {
                        "workspace_id": 1,
                        "prompt_key": 1,
                        "_migration_active_priority": -1,
                        "version": -1,
                    }
                },
                doc! {
                    "$group": {
                        "_id": { "workspace_id": "$workspace_id", "prompt_key": "$prompt_key" },
                        "target_id": { "$first": "$_id" },
                    }
                },
            ],
            None,
        )
        .await?;
    let mut promoted: u64 = 0;
    while let Some(group) = cursor.try_next().await? {
        let scope = group.get_document("_id").ok();
        let workspace_id = scope.and_then(|value| value.get_str("workspace_id").ok());
        let prompt_key = scope.and_then(|value| value.get_str("prompt_key").ok());
        let target_id = group.get("target_id").and_then(|b| b.as_object_id());
        let (Some(workspace_id), Some(prompt_key), Some(target)) =
            (workspace_id, prompt_key, target_id)
        else {
            continue;
        };
        // Promote first so interruption cannot leave the scope with zero
        // current rows. The following demotion makes retries converge to one.
        let result = coll
            .update_one(
                doc! { "_id": target },
                doc! { "$set": { "current_version": true } },
                None,
            )
            .await?;
        promoted += result.modified_count;
        coll.update_many(
            doc! {
                "workspace_id": workspace_id,
                "prompt_key": prompt_key,
                "_id": { "$ne": target },
            },
            doc! { "$set": { "current_version": false } },
            None,
        )
        .await?;
    }

    tracing::info!(
        migration_id = "2026_05_M4_001_prompt_template_versioned",
        backfilled = stage1.modified_count,
        promoted_current = promoted,
        "upgraded prompt_templates to multi-version layout"
    );
    Ok(())
}

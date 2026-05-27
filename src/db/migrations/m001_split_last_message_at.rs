//! 2026_05_001：把存量 contact 的 `last_message_at` 回填到 `last_inbound_at`，
//! 仅在 `last_inbound_at` 缺失（不存在或为 null）且 `last_message_at` 存在时回填。
//!
//! 用 aggregation pipeline 形式的 `update_many`，单次原子操作即完成；
//! 二次执行时所有候选都已回填过，filter 不再命中，从而幂等。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let pipeline: Vec<Document> = vec![doc! {
        "$set": {
            "last_inbound_at": "$last_message_at"
        }
    }];
    let result = db
        .contacts()
        .update_many(
            doc! {
                "$and": [
                    { "last_message_at": { "$exists": true, "$ne": null } },
                    {
                        "$or": [
                            { "last_inbound_at": { "$exists": false } },
                            { "last_inbound_at": null }
                        ]
                    }
                ]
            },
            pipeline,
            None,
        )
        .await?;
    tracing::info!(
        modified = result.modified_count,
        matched = result.matched_count,
        "backfilled last_inbound_at from last_message_at"
    );
    Ok(())
}

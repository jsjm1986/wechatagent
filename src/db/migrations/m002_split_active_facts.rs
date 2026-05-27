//! 2026_05_002：把 `operating_memories.memory_card.activeFacts` 拆分为
//! `coreFacts`（前 6 条按重要度，由旧顺序近似）和 `recentFacts`（剩余项）。
//!
//! 拆完后 `$unset memory_card.activeFacts`，再次执行时 filter 不再命中，
//! 从而幂等。`migrations` 集合的版本记录是双保险。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let pipeline: Vec<Document> = vec![
        doc! {
            "$set": {
                "memory_card.coreFacts": {
                    "$slice": [
                        { "$ifNull": ["$memory_card.activeFacts", []] },
                        6
                    ]
                },
                "memory_card.recentFacts": {
                    "$slice": [
                        { "$ifNull": ["$memory_card.activeFacts", []] },
                        6,
                        10000_i64
                    ]
                }
            }
        },
        doc! {
            "$unset": "memory_card.activeFacts"
        },
    ];
    let result = db
        .operating_memories()
        .update_many(
            doc! { "memory_card.activeFacts": { "$exists": true } },
            pipeline,
            None,
        )
        .await?;
    tracing::info!(
        modified = result.modified_count,
        matched = result.matched_count,
        "split memory_card.activeFacts into coreFacts/recentFacts"
    );
    Ok(())
}

//! 2026_05_008（M2 Strategic Planner）：把 `contacts.last_commitment: Option<String>`
//! 升级为结构化数组 `commitments: [{ id, text, due_at:null, created_at }]`，并 `$unset`
//! 旧字段。
//!
//! 历史 `last_commitment` 是自由文本，没有 due_at；本迁移仅做形态升级，把字符串
//! 包成单元素 Vec，`due_at` 留 null（Planner `scan_commitments` 对 `Plain`/无 due_at
//! 的元素跳过，等下次 Reply Agent 重塑 memoryCard 时由 LLM 给出 due_at）。
//!
//! 幂等：filter 要求 `commitments` 不存在，二次执行时该条件不再命中。

use mongodb::bson::{doc, DateTime, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    let pipeline: Vec<Document> = vec![
        doc! {
            "$set": {
                "commitments": {
                    "$cond": [
                        {
                            "$and": [
                                { "$ne": [{ "$type": "$last_commitment" }, "missing"] },
                                { "$ne": ["$last_commitment", null] },
                                { "$ne": ["$last_commitment", ""] }
                            ]
                        },
                        [{
                            "id": { "$toString": "$_id" },
                            "text": "$last_commitment",
                            "createdAt": { "$ifNull": ["$updated_at", now] }
                        }],
                        []
                    ]
                }
            }
        },
        doc! { "$unset": "last_commitment" },
    ];
    let result = db
        .contacts()
        .update_many(
            doc! { "commitments": { "$exists": false } },
            pipeline,
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_05_008_contact_commitments_reshape",
        modified = result.modified_count,
        matched = result.matched_count,
        "reshaped contacts.last_commitment to commitments array"
    );
    Ok(())
}

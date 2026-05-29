//! 2026_05_X1_002（P1-1 配套）：在新增 `uniq_outcome_aggregation_kind_account_content`
//! partial unique index 之前清理 `agent_tasks` 中现存的 `outcome_aggregation`
//! 重复项。
//!
//! 背景：`tasks.rs::ensure_today_outcome_aggregation_tasks` 历史路径是
//! `find_one + insert_one` 的 TOCTOU；多副本 / 重叠 tick 都会通过检查并双写，
//! 导致 `(kind, account_id, content)` 出现多条记录。新索引带 partial filter
//! `kind="outcome_aggregation"` 限定范围，但首启时若已有重复行，索引创建会失败。
//!
//! 策略：保留每组 `(account_id, content)` 中 `created_at` 最小（最早）的一条，
//! 删除其余。最早行通常已经被 worker 处理过（status=completed），删后续重复
//! 的 pending/retry 副本不会丢失业务语义；最差情况是丢弃一条尚未跑过的副本，
//! 当日 tick 再次跑时会幂等重建。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    use futures::TryStreamExt;
    let pipeline = vec![
        doc! { "$match": { "kind": "outcome_aggregation" } },
        doc! { "$sort": { "created_at": 1 } },
        doc! {
            "$group": {
                "_id": { "account_id": "$account_id", "content": "$content" },
                "ids": { "$push": "$_id" },
                "count": { "$sum": 1 }
            }
        },
        doc! { "$match": { "count": { "$gt": 1 } } },
    ];
    let mut cursor = db.tasks().aggregate(pipeline, None).await?;
    let mut total_removed: u64 = 0;
    while let Some(doc) = cursor.try_next().await? {
        let ids = match doc.get_array("ids") {
            Ok(arr) => arr.clone(),
            Err(_) => continue,
        };
        // 第一条（_sort: created_at:1）保留，其余删除。
        if ids.len() <= 1 {
            continue;
        }
        let to_delete: Vec<_> = ids.iter().skip(1).cloned().collect();
        let filter = doc! { "_id": { "$in": to_delete } };
        let result = db.tasks().delete_many(filter, None).await?;
        total_removed += result.deleted_count;
    }
    tracing::info!(
        migration_id = "2026_05_X1_002_dedupe_outcome_aggregation_tasks",
        removed = total_removed,
        "outcome_aggregation duplicates removed before unique index creation"
    );
    let _: Document = doc! {};
    Ok(())
}

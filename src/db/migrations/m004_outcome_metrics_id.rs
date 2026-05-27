//! 2026_05_004：把 `agent_outcome_metrics._id` 从老 3 段
//! `{account}:{horizon}:{date}` 升级到 4 段 `{workspace}:{account}:{horizon}:{date}`。
//!
//! 不能就地 `update_one` 改 `_id`（MongoDB 禁止），所以用 "insert+delete" 方式：
//! 1. 扫描所有 _id 不含 4 段的文档；
//! 2. 用文档自带的 workspace_id / account_id 字段拼新 _id 写入新文档；
//! 3. 删除老文档。
//!
//! 幂等性：filter 命中"_id 字符串中不含 3 个冒号"才动；二次执行时所有文档都已是
//! 4 段，filter 不再命中。`migrations` 集合的版本记录是双保险。

use futures::TryStreamExt;
use mongodb::bson::doc;
use mongodb::options::UpdateOptions;

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let coll = db.outcome_metrics();
    let mut cursor = coll
        .find(
            doc! {
                "_id": { "$type": "string" },
                "workspace_id": { "$exists": true, "$type": "string" },
                "$expr": {
                    "$ne": [
                        { "$size": { "$split": ["$_id", ":"] } },
                        4_i32
                    ]
                }
            },
            None,
        )
        .await?;
    let mut migrated = 0_u64;
    while let Some(metric) = cursor.try_next().await? {
        let new_id = format!(
            "{}:{}:{}:{}",
            metric.workspace_id, metric.account_id, metric.horizon, metric.date
        );
        if new_id == metric.id {
            continue;
        }
        let mut new_metric = metric.clone();
        new_metric.id = new_id.clone();
        let new_doc = mongodb::bson::to_document(&new_metric)?;
        coll.update_one(
            doc! { "_id": &new_id },
            doc! { "$set": new_doc },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
        coll.delete_one(doc! { "_id": &metric.id }, None).await?;
        migrated += 1;
    }
    tracing::info!(
        migrated,
        "rewrote agent_outcome_metrics._id to include workspace_id"
    );
    Ok(())
}

//! 2026_05_005（W5 task 6.6 实质实现）：
//! 把 `operating_memories.memory_card.coreFacts / recentFacts` 中的字符串元素
//! 升级为结构化 `MemoryFact { id, text, confidence, importance, ... }`。
//!
//! 升级规则（与 `MemoryFactRepr::Plain → Structured` 反序列化路径一致，
//! 详见 `src/models.rs::MemoryFact`）：
//! * `id`：fresh UUIDv4（关键：避免老数据无 id 后续合并失真）；
//! * `text`：原始字符串；
//! * `confidence: 7` / `importance: 5`（默认中等）；
//! * `mayExpire: false`；
//! * `createdAt = updatedAt = now`；
//! * `deprecatedAt / deprecationReason / sourceMessageIds / sourceRunId`：缺省。
//!
//! 幂等：用每个元素是否含 `id` 字段判定，已结构化的元素跳过；本迁移把
//! `memory_card_version` 加 1 让上层缓存失效。

use futures::TryStreamExt;
use mongodb::bson::oid::ObjectId;
use mongodb::bson::{doc, Bson, DateTime, Document};

use crate::db::Database;
use crate::error::AppResult;

use super::helpers::upgrade_fact_array;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let collection = db.operating_memories();
    let mut cursor = collection
        .clone_with_type::<Document>()
        .find(
            doc! {
                "$or": [
                    { "memory_card.coreFacts": { "$type": "array" } },
                    { "memory_card.recentFacts": { "$type": "array" } }
                ]
            },
            None,
        )
        .await?;
    let mut upgraded_docs: i64 = 0;
    let mut upgraded_facts: i64 = 0;
    let now = DateTime::now();

    while let Some(raw) = cursor.try_next().await? {
        let Some(id) = raw.get_object_id("_id").ok() else {
            continue;
        };
        let card = match raw.get_document("memory_card") {
            Ok(c) => c.clone(),
            Err(_) => continue,
        };
        let (new_core, core_changed) =
            upgrade_fact_array(card.get_array("coreFacts").ok(), now, &mut upgraded_facts);
        let (new_recent, recent_changed) = upgrade_fact_array(
            card.get_array("recentFacts").ok(),
            now,
            &mut upgraded_facts,
        );
        if !core_changed && !recent_changed {
            continue;
        }
        let mut new_card = card.clone();
        if core_changed {
            new_card.insert("coreFacts", Bson::Array(new_core));
        }
        if recent_changed {
            new_card.insert("recentFacts", Bson::Array(new_recent));
        }
        collection
            .clone_with_type::<Document>()
            .update_one(
                doc! { "_id": ObjectId::from(id) },
                doc! {
                    "$set": {
                        "memory_card": new_card,
                        "updated_at": now,
                    },
                    "$inc": { "memory_card_version": 1 }
                },
                None,
            )
            .await?;
        upgraded_docs += 1;
    }

    tracing::info!(
        migration_id = "2026_05_005_memory_facts_to_structured",
        upgraded_docs,
        upgraded_facts,
        "structured fact upgrade applied"
    );
    Ok(())
}

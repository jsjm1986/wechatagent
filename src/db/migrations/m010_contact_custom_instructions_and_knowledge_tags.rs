//! 2026_05_V3_001：联系人 custom_agent_instructions + 知识库三集合标签字段回填。
//!
//! 给 `contacts` 加 `custom_agent_instructions: null`（上限 1000 字符的运营人员
//! 特别指令；后台 PUT 维护，作为 Operator Instruction 层注入 user.reply prompt）。
//!
//! 给 `operation_knowledge_documents` / `_items` / `_chunks` 三集合加：
//!   - `product_tags: []`（≤5）
//!   - `business_topics: []`（≤3）
//!
//! 幂等：每个 filter 用 `$exists: false` 仅命中未升级文档；二次启动不变更。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let contacts_result = db
        .contacts()
        .update_many(
            doc! { "custom_agent_instructions": { "$exists": false } },
            doc! { "$set": { "custom_agent_instructions": null } },
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_05_V3_001_contact_custom_instructions_and_knowledge_tags",
        contacts_modified = contacts_result.modified_count,
        contacts_matched = contacts_result.matched_count,
        "backfilled contacts.custom_agent_instructions"
    );

    let raw = db.raw();
    let docs_coll = raw.collection::<Document>("operation_knowledge_documents");
    let items_coll = raw.collection::<Document>("operation_knowledge_items");
    let chunks_coll = raw.collection::<Document>("operation_knowledge_chunks");
    let collections: [(&str, &mongodb::Collection<Document>); 3] = [
        ("operation_knowledge_documents", &docs_coll),
        ("operation_knowledge_items", &items_coll),
        ("operation_knowledge_chunks", &chunks_coll),
    ];
    for (coll_name, coll) in collections {
        let result = coll
            .update_many(
                doc! {
                    "$or": [
                        { "product_tags": { "$exists": false } },
                        { "business_topics": { "$exists": false } },
                    ]
                },
                doc! {
                    "$set": {
                        "product_tags": [],
                        "business_topics": [],
                    }
                },
                None,
            )
            .await?;
        tracing::info!(
            migration_id = "2026_05_V3_001_contact_custom_instructions_and_knowledge_tags",
            collection = coll_name,
            modified = result.modified_count,
            matched = result.matched_count,
            "backfilled knowledge tag fields"
        );
    }
    Ok(())
}

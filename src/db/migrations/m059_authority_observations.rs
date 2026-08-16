//! Introduce explicit authority metadata without treating legacy model output as verified fact.
//!
//! Existing text assets were written by authenticated administrators, so they remain enabled
//! while receiving explicit insertion governance. Existing memory facts cannot prove their
//! original source and are therefore marked `legacy_unverified`.

use mongodb::bson::{doc, Bson, DateTime, Document};

use crate::db::Database;
use crate::error::AppResult;

fn fact_array_pipeline(path: &str, now: DateTime) -> Vec<Document> {
    let source_path = format!("${path}");
    let upgraded = doc! {
        "$map": {
            "input": { "$ifNull": [source_path, Bson::Array(Vec::new())] },
            "as": "fact",
            "in": {
                "$cond": [
                    { "$eq": [{ "$type": "$$fact" }, "object"] },
                    {
                        "$mergeObjects": [
                            "$$fact",
                            {
                                "authority": { "$ifNull": ["$$fact.authority", "legacy_unverified"] },
                                "sourceType": { "$ifNull": ["$$fact.sourceType", "legacy"] },
                                "validFrom": {
                                    "$ifNull": ["$$fact.validFrom", { "$ifNull": ["$$fact.createdAt", now] }]
                                },
                                "status": { "$ifNull": ["$$fact.status", "legacy_unverified"] }
                            }
                        ]
                    },
                    "$$fact"
                ]
            }
        }
    };
    let mut set = Document::new();
    set.insert(path, upgraded);
    vec![doc! { "$set": set }]
}

fn governed_text_filter(status: Document, only_missing_enabled: bool) -> Document {
    let mut filter = doc! {
        "kind": { "$nin": ["media", "forbidden_expression"] },
        "body": { "$type": "string" },
        "$and": [status],
    };
    if only_missing_enabled {
        filter.insert("enabled", doc! { "$exists": false });
    }
    filter
}

fn governed_text_update(now: DateTime, enabled: bool, approve_missing: bool) -> Vec<Document> {
    let mut fields = doc! {
        "enabled": enabled,
        "allowed_insertion_levels": {
            "$ifNull": ["$allowed_insertion_levels", ["subtle", "contextual", "direct"]]
        },
        "updated_at": now,
    };
    if approve_missing {
        fields.insert("review_status", "approved");
    }
    vec![doc! { "$set": fields }]
}

fn asset_governance_steps(now: DateTime) -> Vec<(Document, Vec<Document>)> {
    vec![
        (
            governed_text_filter(
                doc! {
                    "$or": [
                        { "review_status": { "$exists": false } },
                        { "review_status": null },
                        { "review_status": "" },
                    ]
                },
                true,
            ),
            governed_text_update(now, true, true),
        ),
        (
            governed_text_filter(doc! { "review_status": "approved" }, true),
            governed_text_update(now, true, false),
        ),
        (
            governed_text_filter(
                doc! {
                    "review_status": {
                        "$exists": true,
                        "$nin": [Bson::Null, Bson::String(String::new()), Bson::String("approved".to_string())],
                    }
                },
                false,
            ),
            governed_text_update(now, false, false),
        ),
        (
            doc! {
                "$or": [
                    { "kind": { "$in": ["media", "forbidden_expression"] } },
                    { "body": { "$not": { "$type": "string" } } },
                ],
            },
            vec![doc! { "$set": { "enabled": false, "updated_at": now } }],
        ),
    ]
}

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    let assets = db.content_assets().clone_with_type::<Document>();
    let mut governed_assets = 0_u64;
    for (filter, update) in asset_governance_steps(now) {
        governed_assets += assets
            .update_many(filter, update, None)
            .await?
            .modified_count;
    }

    let memories = db.operating_memories().clone_with_type::<Document>();
    let mut upgraded_facts = 0_u64;
    for path in [
        "memory_card.coreFacts",
        "memory_card.recentFacts",
        "memory_card.deprecatedFacts",
    ] {
        let mut filter = Document::new();
        filter.insert(path, doc! { "$type": "array" });
        let result = memories
            .update_many(filter, fact_array_pipeline(path, now), None)
            .await?;
        upgraded_facts += result.modified_count;
    }

    tracing::info!(
        migration_id = "2026_08_059_authority_observations",
        governed_assets,
        upgraded_memory_documents = upgraded_facts,
        "materialized authority metadata"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_pipeline_marks_only_object_facts_unverified() {
        let pipeline = fact_array_pipeline("memory_card.coreFacts", DateTime::from_millis(7));
        let rendered = format!("{pipeline:?}");
        assert!(rendered.contains("legacy_unverified"));
        assert!(rendered.contains("sourceType"));
        assert!(rendered.contains("$$fact"));
    }

    #[test]
    fn asset_governance_preserves_review_lifecycle_and_excludes_non_text_assets() {
        let steps = asset_governance_steps(DateTime::from_millis(7));
        assert_eq!(steps.len(), 4);

        let missing_review_set = steps[0].1[0].get_document("$set").unwrap();
        assert_eq!(
            missing_review_set.get_str("review_status").unwrap(),
            "approved"
        );
        assert_eq!(missing_review_set.get_bool("enabled").unwrap(), true);

        let approved_set = steps[1].1[0].get_document("$set").unwrap();
        assert!(!approved_set.contains_key("review_status"));
        assert_eq!(approved_set.get_bool("enabled").unwrap(), true);

        let non_approved_filter = format!("{:?}", steps[2].0);
        let non_approved_set = steps[2].1[0].get_document("$set").unwrap();
        assert!(non_approved_filter.contains("$nin"));
        assert_eq!(non_approved_set.get_bool("enabled").unwrap(), false);

        let excluded_filter = format!("{:?}", steps[3].0);
        let excluded_set = steps[3].1[0].get_document("$set").unwrap();
        assert!(excluded_filter.contains("media"));
        assert!(excluded_filter.contains("forbidden_expression"));
        assert_eq!(excluded_set.get_bool("enabled").unwrap(), false);
    }
}

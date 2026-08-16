//! Add an explicit lifecycle to structured contact commitments.

use mongodb::bson::{doc, Bson, Document};

use crate::db::Database;
use crate::error::AppResult;

fn commitment_pipeline() -> Vec<Document> {
    vec![doc! {
        "$set": {
            "commitments": {
                "$map": {
                    "input": { "$ifNull": ["$commitments", Bson::Array(Vec::new())] },
                    "as": "entry",
                    "in": {
                        "$cond": [
                            { "$eq": [{ "$type": "$$entry" }, "object"] },
                            {
                                "$mergeObjects": [
                                    "$$entry",
                                    { "status": { "$ifNull": ["$$entry.status", "active"] } }
                                ]
                            },
                            "$$entry"
                        ]
                    }
                }
            }
        }
    }]
}

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let result = db
        .contacts()
        .clone_with_type::<Document>()
        .update_many(
            doc! { "commitments": { "$type": "array" } },
            commitment_pipeline(),
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_08_060_appointment_commitment_lifecycle",
        upgraded_contacts = result.modified_count,
        "materialized commitment lifecycle"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_pipeline_preserves_plain_entries() {
        let rendered = format!("{:?}", commitment_pipeline());
        assert!(rendered.contains("$type"));
        assert!(rendered.contains("active"));
        assert!(rendered.contains("$$entry"));
    }
}

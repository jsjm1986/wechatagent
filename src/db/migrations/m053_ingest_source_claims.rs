//! Upgrade auto-ingest sources to configuration generations and leased claims.
//!
//! Legacy rows predate both concepts. The migration validates any existing
//! protocol fields, initializes missing generations, and leaves valid active
//! claims untouched so a restart or concurrent migration run is idempotent.

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

fn optional_nonnegative_i64(row: &Document, field: &str) -> AppResult<Option<i64>> {
    let value = match row.get(field) {
        None | Some(Bson::Null) => return Ok(None),
        Some(Bson::Int32(value)) => i64::from(*value),
        Some(Bson::Int64(value)) => *value,
        Some(_) => {
            return Err(AppError::External(format!(
                "ingest source migration found non-integer {field}"
            )))
        }
    };
    if value < 0 {
        return Err(AppError::External(format!(
            "ingest source migration found negative {field}"
        )));
    }
    Ok(Some(value))
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "ingest_sources")
    {
        return Ok(());
    }

    let sources = db.raw().collection::<Document>("ingest_sources");
    let mut cursor = sources.find(doc! {}, None).await?;
    let mut upgraded = 0_u64;
    while let Some(row) = cursor.try_next().await? {
        let id = row.get("_id").cloned().ok_or_else(|| {
            AppError::External("ingest source migration found row without _id".to_string())
        })?;
        let source_generation = optional_nonnegative_i64(&row, "source_generation")?;
        let claim_generation = optional_nonnegative_i64(&row, "claim_generation")?;
        if source_generation.is_some() && claim_generation.is_some() {
            continue;
        }

        let mut changed = false;
        if source_generation.is_none() {
            let result = sources
                .update_one(
                    doc! {
                        "_id": id.clone(),
                        "$or": [
                            { "source_generation": { "$exists": false } },
                            { "source_generation": null },
                        ],
                    },
                    doc! { "$set": { "source_generation": 1_i64 } },
                    None,
                )
                .await?;
            changed |= result.modified_count == 1;
        }
        if claim_generation.is_none() {
            let result = sources
                .update_one(
                    doc! {
                        "_id": id,
                        "$or": [
                            { "claim_generation": { "$exists": false } },
                            { "claim_generation": null },
                        ],
                    },
                    doc! { "$set": { "claim_generation": 0_i64 } },
                    None,
                )
                .await?;
            changed |= result.modified_count == 1;
        }
        if changed {
            upgraded += 1;
        }
    }

    tracing::info!(
        migration_id = "2026_07_053_ingest_source_claims",
        upgraded_sources = upgraded,
        "upgraded ingest sources to leased generation claims"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_generation_accepts_missing_and_integer_widths() {
        assert_eq!(optional_nonnegative_i64(&doc! {}, "g").unwrap(), None);
        assert_eq!(
            optional_nonnegative_i64(&doc! { "g": null }, "g").unwrap(),
            None
        );
        assert_eq!(
            optional_nonnegative_i64(&doc! { "g": 2_i32 }, "g").unwrap(),
            Some(2)
        );
        assert_eq!(
            optional_nonnegative_i64(&doc! { "g": 3_i64 }, "g").unwrap(),
            Some(3)
        );
        assert!(optional_nonnegative_i64(&doc! { "g": -1_i64 }, "g").is_err());
        assert!(optional_nonnegative_i64(&doc! { "g": "1" }, "g").is_err());
    }
}

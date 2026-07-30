//! Upgrade asynchronous import jobs to generation-fenced claims.
//!
//! Legacy rows predate `claim_generation`. The migration validates every
//! existing value before writing, initializes only missing/null generations,
//! and deliberately preserves any active claim token and timestamp.

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

fn validate_generation(row: &Document) -> AppResult<bool> {
    match row.get("claim_generation") {
        None | Some(Bson::Null) => Ok(true),
        Some(Bson::Int32(value)) if *value >= 0 => Ok(false),
        Some(Bson::Int64(value)) if *value >= 0 => Ok(false),
        Some(_) => Err(AppError::External(
            "import job migration found invalid claim_generation".to_string(),
        )),
    }
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "import_jobs")
    {
        return Ok(());
    }

    let jobs = db.raw().collection::<Document>("import_jobs");
    let mut cursor = jobs.find(doc! {}, None).await?;
    let mut missing_ids = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        if validate_generation(&row)? {
            missing_ids.push(row.get("_id").cloned().ok_or_else(|| {
                AppError::External("import job migration found row without _id".to_string())
            })?);
        }
    }

    let mut upgraded = 0_u64;
    for id in missing_ids {
        let result = jobs
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
        upgraded += result.modified_count;
    }

    tracing::info!(
        migration_id = "2026_07_056_import_job_claims",
        upgraded_jobs = upgraded,
        "upgraded import jobs to generation-fenced claims"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_validation_is_fail_closed() {
        assert!(validate_generation(&doc! {}).unwrap());
        assert!(validate_generation(&doc! { "claim_generation": null }).unwrap());
        assert!(!validate_generation(&doc! { "claim_generation": 0_i32 }).unwrap());
        assert!(!validate_generation(&doc! { "claim_generation": 9_i64 }).unwrap());
        assert!(validate_generation(&doc! { "claim_generation": -1_i64 }).is_err());
        assert!(validate_generation(&doc! { "claim_generation": "1" }).is_err());
    }
}

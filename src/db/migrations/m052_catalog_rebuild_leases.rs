//! Upgrade legacy catalog rebuild work to the leased generation protocol.
//!
//! Old rows had no target generation or lease and could remain `processing`
//! forever. Every existing knowledge document receives one deterministic
//! reconciliation intent through a restart-safe document marker. This also repairs
//! a catalog that became stale when the old post-commit best-effort enqueue was
//! lost. Legacy non-terminal work is then retired as `superseded`; completed
//! rows remain untouched as audit history.

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::{FindOptions, UpdateOptions},
};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

const RECONCILIATION_GENERATION_FIELD: &str = "catalog_m052_reconciliation_generation";

fn optional_i64(row: &Document, field: &str) -> AppResult<i64> {
    let value = match row.get(field) {
        None | Some(mongodb::bson::Bson::Null) => 0,
        Some(mongodb::bson::Bson::Int32(value)) => i64::from(*value),
        Some(mongodb::bson::Bson::Int64(value)) => *value,
        Some(_) => {
            return Err(AppError::External(format!(
                "catalog migration found non-integer {field}"
            )))
        }
    };
    if value < 0 {
        return Err(AppError::External(format!(
            "catalog migration found negative {field}"
        )));
    }
    Ok(value)
}

async fn reconcile_document(
    db: &Database,
    document_id: ObjectId,
    workspace_id: &str,
) -> AppResult<bool> {
    let job_id = format!("crj_m052_{}", document_id.to_hex());
    let documents = db
        .raw()
        .collection::<Document>("operation_knowledge_documents");
    let target = loop {
        let parent = documents
            .find_one(
                doc! { "_id": document_id, "workspace_id": workspace_id },
                None,
            )
            .await?
            .ok_or_else(|| AppError::Conflict("catalog_migration_parent_changed".to_string()))?;
        let desired = optional_i64(&parent, "catalog_desired_generation")?;
        let applied = optional_i64(&parent, "catalog_applied_generation")?;
        if let Some(marker) = parent.get(RECONCILIATION_GENERATION_FIELD) {
            let marker = match marker {
                mongodb::bson::Bson::Int32(value) => i64::from(*value),
                mongodb::bson::Bson::Int64(value) => *value,
                _ => {
                    return Err(AppError::External(
                        "catalog migration found invalid reconciliation marker".to_string(),
                    ))
                }
            };
            if marker <= 0 || desired < marker || applied > desired {
                return Err(AppError::External(
                    "catalog migration found inconsistent reconciliation generation".to_string(),
                ));
            }
            break marker;
        }

        let candidate = desired
            .max(applied)
            .checked_add(1)
            .ok_or_else(|| AppError::Conflict("catalog_generation_exhausted".to_string()))?;
        let mut filter = doc! {
            "_id": document_id,
            "workspace_id": workspace_id,
            RECONCILIATION_GENERATION_FIELD: { "$exists": false },
        };
        filter.insert("catalog_desired_generation", generation_match(desired));
        filter.insert("catalog_applied_generation", generation_match(applied));
        let advanced = documents
            .update_one(
                filter,
                doc! {
                    "$set": {
                        "catalog_desired_generation": candidate,
                        "catalog_applied_generation": applied,
                        RECONCILIATION_GENERATION_FIELD: candidate,
                    },
                },
                None,
            )
            .await?;
        if advanced.matched_count == 1 {
            break candidate;
        }
        // A concurrent startup or catalog mutation won the CAS. Re-read its
        // marker/current generations and converge instead of allocating twice.
    };

    let now = DateTime::now();
    let jobs = db.raw().collection::<Document>("catalog_rebuild_jobs");
    let inserted = jobs
        .update_one(
            // Reuse the parent ObjectId as the reconciliation job identity.
            // `_id` is unique before application indexes are installed, so two
            // processes running migrations concurrently still converge.
            doc! { "_id": document_id },
            doc! { "$setOnInsert": {
                "_id": document_id,
                "job_id": &job_id,
                "workspace_id": workspace_id,
                "document_id": document_id,
                "queued_at": now,
                "target_generation": target,
                "status": "queued",
                "attempts": 0i32,
                "claim_generation": 0i64,
            } },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    let stored = jobs
        .find_one(doc! { "_id": document_id }, None)
        .await?
        .ok_or_else(|| {
            AppError::External("catalog migration lost reconciliation job".to_string())
        })?;
    if optional_i64(&stored, "target_generation")? != target
        || stored.get_str("job_id").ok() != Some(job_id.as_str())
        || stored.get_object_id("document_id").ok() != Some(document_id)
        || stored.get_str("workspace_id").ok() != Some(workspace_id)
    {
        return Err(AppError::External(
            "catalog migration found conflicting reconciliation job".to_string(),
        ));
    }
    Ok(inserted.upserted_id.is_some())
}

fn generation_match(value: i64) -> mongodb::bson::Bson {
    if value == 0 {
        mongodb::bson::to_bson(&doc! {
            "$in": [
                mongodb::bson::Bson::Int64(0),
                mongodb::bson::Bson::Int32(0),
                mongodb::bson::Bson::Null,
            ]
        })
        .expect("generation match serializes")
    } else {
        mongodb::bson::Bson::Int64(value)
    }
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    let names = db.raw().list_collection_names(None).await?;
    if !names
        .iter()
        .any(|name| name == "operation_knowledge_documents")
    {
        return Ok(());
    }

    let documents = db
        .raw()
        .collection::<Document>("operation_knowledge_documents");
    let mut cursor = documents
        .find(
            doc! {},
            FindOptions::builder().sort(doc! { "_id": 1 }).build(),
        )
        .await?;
    let mut identities = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        let id = row.get_object_id("_id").map_err(|_| {
            AppError::External("catalog migration found document without ObjectId _id".to_string())
        })?;
        let workspace_id = row.get_str("workspace_id").map_err(|_| {
            AppError::External(format!(
                "catalog migration found document {id} without workspace_id"
            ))
        })?;
        if workspace_id.is_empty() || workspace_id.trim() != workspace_id {
            return Err(AppError::External(format!(
                "catalog migration found document {id} with invalid workspace_id"
            )));
        }
        optional_i64(&row, "catalog_desired_generation")?;
        optional_i64(&row, "catalog_applied_generation")?;
        identities.push((id, workspace_id.to_string()));
    }

    let mut inserted = 0usize;
    for (document_id, workspace_id) in identities {
        inserted += usize::from(reconcile_document(db, document_id, &workspace_id).await?);
    }

    let retired = if names.iter().any(|name| name == "catalog_rebuild_jobs") {
        db.raw()
            .collection::<Document>("catalog_rebuild_jobs")
            .update_many(
                doc! {
                    "job_id": { "$not": { "$regex": "^crj_m052_" } },
                    "status": { "$in": ["queued", "processing", "failed"] },
                    "$or": [
                        { "target_generation": { "$exists": false } },
                        { "target_generation": null },
                        { "target_generation": { "$lte": 0i64 } },
                    ],
                },
                doc! {
                    "$set": {
                        "status": "superseded",
                        "finished_at": DateTime::now(),
                        "last_error": "retired by catalog lease/generation migration",
                    },
                    "$unset": {
                        "worker_id": "",
                        "claim_token": "",
                        "locked_until": "",
                        "next_retry_at": "",
                    },
                },
                None,
            )
            .await?
            .modified_count
    } else {
        0
    };

    tracing::info!(
        migration_id = "2026_07_052_catalog_rebuild_leases",
        reconciliation_jobs = inserted,
        retired_legacy_jobs = retired,
        "upgraded catalog rebuild work to leased generations"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_generation_accepts_legacy_missing_and_integer_widths() {
        assert_eq!(optional_i64(&doc! {}, "g").unwrap(), 0);
        assert_eq!(optional_i64(&doc! { "g": 7i32 }, "g").unwrap(), 7);
        assert_eq!(optional_i64(&doc! { "g": 9i64 }, "g").unwrap(), 9);
        assert!(optional_i64(&doc! { "g": -1i64 }, "g").is_err());
        assert!(optional_i64(&doc! { "g": "9" }, "g").is_err());
    }
}

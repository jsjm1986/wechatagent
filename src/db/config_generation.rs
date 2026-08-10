//! Lightweight cross-replica configuration generations.
//!
//! Runtime readers compare one small `(namespace, workspace)` row before consulting their local
//! cache. Runtime-visible production writers advance the row in the same transaction as the
//! authoritative mutation. Direct/manual database edits remain a
//! supported recovery path through each cache's bounded TTL, but are not an immediate-consistency
//! API.

use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument, UpdateOptions};
use mongodb::ClientSession;

use crate::db::Database;
use crate::error::{AppError, AppResult};

pub const DOMAIN_PROFILE_NAMESPACE: &str = "domain_profile";
pub const TAXONOMY_NAMESPACE: &str = "taxonomy";
pub const LLM_PROVIDER_NAMESPACE: &str = "llm_provider";

fn generation_id(namespace: &str, workspace_id: &str) -> String {
    format!("{namespace}\0{workspace_id}")
}

fn generation_update(namespace: &str, workspace_id: &str) -> Document {
    doc! {
        "$set": {
            "namespace": namespace,
            "workspace_id": workspace_id,
            "updated_at": DateTime::now(),
        },
        "$inc": { "generation": 1_i64 },
        "$setOnInsert": { "created_at": DateTime::now() },
    }
}

pub async fn read_generation(db: &Database, namespace: &str, workspace_id: &str) -> AppResult<i64> {
    let row = db
        .raw()
        .collection::<Document>("configuration_generations")
        .find_one(doc! { "_id": generation_id(namespace, workspace_id) }, None)
        .await?;
    match row {
        None => Ok(0),
        Some(row) => row
            .get_i64("generation")
            .or_else(|_| row.get_i32("generation").map(i64::from))
            .map_err(|_| {
                AppError::External(format!(
                    "invalid configuration generation for {namespace}/{workspace_id}"
                ))
            }),
    }
}

pub async fn bump_generation(db: &Database, namespace: &str, workspace_id: &str) -> AppResult<i64> {
    let row = db
        .raw()
        .collection::<Document>("configuration_generations")
        .find_one_and_update(
            doc! { "_id": generation_id(namespace, workspace_id) },
            generation_update(namespace, workspace_id),
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::External("configuration generation disappeared".into()))?;
    row.get_i64("generation")
        .or_else(|_| row.get_i32("generation").map(i64::from))
        .map_err(|_| AppError::External("invalid configuration generation result".into()))
}

pub async fn bump_generation_with_session(
    db: &Database,
    namespace: &str,
    workspace_id: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.raw()
        .collection::<Document>("configuration_generations")
        .update_one_with_session(
            doc! { "_id": generation_id(namespace, workspace_id) },
            generation_update(namespace, workspace_id),
            UpdateOptions::builder().upsert(true).build(),
            session,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_namespace_and_workspace_scoped() {
        assert_ne!(
            generation_id("taxonomy", "a"),
            generation_id("taxonomy", "b")
        );
        assert_ne!(
            generation_id("taxonomy", "a"),
            generation_id("domain_profile", "a")
        );
        assert_eq!(generation_id("taxonomy", "a"), "taxonomy\0a");
    }
}

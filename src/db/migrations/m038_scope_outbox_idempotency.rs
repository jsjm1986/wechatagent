//! Scope legacy Outbox idempotency keys by workspace and account.
//!
//! Runtime startup executes migrations before index creation. This step rewrites every
//! pre-v2 key with the exact runtime helper, then retires singleton `idempotency_key`
//! indexes. `ensure_indexes` creates the replacement compound unique index afterward.

use futures::TryStreamExt;
use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::{AppError, AppResult};

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let collection_names = db.raw().list_collection_names(None).await?;
    if !collection_names
        .iter()
        .any(|name| name == "agent_send_outbox")
    {
        return Ok(());
    }

    let collection = db.raw().collection::<Document>("agent_send_outbox");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut rewritten = 0_u64;
    while let Some(row) = cursor.try_next().await? {
        let id = row.get("_id").cloned().ok_or_else(|| {
            AppError::External("outbox idempotency migration found row without _id".to_string())
        })?;
        let workspace_id = required_non_empty(&row, "workspace_id", &id)?;
        let account_id = required_non_empty(&row, "account_id", &id)?;
        let current_key = required_non_empty(&row, "idempotency_key", &id)?;
        if crate::agent::outbox::is_scoped_outbox_idempotency_key(current_key) {
            continue;
        }

        let scoped = crate::agent::outbox::scoped_outbox_idempotency_key(
            workspace_id,
            account_id,
            current_key,
        );
        let result = collection
            .update_one(
                doc! { "_id": id.clone(), "idempotency_key": current_key },
                doc! { "$set": { "idempotency_key": &scoped } },
                None,
            )
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::External(format!(
                "outbox idempotency migration lost CAS for row {id:?}"
            )));
        }
        rewritten += 1;
    }

    let typed = db.collection_agent_send_outbox();
    let mut indexes = typed.list_indexes(None).await?;
    let mut legacy_index_names = Vec::new();
    while let Some(index) = indexes.try_next().await? {
        if index.keys == doc! { "idempotency_key": 1 } {
            if let Some(name) = index.options.and_then(|options| options.name) {
                legacy_index_names.push(name);
            }
        }
    }
    for name in legacy_index_names {
        typed.drop_index(name, None).await?;
    }

    tracing::info!(
        migration_id = "2026_07_038_scope_outbox_idempotency",
        rewritten,
        "scoped outbox idempotency keys and retired legacy singleton indexes"
    );
    Ok(())
}

fn required_non_empty<'a>(
    row: &'a Document,
    field: &str,
    id: &mongodb::bson::Bson,
) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "outbox idempotency migration found row {id:?} without {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "outbox idempotency migration found row {id:?} with non-canonical {field}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_scope_rejects_missing_or_empty_values() {
        let id = mongodb::bson::Bson::String("row-1".to_string());
        assert!(required_non_empty(&doc! {}, "workspace_id", &id).is_err());
        assert!(required_non_empty(&doc! { "workspace_id": "  " }, "workspace_id", &id).is_err());
        assert!(
            required_non_empty(&doc! { "workspace_id": " ws-a " }, "workspace_id", &id).is_err()
        );
        assert_eq!(
            required_non_empty(&doc! { "workspace_id": "ws-a" }, "workspace_id", &id).unwrap(),
            "ws-a"
        );
    }
}

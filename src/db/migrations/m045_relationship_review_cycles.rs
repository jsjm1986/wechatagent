//! Retire the legacy full-history relationship suggestion unique index.
//!
//! The replacement index is created by `Database::ensure_indexes` after migrations and is
//! unique only for `status=pending`. Terminal review rows therefore remain immutable history
//! without permanently blocking a later evidence cycle for the same contact.

use std::collections::HashSet;

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "relationship_type_suggestions")
    {
        return Ok(());
    }

    let collection = db
        .raw()
        .collection::<Document>("relationship_type_suggestions");
    let pending_rows = audit_pending_rows(&collection).await?;
    let mut indexes = collection.list_indexes(None).await?;
    let mut legacy_names = Vec::new();
    while let Some(index) = indexes.try_next().await? {
        if !has_same_key_fields(&index.keys) {
            continue;
        }
        if !has_legacy_keys(&index.keys) {
            return Err(AppError::External(format!(
                "relationship review migration found relationship key index in unexpected order {:?}; reconcile explicitly before startup",
                index.options.as_ref().and_then(|options| options.name.as_deref())
            )));
        }
        let options = index.options.unwrap_or_default();
        let is_replacement = options.unique == Some(true)
            && options.partial_filter_expression == Some(doc! { "status": "pending" });
        if is_replacement {
            continue;
        }
        let is_legacy = options.unique == Some(true) && options.partial_filter_expression.is_none();
        if !is_legacy {
            return Err(AppError::External(format!(
                "relationship review migration found unrecognized index shape {:?}; reconcile explicitly before startup",
                options.name
            )));
        }
        let name = options.name.ok_or_else(|| {
            AppError::External(
                "relationship review migration found unnamed legacy index".to_string(),
            )
        })?;
        legacy_names.push(name);
    }

    for name in &legacy_names {
        collection.drop_index(name, None).await?;
    }
    tracing::info!(
        migration_id = "2026_07_045_relationship_review_cycles",
        pending_rows,
        retired_indexes = legacy_names.len(),
        "audited relationship review cycles and retired legacy full-history uniqueness"
    );
    Ok(())
}

async fn audit_pending_rows(collection: &mongodb::Collection<Document>) -> AppResult<usize> {
    let mut cursor = collection.find(doc! { "status": "pending" }, None).await?;
    let mut seen = HashSet::new();
    let mut count = 0;
    while let Some(row) = cursor.try_next().await? {
        let id = row
            .get("_id")
            .cloned()
            .unwrap_or(Bson::String("<missing>".to_string()));
        let workspace_id = canonical(&row, "workspace_id", &id)?;
        canonical(&row, "account_id", &id)?;
        let contact_id = canonical(&row, "contact_id", &id)?;
        if !seen.insert((workspace_id.to_string(), contact_id.to_string())) {
            return Err(AppError::External(format!(
                "relationship review migration found duplicate pending key {workspace_id}/{contact_id}"
            )));
        }
        count += 1;
    }
    Ok(count)
}

fn canonical<'a>(row: &'a Document, field: &str, id: &Bson) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "relationship review migration found pending row {id:?} without {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "relationship review migration found pending row {id:?} with non-canonical {field}"
        )));
    }
    Ok(value)
}

fn has_legacy_keys(keys: &Document) -> bool {
    let expected = [
        ("workspace_id", Bson::Int32(1)),
        ("contact_id", Bson::Int32(1)),
    ];
    keys.len() == expected.len()
        && keys.iter().zip(expected.iter()).all(
            |((actual_name, actual_value), (expected_name, expected_value))| {
                actual_name == expected_name && actual_value == expected_value
            },
        )
}

fn has_same_key_fields(keys: &Document) -> bool {
    keys.len() == 2 && keys.contains_key("workspace_id") && keys.contains_key("contact_id")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_key_match_requires_exact_order_and_shape() {
        assert!(has_legacy_keys(
            &doc! { "workspace_id": 1, "contact_id": 1 }
        ));
        assert!(!has_legacy_keys(
            &doc! { "contact_id": 1, "workspace_id": 1 }
        ));
        assert!(!has_legacy_keys(
            &doc! { "workspace_id": 1, "contact_id": 1, "status": 1 }
        ));
        assert!(!has_legacy_keys(&doc! { "workspace_id": 1 }));
        assert!(has_same_key_fields(
            &doc! { "contact_id": 1, "workspace_id": 1 }
        ));
        assert!(!has_same_key_fields(
            &doc! { "workspace_id": 1, "contact_id": 1, "status": 1 }
        ));
    }

    #[test]
    fn pending_identity_must_be_canonical() {
        let id = Bson::String("row-1".to_string());
        assert_eq!(
            canonical(&doc! { "workspace_id": "ws" }, "workspace_id", &id).unwrap(),
            "ws"
        );
        assert!(canonical(&doc! { "workspace_id": " ws " }, "workspace_id", &id).is_err());
        assert!(canonical(&doc! {}, "workspace_id", &id).is_err());
    }
}

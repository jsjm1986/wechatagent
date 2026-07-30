//! Retire the legacy principal-escalation pending key that omitted `account_id`.
//!
//! The replacement partial unique index is created by `Database::ensure_indexes` after all
//! migrations. We validate every pending identity before dropping anything and only retire the
//! exact historical index shape; unknown shapes fail closed.

use std::collections::HashSet;

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

const LEGACY_NAME: &str = "uniq_principal_escalation_pending_ws_contact_category";

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "agent_principal_escalations")
    {
        return Ok(());
    }

    let collection = db
        .raw()
        .collection::<Document>("agent_principal_escalations");
    let pending_rows = audit_pending_rows(&collection).await?;
    let mut indexes = collection.list_indexes(None).await?;
    let mut retire = Vec::new();
    while let Some(index) = indexes.try_next().await? {
        if !has_legacy_key_fields(&index.keys) {
            continue;
        }
        if !has_exact_legacy_keys(&index.keys) {
            return Err(AppError::External(format!(
                "principal escalation migration found legacy key fields in unexpected order {:?}",
                index
                    .options
                    .as_ref()
                    .and_then(|options| options.name.as_deref())
            )));
        }
        let options = index.options.unwrap_or_default();
        let recognized = options.name.as_deref() == Some(LEGACY_NAME)
            && options.unique == Some(true)
            && options.partial_filter_expression == Some(doc! { "status": "pending" });
        if !recognized {
            return Err(AppError::External(format!(
                "principal escalation migration found unrecognized legacy index shape {:?}",
                options.name
            )));
        }
        retire.push(LEGACY_NAME.to_string());
    }

    for name in &retire {
        collection.drop_index(name, None).await?;
    }
    tracing::info!(
        migration_id = "2026_07_046_scope_principal_escalation_pending",
        pending_rows,
        retired_indexes = retire.len(),
        "audited account-scoped principal escalation identity and retired legacy index"
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
        let workspace = canonical(&row, "workspace_id", &id)?;
        let account = canonical(&row, "account_id", &id)?;
        let contact = canonical(&row, "contact_wxid", &id)?;
        let category = canonical(&row, "category", &id)?;
        let key = (
            workspace.to_string(),
            account.to_string(),
            contact.to_string(),
            category.to_string(),
        );
        if !seen.insert(key) {
            return Err(AppError::External(format!(
                "principal escalation migration found duplicate account-scoped pending identity at {id:?}"
            )));
        }
        count += 1;
    }
    Ok(count)
}

fn canonical<'a>(row: &'a Document, field: &str, id: &Bson) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "principal escalation migration found pending row {id:?} without {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "principal escalation migration found pending row {id:?} with non-canonical {field}"
        )));
    }
    Ok(value)
}

fn has_exact_legacy_keys(keys: &Document) -> bool {
    let expected = [
        ("workspace_id", Bson::Int32(1)),
        ("contact_wxid", Bson::Int32(1)),
        ("category", Bson::Int32(1)),
    ];
    keys.len() == expected.len()
        && keys.iter().zip(expected.iter()).all(
            |((actual_name, actual_value), (expected_name, expected_value))| {
                actual_name == expected_name && actual_value == expected_value
            },
        )
}

fn has_legacy_key_fields(keys: &Document) -> bool {
    keys.len() == 3
        && keys.contains_key("workspace_id")
        && keys.contains_key("contact_wxid")
        && keys.contains_key("category")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_index_match_is_exact() {
        assert!(has_exact_legacy_keys(
            &doc! { "workspace_id": 1, "contact_wxid": 1, "category": 1 }
        ));
        assert!(!has_exact_legacy_keys(
            &doc! { "contact_wxid": 1, "workspace_id": 1, "category": 1 }
        ));
        assert!(has_legacy_key_fields(
            &doc! { "category": 1, "contact_wxid": 1, "workspace_id": 1 }
        ));
    }

    #[test]
    fn pending_identity_requires_account_and_canonical_values() {
        let id = Bson::String("row-1".to_string());
        assert_eq!(
            canonical(&doc! { "account_id": "a" }, "account_id", &id).unwrap(),
            "a"
        );
        assert!(canonical(&doc! {}, "account_id", &id).is_err());
        assert!(canonical(&doc! { "account_id": " a " }, "account_id", &id).is_err());
    }
}

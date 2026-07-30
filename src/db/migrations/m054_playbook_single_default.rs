//! Fail closed before installing the account-scoped Playbook default index.
//!
//! A missing default is recoverable by the normal lazy bootstrap path. Multiple
//! defaults are ambiguous business state, so this migration audits without
//! guessing a winner. Legacy rows predate release state and are deterministically
//! backfilled as published; explicit draft/published values are never rewritten.

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::{Bson, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

fn canonical<'a>(row: &'a Document, field: &str) -> AppResult<&'a str> {
    let value = row
        .get_str(field)
        .map_err(|_| AppError::External(format!("playbook default audit found invalid {field}")))?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "playbook default audit found non-canonical {field}"
        )));
    }
    Ok(value)
}

fn audit_rows(rows: impl IntoIterator<Item = Document>) -> AppResult<usize> {
    let mut scopes = HashSet::new();
    let mut defaults: HashMap<(String, String), usize> = HashMap::new();
    for row in rows {
        let scope = (
            canonical(&row, "workspace_id")?.to_string(),
            canonical(&row, "account_id")?.to_string(),
        );
        scopes.insert(scope.clone());
        let is_default = row.get_bool("is_default").map_err(|_| {
            AppError::External("playbook default audit found invalid is_default".to_string())
        })?;
        let release_status = match row.get("release_status") {
            None => None,
            Some(Bson::String(status)) if matches!(status.as_str(), "draft" | "published") => {
                Some(status.as_str())
            }
            Some(_) => {
                return Err(AppError::External(
                    "playbook default audit found invalid release_status".to_string(),
                ));
            }
        };
        if is_default && release_status == Some("draft") {
            return Err(AppError::External(
                "playbook default audit found draft default".to_string(),
            ));
        }
        if is_default {
            *defaults.entry(scope).or_default() += 1;
        }
    }
    if let Some((scope, count)) = defaults.iter().find(|(_, count)| **count > 1) {
        return Err(AppError::External(format!(
            "playbook default audit found {count} defaults for {scope:?}"
        )));
    }
    Ok(scopes.len())
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "operation_playbooks")
    {
        return Ok(());
    }
    let collection = db.raw().collection::<Document>("operation_playbooks");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }
    let scopes = audit_rows(rows)?;
    let backfilled = collection
        .update_many(
            mongodb::bson::doc! { "release_status": { "$exists": false } },
            mongodb::bson::doc! { "$set": { "release_status": "published" } },
            None,
        )
        .await?
        .modified_count;
    tracing::info!(
        migration_id = "2026_07_054_playbook_single_default",
        scopes,
        backfilled,
        "audited operation playbook default invariants"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn permits_zero_or_one_default_and_rejects_multiple() {
        assert_eq!(
            audit_rows([
                doc! { "workspace_id": "ws", "account_id": "a", "is_default": false },
                doc! { "workspace_id": "ws", "account_id": "a", "is_default": true },
                doc! { "workspace_id": "ws", "account_id": "b", "is_default": false },
            ])
            .unwrap(),
            2
        );
        assert!(audit_rows([
            doc! { "workspace_id": "ws", "account_id": "a", "is_default": true },
            doc! { "workspace_id": "ws", "account_id": "a", "is_default": true },
        ])
        .unwrap_err()
        .to_string()
        .contains("2 defaults"));
        assert!(audit_rows([doc! {
            "workspace_id": "ws",
            "account_id": "a",
            "is_default": true,
            "release_status": "draft",
        },])
        .unwrap_err()
        .to_string()
        .contains("draft default"));
        assert!(audit_rows([doc! {
            "workspace_id": "ws",
            "account_id": "a",
            "is_default": false,
            "release_status": 1,
        }])
        .unwrap_err()
        .to_string()
        .contains("invalid release_status"));
    }
}

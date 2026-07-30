//! Audit send-ledger delivery anchors before the unique index is ensured.
//!
//! Legacy rows without `outbox_id` remain readable and are intentionally left
//! untouched: there is no safe way to infer one delivery from run/target/time.
//! Rows that claim an anchor must use an ObjectId, carry canonical tenant
//! scope, and be globally unique. The migration is read-only and fails before
//! index creation when reconciliation needs an operator decision; it never
//! deletes or merges historical attribution data automatically.

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::{Bson, Document};

use crate::db::Database;
use crate::error::{AppError, AppResult};

#[derive(Debug)]
struct SeenAnchor {
    row_id: String,
    workspace_id: String,
    account_id: String,
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "agent_send_ledger")
    {
        return Ok(());
    }

    let collection = db.raw().collection::<Document>("agent_send_ledger");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut seen = HashMap::new();
    let mut anchored = 0_u64;
    let mut legacy_unanchored = 0_u64;

    while let Some(row) = cursor.try_next().await? {
        if audit_row(&row, &mut seen)? {
            anchored += 1;
        } else {
            legacy_unanchored += 1;
        }
    }

    tracing::info!(
        migration_id = "2026_07_041_audit_send_ledger_anchors",
        anchored,
        legacy_unanchored,
        "audited send-ledger anchors before unique-index creation"
    );
    Ok(())
}

fn audit_row(
    row: &Document,
    seen: &mut HashMap<mongodb::bson::oid::ObjectId, SeenAnchor>,
) -> AppResult<bool> {
    let row_id = row
        .get("_id")
        .map(|value| format!("{value:?}"))
        .unwrap_or_else(|| "<missing _id>".to_string());
    let outbox_id = match row.get("outbox_id") {
        None | Some(Bson::Null) => return Ok(false),
        Some(Bson::ObjectId(id)) => *id,
        Some(other) => {
            return Err(AppError::External(format!(
                "send-ledger anchor audit found row {row_id} with non-ObjectId outbox_id {other:?}"
            )))
        }
    };
    let workspace_id = required_scope(row, "workspace_id", &row_id)?;
    let account_id = required_scope(row, "account_id", &row_id)?;

    if let Some(existing) = seen.insert(
        outbox_id,
        SeenAnchor {
            row_id: row_id.clone(),
            workspace_id: workspace_id.to_string(),
            account_id: account_id.to_string(),
        },
    ) {
        return Err(AppError::External(format!(
            "send-ledger anchor audit found duplicate outbox_id {outbox_id}: row {} ({}/{}) and row {row_id} ({workspace_id}/{account_id}); reconcile explicitly before startup",
            existing.row_id, existing.workspace_id, existing.account_id,
        )));
    }
    Ok(true)
}

fn required_scope<'a>(row: &'a Document, field: &str, row_id: &str) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "send-ledger anchor audit found anchored row {row_id} without {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "send-ledger anchor audit found anchored row {row_id} with non-canonical {field}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{doc, oid::ObjectId};

    #[test]
    fn legacy_unanchored_rows_are_allowed_without_guessing() {
        let mut seen = HashMap::new();
        assert!(!audit_row(
            &doc! { "_id": ObjectId::new(), "workspace_id": "ws", "account_id": "acct" },
            &mut seen,
        )
        .unwrap());
        assert!(seen.is_empty());
    }

    #[test]
    fn duplicate_anchor_is_rejected_even_across_accounts() {
        let outbox_id = ObjectId::new();
        let mut seen = HashMap::new();
        assert!(audit_row(
            &doc! {
                "_id": ObjectId::new(), "outbox_id": outbox_id,
                "workspace_id": "ws", "account_id": "acct-a"
            },
            &mut seen,
        )
        .unwrap());
        let error = audit_row(
            &doc! {
                "_id": ObjectId::new(), "outbox_id": outbox_id,
                "workspace_id": "ws", "account_id": "acct-b"
            },
            &mut seen,
        )
        .expect_err("duplicate delivery attribution must fail closed");
        assert!(error.to_string().contains("duplicate outbox_id"));
    }

    #[test]
    fn malformed_anchor_or_scope_is_rejected() {
        let mut seen = HashMap::new();
        assert!(audit_row(
            &doc! {
                "_id": ObjectId::new(), "outbox_id": "not-an-object-id",
                "workspace_id": "ws", "account_id": "acct"
            },
            &mut seen,
        )
        .is_err());
        assert!(audit_row(
            &doc! {
                "_id": ObjectId::new(), "outbox_id": ObjectId::new(),
                "workspace_id": "ws", "account_id": " acct "
            },
            &mut seen,
        )
        .is_err());
    }
}

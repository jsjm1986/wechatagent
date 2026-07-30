//! Audit DomainSchema identity and active-pointer invariants before unique indexes.
//!
//! This migration deliberately does not elect a winner. Duplicate versions or
//! multiple active rows require an operator decision and fail before any write.

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::Document;

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

type Lineage = (String, String);

fn audit_rows(rows: impl IntoIterator<Item = Document>) -> AppResult<usize> {
    let mut versions: HashMap<Lineage, HashSet<i32>> = HashMap::new();
    let mut active_by_workspace: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let workspace = canonical(&row, "workspace_id")?;
        let schema_id = canonical(&row, "schema_id")?;
        let version = row.get_i32("version").map_err(|_| {
            AppError::External("domain schema migration found invalid version".to_string())
        })?;
        if version <= 0 {
            return Err(AppError::External(
                "domain schema migration found non-positive version".to_string(),
            ));
        }
        if !versions
            .entry((workspace.to_string(), schema_id.to_string()))
            .or_default()
            .insert(version)
        {
            return Err(AppError::External(format!(
                "domain schema migration found duplicate version {version} for {workspace}/{schema_id}"
            )));
        }
        let active = row.get_bool("is_active").map_err(|_| {
            AppError::External("domain schema migration found invalid is_active".to_string())
        })?;
        if active {
            *active_by_workspace
                .entry(workspace.to_string())
                .or_default() += 1;
        }
    }

    if let Some((workspace, count)) = active_by_workspace.iter().find(|(_, count)| **count > 1) {
        return Err(AppError::External(format!(
            "domain schema migration found {count} active rows for workspace {workspace}"
        )));
    }
    Ok(versions.len())
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "domain_schemas")
    {
        return Ok(());
    }

    let collection = db.raw().collection::<Document>("domain_schemas");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }
    let lineages = audit_rows(rows)?;

    tracing::info!(
        migration_id = "2026_07_044_domain_schema_single_active",
        lineages,
        "audited domain schema version and active invariants"
    );
    Ok(())
}

fn canonical<'a>(row: &'a Document, field: &str) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!("domain schema migration found invalid {field}"))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "domain schema migration found non-canonical {field}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn canonical_identity_rejects_empty_or_padded_values() {
        assert_eq!(
            canonical(&doc! { "schema_id": "sales" }, "schema_id").unwrap(),
            "sales"
        );
        assert!(canonical(&doc! { "schema_id": " sales " }, "schema_id").is_err());
        assert!(canonical(&doc! { "schema_id": "" }, "schema_id").is_err());
    }

    #[test]
    fn audit_rejects_duplicate_versions_and_multiple_active_rows() {
        let row = |schema_id: &str, version: i32, active: bool| {
            doc! {
                "workspace_id": "ws",
                "schema_id": schema_id,
                "version": version,
                "is_active": active,
            }
        };
        let duplicate = audit_rows([row("sales", 1, false), row("sales", 1, false)])
            .unwrap_err()
            .to_string();
        assert!(duplicate.contains("duplicate version"));

        let multiple_active = audit_rows([row("sales", 1, true), row("support", 1, true)])
            .unwrap_err()
            .to_string();
        assert!(multiple_active.contains("2 active rows"));

        assert_eq!(
            audit_rows([row("sales", 1, true), row("sales", 2, false)]).unwrap(),
            1
        );
    }
}

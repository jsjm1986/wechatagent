//! Audit DomainProfile release identities before unique indexes are created.
//!
//! Draft-only lineages may have no current row. A lineage may have at most one
//! current published row, and a workspace may have at most one active runtime
//! row. Active and current are deliberately independent: publishing a successor
//! does not activate it or stop the old runtime version.

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::{doc, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

type Lineage = (String, String);

fn canonical<'a>(row: &'a Document, field: &str) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "domain profile invariant audit found invalid {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "domain profile invariant audit found non-canonical {field}"
        )));
    }
    Ok(value)
}

fn audit_rows(rows: impl IntoIterator<Item = Document>) -> AppResult<usize> {
    let mut versions: HashMap<Lineage, HashSet<i32>> = HashMap::new();
    let mut currents: HashMap<Lineage, usize> = HashMap::new();
    let mut active_by_workspace: HashMap<String, usize> = HashMap::new();

    for row in rows {
        let workspace = canonical(&row, "workspace_id")?.to_string();
        let profile_id = canonical(&row, "profile_id")?.to_string();
        let lineage = (workspace.clone(), profile_id);
        let version = row.get_i32("version").map_err(|_| {
            AppError::External("domain profile invariant audit found invalid version".to_string())
        })?;
        if version <= 0 {
            return Err(AppError::External(
                "domain profile invariant audit found non-positive version".to_string(),
            ));
        }
        if !versions.entry(lineage.clone()).or_default().insert(version) {
            return Err(AppError::External(format!(
                "domain profile invariant audit found duplicate version {version} for {lineage:?}"
            )));
        }
        let current = row.get_bool("current_version").map_err(|_| {
            AppError::External(
                "domain profile invariant audit found invalid current_version".to_string(),
            )
        })?;
        let active = row.get_bool("is_active").map_err(|_| {
            AppError::External("domain profile invariant audit found invalid is_active".to_string())
        })?;
        if current {
            *currents.entry(lineage).or_default() += 1;
        }
        if active {
            *active_by_workspace.entry(workspace).or_default() += 1;
        }
    }

    if let Some((lineage, count)) = currents.iter().find(|(_, count)| **count > 1) {
        return Err(AppError::External(format!(
            "domain profile invariant audit found {count} current rows for {lineage:?}"
        )));
    }
    if let Some((workspace, count)) = active_by_workspace.iter().find(|(_, count)| **count > 1) {
        return Err(AppError::External(format!(
            "domain profile invariant audit found {count} active rows for workspace {workspace}"
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
        .any(|name| name == "domain_profiles")
    {
        return Ok(());
    }
    let collection = db.raw().collection::<Document>("domain_profiles");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }
    let lineages = audit_rows(rows)?;
    let backfilled = collection
        .update_many(
            doc! { "release_status": { "$exists": false } },
            doc! { "$set": { "release_status": "published" } },
            None,
        )
        .await?
        .modified_count;
    tracing::info!(
        migration_id = "2026_07_051_domain_profile_release_invariants",
        lineages,
        backfilled,
        "audited domain profile release invariants"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    fn row(workspace: &str, profile: &str, version: i32, current: bool, active: bool) -> Document {
        doc! {
            "workspace_id": workspace,
            "profile_id": profile,
            "version": version,
            "current_version": current,
            "is_active": active,
        }
    }

    #[test]
    fn permits_draft_only_and_active_independent_from_current() {
        assert_eq!(
            audit_rows([
                row("ws", "sales", 1, false, true),
                row("ws", "sales", 2, true, false),
                row("ws", "draft", 1, false, false),
            ])
            .unwrap(),
            2
        );
    }

    #[test]
    fn rejects_duplicate_version_current_or_active() {
        assert!(audit_rows([
            row("ws", "sales", 1, false, false),
            row("ws", "sales", 1, false, false),
        ])
        .unwrap_err()
        .to_string()
        .contains("duplicate version"));
        assert!(audit_rows([
            row("ws", "sales", 1, true, false),
            row("ws", "sales", 2, true, false),
        ])
        .unwrap_err()
        .to_string()
        .contains("current rows"));
        assert!(audit_rows([
            row("ws", "sales", 1, true, true),
            row("ws", "support", 1, true, true),
        ])
        .unwrap_err()
        .to_string()
        .contains("active rows"));
    }
}

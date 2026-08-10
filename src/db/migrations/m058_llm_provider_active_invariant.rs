//! Audit the text-provider active pointer before its partial unique index is created.
//!
//! Multiple active rows are ambiguous operator-owned configuration. This migration is deliberately
//! read-only and fail-closed: it reports the exact workspace/provider identities and performs no
//! election. `ensure_indexes` runs only after this audit succeeds.

use std::collections::BTreeMap;

use futures::TryStreamExt;
use mongodb::bson::{doc, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

fn audit_rows(rows: impl IntoIterator<Item = Document>) -> AppResult<usize> {
    let mut active: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let workspace = row.get_str("workspaceId").map_err(|_| {
            AppError::External("active LLM provider has invalid workspaceId".into())
        })?;
        let provider = row
            .get_str("providerId")
            .map_err(|_| AppError::External("active LLM provider has invalid providerId".into()))?;
        if workspace.trim().is_empty() || provider.trim().is_empty() {
            return Err(AppError::External(
                "active LLM provider has empty identity".into(),
            ));
        }
        active
            .entry(workspace.to_string())
            .or_default()
            .push(provider.to_string());
    }
    if let Some((workspace, providers)) = active.iter().find(|(_, providers)| providers.len() > 1) {
        return Err(AppError::Conflict(format!(
            "multiple active LLM providers for workspace {workspace}: {}",
            providers.join(",")
        )));
    }
    Ok(active.len())
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "llm_provider_configs")
    {
        return Ok(());
    }
    let mut cursor = db
        .raw()
        .collection::<Document>("llm_provider_configs")
        .find(doc! { "isActive": true }, None)
        .await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }
    let workspaces = audit_rows(rows)?;
    tracing::info!(
        migration_id = "2026_08_058_llm_provider_active_invariant",
        workspaces,
        "audited active LLM provider invariant"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_active_pointer_without_election() {
        assert_eq!(
            audit_rows([doc! {"workspaceId":"a","providerId":"p1"}]).unwrap(),
            1
        );
        let err = audit_rows([
            doc! {"workspaceId":"a","providerId":"p1"},
            doc! {"workspaceId":"a","providerId":"p2"},
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("workspace a"));
        assert!(err.contains("p1,p2"));
    }
}

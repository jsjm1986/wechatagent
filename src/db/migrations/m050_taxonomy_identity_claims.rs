//! Backfill and lock the canonical/alias identity namespace for Taxonomy.
//!
//! Every current active value owns its canonical id and every alias in one
//! `(workspace_id, scope, kind)` namespace. The migration validates the whole
//! collection before its first write, so ambiguous legacy data fails closed
//! without partially choosing a winner. After validation, every historical
//! version receives `value.identityClaims`; this keeps future rollout of an
//! old version under the same database uniqueness constraint.

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
    models::taxonomy_identity_claims,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaimPlan {
    id: ObjectId,
    claims: Vec<String>,
}

fn canonical<'a>(row: &'a Document, field: &str, id: ObjectId) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "taxonomy identity migration found {id} without string {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "taxonomy identity migration found {id} with non-canonical {field}"
        )));
    }
    Ok(value)
}

fn aliases(row: &Document, id: ObjectId) -> AppResult<Vec<String>> {
    let value = row.get_array("aliases").map_err(|_| {
        AppError::External(format!(
            "taxonomy identity migration found {id} without array value.aliases"
        ))
    })?;
    let mut aliases = Vec::with_capacity(value.len());
    for item in value {
        let alias = item.as_str().ok_or_else(|| {
            AppError::External(format!(
                "taxonomy identity migration found {id} with non-string alias"
            ))
        })?;
        if alias.is_empty() || alias.trim() != alias {
            return Err(AppError::External(format!(
                "taxonomy identity migration found {id} with non-canonical alias"
            )));
        }
        if aliases.iter().any(|existing| existing == alias) {
            return Err(AppError::External(format!(
                "taxonomy identity migration found {id} with duplicate alias {alias}"
            )));
        }
        aliases.push(alias.to_string());
    }
    Ok(aliases)
}

fn plan_rows(rows: impl IntoIterator<Item = Document>) -> AppResult<Vec<ClaimPlan>> {
    let mut owners: HashMap<(String, String, String, String), ObjectId> = HashMap::new();
    let mut plans = Vec::new();

    for row in rows {
        let id = row.get_object_id("_id").map_err(|_| {
            AppError::External(
                "taxonomy identity migration found row without ObjectId _id".to_string(),
            )
        })?;
        let workspace = canonical(&row, "workspace_id", id)?.to_string();
        let scope = canonical(&row, "scope", id)?.to_string();
        let kind = canonical(&row, "kind", id)?.to_string();
        let value = row.get_document("value").map_err(|_| {
            AppError::External(format!(
                "taxonomy identity migration found {id} without value document"
            ))
        })?;
        let canonical_id = canonical(value, "id", id)?;
        let aliases = aliases(value, id)?;
        let claims = taxonomy_identity_claims(canonical_id, &aliases);
        if claims.len() != aliases.len() + 1 {
            return Err(AppError::External(format!(
                "taxonomy identity migration found {id} with alias equal to canonical id"
            )));
        }

        let current = row.get_bool("current_version").map_err(|_| {
            AppError::External(format!(
                "taxonomy identity migration found {id} without bool current_version"
            ))
        })?;
        let status = canonical(value, "status", id)?;
        if status != "active" && status != "deprecated" {
            return Err(AppError::External(format!(
                "taxonomy identity migration found {id} with invalid status {status}"
            )));
        }

        if current && status == "active" {
            for claim in &claims {
                let key = (
                    workspace.clone(),
                    scope.clone(),
                    kind.clone(),
                    claim.clone(),
                );
                if let Some(previous) = owners.insert(key, id) {
                    return Err(AppError::External(format!(
                        "taxonomy identity migration found ambiguous active claim {claim} for {workspace}/{scope}/{kind}: {previous} and {id}"
                    )));
                }
            }
        }
        plans.push(ClaimPlan { id, claims });
    }

    plans.sort_by_key(|plan| plan.id.to_hex());
    Ok(plans)
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "system_taxonomies")
    {
        return Ok(());
    }

    let collection = db.raw().collection::<Document>("system_taxonomies");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }

    // Full validation finishes before the first update.
    let plans = plan_rows(rows)?;
    for plan in &plans {
        let result = collection
            .update_one(
                doc! { "_id": plan.id },
                doc! { "$set": { "value.identityClaims": Bson::Array(
                    plan.claims.iter().cloned().map(Bson::String).collect()
                ) } },
                None,
            )
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::External(format!(
                "taxonomy identity migration lost row {} during backfill",
                plan.id
            )));
        }
    }

    tracing::info!(
        migration_id = "2026_07_050_taxonomy_identity_claims",
        rows = plans.len(),
        "audited and backfilled taxonomy identity claims"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: ObjectId, canonical_id: &str, aliases: &[&str], current: bool) -> Document {
        doc! {
            "_id": id,
            "workspace_id": "ws",
            "scope": "global",
            "kind": "stage",
            "value": {
                "id": canonical_id,
                "aliases": aliases,
                "status": "active",
            },
            "current_version": current,
        }
    }

    #[test]
    fn plans_canonical_and_alias_claims_for_all_versions() {
        let current = ObjectId::new();
        let historical = ObjectId::new();
        let plans = plan_rows([
            row(current, "first", &["one"], true),
            row(historical, "first", &["legacy"], false),
        ])
        .unwrap();
        assert_eq!(plans.len(), 2);
        assert!(plans.iter().any(|plan| {
            plan.id == current && plan.claims == ["first".to_string(), "one".to_string()]
        }));
        assert!(plans.iter().any(|plan| {
            plan.id == historical && plan.claims == ["first".to_string(), "legacy".to_string()]
        }));
    }

    #[test]
    fn rejects_alias_to_alias_and_alias_to_canonical_ambiguity() {
        let duplicate_alias = plan_rows([
            row(ObjectId::new(), "first", &["shared"], true),
            row(ObjectId::new(), "second", &["shared"], true),
        ])
        .unwrap_err()
        .to_string();
        assert!(duplicate_alias.contains("ambiguous active claim shared"));

        let alias_to_canonical = plan_rows([
            row(ObjectId::new(), "first", &["second"], true),
            row(ObjectId::new(), "second", &[], true),
        ])
        .unwrap_err()
        .to_string();
        assert!(alias_to_canonical.contains("ambiguous active claim second"));
    }

    #[test]
    fn rejects_noncanonical_or_duplicate_claims() {
        assert!(plan_rows([row(ObjectId::new(), " first ", &[], true)]).is_err());
        assert!(plan_rows([row(ObjectId::new(), "first", &[" alias "], true)]).is_err());
        assert!(plan_rows([row(ObjectId::new(), "first", &["x", "x"], true)]).is_err());
        assert!(plan_rows([row(ObjectId::new(), "first", &["first"], true)]).is_err());
    }
}

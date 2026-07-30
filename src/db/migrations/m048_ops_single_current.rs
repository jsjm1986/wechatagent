//! Reconcile the three versioned operations tables to one current row per logical scope.
//!
//! SR-008 product decision: versions remain append-only history, but a logical
//! scope has exactly one effective version.  Existing scopes with one current
//! row are preserved.  Zero/multiple-current legacy scopes deterministically
//! elect the greatest `(version, _id)` row before partial unique indexes are
//! created.  All three collections are parsed and validated before the first
//! write, so malformed identity/version data cannot cause a partial repair.

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

#[derive(Clone, Copy)]
struct CollectionSpec {
    name: &'static str,
    scope_fields: &'static [&'static str],
}

const SPECS: &[CollectionSpec] = &[
    CollectionSpec {
        name: "operation_domain_configs",
        scope_fields: &["workspace_id", "domain"],
    },
    CollectionSpec {
        name: "operation_state_policies",
        scope_fields: &["workspace_id", "domain", "state_key"],
    },
    CollectionSpec {
        name: "system_taxonomies",
        scope_fields: &["workspace_id", "scope", "kind", "value.id"],
    },
];

#[derive(Debug, Clone)]
struct Row {
    id: ObjectId,
    version: i32,
    current: bool,
}

#[derive(Debug)]
struct ScopePlan {
    filter: Document,
    winner: ObjectId,
    winner_version: i32,
    needs_repair: bool,
}

#[derive(Debug)]
struct CollectionPlan {
    name: &'static str,
    scopes: Vec<ScopePlan>,
}

fn canonical_component(document: &Document, path: &str, id: ObjectId) -> AppResult<String> {
    let value = if let Some((outer, inner)) = path.split_once('.') {
        document
            .get_document(outer)
            .ok()
            .and_then(|nested| nested.get_str(inner).ok())
    } else {
        document.get_str(path).ok()
    }
    .ok_or_else(|| {
        AppError::External(format!(
            "ops single-current migration found {id} without string {path}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "ops single-current migration found {id} with non-canonical {path}"
        )));
    }
    Ok(value.to_string())
}

fn scope_filter(fields: &[&str], values: &[String]) -> Document {
    fields
        .iter()
        .zip(values)
        .map(|(field, value)| ((*field).to_string(), Bson::String(value.clone())))
        .collect()
}

fn plan_documents(spec: CollectionSpec, documents: Vec<Document>) -> AppResult<CollectionPlan> {
    let mut grouped: HashMap<Vec<String>, Vec<Row>> = HashMap::new();
    let mut versions: HashMap<Vec<String>, HashSet<i32>> = HashMap::new();
    for document in documents {
        let id = document.get_object_id("_id").map_err(|_| {
            AppError::External(format!(
                "ops single-current migration found {} row without ObjectId _id",
                spec.name
            ))
        })?;
        let scope = spec
            .scope_fields
            .iter()
            .map(|field| canonical_component(&document, field, id))
            .collect::<AppResult<Vec<_>>>()?;
        let version = document.get_i32("version").map_err(|_| {
            AppError::External(format!(
                "ops single-current migration found {id} without int32 version"
            ))
        })?;
        if version <= 0 {
            return Err(AppError::External(format!(
                "ops single-current migration found {id} with non-positive version"
            )));
        }
        if !versions.entry(scope.clone()).or_default().insert(version) {
            return Err(AppError::External(format!(
                "ops single-current migration found duplicate version {version} in {} scope {:?}",
                spec.name, scope
            )));
        }
        let current = document.get_bool("current_version").map_err(|_| {
            AppError::External(format!(
                "ops single-current migration found {id} without bool current_version"
            ))
        })?;
        grouped.entry(scope).or_default().push(Row {
            id,
            version,
            current,
        });
    }

    let mut scopes = Vec::with_capacity(grouped.len());
    for (scope, rows) in grouped {
        let currents: Vec<&Row> = rows.iter().filter(|row| row.current).collect();
        let (winner, needs_repair) = if currents.len() == 1 {
            (currents[0], false)
        } else {
            (
                rows.iter()
                    .max_by_key(|row| (row.version, row.id.to_hex()))
                    .ok_or_else(|| {
                        AppError::External(
                            "ops single-current scope unexpectedly empty".to_string(),
                        )
                    })?,
                true,
            )
        };
        scopes.push(ScopePlan {
            filter: scope_filter(spec.scope_fields, &scope),
            winner: winner.id,
            winner_version: winner.version,
            needs_repair,
        });
    }
    scopes.sort_by(|left, right| left.filter.to_string().cmp(&right.filter.to_string()));
    Ok(CollectionPlan {
        name: spec.name,
        scopes,
    })
}

async fn load_plan(db: &Database, spec: CollectionSpec) -> AppResult<CollectionPlan> {
    let collection = db.raw().collection::<Document>(spec.name);
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut documents = Vec::new();
    while let Some(document) = cursor.try_next().await? {
        documents.push(document);
    }
    plan_documents(spec, documents)
}

async fn apply_plan(db: &Database, plan: &CollectionPlan) -> AppResult<u64> {
    let collection = db.raw().collection::<Document>(plan.name);
    let now = mongodb::bson::DateTime::now();
    let mut repaired = 0_u64;
    for scope in plan.scopes.iter().filter(|scope| scope.needs_repair) {
        let mut winner_filter = scope.filter.clone();
        winner_filter.insert("_id", scope.winner);
        winner_filter.insert("version", scope.winner_version);
        let promoted = collection
            .update_one(
                winner_filter,
                doc! { "$set": { "current_version": true, "updated_at": now } },
                None,
            )
            .await?;
        if promoted.matched_count != 1 {
            return Err(AppError::External(format!(
                "ops single-current migration lost winner CAS in {} for {}",
                plan.name, scope.winner
            )));
        }
        let mut siblings = scope.filter.clone();
        siblings.insert("_id", doc! { "$ne": scope.winner });
        collection
            .update_many(
                siblings,
                doc! { "$set": { "current_version": false, "updated_at": now } },
                None,
            )
            .await?;
        repaired += 1;
    }
    Ok(repaired)
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    let existing: HashSet<String> = db
        .raw()
        .list_collection_names(None)
        .await?
        .into_iter()
        .collect();
    let mut plans = Vec::new();
    for spec in SPECS
        .iter()
        .copied()
        .filter(|spec| existing.contains(spec.name))
    {
        plans.push(load_plan(db, spec).await?);
    }

    // Validation of every collection is complete before this first mutation.
    for plan in &plans {
        let repaired = apply_plan(db, plan).await?;
        tracing::info!(
            migration_id = "2026_07_048_ops_single_current",
            collection = plan.name,
            scopes = plan.scopes.len(),
            repaired,
            "reconciled operations current pointers"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: ObjectId, version: i32, current: bool) -> Document {
        doc! {
            "_id": id,
            "workspace_id": "ws",
            "domain": "user_operations",
            "version": version,
            "current_version": current,
        }
    }

    #[test]
    fn preserves_one_pointer_and_elects_highest_for_zero_or_many() {
        let spec = SPECS[0];
        let existing = ObjectId::new();
        let plan = plan_documents(
            spec,
            vec![row(existing, 1, true), row(ObjectId::new(), 2, false)],
        )
        .unwrap();
        assert_eq!(plan.scopes[0].winner, existing);
        assert!(!plan.scopes[0].needs_repair);

        let low = ObjectId::new();
        let high = ObjectId::new();
        let plan = plan_documents(spec, vec![row(low, 1, false), row(high, 2, false)]).unwrap();
        assert_eq!(plan.scopes[0].winner, high);
        assert!(plan.scopes[0].needs_repair);

        let plan = plan_documents(spec, vec![row(low, 1, true), row(high, 2, true)]).unwrap();
        assert_eq!(plan.scopes[0].winner, high);
        assert!(plan.scopes[0].needs_repair);
    }

    #[test]
    fn rejects_noncanonical_identity_and_duplicate_version() {
        let spec = SPECS[0];
        let mut invalid = row(ObjectId::new(), 1, true);
        invalid.insert("domain", " user_operations ");
        assert!(plan_documents(spec, vec![invalid]).is_err());
        assert!(plan_documents(
            spec,
            vec![
                row(ObjectId::new(), 1, true),
                row(ObjectId::new(), 1, false)
            ]
        )
        .unwrap_err()
        .to_string()
        .contains("duplicate version"));
    }
}

//! Reconcile legacy Evolution threshold overrides with the revision/CAS protocol.
//!
//! The runtime creates indexes after migrations. This step therefore validates every
//! legacy row, gives it a deterministic immutable revision, and elects exactly one
//! unrolled current row for each `(workspace, account, gate)` before partial unique
//! indexes are created. No tenant or threshold value is guessed.

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};

use crate::db::Database;
use crate::error::{AppError, AppResult};

type ScopeKey = (String, String, String);
type ProposalArtifactKey = (String, String, ObjectId);

#[derive(Debug)]
struct ReconciledOverride {
    id: ObjectId,
    expected_revision: String,
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "threshold_overrides")
    {
        return Ok(());
    }

    let collection = db.raw().collection::<Document>("threshold_overrides");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut newest_unrolled: HashMap<ScopeKey, (i64, ObjectId)> = HashMap::new();
    let mut proposal_artifacts: HashMap<ProposalArtifactKey, ObjectId> = HashMap::new();
    let mut reconciliation_plan = Vec::new();

    while let Some(row) = cursor.try_next().await? {
        let id = required_object_id(&row, "_id", None)?;
        let workspace_id = required_canonical_string(&row, "workspace_id", id)?;
        let account_id = required_canonical_string(&row, "account_id", id)?;
        let gate_key = required_canonical_string(&row, "gate_key", id)?;
        let source_proposal_id = required_object_id(&row, "source_proposal_id", Some(id))?;
        let artifact_key = (
            workspace_id.to_string(),
            account_id.to_string(),
            source_proposal_id,
        );
        if let Some(existing_id) = proposal_artifacts.insert(artifact_key, id) {
            return Err(AppError::External(format!(
                "evolution release migration found duplicate threshold artifacts for proposal {source_proposal_id}: {existing_id} and {id}"
            )));
        }
        let value = row.get_f64("value").map_err(|_| {
            AppError::External(format!(
                "evolution release migration found override {id} without a double value"
            ))
        })?;
        if !value.is_finite() {
            return Err(AppError::External(format!(
                "evolution release migration found override {id} with non-finite value"
            )));
        }
        let released_at = row.get_datetime("released_at").map_err(|_| {
            AppError::External(format!(
                "evolution release migration found override {id} without released_at"
            ))
        })?;

        let expected_revision = crate::evolution::revision::threshold_revision(Some(id), value);
        if let Ok(existing) = row.get_str("released_revision") {
            if existing != expected_revision {
                return Err(AppError::External(format!(
                    "evolution release migration found override {id} with conflicting released_revision"
                )));
            }
        }
        reconciliation_plan.push(ReconciledOverride {
            id,
            expected_revision,
        });

        let rolled_back = match row.get("rolled_back_at") {
            None | Some(Bson::Null) => false,
            Some(Bson::DateTime(_)) => true,
            Some(_) => {
                return Err(AppError::External(format!(
                    "evolution release migration found override {id} with invalid rolled_back_at"
                )))
            }
        };
        if !rolled_back {
            let scope = (
                workspace_id.to_string(),
                account_id.to_string(),
                gate_key.to_string(),
            );
            let candidate = (released_at.timestamp_millis(), id);
            match newest_unrolled.get_mut(&scope) {
                Some(current)
                    if candidate.0 > current.0
                        || (candidate.0 == current.0
                            && candidate.1.to_hex() > current.1.to_hex()) =>
                {
                    *current = candidate;
                }
                None => {
                    newest_unrolled.insert(scope, candidate);
                }
                _ => {}
            }
        }
    }

    // Validation above is deliberately read-only. Only after every legacy row
    // has passed ownership, revision, and uniqueness checks may the migration
    // change persisted state; malformed later rows therefore cannot leave a
    // partially reconciled collection.
    for row in &reconciliation_plan {
        let update = collection
            .update_one(
                doc! { "_id": row.id },
                doc! { "$set": { "released_revision": &row.expected_revision } },
                None,
            )
            .await?;
        if update.matched_count != 1 {
            return Err(AppError::External(format!(
                "evolution release migration lost revision CAS for override {}",
                row.id
            )));
        }
    }

    // Demote first so this step is safe even when rerun after the unique index exists.
    collection
        .update_many(
            doc! { "current_version": { "$ne": false } },
            doc! { "$set": { "current_version": false } },
            None,
        )
        .await?;
    for ((workspace_id, account_id, gate_key), (_, id)) in &newest_unrolled {
        let promoted = collection
            .update_one(
                doc! {
                    "_id": id,
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "gate_key": gate_key,
                    "rolled_back_at": null,
                },
                doc! { "$set": { "current_version": true } },
                None,
            )
            .await?;
        if promoted.matched_count != 1 {
            return Err(AppError::External(format!(
                "evolution release migration lost current CAS for override {id}"
            )));
        }
    }

    tracing::info!(
        migration_id = "2026_07_040_evolution_release_protocol",
        reconciled = reconciliation_plan.len(),
        current_scopes = newest_unrolled.len(),
        "reconciled evolution threshold release revisions and current pointers"
    );
    Ok(())
}

fn required_object_id(
    row: &Document,
    field: &str,
    row_id: Option<ObjectId>,
) -> AppResult<ObjectId> {
    row.get_object_id(field).map_err(|_| {
        AppError::External(format!(
            "evolution release migration found row {} without ObjectId {field}",
            row_id
                .map(|id| id.to_hex())
                .unwrap_or_else(|| "<unknown>".to_string())
        ))
    })
}

fn required_canonical_string<'a>(
    row: &'a Document,
    field: &str,
    id: ObjectId,
) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "evolution release migration found override {id} without {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "evolution release migration found override {id} with non-canonical {field}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_scope_rejects_missing_and_whitespace() {
        let id = ObjectId::new();
        assert!(required_canonical_string(&doc! {}, "workspace_id", id).is_err());
        assert!(
            required_canonical_string(&doc! { "workspace_id": " ws " }, "workspace_id", id,)
                .is_err()
        );
        assert_eq!(
            required_canonical_string(&doc! { "workspace_id": "ws" }, "workspace_id", id,).unwrap(),
            "ws"
        );
    }
}

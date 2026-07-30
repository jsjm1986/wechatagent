//! Add explicit tenant identity to chunk revisions and behavior signals.
//!
//! Both backfills derive ownership from authoritative parent rows. Missing or ambiguous
//! ownership fails startup; no default workspace/account is guessed. Index replacement runs
//! only after every row has been validated and backfilled, making retries safe.

use std::collections::HashSet;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};

use crate::db::{
    indexes::{
        behavior_signal_identity_index, behavior_signal_timeline_index,
        chunk_revision_identity_index, chunk_revision_timeline_index,
    },
    Database,
};
use crate::error::{AppError, AppResult};

pub async fn run_step(db: &Database) -> AppResult<()> {
    let collections: HashSet<String> = db
        .raw()
        .list_collection_names(None)
        .await?
        .into_iter()
        .collect();

    // Build both plans before the first write. A malformed row in either
    // collection must leave the other collection untouched so a failed
    // startup never exposes a half-materialized tenant boundary.
    let revision_plan = if collections.contains("chunk_revisions") {
        plan_chunk_revision_backfill(db).await?
    } else {
        Vec::new()
    };
    let signal_plan = if collections.contains("behavior_signals") {
        plan_behavior_signal_backfill(db).await?
    } else {
        Vec::new()
    };

    apply_backfill_plan(
        db.raw().collection::<Document>("chunk_revisions"),
        "workspace_id",
        &revision_plan,
        "revision",
    )
    .await?;
    apply_backfill_plan(
        db.raw().collection::<Document>("behavior_signals"),
        "account_id",
        &signal_plan,
        "behavior signal",
    )
    .await?;

    if collections.contains("chunk_revisions") {
        create_chunk_revision_indexes(db).await?;
        drop_indexes_with_keys(
            db.raw().collection::<Document>("chunk_revisions"),
            &[
                doc! { "chunk_id": 1, "revision_id": -1 },
                doc! { "created_at": -1 },
            ],
        )
        .await?;
    }
    if collections.contains("behavior_signals") {
        create_behavior_signal_indexes(db).await?;
        drop_indexes_with_keys(
            db.raw().collection::<Document>("behavior_signals"),
            &[
                doc! { "workspace_id": 1, "dedupe_key": 1 },
                doc! { "workspace_id": 1, "contact_wxid": 1, "observed_at": -1 },
            ],
        )
        .await?;
    }

    tracing::info!(
        migration_id = "2026_07_039_scope_revision_and_behavior_identity",
        revision_rows_backfilled = revision_plan.len(),
        behavior_signal_rows_backfilled = signal_plan.len(),
        "backfilled explicit revision and behavior-signal tenant identity"
    );
    Ok(())
}

/// Install the final scoped indexes before retiring their legacy counterparts.
/// A duplicate or malformed backfill therefore fails while the old indexes are
/// still present; there is never a startup window with neither index family.
async fn create_chunk_revision_indexes(db: &Database) -> AppResult<()> {
    let revisions = db.raw().collection::<Document>("chunk_revisions");
    revisions
        .create_index(chunk_revision_identity_index(), None)
        .await?;
    revisions
        .create_index(chunk_revision_timeline_index(), None)
        .await?;
    Ok(())
}

async fn create_behavior_signal_indexes(db: &Database) -> AppResult<()> {
    let signals = db.raw().collection::<Document>("behavior_signals");
    signals
        .create_index(behavior_signal_identity_index(), None)
        .await?;
    signals
        .create_index(behavior_signal_timeline_index(), None)
        .await?;
    Ok(())
}

async fn plan_chunk_revision_backfill(db: &Database) -> AppResult<Vec<(Bson, String)>> {
    let revisions = db.raw().collection::<Document>("chunk_revisions");
    let chunks = db
        .raw()
        .collection::<Document>("operation_knowledge_chunks");
    let mut cursor = revisions.find(Document::new(), None).await?;
    let mut plan = Vec::new();

    while let Some(row) = cursor.try_next().await? {
        let id = required_id(&row, "chunk_revisions")?;
        let chunk_id = required_canonical_string(&row, "chunk_id", &id)?;
        let chunk_oid = ObjectId::parse_str(chunk_id).map_err(|_| {
            AppError::External(format!(
                "revision tenant migration found row {id:?} with invalid chunk_id"
            ))
        })?;
        let parent = chunks
            .find_one(doc! { "_id": chunk_oid }, None)
            .await?
            .ok_or_else(|| {
                AppError::External(format!(
                    "revision tenant migration found row {id:?} without parent chunk"
                ))
            })?;
        let parent_workspace = required_canonical_string(&parent, "workspace_id", &id)?;
        if row.contains_key("workspace_id") {
            let workspace_id = required_canonical_string(&row, "workspace_id", &id)?;
            if workspace_id != parent_workspace {
                return Err(AppError::External(format!(
                    "revision tenant migration found row {id:?} whose workspace_id does not match its parent chunk"
                )));
            }
            continue;
        }
        plan.push((id, parent_workspace.to_string()));
    }
    Ok(plan)
}

async fn plan_behavior_signal_backfill(db: &Database) -> AppResult<Vec<(Bson, String)>> {
    let signals = db.raw().collection::<Document>("behavior_signals");
    let contacts = db.raw().collection::<Document>("contacts");
    let mut cursor = signals.find(Document::new(), None).await?;
    let mut plan = Vec::new();

    while let Some(row) = cursor.try_next().await? {
        let id = required_id(&row, "behavior_signals")?;
        let workspace_id = required_canonical_string(&row, "workspace_id", &id)?;
        let wxid = required_canonical_string(&row, "contact_wxid", &id)?;
        if row.contains_key("account_id") {
            required_canonical_string(&row, "account_id", &id)?;
            // This is an append-only historical fact. m029 may already have
            // removed a non-person Contact, so a materialized account identity
            // must not depend on the current Contact row still existing.
            continue;
        }

        // Legacy rows have no account_id. Infer it only when the authoritative
        // contact identity is unique inside the workspace; ambiguity is unsafe.
        let mut account_cursor = contacts
            .find(doc! { "workspace_id": workspace_id, "wxid": wxid }, None)
            .await?;
        let mut accounts = HashSet::new();
        while let Some(contact) = account_cursor.try_next().await? {
            accounts.insert(required_canonical_string(&contact, "account_id", &id)?.to_string());
        }
        if accounts.len() != 1 {
            return Err(AppError::External(format!(
                "behavior signal tenant migration found row {id:?} with {} matching accounts",
                accounts.len()
            )));
        }
        let expected = accounts.into_iter().next().expect("length checked");
        plan.push((id, expected));
    }
    Ok(plan)
}

async fn apply_backfill_plan(
    collection: mongodb::Collection<Document>,
    field: &str,
    plan: &[(Bson, String)],
    kind: &str,
) -> AppResult<()> {
    for (id, value) in plan {
        let mut missing_field = Document::new();
        missing_field.insert(field, doc! { "$exists": false });
        let mut filter = doc! { "_id": id.clone() };
        filter.extend(missing_field);
        let mut set = Document::new();
        set.insert(field, value.clone());
        let result = collection
            .update_one(filter, doc! { "$set": set }, None)
            .await?;
        require_single_match(result.matched_count, id, kind)?;
    }
    Ok(())
}

async fn drop_indexes_with_keys(
    collection: mongodb::Collection<Document>,
    legacy_keys: &[Document],
) -> AppResult<()> {
    let mut cursor = collection.list_indexes(None).await?;
    let mut names = Vec::new();
    while let Some(index) = cursor.try_next().await? {
        if legacy_keys.iter().any(|keys| index.keys == *keys) {
            if let Some(name) = index.options.and_then(|options| options.name) {
                names.push(name);
            }
        }
    }
    for name in names {
        collection.drop_index(name, None).await?;
    }
    Ok(())
}

fn required_id(row: &Document, collection: &str) -> AppResult<Bson> {
    row.get("_id").cloned().ok_or_else(|| {
        AppError::External(format!(
            "{collection} tenant migration found row without _id"
        ))
    })
}

fn required_canonical_string<'a>(row: &'a Document, field: &str, id: &Bson) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!("tenant migration found row {id:?} without {field}"))
    })?;
    canonical_value(value, field, id)
}

fn canonical_value<'a>(value: &'a str, field: &str, id: &Bson) -> AppResult<&'a str> {
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "tenant migration found row {id:?} with non-canonical {field}"
        )));
    }
    Ok(value)
}

fn require_single_match(matched: u64, id: &Bson, kind: &str) -> AppResult<()> {
    if matched == 1 {
        Ok(())
    } else {
        Err(AppError::External(format!(
            "{kind} tenant migration lost CAS for row {id:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_identity_rejects_missing_empty_and_whitespace() {
        let id = Bson::String("row-1".into());
        assert!(required_canonical_string(&doc! {}, "workspace_id", &id).is_err());
        assert!(
            required_canonical_string(&doc! { "workspace_id": "" }, "workspace_id", &id).is_err()
        );
        assert!(
            required_canonical_string(&doc! { "workspace_id": " ws " }, "workspace_id", &id)
                .is_err()
        );
        assert_eq!(
            required_canonical_string(&doc! { "workspace_id": "ws" }, "workspace_id", &id).unwrap(),
            "ws"
        );
    }
}

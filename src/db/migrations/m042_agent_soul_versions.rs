//! Reconcile legacy Agent Soul pointers before immutable-version indexes exist.
//!
//! Every row is validated before the first write. Duplicate versions or
//! malformed ownership/status fail startup with zero mutation. For a legacy
//! scope containing multiple published rows, the highest `(version, _id)` is
//! retained and the others are archived. Historical content is never deleted.

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

type Scope = (String, String);

#[derive(Debug)]
struct Row {
    id: ObjectId,
    scope: Scope,
    version: i32,
    published: bool,
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "agent_souls")
    {
        return Ok(());
    }

    let collection = db.raw().collection::<Document>("agent_souls");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut rows = Vec::new();
    let mut versions: HashMap<Scope, HashSet<i32>> = HashMap::new();
    while let Some(document) = cursor.try_next().await? {
        let row = parse_row(&document)?;
        if !versions
            .entry(row.scope.clone())
            .or_default()
            .insert(row.version)
        {
            return Err(AppError::External(format!(
                "agent soul migration found duplicate version {} for {}/{}; reconcile explicitly before startup",
                row.version, row.scope.0, row.scope.1
            )));
        }
        rows.push(row);
    }

    let mut winners: HashMap<Scope, (i32, ObjectId)> = HashMap::new();
    for row in rows.iter().filter(|row| row.published) {
        let candidate = (row.version, row.id);
        match winners.get_mut(&row.scope) {
            Some(winner)
                if candidate.0 > winner.0
                    || (candidate.0 == winner.0 && candidate.1.to_hex() > winner.1.to_hex()) =>
            {
                *winner = candidate;
            }
            None => {
                winners.insert(row.scope.clone(), candidate);
            }
            _ => {}
        }
    }

    let mut archived = 0_u64;
    for row in rows.iter().filter(|row| row.published) {
        let winner_id = winners
            .get(&row.scope)
            .map(|winner| winner.1)
            .ok_or_else(|| AppError::External("soul migration winner missing".to_string()))?;
        if row.id == winner_id {
            continue;
        }
        let result = collection
            .update_one(
                doc! {
                    "_id": row.id,
                    "workspace_id": &row.scope.0,
                    "agent_kind": &row.scope.1,
                    "version": row.version,
                    "status": "published",
                },
                doc! {
                    "$set": {
                        "status": "archived",
                        "updated_at": mongodb::bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        if result.modified_count != 1 {
            return Err(AppError::External(format!(
                "agent soul migration lost pointer CAS for {}",
                row.id
            )));
        }
        archived += 1;
    }

    tracing::info!(
        migration_id = "2026_07_042_agent_soul_versions",
        rows = rows.len(),
        published_scopes = winners.len(),
        archived,
        "reconciled immutable Agent Soul versions"
    );
    Ok(())
}

fn parse_row(document: &Document) -> AppResult<Row> {
    let id = document.get_object_id("_id").map_err(|_| {
        AppError::External("agent soul migration found row without ObjectId _id".to_string())
    })?;
    let workspace = canonical(document, "workspace_id", id)?;
    let kind = canonical(document, "agent_kind", id)?;
    let version = document.get_i32("version").map_err(|_| {
        AppError::External(format!(
            "agent soul migration found {id} without int32 version"
        ))
    })?;
    if version <= 0 {
        return Err(AppError::External(format!(
            "agent soul migration found {id} with non-positive version"
        )));
    }
    let status = document.get_str("status").map_err(|_| {
        AppError::External(format!("agent soul migration found {id} without status"))
    })?;
    if !matches!(status, "draft" | "published" | "archived") {
        return Err(AppError::External(format!(
            "agent soul migration found {id} with invalid status {status}"
        )));
    }
    Ok(Row {
        id,
        scope: (workspace.to_string(), kind.to_string()),
        version,
        published: status == "published",
    })
}

fn canonical<'a>(document: &'a Document, field: &str, id: ObjectId) -> AppResult<&'a str> {
    let value = document.get_str(field).map_err(|_| {
        AppError::External(format!("agent soul migration found {id} without {field}"))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "agent soul migration found {id} with non-canonical {field}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_rejects_invalid_scope_version_and_status() {
        let id = ObjectId::new();
        let valid = doc! {
            "_id": id,
            "workspace_id": "ws",
            "agent_kind": "user",
            "version": 1_i32,
            "status": "draft",
        };
        assert!(parse_row(&valid).is_ok());
        let mut invalid = valid.clone();
        invalid.insert("workspace_id", " ws ");
        assert!(parse_row(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid.insert("version", 0_i32);
        assert!(parse_row(&invalid).is_err());
        let mut invalid = valid;
        invalid.insert("status", "active");
        assert!(parse_row(&invalid).is_err());
    }
}

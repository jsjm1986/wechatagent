//! Reconcile prompt lifecycle metadata to one canonical current pointer.
//!
//! The whole collection is validated before the first write. Existing active
//! streams must already identify exactly one current row; this migration does
//! not guess a business winner. It archives non-current active rows and clears
//! the legacy current flag that m034 could place on a draft-only stream.

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
    status: String,
    current: bool,
}

#[derive(Default)]
struct ScopeState {
    active: usize,
    current: usize,
    current_active: usize,
}

fn validate_scope_state(scope: &Scope, state: &ScopeState) -> AppResult<()> {
    if state.active > 0 && (state.current != 1 || state.current_active != 1) {
        return Err(AppError::External(format!(
            "prompt migration requires one active current for {}/{}; active={} current={} current_active={}",
            scope.0, scope.1, state.active, state.current, state.current_active
        )));
    }
    if state.active == 0 && state.current > 1 {
        return Err(AppError::External(format!(
            "prompt migration found multiple draft-only currents for {}/{}",
            scope.0, scope.1
        )));
    }
    Ok(())
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    if !db
        .raw()
        .list_collection_names(None)
        .await?
        .iter()
        .any(|name| name == "prompt_templates")
    {
        return Ok(());
    }

    let collection = db.raw().collection::<Document>("prompt_templates");
    let mut cursor = collection.find(Document::new(), None).await?;
    let mut rows = Vec::new();
    let mut versions: HashMap<Scope, HashSet<i32>> = HashMap::new();
    let mut scopes: HashMap<Scope, ScopeState> = HashMap::new();
    while let Some(document) = cursor.try_next().await? {
        let row = parse_row(&document)?;
        if !versions
            .entry(row.scope.clone())
            .or_default()
            .insert(row.version)
        {
            return Err(AppError::External(format!(
                "prompt migration found duplicate version {} for {}/{}",
                row.version, row.scope.0, row.scope.1
            )));
        }
        let state = scopes.entry(row.scope.clone()).or_default();
        state.active += usize::from(row.status == "active");
        state.current += usize::from(row.current);
        state.current_active += usize::from(row.current && row.status == "active");
        rows.push(row);
    }

    for (scope, state) in &scopes {
        validate_scope_state(scope, state)?;
    }

    let now = mongodb::bson::DateTime::now();
    let mut archived = 0_u64;
    let mut cleared_draft_currents = 0_u64;
    for row in rows {
        let state = scopes
            .get(&row.scope)
            .ok_or_else(|| AppError::External("prompt migration scope missing".to_string()))?;
        if row.status == "active" && !row.current {
            let result = collection
                .update_one(
                    doc! {
                        "_id": row.id,
                        "workspace_id": &row.scope.0,
                        "prompt_key": &row.scope.1,
                        "version": row.version,
                        "status": "active",
                        "current_version": false,
                    },
                    doc! { "$set": { "status": "archived", "updated_at": now } },
                    None,
                )
                .await?;
            if result.modified_count != 1 {
                return Err(AppError::External(format!(
                    "prompt migration lost archive CAS for {}",
                    row.id
                )));
            }
            archived += 1;
        } else if state.active == 0 && row.current {
            let result = collection
                .update_one(
                    doc! {
                        "_id": row.id,
                        "workspace_id": &row.scope.0,
                        "prompt_key": &row.scope.1,
                        "version": row.version,
                        "status": &row.status,
                        "current_version": true,
                    },
                    doc! { "$set": { "current_version": false, "updated_at": now } },
                    None,
                )
                .await?;
            if result.modified_count != 1 {
                return Err(AppError::External(format!(
                    "prompt migration lost draft pointer CAS for {}",
                    row.id
                )));
            }
            cleared_draft_currents += 1;
        }
    }

    tracing::info!(
        migration_id = "2026_07_043_prompt_single_current",
        scopes = scopes.len(),
        archived,
        cleared_draft_currents,
        "reconciled prompt current pointer"
    );
    Ok(())
}

fn parse_row(document: &Document) -> AppResult<Row> {
    let id = document.get_object_id("_id").map_err(|_| {
        AppError::External("prompt migration found row without ObjectId _id".to_string())
    })?;
    let workspace = canonical(document, "workspace_id", id)?;
    let prompt_key = canonical(document, "prompt_key", id)?;
    let version = document.get_i32("version").map_err(|_| {
        AppError::External(format!("prompt migration found {id} without int32 version"))
    })?;
    if version <= 0 {
        return Err(AppError::External(format!(
            "prompt migration found {id} with non-positive version"
        )));
    }
    let status = document
        .get_str("status")
        .map_err(|_| AppError::External(format!("prompt migration found {id} without status")))?;
    if !matches!(status, "draft" | "active" | "archived") {
        return Err(AppError::External(format!(
            "prompt migration found {id} with invalid status {status}"
        )));
    }
    let current = document.get_bool("current_version").map_err(|_| {
        AppError::External(format!(
            "prompt migration found {id} without bool current_version"
        ))
    })?;
    Ok(Row {
        id,
        scope: (workspace.to_string(), prompt_key.to_string()),
        version,
        status: status.to_string(),
        current,
    })
}

fn canonical<'a>(document: &'a Document, field: &str, id: ObjectId) -> AppResult<&'a str> {
    let value = document
        .get_str(field)
        .map_err(|_| AppError::External(format!("prompt migration found {id} without {field}")))?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "prompt migration found {id} with non-canonical {field}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_requires_canonical_scope_closed_status_and_bool_pointer() {
        let id = ObjectId::new();
        let valid = doc! {
            "_id": id,
            "workspace_id": "ws",
            "prompt_key": "user.reply.task",
            "version": 1_i32,
            "status": "active",
            "current_version": true,
        };
        assert!(parse_row(&valid).is_ok());
        let mut invalid = valid.clone();
        invalid.insert("prompt_key", " user.reply.task ");
        assert!(parse_row(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid.insert("status", "published");
        assert!(parse_row(&invalid).is_err());
        let mut invalid = valid;
        invalid.remove("current_version");
        assert!(parse_row(&invalid).is_err());
    }

    #[test]
    fn scope_validation_rejects_split_or_duplicate_pointers() {
        let scope = ("ws".to_string(), "user.reply.task".to_string());
        assert!(validate_scope_state(
            &scope,
            &ScopeState {
                active: 2,
                current: 1,
                current_active: 1,
            },
        )
        .is_ok());
        assert!(validate_scope_state(
            &scope,
            &ScopeState {
                active: 1,
                current: 1,
                current_active: 0,
            },
        )
        .unwrap_err()
        .to_string()
        .contains("requires one active current"));
        assert!(validate_scope_state(
            &scope,
            &ScopeState {
                active: 0,
                current: 2,
                current_active: 0,
            },
        )
        .unwrap_err()
        .to_string()
        .contains("multiple draft-only currents"));
    }
}

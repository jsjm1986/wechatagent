//! Immutable version lifecycle for Agent Soul documents.
//!
//! Content rows are append-only. Only lifecycle metadata (`status`, publish
//! audit fields) may change, and switching the published pointer is a Mongo
//! transaction guarded by exact status/version predicates.

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime},
    error::{ErrorKind, WriteFailure},
    options::{FindOneOptions, FindOptions, TransactionOptions},
};

use crate::{
    db::Database,
    error::{AppError, AppResult},
    models::AgentSoul,
};

const VERSION_INSERT_RETRIES: usize = 8;

#[derive(Debug, Clone)]
pub struct NewSoulVersion<'a> {
    pub agent_kind: &'a str,
    pub name: &'a str,
    pub content: &'a str,
    pub seeded_by: &'a str,
    pub previous_version: Option<i32>,
}

fn validate_new_version(input: &NewSoulVersion<'_>) -> AppResult<()> {
    if input.agent_kind.trim().is_empty()
        || input.agent_kind.trim() != input.agent_kind
        || input.name.trim().is_empty()
        || input.content.trim().is_empty()
        || input.seeded_by.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "agentKind, name and content are required; agentKind must be canonical".to_string(),
        ));
    }
    Ok(())
}

fn build_version(
    workspace_id: &str,
    input: &NewSoulVersion<'_>,
    version: i32,
    status: &str,
) -> AgentSoul {
    let now = DateTime::now();
    AgentSoul {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        agent_kind: input.agent_kind.to_string(),
        name: input.name.to_string(),
        content: input.content.to_string(),
        status: status.to_string(),
        version,
        created_at: now,
        updated_at: now,
        seeded_by: Some(input.seeded_by.to_string()),
        previous_version: input.previous_version,
        published_at: (status == "published").then_some(now),
        published_by: (status == "published").then(|| input.seeded_by.to_string()),
    }
}

/// Append one immutable content version. The unique version index arbitrates
/// concurrent max+1 allocation; losers re-read and retry without overwriting.
pub async fn append_version(
    db: &Database,
    workspace_id: &str,
    input: NewSoulVersion<'_>,
) -> AppResult<AgentSoul> {
    validate_new_version(&input)?;
    if workspace_id.trim().is_empty() || workspace_id.trim() != workspace_id {
        return Err(AppError::BadRequest("invalid workspace".to_string()));
    }

    for _ in 0..VERSION_INSERT_RETRIES {
        let latest = db
            .agent_souls()
            .find_one(
                doc! {
                    "workspace_id": workspace_id,
                    "agent_kind": input.agent_kind,
                },
                FindOneOptions::builder()
                    .sort(doc! { "version": -1 })
                    .build(),
            )
            .await?;
        let version = latest.map(|row| row.version + 1).unwrap_or(1);
        let row = build_version(workspace_id, &input, version, "draft");
        match db.agent_souls().insert_one(&row, None).await {
            Ok(_) => return Ok(row),
            Err(error) if is_duplicate_key_error(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::Conflict(
        "soul_version_allocation_conflict".to_string(),
    ))
}

/// Seed version 1 as published only when the whole `(workspace, kind)` stream
/// is absent. A racing writer is re-read; an existing draft-only stream is an
/// invariant error and is never silently repaired.
pub async fn ensure_initial_published(
    db: &Database,
    workspace_id: &str,
    input: NewSoulVersion<'_>,
) -> AppResult<(AgentSoul, bool)> {
    validate_new_version(&input)?;
    if workspace_id.trim().is_empty()
        || workspace_id.trim() != workspace_id
        || input.previous_version.is_some()
    {
        return Err(AppError::BadRequest(
            "invalid initial soul version".to_string(),
        ));
    }
    let filter = doc! {
        "workspace_id": workspace_id,
        "agent_kind": input.agent_kind,
    };
    if db
        .agent_souls()
        .find_one(filter.clone(), None)
        .await?
        .is_some()
    {
        return Ok((
            load_unique_published(db, workspace_id, input.agent_kind).await?,
            false,
        ));
    }

    let row = build_version(workspace_id, &input, 1, "published");
    match db.agent_souls().insert_one(&row, None).await {
        Ok(_) => Ok((row, true)),
        Err(error) if is_duplicate_key_error(&error) => Ok((
            load_unique_published(db, workspace_id, input.agent_kind).await?,
            false,
        )),
        Err(error) => Err(error.into()),
    }
}

/// Seed version 1 as a draft placeholder only when the whole
/// `(workspace, kind)` stream is absent. Existing operator-owned streams are
/// preserved unchanged; a concurrent initializer is re-read after the unique
/// version index arbitrates the race.
pub async fn ensure_initial_draft(
    db: &Database,
    workspace_id: &str,
    input: NewSoulVersion<'_>,
) -> AppResult<(AgentSoul, bool)> {
    validate_new_version(&input)?;
    if workspace_id.trim().is_empty()
        || workspace_id.trim() != workspace_id
        || input.previous_version.is_some()
    {
        return Err(AppError::BadRequest(
            "invalid initial soul version".to_string(),
        ));
    }
    let filter = doc! {
        "workspace_id": workspace_id,
        "agent_kind": input.agent_kind,
    };
    let collection = db.agent_souls();
    let find_latest = || {
        collection.find_one(
            filter.clone(),
            FindOneOptions::builder()
                .sort(doc! { "version": -1 })
                .build(),
        )
    };
    if let Some(existing) = find_latest().await? {
        return Ok((existing, false));
    }

    let row = build_version(workspace_id, &input, 1, "draft");
    match collection.insert_one(&row, None).await {
        Ok(_) => Ok((row, true)),
        Err(error) if is_duplicate_key_error(&error) => find_latest()
            .await?
            .map(|existing| (existing, false))
            .ok_or_else(|| AppError::Conflict("soul_version_allocation_conflict".to_string())),
        Err(error) => Err(error.into()),
    }
}

/// Create a successor draft from a selected immutable version. Changing the
/// kind would splice two independent version streams and is rejected.
pub async fn append_edited_draft(
    db: &Database,
    workspace_id: &str,
    source_id: ObjectId,
    agent_kind: &str,
    name: &str,
    content: &str,
    actor: &str,
) -> AppResult<AgentSoul> {
    let source = db
        .agent_souls()
        .find_one(
            doc! { "_id": source_id, "workspace_id": workspace_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("agent soul not found".to_string()))?;
    if source.agent_kind != agent_kind {
        return Err(AppError::Conflict(
            "soul_agent_kind_is_immutable".to_string(),
        ));
    }
    append_version(
        db,
        workspace_id,
        NewSoulVersion {
            agent_kind,
            name,
            content,
            seeded_by: actor,
            previous_version: Some(source.version),
        },
    )
    .await
}

/// Atomically archive the previous published pointer and publish `target_id`.
/// Historical rows are retained. A concurrent publisher can only win or get a
/// conflict; it cannot leave the kind with zero published rows.
pub async fn publish_version(
    db: &Database,
    workspace_id: &str,
    target_id: ObjectId,
    actor: &str,
) -> AppResult<AgentSoul> {
    if workspace_id.trim().is_empty()
        || workspace_id.trim() != workspace_id
        || actor.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "invalid soul publish request".to_string(),
        ));
    }
    let mut session = db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;

    let transaction_result: AppResult<AgentSoul> = async {
        let target = db
            .agent_souls()
            .find_one_with_session(
                doc! { "_id": target_id, "workspace_id": workspace_id },
                None,
                &mut session,
            )
            .await?
            .ok_or_else(|| AppError::NotFound("agent soul not found".to_string()))?;
        if !matches!(target.status.as_str(), "draft" | "archived" | "published") {
            return Err(AppError::Conflict(
                "soul_status_not_publishable".to_string(),
            ));
        }

        let current = db
            .agent_souls()
            .find_one_with_session(
                doc! {
                    "workspace_id": workspace_id,
                    "agent_kind": &target.agent_kind,
                    "status": "published",
                },
                FindOneOptions::builder()
                    .sort(doc! { "version": -1 })
                    .build(),
                &mut session,
            )
            .await?;
        if let Some(current) = current.as_ref() {
            let current_id = current
                .id
                .ok_or_else(|| AppError::External("published soul missing _id".to_string()))?;
            let duplicate = db
                .agent_souls()
                .find_one_with_session(
                    doc! {
                        "workspace_id": workspace_id,
                        "agent_kind": &target.agent_kind,
                        "status": "published",
                        "_id": { "$ne": current_id },
                    },
                    None,
                    &mut session,
                )
                .await?;
            if duplicate.is_some() {
                return Err(AppError::Conflict("multiple_published_souls".to_string()));
            }
        }
        if current.as_ref().and_then(|row| row.id) == Some(target_id) {
            return Ok(target);
        }

        let now = DateTime::now();
        if let Some(current) = current {
            let current_id = current
                .id
                .ok_or_else(|| AppError::External("published soul missing _id".to_string()))?;
            let archived = db
                .agent_souls()
                .update_one_with_session(
                    doc! {
                        "_id": current_id,
                        "workspace_id": workspace_id,
                        "agent_kind": &target.agent_kind,
                        "version": current.version,
                        "status": "published",
                    },
                    doc! { "$set": { "status": "archived", "updated_at": now } },
                    None,
                    &mut session,
                )
                .await?;
            if archived.modified_count != 1 {
                return Err(AppError::Conflict(
                    "soul_publish_pointer_changed".to_string(),
                ));
            }
        }

        let promoted = db
            .agent_souls()
            .update_one_with_session(
                doc! {
                    "_id": target_id,
                    "workspace_id": workspace_id,
                    "agent_kind": &target.agent_kind,
                    "version": target.version,
                    "status": &target.status,
                },
                doc! {
                    "$set": {
                        "status": "published",
                        "updated_at": now,
                        "published_at": now,
                        "published_by": actor,
                    }
                },
                None,
                &mut session,
            )
            .await?;
        if promoted.modified_count != 1 {
            return Err(AppError::Conflict(
                "soul_publish_target_changed".to_string(),
            ));
        }
        let mut published = target;
        published.status = "published".to_string();
        published.updated_at = now;
        published.published_at = Some(now);
        published.published_by = Some(actor.to_string());
        Ok(published)
    }
    .await;

    let published = match transaction_result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(match error {
                AppError::Db(db_error) => {
                    tracing::warn!(error = %db_error, "soul publish transaction conflicted");
                    AppError::Conflict("soul_publish_conflict".to_string())
                }
                other => other,
            });
        }
    };
    loop {
        match session.commit_transaction().await {
            Ok(()) => break,
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => {
                let _ = session.abort_transaction().await;
                tracing::warn!(error = %error, "soul publish commit failed");
                return Err(AppError::Conflict("soul_publish_conflict".to_string()));
            }
        }
    }
    Ok(published)
}

/// Runtime reads fail closed unless exactly one published pointer exists.
pub async fn load_unique_published(
    db: &Database,
    workspace_id: &str,
    agent_kind: &str,
) -> AppResult<AgentSoul> {
    let mut cursor = db
        .agent_souls()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "agent_kind": agent_kind,
                "status": "published",
            },
            FindOptions::builder()
                .sort(doc! { "version": -1 })
                .limit(2)
                .build(),
        )
        .await?;
    let mut rows = Vec::with_capacity(2);
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }
    match rows.len() {
        1 => Ok(rows.remove(0)),
        0 => {
            tracing::error!(workspace_id, agent_kind, "published soul missing");
            Err(AppError::External("published_soul_missing".to_string()))
        }
        _ => {
            tracing::error!(workspace_id, agent_kind, "multiple published souls found");
            Err(AppError::External("multiple_published_souls".to_string()))
        }
    }
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    match &*error.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            matches!(write_error.code, 11000 | 11001)
        }
        ErrorKind::BulkWrite(bulk) => bulk.write_errors.as_ref().is_some_and(|errors| {
            errors
                .iter()
                .any(|error| matches!(error.code, 11000 | 11001))
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_versions_require_canonical_kind_and_closed_status() {
        let base = NewSoulVersion {
            agent_kind: "user",
            name: "name",
            content: "content",
            seeded_by: "admin",
            previous_version: None,
        };
        assert!(validate_new_version(&base).is_ok());
        assert!(validate_new_version(&NewSoulVersion {
            agent_kind: " user ",
            ..base.clone()
        })
        .is_err());
        assert!(validate_new_version(&NewSoulVersion {
            seeded_by: "",
            ..base
        })
        .is_err());
    }
}

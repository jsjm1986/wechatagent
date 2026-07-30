//! Immutable prompt-template version lifecycle.

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime},
    error::{ErrorKind, WriteFailure},
    options::{FindOneOptions, FindOptions, TransactionOptions},
};

use crate::{
    db::Database,
    error::{AppError, AppResult},
    models::PromptTemplate,
};

const VERSION_INSERT_RETRIES: usize = 8;

#[derive(Debug, Clone)]
pub struct NewPromptTemplateVersion<'a> {
    pub prompt_key: &'a str,
    pub agent_kind: &'a str,
    pub layer: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub content: &'a str,
    pub prompt_pack_version: &'a str,
    pub actor: &'a str,
    pub seeded_by: &'a str,
    pub locale: Option<&'a str>,
    pub previous_version: Option<i32>,
    pub source_proposal_id: Option<ObjectId>,
}

fn validate_new_version(input: &NewPromptTemplateVersion<'_>) -> AppResult<()> {
    let canonical = |value: &str| !value.is_empty() && value.trim() == value;
    if !canonical(input.prompt_key)
        || !canonical(input.agent_kind)
        || !canonical(input.layer)
        || input.title.trim().is_empty()
        || input.content.trim().is_empty()
        || !canonical(input.prompt_pack_version)
        || !canonical(input.actor)
        || !canonical(input.seeded_by)
    {
        return Err(AppError::BadRequest(
            "invalid prompt template version".to_string(),
        ));
    }
    Ok(())
}

fn build_draft(
    workspace_id: &str,
    input: &NewPromptTemplateVersion<'_>,
    version: i32,
) -> PromptTemplate {
    let now = DateTime::now();
    PromptTemplate {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        prompt_key: input.prompt_key.to_string(),
        agent_kind: input.agent_kind.to_string(),
        layer: input.layer.to_string(),
        title: input.title.to_string(),
        description: input.description.map(str::to_string),
        content: input.content.to_string(),
        status: "draft".to_string(),
        version,
        prompt_pack_version: input.prompt_pack_version.to_string(),
        created_by: input.actor.to_string(),
        created_at: now,
        updated_at: now,
        current_version: false,
        previous_version: input.previous_version,
        seeded_by: Some(input.seeded_by.to_string()),
        locale: input.locale.map(str::to_string),
        source_proposal_id: input.source_proposal_id,
    }
}

pub async fn append_version(
    db: &Database,
    workspace_id: &str,
    input: NewPromptTemplateVersion<'_>,
) -> AppResult<PromptTemplate> {
    validate_new_version(&input)?;
    if workspace_id.is_empty() || workspace_id.trim() != workspace_id {
        return Err(AppError::BadRequest("invalid workspace".to_string()));
    }
    for _ in 0..VERSION_INSERT_RETRIES {
        let latest = db
            .prompt_templates()
            .find_one(
                doc! { "workspace_id": workspace_id, "prompt_key": input.prompt_key },
                FindOneOptions::builder()
                    .sort(doc! { "version": -1 })
                    .build(),
            )
            .await?;
        let version = latest.map(|row| row.version + 1).unwrap_or(1);
        let row = build_draft(workspace_id, &input, version);
        match db.prompt_templates().insert_one(&row, None).await {
            Ok(_) => return Ok(row),
            Err(error) if is_duplicate_key_error(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::Conflict(
        "prompt_version_allocation_conflict".to_string(),
    ))
}

pub async fn append_edited_draft(
    db: &Database,
    workspace_id: &str,
    source_id: ObjectId,
    input: NewPromptTemplateVersion<'_>,
) -> AppResult<PromptTemplate> {
    let source = db
        .prompt_templates()
        .find_one(
            doc! { "_id": source_id, "workspace_id": workspace_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("prompt template not found".to_string()))?;
    if source.prompt_key != input.prompt_key {
        return Err(AppError::Conflict("prompt_key_is_immutable".to_string()));
    }
    let mut successor = input;
    successor.previous_version = Some(source.version);
    append_version(db, workspace_id, successor).await
}

pub async fn load_unique_current(
    db: &Database,
    workspace_id: &str,
    prompt_key: &str,
) -> AppResult<Option<PromptTemplate>> {
    let current = load_current_for_publish(db, workspace_id, prompt_key).await?;
    if current.is_none() {
        let any_version = db
            .prompt_templates()
            .find_one(
                doc! {
                    "workspace_id": workspace_id,
                    "prompt_key": prompt_key,
                },
                None,
            )
            .await?;
        if any_version.is_some() {
            return Err(AppError::External("current_prompt_missing".to_string()));
        }
    }
    Ok(current)
}

/// Publication may establish the first pointer for a draft-only stream. It
/// still rejects a malformed current pointer or duplicate currents.
pub async fn load_current_for_publish(
    db: &Database,
    workspace_id: &str,
    prompt_key: &str,
) -> AppResult<Option<PromptTemplate>> {
    let mut cursor = db
        .prompt_templates()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "prompt_key": prompt_key,
                "current_version": true,
            },
            FindOptions::builder().limit(2).build(),
        )
        .await?;
    let mut rows = Vec::with_capacity(2);
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }
    match rows.len() {
        0 => Ok(None),
        1 if rows[0].status == "active" => Ok(rows.pop()),
        1 => Err(AppError::External("current_prompt_not_active".to_string())),
        _ => Err(AppError::External("multiple_current_prompts".to_string())),
    }
}

fn validate_publish_pointer_state(
    current_count: usize,
    current_is_active: bool,
    has_non_current_active: bool,
) -> AppResult<()> {
    if current_count > 1 {
        return Err(AppError::Conflict("multiple_current_prompts".to_string()));
    }
    if current_count == 1 && !current_is_active {
        return Err(AppError::Conflict("current_prompt_not_active".to_string()));
    }
    if has_non_current_active {
        return Err(AppError::Conflict("non_current_active_prompt".to_string()));
    }
    Ok(())
}

/// Atomically archive the previous current pointer and publish `target_id`.
/// Content rows are retained; only lifecycle metadata changes.
pub async fn publish_version(
    db: &Database,
    workspace_id: &str,
    target_id: ObjectId,
    actor: &str,
) -> AppResult<PromptTemplate> {
    if workspace_id.is_empty() || workspace_id.trim() != workspace_id || actor.trim().is_empty() {
        return Err(AppError::BadRequest(
            "invalid prompt publish request".to_string(),
        ));
    }
    let mut session = db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;

    let transaction_result: AppResult<PromptTemplate> = async {
        let target = db
            .prompt_templates()
            .find_one_with_session(
                doc! { "_id": target_id, "workspace_id": workspace_id },
                None,
                &mut session,
            )
            .await?
            .ok_or_else(|| AppError::NotFound("prompt template not found".to_string()))?;
        if !matches!(target.status.as_str(), "draft" | "archived" | "active") {
            return Err(AppError::Conflict(
                "prompt_status_not_publishable".to_string(),
            ));
        }

        let mut currents = db
            .prompt_templates()
            .find_with_session(
                doc! {
                    "workspace_id": workspace_id,
                    "prompt_key": &target.prompt_key,
                    "current_version": true,
                },
                FindOptions::builder()
                    .sort(doc! { "version": -1 })
                    .limit(2)
                    .build(),
                &mut session,
            )
            .await?;
        let mut current_rows = Vec::with_capacity(2);
        while let Some(row) = currents.next(&mut session).await.transpose()? {
            current_rows.push(row);
        }
        let has_non_current_active = db
            .prompt_templates()
            .find_one_with_session(
                doc! {
                    "workspace_id": workspace_id,
                    "prompt_key": &target.prompt_key,
                    "status": "active",
                    "current_version": false,
                },
                None,
                &mut session,
            )
            .await?
            .is_some();
        validate_publish_pointer_state(
            current_rows.len(),
            current_rows
                .first()
                .is_none_or(|row| row.status == "active"),
            has_non_current_active,
        )?;
        if current_rows.first().and_then(|row| row.id) == Some(target_id) {
            return Ok(target);
        }

        let now = DateTime::now();
        if let Some(current) = current_rows.pop() {
            let current_id = current
                .id
                .ok_or_else(|| AppError::External("current prompt missing _id".to_string()))?;
            let archived = db
                .prompt_templates()
                .update_one_with_session(
                    doc! {
                        "_id": current_id,
                        "workspace_id": workspace_id,
                        "prompt_key": &target.prompt_key,
                        "version": current.version,
                        "current_version": true,
                    },
                    doc! { "$set": {
                        "current_version": false,
                        "status": "archived",
                        "updated_at": now,
                    } },
                    None,
                    &mut session,
                )
                .await?;
            if archived.modified_count != 1 {
                return Err(AppError::Conflict(
                    "prompt_publish_pointer_changed".to_string(),
                ));
            }
        }

        let promoted = db
            .prompt_templates()
            .update_one_with_session(
                doc! {
                    "_id": target_id,
                    "workspace_id": workspace_id,
                    "prompt_key": &target.prompt_key,
                    "version": target.version,
                    "status": &target.status,
                    "current_version": false,
                },
                doc! { "$set": {
                    "current_version": true,
                    "status": "active",
                    "updated_at": now,
                } },
                None,
                &mut session,
            )
            .await?;
        if promoted.modified_count != 1 {
            return Err(AppError::Conflict(
                "prompt_publish_target_changed".to_string(),
            ));
        }
        let mut published = target;
        published.current_version = true;
        published.status = "active".to_string();
        published.updated_at = now;
        Ok(published)
    }
    .await;

    let published = match transaction_result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(match error {
                AppError::Db(db_error) => {
                    tracing::warn!(error = %db_error, "prompt publish transaction conflicted");
                    AppError::Conflict("prompt_publish_conflict".to_string())
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
                tracing::warn!(error = %error, "prompt publish commit failed");
                return Err(AppError::Conflict("prompt_publish_conflict".to_string()));
            }
        }
    }
    Ok(published)
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
    fn new_versions_require_canonical_stream_identity() {
        let base = NewPromptTemplateVersion {
            prompt_key: "user.reply.task",
            agent_kind: "user",
            layer: "task",
            title: "title",
            description: None,
            content: "content",
            prompt_pack_version: "custom",
            actor: "admin",
            seeded_by: "manual",
            locale: Some("zh-CN"),
            previous_version: None,
            source_proposal_id: None,
        };
        assert!(validate_new_version(&base).is_ok());
        assert!(validate_new_version(&NewPromptTemplateVersion {
            prompt_key: " user.reply.task ",
            ..base.clone()
        })
        .is_err());
        assert!(validate_new_version(&NewPromptTemplateVersion { actor: "", ..base }).is_err());
    }

    #[test]
    fn publish_pointer_state_is_closed_and_single() {
        assert!(validate_publish_pointer_state(0, true, false).is_ok());
        assert!(validate_publish_pointer_state(1, true, false).is_ok());
        assert_eq!(
            validate_publish_pointer_state(2, true, false)
                .unwrap_err()
                .to_string(),
            "multiple_current_prompts"
        );
        assert_eq!(
            validate_publish_pointer_state(1, false, false)
                .unwrap_err()
                .to_string(),
            "current_prompt_not_active"
        );
        assert_eq!(
            validate_publish_pointer_state(1, true, true)
                .unwrap_err()
                .to_string(),
            "non_current_active_prompt"
        );
    }
}

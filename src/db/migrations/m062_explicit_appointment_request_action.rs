//! Materialize `appointment_request` in every operation-state policy allowlist.
//!
//! The action is a reactive, independently authorized durable write.  Existing policy rows
//! predate it, so this migration makes their effective behavior explicit without overriding an
//! operator's explicit prohibition.  The step is validation-before-write and CAS-protected so a
//! concurrent policy edit is never silently clobbered.

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson};

use crate::db::Database;
use crate::error::{AppError, AppResult};

pub const KNOWN_STATE_ACTIONS: &[&str] = &[
    "reply",
    "acknowledgement",
    "silent",
    "follow_up",
    "cooldown",
    "appointment_request",
];

fn explicit_allowed(allowed: &[String], forbidden: &[String]) -> Vec<String> {
    let mut result = if allowed.is_empty() {
        KNOWN_STATE_ACTIONS
            .iter()
            .filter(|action| !forbidden.iter().any(|item| item == **action))
            .map(|action| (*action).to_string())
            .collect::<Vec<_>>()
    } else {
        allowed.to_vec()
    };
    if !forbidden.iter().any(|item| item == "appointment_request")
        && !result.iter().any(|item| item == "appointment_request")
    {
        result.push("appointment_request".to_string());
    }
    result.retain(|action| !forbidden.iter().any(|item| item == action));
    result.sort_by_key(|action| {
        KNOWN_STATE_ACTIONS
            .iter()
            .position(|known| known == action)
            .unwrap_or(usize::MAX)
    });
    result.dedup();
    result
}

fn string_array(row: &mongodb::bson::Document, field: &str) -> AppResult<Vec<String>> {
    match row.get(field) {
        None | Some(Bson::Null) => Ok(Vec::new()),
        Some(Bson::Array(values)) => values
            .iter()
            .map(|value| {
                value.as_str().map(ToString::to_string).ok_or_else(|| {
                    AppError::Conflict(format!("state policy {field} contains a non-string action"))
                })
            })
            .collect(),
        Some(_) => Err(AppError::Conflict(format!(
            "state policy {field} is not an array"
        ))),
    }
}

fn field_shape_cas(row: &mongodb::bson::Document, field: &str) -> mongodb::bson::Document {
    match row.get(field) {
        None => doc! { field: { "$exists": false } },
        Some(Bson::Null) => doc! { field: { "$type": 10_i32 } },
        Some(value) => doc! { field: value.clone() },
    }
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    let collection = db
        .operation_state_policies()
        .clone_with_type::<mongodb::bson::Document>();
    let mut cursor = collection.find(doc! {}, None).await?;
    let mut changed = 0_u64;
    while let Some(row) = cursor.try_next().await? {
        let id = row
            .get("_id")
            .cloned()
            .ok_or_else(|| AppError::Conflict("state policy without _id".to_string()))?;
        let allowed = string_array(&row, "allowed")?;
        let forbidden = string_array(&row, "forbidden")?;
        let explicit = explicit_allowed(&allowed, &forbidden);
        if explicit == allowed && matches!(row.get("allowed"), Some(Bson::Array(_))) {
            continue;
        }

        let mut filter = doc! { "_id": id.clone() };
        filter.extend(field_shape_cas(&row, "allowed"));
        filter.extend(field_shape_cas(&row, "forbidden"));
        let result = collection
            .update_one(
                filter,
                doc! { "$set": { "allowed": explicit.clone() } },
                None,
            )
            .await?;
        if result.matched_count != 1 {
            let current = collection
                .find_one(doc! { "_id": id }, None)
                .await?
                .ok_or_else(|| {
                    AppError::Conflict(
                        "state policy disappeared during appointment action migration".to_string(),
                    )
                })?;
            if string_array(&current, "allowed")? != explicit
                || !matches!(current.get("allowed"), Some(Bson::Array(_)))
            {
                return Err(AppError::Conflict(
                    "state policy changed during appointment action migration".to_string(),
                ));
            }
            continue;
        }
        changed += result.modified_count;
    }
    tracing::info!(
        changed,
        "materialized explicit appointment request state action"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_appointment_request_without_overriding_forbidden() {
        assert_eq!(
            explicit_allowed(&["reply".to_string()], &[]),
            vec!["reply".to_string(), "appointment_request".to_string()]
        );
        assert_eq!(
            explicit_allowed(&[], &["appointment_request".to_string()]),
            vec![
                "reply".to_string(),
                "acknowledgement".to_string(),
                "silent".to_string(),
                "follow_up".to_string(),
                "cooldown".to_string(),
            ]
        );
    }

    #[test]
    fn preserves_existing_custom_actions_and_deduplicates() {
        assert_eq!(
            explicit_allowed(
                &[
                    "follow_up".to_string(),
                    "appointment_request".to_string(),
                    "follow_up".to_string(),
                ],
                &[]
            ),
            vec!["follow_up".to_string(), "appointment_request".to_string()]
        );
    }
}

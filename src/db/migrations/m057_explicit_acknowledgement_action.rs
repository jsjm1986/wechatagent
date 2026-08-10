//! Make `acknowledgement` an explicit operation-state action.
//!
//! Older policy rows predate this action. Runtime historically granted it through a code-level
//! exception, which made `allowed` an incomplete policy description. This migration preserves the
//! old effective behavior while materializing a complete allowlist for every historical version,
//! so rollout/rollback and the frontend all observe the same policy.

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
    if !forbidden.iter().any(|item| item == "acknowledgement")
        && !result.iter().any(|item| item == "acknowledgement")
    {
        result.push("acknowledgement".to_string());
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
        // `explicit` is derived from both fields. CAS both BSON shapes so a concurrent policy
        // edit cannot make us persist an allowlist computed from a stale forbidden set.
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
            // Concurrent startup may have applied the exact same migration after our read.
            // Re-read and accept only the desired terminal shape; every other change remains a
            // fail-closed conflict rather than being overwritten.
            let current = collection
                .find_one(doc! { "_id": id }, None)
                .await?
                .ok_or_else(|| {
                    AppError::Conflict(
                        "state policy disappeared during acknowledgement migration".to_string(),
                    )
                })?;
            if string_array(&current, "allowed")? != explicit
                || !matches!(current.get("allowed"), Some(Bson::Array(_)))
            {
                return Err(AppError::Conflict(
                    "state policy changed during acknowledgement migration".to_string(),
                ));
            }
            continue;
        }
        changed += result.modified_count;
    }
    tracing::info!(
        changed,
        "materialized explicit acknowledgement state action"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_expands_without_overriding_forbidden() {
        assert_eq!(
            explicit_allowed(&[], &["reply".to_string()]),
            vec![
                "acknowledgement".to_string(),
                "silent".to_string(),
                "follow_up".to_string(),
                "cooldown".to_string(),
            ]
        );
    }

    #[test]
    fn cas_preserves_missing_null_and_array_shapes_for_every_input() {
        assert_eq!(
            field_shape_cas(&doc! {}, "allowed"),
            doc! { "allowed": { "$exists": false } }
        );
        assert_eq!(
            field_shape_cas(&doc! { "allowed": null }, "allowed"),
            doc! { "allowed": { "$type": 10_i32 } }
        );
        assert_eq!(
            field_shape_cas(&doc! { "forbidden": ["reply"] }, "forbidden"),
            doc! { "forbidden": ["reply"] }
        );
    }

    #[test]
    fn malformed_actions_fail_closed() {
        assert!(string_array(&doc! { "allowed": ["reply", 1] }, "allowed").is_err());
        assert!(string_array(&doc! { "allowed": "reply" }, "allowed").is_err());
    }

    #[test]
    fn existing_allowlist_gains_ack_unless_explicitly_forbidden() {
        assert_eq!(
            explicit_allowed(&["follow_up".to_string()], &[]),
            vec!["acknowledgement".to_string(), "follow_up".to_string()]
        );
        assert_eq!(
            explicit_allowed(&["follow_up".to_string()], &["acknowledgement".to_string()]),
            vec!["follow_up".to_string()]
        );
    }
}

//! Deterministic lifecycle support for model-selected commitment updates.
//!
//! AI owns the semantic action (`fulfilled`, `cancelled`, `superseded`, or `expired`). This module
//! validates only closed enums, referenced active ids, temporal preconditions, tenant scope, and
//! the atomic MongoDB transition shape. Customer text is never inspected here.

use mongodb::bson::{doc, Bson, DateTime, Document};

use crate::models::CommitmentRepr;

use super::types::{CommitmentLifecycleAction, CommitmentLifecycleDecision};

const MAX_COMMITMENT_UPDATES_PER_TURN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitmentLifecycleIssue {
    TooManyUpdates,
    MissingCommitmentId,
    MissingReason,
    ReasonTooLong,
    InvalidAction,
    DuplicateCommitmentId,
    CommitmentNotActive,
    ExpiryRequiresPastDueAt,
    SupersedeRequiresReplacement,
}

impl CommitmentLifecycleIssue {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::TooManyUpdates => "commitment_updates_over_limit",
            Self::MissingCommitmentId => "commitment_update_id_missing",
            Self::MissingReason => "commitment_update_reason_missing",
            Self::ReasonTooLong => "commitment_update_reason_too_long",
            Self::InvalidAction => "commitment_update_action_invalid",
            Self::DuplicateCommitmentId => "commitment_update_duplicate_id",
            Self::CommitmentNotActive => "commitment_update_target_not_active",
            Self::ExpiryRequiresPastDueAt => "commitment_expiry_requires_past_due_at",
            Self::SupersedeRequiresReplacement => "commitment_supersede_requires_replacement",
        }
    }

    pub(crate) fn repair_instruction(self) -> &'static str {
        match self {
            Self::TooManyUpdates => {
                "Return no more than eight commitmentUpdates and include only transitions required by this turn."
            }
            Self::MissingCommitmentId
            | Self::DuplicateCommitmentId
            | Self::CommitmentNotActive => {
                "Every commitmentUpdates item must reference one unique id from the injected current active commitments list. Do not invent ids or update pending/terminal entries."
            }
            Self::MissingReason | Self::ReasonTooLong => {
                "Every commitmentUpdates item needs one concise audit reason, no longer than 500 characters, explaining the semantic basis in the full conversation."
            }
            Self::InvalidAction => {
                "Each commitmentUpdates.action must be exactly fulfilled, cancelled, superseded, or expired."
            }
            Self::ExpiryRequiresPastDueAt => {
                "Use expired only for an active commitment whose injected dueAtMillis is present and not later than the current time. Otherwise leave it active or choose another supported lifecycle action."
            }
            Self::SupersedeRequiresReplacement => {
                "Use superseded only when this same reply creates one new commitment that replaces the referenced active commitment; include lastCommitment/commitment for the replacement."
            }
        }
    }
}

pub(crate) fn validate_commitment_updates(
    commitments: &[CommitmentRepr],
    updates: &[CommitmentLifecycleDecision],
    has_replacement_commitment: bool,
    now: DateTime,
) -> Option<CommitmentLifecycleIssue> {
    if updates.len() > MAX_COMMITMENT_UPDATES_PER_TURN {
        return Some(CommitmentLifecycleIssue::TooManyUpdates);
    }

    let mut seen = Vec::with_capacity(updates.len());
    for update in updates {
        let commitment_id = update.commitment_id.trim();
        if commitment_id.is_empty() {
            return Some(CommitmentLifecycleIssue::MissingCommitmentId);
        }
        if seen.iter().any(|existing| existing == commitment_id) {
            return Some(CommitmentLifecycleIssue::DuplicateCommitmentId);
        }
        seen.push(commitment_id.to_string());

        let reason = update.reason.trim();
        if reason.is_empty() {
            return Some(CommitmentLifecycleIssue::MissingReason);
        }
        if reason.chars().count() > 500 {
            return Some(CommitmentLifecycleIssue::ReasonTooLong);
        }
        if update.action == CommitmentLifecycleAction::Unknown {
            return Some(CommitmentLifecycleIssue::InvalidAction);
        }

        let Some(entry) = commitments.iter().find_map(|repr| match repr {
            CommitmentRepr::Structured(entry)
                if entry.status == "active" && entry.id == commitment_id =>
            {
                Some(entry)
            }
            _ => None,
        }) else {
            return Some(CommitmentLifecycleIssue::CommitmentNotActive);
        };

        if update.action == CommitmentLifecycleAction::Expired
            && entry
                .due_at
                .is_none_or(|due_at| due_at.timestamp_millis() > now.timestamp_millis())
        {
            return Some(CommitmentLifecycleIssue::ExpiryRequiresPastDueAt);
        }
        if update.action == CommitmentLifecycleAction::Superseded && !has_replacement_commitment {
            return Some(CommitmentLifecycleIssue::SupersedeRequiresReplacement);
        }
    }
    None
}

/// Compact prompt projection of current delivered commitments. Legacy plain rows and terminal or
/// pending structured rows have no current action authority and are intentionally excluded.
pub(crate) fn active_commitments_for_prompt(
    commitments: &[CommitmentRepr],
) -> Vec<serde_json::Value> {
    commitments
        .iter()
        .filter_map(|repr| match repr {
            CommitmentRepr::Structured(entry)
                if entry.status == "active"
                    && !entry.id.trim().is_empty()
                    && !entry.text.trim().is_empty() =>
            {
                Some(serde_json::json!({
                    "id": entry.id,
                    "text": entry.text,
                    "status": entry.status,
                    "dueAtMillis": entry.due_at.map(|value| value.timestamp_millis()),
                    "createdAtMillis": entry.created_at.timestamp_millis(),
                }))
            }
            _ => None,
        })
        .collect()
}

pub(crate) fn format_active_commitments_for_prompt(commitments: &[CommitmentRepr]) -> String {
    serde_json::to_string(&active_commitments_for_prompt(commitments))
        .unwrap_or_else(|_| "[]".to_string())
}

pub(crate) struct CommitmentTransitionMutation {
    pub(crate) commitment_id: String,
    pub(crate) target_status: &'static str,
    pub(crate) filter: Document,
    pub(crate) pipeline: Vec<Document>,
}

fn target_status(action: CommitmentLifecycleAction) -> Option<&'static str> {
    match action {
        CommitmentLifecycleAction::Fulfilled => Some("fulfilled"),
        CommitmentLifecycleAction::Cancelled => Some("cancelled"),
        CommitmentLifecycleAction::Superseded => Some("superseded"),
        CommitmentLifecycleAction::Expired => Some("expired"),
        CommitmentLifecycleAction::Unknown => None,
    }
}

pub(crate) fn build_commitment_transition_mutations(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    updates: &[CommitmentLifecycleDecision],
    replacement_commitment_id: Option<&str>,
    lifecycle_source_id: &str,
    now: DateTime,
) -> Result<Vec<CommitmentTransitionMutation>, String> {
    let mut mutations = Vec::with_capacity(updates.len());
    for update in updates {
        let target_status = target_status(update.action)
            .ok_or_else(|| "unknown commitment lifecycle action".to_string())?;
        let commitment_id = update.commitment_id.trim();
        if commitment_id.is_empty() {
            return Err("commitment lifecycle id is empty".to_string());
        }
        let replacement = if update.action == CommitmentLifecycleAction::Superseded {
            Some(
                replacement_commitment_id
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "superseded commitment has no replacement id".to_string())?,
            )
        } else {
            None
        };

        let mut terminal_fields = doc! {
            "status": target_status,
            "fulfilledAt": Bson::Null,
            "cancelledAt": Bson::Null,
            "supersededAt": Bson::Null,
            "expiredAt": Bson::Null,
            "supersededBy": Bson::Null,
            "lifecycleUpdatedAt": now,
            "lifecycleReason": update.reason.trim(),
            "lifecycleSourceId": lifecycle_source_id,
        };
        match update.action {
            CommitmentLifecycleAction::Fulfilled => {
                terminal_fields.insert("fulfilledAt", now);
            }
            CommitmentLifecycleAction::Cancelled => {
                terminal_fields.insert("cancelledAt", now);
            }
            CommitmentLifecycleAction::Superseded => {
                terminal_fields.insert("supersededAt", now);
                terminal_fields.insert("supersededBy", replacement.unwrap());
            }
            CommitmentLifecycleAction::Expired => {
                terminal_fields.insert("expiredAt", now);
            }
            CommitmentLifecycleAction::Unknown => unreachable!(),
        }

        let mut already_applied = doc! {
            "id": commitment_id,
            "status": target_status,
            "lifecycleSourceId": lifecycle_source_id,
        };
        if let Some(replacement) = replacement {
            already_applied.insert("supersededBy", replacement);
        }
        let filter = doc! {
            "workspace_id": workspace_id,
            "account_id": account_id,
            "wxid": contact_wxid,
            "$or": [
                { "commitments": { "$elemMatch": { "id": commitment_id, "status": "active" } } },
                { "commitments": { "$elemMatch": already_applied } },
            ],
        };
        let pipeline = vec![doc! {
            "$set": {
                "commitments": {
                    "$map": {
                        "input": { "$ifNull": ["$commitments", Bson::Array(Vec::new())] },
                        "as": "entry",
                        "in": {
                            "$cond": [
                                {
                                    "$and": [
                                        { "$eq": [{ "$type": "$$entry" }, "object"] },
                                        { "$eq": ["$$entry.id", commitment_id] },
                                        { "$eq": ["$$entry.status", "active"] },
                                    ]
                                },
                                { "$mergeObjects": ["$$entry", terminal_fields] },
                                "$$entry",
                            ]
                        }
                    }
                },
                "updated_at": now,
            }
        }];
        mutations.push(CommitmentTransitionMutation {
            commitment_id: commitment_id.to_string(),
            target_status,
            filter,
            pipeline,
        });
    }
    Ok(mutations)
}

/// Apply already-authorized transitions to an in-memory contact projection. This mirrors the
/// MongoDB mutation semantics for simulations: active rows transition once, an already-terminal
/// row is a stale/idempotent no-op, and customer text is never inspected.
pub(crate) fn apply_commitment_updates_to_projection(
    commitments: &mut [CommitmentRepr],
    updates: &[CommitmentLifecycleDecision],
    replacement_commitment_id: Option<&str>,
    lifecycle_source_id: &str,
    now: DateTime,
) -> Result<Vec<String>, String> {
    let mut applied = Vec::new();
    for update in updates {
        let target_status = target_status(update.action)
            .ok_or_else(|| "unknown commitment lifecycle action".to_string())?;
        let replacement = if update.action == CommitmentLifecycleAction::Superseded {
            Some(
                replacement_commitment_id
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "superseded commitment has no replacement id".to_string())?,
            )
        } else {
            None
        };
        let Some(entry) = commitments.iter_mut().find_map(|repr| match repr {
            CommitmentRepr::Structured(entry) if entry.id == update.commitment_id => Some(entry),
            _ => None,
        }) else {
            continue;
        };
        if entry.status != "active" {
            continue;
        }

        entry.status = target_status.to_string();
        entry.fulfilled_at = None;
        entry.cancelled_at = None;
        entry.superseded_at = None;
        entry.expired_at = None;
        entry.superseded_by = None;
        entry.lifecycle_updated_at = Some(now);
        entry.lifecycle_reason = Some(update.reason.trim().to_string());
        entry.lifecycle_source_id = Some(lifecycle_source_id.to_string());
        match update.action {
            CommitmentLifecycleAction::Fulfilled => entry.fulfilled_at = Some(now),
            CommitmentLifecycleAction::Cancelled => entry.cancelled_at = Some(now),
            CommitmentLifecycleAction::Superseded => {
                entry.superseded_at = Some(now);
                entry.superseded_by = replacement.map(ToString::to_string);
            }
            CommitmentLifecycleAction::Expired => entry.expired_at = Some(now),
            CommitmentLifecycleAction::Unknown => unreachable!(),
        }
        applied.push(entry.id.clone());
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CommitmentEntry;

    fn active_entry(id: &str, due_at: Option<DateTime>) -> CommitmentEntry {
        let mut entry = CommitmentEntry::from_plain_text("send the agreed document".to_string());
        entry.id = id.to_string();
        entry.status = "active".to_string();
        entry.due_at = due_at;
        entry
    }

    #[test]
    fn prompt_projection_contains_only_delivered_active_structured_rows() {
        let mut terminal = active_entry("terminal", None);
        terminal.status = "fulfilled".to_string();
        let mut pending = active_entry("pending", None);
        pending.status = "pending_delivery".to_string();
        let commitments = vec![
            CommitmentRepr::Plain("legacy".to_string()),
            CommitmentRepr::Structured(terminal),
            CommitmentRepr::Structured(pending),
            CommitmentRepr::Structured(active_entry("active", None)),
        ];

        let rendered = format_active_commitments_for_prompt(&commitments);
        assert!(rendered.contains("active"));
        assert!(!rendered.contains("terminal"));
        assert!(!rendered.contains("pending"));
        assert!(!rendered.contains("legacy"));
    }

    #[test]
    fn expiry_requires_an_existing_past_due_time() {
        let now = DateTime::from_millis(10_000);
        let update = CommitmentLifecycleDecision {
            commitment_id: "c1".to_string(),
            action: CommitmentLifecycleAction::Expired,
            reason: "the obligation window ended".to_string(),
        };
        let future = vec![CommitmentRepr::Structured(active_entry(
            "c1",
            Some(DateTime::from_millis(20_000)),
        ))];
        assert_eq!(
            validate_commitment_updates(&future, &[update.clone()], false, now),
            Some(CommitmentLifecycleIssue::ExpiryRequiresPastDueAt)
        );

        let past = vec![CommitmentRepr::Structured(active_entry(
            "c1",
            Some(DateTime::from_millis(5_000)),
        ))];
        assert_eq!(
            validate_commitment_updates(&past, &[update], false, now),
            None
        );
    }

    #[test]
    fn supersede_requires_a_same_turn_replacement() {
        let commitments = vec![CommitmentRepr::Structured(active_entry("c1", None))];
        let update = CommitmentLifecycleDecision {
            commitment_id: "c1".to_string(),
            action: CommitmentLifecycleAction::Superseded,
            reason: "a new commitment replaces it".to_string(),
        };
        assert_eq!(
            validate_commitment_updates(&commitments, &[update.clone()], false, DateTime::now()),
            Some(CommitmentLifecycleIssue::SupersedeRequiresReplacement)
        );
        assert_eq!(
            validate_commitment_updates(&commitments, &[update], true, DateTime::now()),
            None
        );
    }

    #[test]
    fn transition_pipeline_is_tenant_scoped_and_idempotent_without_text_matching() {
        let update = CommitmentLifecycleDecision {
            commitment_id: "c1".to_string(),
            action: CommitmentLifecycleAction::Superseded,
            reason: "new obligation replaces the old one".to_string(),
        };
        let mutations = build_commitment_transition_mutations(
            "ws",
            "acct",
            "wxid",
            &[update],
            Some("c2"),
            "run-1",
            DateTime::from_millis(100),
        )
        .expect("mutation");
        let mutation = &mutations[0];
        assert_eq!(mutation.filter.get_str("workspace_id"), Ok("ws"));
        assert_eq!(mutation.filter.get_str("account_id"), Ok("acct"));
        assert_eq!(mutation.filter.get_str("wxid"), Ok("wxid"));
        let rendered = format!("{:?}", mutation.pipeline);
        assert!(rendered.contains("supersededBy"));
        assert!(rendered.contains("c2"));
        assert!(rendered.contains("lifecycleSourceId"));
        assert!(!rendered.contains("customer"));
    }

    #[test]
    fn validation_rejects_forged_pending_terminal_and_duplicate_targets() {
        let mut pending = active_entry("pending", None);
        pending.status = "pending_delivery".to_string();
        let mut terminal = active_entry("terminal", None);
        terminal.status = "fulfilled".to_string();
        let commitments = vec![
            CommitmentRepr::Structured(active_entry("active", None)),
            CommitmentRepr::Structured(pending),
            CommitmentRepr::Structured(terminal),
        ];
        let update = |id: &str| CommitmentLifecycleDecision {
            commitment_id: id.to_string(),
            action: CommitmentLifecycleAction::Cancelled,
            reason: "the full conversation no longer requires this obligation".to_string(),
        };

        for id in ["forged", "pending", "terminal"] {
            assert_eq!(
                validate_commitment_updates(&commitments, &[update(id)], false, DateTime::now()),
                Some(CommitmentLifecycleIssue::CommitmentNotActive),
                "{id} must not be accepted as an active target"
            );
        }
        assert_eq!(
            validate_commitment_updates(
                &commitments,
                &[update("active"), update("active")],
                false,
                DateTime::now(),
            ),
            Some(CommitmentLifecycleIssue::DuplicateCommitmentId)
        );
    }

    #[test]
    fn projection_applies_all_terminal_actions_and_is_idempotent() {
        let now = DateTime::from_millis(123_000);
        let cases = [
            (CommitmentLifecycleAction::Fulfilled, "fulfilled"),
            (CommitmentLifecycleAction::Cancelled, "cancelled"),
            (CommitmentLifecycleAction::Superseded, "superseded"),
            (CommitmentLifecycleAction::Expired, "expired"),
        ];
        for (index, (action, expected_status)) in cases.into_iter().enumerate() {
            let id = format!("c{index}");
            let mut commitments = vec![CommitmentRepr::Structured(active_entry(&id, None))];
            let update = CommitmentLifecycleDecision {
                commitment_id: id.clone(),
                action,
                reason: format!("semantic lifecycle reason {index}"),
            };
            let replacement =
                (action == CommitmentLifecycleAction::Superseded).then_some("replacement");
            let applied = apply_commitment_updates_to_projection(
                &mut commitments,
                &[update.clone()],
                replacement,
                "decision-1",
                now,
            )
            .expect("projection transition");
            assert_eq!(applied, vec![id.clone()]);
            let CommitmentRepr::Structured(entry) = &commitments[0] else {
                unreachable!();
            };
            assert_eq!(entry.status, expected_status);
            assert_eq!(entry.lifecycle_updated_at, Some(now));
            assert_eq!(entry.lifecycle_source_id.as_deref(), Some("decision-1"));
            assert_eq!(
                entry.superseded_by.as_deref(),
                (action == CommitmentLifecycleAction::Superseded).then_some("replacement")
            );

            let retry = apply_commitment_updates_to_projection(
                &mut commitments,
                &[update],
                replacement,
                "decision-1",
                now,
            )
            .expect("idempotent retry");
            assert!(retry.is_empty());
        }
    }
}

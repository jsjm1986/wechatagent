//! Backfill durable per-escalation ownership behind the coarse principal-awaiting marker.
//!
//! We only materialize owners backed by durable evidence:
//! - every pending escalation;
//! - resolved new-protocol rows whose relay is pending/enqueued;
//! - legacy resolved rows that still have a pre-Outbox principal relay task.
//!
//! Legacy `outbox_enqueued` is deliberately not treated as active: old workers could deliver
//! successfully without finalizing the task, so that state is ambiguous and must not create a
//! new permanent awaiting marker. This migration never creates/replays tasks and never clears an
//! existing coarse marker.

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
    models::{
        AWAITING_PRINCIPAL_DECISION_ATTR, AWAITING_PRINCIPAL_DECISION_IDS_ATTR,
        PRINCIPAL_ESCALATION_STATUS_PENDING, PRINCIPAL_ESCALATION_STATUS_RESOLVED,
        PRINCIPAL_RELAY_STATE_ENQUEUED, PRINCIPAL_RELAY_STATE_PENDING,
    },
};

type ContactKey = (String, String, String);

pub async fn run_step(db: &Database) -> AppResult<()> {
    let names = db.raw().list_collection_names(None).await?;
    if !names
        .iter()
        .any(|name| name == "agent_principal_escalations")
    {
        return Ok(());
    }
    if !names.iter().any(|name| name == "contacts") {
        return Err(AppError::External(
            "principal awaiting owner migration found escalations without contacts collection"
                .to_string(),
        ));
    }

    let escalations = db
        .raw()
        .collection::<Document>("agent_principal_escalations");
    let tasks = db.raw().collection::<Document>("agent_tasks");
    let contacts = db.raw().collection::<Document>("contacts");

    let mut rows_by_id = HashMap::<ObjectId, (ContactKey, String)>::new();
    let mut owners = HashMap::<ContactKey, HashSet<ObjectId>>::new();
    let mut cursor = escalations
        .find(
            doc! { "$or": [
                { "status": PRINCIPAL_ESCALATION_STATUS_PENDING },
                {
                    "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                    "relay_state": { "$in": [
                        PRINCIPAL_RELAY_STATE_PENDING,
                        PRINCIPAL_RELAY_STATE_ENQUEUED,
                    ] },
                },
                {
                    "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                    "relay_state": { "$exists": false },
                },
            ] },
            None,
        )
        .await?;
    while let Some(row) = cursor.try_next().await? {
        let id = required_object_id(&row, "_id")?;
        let key = contact_key(&row, id)?;
        let short_code = canonical(&row, "short_code", id)?.to_string();
        let status = canonical(&row, "status", id)?;
        let definitely_active = status == PRINCIPAL_ESCALATION_STATUS_PENDING
            || row.get_str("relay_state").is_ok_and(|state| {
                state == PRINCIPAL_RELAY_STATE_PENDING || state == PRINCIPAL_RELAY_STATE_ENQUEUED
            });
        if definitely_active {
            owners.entry(key.clone()).or_default().insert(id);
        }
        rows_by_id.insert(id, (key, short_code));
    }

    if names.iter().any(|name| name == "agent_tasks") {
        let mut task_cursor = tasks.find(active_legacy_relay_task_filter(), None).await?;
        while let Some(task) = task_cursor.try_next().await? {
            let task_id = task.get_object_id("_id").ok();
            let task_key = contact_key_for_task(&task)?;
            let content = canonical_for_task(&task, "content")?;
            let matched = task_id
                .and_then(|id| rows_by_id.get(&id).map(|row| (id, row)))
                .filter(|(_, (key, code))| key == &task_key && code == content)
                .or_else(|| {
                    rows_by_id
                        .iter()
                        .find(|(_, (key, code))| key == &task_key && code == content)
                        .map(|(id, row)| (*id, row))
                });
            if let Some((id, (key, _))) = matched {
                owners.entry(key.clone()).or_default().insert(id);
            }
        }
    }

    // Validate all contact identities before the first write.
    for key in owners.keys() {
        let count = contacts.count_documents(contact_filter(key), None).await?;
        if count != 1 {
            return Err(AppError::External(format!(
                "principal awaiting owner migration expected exactly one contact for {:?}, found {count}",
                key
            )));
        }
    }

    let mut updated = 0_u64;
    for (key, ids) in owners {
        let mut id_strings: Vec<String> = ids.into_iter().map(|id| id.to_hex()).collect();
        id_strings.sort();
        let result = contacts
            .update_one(
                contact_filter(&key),
                owner_backfill_pipeline(&id_strings, DateTime::now()),
                None,
            )
            .await?;
        updated += result.modified_count;
    }
    tracing::info!(
        migration_id = "2026_07_047_backfill_principal_awaiting_owners",
        updated_contacts = updated,
        "backfilled durable principal awaiting owners"
    );
    Ok(())
}

fn owner_backfill_pipeline(owners: &[String], now: DateTime) -> Vec<Document> {
    let owners_path = format!("$domain_attributes.{AWAITING_PRINCIPAL_DECISION_IDS_ATTR}");
    let mut patch = Document::new();
    patch.insert(AWAITING_PRINCIPAL_DECISION_ATTR, true);
    patch.insert(
        AWAITING_PRINCIPAL_DECISION_IDS_ATTR,
        doc! { "$setUnion": ["$$owners", owners] },
    );
    vec![doc! { "$set": {
        "domain_attributes": {
            "$let": {
                "vars": {
                    "attrs": { "$cond": [
                        { "$eq": [{ "$type": "$domain_attributes" }, "object"] },
                        "$domain_attributes",
                        {},
                    ] },
                    "owners": { "$cond": [
                        { "$isArray": &owners_path },
                        &owners_path,
                        [],
                    ] },
                },
                "in": { "$mergeObjects": ["$$attrs", patch] },
            },
        },
        "domain_attributes_updated_at": now,
    } }]
}

fn required_object_id(row: &Document, field: &str) -> AppResult<ObjectId> {
    row.get_object_id(field).map_err(|_| {
        AppError::External(format!(
            "principal awaiting owner migration found row without ObjectId {field}: {:?}",
            row.get("_id").unwrap_or(&Bson::Null)
        ))
    })
}

fn canonical<'a>(row: &'a Document, field: &str, id: ObjectId) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "principal awaiting owner migration found escalation {id} without {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "principal awaiting owner migration found escalation {id} with non-canonical {field}"
        )));
    }
    Ok(value)
}

fn contact_key(row: &Document, id: ObjectId) -> AppResult<ContactKey> {
    Ok((
        canonical(row, "workspace_id", id)?.to_string(),
        canonical(row, "account_id", id)?.to_string(),
        canonical(row, "contact_wxid", id)?.to_string(),
    ))
}

fn canonical_for_task<'a>(row: &'a Document, field: &str) -> AppResult<&'a str> {
    let id = row.get("_id").unwrap_or(&Bson::Null);
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "principal awaiting owner migration found active relay task {id:?} without {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "principal awaiting owner migration found active relay task {id:?} with non-canonical {field}"
        )));
    }
    Ok(value)
}

fn contact_key_for_task(row: &Document) -> AppResult<ContactKey> {
    Ok((
        canonical_for_task(row, "workspace_id")?.to_string(),
        canonical_for_task(row, "account_id")?.to_string(),
        canonical_for_task(row, "contact_wxid")?.to_string(),
    ))
}

fn contact_filter(key: &ContactKey) -> Document {
    doc! {
        "workspace_id": &key.0,
        "account_id": &key.1,
        "wxid": &key.2,
    }
}

fn active_legacy_relay_task_filter() -> Document {
    doc! {
        "kind": "principal_decision_relay",
        // Once a legacy task reached Outbox, delivery may already have succeeded even when the
        // old worker failed to finalize the task. Only pre-Outbox states are safe owner evidence.
        "status": { "$in": ["pending", "retry", "running"] },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_backfill_uses_wire_names_and_set_union() {
        let pipeline = owner_backfill_pipeline(
            &["a".to_string(), "b".to_string()],
            DateTime::from_millis(1),
        );
        let rendered = format!("{pipeline:?}");
        assert!(rendered.contains(AWAITING_PRINCIPAL_DECISION_ATTR));
        assert!(rendered.contains(AWAITING_PRINCIPAL_DECISION_IDS_ATTR));
        assert!(rendered.contains("$setUnion"));
        assert!(!rendered.contains("awaiting_key"));
        assert!(!rendered.contains("owners_key"));
    }

    #[test]
    fn legacy_owner_evidence_stops_before_outbox() {
        let filter = active_legacy_relay_task_filter();
        let allowed = filter
            .get_document("status")
            .expect("status predicate")
            .get_array("$in")
            .expect("allowed task states")
            .iter()
            .map(|value| value.as_str().expect("string task state"))
            .collect::<Vec<_>>();

        assert_eq!(allowed, vec!["pending", "retry", "running"]);
        assert!(!allowed.contains(&"outbox_enqueued"));
    }
}

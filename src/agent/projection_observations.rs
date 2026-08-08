//! Idempotent observation ledger for crash-replayed post-decision effects.
//!
//! Main entities keep aggregate counters for reads, while this collection owns the strict
//! `(entity, run)` identity. Replays insert the same observation at most once and then reconcile
//! the aggregate from the ledger count, so a crash between the two writes is self-healing.

use std::collections::HashSet;

use mongodb::bson::{doc, DateTime, Document};

use crate::db::Database;
use crate::error::{AppError, AppResult};

pub(crate) const COLLECTION: &str = "projection_observations";
pub(crate) const RECENT_RUN_IDS_LIMIT: i32 = 32;

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    let text = error.to_string();
    text.contains("E11000") || text.contains("duplicate key")
}

/// Persist strict identities for every legacy cached run plus the current run, then return the
/// authoritative ledger count. Backfilling before truncating the display cache preserves strict
/// idempotency even for a very late replay of a historical run.
pub(crate) async fn record_and_count(
    db: &Database,
    workspace_id: &str,
    entity_type: &str,
    entity_id: &str,
    legacy_run_ids: &[String],
    run_id: &str,
) -> AppResult<i64> {
    let collection = db.raw().collection::<Document>(COLLECTION);
    let mut identities = legacy_run_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    identities.insert(run_id);
    for identity in identities {
        match collection
            .insert_one(
                doc! {
                    "workspace_id": workspace_id,
                    "entity_type": entity_type,
                    "entity_id": entity_id,
                    "run_id": identity,
                    "observed_at": DateTime::now(),
                },
                None,
            )
            .await
        {
            Ok(_) => {}
            Err(error) if duplicate_key(&error) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let count = collection
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "entity_type": entity_type,
                "entity_id": entity_id,
            },
            None,
        )
        .await?;
    i64::try_from(count)
        .map_err(|_| AppError::External("projection observation count overflow".to_string()))
}

/// Aggregation-pipeline stages that reconcile a legacy aggregate with the ledger count and keep
/// only a bounded recent-run display cache. The baseline captures observations that predate the
/// legacy run-id array (if any); `$max` makes concurrent reconciliation monotonic.
pub(crate) fn reconcile_stages(
    ledger_count: i64,
    run_id: &str,
    legacy_run_id_count: i64,
) -> Vec<Document> {
    vec![
        doc! { "$set": {
            "observation_ledger_baseline": { "$ifNull": [
                "$observation_ledger_baseline",
                { "$max": [
                    0i64,
                    { "$subtract": [
                        { "$ifNull": ["$occurrences", 0i64] },
                        legacy_run_id_count,
                    ] },
                ] },
            ] },
        } },
        doc! { "$set": {
            "occurrences": { "$max": [
                { "$ifNull": ["$occurrences", 0i64] },
                { "$add": ["$observation_ledger_baseline", ledger_count] },
            ] },
            "source_run_ids": { "$slice": [
                { "$concatArrays": [
                    { "$filter": {
                        "input": { "$ifNull": ["$source_run_ids", Vec::<String>::new()] },
                        "as": "source_run_id",
                        "cond": { "$ne": ["$$source_run_id", run_id] },
                    } },
                    [run_id],
                ] },
                -RECENT_RUN_IDS_LIMIT,
            ] },
        } },
    ]
}

pub(crate) fn source_run_ids(document: &Document) -> Vec<String> {
    let mut seen = HashSet::new();
    document
        .get_array("source_run_ids")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .filter(|value| seen.insert((*value).to_string()))
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciliation_is_bounded_and_preserves_legacy_count() {
        let stages = reconcile_stages(1, "run-1", 1);
        assert_eq!(stages.len(), 2);
        let rendered = format!("{stages:?}");
        assert!(rendered.contains("observation_ledger_baseline"));
        assert!(rendered.contains("-32"));
    }
}

//! Audit and backfill the durable Lesson -> peer-case identity.
//!
//! Older promotion code inserted a Chunk before updating the Lesson and did not
//! persist a source identity on the Chunk.  This migration validates every
//! legacy promotion before its first write, backfills only exact Lesson/Chunk
//! pairs, and rejects orphaned or ambiguous promotion-shaped chunks rather than
//! guessing which business fact should win.

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};

use crate::{
    db::Database,
    error::{AppError, AppResult},
};

pub const LESSON_PROMOTION_SOURCE: &str = "lesson_promotion";

#[derive(Debug, Clone, PartialEq)]
struct BackfillPlan {
    chunk_id: ObjectId,
    provenance: Document,
}

fn canonical<'a>(row: &'a Document, field: &str, kind: &str) -> AppResult<&'a str> {
    let value = row.get_str(field).map_err(|_| {
        AppError::External(format!(
            "lesson promotion migration found {kind} without string {field}"
        ))
    })?;
    if value.is_empty() || value.trim() != value {
        return Err(AppError::External(format!(
            "lesson promotion migration found {kind} with non-canonical {field}"
        )));
    }
    Ok(value)
}

fn row_id(row: &Document, kind: &str) -> AppResult<ObjectId> {
    row.get_object_id("_id").map_err(|_| {
        AppError::External(format!(
            "lesson promotion migration found {kind} without ObjectId _id"
        ))
    })
}

fn promotion_shaped_chunk(row: &Document) -> bool {
    row.get_str("chunk_type") == Ok("peer_case")
        && row
            .get_str("business_context")
            .is_ok_and(|value| value.starts_with("lessons_learned::"))
}

fn exact_anchor(row: &Document, lesson_id: &str) -> bool {
    row.get_document("provenance").is_ok_and(|provenance| {
        provenance.get_str("source") == Ok(LESSON_PROMOTION_SOURCE)
            && provenance.get_str("source_doc_id") == Ok(lesson_id)
    })
}

fn plan_rows(
    lessons: impl IntoIterator<Item = Document>,
    chunks: impl IntoIterator<Item = Document>,
) -> AppResult<Vec<BackfillPlan>> {
    let mut chunks_by_id = HashMap::new();
    for chunk in chunks {
        let id = row_id(&chunk, "chunk")?;
        if chunks_by_id.insert(id, chunk).is_some() {
            return Err(AppError::External(format!(
                "lesson promotion migration found duplicate chunk _id {id}"
            )));
        }
    }

    let mut lesson_keys = HashSet::new();
    let mut referenced_chunks = HashMap::new();
    let mut plans = Vec::new();
    for lesson in lessons {
        let lesson_row_id = row_id(&lesson, "lesson")?;
        let workspace = canonical(&lesson, "workspace_id", "lesson")?.to_string();
        let lesson_id = canonical(&lesson, "lesson_id", "lesson")?.to_string();
        if !lesson_keys.insert((workspace.clone(), lesson_id.clone())) {
            return Err(AppError::External(format!(
                "lesson promotion migration found duplicate lesson identity {workspace}/{lesson_id}"
            )));
        }
        let status = match lesson.get("review_status") {
            None | Some(Bson::Null) => "pending_review",
            Some(Bson::String(value)) if !value.is_empty() && value.trim() == value => value,
            Some(_) => {
                return Err(AppError::External(format!(
                    "lesson promotion migration found invalid review_status for {lesson_id}"
                )))
            }
        };
        if !matches!(status, "pending_review" | "promoted") {
            return Err(AppError::External(format!(
                "lesson promotion migration found invalid review_status {status} for {lesson_id}"
            )));
        }
        let promoted_chunk = match lesson.get("promoted_chunk_id") {
            None | Some(Bson::Null) => None,
            Some(Bson::String(value)) => Some(ObjectId::parse_str(value).map_err(|_| {
                AppError::External(format!(
                    "lesson promotion migration found invalid promoted_chunk_id for {lesson_id}"
                ))
            })?),
            Some(_) => {
                return Err(AppError::External(format!(
                    "lesson promotion migration found non-string promoted_chunk_id for {lesson_id}"
                )))
            }
        };
        if status == "pending_review" {
            if promoted_chunk.is_some() {
                return Err(AppError::External(format!(
                    "lesson promotion migration found pending lesson {lesson_id} with promoted chunk"
                )));
            }
            continue;
        }
        let chunk_id = promoted_chunk.ok_or_else(|| {
            AppError::External(format!(
                "lesson promotion migration found promoted lesson {lesson_id} without chunk"
            ))
        })?;
        if let Some(previous) = referenced_chunks.insert(chunk_id, lesson_id.clone()) {
            return Err(AppError::External(format!(
                "lesson promotion migration found chunk {chunk_id} referenced by {previous} and {lesson_id}"
            )));
        }
        let chunk = chunks_by_id.get(&chunk_id).ok_or_else(|| {
            AppError::External(format!(
                "lesson promotion migration found promoted lesson {lesson_id} with missing chunk {chunk_id}"
            ))
        })?;
        if chunk.get_str("workspace_id") != Ok(workspace.as_str()) || !promotion_shaped_chunk(chunk)
        {
            return Err(AppError::External(format!(
                "lesson promotion migration found mismatched chunk {chunk_id} for {lesson_id}"
            )));
        }
        let expected_context = format!(
            "lessons_learned::{}",
            canonical(&lesson, "pattern_kind", "lesson")?
        );
        if chunk.get_str("business_context") != Ok(expected_context.as_str()) {
            return Err(AppError::External(format!(
                "lesson promotion migration found wrong context on chunk {chunk_id} for {lesson_id}"
            )));
        }
        match chunk.get("provenance") {
            None | Some(Bson::Null) => {
                let edited_at = lesson
                    .get_datetime("updated_at")
                    .copied()
                    .unwrap_or_else(|_| DateTime::now());
                plans.push(BackfillPlan {
                    chunk_id,
                    provenance: doc! {
                        "source": LESSON_PROMOTION_SOURCE,
                        "source_doc_id": &lesson_id,
                        "edited_at": edited_at,
                    },
                });
            }
            Some(Bson::Document(_)) if exact_anchor(chunk, &lesson_id) => {}
            Some(_) => {
                return Err(AppError::External(format!(
                    "lesson promotion migration found conflicting provenance on chunk {chunk_id} for {lesson_id}"
                )))
            }
        }
        let _ = lesson_row_id;
    }

    for (chunk_id, chunk) in &chunks_by_id {
        if promotion_shaped_chunk(chunk) && !referenced_chunks.contains_key(chunk_id) {
            return Err(AppError::External(format!(
                "lesson promotion migration found orphan promotion-shaped chunk {chunk_id}"
            )));
        }
        if chunk
            .get_document("provenance")
            .is_ok_and(|value| value.get_str("source") == Ok(LESSON_PROMOTION_SOURCE))
            && !referenced_chunks.contains_key(chunk_id)
        {
            return Err(AppError::External(format!(
                "lesson promotion migration found unowned lesson anchor on chunk {chunk_id}"
            )));
        }
    }

    plans.sort_by_key(|plan| plan.chunk_id.to_hex());
    Ok(plans)
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    let names = db.raw().list_collection_names(None).await?;
    if !names.iter().any(|name| name == "lessons_learned") {
        return Ok(());
    }
    let lessons = db.raw().collection::<Document>("lessons_learned");
    let chunks = db
        .raw()
        .collection::<Document>("operation_knowledge_chunks");
    let mut lesson_cursor = lessons.find(doc! {}, None).await?;
    let mut lesson_rows = Vec::new();
    while let Some(row) = lesson_cursor.try_next().await? {
        lesson_rows.push(row);
    }
    let mut chunk_cursor = chunks
        .find(
            doc! {
                "$or": [
                    { "business_context": { "$regex": "^lessons_learned::" }, "chunk_type": "peer_case" },
                    { "provenance.source": LESSON_PROMOTION_SOURCE },
                ]
            },
            None,
        )
        .await?;
    let mut chunk_rows = Vec::new();
    while let Some(row) = chunk_cursor.try_next().await? {
        chunk_rows.push(row);
    }

    // Validation of the complete legacy graph finishes before the first write.
    let plans = plan_rows(lesson_rows, chunk_rows)?;
    for plan in &plans {
        let result = chunks
            .update_one(
                doc! {
                    "_id": plan.chunk_id,
                    "$or": [
                        { "provenance": { "$exists": false } },
                        { "provenance": null },
                    ],
                },
                doc! { "$set": { "provenance": plan.provenance.clone() } },
                None,
            )
            .await?;
        if result.modified_count != 1 {
            return Err(AppError::External(format!(
                "lesson promotion migration lost chunk {} during backfill",
                plan.chunk_id
            )));
        }
    }
    tracing::info!(
        migration_id = "2026_07_055_lesson_promotion_identity",
        backfilled_chunks = plans.len(),
        "audited and backfilled lesson promotion identities"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lesson(id: &str, status: &str, chunk_id: Option<ObjectId>) -> Document {
        doc! {
            "_id": ObjectId::new(),
            "workspace_id": "ws",
            "lesson_id": id,
            "pattern_kind": "success",
            "review_status": status,
            "promoted_chunk_id": chunk_id.map(|id| id.to_hex()),
            "updated_at": DateTime::from_millis(1_000),
        }
    }

    fn chunk(id: ObjectId, provenance: Option<Document>) -> Document {
        let mut row = doc! {
            "_id": id,
            "workspace_id": "ws",
            "chunk_type": "peer_case",
            "business_context": "lessons_learned::success",
        };
        if let Some(provenance) = provenance {
            row.insert("provenance", provenance);
        }
        row
    }

    #[test]
    fn plans_only_exact_legacy_pair() {
        let id = ObjectId::new();
        let plans = plan_rows(
            [lesson("ws::success", "promoted", Some(id))],
            [chunk(id, None)],
        )
        .unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].chunk_id, id);
        assert_eq!(
            plans[0].provenance.get_str("source_doc_id"),
            Ok("ws::success")
        );
    }

    #[test]
    fn rejects_orphan_or_conflicting_legacy_state() {
        let orphan = ObjectId::new();
        assert!(plan_rows(
            [lesson("ws::success", "pending_review", None)],
            [chunk(orphan, None)],
        )
        .unwrap_err()
        .to_string()
        .contains("orphan"));

        let id = ObjectId::new();
        assert!(plan_rows(
            [lesson("ws::success", "promoted", Some(id))],
            [chunk(
                id,
                Some(doc! { "source": "human", "edited_at": DateTime::now() })
            )],
        )
        .unwrap_err()
        .to_string()
        .contains("conflicting provenance"));
    }

    #[test]
    fn missing_review_status_is_legacy_pending_review() {
        let mut legacy = lesson("ws::success", "pending_review", None);
        legacy.remove("review_status");
        assert!(plan_rows([legacy], []).unwrap().is_empty());

        let mut invalid = lesson("ws::success", "pending_review", None);
        invalid.insert("review_status", 7);
        assert!(plan_rows([invalid], [])
            .unwrap_err()
            .to_string()
            .contains("review_status"));
    }
}

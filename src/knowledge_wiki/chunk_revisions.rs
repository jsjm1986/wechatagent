//! `apply_chunk_revision` 状态机 —— 写入路径的"七个动作"统一入口。
//!
//! 设计契约（沿用 `nashsu/llm_wiki` 的 page-merge 三层保护）：
//!
//! 1. **锁定字段守门**：patch 携带 `_id / workspace_id / account_id / document_id /
//!    item_id / wiki_type / chunk_type / created_at` 任一 → 4xx 拒收。
//!    LLM 永远没机会改这些字段。
//! 2. **数组字段 union**：`tags / search_terms / applicable_scenes / ...`
//!    永远 existing ∪ patch，应用层完成，LLM 输出空数组 ≠ 清空。
//! 3. **70% body 长度阈值**：patch 后 body/answer/summary 短于既有 70% → 拒收。
//!
//! 7 个动作：create / patch / split / merge / archive / restore / rollback。
//! 每次写入双写：`operation_knowledge_chunks` + `chunk_revisions`，先写 revisions
//! 后写 chunks（前者失败 → 直接 abort；后者失败 → revisions 留下"试图未成功"
//! 痕迹，便于人工查 last_revision != current_state）。
//!
//! AI source（`ProvenanceSource::Ai`）的写入强制 `status="draft"` +
//! `integrity_status="needs_review"`，对齐 CLAUDE.md "AI 永不自动 verify" 硬约束。

use std::str::FromStr;

use mongodb::{
    bson::{doc, oid::ObjectId, Bson, DateTime, Document},
    options::FindOptions,
    ClientSession,
};
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::knowledge_wiki::page_merge::{
    apply_field_patch, compute_chunk_hash, effective_locked_fields, enforce_locked_fields,
    is_body_truncated, union_array_fields, RevisionError, BODY_TRUNCATION_THRESHOLD,
    DEFAULT_LOCKED_FIELDS, DEFAULT_UNION_ARRAY_KEYS,
};
use crate::models::{CatalogRebuildJob, ChunkRevision, DomainSchema, OperationKnowledgeChunk};

pub(crate) fn chunk_replace_filter(
    chunk_object_id: ObjectId,
    workspace_id: &str,
    expected_updated_at: DateTime,
) -> Document {
    doc! {
        "_id": chunk_object_id,
        "workspace_id": workspace_id,
        "updated_at": expected_updated_at,
    }
}

/// MongoDB DateTime has millisecond precision. Keep the CAS token strictly newer.
pub(crate) fn monotonic_chunk_updated_at(
    expected_updated_at: DateTime,
    candidate: DateTime,
) -> DateTime {
    DateTime::from_millis(
        candidate
            .timestamp_millis()
            .max(expected_updated_at.timestamp_millis().saturating_add(1)),
    )
}

// ── 操作语义封闭枚举 ───────────────────────────────────────────────────

/// `chunk_revisions.op` 合法值（design.md §9 / CLAUDE.md）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionOp {
    Create,
    Patch,
    Split,
    Merge,
    Rollback,
    Archive,
    Restore,
    Verify,
    Unverify,
    Reject,
}

impl RevisionOp {
    pub fn as_str(self) -> &'static str {
        match self {
            RevisionOp::Create => "create",
            RevisionOp::Patch => "patch",
            RevisionOp::Split => "split",
            RevisionOp::Merge => "merge",
            RevisionOp::Rollback => "rollback",
            RevisionOp::Archive => "archive",
            RevisionOp::Restore => "restore",
            RevisionOp::Verify => "verify",
            RevisionOp::Unverify => "unverify",
            RevisionOp::Reject => "reject",
        }
    }
}

/// `chunk_revisions.source` 合法值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceSource {
    /// LLM 调用方写入；强制 `status=draft + integrity_status=needs_review`。
    Ai,
    /// 运营 / admin 直接编辑（包含 verify 通过后的人工签字路径）。
    Human,
    /// feedback worker / sweep / cleanup 触发的规则化写入。
    Rule,
    /// import_apply 流式块导入。
    Imported,
    /// 领导（真人）经决策请示通道授权的知识——验证者是领导本人，视同 Human
    /// 人类权威家族（绝非 AI 自动验证：源头是真人裁决，只是把知识库复核前移到
    /// 裁决当下）。不落入下方 `source=Ai` 的 draft 强制降级分支，故可直接带 verified。
    PrincipalAuthorized,
}

impl ProvenanceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ProvenanceSource::Ai => "ai",
            ProvenanceSource::Human => "human",
            ProvenanceSource::Rule => "rule",
            ProvenanceSource::Imported => "imported",
            ProvenanceSource::PrincipalAuthorized => "principal_authorized",
        }
    }
}

impl FromStr for ProvenanceSource {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ai" => Ok(ProvenanceSource::Ai),
            "human" => Ok(ProvenanceSource::Human),
            "rule" => Ok(ProvenanceSource::Rule),
            "imported" => Ok(ProvenanceSource::Imported),
            "principal_authorized" => Ok(ProvenanceSource::PrincipalAuthorized),
            other => Err(AppError::BadRequest(format!(
                "invalid revision source '{other}'; expected one of ai|human|rule|imported|principal_authorized"
            ))),
        }
    }
}

// ── 写入结果 ──────────────────────────────────────────────────────────

/// `apply_chunk_revision` 成功返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionApplied {
    pub revision_id: String,
    pub chunk_id: String,
    pub op: String,
    pub before_hash: String,
    pub after_hash: String,
    /// 若内容 hash 未变（patch 全部命中既有值），返回 `unchanged=true`，
    /// 调用方可据此跳过 catalog rebuild enqueue。
    pub unchanged: bool,
}

// ── 入参 ──────────────────────────────────────────────────────────────

/// `apply_chunk_revision` 的入参。
///
/// `patch` 是 BSON `Document`，**仅含要变更的字段**。
/// `actor` / `reason` 写到 `chunk_revisions.created_by / reason` 用于审计追溯。
pub struct RevisionRequest {
    pub op: RevisionOp,
    pub source: ProvenanceSource,
    pub patch: Document,
    pub reason: Option<String>,
    pub actor: Option<String>,
}

/// Fields whose meaning can be served to the reply agent. Any ordinary edit
/// touching one of them invalidates a previous human verification. The edit is
/// still committed, but it returns to the existing draft -> review -> verify
/// lifecycle. Structural relation-only patches are intentionally excluded.
const REVIEW_SENSITIVE_PATCH_FIELDS: &[&str] = &[
    "title",
    "summary",
    "body",
    "knowledge_type",
    "business_context",
    "applicable_scenes",
    "not_applicable_scenes",
    "product_tags",
    "business_topics",
    "source_quote",
    "source_anchors",
    "domain_attributes",
];

pub(crate) fn chunk_patch_requires_review(op: RevisionOp, patch: &Document) -> bool {
    matches!(
        op,
        RevisionOp::Patch | RevisionOp::Split | RevisionOp::Merge | RevisionOp::Rollback
    ) && patch
        .keys()
        .any(|key| REVIEW_SENSITIVE_PATCH_FIELDS.contains(&key.as_str()))
}

/// Apply lifecycle fields after per-chunk locks have been enforced. Lifecycle
/// state is owned by the revision operation, so a user-configured lock cannot
/// keep a content edit verified or suppress archive/reject transitions.
fn apply_server_owned_lifecycle(
    merged: &mut Document,
    op: RevisionOp,
    source: ProvenanceSource,
    patch: &Document,
) {
    let requires_review =
        matches!(source, ProvenanceSource::Ai) || chunk_patch_requires_review(op, patch);
    if requires_review {
        merged.insert("status", "draft");
        merged.insert("integrity_status", "needs_review");
        merged.insert("confidence_score", 0i32);
        return;
    }

    match op {
        RevisionOp::Archive => {
            merged.insert("status", "archived");
        }
        RevisionOp::Restore => {
            merged.insert("status", "active");
        }
        RevisionOp::Verify | RevisionOp::Unverify | RevisionOp::Reject => {
            for key in ["status", "integrity_status", "confidence_score"] {
                if let Some(value) = patch.get(key) {
                    merged.insert(key, value.clone());
                }
            }
        }
        RevisionOp::Split | RevisionOp::Merge => {
            for key in [
                "status",
                "integrity_status",
                "confidence_score",
                "superseded_by",
                "previous_version_id",
            ] {
                if let Some(value) = patch.get(key) {
                    merged.insert(key, value.clone());
                }
            }
        }
        RevisionOp::Create | RevisionOp::Patch | RevisionOp::Rollback => {}
    }
}

/// Keep the chunk's durable knowledge origin on later edits. The immutable
/// revision records who/what performed each mutation; the main-row provenance
/// keeps where the knowledge originally came from while refreshing only the
/// last-edit metadata. Legacy rows without provenance are initialized from the
/// current revision source.
fn build_chunk_provenance(
    existing: &Document,
    source: ProvenanceSource,
    actor: Option<&str>,
    edited_at: DateTime,
) -> Document {
    let mut provenance = existing
        .get_document("provenance")
        .cloned()
        .unwrap_or_else(|_| doc! { "source": source.as_str() });
    provenance.insert("edited_at", edited_at);
    match actor {
        Some(actor) => {
            provenance.insert("edited_by", actor);
        }
        None => {
            provenance.remove("edited_by");
        }
    }
    provenance
}

struct PreparedChunkRevision {
    revision: ChunkRevision,
    /// Exact BSON that was hashed. Keeping the replacement as BSON preserves
    /// forward-compatible review fields that are not yet projected by the typed model.
    replacement: Option<Document>,
    replace_filter: Document,
    catalog_job: Option<CatalogRebuildJob>,
    applied: RevisionApplied,
}

fn map_revision_error(error: RevisionError) -> AppError {
    match error {
        RevisionError::LockedFieldInPatch { field } => {
            AppError::BadRequest(format!("字段 {field} 受锁定保护，不允许通过 patch 修改"))
        }
        RevisionError::BodyTruncated {
            old_len,
            new_len,
            threshold,
        } => AppError::BadRequest(format!(
            "新 body 长度 {new_len} 低于既有 {old_len} 的 {:.0}% 阈值；疑似 LLM 截断/偷懒，已拒收",
            threshold * 100.0
        )),
    }
}

fn prepare_chunk_revision(
    workspace_id: &str,
    chunk_object_id: ObjectId,
    existing_doc: &OperationKnowledgeChunk,
    existing_bson: Document,
    schema: Option<&DomainSchema>,
    req: RevisionRequest,
) -> AppResult<PreparedChunkRevision> {
    let chunk_id_hex = chunk_object_id.to_hex();
    let before_hash = compute_chunk_hash(&existing_bson);
    let after_patch = apply_field_patch(&existing_bson, &req.patch, DEFAULT_LOCKED_FIELDS)
        .map_err(map_revision_error)?;
    let merged = union_array_fields(
        &after_patch,
        &existing_bson,
        &req.patch,
        DEFAULT_UNION_ARRAY_KEYS,
    );

    // Protect every patched text field independently. A long summary must not
    // hide an accidentally truncated body (or vice versa).
    for field in ["body", "summary", "answer"] {
        if !req.patch.contains_key(field) {
            continue;
        }
        let old_len = text_field_len(&existing_bson, field);
        let new_len = text_field_len(&merged, field);
        let incoming_len = text_field_len(&req.patch, field);
        if is_body_truncated(old_len, incoming_len, new_len, BODY_TRUNCATION_THRESHOLD) {
            return Err(AppError::BadRequest(format!(
                "新 {field} 长度 {new_len} 低于既有 {old_len} 的 70% 阈值；疑似截断，已拒收。如确需缩短请通过明确的人工编辑流程",
            )));
        }
    }

    let effective_locked_owned = effective_locked_fields(&existing_bson);
    let effective_enforce_locked: Vec<&str> =
        effective_locked_owned.iter().map(|s| s.as_str()).collect();
    let mut merged = enforce_locked_fields(&merged, &existing_bson, &effective_enforce_locked);
    if let Some(schema) = schema {
        if let Ok(attrs) = merged.get_document("domain_attributes") {
            let enforced =
                crate::routes::domain_schemas::enforce_domain_attributes(schema, &attrs.clone())?;
            merged.insert("domain_attributes", Bson::Document(enforced));
        }
    }

    apply_server_owned_lifecycle(&mut merged, req.op, req.source, &req.patch);
    merged.insert(
        "provenance",
        build_chunk_provenance(
            &existing_bson,
            req.source,
            req.actor.as_deref(),
            DateTime::now(),
        ),
    );
    merged.insert(
        "updated_at",
        monotonic_chunk_updated_at(existing_doc.updated_at, DateTime::now()),
    );

    let after_hash = compute_chunk_hash(&merged);
    let unchanged = before_hash == after_hash;
    // Create is invoked after the derived row has been inserted. Even when its
    // content hash is unchanged, persist server-owned provenance/lifecycle and
    // enqueue its document catalog rebuild.
    let force_create_write = req.op == RevisionOp::Create;
    let revision_id = format!("rev_{}_{}", chunk_id_hex, uuid::Uuid::new_v4().simple());
    let after_snapshot = if unchanged && !force_create_write {
        existing_bson.clone()
    } else {
        merged.clone()
    };
    let revision = ChunkRevision {
        id: None,
        workspace_id: workspace_id.to_string(),
        chunk_id: chunk_id_hex.clone(),
        revision_id: revision_id.clone(),
        op: req.op.as_str().to_string(),
        patch: req.patch,
        before_hash: before_hash.clone(),
        after_hash: after_hash.clone(),
        before_snapshot: Some(existing_bson),
        after_snapshot: Some(after_snapshot),
        source: req.source.as_str().to_string(),
        reason: req.reason,
        created_at: DateTime::now(),
        created_by: req.actor,
    };
    let replacement = if unchanged && !force_create_write {
        None
    } else {
        // Validate every modeled field before persisting, but retain the exact BSON
        // used for after_hash so forward-compatible review fields are not discarded.
        let _: OperationKnowledgeChunk = mongodb::bson::from_document(merged.clone())
            .map_err(|e| AppError::External(format!("deserialize merged chunk failed: {e}")))?;
        Some(merged)
    };
    let catalog_job = if unchanged && !force_create_write {
        None
    } else {
        existing_doc
            .document_id
            .map(|document_id| CatalogRebuildJob {
                id: None,
                job_id: format!(
                    "crj_{}_{}",
                    document_id.to_hex(),
                    uuid::Uuid::new_v4().simple()
                ),
                workspace_id: workspace_id.to_string(),
                document_id,
                queued_at: DateTime::now(),
                target_generation: 0,
                status: "queued".to_string(),
                attempts: 0,
                claim_generation: 0,
                worker_id: None,
                claim_token: None,
                locked_until: None,
                next_retry_at: None,
                last_error: None,
                started_at: None,
                finished_at: None,
            })
    };
    Ok(PreparedChunkRevision {
        revision,
        replacement,
        replace_filter: chunk_replace_filter(
            chunk_object_id,
            workspace_id,
            existing_doc.updated_at,
        ),
        catalog_job,
        applied: RevisionApplied {
            revision_id,
            chunk_id: chunk_id_hex,
            op: req.op.as_str().to_string(),
            before_hash,
            after_hash,
            unchanged,
        },
    })
}

async fn persist_prepared_chunk_revision_with_session(
    db: &Database,
    prepared: PreparedChunkRevision,
    session: &mut ClientSession,
) -> AppResult<RevisionApplied> {
    let PreparedChunkRevision {
        revision,
        replacement,
        replace_filter,
        catalog_job,
        applied,
    } = prepared;
    db.chunk_revisions()
        .insert_one_with_session(revision, None, session)
        .await?;
    if let Some(replacement) = replacement {
        let replace_result = db
            .raw()
            .collection::<Document>("operation_knowledge_chunks")
            .replace_one_with_session(replace_filter, replacement, None, session)
            .await?;
        if replace_result.matched_count != 1 {
            return Err(AppError::Conflict("chunk_revision_conflict".to_string()));
        }
    }
    if let Some(job) = catalog_job {
        enqueue_catalog_job_with_session(db, job, session).await?;
    }
    Ok(applied)
}

/// Advance the parent document's desired catalog generation and persist the
/// matching durable intent in the caller's transaction. The explicit CAS keeps
/// composed split/merge operations monotonic and makes a missing parent fail
/// the whole knowledge mutation instead of silently losing projection work.
async fn enqueue_catalog_job_with_session(
    db: &Database,
    mut job: CatalogRebuildJob,
    session: &mut ClientSession,
) -> AppResult<()> {
    let parent = db
        .operation_knowledge_documents()
        .find_one_with_session(
            doc! {
                "_id": job.document_id,
                "workspace_id": &job.workspace_id,
            },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::Conflict("catalog_parent_missing".to_string()))?;
    let next_generation = parent
        .catalog_desired_generation
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("catalog_generation_exhausted".to_string()))?;
    let generation_filter = if parent.catalog_desired_generation == 0 {
        doc! {
            "$or": [
                { "catalog_desired_generation": 0i64 },
                { "catalog_desired_generation": { "$exists": false } },
                { "catalog_desired_generation": null },
            ]
        }
    } else {
        doc! { "catalog_desired_generation": parent.catalog_desired_generation }
    };
    let mut parent_filter = doc! {
        "_id": job.document_id,
        "workspace_id": &job.workspace_id,
    };
    parent_filter.extend(generation_filter);
    let advanced = db
        .operation_knowledge_documents()
        .update_one_with_session(
            parent_filter,
            doc! {
                "$set": { "catalog_desired_generation": next_generation },
            },
            None,
            session,
        )
        .await?;
    if advanced.matched_count != 1 {
        return Err(AppError::Conflict(
            "catalog_generation_conflict".to_string(),
        ));
    }
    job.target_generation = next_generation;
    db.catalog_rebuild_jobs()
        .insert_one_with_session(job, None, session)
        .await?;
    Ok(())
}

async fn unique_active_schema_with_session(
    db: &Database,
    workspace_id: &str,
    session: &mut ClientSession,
) -> AppResult<Option<DomainSchema>> {
    let mut cursor = db
        .domain_schemas()
        .find_with_session(
            doc! { "workspace_id": workspace_id, "is_active": true },
            FindOptions::builder().limit(2).build(),
            session,
        )
        .await?;
    let first = cursor.next(session).await.transpose()?;
    if cursor.next(session).await.transpose()?.is_some() {
        return Err(AppError::External(
            "multiple_active_domain_schemas".to_string(),
        ));
    }
    Ok(first)
}

pub(crate) async fn commit_chunk_transaction(session: &mut ClientSession) -> AppResult<()> {
    loop {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => {
                let _ = session.abort_transaction().await;
                return Err(error.into());
            }
        }
    }
}

pub(crate) fn map_chunk_transaction_error(error: AppError) -> AppError {
    match error {
        AppError::Db(db_error) if db_error.contains_label("TransientTransactionError") => {
            AppError::Conflict("chunk_revision_conflict".to_string())
        }
        other => other,
    }
}

/// Transaction-aware variant used by multi-object knowledge operations. The
/// caller owns transaction start/abort/commit and may compose several calls.
pub(crate) async fn apply_chunk_revision_with_session(
    db: &Database,
    workspace_id: &str,
    chunk_object_id: ObjectId,
    req: RevisionRequest,
    session: &mut ClientSession,
) -> AppResult<RevisionApplied> {
    let existing_bson = db
        .raw()
        .collection::<Document>("operation_knowledge_chunks")
        .find_one_with_session(
            doc! { "_id": chunk_object_id, "workspace_id": workspace_id },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    let existing_doc: OperationKnowledgeChunk = mongodb::bson::from_document(existing_bson.clone())
        .map_err(|error| {
            AppError::External(format!("deserialize existing chunk failed: {error}"))
        })?;
    let schema = unique_active_schema_with_session(db, workspace_id, session).await?;
    let prepared = prepare_chunk_revision(
        workspace_id,
        chunk_object_id,
        &existing_doc,
        existing_bson,
        schema.as_ref(),
        req,
    )?;
    persist_prepared_chunk_revision_with_session(db, prepared, session).await
}

const ROLLBACK_PRESERVED_FIELDS: &[&str] = &[
    "_id",
    "workspace_id",
    "account_id",
    "document_id",
    "item_id",
    "domain",
    "created_at",
    "wiki_type",
    "chunk_type",
    "locked_fields",
    "usage_stats",
    "dynamic_confidence",
    "integrity_score",
];

fn build_snapshot_rollback(
    current: &Document,
    target_before_snapshot: &Document,
    actor: &str,
) -> Document {
    let mut restored = target_before_snapshot.clone();
    for key in ROLLBACK_PRESERVED_FIELDS {
        match current.get(*key) {
            Some(value) => {
                restored.insert(*key, value.clone());
            }
            None => {
                restored.remove(*key);
            }
        }
    }
    // A historical snapshot must not bypass locks configured after that
    // revision. Restore locked values from the current row first; lifecycle
    // fields are then deliberately re-owned by the server below.
    let effective_locked = effective_locked_fields(current);
    let locked_refs: Vec<&str> = effective_locked.iter().map(String::as_str).collect();
    let mut restored = enforce_locked_fields(&restored, current, &locked_refs);
    restored.insert("status", "draft");
    restored.insert("integrity_status", "needs_review");
    restored.insert("confidence_score", 0_i32);
    restored.insert(
        "provenance",
        build_chunk_provenance(
            current,
            ProvenanceSource::Human,
            Some(actor),
            DateTime::now(),
        ),
    );
    let current_updated_at = current
        .get_datetime("updated_at")
        .copied()
        .unwrap_or_else(|_| DateTime::from_millis(0));
    restored.insert(
        "updated_at",
        monotonic_chunk_updated_at(current_updated_at, DateTime::now()),
    );
    restored
}

fn enforce_snapshot_domain_attributes(
    restored: &mut Document,
    schema: &DomainSchema,
) -> AppResult<()> {
    let had_attributes = restored.contains_key("domain_attributes");
    let attributes = match restored.get("domain_attributes") {
        Some(Bson::Document(attributes)) => attributes.clone(),
        Some(Bson::Null) | None => Document::new(),
        Some(_) => {
            return Err(AppError::Conflict(
                "chunk_revision_snapshot_invalid_domain_attributes".to_string(),
            ));
        }
    };
    let enforced = crate::routes::domain_schemas::enforce_domain_attributes(schema, &attributes)?;
    if had_attributes || !enforced.is_empty() {
        restored.insert("domain_attributes", Bson::Document(enforced));
    } else {
        restored.remove("domain_attributes");
    }
    Ok(())
}

pub(crate) async fn rollback_chunk_revision_with_session(
    db: &Database,
    workspace_id: &str,
    chunk_object_id: ObjectId,
    target_revision_id: &str,
    actor: &str,
    reason: Option<String>,
    session: &mut ClientSession,
) -> AppResult<RevisionApplied> {
    let chunk_id = chunk_object_id.to_hex();
    let target = db
        .chunk_revisions()
        .find_one_with_session(
            doc! {
                "workspace_id": workspace_id,
                "chunk_id": &chunk_id,
                "revision_id": target_revision_id,
            },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("revision {target_revision_id} not found")))?;
    let target_snapshot = target
        .before_snapshot
        .ok_or_else(|| AppError::Conflict("chunk_revision_snapshot_unavailable".to_string()))?;
    let current_bson = db
        .raw()
        .collection::<Document>("operation_knowledge_chunks")
        .find_one_with_session(
            doc! { "_id": chunk_object_id, "workspace_id": workspace_id },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    let current: OperationKnowledgeChunk = mongodb::bson::from_document(current_bson.clone())
        .map_err(|error| AppError::Conflict(format!("chunk_revision_snapshot_invalid: {error}")))?;
    let mut restored = build_snapshot_rollback(&current_bson, &target_snapshot, actor);
    if let Some(schema) = unique_active_schema_with_session(db, workspace_id, session).await? {
        enforce_snapshot_domain_attributes(&mut restored, &schema)?;
    }
    let _: OperationKnowledgeChunk = mongodb::bson::from_document(restored.clone())
        .map_err(|error| AppError::Conflict(format!("chunk_revision_snapshot_invalid: {error}")))?;
    let replacement = restored.clone();
    let before_hash = compute_chunk_hash(&current_bson);
    let after_hash = compute_chunk_hash(&restored);
    let revision_id = format!("rev_{}_{}", chunk_id, uuid::Uuid::new_v4().simple());
    let revision = ChunkRevision {
        id: None,
        workspace_id: workspace_id.to_string(),
        chunk_id: chunk_id.clone(),
        revision_id: revision_id.clone(),
        op: RevisionOp::Rollback.as_str().to_string(),
        patch: doc! { "rollback_to_revision": target_revision_id },
        before_hash: before_hash.clone(),
        after_hash: after_hash.clone(),
        before_snapshot: Some(current_bson),
        after_snapshot: Some(restored),
        source: ProvenanceSource::Human.as_str().to_string(),
        reason: reason.or_else(|| Some(format!("rollback to revision {target_revision_id}"))),
        created_at: DateTime::now(),
        created_by: Some(actor.to_string()),
    };
    let catalog_job = current.document_id.map(|document_id| CatalogRebuildJob {
        id: None,
        job_id: format!(
            "crj_{}_{}",
            document_id.to_hex(),
            uuid::Uuid::new_v4().simple()
        ),
        workspace_id: workspace_id.to_string(),
        document_id,
        queued_at: DateTime::now(),
        target_generation: 0,
        status: "queued".to_string(),
        attempts: 0,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        next_retry_at: None,
        last_error: None,
        started_at: None,
        finished_at: None,
    });
    persist_prepared_chunk_revision_with_session(
        db,
        PreparedChunkRevision {
            revision,
            replacement: Some(replacement),
            replace_filter: chunk_replace_filter(chunk_object_id, workspace_id, current.updated_at),
            catalog_job,
            applied: RevisionApplied {
                revision_id,
                chunk_id,
                op: RevisionOp::Rollback.as_str().to_string(),
                before_hash: before_hash.clone(),
                after_hash: after_hash.clone(),
                unchanged: before_hash == after_hash,
            },
        },
        session,
    )
    .await
}

// ── 主入口 ────────────────────────────────────────────────────────────

/// 三层保护下的 chunk 写入入口。
///
/// 步骤：
/// 1. `find_one` 既有 chunk（`workspace_id` 守门，跨 workspace 写入 → NotFound）；
/// 2. `apply_field_patch` 对 patch 的顶层 key 做锁定字段守门（含 patch 拒收）；
/// 3. `union_array_fields` 对默认数组字段做 existing ∪ patch；
/// 4. body/answer/summary 长度 < existing × 70% → 拒收；
/// 5. AI source 强制 draft + needs_review；
/// 6. `enforce_locked_fields` 末次防线；
/// 7. 同一 Mongo 事务内写 revision + 以 updated_at CAS 替换 chunk；
/// 8. 同一事务推进父文档 catalog generation 并写 durable rebuild intent。
pub async fn apply_chunk_revision(
    db: &Database,
    workspace_id: &str,
    chunk_object_id: ObjectId,
    req: RevisionRequest,
) -> AppResult<RevisionApplied> {
    let mut session = db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result =
        apply_chunk_revision_with_session(db, workspace_id, chunk_object_id, req, &mut session)
            .await;
    let applied = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(map_chunk_transaction_error(error));
        }
    };
    commit_chunk_transaction(&mut session)
        .await
        .map_err(map_chunk_transaction_error)?;

    Ok(applied)
}

// ── 帮手：字段级 text payload 长度 ────────────────────────────────

fn text_field_len(document: &Document, field: &str) -> usize {
    document
        .get_str(field)
        .ok()
        .map(|value| value.chars().count())
        .unwrap_or(0)
}

// ── 删除级联：normalize_ref_key / cleanup_dangling_refs ───────────────

/// 把 chunk 引用 key 规范化（防 substring 误伤："openai" 不应匹配 "ai"）。
///
/// 借鉴 LLW `wiki-cleanup.ts:49-130`：
/// - 去 `.md` 扩展名；
/// - 取末段（按 `/` 分割），避免 path 前缀干扰；
/// - 全部小写；
/// - 去除 ASCII 空格 / 短横 / 下划线。
pub fn normalize_ref_key(s: &str) -> String {
    let leaf = s.trim_end_matches(".md").rsplit('/').next().unwrap_or(s);
    leaf.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect()
}

/// chunk 删除（archive）后清理其它 chunk 中指向它的 `related_chunks` 条目。
///
/// 实现：
/// 1. 查同 workspace 所有 chunks，遍历其 `related_chunks: Vec<RelatedRef>`；
/// 2. 命中 `chunk_id == archived_id`（或 normalize_ref_key 等价）→ 移除；
/// 3. 每条受影响的 chunk 自己也走 `apply_chunk_revision(op=Patch, source=Rule,
///    reason="cleanup_dangling_refs")`，留追溯。
///
/// 失败不冒泡 —— archive 主动作已成，cleanup 仅 best-effort。
pub async fn cleanup_dangling_refs(
    db: &Database,
    workspace_id: &str,
    archived_chunk_id_hex: &str,
) -> AppResult<usize> {
    use futures::TryStreamExt;
    let normalized_target = normalize_ref_key(archived_chunk_id_hex);
    let mut cursor = db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "related_chunks": { "$exists": true, "$ne": [] }
            },
            None,
        )
        .await?;
    let mut affected = 0usize;
    while let Some(chunk) = cursor.try_next().await? {
        let related = match chunk.related_chunks.clone() {
            Some(r) => r,
            None => continue,
        };
        let kept: Vec<_> = related
            .into_iter()
            .filter(|r| {
                let by_id = r.chunk_id == archived_chunk_id_hex;
                let by_norm = normalize_ref_key(&r.chunk_id) == normalized_target;
                !(by_id || by_norm)
            })
            .collect();
        if kept.len() == chunk.related_chunks.as_ref().map(|v| v.len()).unwrap_or(0) {
            continue;
        }
        // 写一条 patch revision 留痕迹
        let chunk_oid = match chunk.id {
            Some(o) => o,
            None => continue,
        };
        let patch = doc! {
            "related_chunks": mongodb::bson::to_bson(&kept).unwrap_or(Bson::Null),
        };
        // 注意：related_chunks 不在 DEFAULT_UNION_ARRAY_KEYS（结构数组按 chunk_id
        // 去重才正确，简单 string union 不适用），所以这里直接 patch 整数组。
        let req = RevisionRequest {
            op: RevisionOp::Patch,
            source: ProvenanceSource::Rule,
            patch,
            reason: Some(format!(
                "cleanup_dangling_refs: archived chunk {archived_chunk_id_hex}"
            )),
            actor: Some("system:cleanup_worker".to_string()),
        };
        match apply_chunk_revision(db, workspace_id, chunk_oid, req).await {
            Ok(_) => affected += 1,
            Err(err) => {
                tracing::warn!(
                    chunk_id = %chunk_oid.to_hex(),
                    error = %err,
                    "cleanup_dangling_refs apply_chunk_revision failed (non-fatal)"
                );
            }
        }
    }
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_replace_filter_carries_expected_updated_at() {
        let chunk_id = ObjectId::new();
        let expected = DateTime::from_millis(1_700_000_000_123);
        let filter = chunk_replace_filter(chunk_id, "ws_a", expected);

        assert_eq!(filter.get_object_id("_id").unwrap(), chunk_id);
        assert_eq!(filter.get_str("workspace_id").unwrap(), "ws_a");
        assert_eq!(filter.get_datetime("updated_at").unwrap(), &expected);
    }

    #[test]
    fn chunk_updated_at_is_strictly_monotonic_with_millisecond_precision() {
        let expected = DateTime::from_millis(1_700_000_000_123);

        assert_eq!(
            monotonic_chunk_updated_at(expected, expected).timestamp_millis(),
            expected.timestamp_millis() + 1
        );
        assert_eq!(
            monotonic_chunk_updated_at(
                expected,
                DateTime::from_millis(expected.timestamp_millis() - 10),
            )
            .timestamp_millis(),
            expected.timestamp_millis() + 1
        );
        assert_eq!(
            monotonic_chunk_updated_at(
                expected,
                DateTime::from_millis(expected.timestamp_millis() + 10),
            )
            .timestamp_millis(),
            expected.timestamp_millis() + 10
        );
    }

    #[test]
    fn normalize_strips_ext_path_and_punctuation() {
        assert_eq!(normalize_ref_key("OpenAI"), "openai");
        assert_eq!(normalize_ref_key("docs/ai_lab.md"), "ailab");
        assert_eq!(normalize_ref_key("a-b_c"), "abc");
    }

    #[test]
    fn normalize_does_not_substring_match_openai_to_ai() {
        // 关键安全保证：normalize 后 "openai" != "ai"
        assert_ne!(normalize_ref_key("openai"), normalize_ref_key("ai"));
    }

    #[test]
    fn revision_op_round_trip() {
        for (op, s) in [
            (RevisionOp::Create, "create"),
            (RevisionOp::Patch, "patch"),
            (RevisionOp::Split, "split"),
            (RevisionOp::Merge, "merge"),
            (RevisionOp::Rollback, "rollback"),
            (RevisionOp::Archive, "archive"),
            (RevisionOp::Restore, "restore"),
            (RevisionOp::Verify, "verify"),
            (RevisionOp::Unverify, "unverify"),
            (RevisionOp::Reject, "reject"),
        ] {
            assert_eq!(op.as_str(), s);
        }
    }

    #[test]
    fn provenance_source_round_trip() {
        assert_eq!(ProvenanceSource::Ai.as_str(), "ai");
        assert_eq!(
            ProvenanceSource::from_str("imported").unwrap().as_str(),
            "imported"
        );
        assert!(ProvenanceSource::from_str("evil").is_err());
    }

    #[test]
    fn provenance_principal_authorized_roundtrip() {
        assert_eq!(
            ProvenanceSource::PrincipalAuthorized.as_str(),
            "principal_authorized"
        );
        assert_eq!(
            "principal_authorized".parse::<ProvenanceSource>().unwrap(),
            ProvenanceSource::PrincipalAuthorized
        );
    }

    #[test]
    fn later_revision_preserves_origin_evidence_and_updates_editor() {
        let original_time = DateTime::from_millis(1_600_000_000_000);
        let edit_time = DateTime::from_millis(1_700_000_000_000);
        let existing = doc! {
            "provenance": {
                "source": "imported",
                "source_doc_id": "doc-42",
                "source_quote": "原始引文",
                "llm_model_alias": "provider-a",
                "edited_at": original_time,
                "edited_by": "import-worker",
            }
        };

        let provenance = build_chunk_provenance(
            &existing,
            ProvenanceSource::Human,
            Some("operator"),
            edit_time,
        );

        assert_eq!(provenance.get_str("source").unwrap(), "imported");
        assert_eq!(provenance.get_str("source_doc_id").unwrap(), "doc-42");
        assert_eq!(provenance.get_str("source_quote").unwrap(), "原始引文");
        assert_eq!(provenance.get_str("llm_model_alias").unwrap(), "provider-a");
        assert_eq!(provenance.get_datetime("edited_at").unwrap(), &edit_time);
        assert_eq!(provenance.get_str("edited_by").unwrap(), "operator");
    }

    #[test]
    fn revision_preserves_unmodeled_review_fields_in_hashed_replacement() {
        let chunk_id = ObjectId::new();
        let existing = OperationKnowledgeChunk {
            id: Some(chunk_id),
            workspace_id: "ws-review".to_string(),
            domain: "user_operations".to_string(),
            title: "reviewed fact".to_string(),
            status: "draft".to_string(),
            integrity_status: Some("needs_review".to_string()),
            ..OperationKnowledgeChunk::default()
        };
        let existing_bson = mongodb::bson::to_document(&existing).unwrap();
        let prepared = prepare_chunk_revision(
            "ws-review",
            chunk_id,
            &existing,
            existing_bson,
            None,
            RevisionRequest {
                op: RevisionOp::Verify,
                source: ProvenanceSource::Human,
                patch: doc! {
                    "integrity_status": "verified",
                    "status": "active",
                    "verified_claims": ["claim-a"],
                    "unsupported_claims": [],
                },
                reason: None,
                actor: Some("operator".to_string()),
            },
        )
        .unwrap();

        let replacement = prepared.replacement.expect("replacement");
        assert_eq!(
            replacement.get_array("verified_claims").unwrap(),
            &vec![Bson::String("claim-a".to_string())]
        );
        assert_eq!(
            prepared.applied.after_hash,
            compute_chunk_hash(&replacement)
        );
        assert_eq!(
            prepared.revision.after_snapshot.as_ref().unwrap(),
            &replacement,
            "审计快照、哈希输入和实际 replacement 必须是同一 BSON"
        );
    }

    #[test]
    fn revision_initializes_missing_provenance_without_fake_editor() {
        let edit_time = DateTime::from_millis(1_700_000_000_000);
        let provenance = build_chunk_provenance(
            &Document::new(),
            ProvenanceSource::Imported,
            None,
            edit_time,
        );

        assert_eq!(provenance.get_str("source").unwrap(), "imported");
        assert_eq!(provenance.get_datetime("edited_at").unwrap(), &edit_time);
        assert!(!provenance.contains_key("edited_by"));
    }

    #[test]
    fn content_patch_requires_review_but_relation_patch_does_not() {
        assert!(chunk_patch_requires_review(
            RevisionOp::Patch,
            &doc! { "summary": "changed" }
        ));
        assert!(chunk_patch_requires_review(
            RevisionOp::Merge,
            &doc! { "body": "merged content" }
        ));
        assert!(!chunk_patch_requires_review(
            RevisionOp::Patch,
            &doc! { "related_chunks": [] }
        ));
        assert!(!chunk_patch_requires_review(
            RevisionOp::Verify,
            &doc! { "integrity_status": "verified", "status": "active" }
        ));
    }

    #[test]
    fn server_lifecycle_overrides_locked_verified_state_after_content_edit() {
        let mut merged = doc! {
            "status": "active",
            "integrity_status": "verified",
            "confidence_score": 100,
        };
        apply_server_owned_lifecycle(
            &mut merged,
            RevisionOp::Patch,
            ProvenanceSource::Human,
            &doc! { "summary": "changed" },
        );
        assert_eq!(merged.get_str("status").unwrap(), "draft");
        assert_eq!(merged.get_str("integrity_status").unwrap(), "needs_review");
        assert_eq!(merged.get_i32("confidence_score").unwrap(), 0);
    }

    #[test]
    fn rule_verify_keeps_requested_human_audit_state() {
        let patch = doc! {
            "integrity_status": "needs_human_audit",
            "confidence_score": 73,
        };
        let mut merged = doc! {
            "status": "draft",
            "integrity_status": "verified",
            "confidence_score": 100,
        };
        apply_server_owned_lifecycle(
            &mut merged,
            RevisionOp::Verify,
            ProvenanceSource::Rule,
            &patch,
        );
        assert_eq!(
            merged.get_str("integrity_status").unwrap(),
            "needs_human_audit"
        );
        assert_eq!(merged.get_i32("confidence_score").unwrap(), 73);
    }

    #[test]
    fn ai_verify_operation_cannot_promote_knowledge() {
        let mut merged = doc! { "status": "active", "integrity_status": "verified" };
        apply_server_owned_lifecycle(
            &mut merged,
            RevisionOp::Verify,
            ProvenanceSource::Ai,
            &doc! {
                "status": "active",
                "integrity_status": "verified",
                "confidence_score": 100,
            },
        );
        assert_eq!(merged.get_str("status").unwrap(), "draft");
        assert_eq!(merged.get_str("integrity_status").unwrap(), "needs_review");
        assert_eq!(merged.get_i32("confidence_score").unwrap(), 0);
    }

    #[test]
    fn snapshot_rollback_restores_content_but_preserves_server_owned_state() {
        let current_updated_at = DateTime::from_millis(1_700_000_000_100);
        let current = doc! {
            "_id": ObjectId::new(),
            "workspace_id": "ws_current",
            "account_id": "account_current",
            "document_id": ObjectId::new(),
            "domain": "user_operations",
            "title": "current title",
            "body": "current body",
            "product_tags": ["old", "new"],
            "status": "active",
            "integrity_status": "verified",
            "confidence_score": 100,
            "locked_fields": ["body"],
            "usage_stats": { "hit_count_30d": 42_i32 },
            "provenance": {
                "source": "imported",
                "source_doc_id": "doc-current",
                "source_quote": "当前来源证据",
                "edited_at": DateTime::from_millis(1_650_000_000_000),
                "edited_by": "import-worker",
            },
            "updated_at": current_updated_at,
        };
        let historical = doc! {
            "_id": ObjectId::new(),
            "workspace_id": "ws_stale",
            "account_id": "account_stale",
            "domain": "stale_domain",
            "title": "historical title",
            "body": "historical body",
            "product_tags": ["old"],
            "status": "active",
            "integrity_status": "verified",
            "confidence_score": 100,
            "updated_at": DateTime::from_millis(1_600_000_000_000),
        };

        let restored = build_snapshot_rollback(&current, &historical, "operator");

        assert_eq!(restored.get_str("workspace_id").unwrap(), "ws_current");
        assert_eq!(restored.get_str("account_id").unwrap(), "account_current");
        assert_eq!(restored.get_str("domain").unwrap(), "user_operations");
        assert_eq!(restored.get_str("title").unwrap(), "historical title");
        assert_eq!(
            restored.get_str("body").unwrap(),
            "current body",
            "a current per-chunk lock must prevent snapshot rollback from changing body"
        );
        assert_eq!(
            restored.get_array("product_tags").unwrap(),
            &vec![Bson::String("old".to_string())]
        );
        assert_eq!(restored.get_str("status").unwrap(), "draft");
        assert_eq!(
            restored.get_str("integrity_status").unwrap(),
            "needs_review"
        );
        assert_eq!(restored.get_i32("confidence_score").unwrap(), 0);
        assert_eq!(
            restored
                .get_document("usage_stats")
                .unwrap()
                .get_i32("hit_count_30d")
                .unwrap(),
            42
        );
        assert_eq!(
            restored
                .get_document("provenance")
                .unwrap()
                .get_str("edited_by")
                .unwrap(),
            "operator"
        );
        assert_eq!(
            restored
                .get_document("provenance")
                .unwrap()
                .get_str("source")
                .unwrap(),
            "imported"
        );
        assert_eq!(
            restored
                .get_document("provenance")
                .unwrap()
                .get_str("source_doc_id")
                .unwrap(),
            "doc-current"
        );
        assert!(
            restored
                .get_datetime("updated_at")
                .unwrap()
                .timestamp_millis()
                > current_updated_at.timestamp_millis()
        );
    }

    #[test]
    fn snapshot_rollback_validates_missing_attributes_against_current_schema() {
        let schema = DomainSchema {
            id: None,
            schema_id: "required-stage".to_string(),
            workspace_id: "ws_current".to_string(),
            name: "required stage".to_string(),
            version: 1,
            fields: vec![crate::models::DomainField {
                name: "stage".to_string(),
                label: "Stage".to_string(),
                kind: "string".to_string(),
                required: true,
                allowed_values: None,
                alias_of: None,
            }],
            alias_dict: Document::new(),
            guard_dsl: None,
            is_active: true,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        let mut restored = doc! { "title": "historical title" };

        let error = enforce_snapshot_domain_attributes(&mut restored, &schema)
            .expect_err("missing required attribute must fail closed");

        assert!(matches!(error, AppError::BadRequest(_)));
        assert!(!restored.contains_key("domain_attributes"));
    }
}

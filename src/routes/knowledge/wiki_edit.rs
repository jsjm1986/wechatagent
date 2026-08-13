//! 运营知识库 wiki 编辑：切片 patch/archive/restore/rollback/split/merge/relate + 批量核验/归档 + 引用查询。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use mongodb::options::{FindOptions, TransactionOptions};
use mongodb::{
    bson::{doc, Bson, DateTime, Document},
    ClientSession,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};

use super::super::shared::*;
use super::super::AppState;
use super::verify::{parse_verify_expected_updated_at, verify_chunk_at_version};
use super::*;

// ──────────────────────────────────────────────────────────────────────
// knowledge-wiki Phase C: 7 个 chunk 编辑路由 + 1 个删除级联包装
// ──────────────────────────────────────────────────────────────────────
//
// 全部走 `crate::knowledge_wiki::chunk_revisions::apply_chunk_revision`：
// 1) 锁定字段守门（patch 含 chunk_id/wiki_type/source_anchor/... → 4xx）
// 2) 数组字段 union（应用层完成，零 LLM 风险）
// 3) 70% body 长度阈值（LLM 截断/偷懒拒收）
// 4) AI source 强制 status=draft + integrity_status=needs_review
// 5) 同一事务写 chunk_revisions + chunks，并以 updated_at CAS 防并发覆盖
// 6) 同一事务推进父 Document catalog generation 并写 durable rebuild intent

use crate::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, apply_chunk_revision_with_session, commit_chunk_transaction,
    rollback_chunk_revision_with_session, ProvenanceSource, RevisionApplied, RevisionOp,
    RevisionRequest,
};
use crate::knowledge_wiki::page_merge::effective_locked_fields;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::routes) struct ChunkPatchRequest {
    /// Public camelCase content patch. Scope, lifecycle and review fields are
    /// rejected by `normalize_editable_chunk_patch`.
    pub patch: Value,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkArchiveRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkRollbackRequest {
    pub reason: Option<String>,
}

fn revision_applied_to_json(r: &RevisionApplied) -> Value {
    json!({
        "ok": true,
        "revisionId": r.revision_id,
        "chunkId": r.chunk_id,
        "op": r.op,
        "beforeHash": r.before_hash,
        "afterHash": r.after_hash,
        "unchanged": r.unchanged,
    })
}

/// `POST /operation-knowledge/chunks/:id/patch` — 字段级 patch。
pub(in crate::routes) async fn patch_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkPatchRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let applied = apply_controlled_chunk_patch(
        &state,
        &admin.current_workspace,
        &admin.username,
        object_id,
        &payload.patch,
        ProvenanceSource::Human,
        payload.reason,
    )
    .await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "patch",
        &admin.username,
    );
    Ok(Json(revision_applied_to_json(&applied)))
}

/// `POST /operation-knowledge/chunks/:id/archive` — 软归档。
/// Relations remain intact so archived knowledge and its immutable revisions
/// stay traceable; only a future privileged purge may remove graph edges.
pub(in crate::routes) async fn archive_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkArchiveRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let req = RevisionRequest {
        op: RevisionOp::Archive,
        source: ProvenanceSource::Human,
        patch: Document::new(),
        reason: payload.reason,
        actor: Some(admin.username.clone()),
    };
    let applied = apply_chunk_revision(&state.db, &admin.current_workspace, object_id, req).await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "archive",
        &admin.username,
    );
    Ok(Json(revision_applied_to_json(&applied)))
}

/// `POST /operation-knowledge/chunks/:id/restore` — 取消 archive。
pub(in crate::routes) async fn restore_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkArchiveRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let req = RevisionRequest {
        op: RevisionOp::Restore,
        source: ProvenanceSource::Human,
        patch: Document::new(),
        reason: payload.reason,
        actor: Some(admin.username.clone()),
    };
    let applied = apply_chunk_revision(&state.db, &admin.current_workspace, object_id, req).await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "restore",
        &admin.username,
    );
    Ok(Json(revision_applied_to_json(&applied)))
}

/// `POST /operation-knowledge/chunks/:id/rollback/:revision_id` restores the
/// exact state immediately before the selected revision. Identity, tenant,
/// lock and runtime-stat fields remain server-owned, and restored content must
/// be reviewed again. Legacy revisions without snapshots fail closed.
pub(in crate::routes) async fn rollback_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path((id, revision_id)): Path<(String, String)>,
    Json(payload): Json<ChunkRollbackRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let result = rollback_chunk_revision_with_session(
        &state.db,
        &admin.current_workspace,
        object_id,
        &revision_id,
        &admin.username,
        payload.reason,
        &mut session,
    )
    .await;
    let applied = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    };
    commit_chunk_transaction(&mut session).await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "rollback",
        &admin.username,
    );
    let mut value = revision_applied_to_json(&applied);
    if let Some(o) = value.as_object_mut() {
        o.insert("rollbackTo".to_string(), json!(revision_id));
    }
    Ok(Json(value))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkRevisionsQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// `GET /operation-knowledge/chunks/:id/revisions` — 分页拉取编辑历史。
///
/// 长字段（patch 内的 body / answer 等）在响应里保留原文；前端长 body 自行 mask。
pub(in crate::routes) async fn list_operation_knowledge_chunk_revisions(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Query(query): Query<ChunkRevisionsQuery>,
) -> AppResult<Json<Value>> {
    use futures::TryStreamExt;
    let object_id = parse_object_id(&id)?;
    // 多租户隔离：父 chunk 授权与 revision 自身 workspace filter 双重收口。
    state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    let limit = query.limit.unwrap_or(20).clamp(1, 200) as i64;
    let skip = query.offset.unwrap_or(0) as u64;
    let opts = FindOptions::builder()
        .sort(doc! { "created_at": -1 })
        .limit(limit)
        .skip(skip)
        .build();
    let revisions: Vec<_> = state
        .db
        .chunk_revisions()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "chunk_id": object_id.to_hex(),
            },
            opts,
        )
        .await?
        .try_collect()
        .await?;
    let items: Vec<Value> = revisions
        .iter()
        .map(|r| {
            json!({
                "revisionId": r.revision_id,
                "chunkId": r.chunk_id,
                "op": r.op,
                "patch": mongodb::bson::Bson::Document(r.patch.clone()).into_canonical_extjson(),
                "beforeHash": r.before_hash,
                "afterHash": r.after_hash,
                "source": r.source,
                "reason": r.reason,
                "createdAt": r.created_at.to_string(),
                "createdBy": r.created_by,
            })
        })
        .collect();
    Ok(Json(json!({
        "items": items,
        "limit": limit,
        "offset": skip,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::routes) struct ChunkSplitRequest {
    /// Unicode character offset in the existing body (or summary fallback).
    /// The server derives both child chunks and all scope fields.
    pub offset: usize,
    pub reason: Option<String>,
}

/// `POST /operation-knowledge/chunks/:id/split` — 拆分 chunk。
///
/// The caller supplies only a character offset. Both children inherit scope and
/// classification from the source and always start as draft+needs_review. The
/// source is archived only after both children and their revisions exist.
async fn split_operation_knowledge_chunk_in_transaction(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    object_id: mongodb::bson::oid::ObjectId,
    payload: ChunkSplitRequest,
    session: &mut ClientSession,
) -> AppResult<(RevisionApplied, Vec<String>)> {
    let workspace_id = &admin.current_workspace;
    let original = state
        .db
        .operation_knowledge_chunks()
        .find_one_with_session(
            doc! { "_id": object_id, "workspace_id": workspace_id },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    if original.status == "archived" {
        return Err(AppError::BadRequest(
            "archived chunk cannot be split".to_string(),
        ));
    }
    let text = original
        .body
        .as_deref()
        .or(original.summary.as_deref())
        .unwrap_or_default();
    let char_count = text.chars().count();
    if payload.offset == 0 || payload.offset >= char_count {
        return Err(AppError::BadRequest(format!(
            "offset must be between 1 and {} Unicode characters",
            char_count.saturating_sub(1)
        )));
    }
    let left: String = text.chars().take(payload.offset).collect();
    let right: String = text.chars().skip(payload.offset).collect();
    if left.trim().is_empty() || right.trim().is_empty() {
        return Err(AppError::BadRequest(
            "split would create an empty chunk".to_string(),
        ));
    }

    // Create both derived chunks first. A failure leaves the original active.
    let mut new_ids: Vec<String> = Vec::new();
    for (index, body) in [left.trim(), right.trim()].into_iter().enumerate() {
        let now = DateTime::now();
        let mut child = original.clone();
        child.id = None;
        child.title = format!("{}（{}/2）", original.title, index + 1);
        child.body = Some(body.to_string());
        child.summary = None;
        // The original quote/anchors proved the original claim set. Once the
        // body is split they cannot be assumed to prove either derived child.
        // Fresh evidence must be supplied before the dedicated verify route can
        // promote a child again.
        child.source_quote = None;
        child.source_anchors.clear();
        child.status = "draft".to_string();
        child.integrity_status = Some("needs_review".to_string());
        child.confidence_score = Some(0);
        child.created_at = now;
        child.updated_at = now;
        child.previous_version_id = Some(object_id.to_hex());
        child.superseded_by = None;
        child.related_chunks = None;
        child.usage_stats = None;
        child.dynamic_confidence = None;
        child.integrity_score = None;
        let inserted = state
            .db
            .operation_knowledge_chunks()
            .insert_one_with_session(child, None, session)
            .await?;
        let oid = inserted.inserted_id.as_object_id().ok_or_else(|| {
            AppError::External("split child insert did not return ObjectId".to_string())
        })?;
        apply_chunk_revision_with_session(
            &state.db,
            workspace_id,
            oid,
            RevisionRequest {
                op: RevisionOp::Create,
                source: ProvenanceSource::Human,
                patch: Document::new(),
                reason: Some(format!("split from chunk {}", object_id.to_hex())),
                actor: Some(admin.username.clone()),
            },
            session,
        )
        .await?;
        new_ids.push(oid.to_hex());
    }

    let archived = apply_chunk_revision_with_session(
        &state.db,
        workspace_id,
        object_id,
        RevisionRequest {
            op: RevisionOp::Split,
            source: ProvenanceSource::Human,
            patch: doc! { "status": "archived" },
            reason: payload
                .reason
                .or_else(|| Some(format!("split at {}", payload.offset))),
            actor: Some(admin.username.clone()),
        },
        session,
    )
    .await?;
    Ok((archived, new_ids))
}

pub(in crate::routes) async fn split_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkSplitRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let result = split_operation_knowledge_chunk_in_transaction(
        &state,
        &admin,
        object_id,
        payload,
        &mut session,
    )
    .await;
    let (archived, new_ids) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    };
    commit_chunk_transaction(&mut session).await?;

    for chunk_id in &new_ids {
        super::super::chunk_locks::broadcast_chunk_revised_in(
            &state,
            &admin.current_workspace,
            chunk_id,
            "create",
            &admin.username,
        );
    }
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &archived.chunk_id,
        "split",
        &admin.username,
    );
    Ok(Json(json!({
        "ok": true,
        "archived": revision_applied_to_json(&archived),
        "newChunkIds": new_ids,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::routes) struct ChunkMergeRequest {
    /// Existing target chunk. Both source and target must have identical domain
    /// and account scope; the target owns all resulting scope fields.
    pub target_id: String,
    pub reason: Option<String>,
}

async fn merge_operation_knowledge_chunk_in_transaction(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    object_id: mongodb::bson::oid::ObjectId,
    payload: ChunkMergeRequest,
    session: &mut ClientSession,
) -> AppResult<(RevisionApplied, RevisionApplied)> {
    let target_id = parse_object_id(&payload.target_id)?;
    let workspace_id = &admin.current_workspace;
    if object_id == target_id {
        return Err(AppError::BadRequest(
            "cannot merge a chunk into itself".to_string(),
        ));
    }
    let source = state
        .db
        .operation_knowledge_chunks()
        .find_one_with_session(
            doc! { "_id": object_id, "workspace_id": workspace_id },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("source chunk not found".to_string()))?;
    let target = state
        .db
        .operation_knowledge_chunks()
        .find_one_with_session(
            doc! { "_id": target_id, "workspace_id": workspace_id },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("target chunk not found".to_string()))?;
    if source.status == "archived" || target.status == "archived" {
        return Err(AppError::BadRequest(
            "archived chunks cannot participate in merge".to_string(),
        ));
    }
    if source.domain != target.domain || source.account_id != target.account_id {
        return Err(AppError::BadRequest(
            "merge target must have the same domain and account scope".to_string(),
        ));
    }

    fn join_distinct(first: Option<&str>, second: Option<&str>) -> Option<String> {
        let first = first.map(str::trim).filter(|s| !s.is_empty());
        let second = second.map(str::trim).filter(|s| !s.is_empty());
        match (first, second) {
            (Some(a), Some(b)) if a == b => Some(a.to_string()),
            (Some(a), Some(b)) => Some(format!("{a}\n\n{b}")),
            (Some(a), None) | (None, Some(a)) => Some(a.to_string()),
            (None, None) => None,
        }
    }
    let mut target_patch = Document::new();
    if let Some(body) = join_distinct(target.body.as_deref(), source.body.as_deref()) {
        target_patch.insert("body", body);
    }
    if let Some(summary) = join_distinct(target.summary.as_deref(), source.summary.as_deref()) {
        target_patch.insert("summary", summary);
    }
    for (key, values) in [
        ("applicable_scenes", &source.applicable_scenes),
        ("not_applicable_scenes", &source.not_applicable_scenes),
        ("product_tags", &source.product_tags),
        ("business_topics", &source.business_topics),
    ] {
        if !values.is_empty() {
            target_patch.insert(
                key,
                mongodb::bson::to_bson(values).map_err(|e| {
                    AppError::External(format!("serialize merge field {key} failed: {e}"))
                })?,
            );
        }
    }
    if target_patch.is_empty() {
        return Err(AppError::BadRequest(
            "source chunk has no mergeable content".to_string(),
        ));
    }
    // The merged claim set needs fresh evidence before it can be verified.
    target_patch.insert("source_quote", "");
    target_patch.insert("source_anchors", Bson::Array(Vec::new()));

    // Per-chunk locks are enforced by the revision harness by preserving the
    // existing value. That behavior is right for ordinary mixed-field edits,
    // but merge must be all-or-nothing at the semantic level: silently dropping
    // any source content and then archiving the source would lose knowledge.
    let target_document = mongodb::bson::to_document(&target)
        .map_err(|e| AppError::External(format!("serialize merge target failed: {e}")))?;
    let locked = effective_locked_fields(&target_document);
    let blocked: Vec<_> = target_patch
        .keys()
        .filter(|key| locked.iter().any(|locked_key| locked_key == *key))
        .cloned()
        .collect();
    if !blocked.is_empty() {
        return Err(AppError::BadRequest(format!(
            "merge target locks required fields: {}",
            blocked.join(", ")
        )));
    }

    let tgt = apply_chunk_revision_with_session(
        &state.db,
        workspace_id,
        target_id,
        RevisionRequest {
            op: RevisionOp::Merge,
            source: ProvenanceSource::Human,
            patch: target_patch,
            reason: payload
                .reason
                .clone()
                .or_else(|| Some(format!("merged from chunk {}", object_id.to_hex()))),
            actor: Some(admin.username.clone()),
        },
        session,
    )
    .await?;
    let arch = apply_chunk_revision_with_session(
        &state.db,
        workspace_id,
        object_id,
        RevisionRequest {
            op: RevisionOp::Merge,
            source: ProvenanceSource::Human,
            patch: doc! { "status": "archived", "superseded_by": target_id.to_hex() },
            reason: payload.reason,
            actor: Some(admin.username.clone()),
        },
        session,
    )
    .await?;
    Ok((tgt, arch))
}

/// `POST /operation-knowledge/chunks/:id/merge` — atomically merge into target.
pub(in crate::routes) async fn merge_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkMergeRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let result = merge_operation_knowledge_chunk_in_transaction(
        &state,
        &admin,
        object_id,
        payload,
        &mut session,
    )
    .await;
    let (tgt, arch) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    };
    commit_chunk_transaction(&mut session).await?;

    for changed in [&tgt, &arch] {
        super::super::chunk_locks::broadcast_chunk_revised_in(
            &state,
            &admin.current_workspace,
            &changed.chunk_id,
            "merge",
            &admin.username,
        );
    }
    Ok(Json(json!({
        "ok": true,
        "archived": revision_applied_to_json(&arch),
        "target": revision_applied_to_json(&tgt),
    })))
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(in crate::routes) enum ChunkRelationKind {
    SupersededBy,
    References,
    Requires,
    Contradicts,
    Clarifies,
    Refines,
}

impl ChunkRelationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::SupersededBy => "superseded_by",
            Self::References => "references",
            Self::Requires => "requires",
            Self::Contradicts => "contradicts",
            Self::Clarifies => "clarifies",
            Self::Refines => "refines",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::routes) struct ChunkRelateRequest {
    pub target_id: String,
    pub kind: ChunkRelationKind,
    pub note: Option<String>,
    pub reason: Option<String>,
}

/// `POST /operation-knowledge/chunks/:id/relate` — 添加一条 related_chunks。
pub(in crate::routes) async fn relate_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkRelateRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let target_oid = parse_object_id(&payload.target_id)?;
    if object_id == target_oid {
        return Err(AppError::BadRequest(
            "cannot relate a chunk to itself".to_string(),
        ));
    }
    let relation_kind = payload.kind.as_str();
    let existing = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    // The target must be visible from the source capability. A private source may
    // point to shared or same-account knowledge; a shared source may only point
    // to shared knowledge. This prevents relation traversal from becoming an
    // account-to-account read tunnel.
    let target_visibility = match existing.account_id.as_deref() {
        Some(account_id) => vec![
            doc! { "account_id": null },
            doc! { "account_id": account_id },
        ],
        None => vec![doc! { "account_id": null }],
    };
    state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": target_oid,
                "workspace_id": &admin.current_workspace,
                "domain": &existing.domain,
                "$or": target_visibility,
            },
            None,
        )
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(
                "relate target is outside the source account visibility scope".to_string(),
            )
        })?;
    let mut related = existing.related_chunks.clone().unwrap_or_default();
    // 同 (target_id, kind) 已存在 → 视为幂等成功，更新 note
    if let Some(found) = related
        .iter_mut()
        .find(|r| r.chunk_id == payload.target_id && r.kind == relation_kind)
    {
        found.note = payload.note.clone().or_else(|| found.note.clone());
    } else {
        related.push(crate::models::RelatedRef {
            chunk_id: payload.target_id.clone(),
            kind: relation_kind.to_string(),
            note: payload.note.clone(),
        });
    }
    let req = RevisionRequest {
        op: RevisionOp::Patch,
        source: ProvenanceSource::Human,
        patch: doc! {
            "related_chunks": mongodb::bson::to_bson(&related)
                .map_err(|e| AppError::External(format!("serialize related_chunks failed: {e}")))?
        },
        reason: payload.reason.or_else(|| {
            Some(format!(
                "relate -> {} ({})",
                payload.target_id, relation_kind
            ))
        }),
        actor: Some(admin.username.clone()),
    };
    let applied = apply_chunk_revision(&state.db, &admin.current_workspace, object_id, req).await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "relate",
        &admin.username,
    );
    Ok(Json(revision_applied_to_json(&applied)))
}

/// `DELETE /operation-knowledge/chunks/:id/relate/:target_id` — 移除单条关系。
pub(in crate::routes) async fn unrelate_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path((id, target_id)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let existing = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    let original_len = existing
        .related_chunks
        .as_ref()
        .map(|v| v.len())
        .unwrap_or(0);
    let kept: Vec<_> = existing
        .related_chunks
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.chunk_id != target_id)
        .collect();
    if kept.len() == original_len {
        return Ok(Json(json!({
            "ok": true,
            "removed": 0,
        })));
    }
    let req = RevisionRequest {
        op: RevisionOp::Patch,
        source: ProvenanceSource::Human,
        patch: doc! {
            "related_chunks": mongodb::bson::to_bson(&kept)
                .map_err(|e| AppError::External(format!("serialize related_chunks failed: {e}")))?
        },
        reason: Some(format!("unrelate -> {target_id}")),
        actor: Some(admin.username.clone()),
    };
    let applied = apply_chunk_revision(&state.db, &admin.current_workspace, object_id, req).await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "unrelate",
        &admin.username,
    );
    let mut value = revision_applied_to_json(&applied);
    if let Some(o) = value.as_object_mut() {
        o.insert("removed".to_string(), json!(original_len - kept.len()));
    }
    Ok(Json(value))
}

// ── G3 · 反向查询 + 批量动作（admin 手工触发，非 AI 自动）──────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkReferrersQuery {
    /// 主名 `targetId`（camelCase 契约）；`target_id` 作为历史别名保留，
    /// 兼容早期前端/书签的 snake_case 写法（曾导致该接口恒 400）。
    #[serde(alias = "target_id")]
    pub target_id: String,
}

/// `GET /operation-knowledge/chunks/referrers?targetId=...`
/// 扫 `related_chunks.chunk_id == target_id`，返回反向引用列表。
/// 不物化反向 link（避免双向写入一致性问题），每次查询走 query path。
pub async fn list_chunk_referrers(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<ChunkReferrersQuery>,
) -> AppResult<Json<Value>> {
    if q.target_id.trim().is_empty() {
        return Err(AppError::BadRequest("target_id is required".to_string()));
    }
    let mut cur = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "related_chunks.chunk_id": &q.target_id,
            },
            None,
        )
        .await?;
    let mut items: Vec<Value> = Vec::new();
    while cur.advance().await? {
        let chunk = cur.deserialize_current()?;
        let chunk_id = chunk.id.map(|o| o.to_hex()).unwrap_or_default();
        let related = chunk.related_chunks.clone().unwrap_or_default();
        let matched: Vec<&_> = related
            .iter()
            .filter(|r| r.chunk_id == q.target_id)
            .collect();
        for r in matched {
            items.push(json!({
                "chunkId": chunk_id,
                "title": chunk.title.clone(),
                "wikiType": chunk.wiki_type.clone(),
                "status": chunk.status.clone(),
                "kind": r.kind.clone(),
                "note": r.note.clone(),
            }));
            if items.len() >= 50 {
                break;
            }
        }
        if items.len() >= 50 {
            break;
        }
    }
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChunkBatchVerifyItem {
    pub id: String,
    pub expected_updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChunkBatchVerifyRequest {
    pub items: Vec<ChunkBatchVerifyItem>,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /operation-knowledge/chunks/batch-verify`
/// 批量调用 verify_operation_knowledge_chunk 主体逻辑；每条独立 chunk_revisions(op=verify)。
/// 单条失败不阻断其它（部分成功）；返回 `{ verified: [...], skipped: [{id, reason}] }`。
/// AI 永不自动 verify 红线保留：批量入口仍需 admin 手工触发，与单条同 auth 路径。
pub async fn batch_verify_chunks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<ChunkBatchVerifyRequest>,
) -> AppResult<Json<Value>> {
    if payload.items.is_empty() {
        return Err(AppError::BadRequest("items is required".to_string()));
    }
    if payload.items.len() > 100 {
        return Err(AppError::BadRequest("max 100 items per batch".to_string()));
    }
    let mut verified: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for item in &payload.items {
        let object_id = match parse_object_id(&item.id) {
            Ok(value) => value,
            Err(_) => {
                skipped.push(json!({ "id": item.id, "reason": "invalid_object_id" }));
                continue;
            }
        };
        let expected_updated_at = match parse_verify_expected_updated_at(&item.expected_updated_at)
        {
            Ok(value) => value,
            Err(error) => {
                skipped.push(json!({ "id": item.id, "reason": error.to_string() }));
                continue;
            }
        };
        match verify_chunk_at_version(
            &state,
            &admin.current_workspace,
            object_id,
            expected_updated_at,
            &[],
            payload.note.clone(),
            &admin.username,
        )
        .await
        {
            Ok(_) => verified.push(item.id.clone()),
            Err(error) => skipped.push(json!({ "id": item.id, "reason": error.to_string() })),
        }
    }
    Ok(Json(json!({
        "verified": verified,
        "skipped": skipped,
        "note": payload.note,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkBatchArchiveRequest {
    pub ids: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /operation-knowledge/chunks/batch-archive`
/// 复用 archive_operation_knowledge_chunk 内部 RevisionRequest 路径。
pub async fn batch_archive_chunks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<ChunkBatchArchiveRequest>,
) -> AppResult<Json<Value>> {
    if payload.ids.is_empty() {
        return Err(AppError::BadRequest("ids is required".to_string()));
    }
    if payload.ids.len() > 100 {
        return Err(AppError::BadRequest("max 100 ids per batch".to_string()));
    }
    let mut archived: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for id in payload.ids.iter() {
        let object_id = match parse_object_id(id) {
            Ok(v) => v,
            Err(_) => {
                skipped.push(json!({ "id": id, "reason": "invalid_object_id" }));
                continue;
            }
        };
        let req = RevisionRequest {
            op: RevisionOp::Archive,
            source: ProvenanceSource::Human,
            patch: Document::new(),
            reason: payload.reason.clone(),
            actor: Some(admin.username.clone()),
        };
        match apply_chunk_revision(&state.db, &admin.current_workspace, object_id, req).await {
            Ok(_) => archived.push(id.clone()),
            Err(e) => skipped.push(json!({ "id": id, "reason": format!("{}", e) })),
        }
    }
    Ok(Json(json!({
        "archived": archived,
        "skipped": skipped,
    })))
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    use crate::knowledge_wiki::chunk_revisions::RevisionApplied;

    #[test]
    fn revision_applied_to_json_matches_contract_fixture() {
        let applied = RevisionApplied {
            revision_id: "rev-1".to_string(),
            chunk_id: "chunk-1".to_string(),
            op: "patch".to_string(),
            before_hash: "hash-before".to_string(),
            after_hash: "hash-after".to_string(),
            unchanged: false,
        };
        let projected = revision_applied_to_json(&applied);
        crate::routes::contract_snapshot::assert_contract_fixture("revision_applied", projected);
    }

    #[test]
    fn chunk_action_requests_accept_only_the_published_wire_contract() {
        let patch: ChunkPatchRequest = serde_json::from_value(json!({
            "patch": { "summary": "updated" }
        }))
        .expect("published patch body");
        assert_eq!(patch.patch["summary"], "updated");

        let split: ChunkSplitRequest =
            serde_json::from_value(json!({ "offset": 5 })).expect("published split body");
        assert_eq!(split.offset, 5);

        let merge: ChunkMergeRequest = serde_json::from_value(json!({
            "targetId": "507f1f77bcf86cd799439011"
        }))
        .expect("published merge body");
        assert_eq!(merge.target_id, "507f1f77bcf86cd799439011");

        let relate: ChunkRelateRequest = serde_json::from_value(json!({
            "targetId": "507f1f77bcf86cd799439012",
            "kind": "references",
            "note": "source"
        }))
        .expect("published relate body");
        assert_eq!(relate.kind, ChunkRelationKind::References);

        assert!(serde_json::from_value::<ChunkPatchRequest>(json!({
            "summary": "wrong level"
        }))
        .is_err());
        assert!(serde_json::from_value::<ChunkSplitRequest>(json!({
            "offset": 5,
            "newChunks": []
        }))
        .is_err());
        assert!(serde_json::from_value::<ChunkMergeRequest>(json!({
            "target_id": "507f1f77bcf86cd799439011"
        }))
        .is_err());
        assert!(serde_json::from_value::<ChunkRelateRequest>(json!({
            "target_id": "507f1f77bcf86cd799439012",
            "kind": "references"
        }))
        .is_err());
        assert!(serde_json::from_value::<ChunkRelateRequest>(json!({
            "targetId": "507f1f77bcf86cd799439012",
            "kind": "supports"
        }))
        .is_err());
    }

    #[test]
    fn chunk_referrers_query_accepts_camel_case_and_snake_case_target_id() {
        use axum::extract::Query;
        use axum::http::Uri;

        let camel: Query<ChunkReferrersQuery> = Query::try_from_uri(
            &"/operation-knowledge/chunks/referrers?targetId=abc"
                .parse::<Uri>()
                .expect("uri"),
        )
        .expect("camelCase targetId accepted");
        assert_eq!(camel.target_id, "abc");

        let snake: Query<ChunkReferrersQuery> = Query::try_from_uri(
            &"/operation-knowledge/chunks/referrers?target_id=abc"
                .parse::<Uri>()
                .expect("uri"),
        )
        .expect("snake_case target_id accepted as legacy alias");
        assert_eq!(snake.target_id, "abc");
    }
}

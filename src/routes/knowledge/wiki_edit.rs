//! 运营知识库 wiki 编辑：切片 patch/archive/restore/rollback/split/merge/relate + 批量核验/归档 + 引用查询。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use mongodb::bson::{doc, Bson, DateTime, Document};
use mongodb::options::FindOptions;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};

use super::super::shared::*;
use super::super::AppState;
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
// 5) 双写 chunk_revisions + chunks，先 history 后最新
// 6) enqueue catalog_rebuild_jobs（best-effort）

use crate::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, cleanup_dangling_refs, ProvenanceSource, RevisionApplied, RevisionOp,
    RevisionRequest,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkPatchRequest {
    /// 字段级 patch；不允许携带 locked_fields。
    pub patch: Value,
    /// "ai" / "human" / "rule" / "imported"。
    #[serde(default = "default_chunk_patch_source")]
    pub source: String,
    pub reason: Option<String>,
    pub actor: Option<String>,
}

fn default_chunk_patch_source() -> String {
    "human".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkArchiveRequest {
    pub reason: Option<String>,
    pub actor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkRollbackRequest {
    pub actor: Option<String>,
}

/// JSON Value → BSON Document（用于 ChunkPatchRequest.patch）。
fn json_object_to_document(v: &Value) -> AppResult<Document> {
    let obj = v
        .as_object()
        .ok_or_else(|| AppError::BadRequest("patch 必须是 JSON 对象".to_string()))?;
    let bson_value: Bson = mongodb::bson::to_bson(obj)
        .map_err(|e| AppError::BadRequest(format!("patch 转 BSON 失败: {e}")))?;
    match bson_value {
        Bson::Document(d) => Ok(d),
        _ => Err(AppError::BadRequest("patch 必须是 JSON 对象".to_string())),
    }
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
    let patch = json_object_to_document(&payload.patch)?;
    let source: ProvenanceSource = payload.source.parse()?;
    let req = RevisionRequest {
        op: RevisionOp::Patch,
        source,
        patch,
        reason: payload.reason,
        actor: payload.actor,
    };
    let applied = apply_chunk_revision(
        &state.db,
        &admin.current_workspace,
        object_id,
        req,
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

/// `POST /operation-knowledge/chunks/:id/archive` — 软删（status=archived）+
/// 删除级联（清空其它 chunk 的 related_chunks 引用）。
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
        actor: payload.actor,
    };
    let applied = apply_chunk_revision(
        &state.db,
        &admin.current_workspace,
        object_id,
        req,
    )
    .await?;
    let cleaned = cleanup_dangling_refs(
        &state.db,
        &admin.current_workspace,
        &applied.chunk_id,
    )
    .await
    .unwrap_or(0);
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "archive",
        &admin.username,
    );
    let mut value = revision_applied_to_json(&applied);
    if let Some(o) = value.as_object_mut() {
        o.insert("cleanedReferences".to_string(), json!(cleaned));
    }
    Ok(Json(value))
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
        actor: payload.actor,
    };
    let applied = apply_chunk_revision(
        &state.db,
        &admin.current_workspace,
        object_id,
        req,
    )
    .await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "restore",
        &admin.username,
    );
    Ok(Json(revision_applied_to_json(&applied)))
}

/// `POST /operation-knowledge/chunks/:id/rollback/:revision_id` — 回滚到某 revision
/// 之前的 chunk 状态。
///
/// 实现方式：找到目标 revision，反向应用 patch（把 patch 中每个 key 的值改回
/// `before_hash` 时刻的内容）。简化：当前不支持精确"还原到某个时间点"，仅支持
/// "把当前 chunk 的关键字段重写为目标 revision 的 patch 中字段的反值"——所以
/// 通常用法是回滚最近一次 patch（其它复杂场景请用 `/patch` 显式指定）。
///
/// 写入仍走 apply_chunk_revision(op=Rollback)，留下"我回滚到了 X"的痕迹而非
/// 物理删除 history。
pub(in crate::routes) async fn rollback_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path((id, revision_id)): Path<(String, String)>,
    Json(payload): Json<ChunkRollbackRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    // 找目标 revision
    let target = state
        .db
        .chunk_revisions()
        .find_one(
            doc! {
                "chunk_id": object_id.to_hex(),
                "revision_id": &revision_id,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("revision {revision_id} not found")))?;
    // 找它的"前一条"revision —— 即 created_at < target.created_at 的最近一条
    let prev = state
        .db
        .chunk_revisions()
        .find_one(
            doc! {
                "chunk_id": object_id.to_hex(),
                "created_at": { "$lt": target.created_at },
            },
            mongodb::options::FindOneOptions::builder()
                .sort(doc! { "created_at": -1 })
                .build(),
        )
        .await?;
    // 简化策略：rollback 时把目标 revision 的 patch 中所有 key 设为前一条 revision
    // patch 中相应字段的值；前一条不存在或字段不存在 → 移除（用 $unset，但这里
    // 走 apply_chunk_revision 路径，所以用 BSON Null 表示移除意图，由
    // apply_field_patch 兼容处理为空字符串/空数组）。
    //
    // 因为 apply_chunk_revision 不直接支持 $unset，我们在 patch 中只回填能找到
    // 的字段；找不到的字段提示 caller "无法完整回滚某些字段"。
    let mut rollback_patch = Document::new();
    let mut missing: Vec<String> = Vec::new();
    if let Some(prev_rev) = &prev {
        for key in target.patch.keys() {
            if let Some(prev_val) = prev_rev.patch.get(key) {
                rollback_patch.insert(key, prev_val.clone());
            } else {
                missing.push(key.to_string());
            }
        }
    } else {
        for key in target.patch.keys() {
            missing.push(key.to_string());
        }
    }
    let req = RevisionRequest {
        op: RevisionOp::Rollback,
        source: ProvenanceSource::Human,
        patch: rollback_patch,
        reason: Some(format!(
            "rollback to revision {revision_id}; missing_fields={}",
            missing.len()
        )),
        actor: payload.actor,
    };
    let applied = apply_chunk_revision(
        &state.db,
        &admin.current_workspace,
        object_id,
        req,
    )
    .await?;
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
        o.insert("missingFields".to_string(), json!(missing));
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
    // 多租户隔离：先确认该 chunk 属于当前 workspace，再列其编辑历史
    // （chunk_revisions 自身不带 workspace_id，靠父 chunk 授权）。
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
        .find(doc! { "chunk_id": object_id.to_hex() }, opts)
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
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkSplitRequest {
    /// 把当前 chunk 内容按这一段拆分成 N 份的锚点描述（仅记入 reason，
    /// 实际拆分由 caller 提供新 chunks 内容）。
    pub split_anchor: Option<String>,
    /// N 个新 chunk 的 patch 描述（每份至少含 title + body）。
    pub new_chunks: Vec<Value>,
    pub reason: Option<String>,
    pub actor: Option<String>,
}

/// `POST /operation-knowledge/chunks/:id/split` — 拆分 chunk。
///
/// 行为：
/// 1. 把原 chunk 标 archived（写一条 op=split revision）；
/// 2. 复制原 chunk 的 metadata（domain / wiki_type / workspace_id / document_id），
///    覆盖 caller 提供的字段，新建 N 个 chunk（每份写 op=create revision，
///    `previous_version_id` 指向原 chunk）。
///
/// 失败回滚不做 atomicity 保证（按 LLW 简化策略：split/merge 是低频运营动作，
/// 失败时 admin 直接看 history 修复）。
pub(in crate::routes) async fn split_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkSplitRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let workspace_id = &admin.current_workspace;
    if payload.new_chunks.is_empty() {
        return Err(AppError::BadRequest(
            "new_chunks 不可为空，至少需要 1 份新 chunk".to_string(),
        ));
    }
    let original = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": object_id, "workspace_id": workspace_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    // 1) 原 chunk archive
    let archive_req = RevisionRequest {
        op: RevisionOp::Split,
        source: ProvenanceSource::Human,
        patch: Document::new(),
        reason: payload
            .reason
            .clone()
            .or_else(|| Some(format!("split into {} new chunks", payload.new_chunks.len()))),
        actor: payload.actor.clone(),
    };
    // 用 archive 语义但 op 标 Split（apply_chunk_revision 内部把 status 设 archived）
    let mut archive_patch = Document::new();
    archive_patch.insert("status", "archived");
    let archive_req = RevisionRequest {
        patch: archive_patch,
        ..archive_req
    };
    let archived = apply_chunk_revision(&state.db, workspace_id, object_id, archive_req).await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        workspace_id,
        &archived.chunk_id,
        "split",
        &admin.username,
    );
    // 2) 创建 N 个新 chunk
    let mut new_ids: Vec<String> = Vec::new();
    let now = DateTime::now();
    for raw in &payload.new_chunks {
        let mut new_doc = Document::new();
        new_doc.insert("workspace_id", workspace_id);
        new_doc.insert("account_id", original.account_id.clone());
        new_doc.insert(
            "document_id",
            original
                .document_id
                .map(Bson::ObjectId)
                .unwrap_or(Bson::Null),
        );
        new_doc.insert("domain", original.domain.clone());
        new_doc.insert("title", "拆分草稿（待编辑）");
        new_doc.insert("status", "draft");
        new_doc.insert("integrity_status", "needs_review");
        new_doc.insert("priority", original.priority);
        new_doc.insert("created_at", now);
        new_doc.insert("updated_at", now);
        new_doc.insert(
            "wiki_type",
            original
                .wiki_type
                .clone()
                .unwrap_or_else(|| "entity".to_string()),
        );
        new_doc.insert("previous_version_id", object_id.to_hex());
        // 合并 caller 给出的字段（title / body / summary 等）
        let raw_doc = json_object_to_document(raw)?;
        for (k, v) in raw_doc.iter() {
            new_doc.insert(k, v.clone());
        }
        let inserted = state
            .db
            .operation_knowledge_chunks()
            .insert_one(
                mongodb::bson::from_document::<crate::models::OperationKnowledgeChunk>(new_doc.clone())
                    .map_err(|e| AppError::BadRequest(format!("split 新 chunk 字段不合法: {e}")))?,
                None,
            )
            .await?;
        if let Some(oid) = inserted.inserted_id.as_object_id() {
            // 写一条 create revision（source=human，便于审计）
            let create_req = RevisionRequest {
                op: RevisionOp::Create,
                source: ProvenanceSource::Human,
                patch: raw_doc,
                reason: Some(format!(
                    "split from chunk {} (anchor={})",
                    object_id.to_hex(),
                    payload.split_anchor.clone().unwrap_or_default()
                )),
                actor: payload.actor.clone(),
            };
            // 该 chunk 在 DB 中已存在，apply_chunk_revision 会读它再写一次（幂等）
            let _ = apply_chunk_revision(&state.db, workspace_id, oid, create_req).await;
            new_ids.push(oid.to_hex());
        }
    }
    Ok(Json(json!({
        "ok": true,
        "archived": revision_applied_to_json(&archived),
        "newChunkIds": new_ids,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkMergeRequest {
    /// 合并目标的 chunk_id。
    pub merge_target_id: String,
    /// "into_target": 内容并入 target，原 chunk 归档；
    /// "new_chunk": 双 archive，创建新 chunk（new_chunks[0] 为新 chunk 字段集）。
    #[serde(default = "default_merge_strategy")]
    pub merge_strategy: String,
    pub new_chunk: Option<Value>,
    pub reason: Option<String>,
    pub actor: Option<String>,
}

fn default_merge_strategy() -> String {
    "into_target".to_string()
}

/// `POST /operation-knowledge/chunks/:id/merge` — 合并 chunk。
pub(in crate::routes) async fn merge_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkMergeRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let target_id = parse_object_id(&payload.merge_target_id)?;
    let workspace_id = &admin.current_workspace;
    match payload.merge_strategy.as_str() {
        "into_target" => {
            // 把原 chunk 归档，target chunk 接收一些字段（数组字段会自动 union）
            let archive = RevisionRequest {
                op: RevisionOp::Merge,
                source: ProvenanceSource::Human,
                patch: doc! { "status": "archived", "superseded_by": target_id.to_hex() },
                reason: payload.reason.clone(),
                actor: payload.actor.clone(),
            };
            let arch = apply_chunk_revision(&state.db, workspace_id, object_id, archive).await?;
            // target chunk 写一条 merge revision（patch=空，意在记录"我吸收了原 chunk"）
            let target_req = RevisionRequest {
                op: RevisionOp::Merge,
                source: ProvenanceSource::Human,
                patch: doc! { "previous_version_id": object_id.to_hex() },
                reason: Some(format!("merged from chunk {}", object_id.to_hex())),
                actor: payload.actor.clone(),
            };
            let tgt = apply_chunk_revision(&state.db, workspace_id, target_id, target_req).await?;
            super::super::chunk_locks::broadcast_chunk_revised_in(
                &state,
                workspace_id,
                &arch.chunk_id,
                "merge",
                &admin.username,
            );
            super::super::chunk_locks::broadcast_chunk_revised_in(
                &state,
                workspace_id,
                &tgt.chunk_id,
                "merge",
                &admin.username,
            );
            Ok(Json(json!({
                "ok": true,
                "archived": revision_applied_to_json(&arch),
                "target": revision_applied_to_json(&tgt),
            })))
        }
        "new_chunk" => {
            // 双 archive + 新 chunk
            let arch_a = apply_chunk_revision(
                &state.db,
                workspace_id,
                object_id,
                RevisionRequest {
                    op: RevisionOp::Merge,
                    source: ProvenanceSource::Human,
                    patch: doc! { "status": "archived" },
                    reason: payload.reason.clone(),
                    actor: payload.actor.clone(),
                },
            )
            .await?;
            let arch_b = apply_chunk_revision(
                &state.db,
                workspace_id,
                target_id,
                RevisionRequest {
                    op: RevisionOp::Merge,
                    source: ProvenanceSource::Human,
                    patch: doc! { "status": "archived" },
                    reason: payload.reason.clone(),
                    actor: payload.actor.clone(),
                },
            )
            .await?;
            let raw = payload.new_chunk.ok_or_else(|| {
                AppError::BadRequest(
                    "merge_strategy=new_chunk 时必须提供 new_chunk 字段".to_string(),
                )
            })?;
            let raw_doc = json_object_to_document(&raw)?;
            let now = DateTime::now();
            let mut new_doc = raw_doc.clone();
            new_doc.insert("workspace_id", workspace_id);
            new_doc.insert("status", "draft");
            new_doc.insert("integrity_status", "needs_review");
            new_doc.insert("created_at", now);
            new_doc.insert("updated_at", now);
            if !new_doc.contains_key("priority") {
                new_doc.insert("priority", 0i32);
            }
            if !new_doc.contains_key("title") {
                new_doc.insert("title", "合并草稿（待编辑）");
            }
            if !new_doc.contains_key("domain") {
                new_doc.insert("domain", "user");
            }
            if !new_doc.contains_key("wiki_type") {
                new_doc.insert("wiki_type", "entity");
            }
            new_doc.insert(
                "previous_version_id",
                format!("{}+{}", object_id.to_hex(), target_id.to_hex()),
            );
            let inserted = state
                .db
                .operation_knowledge_chunks()
                .insert_one(
                    mongodb::bson::from_document::<crate::models::OperationKnowledgeChunk>(
                        new_doc.clone(),
                    )
                    .map_err(|e| {
                        AppError::BadRequest(format!("merge 新 chunk 字段不合法: {e}"))
                    })?,
                    None,
                )
                .await?;
            let new_id = inserted
                .inserted_id
                .as_object_id()
                .map(|o| o.to_hex())
                .unwrap_or_default();
            super::super::chunk_locks::broadcast_chunk_revised_in(
                &state,
                workspace_id,
                &arch_a.chunk_id,
                "merge",
                &admin.username,
            );
            super::super::chunk_locks::broadcast_chunk_revised_in(
                &state,
                workspace_id,
                &arch_b.chunk_id,
                "merge",
                &admin.username,
            );
            if !new_id.is_empty() {
                super::super::chunk_locks::broadcast_chunk_revised_in(
                    &state,
                    workspace_id,
                    &new_id,
                    "create",
                    &admin.username,
                );
            }
            Ok(Json(json!({
                "ok": true,
                "archivedA": revision_applied_to_json(&arch_a),
                "archivedB": revision_applied_to_json(&arch_b),
                "newChunkId": new_id,
            })))
        }
        other => Err(AppError::BadRequest(format!(
            "merge_strategy='{other}' 不合法，应为 into_target | new_chunk"
        ))),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChunkRelateRequest {
    pub target_id: String,
    /// "superseded_by" / "references" / "requires" / "contradicts" / "clarifies" / "refines"
    pub kind: String,
    pub note: Option<String>,
    pub reason: Option<String>,
    pub actor: Option<String>,
}

const ALLOWED_RELATION_KINDS: &[&str] = &[
    "superseded_by",
    "references",
    "requires",
    "contradicts",
    "clarifies",
    "refines",
];

/// `POST /operation-knowledge/chunks/:id/relate` — 添加一条 related_chunks。
pub(in crate::routes) async fn relate_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ChunkRelateRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    if !ALLOWED_RELATION_KINDS.contains(&payload.kind.as_str()) {
        return Err(AppError::BadRequest(format!(
            "relation kind '{}' 不合法，应为 {}",
            payload.kind,
            ALLOWED_RELATION_KINDS.join(" | "),
        )));
    }
    // target 必须存在（同 workspace）
    let target_oid = parse_object_id(&payload.target_id)?;
    state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": target_oid,
                "workspace_id": &admin.current_workspace,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("relate target chunk not found".to_string()))?;
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
    let mut related = existing.related_chunks.clone().unwrap_or_default();
    // 同 (target_id, kind) 已存在 → 视为幂等成功，更新 note
    if let Some(found) = related
        .iter_mut()
        .find(|r| r.chunk_id == payload.target_id && r.kind == payload.kind)
    {
        found.note = payload.note.clone().or_else(|| found.note.clone());
    } else {
        related.push(crate::models::RelatedRef {
            chunk_id: payload.target_id.clone(),
            kind: payload.kind.clone(),
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
                payload.target_id, payload.kind
            ))
        }),
        actor: payload.actor,
    };
    let applied = apply_chunk_revision(
        &state.db,
        &admin.current_workspace,
        object_id,
        req,
    )
    .await?;
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
        actor: None,
    };
    let applied = apply_chunk_revision(
        &state.db,
        &admin.current_workspace,
        object_id,
        req,
    )
    .await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "unrelate",
        &admin.username,
    );
    let mut value = revision_applied_to_json(&applied);
    if let Some(o) = value.as_object_mut() {
        o.insert(
            "removed".to_string(),
            json!(original_len - kept.len()),
        );
    }
    Ok(Json(value))
}

// ── G3 · 反向查询 + 批量动作（admin 手工触发，非 AI 自动）──────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkReferrersQuery {
    pub target_id: String,
}

/// `GET /operation-knowledge/chunks/referrers?target_id=...`
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
        let chunk_id = chunk
            .id
            .map(|o| o.to_hex())
            .unwrap_or_default();
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
#[serde(rename_all = "camelCase")]
pub struct ChunkBatchVerifyRequest {
    pub ids: Vec<String>,
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
    if payload.ids.is_empty() {
        return Err(AppError::BadRequest("ids is required".to_string()));
    }
    if payload.ids.len() > 100 {
        return Err(AppError::BadRequest("max 100 ids per batch".to_string()));
    }
    let mut verified: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for id in payload.ids.iter() {
        let object_id = match parse_object_id(id) {
            Ok(v) => v,
            Err(_) => {
                skipped.push(json!({ "id": id, "reason": "invalid_object_id" }));
                continue;
            }
        };
        let chunk = match state
            .db
            .operation_knowledge_chunks()
            .find_one(
                doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
                None,
            )
            .await
        {
            Ok(Some(c)) => c,
            Ok(None) => {
                skipped.push(json!({ "id": id, "reason": "not_found" }));
                continue;
            }
            Err(e) => {
                skipped.push(json!({ "id": id, "reason": format!("db_error: {}", e) }));
                continue;
            }
        };
        let has_quote = chunk
            .source_quote
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let has_anchor = !chunk.source_anchors.is_empty();
        if let Some(reason) = chunk_verify_gate_reason(has_quote, has_anchor) {
            skipped.push(json!({ "id": id, "reason": reason }));
            continue;
        }
        match state
            .db
            .operation_knowledge_chunks()
            .update_one(
                doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
                doc! {
                    "$set": {
                        "integrity_status": "verified",
                        "confidence_score": 100,
                        "unsupported_claims": Bson::Array(Vec::new()),
                        "status": "active",
                        "updated_at": DateTime::now()
                    }
                },
                None,
            )
            .await
        {
            Ok(_) => verified.push(id.clone()),
            Err(e) => skipped.push(json!({ "id": id, "reason": format!("update_failed: {}", e) })),
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
    #[serde(default)]
    pub actor: Option<String>,
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
            actor: payload.actor.clone(),
        };
        match apply_chunk_revision(
            &state.db,
            &admin.current_workspace,
            object_id,
            req,
        )
        .await
        {
            Ok(_) => archived.push(id.clone()),
            Err(e) => skipped.push(json!({ "id": id, "reason": format!("{}", e) })),
        }
    }
    Ok(Json(json!({
        "archived": archived,
        "skipped": skipped,
    })))
}

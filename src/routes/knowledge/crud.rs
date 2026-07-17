//! 运营知识库 CRUD：文档 / 切片 / 条目基础增删改查。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{bson::doc, bson::oid::ObjectId, options::FindOptions};
use serde_json::{json, Value};

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::knowledge_wiki::chunk_revisions::{
    chunk_replace_filter, monotonic_chunk_updated_at,
};
use crate::knowledge_wiki::page_merge::{
    compute_chunk_hash, effective_locked_fields, enforce_locked_fields,
};
use crate::models::{ChunkRevision, OperationKnowledgeChunk};

use super::super::shared::*;
use super::super::AppState;
use super::*;

pub(in crate::routes) async fn list_operation_knowledge(
    State(_state): State<AppState>,
    Query(_query): Query<OperationKnowledgeQuery>,
) -> AppResult<Json<Value>> {
    // operation_knowledge_items 已随 sales 旧库删除；旧 list 端口现在保持兼容
    // 形状但永远返回空集合。新的 wiki 流程走 operation_knowledge_chunks。
    Ok(Json(json!({ "items": Vec::<Value>::new() })))
}

pub(in crate::routes) async fn list_operation_knowledge_documents(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<OperationKnowledgeDocumentQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! {
        "workspace_id": &admin.current_workspace,
        "domain": "user_operations"
    };
    if let Some(account_id) = query.account_id {
        filter.insert(
            "$or",
            vec![
                doc! { "account_id": null },
                doc! { "account_id": account_id },
            ],
        );
    }
    if let Some(status) = normalize_optional(query.status) {
        filter.insert("status", status);
    }
    let mut cursor = state
        .db
        .operation_knowledge_documents()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(200)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(operation_knowledge_document_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(in crate::routes) async fn create_operation_knowledge_document(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<OperationKnowledgeDocumentRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_document(&payload)?;
    let result = state
        .db
        .operation_knowledge_documents()
        .insert_one(
            operation_knowledge_document_from_request(&state, &admin.current_workspace, payload, None),
            None,
        )
        .await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

pub(in crate::routes) async fn get_operation_knowledge_document(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let item = state
        .db
        .operation_knowledge_documents()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge document not found".to_string()))?;
    Ok(Json(
        json!({ "item": operation_knowledge_document_json(item) }),
    ))
}

pub(in crate::routes) async fn update_operation_knowledge_document(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OperationKnowledgeDocumentRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_document(&payload)?;
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_documents()
        .replace_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            operation_knowledge_document_from_request(&state, &admin.current_workspace, payload, Some(object_id)),
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(in crate::routes) async fn delete_operation_knowledge_document(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_documents()
        .delete_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?;
    state
        .db
        .operation_knowledge_chunks()
        .delete_many(
            doc! {
                "document_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(in crate::routes) async fn list_operation_knowledge_chunks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<OperationKnowledgeChunkQuery>,
) -> AppResult<Json<Value>> {
    let items =
        load_operation_knowledge_chunks_for_query(&state, &admin.current_workspace, query).await?;
    Ok(Json(json!({ "items": items })))
}

pub(in crate::routes) async fn list_operation_knowledge_document_chunks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let document_id = parse_object_id(&id)?;
    let items = load_operation_knowledge_chunks_for_query(
        &state,
        &admin.current_workspace,
        OperationKnowledgeChunkQuery {
            account_id: None,
            document_id: Some(document_id.to_hex()),
            item_id: None,
            status: None,
        },
    )
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub(in crate::routes) async fn create_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(mut payload): Json<OperationKnowledgeChunkRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_chunk(&payload)?;
    coerce_integrity_against_d2_gate(&mut payload);
    let result = state
        .db
        .operation_knowledge_chunks()
        .insert_one(
            operation_knowledge_chunk_from_request(&state, &admin.current_workspace, payload, None)?,
            None,
        )
        .await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

pub async fn update_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(mut payload): Json<OperationKnowledgeChunkRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_chunk(&payload)?;
    let object_id = parse_object_id(&id)?;
    // 取父文档原文，重新跑 apply_chunk_integrity：
    // 这样 PUT 能让 source_quote 通过模糊匹配回填 source_anchors，
    // AI 自主修复 / 运维直接编辑都走同一条 integrity 重算路径。
    let document_object_id = payload
        .document_id
        .as_deref()
        .and_then(|s| ObjectId::parse_str(s.trim()).ok());
    if let Some(document_id) = document_object_id {
        if let Some(document) = state
            .db
            .operation_knowledge_documents()
            .find_one(
                doc! {
                    "_id": document_id,
                    "workspace_id": &admin.current_workspace
                },
                None,
            )
            .await?
        {
            if let Some(raw) = document.raw_content.as_deref() {
                apply_chunk_integrity(&mut payload, raw, Some(document_id));
            }
        }
    }
    coerce_integrity_against_d2_gate(&mut payload);
    // 取原 chunk：用于回填请求体 OperationKnowledgeChunkRequest 无法表达、但
    // replace_one 会整条清空的 model 字段（provenance 来源追溯 / wiki_type /
    // locked_fields 等 + created_at）。filter 必须带 workspace_id（与 replace_one 一致），
    // 不能跨租户取原值。PUT 一个不存在的 chunk 返回 NotFound（create 有独立 POST 端点，
    // PUT 不该 upsert）。
    let existing = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    let next =
        operation_knowledge_chunk_from_request(&state, &admin.current_workspace, payload, Some(object_id))?;
    let next = preserve_unmodeled_chunk_fields(next, &existing);

    // KB-10：admin PUT 走 replace_one 整条替换，需在替换前把运营锁定字段从 existing
    // 强制覆盖回 next——否则 PUT 能绕过 per-chunk 锁定字段（`locked_fields` + DEFAULT 集）。
    // 复用与 apply_chunk_revision 同一份 effective_locked_fields + enforce_locked_fields
    // 纯函数（单一真相源，不造新 dual-path）。next/existing 是 typed struct，enforce_*
    // 收 &Document，故 to_document 转换后喂它们、再 from_document 转回 typed 供 replace_one。
    let existing_doc = mongodb::bson::to_document(&existing).map_err(|e| {
        AppError::External(format!("serialize existing chunk to bson failed: {e}"))
    })?;
    let next_doc = mongodb::bson::to_document(&next)
        .map_err(|e| AppError::External(format!("serialize next chunk to bson failed: {e}")))?;
    let effective_locked_owned = effective_locked_fields(&existing_doc);
    let effective_locked: Vec<&str> =
        effective_locked_owned.iter().map(|s| s.as_str()).collect();
    let mut enforced_doc = enforce_locked_fields(&next_doc, &existing_doc, &effective_locked);
    enforced_doc.insert(
        "updated_at",
        monotonic_chunk_updated_at(existing.updated_at, mongodb::bson::DateTime::now()),
    );
    let before_hash = compute_chunk_hash(&existing_doc);
    let after_hash = compute_chunk_hash(&enforced_doc);
    let enforced: OperationKnowledgeChunk = mongodb::bson::from_document(enforced_doc)
        .map_err(|e| AppError::External(format!("deserialize enforced chunk failed: {e}")))?;

    let replace_result = state
        .db
        .operation_knowledge_chunks()
        .replace_one(
            chunk_replace_filter(object_id, &admin.current_workspace, existing.updated_at),
            enforced,
            None,
        )
        .await?;
    if replace_result.matched_count == 0 {
        return Err(AppError::Conflict("chunk_revision_conflict".to_string()));
    }

    // KB-10：replace 成功后补写一条 chunk_revisions 审计行，补齐 admin 直接编辑的修订链。
    // patch 留空 doc!{}（整条替换、非增量，语义诚实）；before/after hash 标识本次编辑改了什么。
    // fail-soft：审计写失败仅记 warn，不回滚 replace、不返 Err——replace 已成功数据正确，
    // 审计缺一行是可观测运维问题。
    let revision = ChunkRevision {
        id: None,
        chunk_id: object_id.to_hex(),
        revision_id: format!("rev_{}_{}", object_id.to_hex(), uuid::Uuid::new_v4().simple()),
        op: "patch".to_string(),
        patch: doc! {},
        before_hash,
        after_hash,
        source: "human".to_string(),
        reason: Some("admin 直接编辑".to_string()),
        created_at: mongodb::bson::DateTime::now(),
        created_by: Some(admin.user_id.clone()),
    };
    if let Err(err) = state.db.chunk_revisions().insert_one(revision, None).await {
        tracing::warn!(
            chunk_id = %object_id.to_hex(),
            error = %err,
            "admin PUT chunk_revisions 审计行写入失败 (non-fatal)"
        );
    }

    Ok(Json(json!({ "ok": true })))
}

pub(in crate::routes) async fn delete_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_chunks()
        .delete_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(in crate::routes) async fn get_operation_knowledge_chunk_source(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let chunk = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    let document = if let Some(document_id) = chunk.document_id {
        state
            .db
            .operation_knowledge_documents()
            .find_one(
                doc! {
                    "_id": document_id,
                    "workspace_id": &admin.current_workspace
                },
                None,
            )
            .await?
    } else {
        None
    };
    Ok(Json(json!({
        "chunk": operation_knowledge_chunk_json(chunk),
        "document": document.map(operation_knowledge_document_json)
    })))
}

/// `GET /operation-knowledge/chunks/:id` — 取单个 chunk（前端收件箱 rich 深链用）。
pub async fn get_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let oid = mongodb::bson::oid::ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("无效 chunk id".into()))?;
    let item = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            mongodb::bson::doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("无此 chunk 或不属于当前 workspace".into()))?;
    Ok(Json(serde_json::json!({ "item": item })))
}

pub(in crate::routes) async fn create_operation_knowledge(
    State(_state): State<AppState>,
    Json(_payload): Json<OperationKnowledgeRequest>,
) -> AppResult<Json<Value>> {
    // operation_knowledge_items 已随 sales 旧库删除；此端点恒返 400（BadRequest），引导改用 operation_knowledge_chunks。
    Err(AppError::BadRequest(
        "operation_knowledge_items has been removed; use operation_knowledge_chunks instead"
            .to_string(),
    ))
}

pub(in crate::routes) async fn update_operation_knowledge(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
    Json(_payload): Json<OperationKnowledgeRequest>,
) -> AppResult<Json<Value>> {
    Err(AppError::BadRequest(
        "operation_knowledge_items has been removed; use operation_knowledge_chunks instead"
            .to_string(),
    ))
}

pub(in crate::routes) async fn delete_operation_knowledge(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> AppResult<Json<Value>> {
    Err(AppError::BadRequest(
        "operation_knowledge_items has been removed; use operation_knowledge_chunks instead"
            .to_string(),
    ))
}

#[cfg(test)]
mod contract_tests {
    use crate::models::OperationKnowledgeChunk;
    use mongodb::bson::{oid::ObjectId, DateTime};

    /// 详情端点 `get_operation_knowledge_chunk`(crud.rs:357) 直接 `json!({"item": item})`
    /// 裸序列化 model——snake_case + `{$oid}`，与列表投影 camelCase **形状冲突**。
    /// 本快照刻意暴露该冲突(spec §9):快照它,让"统一与否"成为可见的产品决策,而非静默漂移。
    #[test]
    fn chunk_detail_raw_struct_matches_contract_fixture() {
        let chunk = OperationKnowledgeChunk {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899aabb").unwrap()),
            workspace_id: "ws-1".to_string(),
            title: "7x24 自动应答".to_string(),
            domain: "user_operations".to_string(),
            status: "draft".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            ..Default::default()
        };
        let projected = serde_json::json!({ "item": chunk });
        crate::routes::contract_snapshot::assert_contract_fixture(
            "operation_knowledge_chunk_detail",
            projected,
        );
    }
}

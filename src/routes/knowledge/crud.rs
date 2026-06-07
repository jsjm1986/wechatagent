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
    Json(payload): Json<OperationKnowledgeChunkRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_chunk(&payload)?;
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

pub(in crate::routes) async fn update_operation_knowledge_chunk(
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
    state
        .db
        .operation_knowledge_chunks()
        .replace_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            operation_knowledge_chunk_from_request(&state, &admin.current_workspace, payload, Some(object_id))?,
            None,
        )
        .await?;
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

pub(in crate::routes) async fn create_operation_knowledge(
    State(_state): State<AppState>,
    Json(_payload): Json<OperationKnowledgeRequest>,
) -> AppResult<Json<Value>> {
    // operation_knowledge_items 已随 sales 旧库删除；保留 410 行为占位。
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

//! 运营知识库 CRUD：文档 / 切片 / 条目基础增删改查。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, to_bson, Bson, Document},
    options::FindOptions,
    ClientSession,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, apply_chunk_revision_with_session, commit_chunk_transaction,
    ProvenanceSource, RevisionOp, RevisionRequest,
};

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
            operation_knowledge_document_from_request(
                &state,
                &admin.current_workspace,
                payload,
                None,
            ),
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
    let expected_version = payload.version.ok_or_else(|| {
        AppError::BadRequest("version is required for document updates".to_string())
    })?;
    let existing = state
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
    if existing.version != expected_version {
        return Err(AppError::Conflict(
            "knowledge_document_version_conflict".to_string(),
        ));
    }
    let next_version = expected_version
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("knowledge_document_version_exhausted".to_string()))?;
    let updated_at = crate::knowledge_wiki::chunk_revisions::monotonic_chunk_updated_at(
        existing.updated_at,
        mongodb::bson::DateTime::now(),
    );

    // PUT is retained for compatibility but no longer replaces the storage
    // row. Only operator-editable metadata is set. Tenant identity, source
    // material/indexes, lifecycle, creation time and worker-owned catalog state
    // remain server-owned and cannot be cleared or moved by the request.
    let result = state
        .db
        .operation_knowledge_documents()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "version": expected_version,
            },
            doc! {
                "$set": {
                    "source_name": normalize_optional(payload.source_name),
                    "title": payload.title,
                    "summary": normalize_optional(payload.summary),
                    "catalog_summary": normalize_optional(payload.catalog_summary),
                    "routing_map": payload.routing_map,
                    "risk_notes": payload.risk_notes,
                    "product_tags": normalize_knowledge_tags(payload.product_tags, 5, false),
                    "business_topics": normalize_knowledge_tags(payload.business_topics, 3, false),
                    "version": next_version,
                    "updated_at": updated_at,
                }
            },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "knowledge_document_version_conflict".to_string(),
        ));
    }
    Ok(Json(json!({ "ok": true, "version": next_version })))
}

fn insert_patch_value<T: serde::Serialize>(
    update: &mut Document,
    key: &str,
    value: &T,
) -> AppResult<()> {
    update.insert(
        key,
        to_bson(value).map_err(|error| {
            AppError::External(format!("serialize document patch failed: {error}"))
        })?,
    );
    Ok(())
}

pub(in crate::routes) async fn patch_operation_knowledge_document(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OperationKnowledgeDocumentPatchRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let existing = state
        .db
        .operation_knowledge_documents()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge document not found".to_string()))?;
    if existing.version != payload.version {
        return Err(AppError::Conflict(
            "knowledge_document_version_conflict".to_string(),
        ));
    }

    let mut update = Document::new();
    match payload.source_name {
        DocumentMetadataPatch::Missing => {}
        DocumentMetadataPatch::Null => {
            if existing.source_name.is_some() {
                update.insert("source_name", Bson::Null);
            }
        }
        DocumentMetadataPatch::Value(value) => {
            let value = normalize_optional(Some(value));
            if value != existing.source_name {
                insert_patch_value(&mut update, "source_name", &value)?;
            }
        }
    }
    match payload.title {
        DocumentMetadataPatch::Missing => {}
        DocumentMetadataPatch::Null => {
            return Err(AppError::BadRequest("title cannot be null".to_string()));
        }
        DocumentMetadataPatch::Value(value) => {
            let value = value.trim().to_string();
            if value.is_empty() {
                return Err(AppError::BadRequest("title is required".to_string()));
            }
            if value != existing.title {
                update.insert("title", value);
            }
        }
    }
    for (key, patch, current) in [
        ("summary", payload.summary, existing.summary.as_ref()),
        (
            "catalog_summary",
            payload.catalog_summary,
            existing.catalog_summary.as_ref(),
        ),
    ] {
        match patch {
            DocumentMetadataPatch::Missing => {}
            DocumentMetadataPatch::Null => {
                if current.is_some() {
                    update.insert(key, Bson::Null);
                }
            }
            DocumentMetadataPatch::Value(value) => {
                let value = normalize_optional(Some(value));
                if value.as_ref() != current {
                    insert_patch_value(&mut update, key, &value)?;
                }
            }
        }
    }
    for (key, patch, current, max_len) in [
        (
            "routing_map",
            payload.routing_map,
            &existing.routing_map,
            50usize,
        ),
        (
            "risk_notes",
            payload.risk_notes,
            &existing.risk_notes,
            50usize,
        ),
        (
            "product_tags",
            payload.product_tags,
            &existing.product_tags,
            5usize,
        ),
        (
            "business_topics",
            payload.business_topics,
            &existing.business_topics,
            3usize,
        ),
    ] {
        match patch {
            DocumentMetadataPatch::Missing => {}
            DocumentMetadataPatch::Null => {
                if !current.is_empty() {
                    update.insert(key, Bson::Array(Vec::new()));
                }
            }
            DocumentMetadataPatch::Value(values) => {
                let values = normalize_knowledge_tags(values, max_len, false);
                if &values != current {
                    insert_patch_value(&mut update, key, &values)?;
                }
            }
        }
    }

    if update.is_empty() {
        return Ok(Json(json!({
            "ok": true,
            "unchanged": true,
            "version": existing.version,
        })));
    }
    let next_version = existing
        .version
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("knowledge_document_version_exhausted".to_string()))?;
    update.insert("version", next_version);
    update.insert(
        "updated_at",
        crate::knowledge_wiki::chunk_revisions::monotonic_chunk_updated_at(
            existing.updated_at,
            mongodb::bson::DateTime::now(),
        ),
    );
    let result = state
        .db
        .operation_knowledge_documents()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "version": existing.version,
            },
            doc! { "$set": update },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "knowledge_document_version_conflict".to_string(),
        ));
    }
    Ok(Json(json!({
        "ok": true,
        "unchanged": false,
        "version": next_version,
    })))
}

pub(in crate::routes) async fn delete_operation_knowledge_document(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result = archive_document_with_chunks(
        &state,
        &admin.current_workspace,
        &admin.username,
        object_id,
        &mut session,
    )
    .await;
    let (version, archived_chunk_ids) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    };
    commit_chunk_transaction(&mut session).await?;
    for chunk_id in &archived_chunk_ids {
        super::super::chunk_locks::broadcast_chunk_revised_in(
            &state,
            &admin.current_workspace,
            chunk_id,
            "archive",
            &admin.username,
        );
    }
    Ok(Json(json!({
        "ok": true,
        "archived": true,
        "version": version,
        "archivedChunks": archived_chunk_ids.len(),
    })))
}

async fn archive_document_with_chunks(
    state: &AppState,
    workspace_id: &str,
    actor: &str,
    document_id: mongodb::bson::oid::ObjectId,
    session: &mut ClientSession,
) -> AppResult<(i32, Vec<String>)> {
    let document = state
        .db
        .operation_knowledge_documents()
        .find_one_with_session(
            doc! { "_id": document_id, "workspace_id": workspace_id },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge document not found".to_string()))?;

    // Consume the session cursor before applying revisions: a SessionCursor
    // borrows the session while advancing and cannot share it with writes.
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find_with_session(
            doc! {
                "document_id": document_id,
                "workspace_id": workspace_id,
                "status": { "$ne": "archived" },
            },
            FindOptions::builder().sort(doc! { "_id": 1 }).build(),
            session,
        )
        .await?;
    let mut chunk_ids = Vec::new();
    while let Some(chunk) = cursor.next(session).await.transpose()? {
        let chunk_id = chunk
            .id
            .ok_or_else(|| AppError::External("knowledge chunk missing _id".to_string()))?;
        chunk_ids.push(chunk_id);
    }
    drop(cursor);

    let mut archived_chunk_ids = Vec::with_capacity(chunk_ids.len());
    for chunk_id in chunk_ids {
        let applied = apply_chunk_revision_with_session(
            &state.db,
            workspace_id,
            chunk_id,
            RevisionRequest {
                op: RevisionOp::Archive,
                source: ProvenanceSource::Human,
                patch: mongodb::bson::Document::new(),
                reason: Some(format!("parent document {} archived", document_id.to_hex())),
                actor: Some(actor.to_string()),
            },
            session,
        )
        .await?;
        archived_chunk_ids.push(applied.chunk_id);
    }

    let version = if document.status == "archived" {
        document.version
    } else {
        let next_version = document.version.checked_add(1).ok_or_else(|| {
            AppError::Conflict("knowledge_document_version_exhausted".to_string())
        })?;
        let updated_at = crate::knowledge_wiki::chunk_revisions::monotonic_chunk_updated_at(
            document.updated_at,
            mongodb::bson::DateTime::now(),
        );
        let update = state
            .db
            .operation_knowledge_documents()
            .update_one_with_session(
                doc! {
                    "_id": document_id,
                    "workspace_id": workspace_id,
                    "version": document.version,
                    "status": { "$ne": "archived" },
                },
                doc! {
                    "$set": {
                        "status": "archived",
                        "version": next_version,
                        "updated_at": updated_at,
                    }
                },
                None,
                session,
            )
            .await?;
        if update.matched_count != 1 {
            return Err(AppError::Conflict(
                "knowledge_document_version_conflict".to_string(),
            ));
        }
        next_version
    };
    Ok((version, archived_chunk_ids))
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

fn normalized_review_topic(value: &str) -> String {
    value.trim().to_lowercase()
}

fn review_categories_for_chunk(
    chunk: &crate::models::OperationKnowledgeChunk,
    available_chunk_ids: &HashSet<String>,
) -> Vec<&'static str> {
    let mut categories = Vec::with_capacity(4);
    if chunk.integrity_status.as_deref() == Some("rejected") {
        categories.push("contested");
    }
    if chunk.integrity_status.as_deref() == Some("needs_review") {
        categories.push("needs_review");
    }

    let has_quote = chunk
        .source_quote
        .as_deref()
        .is_some_and(|quote| !quote.trim().is_empty());
    // citable 口径（与 D2 verify 闸/读取侧同谓词）：只有畸形锚（缺 sourceQuote 键）
    // 的切片同样不可引用，必须报 source_orphan 而不是 pending_verification。
    let has_anchor = crate::models::chunk_has_citable_anchor(&chunk.source_anchors);
    if !has_quote || !has_anchor {
        categories.push("source_orphan");
    } else if chunk.integrity_status.as_deref() == Some("needs_review") {
        categories.push("pending_verification");
    }

    if chunk.related_chunks.as_ref().is_some_and(|relations| {
        relations
            .iter()
            .any(|relation| !available_chunk_ids.contains(&relation.chunk_id))
    }) {
        categories.push("dependents_pending");
    }
    categories
}

pub(in crate::routes) async fn list_operation_knowledge_review_queue(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<OperationKnowledgeReviewQueueQuery>,
) -> AppResult<Json<Value>> {
    let profile =
        crate::agent::load_active_domain_profile(&state.db, &admin.current_workspace).await?;
    let effective_dimension = match query.dimension.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(requested) => {
            let dimension = profile
                .coverage_dimensions
                .iter()
                .find(|dimension| dimension.key == requested)
                .ok_or_else(|| {
                    AppError::BadRequest("unknown_knowledge_review_dimension".to_string())
                })?;
            let mut aliases = Vec::with_capacity(dimension.review_topic_aliases.len() + 2);
            let mut normalized = HashSet::new();
            for alias in std::iter::once(dimension.key.as_str())
                .chain(std::iter::once(dimension.display_name.as_str()))
                .chain(dimension.review_topic_aliases.iter().map(String::as_str))
            {
                let normalized_alias = normalized_review_topic(alias);
                if normalized.insert(normalized_alias.clone()) {
                    aliases.push((alias.to_string(), normalized_alias));
                }
            }
            Some((
                dimension.key.clone(),
                dimension.display_name.clone(),
                aliases,
            ))
        }
    };

    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": "user_operations",
                "status": { "$ne": "archived" },
            },
            FindOptions::builder()
                .sort(doc! { "priority": -1_i32, "updated_at": -1_i32 })
                .build(),
        )
        .await?;
    let mut all_unarchived = Vec::new();
    while let Some(chunk) = cursor.try_next().await? {
        all_unarchived.push(chunk);
    }
    let available_chunk_ids: HashSet<String> = all_unarchived
        .iter()
        .filter_map(|chunk| chunk.id.map(|id| id.to_hex()))
        .collect();

    let mut category_counts = BTreeMap::from([
        ("contested", 0_u64),
        ("needs_review", 0_u64),
        ("source_orphan", 0_u64),
        ("pending_verification", 0_u64),
        ("dependents_pending", 0_u64),
    ]);
    let mut items = Vec::new();
    for chunk in all_unarchived {
        if !matches!(chunk.status.as_str(), "draft" | "active") {
            continue;
        }
        if let Some((_, _, aliases)) = &effective_dimension {
            let matches_dimension = chunk.business_topics.iter().any(|topic| {
                let topic = normalized_review_topic(topic);
                aliases
                    .iter()
                    .any(|(_, normalized_alias)| normalized_alias == &topic)
            });
            if !matches_dimension {
                continue;
            }
        }

        let categories = review_categories_for_chunk(&chunk, &available_chunk_ids);
        if categories.is_empty() {
            continue;
        }
        for category in &categories {
            if let Some(count) = category_counts.get_mut(category) {
                *count += 1;
            }
        }
        let mut projected = operation_knowledge_chunk_json(chunk);
        projected["reviewCategories"] = json!(categories);
        items.push(projected);
    }

    let dimension_filter = effective_dimension.map(|(key, label, aliases)| {
        json!({
            "key": key,
            "label": label,
            "topicAliases": aliases.into_iter().map(|(alias, _)| alias).collect::<Vec<_>>(),
        })
    });
    Ok(Json(json!({
        "items": items,
        "counts": category_counts,
        "effectiveFilter": {
            "workspaceId": admin.current_workspace,
            "domain": "user_operations",
            "lifecycleStatuses": ["draft", "active"],
            "dimension": dimension_filter,
        }
    })))
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
    // Generic create is an editing entrypoint, not a verification capability.
    // The dedicated /verify route is the only way into active+verified.
    payload.status = "draft".to_string();
    payload.integrity_status = Some("needs_review".to_string());
    payload.confidence_score = Some(0);
    let chunk =
        operation_knowledge_chunk_from_request(&state, &admin.current_workspace, payload, None)?;
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result: AppResult<(mongodb::bson::oid::ObjectId, String)> = async {
        if let Some(document_id) = chunk.document_id {
            state
                .db
                .operation_knowledge_documents()
                .find_one_with_session(
                    doc! {
                        "_id": document_id,
                        "workspace_id": &admin.current_workspace,
                        "status": { "$ne": "archived" },
                    },
                    None,
                    &mut session,
                )
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest("chunk parent document is missing or archived".to_string())
                })?;
        }
        let inserted = state
            .db
            .operation_knowledge_chunks()
            .insert_one_with_session(chunk, None, &mut session)
            .await?;
        let chunk_id = inserted.inserted_id.as_object_id().ok_or_else(|| {
            AppError::External("chunk insert did not return ObjectId".to_string())
        })?;
        let applied = apply_chunk_revision_with_session(
            &state.db,
            &admin.current_workspace,
            chunk_id,
            RevisionRequest {
                op: RevisionOp::Create,
                source: ProvenanceSource::Human,
                patch: mongodb::bson::Document::new(),
                reason: Some("manual chunk create".to_string()),
                actor: Some(admin.username.clone()),
            },
            &mut session,
        )
        .await?;
        Ok((chunk_id, applied.revision_id))
    }
    .await;
    let (chunk_id, revision_id) = match result {
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
        chunk_id.to_hex(),
        "create",
        &admin.username,
    );
    Ok(Json(json!({
        "id": chunk_id.to_hex(),
        "revisionId": revision_id,
        "status": "draft",
        "integrityStatus": "needs_review",
    })))
}

pub async fn update_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OperationKnowledgeChunkRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_chunk(&payload)?;
    let object_id = parse_object_id(&id)?;
    // Backward-compatible URL, narrow semantics: only editable content fields
    // are projected. Scope, lineage, type, lifecycle and review fields from the
    // legacy whole-object DTO are ignored.
    let mut patch = serde_json::Map::new();
    patch.insert("title".to_string(), json!(payload.title));
    for (key, value) in [
        ("knowledgeType", payload.knowledge_type),
        ("businessContext", payload.business_context),
        ("summary", payload.summary),
        ("body", payload.body),
        ("sourceQuote", payload.source_quote),
    ] {
        if let Some(value) = value {
            patch.insert(key.to_string(), json!(value));
        }
    }
    for (key, values) in [
        ("applicableScenes", payload.applicable_scenes),
        ("notApplicableScenes", payload.not_applicable_scenes),
        ("productTags", payload.product_tags),
        ("businessTopics", payload.business_topics),
    ] {
        if !values.is_empty() {
            patch.insert(key.to_string(), json!(values));
        }
    }
    if payload.priority != 0 {
        patch.insert("priority".to_string(), json!(payload.priority));
    }
    let applied = apply_controlled_chunk_patch(
        &state,
        &admin.current_workspace,
        &admin.username,
        object_id,
        &Value::Object(patch),
        ProvenanceSource::Human,
        Some("legacy chunk PUT".to_string()),
    )
    .await?;
    Ok(Json(json!({
        "ok": true,
        "revisionId": applied.revision_id,
        "status": "draft",
        "integrityStatus": "needs_review",
    })))
}

pub(in crate::routes) async fn delete_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
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
    if existing.status == "archived" {
        return Ok(Json(json!({
            "ok": true,
            "archived": true,
            "unchanged": true,
            "revisionId": Value::Null,
        })));
    }
    let applied = apply_chunk_revision(
        &state.db,
        &admin.current_workspace,
        object_id,
        RevisionRequest {
            op: RevisionOp::Archive,
            source: ProvenanceSource::Human,
            patch: mongodb::bson::Document::new(),
            reason: Some("legacy DELETE archived chunk".to_string()),
            actor: Some(admin.username.clone()),
        },
    )
    .await?;
    super::super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "archive",
        &admin.username,
    );
    Ok(Json(json!({
        "ok": true,
        "archived": true,
        "unchanged": applied.unchanged,
        "revisionId": applied.revision_id,
    })))
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
    Ok(Json(serde_json::json!({
        "item": operation_knowledge_chunk_json(item)
    })))
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
mod review_category_tests {
    use super::review_categories_for_chunk;
    use crate::models::OperationKnowledgeChunk;
    use std::collections::HashSet;

    fn base_chunk() -> OperationKnowledgeChunk {
        OperationKnowledgeChunk {
            workspace_id: "ws-1".into(),
            title: "价格政策".into(),
            domain: "user_operations".into(),
            status: "active".into(),
            source_quote: Some("原文片段".into()),
            integrity_status: Some("needs_review".into()),
            ..Default::default()
        }
    }

    /// B4：畸形锚（有 anchor 元素但缺非空 `sourceQuote`）不可被引用，审核队列
    /// 必须归类 `source_orphan`（等价于没有来源），而不是 `pending_verification`
    /// ——旧口径裸 `!is_empty()` 会把这类切片错报成「待核验」，运营看不出真因。
    #[test]
    fn malformed_anchor_is_source_orphan_not_pending_verification() {
        let mut chunk = base_chunk();
        chunk.source_anchors = vec![mongodb::bson::doc! { "startOffset": 0i64 }];
        let categories = review_categories_for_chunk(&chunk, &HashSet::new());
        assert!(categories.contains(&"source_orphan"), "{categories:?}");
        assert!(
            !categories.contains(&"pending_verification"),
            "{categories:?}"
        );
    }

    /// 对偶正例：可引用锚 + 原文 + needs_review → pending_verification（不过度收紧）。
    #[test]
    fn citable_anchor_with_quote_stays_pending_verification() {
        let mut chunk = base_chunk();
        chunk.source_anchors = vec![mongodb::bson::doc! { "sourceQuote": "原文片段" }];
        let categories = review_categories_for_chunk(&chunk, &HashSet::new());
        assert!(
            categories.contains(&"pending_verification"),
            "{categories:?}"
        );
        assert!(!categories.contains(&"source_orphan"), "{categories:?}");
    }
}

#[cfg(test)]
mod contract_tests {
    use crate::models::OperationKnowledgeChunk;
    use mongodb::bson::{oid::ObjectId, DateTime};

    /// 详情端点与列表共用 `operation_knowledge_chunk_json`，确保 deep-link 审核卡
    /// 也能获得 RFC3339 `updatedAt` 版本令牌，而不是 BSON Extended JSON。
    #[test]
    fn chunk_detail_projection_matches_contract_fixture() {
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
        let projected = serde_json::json!({
            "item": super::super::operation_knowledge_chunk_json(chunk)
        });
        crate::routes::contract_snapshot::assert_contract_fixture(
            "operation_knowledge_chunk_detail",
            projected,
        );
    }
}

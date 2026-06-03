//! 运营知识库路由：文档 / 切片 / 条目的全生命周期管理。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, Bson, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use std::sync::Arc;

use crate::{
    auth::AuthenticatedAdmin,
    agent,
    error::{AppError, AppResult},
    llm::LlmProvider,
    models::{
        KnowledgeChatTurn, KnowledgeUsageLog, OperationKnowledgeChunk, OperationKnowledgeDocument,
    },
    prompts,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // HTTP schema：保留兼容旧前端 410 占位 endpoint query
pub(super) struct OperationKnowledgeQuery {
    account_id: Option<String>,
    category: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationKnowledgeDocumentQuery {
    account_id: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationKnowledgeChunkQuery {
    account_id: Option<String>,
    document_id: Option<String>,
    item_id: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationKnowledgeDocumentRequest {
    account_id: Option<String>,
    #[serde(default = "default_user_operations_domain")]
    domain: String,
    #[serde(default = "default_imported_markdown_source_type")]
    source_type: String,
    source_name: Option<String>,
    title: String,
    summary: Option<String>,
    catalog_summary: Option<String>,
    #[serde(default)]
    routing_map: Vec<String>,
    #[serde(default)]
    risk_notes: Vec<String>,
    /// 文档级聚合标签（≤5），通常由所有 chunks 的 product_tags 去重并集而来。
    #[serde(default)]
    product_tags: Vec<String>,
    /// 文档级业务主题（≤3）。
    #[serde(default)]
    business_topics: Vec<String>,
    raw_content: Option<String>,
    content_hash: Option<String>,
    #[serde(default)]
    line_index: Vec<Document>,
    #[serde(default)]
    section_index: Vec<Document>,
    #[serde(default = "default_active_status")]
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // HTTP schema：保留兼容旧前端 410 占位 endpoint payload
pub(super) struct OperationKnowledgeRequest {
    account_id: Option<String>,
    /// 父文档 ObjectId 字符串（import-apply 自动注入；直接 PUT/POST 时可为空）。
    #[serde(default)]
    document_id: Option<String>,
    #[serde(default = "default_user_operations_domain")]
    domain: String,
    #[serde(default)]
    category: String,
    #[serde(default = "default_mixed_business_type")]
    business_type: String,
    knowledge_type: Option<String>,
    business_context: Option<String>,
    title: String,
    summary: Option<String>,
    body: Option<String>,
    routing_card: Option<String>,
    #[serde(default)]
    applicable_scenes: Vec<String>,
    #[serde(default)]
    not_applicable_scenes: Vec<String>,
    #[serde(default)]
    suitable_for: Vec<String>,
    #[serde(default)]
    not_suitable_for: Vec<String>,
    #[serde(default)]
    customer_stages: Vec<String>,
    #[serde(default)]
    operation_states: Vec<String>,
    #[serde(default)]
    intent_levels: Vec<String>,
    #[serde(default)]
    safe_claims: Vec<String>,
    #[serde(default)]
    forbidden_claims: Vec<String>,
    #[serde(default)]
    common_questions: Vec<String>,
    #[serde(default)]
    common_objections: Vec<String>,
    #[serde(default)]
    evidence_items: Vec<String>,
    /// 知识标签（≤5）。
    #[serde(default)]
    product_tags: Vec<String>,
    /// 业务主题（≤3）。
    #[serde(default)]
    business_topics: Vec<String>,
    #[serde(default = "default_manual_source_type")]
    source_type: String,
    source_name: Option<String>,
    #[serde(default = "default_active_status")]
    status: String,
    #[serde(default)]
    priority: i32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // HTTP schema：routing_card / forbidden_claims 字段保留前端兼容
pub(super) struct OperationKnowledgeChunkRequest {
    account_id: Option<String>,
    document_id: Option<String>,
    item_id: Option<String>,
    #[serde(default = "default_user_operations_domain")]
    domain: String,
    knowledge_type: Option<String>,
    business_context: Option<String>,
    title: String,
    summary: Option<String>,
    body: Option<String>,
    routing_card: Option<String>,
    #[serde(default)]
    applicable_scenes: Vec<String>,
    #[serde(default)]
    not_applicable_scenes: Vec<String>,
    #[serde(default)]
    safe_claims: Vec<String>,
    #[serde(default)]
    forbidden_claims: Vec<String>,
    #[serde(default)]
    evidence_items: Vec<String>,
    /// 知识标签（≤5）：产品名/解决方案，LLM 自动抽取，后台可编辑。
    #[serde(default)]
    product_tags: Vec<String>,
    /// 业务主题（≤3）。
    #[serde(default)]
    business_topics: Vec<String>,
    source_quote: Option<String>,
    #[serde(default)]
    source_anchors: Vec<Document>,
    integrity_status: Option<String>,
    confidence_score: Option<i32>,
    #[serde(default)]
    distortion_risks: Vec<String>,
    #[serde(default)]
    unsupported_claims: Vec<String>,
    #[serde(default)]
    verified_claims: Vec<String>,
    #[serde(default = "default_active_status")]
    status: String,
    #[serde(default)]
    priority: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationKnowledgeImportRequest {
    account_id: Option<String>,
    source_name: Option<String>,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationKnowledgeImportApplyRequest {
    account_id: Option<String>,
    source_name: Option<String>,
    document: Option<OperationKnowledgeDocumentRequest>,
    #[serde(default)]
    items: Vec<OperationKnowledgeRequest>,
    #[serde(default)]
    chunks: Vec<OperationKnowledgeChunkRequest>,
    /// knowledge-wiki Phase D：fence-aware 流式块导入。
    ///
    /// 当 caller 提供 `chunkedText` 时，会先 `parse_chunk_blocks` 解析
    /// `---CHUNK: id---...---END CHUNK---` 形式，然后把每块当作 chunk patch
    /// 走 `apply_chunk_revision(op=Create, source=Imported)` 落库 + 留 revision。
    /// 解析 warning（unsafe-id / 流截断 / 重复 id 等）通过 `parseWarnings` 字段
    /// 返回，**不**冒泡为 4xx。
    ///
    /// 与 `chunks` 字段并存：如果两者都给，先处理 `chunks`（旧 JSON 路径），
    /// 再追加 `chunkedText`（新流式路径）。
    chunked_text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KnowledgeToolSearchRequest {
    account_id: String,
    contact_id: Option<String>,
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KnowledgeToolOpenRequest {
    ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeVerifyRequest {
    verified_claims: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAutoVerifyRequest {
    account_id: Option<String>,
    /// 模型置信度阈值（0-10），≥ 该值才算 verified；默认 7。
    #[serde(default)]
    confidence_threshold: Option<i32>,
    /// 人工抽样概率，0.0-1.0；默认 0.1。
    #[serde(default)]
    human_audit_sample_rate: Option<f64>,
    /// 单次最多处理多少条 chunks，默认 50。
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationKnowledgeTestRequest {
    account_id: String,
    contact_id: Option<String>,
    message: String,
}

pub(super) async fn list_operation_knowledge(
    State(_state): State<AppState>,
    Query(_query): Query<OperationKnowledgeQuery>,
) -> AppResult<Json<Value>> {
    // operation_knowledge_items 已随 sales 旧库删除；旧 list 端口现在保持兼容
    // 形状但永远返回空集合。新的 wiki 流程走 operation_knowledge_chunks。
    Ok(Json(json!({ "items": Vec::<Value>::new() })))
}

pub(super) async fn list_operation_knowledge_documents(
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

pub(super) async fn create_operation_knowledge_document(
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

pub(super) async fn get_operation_knowledge_document(
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

pub(super) async fn update_operation_knowledge_document(
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

pub(super) async fn delete_operation_knowledge_document(
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

pub(super) async fn list_operation_knowledge_chunks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<OperationKnowledgeChunkQuery>,
) -> AppResult<Json<Value>> {
    let items =
        load_operation_knowledge_chunks_for_query(&state, &admin.current_workspace, query).await?;
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn list_operation_knowledge_document_chunks(
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

pub(super) async fn create_operation_knowledge_chunk(
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

pub(super) async fn update_operation_knowledge_chunk(
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

pub(super) async fn delete_operation_knowledge_chunk(
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

pub(super) async fn get_operation_knowledge_chunk_source(
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

pub async fn verify_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<KnowledgeVerifyRequest>,
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

    // D2 不变量：verify 之前必须有 sourceQuote 且能锚定到父文档（source_anchors 非空）。
    // 否则任何路径（运营 verify / AI 修复后 apply-and-verify / 老 UI verify）都不可越过。
    let has_quote = chunk
        .source_quote
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_anchor = !chunk.source_anchors.is_empty();
    if let Some(reason) = chunk_verify_gate_reason(has_quote, has_anchor) {
        return Err(AppError::BadRequest(reason));
    }

    let verified_claims = payload.verified_claims.unwrap_or_default();
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            doc! {
                "$set": {
                    "integrity_status": "verified",
                    "confidence_score": 100,
                    "verified_claims": string_bson_array(&verified_claims),
                    "unsupported_claims": Bson::Array(Vec::new()),
                    "status": "active",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn reject_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            doc! {
                "$set": {
                    "integrity_status": "rejected",
                    "confidence_score": 0,
                    "status": "rejected",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// MP-9 / Task 16：批量调用 LLM 对 `needs_review` 的 chunks 自动校验。
///
/// - 串行处理，避免并发烧 token；
/// - confidence ≥ threshold 自动标 `verified`，否则保持 `needs_review`；
/// - 按 `1/N` 概率把判定结果改成 `needs_human_audit` 走 admin 抽查；
/// - 写一条 `agent_events kind="knowledge_auto_verify_done"`。
pub async fn auto_verify_operation_knowledge_chunks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<KnowledgeAutoVerifyRequest>,
) -> AppResult<Json<Value>> {
    let account_id = payload
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let threshold = payload.confidence_threshold.unwrap_or(7).clamp(0, 10);
    let sample_rate = payload
        .human_audit_sample_rate
        .unwrap_or(0.1)
        .clamp(0.0, 1.0);
    let limit = payload.limit.unwrap_or(50).clamp(1, 500);

    let (token_budget, max_llm_calls) =
        auto_verify_budget_limits(&state, &admin.current_workspace).await?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let budget = Arc::new(agent::RunBudget::new(
        run_id.clone(),
        token_budget,
        max_llm_calls,
        // agent-autonomy-loop W3 / Task 4.1：auto_verify 路径不进入 tool-loop，
        // 用 i32::MAX 表示"不限 tool call 次数"，等价于关闭 R4.3 的 tool 维度
        // 硬上限；该字段仍参与 record_tool_call 累加，仅不会先于其它维度饱和。
        i32::MAX,
    ));
    let workspace_id = admin.current_workspace.clone();
    agent::RUN_BUDGET
        .scope(
            budget.clone(),
            auto_verify_operation_knowledge_chunks_inner(
                state,
                workspace_id,
                account_id,
                threshold,
                sample_rate,
                limit,
                run_id,
                budget,
            ),
        )
        .await
}

async fn auto_verify_budget_limits(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<(i64, i32)> {
    let config = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations"
            },
            None,
        )
        .await?;
    let params = config.as_ref().map(|item| &item.runtime_parameters);
    // R15 / ISSUE-009：auto-verify 是批处理（一次跑 N 条 chunk），不能复用 user-ops
    // 单 run 内的 `runMaxLlmCalls`（默认 6，含义=单次会话 tool-call 预算）；
    // 否则 limit=50 会被默默缩到 6，degraded 直接触发 budget_exceeded。
    // 专属 key `autoVerifyMaxLlmCalls`，默认 100；token 预算同样独立。
    Ok((
        doc_i64_with_default(params, "autoVerifyTokenBudget", 240000),
        doc_i32_with_default(params, "autoVerifyMaxLlmCalls", 100).max(1),
    ))
}

fn doc_i64_with_default(doc: Option<&Document>, key: &str, default: i64) -> i64 {
    doc.and_then(|item| {
        item.get_i64(key)
            .ok()
            .or_else(|| item.get_i32(key).ok().map(i64::from))
    })
    .unwrap_or(default)
}

fn doc_i32_with_default(doc: Option<&Document>, key: &str, default: i32) -> i32 {
    doc.and_then(|item| {
        item.get_i32(key).ok().or_else(|| {
            item.get_i64(key)
                .ok()
                .and_then(|value| i32::try_from(value).ok())
        })
    })
    .unwrap_or(default)
}

async fn auto_verify_operation_knowledge_chunks_inner(
    state: AppState,
    workspace_id: String,
    account_id: String,
    threshold: i32,
    sample_rate: f64,
    limit: i64,
    run_id: String,
    budget: Arc<agent::RunBudget>,
) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &workspace_id,
                "domain": "user_operations",
                "integrity_status": { "$in": ["needs_review", null] },
                "$or": [
                    { "account_id": null },
                    { "account_id": &account_id }
                ]
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;

    let system = prompts::load_prompt(
        &state.db,
        &workspace_id,
        "knowledge.auto_verify",
    )
    .await
    .unwrap_or_else(|_| {
        "你是 WechatAgent 知识库自动校验 Agent。只输出严格 JSON。只有 sourceQuote 非空且 sourceAnchors 可定位来源时，才允许 verified。".to_string()
    });

    let mut verified = 0i32;
    let mut needs_review = 0i32;
    let mut rejected = 0i32;
    let mut needs_human_audit = 0i32;
    let mut processed = 0i32;
    let mut degraded = false;

    while let Some(chunk) = cursor.try_next().await? {
        let Some(chunk_id) = chunk.id else { continue };
        if budget.is_exceeded() {
            budget.mark_degraded("knowledge_auto_verify_stopped_budget_exceeded");
            degraded = true;
            break;
        }
        let has_source_quote = chunk
            .source_quote
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some();
        let has_source_anchor = !chunk.source_anchors.is_empty();
        let user = format!(
            r#"请对下面这条知识切片做自动校验。
切片 ID: {}
标题: {}
摘要: {}
正文: {}
source_quote: {}
source_anchors: {}

输出 JSON：
{{
  "confidenceScore": 0,
  "integrityStatus": "verified",
  "verifiedClaims": [],
  "distortionRisks": []
}}"#,
            chunk_id.to_hex(),
            chunk.title,
            chunk.summary.clone().unwrap_or_default(),
            chunk.body.clone().unwrap_or_default(),
            chunk.source_quote.clone().unwrap_or_default(),
            serde_json::to_string(&chunk.source_anchors).unwrap_or_default(),
        );

        let value = match agent::generate_agent_json(
            &state,
            Some(&account_id),
            None,
            Some(&run_id),
            "knowledge.auto_verify",
            &system,
            &user,
        )
        .await
        {
            Ok(v) => v,
            Err(_) => {
                // 单条失败不阻断整体；保留原状态，进入下一条。
                continue;
            }
        };
        processed += 1;

        let confidence = value
            .get("confidenceScore")
            .or_else(|| value.get("confidence_score"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let model_status = value
            .get("integrityStatus")
            .or_else(|| value.get("integrity_status"))
            .and_then(|v| v.as_str())
            .unwrap_or("needs_review")
            .to_string();
        let verified_claims_json = value
            .get("verifiedClaims")
            .or_else(|| value.get("verified_claims"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let distortion_risks_json = value
            .get("distortionRisks")
            .or_else(|| value.get("distortion_risks"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // 决定最终 status：必须有原文引用和锚点，threshold + 抽样改 needs_human_audit。
        let mut final_status =
            decide_auto_verify_status(has_source_quote, has_source_anchor, confidence, threshold, &model_status);
        if final_status == "verified" && sample_rate > 0.0 && fastrand::f64() < sample_rate {
            final_status = "needs_human_audit".to_string();
        }

        match final_status.as_str() {
            "verified" => verified += 1,
            "rejected" => rejected += 1,
            "needs_human_audit" => needs_human_audit += 1,
            _ => needs_review += 1,
        }

        let _ = state
            .db
            .operation_knowledge_chunks()
            .update_one(
                doc! { "_id": chunk_id },
                doc! {
                    "$set": {
                        "integrity_status": &final_status,
                        "confidence_score": confidence,
                        "verified_claims": string_bson_array(&verified_claims_json),
                        "distortion_risks": string_bson_array(&distortion_risks_json),
                        "updated_at": DateTime::now()
                    }
                },
                None,
            )
            .await;
        let _ = state
            .db
            .knowledge_usage_logs()
            .insert_one(
                KnowledgeUsageLog {
                    id: None,
                    workspace_id: workspace_id.clone(),
                    account_id: account_id.clone(),
                    contact_wxid: None,
                    run_id: run_id.clone(),
                    knowledge_ids: vec![chunk_id],
                    route_result: doc! {
                        "kind": "knowledge_auto_verify",
                        "promptKey": "knowledge.auto_verify",
                        "chunkId": chunk_id.to_hex(),
                        "confidenceScore": confidence,
                        "modelStatus": model_status,
                        "finalStatus": &final_status,
                        "hasSourceQuote": has_source_quote,
                        "hasSourceAnchor": has_source_anchor,
                    },
                    reply_text: None,
                    review_approved: final_status == "verified",
                    blocked_reason: if final_status == "verified" {
                        None
                    } else {
                        Some("knowledge_auto_verify_not_verified".to_string())
                    },
                    tool_trace: vec![doc! {
                        "sourceAnchorCount": chunk.source_anchors.len() as i32,
                        "sourceQuotePresent": has_source_quote,
                    }],
                    created_at: DateTime::now(),
                },
                None,
            )
            .await;
    }

    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: workspace_id.clone(),
                account_id: account_id.clone(),
                contact_wxid: None,
                kind: "knowledge_auto_verify_done".to_string(),
                status: "success".to_string(),
                summary: format!(
                    "自动校验完成：verified={verified} needs_review={needs_review} rejected={rejected} needs_human_audit={needs_human_audit}"
                ),
                details: Some(doc! {
                    "processed": processed,
                    "verified": verified,
                    "needsReview": needs_review,
                    "rejected": rejected,
                    "needsHumanAudit": needs_human_audit,
                    "confidenceThreshold": threshold,
                    "humanAuditSampleRate": sample_rate,
                    "degraded": degraded,
                    "budget": budget_document(&budget)
                }),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;

    Ok(Json(json!({
        "processed": processed,
        "verified": verified,
        "needsReview": needs_review,
        "rejected": rejected,
        "needsHumanAudit": needs_human_audit,
        "degraded": degraded,
        "budget": budget_document(&budget)
    })))
}

fn budget_document(budget: &agent::RunBudget) -> Document {
    let snapshot = budget.snapshot();
    doc! {
        "runId": snapshot.run_id,
        "tokenBudget": snapshot.token_budget,
        "tokensUsed": snapshot.tokens_used,
        "maxLlmCalls": snapshot.max_llm_calls,
        "llmCallsUsed": snapshot.llm_calls_used,
        "degradedReasons": snapshot.degraded_reasons,
    }
}

pub(super) async fn get_operation_knowledge_catalog(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let catalog =
        build_operation_knowledge_catalog(&state, &admin.current_workspace, &account_id).await?;
    Ok(Json(json!({ "item": catalog })))
}

/// `GET /api/operation-knowledge/catalog/persisted` —— knowledge-wiki Phase E：
/// 读 `documents.catalog_summary_persisted` 持久化快照，O(1)。
///
/// 返回每个 active document 的 `id / title / catalogVersion / catalogSummaryPersisted`。
/// 若 catalog_rebuild_worker 还没跑过 → `catalogSummaryPersisted=null`，
/// 调用方应回退到 `/catalog`（live 聚合）。
pub(super) async fn get_operation_knowledge_catalog_persisted(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let account_filter = vec![
        doc! { "account_id": null },
        doc! { "account_id": &account_id },
    ];
    let mut cursor = state
        .db
        .operation_knowledge_documents()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": "user_operations",
                "status": "active",
                "$or": account_filter,
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut documents = Vec::new();
    while let Some(d) = cursor.try_next().await? {
        documents.push(json!({
            "id": d.id.map(|id| id.to_hex()).unwrap_or_default(),
            "title": d.title,
            "catalogVersion": d.catalog_version,
            "catalogSummaryPersisted": d.catalog_summary_persisted,
            "updatedAt": crate::models::dt_to_string(d.updated_at).unwrap_or_default(),
        }));
    }
    Ok(Json(json!({ "documents": documents })))
}

pub(super) async fn get_operation_knowledge_completeness(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let item =
        build_operation_knowledge_completeness(&state, &admin.current_workspace, &account_id)
            .await?;
    Ok(Json(json!({ "item": item })))
}

pub(super) async fn refresh_operation_knowledge_completeness(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let item =
        build_operation_knowledge_completeness(&state, &admin.current_workspace, &account_id)
            .await?;
    Ok(Json(json!({ "item": item })))
}

pub(super) async fn get_operation_knowledge_integrity_report(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": "user_operations",
                "$or": [
                    { "account_id": null },
                    { "account_id": account_id }
                ]
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(500)
                .build(),
        )
        .await?;
    let mut total = 0;
    let mut verified = 0;
    let mut needs_review = 0;
    let mut rejected = 0;
    let mut items = Vec::new();
    while let Some(chunk) = cursor.try_next().await? {
        total += 1;
        match chunk.integrity_status.as_deref().unwrap_or("needs_review") {
            "verified" => verified += 1,
            "rejected" => rejected += 1,
            _ => needs_review += 1,
        }
        if chunk.integrity_status.as_deref() != Some("verified") {
            items.push(json!({
                "id": chunk.id.map(|id| id.to_hex()).unwrap_or_default(),
                "title": chunk.title,
                "integrityStatus": chunk.integrity_status.unwrap_or_else(|| "needs_review".to_string()),
                "confidenceScore": chunk.confidence_score.unwrap_or_default(),
                "status": chunk.status
            }));
        }
    }
    Ok(Json(json!({
        "item": {
            "total": total,
            "verified": verified,
            "needsReview": needs_review,
            "rejected": rejected,
            "items": items
        }
    })))
}

pub(super) async fn search_operation_knowledge_tool(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<KnowledgeToolSearchRequest>,
) -> AppResult<Json<Value>> {
    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("query is required".to_string()));
    }
    let contact = if let Some(contact_id) = payload.contact_id {
        Some(find_contact_by_id(&state, &admin.current_workspace, &contact_id).await?)
    } else {
        None
    };
    let result = agent::test_knowledge_route_for_contact(
        &state,
        contact,
        &payload.account_id,
        &payload.query,
    )
    .await?;
    Ok(Json(json!({ "item": result })))
}

pub(super) async fn open_operation_knowledge_slices(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<KnowledgeToolOpenRequest>,
) -> AppResult<Json<Value>> {
    let ids = payload
        .ids
        .into_iter()
        .filter_map(|id| ObjectId::parse_str(id).ok())
        .collect::<Vec<_>>();
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "_id": { "$in": ids }
            },
            FindOptions::builder()
                .sort(doc! { "priority": -1, "updated_at": -1 })
                .limit(50)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(operation_knowledge_chunk_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_operation_knowledge(
    State(_state): State<AppState>,
    Json(_payload): Json<OperationKnowledgeRequest>,
) -> AppResult<Json<Value>> {
    // operation_knowledge_items 已随 sales 旧库删除；保留 410 行为占位。
    Err(AppError::BadRequest(
        "operation_knowledge_items has been removed; use operation_knowledge_chunks instead"
            .to_string(),
    ))
}

pub(super) async fn update_operation_knowledge(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
    Json(_payload): Json<OperationKnowledgeRequest>,
) -> AppResult<Json<Value>> {
    Err(AppError::BadRequest(
        "operation_knowledge_items has been removed; use operation_knowledge_chunks instead"
            .to_string(),
    ))
}

pub(super) async fn delete_operation_knowledge(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> AppResult<Json<Value>> {
    Err(AppError::BadRequest(
        "operation_knowledge_items has been removed; use operation_knowledge_chunks instead"
            .to_string(),
    ))
}

pub async fn import_operation_knowledge_preview(
    State(state): State<AppState>,
    Json(payload): Json<OperationKnowledgeImportRequest>,
) -> AppResult<Json<Value>> {
    if payload.content.trim().is_empty() {
        return Err(AppError::BadRequest("content is required".to_string()));
    }
    let system = "你是企业微信运营知识库导入 Agent。你把长文本拆成 Agent 可渐进查询的文档目录、知识包、知识切片和证据块。只输出严格 JSON。";
    let source_name = payload
        .source_name
        .clone()
        .unwrap_or_else(|| "导入文本".to_string());
    let user = format!(
        r#"请把下面文本拆分为渐进式运营知识。输出 JSON：
{{
  "document": {{
    "domain": "user_operations",
    "sourceType": "imported_markdown",
    "sourceName": "{}",
    "title": "",
    "summary": "",
    "catalogSummary": "给 Agent 看的目录摘要，说明这份文档解决什么问题、何时应该打开",
    "routingMap": ["自然语言目录项，不使用固定分类"],
    "riskNotes": ["不能承诺、证据不足或需要 admin 后台确认的风险点"],
    "productTags": ["产品/品牌/解决方案名称，最多 5 个，可空"],
    "businessTopics": ["业务主题（如 产品定位差异 / 竞品对比 / 部署方式），最多 3 个，可空"],
    "status": "draft"
  }},
  "items": [
    {{
      "domain": "user_operations",
      "category": "用自然语言生成的主题标签，不要使用固定枚举",
      "businessType": "用自然语言说明业务语境，不要使用固定枚举",
      "knowledgeType": "AI 自主生成的知识类型",
      "businessContext": "这条知识适合的业务上下文",
      "title": "",
      "summary": "",
      "body": "",
      "routingCard": "什么时候应该使用这条知识，什么时候不该使用",
      "applicableScenes": [],
      "notApplicableScenes": [],
      "suitableFor": [],
      "notSuitableFor": [],
      "customerStages": [],
      "operationStates": [],
      "intentLevels": [],
      "safeClaims": [],
      "forbiddenClaims": [],
      "commonQuestions": [],
      "commonObjections": [],
      "evidenceItems": [],
      "productTags": ["最多 5 个，可空"],
      "businessTopics": ["最多 3 个，可空"],
      "sourceType": "imported_markdown",
      "sourceName": "{}",
      "status": "draft",
      "priority": 0
    }}
  ],
  "chunks": [
    {{
      "domain": "user_operations",
      "knowledgeType": "AI 自主生成的切片类型",
      "businessContext": "业务上下文",
      "title": "",
      "summary": "",
      "body": "可被 Agent 按需打开的原文要点或经过整理的知识正文",
      "routingCard": "什么时候打开这个切片",
      "applicableScenes": [],
      "notApplicableScenes": [],
      "safeClaims": [],
      "forbiddenClaims": [],
      "evidenceItems": [],
      "productTags": ["如：WechatAgent / AI 私域销售助手；最多 5 个；可空"],
      "businessTopics": ["如：产品定位差异 / 竞品对比；最多 3 个；可空"],
      "sourceQuote": "如有必要，保留支撑该切片的原文短句",
      "status": "draft",
      "priority": 0
    }}
  ]
}}

要求：
- 不要用固定枚举分类；知识类型、适用场景、目录项都用自然语言生成。
- document 是整篇资料的目录入口；items 是主题包；chunks 是 Agent 运行时真正按需打开的知识切片。
- 穷尽且忠实抽取：原文中每一个量化事实（数字/比例/金额/期限/数量）及其**限定条件**（起售门槛、前置要求、适用范围、例外、有效期等）都必须落入对应 chunk 的 body，**绝不能丢掉限定条件**只留主数字（例："X 元起，含 N 个起"必须连"含 N 个起"一起保留）。一条原子承载一个规格/事实时尤其要完整。
- 穷尽覆盖的对象不止量化事实：原文里每一个**离散信息单元**都要落地，不要因为它没有数字就漏掉。离散信息单元包括但不限于——决议/结论、动作项/待办及其**责任人与截止日期**、分项条款、流程步骤、各方观点、适用与不适用条件。例如会议纪要类文档，每一条决议、每一项待办（连同谁负责、何时完成）都必须各自落入 body，绝不能只总结成一句"会上讨论了若干事项"。判断标准：原文每一个可独立成立、能被单独追溯核对的陈述，都应在抽取结果里找得到对应内容。
- 只忠于原文：body、summary、safeClaims、evidenceItems 只能包含原文已陈述的内容，**禁止补充原文没有的描述、范围、功能、优惠条件或推断**。拿不准是否在原文里，就不写。
- safeClaims 必须是有依据、可安全对客户表达的事实。
- forbiddenClaims 必须列出不能承诺、不能暗示、不能编造的内容。
- 案例、报价、效果数据必须进入 evidenceItems；没有证据不要编造成案例。
- routingCard 要短，供运行时知识工具选择使用，不要堆正文。
- productTags / businessTopics 用于运行时把用户消息匹配到对应 chunk。
- document 级 productTags / businessTopics 可以是其下所有 chunks 的去重并集，也可由 LLM 自行抽取。

导入文本：
{}"#,
        source_name, source_name, payload.content
    );
    let value = agent::generate_agent_json(
        &state,
        payload.account_id.as_deref(),
        None,
        None,
        "knowledge.import.preview",
        system,
        &user,
    )
    .await?;
    let document = value
        .get("document")
        .cloned()
        .map(|item| normalize_operation_knowledge_preview_document(item, &payload))
        .unwrap_or_else(|| default_operation_knowledge_preview_document(&payload));
    let items = value
        .get("items")
        .and_then(|item| item.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| normalize_operation_knowledge_preview_item(item, &payload))
        .collect::<Vec<_>>();
    let mut chunks = value
        .get("chunks")
        .and_then(|item| item.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| normalize_operation_knowledge_preview_chunk(item, &payload))
        .collect::<Vec<_>>();
    let integrity_report = integrity_report_for_preview(&payload.content, &mut chunks);
    Ok(Json(
        json!({ "document": document, "items": items, "chunks": chunks, "integrityReport": integrity_report }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractKnowledgeTagsRequest {
    account_id: Option<String>,
    title: Option<String>,
    body: String,
}

/// `POST /api/operation-knowledge/extract-tags` —— 给单条 chunk 抽取
/// productTags / businessTopics 两字段。复用与 import-preview
/// 同样的 LLM prompt 风格，作为 backfill / 单条重抽入口。
///
/// 输入：`{ accountId?, title?, body }`
/// 输出：`{ productTags: [], businessTopics: [] }`
pub async fn extract_operation_knowledge_tags(
    State(state): State<AppState>,
    Json(payload): Json<ExtractKnowledgeTagsRequest>,
) -> AppResult<Json<Value>> {
    if payload.body.trim().is_empty() {
        return Err(AppError::BadRequest("body is required".to_string()));
    }
    let title = payload
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "未命名知识切片".to_string());
    let system = "你是企业微信运营知识库的标签抽取 Agent。给定一个知识切片（标题 + 正文），抽取它的 productTags / businessTopics。只输出严格 JSON。";
    let user = format!(
        r#"请基于下面的知识切片抽取两个字段：

知识标题：{}

知识正文：
{}

输出 JSON：
{{
  "productTags": ["产品/品牌/解决方案名称，最多 5 个；正文确无具体产品/品牌时留空数组"],
  "businessTopics": ["业务主题，最多 3 个；既包括产品维度（如 产品定位差异 / 竞品对比 / 部署方式），也包括方法论/沟通维度（如 价格异议处理 / 销售话术 / 客户关系维护 / 需求澄清）"]
}}

要求：
- productTags 只放正文里**确实出现的**具体产品/品牌/解决方案名；纯方法论/话术正文没有产品名时留空数组，**不要硬塞**。
- businessTopics 概括这条知识"讲的是哪个业务主题"，方法论/话术类内容同样有主题（如价格异议处理、客户沟通），**至少抽 1 个**，不要因为没有产品就整体留空。
- 主题用贴合正文的自然语言短语，不跑题、不空泛。
- 只输出 JSON，不要解释。"#,
        title, payload.body
    );
    let value = agent::generate_agent_json(
        &state,
        payload.account_id.as_deref(),
        None,
        None,
        "knowledge.tags.extract",
        system,
        &user,
    )
    .await?;
    let product_tags = json_string_list(&value, "productTags")
        .or_else(|| json_string_list(&value, "product_tags"))
        .unwrap_or_default();
    let business_topics = json_string_list(&value, "businessTopics")
        .or_else(|| json_string_list(&value, "business_topics"))
        .unwrap_or_default();
    Ok(Json(json!({
        "productTags": normalize_knowledge_tags(product_tags, 5, false),
        "businessTopics": normalize_knowledge_tags(business_topics, 3, false),
    })))
}

pub(super) async fn import_operation_knowledge_apply(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<OperationKnowledgeImportApplyRequest>,
) -> AppResult<Json<Value>> {
    if payload.items.is_empty() && payload.chunked_text.as_deref().unwrap_or("").trim().is_empty() {
        return Err(AppError::BadRequest(
            "items or chunkedText are required".to_string(),
        ));
    }
    let mut document_id = None;
    let raw_content = payload
        .document
        .as_ref()
        .and_then(|document| document.raw_content.clone());
    if let Some(mut document) = payload.document {
        document.account_id = document.account_id.or(payload.account_id.clone());
        document.source_name = document.source_name.or(payload.source_name.clone());
        if document.status == "draft" {
            document.status = "active".to_string();
        }
        validate_operation_knowledge_document(&document)?;
        let result = state
            .db
            .operation_knowledge_documents()
            .insert_one(
                operation_knowledge_document_from_request(&state, &admin.current_workspace, document, None),
                None,
            )
            .await?;
        document_id = result.inserted_id.as_object_id();
    }
    // payload.items 路径已随 operation_knowledge_items 删除；保留空列表
    // 让 chunked_text / chunks 路径继续走。
    let item_ids: Vec<String> = Vec::new();
    let _ = payload.items;
    let mut chunk_ids = Vec::new();
    for mut chunk in payload.chunks {
        chunk.account_id = chunk.account_id.or(payload.account_id.clone());
        if chunk.document_id.is_none() {
            chunk.document_id = document_id.map(|id| id.to_hex());
        }
        if let (Some(raw), Some(document_id)) = (raw_content.as_deref(), document_id) {
            apply_chunk_integrity(&mut chunk, raw, Some(document_id));
        }
        // 红线"AI 永不自动 verify"：import 材料本身未经审核，apply_chunk_integrity
        // 拿 sourceQuote 锚定成功只说明"引用出自这份导入文本"，不等于已核实。无条件
        // 压回 draft + needs_review（保留算出的 source_anchors 作审核线索），与
        // ingest_chunked_text / chunked_text 分支一致，由运营 Inspector 二次确认。
        chunk.status = "draft".to_string();
        chunk.integrity_status = Some("needs_review".to_string());
        validate_operation_knowledge_chunk(&chunk)?;
        let result = state
            .db
            .operation_knowledge_chunks()
            .insert_one(
                operation_knowledge_chunk_from_request(&state, &admin.current_workspace, chunk, None)?,
                None,
            )
            .await?;
        if let Some(id) = result.inserted_id.as_object_id() {
            chunk_ids.push(id.to_hex());
        }
    }
    // ── knowledge-wiki Phase D：fence-aware chunked text 流式块导入 ───────
    let mut parse_warnings_json: Vec<Value> = Vec::new();
    if let Some(text) = payload.chunked_text.as_deref().filter(|s| !s.trim().is_empty()) {
        let (blocks, warnings) =
            crate::knowledge_wiki::block_parser::parse_chunk_blocks(text);
        for w in &warnings.items {
            parse_warnings_json.push(parse_warning_to_json(w));
        }
        for block in blocks {
            // payload 中一律期待 camelCase 字段名（与既有 OperationKnowledgeChunkRequest 一致）；
            // 关键缺省值由下面的 enrich + validate 兜底。
            let mut chunk_req: OperationKnowledgeChunkRequest =
                match serde_json::from_value::<OperationKnowledgeChunkRequest>(block.payload.clone()) {
                    Ok(c) => c,
                    Err(e) => {
                        parse_warnings_json.push(json!({
                            "kind": "blockToChunkRequestError",
                            "id": block.id,
                            "reason": format!("{e}"),
                        }));
                        continue;
                    }
                };
            chunk_req.account_id = chunk_req.account_id.or(payload.account_id.clone());
            if chunk_req.document_id.is_none() {
                chunk_req.document_id = document_id.map(|id| id.to_hex());
            }
            if let (Some(raw), Some(document_id_v)) = (raw_content.as_deref(), document_id) {
                apply_chunk_integrity(&mut chunk_req, raw, Some(document_id_v));
            }
            // 流式块走"AI/Imported source"；强制 draft + needs_review，对齐 CLAUDE.md
            // "AI 永不自动 verify" 硬约束。
            chunk_req.status = "draft".to_string();
            // 红线"AI 永不自动 verify"：无条件压回 needs_review，不接受 block 自带的
            // verified（apply_chunk_integrity 的锚点只作审核线索）。与 ingest_chunked_text 一致。
            chunk_req.integrity_status = Some("needs_review".to_string());
            if let Err(e) = validate_operation_knowledge_chunk(&chunk_req) {
                parse_warnings_json.push(json!({
                    "kind": "blockValidationError",
                    "id": block.id,
                    "reason": format!("{e}"),
                }));
                continue;
            }
            let result = state
                .db
                .operation_knowledge_chunks()
                .insert_one(
                    operation_knowledge_chunk_from_request(&state, &admin.current_workspace, chunk_req, None)?,
                    None,
                )
                .await?;
            if let Some(id) = result.inserted_id.as_object_id() {
                chunk_ids.push(id.to_hex());
                // 留 chunk_revisions(op=create, source=imported) 痕迹
                let req = RevisionRequest {
                    op: RevisionOp::Create,
                    source: ProvenanceSource::Imported,
                    patch: Document::new(),
                    reason: Some(format!("import_apply chunked block id={}", block.id)),
                    actor: payload.account_id.clone(),
                };
                if let Err(e) = apply_chunk_revision(
                    &state.db,
                    &admin.current_workspace,
                    id,
                    req,
                )
                .await
                {
                    tracing::warn!(
                        chunk_id = %id.to_hex(),
                        block_id = %block.id,
                        error = %e,
                        "import_apply: write chunk_revision failed (non-fatal)"
                    );
                }
            }
        }
    }
    Ok(Json(json!({
        "documentId": document_id.map(|id| id.to_hex()),
        "itemIds": item_ids,
        "chunkIds": chunk_ids,
        "parseWarnings": parse_warnings_json,
    })))
}

// ── P1-5 · multimodal 入口 ────────────────────────────────────────────────────
//
// 复用 `import_operation_knowledge_apply` 的 chunked-text 落库逻辑，把不同来源
// （PDF 字节 / 图片 base64 + LLM vision）先归一为 markdown / fence 文本，再交给
// 同一段写入路径。这样保持：
//   - "AI 永不自动 verify" 仍由原路径强制（status=draft + integrity=needs_review）
//   - 1 个 import id 出口与原 import-apply 一致
//   - 红线：fence 文本里的 chunk_id 仍需 admin 在前端 Inspector 二次审核
//
// 端点：
//   POST /operation-knowledge/import-apply-pdf   (multipart, file=...)
//   POST /operation-knowledge/import-apply-image (json, { imageBase64, mime })
//
// 仅当 active LlmProviderConfig.supportsVision==true 时才允许 import-apply-image；
// 否则 502 + visionNotSupported。

pub(super) async fn import_operation_knowledge_apply_pdf(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    mut multipart: axum::extract::Multipart,
) -> AppResult<Json<Value>> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut source_name: Option<String> = None;
    let mut account_id: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart 解析失败: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("读取上传文件失败: {e}")))?;
                file_bytes = Some(bytes.to_vec());
            }
            "sourceName" => {
                source_name = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("sourceName 字段读取失败: {e}")))?,
                );
            }
            "accountId" => {
                account_id = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("accountId 字段读取失败: {e}")))?,
                );
            }
            _ => {}
        }
    }
    let bytes = file_bytes
        .ok_or_else(|| AppError::BadRequest("缺少 file 字段（PDF 字节）".to_string()))?;
    let outcome = import_pdf_bytes(
        &state,
        &admin.current_workspace,
        account_id.as_deref(),
        source_name.as_deref().unwrap_or("uploaded_pdf"),
        bytes,
    )
    .await?;
    Ok(Json(json!({
        "documentId": outcome.document_id,
        "chunkIds": outcome.chunk_ids,
        "parseWarnings": outcome.parse_warnings,
        "fallbackBlob": outcome.fallback_blob,
    })))
}

/// PDF 字节 → 文本抽取 → `ingest_chunked_text` 落库的纯函数核心。
/// 从 multipart handler 抽出，便于集成测试（`tests/import_pdf_smoke.rs`）直接喂
/// PDF 字节、断言产出 chunk（multipart extractor 本身在测试里无法手工构造）。
pub async fn import_pdf_bytes(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    source_name: &str,
    bytes: Vec<u8>,
) -> AppResult<IngestOutcome> {
    if bytes.is_empty() {
        return Err(AppError::BadRequest("file 字段为空".to_string()));
    }
    // pdf-extract 是同步阻塞 API，扔到 spawn_blocking 避免堵 tokio 调度器。
    let extracted = tokio::task::spawn_blocking(move || pdf_extract::extract_text_from_mem(&bytes))
        .await
        .map_err(|e| AppError::External(format!("PDF 抽取任务 join 失败: {e}")))?
        .map_err(|e| AppError::BadRequest(format!("PDF 解析失败: {e}")))?;
    if extracted.trim().is_empty() {
        return Err(AppError::BadRequest(
            "PDF 抽取后文本为空（可能是扫描件 / 加密文档）".to_string(),
        ));
    }
    ingest_chunked_text(state, workspace_id, account_id, source_name, &extracted).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportApplyImageRequest {
    pub image_base64: String,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub source_name: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    /// 可选 hint：让 LLM 在抽取时偏向某个领域。
    #[serde(default)]
    pub hint: Option<String>,
}

/// 视觉模型解析结果：要么复用运行时 active provider（文字主模型本身支持图片），
/// 要么用 workspace 指派的视觉副模型构造的候选链。`Dedicated` 携带按优先级排好序的
/// 一次性 client 列表（专职视觉模型在前，其余支持视觉的备用模型在后），主模型瞬时
/// 不可达时依次自动切换到下一候选，全部失败才向上游报错。`String` 是该候选的 model
/// 名，仅用于切换日志（运行时 DB 值，非源码字面量）。
enum VisionProvider {
    Runtime,
    Dedicated(Vec<(String, crate::llm::LlmClient)>),
}

pub async fn import_operation_knowledge_apply_image(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(req): Json<ImportApplyImageRequest>,
) -> AppResult<Json<Value>> {
    if req.image_base64.trim().is_empty() {
        return Err(AppError::BadRequest("imageBase64 不能为空".to_string()));
    }
    // 1) 解析视觉模型：
    //    a. 若 active 文字主模型本身 supports_vision → 直接用运行时 state.llm。
    //    b. 否则收集本 workspace 所有支持视觉的副模型（supports_vision=true），
    //       专职视觉模型（is_vision_active=true）排在最前，其余按 updated_at 倒序
    //       作为自动切换备用，构造候选 client 链。
    //    c. 一条都没有 → 502 visionNotSupported，让运营去模型设置里配视觉模型。
    let active = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &admin.current_workspace, "isActive": true },
            None,
        )
        .await?;
    let vision_provider: VisionProvider = if active
        .as_ref()
        .map(|c| c.supports_vision)
        .unwrap_or(false)
    {
        // active 文字模型即视觉模型：复用运行时 provider（含热切换 / registry 语义）。
        VisionProvider::Runtime
    } else {
        // 收集所有支持视觉的副模型，专职视觉模型在前、其余备用在后，组成切换候选链。
        // 排序键：is_vision_active 倒序（专职优先），其次 updated_at 倒序（新配置优先）。
        let cursor = state
            .db
            .llm_provider_configs()
            .find(
                doc! {
                    "workspaceId": &admin.current_workspace,
                    "supportsVision": true,
                },
                FindOptions::builder()
                    .sort(doc! { "isVisionActive": -1, "updatedAt": -1 })
                    .build(),
            )
            .await?;
        let vision_cfgs: Vec<_> = cursor.try_collect().await?;
        if vision_cfgs.is_empty() {
            return Err(AppError::External(
                "visionNotSupported: 当前文字模型不支持图片，且未在模型设置中指派专职视觉模型".to_string(),
            ));
        }
        let mut candidates = Vec::with_capacity(vision_cfgs.len());
        for vision_cfg in &vision_cfgs {
            let fmt = crate::llm::LlmFormat::parse(&vision_cfg.format)?;
            let client = crate::llm::LlmClient::with_format(
                vision_cfg.base_url.clone(),
                vision_cfg.api_key.clone(),
                vision_cfg.model.clone(),
                fmt,
                vision_cfg
                    .timeout_seconds
                    .unwrap_or(state.config.llm_timeout_seconds),
                vision_cfg.max_retries.unwrap_or(state.config.llm_max_retries),
                vision_cfg
                    .retry_base_ms
                    .unwrap_or(state.config.llm_retry_base_ms),
            )
            .map_err(|e| AppError::External(format!("构造视觉模型 client 失败: {e}")))?;
            candidates.push((vision_cfg.model.clone(), client));
        }
        VisionProvider::Dedicated(candidates)
    };
    // 2) 拼 vision prompt：约束 LLM 输出 JSON {"fence": "..." }，让我们直接走 chunked_text 流程。
    let mime = req.mime.as_deref().unwrap_or("image/png");
    let hint = req.hint.as_deref().unwrap_or("无特定领域 hint");
    let system_prompt = "你是知识库 chunk 抽取助手。任务：把图片中的可读文本结构化为 fence 块。每块前后用 `---CHUNK: <短安全 id，仅字母数字和连字符>---` 与 `---END CHUNK---` 包裹（结束符必须是 `---END CHUNK---`，不要写 `---END---`）。块体必须是单个 JSON 对象，至少含 `title` 字段，且 `body`/`summary`/`answer` 中至少一个非空字符串，例如 {\"title\":\"小节标题\",\"body\":\"完整正文\"}。\n\
抽取方法（原子信息单元召回，对任何图片一视同仁，不针对特定主题）：\n\
1. 先把图片内容在脑中拆解为一组**原子信息单元**——每个单元是一条可独立成立、不可再拆的事实/条目/字段/陈述（一行表格、一个标题下的一段说明、一条编号项、一组「字段名:值」都各算一个单元）。\n\
2. **穷尽枚举**这些单元：逐个落成 chunk，覆盖图中出现的每一个单元，不要只挑你觉得重要的几条；宁可多分几个 chunk，也不要遗漏。划分以图片自身的视觉/语义边界（标题、分栏、表格行、列表项）为准，而不是以任何预设的主题清单为准。\n\
3. **保留原文 token 粒度**：body 照搬原文的关键表述、专有名词与具体数值（数字、比例、金额、期限、单位、阈值都要原样保留），不要概括、改写或压缩成一句话。\n\
4. **只抽真实存在的文字**：绝不编造、补全、推断或脑补图中没有的内容；图里没写的就不写，看不清的标注为不确定而非猜测。\n\
所有 chunk 默认 needs_review，不要写 verified。返回严格 JSON：{\"fence\": <字符串，全部 fence 文本>}。如果图片无文本可抽取，返回 {\"fence\": \"\"}。".to_string();
    let user_prompt = format!(
        "请按 fence 格式抽取下面这张图片中的知识 chunk。hint：{hint}"
    );
    // 3) 调视觉模型一次：图片以真正的多模态 image_url content block 发送
    //    （generate_json_with_image），而不是把 base64 当文本塞进 prompt——后者
    //    会让纯文字模型"看不到"图片。LlmProvider 默认实现对不支持视觉的 provider
    //    直接报错，这里 VisionProvider 解析阶段已保证选中的是 supports_vision 的模型。
    let raw_value = match &vision_provider {
        VisionProvider::Runtime => state
            .llm
            .generate_json_with_image(&system_prompt, &user_prompt, &req.image_base64, mime)
            .await
            .map_err(|e| match e {
                // 瞬时不可达（429/限流/配额耗尽/网关超时）原样透传结构化变体，
                // 让上游（测试 skip 宏、网关回退逻辑）按瞬时态处理而非当成内容失败。
                AppError::LlmUnavailable { .. } => e,
                other => AppError::External(format!("LLM vision 抽取失败: {other}")),
            }),
        // 候选链：主视觉模型瞬时不可达时自动切到下一备用模型；非瞬时错误立即失败
        // （内容/请求问题换模型也救不了）；全部候选都瞬时不可达才把最后一个瞬时变体
        // 上抛，让上游按瞬时态 skip 而非当成内容失败。
        VisionProvider::Dedicated(candidates) => {
            let mut last_transient: Option<AppError> = None;
            let mut result: Option<AppResult<Value>> = None;
            for (idx, (model, client)) in candidates.iter().enumerate() {
                match client
                    .generate_json_with_image(&system_prompt, &user_prompt, &req.image_base64, mime)
                    .await
                {
                    Ok(v) => {
                        result = Some(Ok(v));
                        break;
                    }
                    Err(e @ AppError::LlmUnavailable { .. }) => {
                        // 当前候选瞬时不可达，记录并切换到下一候选（若有）。
                        if idx + 1 < candidates.len() {
                            tracing::warn!(
                                model = %model,
                                next = %candidates[idx + 1].0,
                                error = %e,
                                "视觉模型瞬时不可达，自动切换到下一备用模型"
                            );
                        } else {
                            tracing::warn!(
                                model = %model,
                                error = %e,
                                "视觉模型瞬时不可达，已无更多备用模型可切换"
                            );
                        }
                        last_transient = Some(e);
                    }
                    Err(other) => {
                        // 非瞬时错误（内容/请求/格式问题）：换模型也无济于事，立即失败。
                        result = Some(Err(AppError::External(format!(
                            "LLM vision 抽取失败: {other}"
                        ))));
                        break;
                    }
                }
            }
            result.unwrap_or_else(|| {
                // 全部候选都瞬时不可达：上抛最后一个瞬时变体，让上游按瞬时态处理。
                Err(last_transient.unwrap_or_else(|| {
                    AppError::External("LLM vision 抽取失败: 无可用视觉模型候选".to_string())
                }))
            })
        }
    };
    let value = raw_value?;
    let raw = value
        .get("fence")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if raw.trim().is_empty() {
        return Ok(Json(json!({
            "documentId": null,
            "chunkIds": [],
            "parseWarnings": [],
            "fallbackBlob": false,
            "note": "vision 返回空文本",
        })));
    }
    let outcome = ingest_chunked_text(
        &state,
        &admin.current_workspace,
        req.account_id.as_deref(),
        req.source_name.as_deref().unwrap_or("uploaded_image"),
        &raw,
    )
    .await?;
    Ok(Json(json!({
        "documentId": outcome.document_id,
        "chunkIds": outcome.chunk_ids,
        "parseWarnings": outcome.parse_warnings,
        "fallbackBlob": outcome.fallback_blob,
    })))
}

#[derive(Debug)]
pub struct IngestOutcome {
    pub document_id: Option<String>,
    pub chunk_ids: Vec<String>,
    pub parse_warnings: Vec<Value>,
    /// fence 完全没解析出 chunk 时，把整段 `text` 落到一个兜底 blob chunk，
    /// 让运营在 Inspector 里手动切分。
    pub fallback_blob: bool,
}

/// 把已经抽取出的 `text` 走 fence 解析，成功的 block 写 `operation_knowledge_chunks`，
/// 失败块写 parse_warnings；fence 完全不命中时落一个 wikiType="raw" 的 blob chunk
/// 让运营手动切分。
pub async fn ingest_chunked_text(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    source_name: &str,
    text: &str,
) -> AppResult<IngestOutcome> {
    use crate::knowledge_wiki::block_parser::parse_chunk_blocks;

    let now = DateTime::now();
    // 先建一个 document 占位，所有 chunk 挂在同一个 document_id 下
    let document = crate::models::OperationKnowledgeDocument {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.map(|s| s.to_string()),
        domain: "user_operations".to_string(),
        source_type: "imported".to_string(),
        source_name: Some(source_name.to_string()),
        title: source_name.to_string(),
        summary: None,
        catalog_summary: None,
        routing_map: Vec::new(),
        risk_notes: Vec::new(),
        product_tags: Vec::new(),
        business_topics: Vec::new(),
        raw_content: Some(text.to_string()),
        content_hash: None,
        line_index: Vec::new(),
        section_index: Vec::new(),
        status: "active".to_string(),
        version: 1,
        created_at: now,
        updated_at: now,
        catalog_summary_persisted: None,
        catalog_version: None,
    };
    let doc_result = state
        .db
        .operation_knowledge_documents()
        .insert_one(&document, None)
        .await?;
    let document_id = doc_result.inserted_id.as_object_id();

    let (blocks, warnings) = parse_chunk_blocks(text);
    let mut parse_warnings: Vec<Value> = Vec::new();
    for w in &warnings.items {
        parse_warnings.push(parse_warning_to_json(w));
    }
    let mut chunk_ids: Vec<String> = Vec::new();
    let mut fallback_blob = false;

    if blocks.is_empty() {
        // fence 解析未命中：落一个 blob chunk，让运营在前端 Inspector 切分。
        fallback_blob = true;
        let chunk = OperationKnowledgeChunkRequest {
            account_id: account_id.map(|s| s.to_string()),
            document_id: document_id.map(|id| id.to_hex()),
            domain: "user_operations".to_string(),
            knowledge_type: Some("raw".to_string()),
            title: format!("{source_name} · 待切分 blob"),
            summary: Some(
                "fence 抽取未命中，整段文本落到此 chunk，等待运营在 Inspector 切分。".to_string(),
            ),
            body: Some(text.to_string()),
            integrity_status: Some("needs_review".to_string()),
            status: "draft".to_string(),
            ..Default::default()
        };
        if let Err(e) = validate_operation_knowledge_chunk(&chunk) {
            parse_warnings.push(json!({
                "kind": "blobValidationError",
                "reason": format!("{e}"),
            }));
        } else {
            let row = operation_knowledge_chunk_from_request(state, workspace_id, chunk, None)?;
            let result = state
                .db
                .operation_knowledge_chunks()
                .insert_one(&row, None)
                .await?;
            if let Some(id) = result.inserted_id.as_object_id() {
                chunk_ids.push(id.to_hex());
            }
        }
        return Ok(IngestOutcome {
            document_id: document_id.map(|id| id.to_hex()),
            chunk_ids,
            parse_warnings,
            fallback_blob,
        });
    }

    for block in blocks {
        let mut chunk_req: OperationKnowledgeChunkRequest =
            match serde_json::from_value::<OperationKnowledgeChunkRequest>(block.payload.clone()) {
                Ok(c) => c,
                Err(e) => {
                    parse_warnings.push(json!({
                        "kind": "blockToChunkRequestError",
                        "id": block.id,
                        "reason": format!("{e}"),
                    }));
                    continue;
                }
            };
        if chunk_req.account_id.is_none() {
            chunk_req.account_id = account_id.map(|s| s.to_string());
        }
        if chunk_req.document_id.is_none() {
            chunk_req.document_id = document_id.map(|id| id.to_hex());
        }
        if let Some(document_id_v) = document_id {
            apply_chunk_integrity(&mut chunk_req, text, Some(document_id_v));
        }
        chunk_req.status = "draft".to_string();
        // 红线"AI 永不自动 verify"：import 路径的 `text` 本身就是这批未经审核的导入
        // 材料，apply_chunk_integrity 拿 chunk 的 sourceQuote 去 `text` 里锚定成功只能
        // 说明"引用确实出自这份导入文本"，并不等于该知识已被核实。因此无条件压回
        // needs_review（保留 apply_chunk_integrity 算出的 source_anchors 作为审核线索），
        // 让运营在 Inspector 二次确认后才进入 agent 的 verified 池。
        chunk_req.integrity_status = Some("needs_review".to_string());
        if let Err(e) = validate_operation_knowledge_chunk(&chunk_req) {
            parse_warnings.push(json!({
                "kind": "blockValidationError",
                "id": block.id,
                "reason": format!("{e}"),
            }));
            continue;
        }
        let row = operation_knowledge_chunk_from_request(state, workspace_id, chunk_req, None)?;
        let result = state
            .db
            .operation_knowledge_chunks()
            .insert_one(&row, None)
            .await?;
        if let Some(id) = result.inserted_id.as_object_id() {
            chunk_ids.push(id.to_hex());
            let req = RevisionRequest {
                op: RevisionOp::Create,
                source: ProvenanceSource::Imported,
                patch: Document::new(),
                reason: Some(format!("ingest_chunked_text source={source_name} block={}", block.id)),
                actor: account_id.map(|s| s.to_string()),
            };
            if let Err(e) = apply_chunk_revision(&state.db, workspace_id, id, req).await {
                tracing::warn!(
                    chunk_id = %id.to_hex(),
                    block_id = %block.id,
                    error = %e,
                    "ingest_chunked_text: write chunk_revision failed (non-fatal)"
                );
            }
        }
    }
    Ok(IngestOutcome {
        document_id: document_id.map(|id| id.to_hex()),
        chunk_ids,
        parse_warnings,
        fallback_blob,
    })
}

pub(super) async fn test_operation_knowledge_match(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<OperationKnowledgeTestRequest>,
) -> AppResult<Json<Value>> {
    if payload.message.trim().is_empty() {
        return Err(AppError::BadRequest("message is required".to_string()));
    }
    let contact = if let Some(contact_id) = payload.contact_id {
        Some(find_contact_by_id(&state, &admin.current_workspace, &contact_id).await?)
    } else {
        None
    };
    let result = agent::test_knowledge_route_for_contact(
        &state,
        contact,
        &payload.account_id,
        &payload.message,
    )
    .await?;
    Ok(Json(json!({ "item": result })))
}

pub(super) async fn list_knowledge_usage(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let mut cursor = state
        .db
        .knowledge_usage_logs()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": account_id
            },
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(knowledge_usage_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AnalyzeLogsQuery {
    account_id: Option<String>,
    /// 回看窗口（小时），缺省 24，硬上限 72。
    hours: Option<i64>,
    /// 仅统计被拦截 / 暂缓的 run，缺省 true。
    only_blocked_or_held: Option<bool>,
}

/// `GET /api/operation-knowledge/logs/analyze`
///
/// 只读：按窗口聚合 `knowledge_usage_logs`，输出 `{window_hours, total_runs,
/// blocked_or_held_runs, top_chunks, items}`。语义与 chat tool
/// `knowledge.analyze_logs` 完全一致，前端 / 运营审查时直接 HTTP 取，不用走
/// LLM。
pub(super) async fn analyze_operation_knowledge_logs(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AnalyzeLogsQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = admin.current_workspace.clone();
    let hours = query.hours.filter(|v| *v > 0).unwrap_or(24).min(72);
    let only_blocked = query.only_blocked_or_held.unwrap_or(true);
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours);
    let cutoff_bson = DateTime::from_millis(cutoff.timestamp_millis());

    let mut filter = doc! {
        "workspace_id": &workspace_id,
        "created_at": { "$gte": cutoff_bson },
    };
    if let Some(account_id) = query
        .account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        filter.insert("account_id", account_id);
    }
    if only_blocked {
        filter.insert(
            "$or",
            Bson::Array(vec![
                Bson::Document(doc! { "review_approved": false }),
                Bson::Document(doc! {
                    "blocked_reason": { "$exists": true, "$ne": Bson::Null },
                }),
            ]),
        );
    }

    let mut cursor = state
        .db
        .knowledge_usage_logs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1_i32 })
                .limit(50)
                .build(),
        )
        .await?;

    let mut chunk_freq: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut items: Vec<Value> = Vec::new();
    let mut blocked: i32 = 0;
    while let Some(log) = cursor.try_next().await? {
        if log.blocked_reason.is_some() || !log.review_approved {
            blocked += 1;
        }
        for kid in &log.knowledge_ids {
            *chunk_freq.entry(kid.to_hex()).or_insert(0) += 1;
        }
        items.push(json!({
            "runId": log.run_id,
            "accountId": log.account_id,
            "blockedReason": log.blocked_reason,
            "reviewApproved": log.review_approved,
            "knowledgeIds": log.knowledge_ids.iter().map(|o| o.to_hex()).collect::<Vec<_>>(),
            "createdAt": crate::models::dt_to_string(log.created_at),
        }));
    }

    let total_runs = items.len() as i32;
    let mut top_chunks: Vec<(String, i32)> = chunk_freq.into_iter().collect();
    top_chunks.sort_by(|a, b| b.1.cmp(&a.1));
    let top_chunks_json: Vec<Value> = top_chunks
        .into_iter()
        .take(8)
        .map(|(id, count)| json!({ "chunkId": id, "hitCount": count }))
        .collect();

    Ok(Json(json!({
        "windowHours": hours,
        "onlyBlockedOrHeld": only_blocked,
        "totalRuns": total_runs,
        "blockedOrHeldRuns": blocked,
        "topChunks": top_chunks_json,
        "items": items,
    })))
}

// operation_knowledge_json removed: OperationKnowledgeItem 已随 sales 旧库删除。
// 新的 wiki 走 operation_knowledge_chunk_json。

pub(super) fn operation_knowledge_document_json(item: OperationKnowledgeDocument) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "domain": item.domain,
        "sourceType": item.source_type,
        "sourceName": item.source_name,
        "title": item.title,
        "summary": item.summary,
        "catalogSummary": item.catalog_summary,
        "routingMap": item.routing_map,
        "riskNotes": item.risk_notes,
        "rawContent": item.raw_content,
        "contentHash": item.content_hash,
        "lineIndex": item.line_index,
        "sectionIndex": item.section_index,
        "status": item.status,
        "version": item.version,
        "updatedAt": crate::models::dt_to_string(item.updated_at)
    })
}

pub(super) fn operation_knowledge_chunk_json(item: OperationKnowledgeChunk) -> Value {
    // source_anchors 是 Vec<bson::Document>；直接 serde_json 序列化会暴露 BSON
    // Extended JSON（如 `{"$numberInt":"42"}`）。前端 KnowledgeTreeView /
    // ReviewView 直接读 `anchor.startLine / endLine / quoteHash / documentId`，
    // 必须先走 `.into_relaxed_extjson()` 桥接成纯 JSON。
    let source_anchors_json: Vec<Value> = item
        .source_anchors
        .into_iter()
        .map(|d| mongodb::bson::Bson::Document(d).into_relaxed_extjson())
        .collect();
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "documentId": item.document_id.map(|id| id.to_hex()),
        "itemId": item.item_id.map(|id| id.to_hex()),
        "domain": item.domain,
        "knowledgeType": item.knowledge_type,
        "businessContext": item.business_context,
        "title": item.title,
        "summary": item.summary,
        "body": item.body,
        "applicableScenes": item.applicable_scenes,
        "notApplicableScenes": item.not_applicable_scenes,
        "sourceQuote": item.source_quote,
        "sourceAnchors": source_anchors_json,
        "integrityStatus": item.integrity_status,
        "confidenceScore": item.confidence_score,
        "status": item.status,
        "priority": item.priority,
        "wikiType": item.wiki_type,
        "relatedChunks": item.related_chunks,
        "businessTopics": item.business_topics,
        "supersededBy": item.superseded_by,
        "previousVersionId": item.previous_version_id,
        "updatedAt": crate::models::dt_to_string(item.updated_at)
    })
}

pub(super) fn knowledge_usage_json(item: KnowledgeUsageLog) -> Value {
    // tool_trace / route_result 都是 BSON Document — 走 extjson 桥接避免
    // `{"$numberInt":"…"}` 等 BSON 包装泄漏到前端。
    let route_result_json =
        mongodb::bson::Bson::Document(item.route_result).into_relaxed_extjson();
    let tool_trace_json: Vec<Value> = item
        .tool_trace
        .into_iter()
        .map(|d| mongodb::bson::Bson::Document(d).into_relaxed_extjson())
        .collect();
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "contactWxid": item.contact_wxid,
        "runId": item.run_id,
        "knowledgeIds": item.knowledge_ids.into_iter().map(|id| id.to_hex()).collect::<Vec<_>>(),
        "routeResult": route_result_json,
        "replyText": item.reply_text,
        "reviewApproved": item.review_approved,
        "blockedReason": item.blocked_reason,
        "toolTrace": tool_trace_json,
        "createdAt": crate::models::dt_to_string(item.created_at)
    })
}

/// 规范化知识标签：trim、可选 lowercase、
/// 去重（保留首次出现顺序）、跳过空字符串、按 max_len 截断。
///
/// LLM 在 import-preview 偶尔会返回字符串而非数组、或重复元素，统一在这里收口。
pub(super) fn normalize_knowledge_tags(
    raw: Vec<String>,
    max_len: usize,
    lowercase: bool,
) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for item in raw.into_iter() {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = if lowercase {
            trimmed.to_lowercase()
        } else {
            trimmed.to_string()
        };
        if seen.insert(normalized.clone()) {
            out.push(normalized);
            if out.len() >= max_len {
                break;
            }
        }
    }
    out
}

pub(super) fn validate_operation_knowledge_document(
    payload: &OperationKnowledgeDocumentRequest,
) -> AppResult<()> {
    if payload.title.trim().is_empty() {
        return Err(AppError::BadRequest("title is required".to_string()));
    }
    Ok(())
}

pub(super) fn validate_operation_knowledge_chunk(
    payload: &OperationKnowledgeChunkRequest,
) -> AppResult<()> {
    if payload.title.trim().is_empty() {
        return Err(AppError::BadRequest("title is required".to_string()));
    }
    Ok(())
}

// operation_knowledge_from_request removed: OperationKnowledgeItem 已随 sales 旧库删除。

pub(super) fn operation_knowledge_document_from_request(
    _state: &AppState,
    workspace_id: &str,
    payload: OperationKnowledgeDocumentRequest,
    id: Option<ObjectId>,
) -> OperationKnowledgeDocument {
    let now = DateTime::now();
    let raw_content = normalize_optional(payload.raw_content);
    let content_hash = payload.content_hash.or_else(|| {
        raw_content
            .as_ref()
            .map(|content| stable_text_hash(content))
    });
    let line_index = if payload.line_index.is_empty() {
        raw_content
            .as_ref()
            .map(|content| build_line_index(content))
            .unwrap_or_default()
    } else {
        payload.line_index
    };
    let section_index = if payload.section_index.is_empty() {
        raw_content
            .as_ref()
            .map(|content| build_section_index(content))
            .unwrap_or_default()
    } else {
        payload.section_index
    };
    OperationKnowledgeDocument {
        id,
        workspace_id: workspace_id.to_string(),
        account_id: payload.account_id,
        domain: normalize_operation_domain(&payload.domain),
        source_type: if payload.source_type.trim().is_empty() {
            default_imported_markdown_source_type()
        } else {
            payload.source_type
        },
        source_name: normalize_optional(payload.source_name),
        title: payload.title,
        summary: normalize_optional(payload.summary),
        catalog_summary: normalize_optional(payload.catalog_summary),
        routing_map: payload.routing_map,
        risk_notes: payload.risk_notes,
        product_tags: normalize_knowledge_tags(payload.product_tags, 5, false),
        business_topics: normalize_knowledge_tags(payload.business_topics, 3, false),
        raw_content,
        content_hash,
        line_index,
        section_index,
        status: if payload.status.trim().is_empty() {
            default_active_status()
        } else {
            payload.status
        },
        version: 1,
        created_at: now,
        updated_at: now,
        // knowledge-wiki Phase A: catalog 落库由 worker 异步填，写入侧默认 None。
        catalog_summary_persisted: None,
        catalog_version: None,
    }
}

pub(super) fn operation_knowledge_chunk_from_request(
    _state: &AppState,
    workspace_id: &str,
    payload: OperationKnowledgeChunkRequest,
    id: Option<ObjectId>,
) -> AppResult<OperationKnowledgeChunk> {
    let now = DateTime::now();
    Ok(OperationKnowledgeChunk {
        id,
        workspace_id: workspace_id.to_string(),
        account_id: payload.account_id,
        document_id: payload
            .document_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(parse_object_id)
            .transpose()?,
        item_id: payload
            .item_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(parse_object_id)
            .transpose()?,
        domain: normalize_operation_domain(&payload.domain),
        knowledge_type: normalize_optional(payload.knowledge_type),
        business_context: normalize_optional(payload.business_context),
        title: payload.title,
        summary: normalize_optional(payload.summary),
        body: normalize_optional(payload.body),
        applicable_scenes: payload.applicable_scenes,
        not_applicable_scenes: payload.not_applicable_scenes,
        product_tags: normalize_knowledge_tags(payload.product_tags, 5, false),
        business_topics: normalize_knowledge_tags(payload.business_topics, 3, false),
        source_quote: normalize_optional(payload.source_quote),
        source_anchors: payload.source_anchors,
        integrity_status: normalize_optional(payload.integrity_status),
        confidence_score: payload.confidence_score,
        status: if payload.status.trim().is_empty() {
            default_active_status()
        } else {
            payload.status
        },
        priority: payload.priority,
        created_at: now,
        updated_at: now,
        ..Default::default()
    })
}

pub(super) fn normalize_operation_knowledge_preview_item(
    value: Value,
    payload: &OperationKnowledgeImportRequest,
) -> Value {
    let source_name = payload
        .source_name
        .clone()
        .unwrap_or_else(|| "导入文本".to_string());
    json!({
        "accountId": payload.account_id,
        "domain": json_string(&value, "domain")
            .map(|raw| normalize_operation_domain(&raw))
            .unwrap_or_else(default_user_operations_domain),
        "category": json_string(&value, "category")
            .or_else(|| json_string(&value, "knowledgeType"))
            .or_else(|| json_string(&value, "knowledge_type"))
            .unwrap_or_else(|| "未分类知识".to_string()),
        "businessType": json_string(&value, "businessType")
            .or_else(|| json_string(&value, "business_type"))
            .or_else(|| json_string(&value, "businessContext"))
            .unwrap_or_else(|| "自动识别".to_string()),
        "knowledgeType": json_string(&value, "knowledgeType").or_else(|| json_string(&value, "knowledge_type")).unwrap_or_default(),
        "businessContext": json_string(&value, "businessContext").or_else(|| json_string(&value, "business_context")).unwrap_or_default(),
        "title": json_string(&value, "title").unwrap_or_else(|| "未命名知识包".to_string()),
        "summary": json_string(&value, "summary").unwrap_or_default(),
        "body": json_string(&value, "body").unwrap_or_default(),
        "routingCard": json_string(&value, "routingCard")
            .or_else(|| json_string(&value, "routing_card"))
            .unwrap_or_default(),
        "applicableScenes": json_string_list(&value, "applicableScenes").or_else(|| json_string_list(&value, "applicable_scenes")).unwrap_or_default(),
        "notApplicableScenes": json_string_list(&value, "notApplicableScenes").or_else(|| json_string_list(&value, "not_applicable_scenes")).unwrap_or_default(),
        "suitableFor": json_string_list(&value, "suitableFor").or_else(|| json_string_list(&value, "suitable_for")).unwrap_or_default(),
        "notSuitableFor": json_string_list(&value, "notSuitableFor").or_else(|| json_string_list(&value, "not_suitable_for")).unwrap_or_default(),
        "customerStages": json_string_list(&value, "customerStages").or_else(|| json_string_list(&value, "customer_stages")).unwrap_or_default(),
        "operationStates": json_string_list(&value, "operationStates").or_else(|| json_string_list(&value, "operation_states")).unwrap_or_default(),
        "intentLevels": json_string_list(&value, "intentLevels").or_else(|| json_string_list(&value, "intent_levels")).unwrap_or_default(),
        "safeClaims": json_string_list(&value, "safeClaims").or_else(|| json_string_list(&value, "safe_claims")).unwrap_or_default(),
        "forbiddenClaims": json_string_list(&value, "forbiddenClaims").or_else(|| json_string_list(&value, "forbidden_claims")).unwrap_or_default(),
        "commonQuestions": json_string_list(&value, "commonQuestions").or_else(|| json_string_list(&value, "common_questions")).unwrap_or_default(),
        "commonObjections": json_string_list(&value, "commonObjections").or_else(|| json_string_list(&value, "common_objections")).unwrap_or_default(),
        "evidenceItems": json_string_list(&value, "evidenceItems").or_else(|| json_string_list(&value, "evidence_items")).unwrap_or_default(),
        "productTags": json_string_list(&value, "productTags").or_else(|| json_string_list(&value, "product_tags")).unwrap_or_default(),
        "businessTopics": json_string_list(&value, "businessTopics").or_else(|| json_string_list(&value, "business_topics")).unwrap_or_default(),
        "sourceType": json_string(&value, "sourceType").or_else(|| json_string(&value, "source_type")).unwrap_or_else(|| "imported_markdown".to_string()),
        "sourceName": json_string(&value, "sourceName").or_else(|| json_string(&value, "source_name")).unwrap_or(source_name),
        "status": json_string(&value, "status").unwrap_or_else(|| "draft".to_string()),
        "priority": value.get("priority").and_then(|item| item.as_i64()).unwrap_or(0) as i32
    })
}

pub(super) fn normalize_operation_knowledge_preview_document(
    value: Value,
    payload: &OperationKnowledgeImportRequest,
) -> Value {
    let source_name = payload
        .source_name
        .clone()
        .unwrap_or_else(|| "导入文本".to_string());
    json!({
        "accountId": payload.account_id,
        "domain": json_string(&value, "domain")
            .map(|raw| normalize_operation_domain(&raw))
            .unwrap_or_else(default_user_operations_domain),
        "sourceType": json_string(&value, "sourceType").or_else(|| json_string(&value, "source_type")).unwrap_or_else(default_imported_markdown_source_type),
        "sourceName": json_string(&value, "sourceName").or_else(|| json_string(&value, "source_name")).unwrap_or(source_name.clone()),
        "title": json_string(&value, "title").unwrap_or(source_name),
        "summary": json_string(&value, "summary").unwrap_or_default(),
        "catalogSummary": json_string(&value, "catalogSummary").or_else(|| json_string(&value, "catalog_summary")).unwrap_or_default(),
        "routingMap": json_string_list(&value, "routingMap").or_else(|| json_string_list(&value, "routing_map")).unwrap_or_default(),
        "riskNotes": json_string_list(&value, "riskNotes").or_else(|| json_string_list(&value, "risk_notes")).unwrap_or_default(),
        "productTags": json_string_list(&value, "productTags").or_else(|| json_string_list(&value, "product_tags")).unwrap_or_default(),
        "businessTopics": json_string_list(&value, "businessTopics").or_else(|| json_string_list(&value, "business_topics")).unwrap_or_default(),
        "rawContent": payload.content,
        "contentHash": stable_text_hash(&payload.content),
        "lineIndex": build_line_index(&payload.content),
        "sectionIndex": build_section_index(&payload.content),
        "status": json_string(&value, "status").unwrap_or_else(|| "draft".to_string())
    })
}

pub(super) fn default_operation_knowledge_preview_document(
    payload: &OperationKnowledgeImportRequest,
) -> Value {
    let source_name = payload
        .source_name
        .clone()
        .unwrap_or_else(|| "导入文本".to_string());
    json!({
        "accountId": payload.account_id,
        "domain": default_user_operations_domain(),
        "sourceType": default_imported_markdown_source_type(),
        "sourceName": source_name,
        "title": source_name,
        "summary": "",
        "catalogSummary": "",
        "routingMap": [],
        "riskNotes": [],
        "rawContent": payload.content,
        "contentHash": stable_text_hash(&payload.content),
        "lineIndex": build_line_index(&payload.content),
        "sectionIndex": build_section_index(&payload.content),
        "status": "draft"
    })
}

pub(super) fn normalize_operation_knowledge_preview_chunk(
    value: Value,
    payload: &OperationKnowledgeImportRequest,
) -> Value {
    json!({
        "accountId": payload.account_id,
        "domain": json_string(&value, "domain")
            .map(|raw| normalize_operation_domain(&raw))
            .unwrap_or_else(default_user_operations_domain),
        "knowledgeType": json_string(&value, "knowledgeType").or_else(|| json_string(&value, "knowledge_type")).unwrap_or_default(),
        "businessContext": json_string(&value, "businessContext").or_else(|| json_string(&value, "business_context")).unwrap_or_default(),
        "title": json_string(&value, "title").unwrap_or_else(|| "未命名知识切片".to_string()),
        "summary": json_string(&value, "summary").unwrap_or_default(),
        "body": json_string(&value, "body").unwrap_or_default(),
        "routingCard": json_string(&value, "routingCard").or_else(|| json_string(&value, "routing_card")).unwrap_or_default(),
        "applicableScenes": json_string_list(&value, "applicableScenes").or_else(|| json_string_list(&value, "applicable_scenes")).unwrap_or_default(),
        "notApplicableScenes": json_string_list(&value, "notApplicableScenes").or_else(|| json_string_list(&value, "not_applicable_scenes")).unwrap_or_default(),
        "safeClaims": json_string_list(&value, "safeClaims").or_else(|| json_string_list(&value, "safe_claims")).unwrap_or_default(),
        "forbiddenClaims": json_string_list(&value, "forbiddenClaims").or_else(|| json_string_list(&value, "forbidden_claims")).unwrap_or_default(),
        "evidenceItems": json_string_list(&value, "evidenceItems").or_else(|| json_string_list(&value, "evidence_items")).unwrap_or_default(),
        "productTags": json_string_list(&value, "productTags").or_else(|| json_string_list(&value, "product_tags")).unwrap_or_default(),
        "businessTopics": json_string_list(&value, "businessTopics").or_else(|| json_string_list(&value, "business_topics")).unwrap_or_default(),
        "sourceQuote": json_string(&value, "sourceQuote").or_else(|| json_string(&value, "source_quote")).unwrap_or_default(),
        "sourceAnchors": [],
        "integrityStatus": "needs_review",
        "confidenceScore": 0,
        "distortionRisks": [],
        "unsupportedClaims": [],
        "verifiedClaims": [],
        "status": json_string(&value, "status").unwrap_or_else(|| "draft".to_string()),
        "priority": value.get("priority").and_then(|item| item.as_i64()).unwrap_or(0) as i32
    })
}

pub(super) fn json_string_list(value: &Value, key: &str) -> Option<Vec<String>> {
    value.get(key).and_then(|item| {
        if let Some(items) = item.as_array() {
            Some(
                items
                    .iter()
                    .filter_map(|entry| entry.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
        } else {
            item.as_str().map(split_lines)
        }
    })
}

pub(super) fn split_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_start_matches(['-', '*', '•']).trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

pub(super) fn string_bson_array(values: &[String]) -> Vec<Bson> {
    values.iter().cloned().map(Bson::String).collect()
}

pub(super) fn stable_text_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(super) fn build_line_index(content: &str) -> Vec<Document> {
    let mut offset = 0usize;
    content
        .split_inclusive('\n')
        .enumerate()
        .map(|(index, segment)| {
            let line = segment.trim_end_matches('\n').trim_end_matches('\r');
            let start = offset;
            let end = start + line.len();
            offset += segment.len();
            doc! {
                "line": (index + 1) as i32,
                "startOffset": start as i32,
                "endOffset": end as i32,
                "hash": stable_text_hash(line)
            }
        })
        .collect()
}

pub(super) fn build_section_index(content: &str) -> Vec<Document> {
    let mut sections = Vec::new();
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            sections.push(doc! {
                "title": trimmed.trim_start_matches('#').trim(),
                "level": trimmed.chars().take_while(|item| *item == '#').count() as i32,
                "startOffset": offset as i32
            });
        }
        offset += line.len();
    }
    sections
}

pub(super) fn source_anchor_for_quote(
    raw_content: &str,
    document_id: Option<ObjectId>,
    source_quote: &str,
) -> Option<Document> {
    let quote = source_quote.trim();
    if quote.is_empty() {
        return None;
    }
    let span = raw_content
        .find(quote)
        .map(|start| (start, start + quote.len()))
        .or_else(|| fuzzy_locate_quote(raw_content, quote));
    span.map(|(start, end)| {
        let start_line = raw_content[..start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        let end_line = raw_content[..end]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        let mut anchor = doc! {
            "startOffset": start as i32,
            "endOffset": end as i32,
            "startLine": start_line as i32,
            "endLine": end_line as i32,
            "sourceQuote": quote,
            "quoteHash": stable_text_hash(quote),
        };
        if let Some(document_id) = document_id {
            anchor.insert("documentId", document_id.to_hex());
        }
        anchor
    })
}

/// 模糊定位：把 quote 和正文都做"压缩空白 + 去常见 markdown/中英标点干扰"再找
/// 子串，命中后回推到原文 byte offset。LLM 出的 source_quote 经常吃掉首/尾空白
/// 或换行被压成一行，硬 `find()` 会失败，但语义上锚点是存在的，落库后没法修。
fn fuzzy_locate_quote(raw_content: &str, quote: &str) -> Option<(usize, usize)> {
    let quote_norm = normalize_for_anchor(quote);
    if quote_norm.is_empty() {
        return None;
    }
    // 维护原文字节位置 → normalized 字符位置 的映射
    let mut norm_chars: Vec<char> = Vec::new();
    let mut norm_to_byte: Vec<usize> = Vec::new();
    let mut last_was_ws = true;
    for (byte_idx, ch) in raw_content.char_indices() {
        if ch.is_whitespace() {
            if !last_was_ws {
                norm_chars.push(' ');
                norm_to_byte.push(byte_idx);
            }
            last_was_ws = true;
        } else {
            norm_chars.push(ch);
            norm_to_byte.push(byte_idx);
            last_was_ws = false;
        }
    }
    // tail sentinel
    norm_to_byte.push(raw_content.len());
    let norm_str: String = norm_chars.iter().collect();
    let q_norm: String = quote_norm.chars().collect();
    let start_char = norm_str.find(&q_norm)?;
    // norm_str 是按 char 拼的；要把 char 偏移转成 norm_chars 索引
    let start_idx = norm_str[..start_char].chars().count();
    let end_idx = start_idx + q_norm.chars().count();
    if end_idx > norm_to_byte.len() {
        return None;
    }
    let start_byte = norm_to_byte[start_idx];
    let end_byte = if end_idx < norm_to_byte.len() {
        norm_to_byte[end_idx]
    } else {
        raw_content.len()
    };
    if start_byte > end_byte || end_byte > raw_content.len() {
        return None;
    }
    Some((start_byte, end_byte))
}

fn normalize_for_anchor(s: &str) -> String {
    let mut out = String::new();
    let mut last_was_ws = true;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_was_ws {
                out.push(' ');
            }
            last_was_ws = true;
        } else {
            out.push(ch);
            last_was_ws = false;
        }
    }
    out.trim().to_string()
}

pub(super) fn integrity_report_for_preview(raw_content: &str, chunks: &mut [Value]) -> Value {
    // 红线「AI 永不自动 verify」：preview 路径恒 0 verified，anchor 命中只作审计线索。
    let verified = 0;
    let mut needs_review = 0;
    let mut rejected = 0;
    let mut items = Vec::new();
    for chunk in chunks.iter_mut() {
        let source_quote = json_string(chunk, "sourceQuote")
            .or_else(|| json_string(chunk, "source_quote"))
            .unwrap_or_default();
        let safe_claims = json_string_list(chunk, "safeClaims").unwrap_or_default();
        let evidence_items = json_string_list(chunk, "evidenceItems").unwrap_or_default();
        let mut risks = Vec::new();
        let mut anchors = Vec::new();
        if let Some(anchor) = source_anchor_for_quote(raw_content, None, &source_quote) {
            anchors.push(anchor);
        } else if !source_quote.trim().is_empty() {
            risks.push("sourceQuote 未在原文中找到".to_string());
        } else {
            risks.push("缺少原文引用".to_string());
        }
        if anchors.is_empty() && (!safe_claims.is_empty() || !evidence_items.is_empty()) {
            risks.push("存在安全事实或证据项，但没有可验证原文锚点".to_string());
        }
        let has_quote = !source_quote.trim().is_empty();
        let anchored = !anchors.is_empty() && risks.is_empty();
        // 红线「AI 永不自动 verify」：preview 与 apply handler（见
        // import_operation_knowledge_apply / *_apply_chunked 落库前
        // 无条件压回 needs_review）保持一致——anchor 命中只作审计线索
        // （保留 anchors + confidence=90，前端「距离 verify 一步之遥」可用），
        // integrityStatus 绝不直接 verified。光有声明却无源仍硬挡 rejected（更严方向）。
        let status = if anchored || has_quote || (safe_claims.is_empty() && evidence_items.is_empty())
        {
            needs_review += 1;
            "needs_review"
        } else {
            rejected += 1;
            "rejected"
        };
        let confidence = if anchored { 90 } else { 45 };
        if let Some(object) = chunk.as_object_mut() {
            object.insert("sourceAnchors".to_string(), json!(anchors));
            object.insert("integrityStatus".to_string(), json!(status));
            object.insert("confidenceScore".to_string(), json!(confidence));
            object.insert("distortionRisks".to_string(), json!(risks.clone()));
            object.insert(
                "unsupportedClaims".to_string(),
                json!(if anchors.is_empty() {
                    safe_claims.clone()
                } else {
                    Vec::<String>::new()
                }),
            );
            object.insert(
                "verifiedClaims".to_string(),
                json!(if anchors.is_empty() {
                    Vec::<String>::new()
                } else {
                    safe_claims.clone()
                }),
            );
        }
        items.push(json!({
            "title": json_string(chunk, "title").unwrap_or_default(),
            "integrityStatus": status,
            "confidenceScore": confidence,
            "distortionRisks": risks,
            "sourceAnchors": anchors
        }));
    }
    json!({
        "verified": verified,
        "needsReview": needs_review,
        "rejected": rejected,
        "items": items
    })
}

pub(super) fn apply_chunk_integrity(
    chunk: &mut OperationKnowledgeChunkRequest,
    raw_content: &str,
    document_id: Option<ObjectId>,
) {
    let source_quote = chunk.source_quote.clone().unwrap_or_default();
    if chunk.source_anchors.is_empty() {
        if let Some(anchor) = source_anchor_for_quote(raw_content, document_id, &source_quote) {
            chunk.source_anchors.push(anchor);
        }
    }
    let has_anchor = !chunk.source_anchors.is_empty();
    let has_quote = !source_quote.trim().is_empty();
    if has_anchor {
        if chunk.verified_claims.is_empty() {
            chunk.verified_claims = chunk.safe_claims.clone();
        }
        chunk.integrity_status = Some("verified".to_string());
        chunk.confidence_score = Some(chunk.confidence_score.unwrap_or(90));
        return;
    }
    // 没 anchor。区分两种情况：
    // 1) 还有 source_quote → AI 出了引用但模糊匹配也没找到，留 needs_review，
    //    让 AI 自主修复流程来纠正引用 / 重新锚定。
    // 2) 既没 quote、也没 safe_claims/evidence_items（光有 routing 元数据）→ needs_review。
    // 3) 没 quote 但有 claim/evidence → rejected（声明无源，硬挡）。
    if has_quote || (chunk.safe_claims.is_empty() && chunk.evidence_items.is_empty()) {
        if !has_quote && chunk.distortion_risks.is_empty() {
            chunk
                .distortion_risks
                .push("缺 sourceQuote 与原文锚点，建议触发 AI 自主修复".to_string());
        } else if has_quote && chunk.distortion_risks.is_empty() {
            chunk
                .distortion_risks
                .push("sourceQuote 未在原文中精确匹配，建议触发 AI 自主修复以纠正引用".to_string());
        }
        chunk.integrity_status = Some(
            chunk
                .integrity_status
                .clone()
                .filter(|s| matches!(s.as_str(), "needs_review" | "verified" | "rejected"))
                .unwrap_or_else(|| "needs_review".to_string()),
        );
        if matches!(chunk.integrity_status.as_deref(), Some("verified")) {
            chunk.integrity_status = Some("needs_review".to_string());
        }
        chunk.confidence_score = Some(chunk.confidence_score.unwrap_or(45));
        return;
    }
    // 既没 quote 又有 claim/evidence：硬声明无源，标 rejected。
    if chunk.unsupported_claims.is_empty() {
        chunk.unsupported_claims = chunk.safe_claims.clone();
    }
    if chunk.distortion_risks.is_empty() {
        chunk
            .distortion_risks
            .push("安全事实或证据缺少 sourceQuote 与原文锚点".to_string());
    }
    chunk.integrity_status = Some("rejected".to_string());
    chunk.confidence_score = Some(0);
}

pub(super) async fn load_operation_knowledge_chunks_for_query(
    state: &AppState,
    workspace_id: &str,
    query: OperationKnowledgeChunkQuery,
) -> AppResult<Vec<Value>> {
    let mut filter = doc! {
        "workspace_id": workspace_id,
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
    if let Some(document_id) = query.document_id {
        filter.insert("document_id", parse_object_id(&document_id)?);
    }
    if let Some(item_id) = query.item_id {
        filter.insert("item_id", parse_object_id(&item_id)?);
    }
    if let Some(status) = normalize_optional(query.status) {
        filter.insert("status", status);
    }
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "priority": -1, "updated_at": -1 })
                .limit(300)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(operation_knowledge_chunk_json(item));
    }
    Ok(items)
}

pub(super) async fn build_operation_knowledge_catalog(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Value> {
    let account_filter = vec![
        doc! { "account_id": null },
        doc! { "account_id": account_id },
    ];
    let mut document_cursor = state
        .db
        .operation_knowledge_documents()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations",
                "status": "active",
                "$or": account_filter.clone()
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut documents = Vec::new();
    while let Some(item) = document_cursor.try_next().await? {
        documents.push(json!({
            "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
            "title": item.title,
            "catalogSummary": item.catalog_summary.or(item.summary),
            "routingMap": item.routing_map,
            "riskNotes": item.risk_notes
        }));
    }
    // operation_knowledge_items 已随 sales 旧库删除；catalog 中的 items 永远空。
    let _ = &account_filter;
    let items: Vec<Value> = Vec::new();
    let mut chunk_cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations",
                "status": "active",
                "integrity_status": "verified",
                "$or": account_filter
            },
            FindOptions::builder()
                .sort(doc! { "priority": -1, "updated_at": -1 })
                .limit(200)
                .build(),
        )
        .await?;
    let mut chunks = Vec::new();
    while let Some(item) = chunk_cursor.try_next().await? {
        chunks.push(json!({
            "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
            "documentId": item.document_id.map(|id| id.to_hex()),
            "itemId": item.item_id.map(|id| id.to_hex()),
            "title": item.title,
            "knowledgeType": item.knowledge_type,
            "businessContext": item.business_context,
            "applicableScenes": item.applicable_scenes,
            "notApplicableScenes": item.not_applicable_scenes,
            "integrityStatus": item.integrity_status,
            "confidenceScore": item.confidence_score,
            "sourceAnchorCount": item.source_anchors.len()
        }));
    }
    Ok(json!({
        "documents": documents,
        "items": items,
        "chunks": chunks
    }))
}

/// 完整度审计 `answeringMode` 的**确定性认知状态闸**（方法论点 6：AI 永不自动
/// verify；草稿审定前不可作为事实依据）。
///
/// `fully_supported` 是最强断言——语义是「关键事实维度都已有 verified 客观事实
/// 支撑」。只要知识库里还存在任何 `needs_review` 待审定草稿，该断言就不成立：
/// 草稿尚未审定、不可作为产品/服务事实依据，知识库就不处于「完全支撑」状态，
/// 至多 `product_safe`（有 verified 证据可在边界内回答，但仍有待审定知识）。
///
/// 这是抽象的认知状态规则，对**任意语料**成立（与具体是报价/SLA/案例无关），
/// 放在代码层兜底，不依赖 LLM 自觉——LLM 在 verified 丰富时常无视草稿误判
/// `fully_supported`。`relationship_only`（无 verified）与 `product_safe` 不上调，
/// 只把过强的 `fully_supported` 在有草稿时降一级。纯函数，cfg(test) 锁。
fn clamp_answering_mode(mode: &str, needs_review: u64) -> String {
    if mode == "fully_supported" && needs_review > 0 {
        "product_safe".to_string()
    } else {
        mode.to_string()
    }
}

/// 完整度 gaps 的确定性下界保证：服务端从 DB count 已知的客观缺口（无 verified /
/// 存在 needs_review 草稿）必须恒在 gaps 中，绝不因 LLM 返回 `gaps: []` 而丢失。
/// 与 [`clamp_answering_mode`] 同源——服务端永不信任 LLM 自觉删掉可自证的事实。
/// 合并语义：确定性下界在前（稳定排序）∪ LLM 追加项；按 trim 后文本去重、丢空串。
/// 纯函数、与具体语料无关，cfg(test) 锁。
fn merge_completeness_gaps(deterministic: Vec<String>, llm_gaps: Vec<String>) -> Vec<String> {
    let mut merged: Vec<String> = Vec::with_capacity(deterministic.len() + llm_gaps.len());
    for gap in deterministic.into_iter().chain(llm_gaps.into_iter()) {
        let trimmed = gap.trim();
        if trimmed.is_empty() {
            continue;
        }
        if merged.iter().any(|existing| existing == trimmed) {
            continue;
        }
        merged.push(trimmed.to_string());
    }
    merged
}

pub async fn build_operation_knowledge_completeness(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Value> {
    let account_filter = vec![
        doc! { "account_id": null },
        doc! { "account_id": account_id },
    ];
    let base_filter = doc! {
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "$or": account_filter.clone()
    };
    let total = state
        .db
        .operation_knowledge_chunks()
        .count_documents(base_filter.clone(), None)
        .await?;
    let verified_filter = doc! {
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "status": "active",
        "integrity_status": "verified",
        "$or": account_filter.clone()
    };
    let verified = state
        .db
        .operation_knowledge_chunks()
        .count_documents(verified_filter.clone(), None)
        .await?;
    let evidence = state
        .db
        .operation_knowledge_chunks()
        .count_documents(
            {
                let mut filter = verified_filter.clone();
                filter.insert("evidence_items.0", doc! { "$exists": true });
                filter
            },
            None,
        )
        .await?;
    let anchored = state
        .db
        .operation_knowledge_chunks()
        .count_documents(
            {
                let mut filter = verified_filter.clone();
                filter.insert("source_anchors.0", doc! { "$exists": true });
                filter
            },
            None,
        )
        .await?;
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            verified_filter,
            FindOptions::builder()
                .sort(doc! { "priority": -1, "updated_at": -1 })
                .limit(80)
                .build(),
        )
        .await?;
    let mut summaries = Vec::new();
    while let Some(chunk) = cursor.try_next().await? {
        summaries.push(json!({
            "title": chunk.title,
            "knowledgeType": chunk.knowledge_type,
            "businessContext": chunk.business_context,
            "summary": chunk.summary,
            // body 是该切片的实际内容——可验证事实（具体数字/条款/能力陈述）住在这里，
            // summary 只是一行 teaser。审计要判「该维度有无具体客观事实 vs 仅方法论话术」，
            // 缺 body 就只能从压缩摘要猜，会把「正文含具体能力事实、摘要却读着像方法论」的
            // 切片（如整体方案/对比/概念）误判为仅方法论。补 body 是与具体语料无关的根因修复。
            "body": chunk.body
        }));
    }
    // 待审定（needs_review）切片：审计必须让运营看到「还有多少未审定知识、涉及哪些主题」，
    // 否则完整度报告只报 verified 的好消息、gaps 恒为空，对运营毫无指导价值（真模型在
    // 缺这份上下文时识别不出「报价含未核实草稿」这类缺口）。
    let needs_review_filter = doc! {
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "integrity_status": "needs_review",
        "$or": account_filter.clone()
    };
    let needs_review = state
        .db
        .operation_knowledge_chunks()
        .count_documents(needs_review_filter.clone(), None)
        .await?;
    let mut pending_cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            needs_review_filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(40)
                .build(),
        )
        .await?;
    let mut pending = Vec::new();
    let mut pending_titles: Vec<String> = Vec::new();
    while let Some(chunk) = pending_cursor.try_next().await? {
        let title = chunk.title.trim().to_string();
        if !title.is_empty() {
            pending_titles.push(title);
        }
        pending.push(json!({
            "title": chunk.title,
            "knowledgeType": chunk.knowledge_type
        }));
    }
    let fallback_mode = if verified == 0 {
        "relationship_only"
    } else if evidence == 0 {
        "product_safe"
    } else {
        "fully_supported"
    };
    // fallback gaps：verified==0 报缺 verified；只要存在 needs_review 草稿，
    // 无论 verified 多少都把「待审定知识」列为缺口——AI 永不自动 verify，
    // 这些草稿在审定前不可作为产品事实依据，运营必须看到。
    let mut fallback_gaps: Vec<String> = Vec::new();
    if verified == 0 {
        fallback_gaps.push(
            "能力/边界/证据维度均缺已验证客观事实，需补采可核验事实切片并审定后方可对客".to_string(),
        );
    }
    if needs_review > 0 {
        let topics = pending_titles
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("、");
        let topic_clause = if topics.is_empty() {
            String::new()
        } else {
            format!("（涉及：{topics}）")
        };
        fallback_gaps.push(format!(
            "存在 {needs_review} 条 needs_review 待审定切片{topic_clause}，当前仅为未审定草稿，\
审定前不可作为产品事实依据，需运营逐条核实后审定或标注为不可对客",
        ));
    }
    // 维度认知状态对象：verifiedFact / methodologyOnly / pendingDraft 三个**独立**布尔
    // + 派生 state 摘要。pendingDraft 与 verifiedFact 正交——同一维度可同时「有已审定
    // 客观事实」且「有未审定草稿」（如企业版报价已审定、旗舰版报价仍是草稿），扁平单
    // bool 无法表达这种共存态，会与 gaps 自相矛盾（coverage 说完全覆盖、gap 却说有草稿）。
    // 这是抽象的认知状态表达问题，对任意语料成立，与具体是报价/SLA/案例无关。
    let cov_state = |verified_fact: bool| {
        json!({
            "verifiedFact": verified_fact,
            "methodologyOnly": false,
            "pendingDraft": false,
            "state": if verified_fact { "verified" } else { "missing" }
        })
    };
    let fallback = json!({
        "answeringMode": fallback_mode,
        "summary": if verified == 0 { "当前没有已验证知识切片，Agent 只能做关系维护和需求澄清。" } else { "当前存在已验证知识切片，Agent 可在证据边界内回答事实问题。" },
        "coverage": {
            "capability": cov_state(verified > 0),
            "pricing": cov_state(false),
            "caseEvidence": cov_state(evidence > 0),
            "effectClaims": cov_state(evidence > 0),
            "deliveryBoundary": cov_state(verified > 0)
        },
        "gaps": fallback_gaps.clone()
    });
    let system = "你是企业用户运营知识库完整度 Auditor。你评估已验证知识是否足够支撑 Agent 回答产品/服务事实，并识别尚未审定的知识缺口，不负责生成销售内容。只输出严格 JSON。";
    let user = format!(
        r#"请基于已验证知识切片与待审定切片输出 JSON：
{{
  "answeringMode": "relationship_only | product_safe | fully_supported",
  "summary": "",
  "coverage": {{
    "capability":      {{ "verifiedFact": false, "methodologyOnly": false, "pendingDraft": false }},
    "pricing":         {{ "verifiedFact": false, "methodologyOnly": false, "pendingDraft": false }},
    "caseEvidence":    {{ "verifiedFact": false, "methodologyOnly": false, "pendingDraft": false }},
    "effectClaims":    {{ "verifiedFact": false, "methodologyOnly": false, "pendingDraft": false }},
    "deliveryBoundary":{{ "verifiedFact": false, "methodologyOnly": false, "pendingDraft": false }}
  }},
  "gaps": []
}}

判断规则：
- relationship_only: 没有足够 verified 知识支撑产品/服务事实，只能关系维护、澄清需求、收集信息。
- product_safe: 可回答部分产品/服务能力，但报价、案例、效果或交付边界仍不足。
- fully_supported: 能力、边界、证据类内容足够支撑常见产品事实问题。
- 不要按固定标签硬判，必须从每条切片的 title / knowledgeType / businessContext / summary / body 的真实语义判断它到底覆盖了什么事实，不要只看标题里的关键词。**body 是切片正文，可验证的具体事实（数字/条款/能力陈述）通常住在 body 而非 summary——summary 读着像方法论不代表该切片没有客观事实，务必读 body 判断。**
- 认知状态分类（对所有维度一视同仁，不偏向任何单一维度）：把每条切片对某业务维度的支撑程度归为四类之一——
  1. 已验证客观事实：verified 切片含可直接对客的具体事实（确定的数字/条款/边界/案例数据/效果数字等可被核验的客观信息）；
  2. 仅方法论/话术：只讲怎么做、怎么沟通、价值主张、谈判策略，不含可对客的客观事实数字；
  3. 未审定草稿：相关具体信息只存在于 needs_review 切片中，审定前不可作为事实依据；
  4. 缺失：知识库里没有该维度的任何内容。
- coverage 的每个维度是一个**认知状态对象**，三个布尔位**相互独立、可同时为 true**，必须如实并存标注（不是单选）：
  - "verifiedFact": 该维度存在第 1 类「已验证客观事实」时为 true；
  - "methodologyOnly": 该维度存在第 2 类「仅方法论/话术」内容时为 true；
  - "pendingDraft": 该维度的具体信息存在于 needs_review 草稿中（第 3 类）时为 true。
  **关键：同一维度可以既 verifiedFact=true 又 pendingDraft=true**（例如企业版报价已审定为客观事实、旗舰版报价仍是未审定草稿，则 pricing 的 verifiedFact 与 pendingDraft 都为 true）。绝不能因为有 verified 事实就把 pendingDraft 抹成 false，也绝不能因为有草稿就把已具备的 verifiedFact 抹成 false——两个方向的漏标都要扣分。三位全 false 表示该维度缺失（第 4 类）。此判据对 pricing / caseEvidence / effectClaims / deliveryBoundary / capability 每一维都同样适用。
- 各 coverage 维度判 verifiedFact=true 的命中锚点（满足"已验证客观事实"时即应判 true，**不要漏判**）：
  - capability：有 verified 切片陈述产品/服务"能做什么"的具体能力或功能事实。
  - pricing：有 verified 切片含具体报价/计费/套餐金额（注意：仅 needs_review 草稿里的报价不计入 verifiedFact，而应置 pendingDraft=true 并入 gap）。
  - caseEvidence：有 verified 切片描述**具体客户案例/实施成效**（含可核验的主体、场景或落地结果），即判 true。
  - effectClaims：有 verified 切片含**可核验的效果数据/量化成果**（如转化率提升、响应时长变化等具体数字），即判 true。
  - deliveryBoundary：有 verified 切片陈述交付方式/SLA/可用性/部署边界等具体条款。
- needs_review 切片**尚未审定**，在审定前绝不可作为产品/服务事实依据；若其涉及关键事实维度，必须把对应维度 pendingDraft 置 true、在 gaps 中写明「该主题存在未核实草稿，需运营审定」，且**不得**因草稿存在就判 fully_supported。
- summary 字段必须如实反映知识库现状：对任一关键维度，若 verified 侧只有方法论/话术或仅有未审定草稿，summary 要点明「具备相关方法论但缺已审定的客观事实」，不要笼统说「可回答产品事实」。
- gaps 必须有指导价值：每条 gap 是一句自含的整改指令，需同时写清三要素——①哪个事实维度；②它当前处于哪种认知状态（缺失 / 仅未审定草稿 / 仅方法论话术 / 已有事实但另有待审定草稿）；③运营下一步该做什么（补采可验证事实 / 审定指定草稿 / 标注为不可对客）。**禁止**输出「知识不足」「需完善」之类无维度、无状态、无动作的笼统空话。每个未达 verified 客观事实、或虽有事实但仍存在待审定草稿的维度都要各有一条对应 gap，不要把多维并成一句含糊带过。

统计：total={} verified={} anchored={} evidence={} needsReview={}

已验证知识切片：
{}

待审定（needs_review，尚未审定，不可作为事实依据）切片：
{}"#,
        total,
        verified,
        anchored,
        evidence,
        needs_review,
        serde_json::to_string(&summaries).unwrap_or_default(),
        serde_json::to_string(&pending).unwrap_or_default()
    );
    let audit = state
        .llm
        .generate_json(system, &user)
        .await
        .unwrap_or(fallback);
    let resolved_mode =
        json_string(&audit, "answeringMode").unwrap_or_else(|| fallback_mode.to_string());
    // 认知状态闸：有任何待审定草稿就绝不宣称 fully_supported（见 [`clamp_answering_mode`]）。
    let answering_mode = clamp_answering_mode(&resolved_mode, needs_review);
    // gaps 确定性下界：服务端已知客观缺口恒在，LLM 返回空 gaps 不得抹掉（见 [`merge_completeness_gaps`]）。
    let llm_gaps = json_string_list(&audit, "gaps").unwrap_or_default();
    let gaps = merge_completeness_gaps(fallback_gaps, llm_gaps);
    Ok(json!({
        "totalChunks": total,
        "verifiedChunks": verified,
        "anchoredChunks": anchored,
        "evidenceChunks": evidence,
        "needsReviewChunks": needs_review,
        "pendingReview": pending,
        "answeringMode": answering_mode,
        "summary": json_string(&audit, "summary").unwrap_or_default(),
        "coverage": audit.get("coverage").cloned().unwrap_or_else(|| json!({})),
        "gaps": gaps
    }))
}

pub(super) fn default_user_operations_domain() -> String {
    crate::agent::domain::USER_OPS_DOMAIN_ID.to_string()
}

/// 允许进库的运营域白名单。LLM 在 import-preview / import-apply 中可能输出
/// 自然语言 domain（如 "私域运营"），如果直接写库会让 knowledge_router、
/// list_chunks（filter `domain: "user_operations"`）和 R5.7 反向门全部漏掉
/// 这条切片，等于在数据库里造一份"看不见的孤儿知识"。
///
/// 这里只允许三个已知运营域；其它任何输入都强制归一为 `user_operations`，
/// 因为 Phase 1 唯一上线的运营域就是 user_operations。group/moments 是
/// roadmap 占位，等真实模块上线再扩。
pub(super) fn normalize_operation_domain(input: &str) -> String {
    const KNOWN: &[&str] = &[
        crate::agent::domain::USER_OPS_DOMAIN_ID,
        "group_operations",
        "moments_operations",
    ];
    let trimmed = input.trim();
    if KNOWN.iter().any(|known| *known == trimmed) {
        trimmed.to_string()
    } else {
        default_user_operations_domain()
    }
}

pub(super) fn default_mixed_business_type() -> String {
    "mixed".to_string()
}

pub(super) fn default_manual_source_type() -> String {
    "manual".to_string()
}

pub(super) fn default_imported_markdown_source_type() -> String {
    "imported_markdown".to_string()
}

pub(super) fn default_active_status() -> String {
    "active".to_string()
}


/// 波 D2：knowledge auto-verify 的"最终状态"判定（先于 admin 后台抽样）。
///
/// 性质：
/// - `verified` ⇔ source_quote 非空 ∧ source_anchors 可定位 ∧ LLM 输出
///   `integrityStatus="verified"` ∧ confidence ≥ threshold；
/// - `rejected` ⇔ LLM 明确给出 `rejected` 且不满足 verified 全部条件；
/// - 其它一律 `needs_review`，**包括** 4 项之一缺失但 LLM 自称 verified。
///
/// 这是 spec「auto-verify 证据强约束」的关键判定，单测覆盖防止后续误改。
pub fn decide_auto_verify_status(
    has_source_quote: bool,
    has_source_anchor: bool,
    confidence: i32,
    threshold: i32,
    model_status: &str,
) -> String {
    if has_source_quote
        && has_source_anchor
        && confidence >= threshold
        && model_status == "verified"
    {
        return "verified".to_string();
    }
    if model_status == "rejected" {
        return "rejected".to_string();
    }
    "needs_review".to_string()
}

/// D2 不变量纯函数：verify gate 在 sourceQuote / source_anchors 缺失时必须挡住任何升级路径。
/// 返回 Some(reason) 表示拒绝，None 表示放行。AI 自主修复后的 "应用并立即运营确认" 也必须经过这个 gate。
fn chunk_verify_gate_reason(has_source_quote: bool, has_source_anchor: bool) -> Option<String> {
    if has_source_quote && has_source_anchor {
        return None;
    }
    let mut missing: Vec<&str> = Vec::with_capacity(2);
    if !has_source_quote {
        missing.push("sourceQuote");
    }
    if !has_source_anchor {
        missing.push("source_anchors");
    }
    Some(format!(
        "拒绝运营确认：切片缺少 {}，请补完后再确认。",
        missing.join(" 与 ")
    ))
}

// ── 知识库 AI 自主修复 ────────────────────────────────────────────
//
// 设计：AI 永远只输出 patch，不写库；落库走前端调用现有 PUT /chunks/:id 与
// /chunks/:id/verify。propose handler 只负责拿到 chunk + source + parent
// pack，构造 prompt，调用 generate_agent_json，解析 JSON，写一条
// KnowledgeUsageLog，返回 ChunkRepairProposal。
//
// budget：每次 propose / answer 都开独立 RUN_BUDGET.scope，单轮 token ≤ 4000，
// LLM 调用 ≤ 4。失败/超预算返回 BudgetExceeded（已 200 + 字段，不打 5xx）。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChunkRepairAnswerBody {
    pub session_id: Option<String>,
    pub previous_patch: Option<Value>,
    pub answers: Vec<ChunkRepairAnswer>,
    pub turn: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChunkRepairAnswer {
    pub id: String,
    pub field: Option<String>,
    pub text: String,
}

/// AI 修复 patch 落库后的"应用事件"上报体。
///
/// 前端 `applyAiRepairPatch` 在调用现有 PUT（+ 可选 verify）成功后，再 POST
/// 一次本端点，让审计链能拼出"AI 提议 → 操作员接受 → 落库"的闭环。本端点
/// 不写知识库本身（patch 已通过现有 PUT 写过），只写一条 AgentEvent
/// `kind=knowledge_repair_applied`，并把 `extras`（schema 没有容器、本轮未持
/// 久化进业务字段的领域专属建议）也带进事件 details 里，避免审计黑洞。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RepairApplyBody {
    /// "chunk" / "pack"
    pub target_kind: String,
    pub target_id: String,
    pub session_id: Option<String>,
    pub turn: Option<u8>,
    /// 操作员实际接受落库的字段名列表（不含 extras）。
    #[serde(default)]
    pub accepted_fields: Vec<String>,
    /// 操作员勾掉的字段名列表。
    #[serde(default)]
    pub skipped_fields: Vec<String>,
    /// AI 自评可信度（透传 propose/answer 返回的 confidenceHint，便于审计）。
    pub confidence_hint: Option<i64>,
    /// AI 在 patch.extras 输出的"领域专属字段建议"，schema 无对应容器，
    /// 当前仅作为审计快照保留，不影响业务字段。
    pub extras: Option<Value>,
    /// 应用同时是否触发了运营确认（POST /verify）。
    #[serde(default)]
    pub then_verify: bool,
}

const REPAIR_TOKEN_BUDGET_PER_TURN: i64 = 4_000;
const REPAIR_MAX_LLM_CALLS_PER_TURN: i32 = 4;
const REPAIR_MAX_TURNS: u8 = 3;

fn truncate_for_prompt(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = String::new();
    for ch in input.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("……（已截断）");
    out
}

fn parse_repair_response(value: &Value) -> Value {
    // 透传 LLM 解出来的对象，并对关键字段做最低限度的形态保证：
    // - patch 必须是对象，否则给空对象（前端 diff 会显示空）；
    // - missingFields / stillMissing 元素既可能是字符串（旧形态）也可能是
    //   { field, reason } 对象（通用 prompt 形态），统一规整为 { field, reason } 对象；
    // - followupQuestions 必须是数组、每项是对象，且整体 ≤ 3 条；
    // - interpretation 透传（领域 / 受众 / 用途 / openConditions），前端展示用；
    // - confidenceHint 转成 i64 0-100。
    let patch = value
        .get("patch")
        .cloned()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let interpretation = value
        .get("interpretation")
        .cloned()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let normalize_missing = |field_name: &str| -> Vec<Value> {
        value
            .get(field_name)
            .or_else(|| {
                let snake = field_name
                    .chars()
                    .flat_map(|c| {
                        if c.is_ascii_uppercase() {
                            vec!['_', c.to_ascii_lowercase()]
                        } else {
                            vec![c]
                        }
                    })
                    .collect::<String>();
                value.get(snake)
            })
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        if let Some(s) = item.as_str() {
                            Some(json!({ "field": s, "reason": Value::Null }))
                        } else if item.is_object() {
                            Some(item.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let missing_fields = normalize_missing("missingFields");
    let still_missing = normalize_missing("stillMissing");
    let followup_raw = value
        .get("followupQuestions")
        .or_else(|| value.get("followup_questions"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let followup: Vec<Value> = followup_raw
        .into_iter()
        .filter(|q| q.is_object())
        .take(REPAIR_MAX_TURNS as usize) // 最多 3 条 followup
        .collect();
    let confidence = value
        .get("confidenceHint")
        .or_else(|| value.get("confidence_hint"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .clamp(0, 100);
    json!({
        "interpretation": interpretation,
        "patch": patch,
        "missingFields": missing_fields,
        "followupQuestions": followup,
        "stillMissing": still_missing,
        "confidenceHint": confidence,
    })
}

async fn write_repair_usage_log(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    run_id: &str,
    chunk_object_id: Option<ObjectId>,
    kind: &'static str,
    prompt_key: &'static str,
    target_id: &str,
    turn: u8,
    confidence: i64,
    missing: &[Value],
    followup_count: usize,
) {
    let _ = state
        .db
        .knowledge_usage_logs()
        .insert_one(
            KnowledgeUsageLog {
                id: None,
                workspace_id: workspace_id.to_string(),
                account_id: account_id.to_string(),
                contact_wxid: None,
                run_id: run_id.to_string(),
                knowledge_ids: chunk_object_id.into_iter().collect(),
                route_result: doc! {
                    "kind": kind,
                    "promptKey": prompt_key,
                    "targetId": target_id,
                    "turn": turn as i32,
                    "confidenceHint": confidence,
                    "missingFieldCount": missing.len() as i32,
                    "followupCount": followup_count as i32,
                },
                reply_text: None,
                review_approved: false,
                blocked_reason: Some(format!("{kind}_proposal_pending_operator_apply")),
                tool_trace: vec![doc! { "phase": format!("{kind}_turn_{turn}") }],
                created_at: DateTime::now(),
            },
            None,
        )
        .await;
}

async fn record_repair_event(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    kind: &'static str,
    summary: String,
    details: Document,
) {
    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: workspace_id.to_string(),
                account_id: account_id.to_string(),
                contact_wxid: None,
                kind: kind.to_string(),
                status: "success".to_string(),
                summary,
                details: Some(details),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;
}

pub async fn propose_chunk_repair(
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

    // parent document（用于 sourceQuote 锚定）
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
    // operation_knowledge_items 已删除；pack 永远为 None。
    let pack: Option<()> = None;
    let _ = chunk.item_id;

    let account_id = chunk
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());

    let system = prompts::load_prompt(
        &state.db,
        &admin.current_workspace,
        "knowledge.chunk.repair.propose",
    )
    .await
    .unwrap_or_else(|_| {
        "你是 WechatAgent 知识库 AI 修复 Agent。只输出严格 JSON，包含 patch / missingFields / followupQuestions / confidenceHint。".to_string()
    });

    let document_payload = document
        .as_ref()
        .map(|d| {
            json!({
                "title": d.title,
                "summary": d.summary,
                "rawText": truncate_for_prompt(d.raw_content.as_deref().unwrap_or(""), 4_000),
            })
        })
        .unwrap_or(Value::Null);
    let pack_payload = pack
        .as_ref()
        .map(|_| Value::Null)
        .unwrap_or(Value::Null);

    let user = format!(
        r#"请为下面这条 integrityStatus = needs_review 的知识切片做 AI 自主修复（首轮）。
切片当前内容：
{}

父知识包元数据：
{}

父文档（已截断到 4000 字）：
{}

请先在脑内回答"这条切片在讲什么领域、面向谁、解决什么问题、何时使用"，把判断写进 interpretation 字段；再按 system 中 schema 输出 JSON。followupQuestions 仅在你确实无法从父文档/父知识包推断字段时给出，且与 missingFields 一一对应。如果某 schema 字段在当前领域不适用，写进 missingFields 并附 reason，不要硬填。"#,
        serde_json::to_string_pretty(&operation_knowledge_chunk_json(chunk.clone()))
            .unwrap_or_default(),
        serde_json::to_string_pretty(&pack_payload).unwrap_or_default(),
        serde_json::to_string_pretty(&document_payload).unwrap_or_default(),
    );

    let session_id = uuid::Uuid::new_v4().to_string();
    let run_id = format!("repair-chunk-{}-{}", id, session_id);
    let budget = Arc::new(agent::RunBudget::new(
        run_id.clone(),
        REPAIR_TOKEN_BUDGET_PER_TURN,
        REPAIR_MAX_LLM_CALLS_PER_TURN,
        i32::MAX,
    ));

    let value = agent::RUN_BUDGET
        .scope(budget.clone(), async {
            agent::generate_agent_json(
                &state,
                Some(&account_id),
                None,
                Some(&run_id),
                "knowledge.chunk.repair.propose",
                &system,
                &user,
            )
            .await
        })
        .await?;

    let parsed = parse_repair_response(&value);
    let confidence = parsed
        .get("confidenceHint")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let missing = parsed
        .get("missingFields")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let followup = parsed
        .get("followupQuestions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    write_repair_usage_log(
        &state,
        &admin.current_workspace,
        &account_id,
        &run_id,
        chunk.id,
        "chunk_repair_session",
        "knowledge.chunk.repair.propose",
        &id,
        1,
        confidence,
        &missing,
        followup.len(),
    )
    .await;
    record_repair_event(
        &state,
        &admin.current_workspace,
        &account_id,
        "knowledge_repair_proposed",
        format!("AI 自主修复 chunk:{id} 第 1 轮"),
        doc! {
            "kind": "chunk_repair_session",
            "chunkId": &id,
            "turn": 1i32,
            "confidenceHint": confidence,
            "followupCount": followup.len() as i32,
            "missingFieldCount": missing.len() as i32,
            "budget": budget_document(&budget),
        },
    )
    .await;

    Ok(Json(json!({
        "chunkId": id,
        "sessionId": session_id,
        "turn": 1,
        "promptKey": "knowledge.chunk.repair.propose",
        "interpretation": parsed.get("interpretation"),
        "patch": parsed.get("patch"),
        "missingFields": parsed.get("missingFields"),
        "followupQuestions": parsed.get("followupQuestions"),
        "stillMissing": parsed.get("stillMissing"),
        "confidenceHint": parsed.get("confidenceHint"),
        "budget": budget_document(&budget),
    })))
}

pub(super) async fn answer_chunk_repair(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(body): Json<ChunkRepairAnswerBody>,
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

    let turn = body.turn.unwrap_or(2).clamp(2, REPAIR_MAX_TURNS);
    let session_id = body
        .session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let account_id = chunk
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());

    let system = prompts::load_prompt(
        &state.db,
        &admin.current_workspace,
        "knowledge.chunk.repair.followup",
    )
    .await
    .unwrap_or_else(|_| {
        "你是 WechatAgent 知识库 AI 修复 Agent，正在合并操作员对追问的回答。只输出严格 JSON。".to_string()
    });

    let answers_for_prompt: Vec<Value> = body
        .answers
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "field": a.field.clone().unwrap_or_default(),
                "text": truncate_for_prompt(&a.text, 600),
            })
        })
        .collect();

    let user = format!(
        r#"这是 chunk:{} 的 AI 自主修复 followup 轮（第 {} 轮，最多 {} 轮）。
上一轮 patch：
{}

操作员对追问的回答：
{}

请把回答合并到 patch（不要原话搬运），按 system 中 schema 输出 JSON，包含 interpretation / patch / stillMissing / followupQuestions / confidenceHint。如果当前已是第 {} 轮（最后一轮），followupQuestions 必须为空数组。"#,
        id,
        turn,
        REPAIR_MAX_TURNS,
        serde_json::to_string_pretty(&body.previous_patch.clone().unwrap_or(Value::Null))
            .unwrap_or_default(),
        serde_json::to_string_pretty(&answers_for_prompt).unwrap_or_default(),
        REPAIR_MAX_TURNS,
    );

    let run_id = format!("repair-chunk-{}-{}", id, session_id);
    let budget = Arc::new(agent::RunBudget::new(
        run_id.clone(),
        REPAIR_TOKEN_BUDGET_PER_TURN,
        REPAIR_MAX_LLM_CALLS_PER_TURN,
        i32::MAX,
    ));

    let value = agent::RUN_BUDGET
        .scope(budget.clone(), async {
            agent::generate_agent_json(
                &state,
                Some(&account_id),
                None,
                Some(&run_id),
                "knowledge.chunk.repair.followup",
                &system,
                &user,
            )
            .await
        })
        .await?;

    let parsed = parse_repair_response(&value);
    let confidence = parsed
        .get("confidenceHint")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let still_missing = parsed
        .get("stillMissing")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // 最后一轮：强制忽略 LLM 任何尝试再追问的内容。
    let followup = if turn >= REPAIR_MAX_TURNS {
        Vec::<Value>::new()
    } else {
        parsed
            .get("followupQuestions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };

    write_repair_usage_log(
        &state,
        &admin.current_workspace,
        &account_id,
        &run_id,
        chunk.id,
        "chunk_repair_session",
        "knowledge.chunk.repair.followup",
        &id,
        turn,
        confidence,
        &still_missing,
        followup.len(),
    )
    .await;
    record_repair_event(
        &state,
        &admin.current_workspace,
        &account_id,
        "knowledge_repair_proposed",
        format!("AI 自主修复 chunk:{id} 第 {turn} 轮"),
        doc! {
            "kind": "chunk_repair_session",
            "chunkId": &id,
            "turn": turn as i32,
            "confidenceHint": confidence,
            "followupCount": followup.len() as i32,
            "stillMissingCount": still_missing.len() as i32,
            "budget": budget_document(&budget),
        },
    )
    .await;

    Ok(Json(json!({
        "chunkId": id,
        "sessionId": session_id,
        "turn": turn,
        "promptKey": "knowledge.chunk.repair.followup",
        "interpretation": parsed.get("interpretation"),
        "patch": parsed.get("patch"),
        "stillMissing": still_missing,
        "followupQuestions": followup,
        "confidenceHint": confidence,
        "isFinalTurn": turn >= REPAIR_MAX_TURNS,
        "budget": budget_document(&budget),
    })))
}

pub(super) async fn propose_pack_repair(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> AppResult<Json<Value>> {
    // operation_knowledge_items 已删除；pack-level 修复路径暂时下线，
    // 等 wiki Phase 重新规划包级别 repair。
    Err(AppError::BadRequest(
        "operation_knowledge_items has been removed; pack repair temporarily disabled"
            .to_string(),
    ))
}

/// 把 `patch.extras`（如果有）按 JSON 形态分类，仅用于审计 detail 中的
/// `extrasKind` 字段，便于后续按 kind 过滤。
fn classify_extras_kind(extras: Option<&Value>) -> &'static str {
    match extras {
        None => "absent",
        Some(v) if v.is_null() => "null",
        Some(v) if v.is_object() => "object",
        Some(v) if v.is_array() => "array",
        Some(_) => "scalar",
    }
}

/// 拼装"AI 修复落库"事件的人类可读 summary。仅用于 AgentEvent.summary，details
/// 仍然按字段拆分写。文案严守 AI 自治定位，不引入暗示外部托管的字面量。
fn format_repair_apply_summary(
    target_kind: &str,
    target_id: &str,
    accepted_count: i32,
    skipped_count: i32,
    then_verify: bool,
) -> String {
    format!(
        "AI 自主修复落库 {} {}（接受 {} 项 / 跳过 {} 项 / 同时确认={}）",
        target_kind, target_id, accepted_count, skipped_count, then_verify
    )
}

/// AI 修复 patch 落库后的"应用事件"端点（POST /api/operation-knowledge/repair/applied）。
///
/// 与 propose / answer 不同，本端点**不调 LLM、不查知识、不写知识本身**——它
/// 只为闭合审计链路而存在：前端 `applyAiRepairPatch` 在已经把 patch 通过现有
/// PUT 写进 chunk/pack（以及可选地走完 /verify）之后，再调用本端点，让
/// `agent_events` 留下一条 `kind=knowledge_repair_applied` 行，details 里携带
/// 操作员实际接受/跳过了哪些字段、是否同时触发 verify、AI 自评可信度，以及
/// AI 在 patch.extras 里输出但 schema 暂无容器的"领域专属字段建议"快照。
///
/// 不做的事：
/// - 不验证字段名合法性（前端已经过 PUT 校验，这里若再校一遍只会出现错位告警）；
/// - 不写 KnowledgeUsageLog（usage log 已在 propose/answer 阶段记过，应用阶段
///   只是事件，不再消耗 LLM）；
/// - 不写主业务集合（patch 已通过现有 PUT 落库，重复写会破坏只读性）。
pub(super) async fn record_repair_apply(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<RepairApplyBody>,
) -> AppResult<Json<Value>> {
    let kind_label = match body.target_kind.as_str() {
        "chunk" => "chunk_repair_session",
        "pack" => "pack_repair_session",
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown repair target kind: {other}"
            )))
        }
    };

    if body.target_id.trim().is_empty() {
        return Err(AppError::BadRequest("targetId cannot be empty".to_string()));
    }

    // 取 account_id：优先从被改写的对象上取，找不到就 fallback default_account_id。
    // 不阻塞调用：任何错误都退化为 None 走 fallback。
    let resolved_account = match body.target_kind.as_str() {
        "chunk" => match parse_object_id(&body.target_id) {
            Ok(oid) => state
                .db
                .operation_knowledge_chunks()
                .find_one(
                    doc! {
                        "_id": oid,
                        "workspace_id": &admin.current_workspace
                    },
                    None,
                )
                .await
                .ok()
                .flatten()
                .and_then(|c| c.account_id),
            Err(_) => None,
        },
        "pack" => {
            // operation_knowledge_items 已删除；pack 维度的 account_id 解析回退到默认账号。
            let _ = parse_object_id(&body.target_id);
            None
        }
        _ => None,
    };
    let account_id =
        resolved_account.unwrap_or_else(|| state.config.default_account_id.clone());

    let accepted_count = body.accepted_fields.len() as i32;
    let skipped_count = body.skipped_fields.len() as i32;
    let extras_doc = body
        .extras
        .as_ref()
        .and_then(|v| mongodb::bson::to_bson(v).ok())
        .unwrap_or(Bson::Null);
    let extras_kind = classify_extras_kind(body.extras.as_ref());

    let summary = format_repair_apply_summary(
        &body.target_kind,
        &body.target_id,
        accepted_count,
        skipped_count,
        body.then_verify,
    );

    record_repair_event(
        &state,
        &admin.current_workspace,
        &account_id,
        "knowledge_repair_applied",
        summary.clone(),
        doc! {
            "kind": kind_label,
            "targetKind": &body.target_kind,
            "targetId": &body.target_id,
            "sessionId": body.session_id.clone().unwrap_or_default(),
            "turn": body.turn.unwrap_or(0) as i32,
            "acceptedFields": &body.accepted_fields,
            "skippedFields": &body.skipped_fields,
            "acceptedCount": accepted_count,
            "skippedCount": skipped_count,
            "thenVerify": body.then_verify,
            "confidenceHint": body.confidence_hint.unwrap_or(0),
            "extrasKind": extras_kind,
            "extras": extras_doc,
        },
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "summary": summary,
        "extrasRecorded": extras_kind != "absent",
    })))
}

// ===========================================================================
// 知识库对话式 Agent（chat）：单轮 turn / 历史 / apply / discard
// ---------------------------------------------------------------------------
// 设计目标：让运营在前端用对话方式新建 / 修改 / 澄清切片或知识包，AI 解析意图、
// 起草 patch、提出追问；运营满意后再「应用为草稿」落库为 status=draft +
// integrityStatus=needs_review，由现有 verify gate 把守活跃池。
//
// 强约束：
// 1. 每轮 RUN_BUDGET ≤ CHAT_TOKEN_BUDGET_PER_TURN / ≤ CHAT_MAX_LLM_CALLS_PER_TURN，
//    超限返回 BudgetExceeded 不打 5xx；
// 2. 每条 turn 写 knowledge_chat_turns 持久化；运营关闭浏览器后凭 sessionId 续聊；
// 3. 每轮成功写 AgentEvent kind="knowledge_chat_turn"；
// 4. apply 必须强制 status=draft + integrityStatus=needs_review；
// 5. AI 不写 verified；落库后由现有 /chunks/:id/verify + sourceQuote→anchor gate
//    把守。
// ===========================================================================

const CHAT_TOKEN_BUDGET_PER_TURN: i64 = 6_000;
const CHAT_MAX_LLM_CALLS_PER_TURN: i32 = 4;
const CHAT_MAX_TURNS_PER_SESSION: i32 = 8;
const CHAT_MAX_FOLLOWUPS: usize = 3;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    pub chunk_id: Option<String>,
    pub item_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurnRequest {
    /// 缺省则后端 new uuid 当 sessionId。
    pub session_id: Option<String>,
    pub account_id: Option<String>,
    /// knowledge-digest-workstation Phase 5：运营 ID（用于隔离 operator memory）。
    /// 缺省回退到 `default`，与 chat_task_create 字段对齐。
    pub operator_id: Option<String>,
    pub content: String,
    /// 引用的切片 / 知识包；本轮只取第 1 条（≤ 1 attachments）。
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
}

pub async fn chat_turn(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<ChatTurnRequest>,
) -> AppResult<Json<Value>> {
    let trimmed = body.content.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "content cannot be empty".to_string(),
        ));
    }
    let session_id = body
        .session_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let account_id = body
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let operator_id = body
        .operator_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string());

    // 加载历史 turns（按 turn_index 升序）
    let history = load_chat_history(&state, &admin.current_workspace, &account_id, &session_id).await?;
    // P1-7：原子预分配两个 turn_index——user turn + assistant turn，避免并发
    // 写者读到同一 last 制造重复索引。返回的是分配后的最大 seq；user 拿
    // `assistant_index - 1`、assistant 拿 `assistant_index`。
    let assistant_index =
        allocate_next_turn_indices(&state, &admin.current_workspace, &session_id, 2).await?;
    let next_index = assistant_index - 1;
    let assistant_turns_so_far = history
        .iter()
        .filter(|t| t.role == "assistant")
        .count() as i32;
    if assistant_turns_so_far >= CHAT_MAX_TURNS_PER_SESSION {
        return Err(AppError::BadRequest(format!(
            "session {session_id} 已达 {CHAT_MAX_TURNS_PER_SESSION} 轮上限，请「应用为草稿」或开启新会话"
        )));
    }

    // 写 user turn
    write_chat_turn(
        &state,
        &admin.current_workspace,
        &account_id,
        &session_id,
        next_index,
        "user",
        None,
        trimmed,
        &body.attachments,
        None,
        &[],
        &[],
        "pending",
        0,
        None,
    )
    .await?;

    let attachment = body.attachments.first();
    let chunk_attached = attachment
        .and_then(|a| a.chunk_id.as_deref())
        .filter(|s| !s.trim().is_empty());
    let item_attached = attachment
        .and_then(|a| a.item_id.as_deref())
        .filter(|s| !s.trim().is_empty());

    let run_id = format!("chat-{session_id}-turn-{next_index}");
    let budget = Arc::new(agent::RunBudget::new(
        run_id.clone(),
        CHAT_TOKEN_BUDGET_PER_TURN,
        CHAT_MAX_LLM_CALLS_PER_TURN,
        i32::MAX,
    ));

    let result = agent::RUN_BUDGET
        .scope(budget.clone(), async {
            run_chat_turn_pipeline(
                &state,
                &admin.current_workspace,
                &account_id,
                &operator_id,
                &session_id,
                trimmed,
                chunk_attached,
                item_attached,
                &history,
            )
            .await
        })
        .await?;

    let intent = result
        .get("intent")
        .and_then(|v| v.as_str())
        .unwrap_or("freeform")
        .to_string();
    let natural_reply = result
        .get("naturalReply")
        .and_then(|v| v.as_str())
        .unwrap_or("（AI 未给出回复）")
        .to_string();
    let patch = result.get("patch").cloned();
    let missing_fields: Vec<String> = result
        .get("missingFields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    x.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| {
                            x.get("field").and_then(|f| f.as_str()).map(|s| s.to_string())
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    let followups: Vec<Value> = result
        .get("followupQuestions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(CHAT_MAX_FOLLOWUPS)
        .collect();
    let draft_kind = result
        .get("draftKind")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let target_chunk_id = result
        .get("targetChunkId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let target_pack_id = result
        .get("targetPackId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let prompt_key = result
        .get("promptKey")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // knowledge-digest-workstation Phase 4 / P4.4：digest_action intent 命中时
    // LLM 出 plannedSteps + estimatedLlmCalls，转发给前端弹「派工确认」小卡。
    let planned_steps = result.get("plannedSteps").cloned();
    let estimated_llm_calls = result
        .get("estimatedLlmCalls")
        .and_then(|v| v.as_i64());
    let can_apply = patch.is_some()
        && missing_fields.is_empty()
        && draft_kind.is_some();
    let tokens_used = budget.snapshot().tokens_used;

    // 写 assistant turn
    let attachments_for_assistant: Vec<ChatAttachment> = match (&target_chunk_id, &target_pack_id) {
        (Some(c), _) => vec![ChatAttachment {
            chunk_id: Some(c.clone()),
            item_id: None,
        }],
        (None, Some(p)) => vec![ChatAttachment {
            chunk_id: None,
            item_id: Some(p.clone()),
        }],
        _ => body.attachments,
    };

    write_chat_turn(
        &state,
        &admin.current_workspace,
        &account_id,
        &session_id,
        assistant_index,
        "assistant",
        Some(&intent),
        &natural_reply,
        &attachments_for_assistant,
        patch.as_ref(),
        &missing_fields,
        &followups,
        "pending",
        tokens_used,
        prompt_key.as_deref(),
    )
    .await?;

    // P2-15：chat 路径的 KnowledgeUsageLog 必须带 promptVersions，复用 R11 既有 prompt 版本
    // 审计语义（与日报 / management 路径对齐）。一次 turn 可能命中 intent/draft/update/clarify
    // 中的多个，统一拉取 4 把 chat 钥匙的 active 版本号；prompt_versions 拉取失败不阻塞主链路。
    let chat_prompt_versions = prompts::prompt_versions(
        &state.db,
        &admin.current_workspace,
        &[
            "knowledge.chat.intent",
            "knowledge.chat.draft_chunk",
            "knowledge.chat.update_chunk",
            "knowledge.chat.clarify",
        ],
        None,
        None,
    )
    .await
    .unwrap_or_else(|_| doc! {});

    let usage_doc = doc! {
        "kind": "chunk_chat_session",
        "intent": &intent,
        "sessionId": &session_id,
        "turnIndex": assistant_index as i32,
        "missingFieldCount": missing_fields.len() as i32,
        "followupCount": followups.len() as i32,
        "draftKind": draft_kind.clone().unwrap_or_default(),
        "promptKey": prompt_key.clone().unwrap_or_default(),
        "promptVersions": chat_prompt_versions.clone(),
    };
    let _ = state
        .db
        .knowledge_usage_logs()
        .insert_one(
            KnowledgeUsageLog {
                id: None,
                workspace_id: admin.current_workspace.clone(),
                account_id: account_id.clone(),
                contact_wxid: None,
                run_id: run_id.clone(),
                knowledge_ids: vec![],
                route_result: usage_doc,
                reply_text: Some(natural_reply.clone()),
                review_approved: false,
                blocked_reason: Some("chunk_chat_session_pending_operator_apply".to_string()),
                tool_trace: vec![doc! { "phase": format!("chunk_chat_turn_{assistant_index}") }],
                created_at: DateTime::now(),
            },
            None,
        )
        .await;
    record_repair_event(
        &state,
        &admin.current_workspace,
        &account_id,
        "knowledge_chat_turn",
        format!(
            "AI 对话补完 sessionId={session_id} 第 {assistant_index} 轮 intent={intent}"
        ),
        doc! {
            "kind": "chunk_chat_session",
            "sessionId": &session_id,
            "turnIndex": assistant_index as i32,
            "intent": &intent,
            "missingFieldCount": missing_fields.len() as i32,
            "followupCount": followups.len() as i32,
            "tokensUsed": tokens_used,
            "draftKind": draft_kind.clone().unwrap_or_default(),
            "budget": budget_document(&budget),
        },
    )
    .await;

    Ok(Json(json!({
        "sessionId": session_id,
        "turnIndex": assistant_index,
        "intent": intent,
        "naturalReply": natural_reply,
        "draftKind": draft_kind,
        "draftPreview": patch,
        "plannedSteps": planned_steps,
        "estimatedLlmCalls": estimated_llm_calls,
        "missingFields": missing_fields,
        "followupQuestions": followups,
        "canApply": can_apply,
        "targetChunkId": target_chunk_id,
        "targetPackId": target_pack_id,
        "promptKey": prompt_key,
        "tokensUsed": tokens_used,
        "budget": budget_document(&budget),
    })))
}

pub(super) async fn chat_history(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
) -> AppResult<Json<Value>> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let mut cursor = state
        .db
        .knowledge_chat_turns()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "session_id": trimmed,
            },
            FindOptions::builder().sort(doc! { "turn_index": 1 }).build(),
        )
        .await?;
    let mut items: Vec<Value> = vec![];
    while let Some(turn) = cursor.try_next().await? {
        items.push(chat_turn_to_view(&turn));
    }
    Ok(Json(json!({
        "sessionId": trimmed,
        "items": items,
        "total": items.len() as i32,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatApplyRequest {
    pub account_id: Option<String>,
}

pub(super) async fn chat_apply(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
    Json(body): Json<ChatApplyRequest>,
) -> AppResult<Json<Value>> {
    let trimmed = session_id.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let history = load_chat_history(&state, &admin.current_workspace, "*", &trimmed).await?;
    let last_assistant = history
        .iter()
        .rev()
        .find(|t| t.role == "assistant" && t.status == "pending" && t.patch.is_some())
        .ok_or_else(|| {
            AppError::BadRequest(
                "session 没有可应用的 AI 草稿（需要先发起 chat 让 AI 起草）".to_string(),
            )
        })?;

    let intent = last_assistant.intent.as_deref().unwrap_or("freeform");
    let patch = last_assistant
        .patch
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("最近一轮 AI 没有 patch".to_string()))?;

    let account_id = body
        .account_id
        .clone()
        .or_else(|| {
            if last_assistant.account_id.is_empty() {
                None
            } else {
                Some(last_assistant.account_id.clone())
            }
        })
        .unwrap_or_else(|| state.config.default_account_id.clone());

    // 取出 attachments 中的 chunk_id / item_id（assistant 已回填）
    let target_chunk_id = last_assistant
        .attachments
        .iter()
        .filter_map(|a| a.get_str("chunk_id").ok())
        .find(|s| !s.is_empty())
        .map(|s| s.to_string());
    let target_pack_id = last_assistant
        .attachments
        .iter()
        .filter_map(|a| a.get_str("item_id").ok())
        .find(|s| !s.is_empty())
        .map(|s| s.to_string());

    let result_value = match intent {
        "create_chunk" => {
            apply_create_chunk(&state, &admin.current_workspace, &account_id, &trimmed, patch, target_pack_id.as_deref())
                .await?
        }
        "update_chunk" => {
            let chunk_id = target_chunk_id.clone().ok_or_else(|| {
                AppError::BadRequest("update_chunk 需要 attachments.chunkId".to_string())
            })?;
            apply_update_chunk(&state, &admin.current_workspace, &account_id, &chunk_id, patch).await?
        }
        "update_pack" => {
            let pack_id = target_pack_id.clone().ok_or_else(|| {
                AppError::BadRequest("update_pack 需要 attachments.itemId".to_string())
            })?;
            apply_update_pack(&state, &account_id, &pack_id, patch).await?
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "intent={other} 不可应用为草稿（仅 create_chunk / update_chunk / update_pack 可应用）"
            )));
        }
    };

    // 标 turn applied
    state
        .db
        .knowledge_chat_turns()
        .update_one(
            doc! {
                "_id": last_assistant.id.expect("turn must have id"),
                "workspace_id": &admin.current_workspace,
            },
            doc! { "$set": { "status": "applied", "updated_at": DateTime::now() } },
            None,
        )
        .await?;

    record_repair_event(
        &state,
        &admin.current_workspace,
        &account_id,
        "knowledge_chat_applied",
        format!("AI 对话产物落库为草稿 sessionId={trimmed} intent={intent}"),
        doc! {
            "kind": "chunk_chat_session",
            "sessionId": &trimmed,
            "intent": intent,
            "result": mongodb::bson::to_bson(&result_value).unwrap_or(Bson::Null),
        },
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "sessionId": trimmed,
        "intent": intent,
        "result": result_value,
    })))
}

pub(super) async fn chat_discard(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
) -> AppResult<Json<Value>> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let res = state
        .db
        .knowledge_chat_turns()
        .update_many(
            doc! {
                "workspace_id": &admin.current_workspace,
                "session_id": trimmed,
                "status": "pending",
            },
            doc! { "$set": { "status": "discarded", "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    Ok(Json(json!({
        "ok": true,
        "sessionId": trimmed,
        "discardedCount": res.modified_count,
    })))
}

// ----- chat 内部辅助 -------------------------------------------------------

/// P1-7：原子分配下一个 `turn_index`。
///
/// 历史路径是「`find_one(sort=desc).turn_index + 1`」，并发两个写者会读到同一
/// `last`，写出重复 turn_index。本路径用 `knowledge_chat_session_seqs` 行
/// `{ _id: "{workspace_id}|{session_id}", seq: i64 }`，配 `findOneAndUpdate`
/// `$inc: { seq: count }` `upsert(true)` `returnDocument=After` 单次原子调
/// 用，返回的 `seq` 即为「分配给本次写入的最后一个 turn_index」；调用方需要
/// 一次写多条 turn 时传 `count > 1`，按 `seq - count + 1 .. seq` 顺序使用。
///
/// 注意：本助手 SHALL ONLY 用来分配新 turn_index，不能用来读历史 turn 数；
/// 历史拉取仍走 `load_chat_history`。
pub(super) async fn allocate_next_turn_indices(
    state: &AppState,
    workspace_id: &str,
    session_id: &str,
    count: u32,
) -> AppResult<i32> {
    use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
    let n = count.max(1) as i64;
    let key = format!("{}|{}", workspace_id, session_id);
    let updated = state
        .db
        .knowledge_chat_session_seqs()
        .find_one_and_update(
            doc! { "_id": &key },
            doc! { "$inc": { "seq": n } },
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let seq = updated
        .as_ref()
        .and_then(|d| d.get_i64("seq").ok())
        .unwrap_or(n);
    // turn_index 字段在模型里是 i32；上限远超 i32::MAX 时直接 saturating，
    // 单 session ≥ 21 亿 turn 不在产品语义范围内。
    Ok(seq.try_into().unwrap_or(i32::MAX))
}

async fn load_chat_history(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
) -> AppResult<Vec<KnowledgeChatTurn>> {
    let mut filter = doc! {
        "workspace_id": workspace_id,
        "session_id": session_id,
    };
    if account_id != "*" {
        filter.insert("account_id", account_id);
    }
    let mut cursor = state
        .db
        .knowledge_chat_turns()
        .find(
            filter,
            FindOptions::builder().sort(doc! { "turn_index": 1 }).build(),
        )
        .await?;
    let mut items = vec![];
    while let Some(t) = cursor.try_next().await? {
        items.push(t);
    }
    Ok(items)
}

#[allow(clippy::too_many_arguments)]
async fn write_chat_turn(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    turn_index: i32,
    role: &str,
    intent: Option<&str>,
    content: &str,
    attachments: &[ChatAttachment],
    patch: Option<&Value>,
    missing_fields: &[String],
    followups: &[Value],
    status: &str,
    tokens_used: i64,
    prompt_key: Option<&str>,
) -> AppResult<()> {
    let attachments_doc: Vec<Document> = attachments
        .iter()
        .filter_map(|a| {
            let mut d = Document::new();
            if let Some(c) = a.chunk_id.as_deref().filter(|s| !s.is_empty()) {
                d.insert("chunk_id", c.to_string());
            }
            if let Some(i) = a.item_id.as_deref().filter(|s| !s.is_empty()) {
                d.insert("item_id", i.to_string());
            }
            if d.is_empty() {
                None
            } else {
                Some(d)
            }
        })
        .collect();
    let patch_doc = patch
        .and_then(|p| mongodb::bson::to_bson(p).ok())
        .and_then(|b| match b {
            Bson::Document(d) => Some(d),
            _ => None,
        });
    let followup_docs: Vec<Document> = followups
        .iter()
        .filter_map(|v| mongodb::bson::to_bson(v).ok())
        .filter_map(|b| match b {
            Bson::Document(d) => Some(d),
            _ => None,
        })
        .collect();

    state
        .db
        .knowledge_chat_turns()
        .insert_one(
            KnowledgeChatTurn {
                id: None,
                workspace_id: workspace_id.to_string(),
                account_id: account_id.to_string(),
                session_id: session_id.to_string(),
                turn_index,
                role: role.to_string(),
                intent: intent.map(|s| s.to_string()),
                content: content.to_string(),
                attachments: attachments_doc,
                patch: patch_doc,
                missing_fields: missing_fields.to_vec(),
                followup_questions: followup_docs,
                status: status.to_string(),
                tokens_used,
                prompt_key: prompt_key.map(|s| s.to_string()),
                created_at: DateTime::now(),
                kind: None,
                tool_calls: vec![],
            },
            None,
        )
        .await?;
    Ok(())
}

fn chat_turn_to_view(turn: &KnowledgeChatTurn) -> Value {
    json!({
        "id": turn.id.map(|o| o.to_hex()),
        "sessionId": turn.session_id,
        "turnIndex": turn.turn_index,
        "role": turn.role,
        "intent": turn.intent,
        "content": turn.content,
        "attachments": turn.attachments,
        "patch": turn.patch,
        "missingFields": turn.missing_fields,
        "followupQuestions": turn.followup_questions,
        "status": turn.status,
        "tokensUsed": turn.tokens_used,
        "promptKey": turn.prompt_key,
        // knowledge-digest-workstation Phase 4：worker 写的进度 turn 用
        // `kind = task_progress / task_summary / tool_call_log` 区分；
        // freeform / chat 默认不写。
        "kind": turn.kind,
        "toolCalls": turn.tool_calls,
        "createdAt": turn.created_at.try_to_rfc3339_string().unwrap_or_default(),
    })
}

/// 当 LLM 产出了 patch/起草结果却漏写 naturalReply（或留空）时，从结构化
/// 字段确定性地合成一句对话回执。通用于所有 draft/update 分支、与具体业务
/// 领域无关：只读结构化字段名，不内嵌任何样例文案。
fn synthesize_natural_reply_from_patch(out: &Value) -> Option<String> {
    let patch = out.get("patch")?.as_object()?;
    fn field_label(k: &str) -> &str {
        match k {
            "title" => "标题",
            "summary" => "摘要",
            "body" => "正文",
            "tags" => "标签",
            "knowledgeType" | "knowledge_type" => "知识类型",
            "priority" => "优先级",
            other => other,
        }
    }
    let filled: Vec<&str> = patch
        .iter()
        .filter(|(_, v)| match v {
            Value::String(s) => !s.trim().is_empty(),
            Value::Null => false,
            Value::Array(a) => !a.is_empty(),
            _ => true,
        })
        .map(|(k, _)| field_label(k.as_str()))
        .collect();
    if filled.is_empty() {
        return None;
    }
    let missing: Vec<String> = out
        .get("missingFields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    x.as_str().map(|s| s.to_string()).or_else(|| {
                        x.get("field").and_then(|f| f.as_str()).map(|s| s.to_string())
                    })
                })
                .map(|s| field_label(&s).to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut reply = if let Some(t) = patch
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        format!("我已经按您的要求起草好{}，拟定的标题是「{t}」。", filled.join("、"))
    } else {
        format!("我已经为您起草好了{}。", filled.join("、"))
    };
    if missing.is_empty() {
        reply.push_str("您看一下内容是否准确，确认无误后即可应用为草稿。");
    } else {
        reply.push_str(&format!(
            "还差{} 需要补充，方便的话请再给我一些信息，我好把它补全。",
            missing.join("、")
        ));
    }
    Some(reply)
}

/// chat_turn 的核心 LLM 编排：先识别 intent，再分流到对应子 prompt。
/// 返回的 Value 至少包含 intent / naturalReply；可选 patch / missingFields /
/// followupQuestions / draftKind / targetChunkId / targetPackId / promptKey。
async fn run_chat_turn_pipeline(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    operator_id: &str,
    session_id: &str,
    user_content: &str,
    chunk_attached: Option<&str>,
    item_attached: Option<&str>,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    // knowledge-digest-workstation Phase 5：先取运营长期偏好记忆，作为
    // intent 分类与下游分支的 prompt header。与 contacts.memory_card 物理
    // 隔离（仅触达 knowledge_operator_memory collection）。
    let operator_memory = agent::load_operator_memory(
        &state.db,
        workspace_id,
        account_id,
        operator_id,
        5,
    )
    .await
    .unwrap_or_default();
    let operator_memory_header = render_operator_memory_for_prompt(&operator_memory);

    // 1. intent 分类
    let intent_result = classify_intent(
        state,
        workspace_id,
        account_id,
        session_id,
        user_content,
        chunk_attached,
        item_attached,
        history,
        &operator_memory_header,
    )
    .await?;
    let intent = intent_result
        .get("intent")
        .and_then(|v| v.as_str())
        .unwrap_or("freeform")
        .to_string();
    let target_chunk_id = intent_result
        .get("targetChunkId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| chunk_attached.map(|s| s.to_string()));
    let target_pack_id = intent_result
        .get("targetPackId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| item_attached.map(|s| s.to_string()));

    // 2. 按 intent 分流
    let mut out = match intent.as_str() {
        "create_chunk" => draft_chunk_for_chat(
            state,
            workspace_id,
            account_id,
            session_id,
            user_content,
            target_pack_id.as_deref(),
            history,
        )
        .await
        .map(|mut v| {
            v["draftKind"] = json!("chunk");
            v["promptKey"] = json!("knowledge.chat.draft_chunk");
            v
        })?,
        "update_chunk" => {
            let chunk_id = target_chunk_id.clone().ok_or_else(|| {
                AppError::BadRequest(
                    "update_chunk 需要 attachments.chunkId 或在对话中明确引用切片".to_string(),
                )
            })?;
            let mut v = update_chunk_for_chat(
                state,
                workspace_id,
                account_id,
                session_id,
                user_content,
                &chunk_id,
                history,
            )
            .await?;
            v["draftKind"] = json!("chunk_update");
            v["promptKey"] = json!("knowledge.chat.update_chunk");
            v
        }
        "update_pack" => {
            let pack_id = target_pack_id.clone().ok_or_else(|| {
                AppError::BadRequest(
                    "update_pack 需要 attachments.itemId 或在对话中明确引用知识包".to_string(),
                )
            })?;
            let mut v = update_pack_for_chat(
                state,
                workspace_id,
                account_id,
                session_id,
                user_content,
                &pack_id,
                history,
            )
            .await?;
            v["draftKind"] = json!("pack_update");
            v["promptKey"] = json!("knowledge.chat.update_chunk");
            v
        }
        "digest_action" => {
            let mut v = dispatch_digest_action_for_chat(
                state,
                workspace_id,
                account_id,
                session_id,
                user_content,
                history,
            )
            .await?;
            v["draftKind"] = json!("digest_dispatch");
            v["promptKey"] = json!("knowledge.digest.dispatch");
            v
        }
        "update_operator_memory" => {
            let mut v = update_operator_memory_for_chat(
                state,
                workspace_id,
                account_id,
                operator_id,
                user_content,
                &intent_result,
            )
            .await?;
            v["draftKind"] = json!("operator_memory");
            v["promptKey"] = json!("knowledge.chat.intent");
            v
        }
        _ => clarify_for_chat(state, workspace_id, account_id, session_id, user_content, history)
            .await
            .map(|mut v| {
                v["promptKey"] = json!("knowledge.chat.clarify");
                v
            })?,
    };

    out["intent"] = json!(intent);
    if let Some(c) = target_chunk_id {
        out["targetChunkId"] = json!(c);
    }
    if let Some(p) = target_pack_id {
        out["targetPackId"] = json!(p);
    }
    let reply_blank = out
        .get("naturalReply")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().is_empty() || s.trim() == "（AI 未给出回复）")
        .unwrap_or(true);
    if reply_blank {
        if let Some(synth) = synthesize_natural_reply_from_patch(&out) {
            out["naturalReply"] = json!(synth);
        }
    }
    Ok(out)
}

fn render_chat_history_for_prompt(history: &[KnowledgeChatTurn]) -> String {
    if history.is_empty() {
        return "（暂无历史）".to_string();
    }
    let mut s = String::new();
    for t in history.iter().rev().take(6).collect::<Vec<_>>().iter().rev() {
        s.push_str(&format!(
            "- [{}] {}: {}\n",
            t.turn_index,
            t.role,
            truncate_for_prompt(&t.content, 200)
        ));
    }
    s
}

/// knowledge-digest-workstation Phase 5：把 KnowledgeOperatorMemory 渲染成
/// system prompt header（≤ 5 条），帮 intent 分类与下游分支保持运营长期偏好。
/// 与 contacts.memory_card 物理隔离，prompt header 也分开命名为「运营长期偏好」。
fn render_operator_memory_for_prompt(
    memories: &[crate::models::KnowledgeOperatorMemory],
) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let mut s = String::from("【运营长期偏好（仅作上下文，不要写回 chunk patch）】\n");
    for m in memories.iter().take(5) {
        let kind_label = match m.kind.as_str() {
            "preference" => "偏好",
            "rejection" => "红线",
            "context" => "背景",
            other => other,
        };
        s.push_str(&format!(
            "- {kind_label}：{}\n",
            truncate_for_prompt(&m.content, 120)
        ));
    }
    s
}

// ===========================================================================
// 知识库 chat agent 的多轮工具循环（knowledge-digest-workstation Phase 5 / P5.2）
// ---------------------------------------------------------------------------
//
// 设计目标：让 chat 三大下游 prompt（draft_chunk / update_chunk / clarify）走真
// 正的 agent tool loop —— Reply Agent 可以多轮自主调用 knowledge.* 工具去观察
// 整个知识库（catalog / search / open_slice / audit_completeness / search_chunks /
// propose_repair / analyze_logs / open_document / inspect_pack / verify_anchor）
// 再决定最终输出。
//
// 强约束（与 user-ops tool_loop 保持同构）：
// - 单 turn ≤ CHAT_TOOL_LOOP_MAX_LOOPS=4 轮；
// - 单轮 toolCalls ≤ 6；
// - 单 dispatch 5s timeout；
// - 失败连击 ≥3 强制结束；
// - 总耗时 30s 硬超时；
// - tool_call_budget 超额按 budget_exceeded 强制结束；
// - 永不写库、永不进 outbox、永不进 mcp（与 user-ops gateway 物理隔离）；
// - AI 永不自动 verify：chat 落库由 chat_apply 强制 status=draft + needs_review。
// ===========================================================================

/// 把基础 system prompt 增广上 tool-calling 协议头：
/// - 解释 decisionPhase 取值（tool_calling / final）；
/// - 列出可用 tool 白名单；
/// - 限制 toolCalls 数量与 final 字段约束。
///
/// 注意：本函数只追加协议提示，不删除/改写原 prompt 内容。
fn augment_chat_system_with_tools(base: &str) -> String {
    let tool_list = agent::ALLOWED_CHAT_TOOL_NAMES.join(" / ");
    format!(
        r#"{base}

【tool-calling 协议（chat agent 必须遵守）】
- 输出 JSON 必须包含 `decisionPhase`，取值仅限 `tool_calling` / `final`。
- 当你需要观察知识库当前状态时，输出 `decisionPhase=tool_calling` + `toolCalls` 数组（≤ 6 个），可用工具：
  {tool_list}
  工具的入参字段名遵循 camelCase（如 chunkId / documentId / itemId / sourceQuote / topK / onlyVerified / hours）。
- `tool_calling` 中间轮 **不要** 输出 `naturalReply / patch / missingFields / followupQuestions`；这些字段只在 `final` 轮给。
- 当不再需要更多工具结果、可以给运营回复时，输出 `decisionPhase=final` + 业务字段（naturalReply / patch? / missingFields? / followupQuestions?）；不要再带 toolCalls。
- 单 turn 最多 4 轮工具循环、6 次 LLM call；超过会被 budget 截断。
- 每轮工具结果会以 `[system tool result]` 段附加到 user prompt 末尾，下一轮直接读。
- 不要伪造工具结果；只能使用实际返回的内容。
"#
    )
}

/// 单次 chat tool-calling 循环的入口。
///
/// 行为：
/// 1. 拉取本 workspace 的 [`agent::types::KnowledgeRuntime`] 快照（document/item/chunk）；
/// 2. 用当前 [`agent::RUN_BUDGET`] 当作循环 budget；
/// 3. 构造 reply_fn 闭包：调 `agent::generate_agent_json`（注入累计的
///    `[system tool result]`）→ 用 `RawAgentDecision::validate_and_promote` 反序列化；
/// 4. 调 [`agent::chat_reply_with_tools_loop`]；
/// 5. 在 final 轮把最近一次 LLM 原始 JSON（含 patch / missingFields / followupQuestions /
///    naturalReply 等业务字段）返回给 caller。
///
/// 返回的 Value 形态与原先直接 `generate_agent_json` 输出一致，下游
/// `run_chat_turn_pipeline` / `chat_turn` handler 不需要任何改造。
async fn run_chat_with_tools(
    state: &AppState,
    workspace_id_in: &str,
    account_id: &str,
    session_id: &str,
    run_key: &str,
    prompt_key: &str,
    system: String,
    user: String,
) -> AppResult<Value> {
    use std::pin::Pin;
    use std::sync::Mutex as StdMutex;

    use agent::types::{KnowledgeRuntime, RawAgentDecision};
    use agent::{
        chat_reply_with_tools_loop, ChatReplyFn, ChatToolLoopError, RunBudget,
        UserRuntimeParameters,
    };

    // 拉 KnowledgeRuntime 快照：documents / items / verified chunks。
    // 与 user-ops `load_operation_knowledge` 的形态对齐，但简化为按 workspace
    // 全量取（chat 不绑定到具体 contact，没有 account_filter）。limit 与 user-ops
    // 一致，避免 KnowledgeRuntime 跨 chunk 数量发散。
    let workspace_id = workspace_id_in.to_string();
    let documents: Vec<OperationKnowledgeDocument> = state
        .db
        .operation_knowledge_documents()
        .find(
            doc! { "workspace_id": &workspace_id, "domain": "user_operations", "status": "active" },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1_i32 })
                .limit(80)
                .build(),
        )
        .await?
        .try_collect()
        .await?;
    let chunks: Vec<OperationKnowledgeChunk> = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &workspace_id,
                "domain": "user_operations",
                "status": "active",
                "integrity_status": "verified",
            },
            FindOptions::builder()
                .sort(doc! { "priority": -1_i32, "updated_at": -1_i32 })
                .limit(200)
                .build(),
        )
        .await?
        .try_collect()
        .await?;
    let knowledge = KnowledgeRuntime {
        documents,
        chunks,
    };
    let runtime = UserRuntimeParameters::default();

    // 取当前 RUN_BUDGET（chat_turn handler 已经 scope 进来了）；
    // 若拿不到——属于不应发生的情况——回退到一个本地 budget（让 loop 仍能跑）。
    let budget = agent::current_run_budget().unwrap_or_else(|| {
        Arc::new(RunBudget::new(
            format!("chat-fallback-{session_id}-{run_key}"),
            CHAT_TOKEN_BUDGET_PER_TURN,
            CHAT_MAX_LLM_CALLS_PER_TURN,
            i32::MAX,
        ))
    });

    // 用 Arc<StdMutex<Option<Value>>> 把每轮 LLM 原始 JSON 透传出来。chat
    // 路径在 `final` 轮需要 patch / missingFields / followupQuestions /
    // naturalReply 等字段，AgentDecision 不直接覆盖这些；最简单是把原始
    // Value 暂存，在循环结束后取出。
    let last_raw: Arc<StdMutex<Option<Value>>> = Arc::new(StdMutex::new(None));

    // reply_fn 闭包：每轮被 chat_reply_with_tools_loop 调用。
    let state_arc = Arc::new(state.clone());
    let account_id_owned = account_id.to_string();
    let session_id_owned = session_id.to_string();
    let run_key_owned = run_key.to_string();
    let prompt_key_owned = prompt_key.to_string();
    let system_owned = system;
    let user_owned = user;
    let last_raw_for_fn = Arc::clone(&last_raw);
    let runtime_for_fn = runtime.clone();

    let reply_fn: ChatReplyFn<'_> = Box::new(move |tool_results: &str, loop_count: i32| {
        let state_arc = Arc::clone(&state_arc);
        let account_id_owned = account_id_owned.clone();
        let session_id_owned = session_id_owned.clone();
        let run_key_owned = run_key_owned.clone();
        let prompt_key_owned = prompt_key_owned.clone();
        let system_owned = system_owned.clone();
        let user_owned = user_owned.clone();
        let tool_results_owned = tool_results.to_string();
        let last_raw = Arc::clone(&last_raw_for_fn);
        let runtime_for_fn = runtime_for_fn.clone();
        let fut: Pin<Box<dyn std::future::Future<Output = _> + Send>> = Box::pin(async move {
            // 把累计的 [system tool result] 注入 user prompt 末尾。
            let user_with_tools = if tool_results_owned.is_empty() {
                user_owned.clone()
            } else {
                format!("{user_owned}\n\n[system tool result]{tool_results_owned}")
            };
            let run_id = format!(
                "chat-{session_id_owned}-{run_key_owned}-loop-{loop_count}"
            );
            let value = agent::generate_agent_json(
                &state_arc,
                Some(&account_id_owned),
                None,
                Some(&run_id),
                &prompt_key_owned,
                &system_owned,
                &user_with_tools,
            )
            .await?;
            // 把原始 JSON 暂存：循环结束后从 last_raw 取出来当 final payload。
            if let Ok(mut guard) = last_raw.lock() {
                *guard = Some(value.clone());
            }
            // 反序列化为 RawAgentDecision，再 promote 到 AgentDecision。
            let raw: RawAgentDecision =
                serde_json::from_value(value).map_err(AppError::from)?;
            let (decision, promote_risks) = raw.validate_and_promote(&runtime_for_fn);
            Ok((decision, promote_risks))
        });
        fut
    });

    // 跑循环。任意 dispatch 错误以 Value 形态注入下一轮，循环只在 budget /
    // failure_streak / total_timeout 三种情况下提前结束。
    let outcome = chat_reply_with_tools_loop(
        &runtime,
        &knowledge,
        &state.db,
        &workspace_id,
        budget,
        Some(source_anchor_for_quote_ffi as agent::AnchorMatchFn),
        reply_fn,
    )
    .await;
    let final_value = match outcome {
        Ok(_outcome) => {
            // 取最后一轮 LLM 原始 JSON 作为 final payload。
            // 若 last_raw 为空（reply_fn 一次都没调用成功），用 empty object 兜底。
            last_raw
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| {
                    json!({
                        "decisionPhase": "final",
                        "naturalReply": "（AI 未给出回复）",
                    })
                })
        }
        Err(ChatToolLoopError::Timeout { elapsed_ms, .. }) => {
            // 超时——返回温和 final，让上层 handler 仍能写 turn 与 event。
            json!({
                "decisionPhase": "final",
                "naturalReply": format!("（AI 工具循环超时 elapsed_ms={elapsed_ms}，请稍后再试或换个说法）"),
            })
        }
        Err(ChatToolLoopError::Reply(err)) => return Err(err),
    };
    Ok(final_value)
}

/// `verify_anchor` 工具的 source_quote→anchor 模糊匹配实现适配器。
/// 把 `source_anchor_for_quote(raw_content, document_id, source_quote)` 中
/// 的 `Option<ObjectId>` 参数转为 `Option<String>`（hex），让其符合
/// [`agent::AnchorMatchFn`] 的纯函数签名（避免 knowledge_tools.rs 直接依赖
/// mongodb::bson::oid::ObjectId 与 routes 模块）。
fn source_anchor_for_quote_ffi(
    raw_content: &str,
    document_id_hex: Option<String>,
    source_quote: &str,
) -> Option<Document> {
    let oid = document_id_hex
        .as_deref()
        .and_then(|h| ObjectId::parse_str(h).ok());
    source_anchor_for_quote(raw_content, oid, source_quote)
}

/// knowledge-digest-workstation Phase 5：intent=update_operator_memory 分支。
///
/// 落库 KnowledgeOperatorMemory 一条；返回的 Value 满足 chat_turn handler 对
/// `naturalReply / missingFields / followupQuestions` 的约定，但不出 patch
/// （AI 偏好/红线不进 chunk）。
async fn update_operator_memory_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    operator_id: &str,
    user_content: &str,
    intent_result: &Value,
) -> AppResult<Value> {
    let kind = intent_result
        .get("memoryKind")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("preference");
    let content = intent_result
        .get("memoryContent")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| user_content.trim().to_string());
    if !["preference", "rejection", "context"].contains(&kind) {
        return Ok(json!({
            "naturalReply": "AI 没法判定您要立的是偏好还是红线，能再说得具体一点吗？",
            "missingFields": ["memoryKind"],
            "followupQuestions": [{
                "id": "q1",
                "field": "memoryKind",
                "question": "请明确：是偏好（preference）/ 红线（rejection）/ 背景（context）？",
            }],
        }));
    }
    let mem = agent::record_operator_memory(
        &state.db,
        workspace_id,
        account_id,
        operator_id,
        kind,
        &content,
    )
    .await?;
    let kind_label = match kind {
        "preference" => "偏好",
        "rejection" => "红线",
        "context" => "背景",
        other => other,
    };
    let summary = format!("已记下您的{kind_label}：{}", truncate_for_prompt(&content, 80));
    record_repair_event(
        state,
        workspace_id,
        account_id,
        "knowledge_operator_memory_added",
        summary.clone(),
        doc! {
            "kind": "operator_memory",
            "memoryKind": kind,
            "operatorId": operator_id,
            "memoryId": mem.id.map(|o| o.to_hex()).unwrap_or_default(),
        },
    )
    .await;
    Ok(json!({
        "naturalReply": format!("{summary}。AI 会在下次起草时遵守这条偏好；如需撤销请直接告诉我。"),
        "missingFields": Vec::<String>::new(),
        "followupQuestions": Vec::<Value>::new(),
        "operatorMemory": {
            "id": mem.id.map(|o| o.to_hex()),
            "kind": mem.kind,
            "content": mem.content,
        }
    }))
}

async fn classify_intent(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    chunk_attached: Option<&str>,
    item_attached: Option<&str>,
    history: &[KnowledgeChatTurn],
    operator_memory_header: &str,
) -> AppResult<Value> {
    let system_base = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.chat.intent",
    )
    .await
    .unwrap_or_else(|_| {
        "你是知识库对话 Agent，仅识别意图。只输出 JSON: {intent, confidence, targetChunkId?, targetPackId?, memoryKind?, memoryContent?, userIntentSummary}.".to_string()
    });
    let system = if operator_memory_header.is_empty() {
        system_base
    } else {
        format!("{system_base}\n\n{operator_memory_header}")
    };
    let user = format!(
        r#"运营本轮输入：
{user_content}

引用的 chunkId（可能为空）：{}
引用的 packId（可能为空）：{}

最近历史（最多 6 条）：
{}

请输出 JSON，intent 必须在 [create_chunk, update_chunk, clarify_chunk, update_pack, digest_action, update_operator_memory, freeform] 中。"#,
        chunk_attached.unwrap_or("(无)"),
        item_attached.unwrap_or("(无)"),
        render_chat_history_for_prompt(history),
    );
    let run_id = format!("chat-{session_id}-intent");
    agent::generate_agent_json(
        state,
        Some(account_id),
        None,
        Some(&run_id),
        "knowledge.chat.intent",
        &system,
        &user,
    )
    .await
}

async fn draft_chunk_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    target_pack_id: Option<&str>,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let system = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.chat.draft_chunk",
    )
    .await
    .unwrap_or_else(|_| {
        "你是知识库对话 Agent，起草新切片草稿。只输出 JSON: {patch, missingFields, followupQuestions, naturalReply}.".to_string()
    });
    // operation_knowledge_items 已删除；catalog/pack_payload 永远为空。
    let catalog: Vec<Value> = vec![];
    let _ = target_pack_id;
    let pack_payload = Value::Null;
    let user = format!(
        r#"运营本轮输入：
{user_content}

知识库已有 pack catalog（≤ 10）：
{}

运营引用的 pack（可能为空）：
{}

最近历史（最多 6 条）：
{}

起草要求：
- patch 必须把运营本轮明确点名要起草的字段全部填上——运营若说「起草标题、摘要和正文」，patch 就必须同时含非空的 title、summary、body 三者，缺任何一个都算答非所问。
- body（正文）是切片的实体内容，承载可验证事实，绝不能因为它最长就省略或留空；其余字段齐全而独缺 body 视为未完成起草。
- 信息确实不足以填某字段时，把该字段名写进 missingFields 并用 followupQuestions 向运营追问，而不是静默丢弃运营已点名的字段。
- naturalReply 必填、不可留空：用对话口吻向运营回报你起草了什么、还差什么，这是给人看的回执，不能只产 patch 就沉默。回执要展示关键产出本身（如把拟定的标题、摘要要点直接说出来），而不是只声明「我起草了标题/摘要」这类字段名——让运营不必去翻 patch 就能判断对不对；仍缺的字段则顺带引导补全。

请按 system 中 schema 输出 JSON 起草一条新切片草稿。"#,
        serde_json::to_string_pretty(&catalog).unwrap_or_default(),
        serde_json::to_string_pretty(&pack_payload).unwrap_or_default(),
        render_chat_history_for_prompt(history),
    );
    let augmented_system = augment_chat_system_with_tools(&system);
    run_chat_with_tools(
        state,
        workspace_id,
        account_id,
        session_id,
        "draft",
        "knowledge.chat.draft_chunk",
        augmented_system,
        user,
    )
    .await
}

async fn update_chunk_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    chunk_id: &str,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let oid = parse_object_id(chunk_id)?;
    let chunk = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": oid,
                "workspace_id": workspace_id,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("chunk {chunk_id} not found")))?;
    let document_payload = if let Some(document_id) = chunk.document_id {
        state
            .db
            .operation_knowledge_documents()
            .find_one(
                doc! {
                    "_id": document_id,
                    "workspace_id": workspace_id,
                },
                None,
            )
            .await?
            .map(|d| {
                json!({
                    "title": d.title,
                    "rawText": truncate_for_prompt(d.raw_content.as_deref().unwrap_or(""), 4000),
                })
            })
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let system = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.chat.update_chunk",
    )
    .await
    .unwrap_or_else(|_| {
        "你是知识库对话 Agent，按运营对话给出已选切片的修改 patch。只输出 JSON: {patch, missingFields, followupQuestions, naturalReply}.".to_string()
    });
    let user = format!(
        r#"运营本轮输入：
{user_content}

待修改切片当前内容：
{}

父文档（可能为空，已截断到 4000 字）：
{}

最近历史（最多 6 条）：
{}

请仅对运营提到的字段做改动；其它字段省略。"#,
        serde_json::to_string_pretty(&operation_knowledge_chunk_json(chunk.clone()))
            .unwrap_or_default(),
        serde_json::to_string_pretty(&document_payload).unwrap_or_default(),
        render_chat_history_for_prompt(history),
    );
    let augmented_system = augment_chat_system_with_tools(&system);
    run_chat_with_tools(
        state,
        workspace_id,
        account_id,
        session_id,
        "update",
        "knowledge.chat.update_chunk",
        augmented_system,
        user,
    )
    .await
}

async fn update_pack_for_chat(
    _state: &AppState,
    _workspace_id: &str,
    _account_id: &str,
    _session_id: &str,
    _user_content: &str,
    pack_id: &str,
    _history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    // operation_knowledge_items 已删除；pack-level chat 路径暂时下线。
    Err(AppError::BadRequest(format!(
        "operation_knowledge_items has been removed; pack {pack_id} chat update is disabled"
    )))
}

async fn clarify_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let system = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.chat.clarify",
    )
    .await
    .unwrap_or_else(|_| {
        "你是知识库对话 Agent，做澄清回答。只输出 JSON: {naturalReply, askMoreField?, askMoreQuestion?, nextSuggestion?}.".to_string()
    });
    let user = format!(
        r#"运营本轮输入：
{user_content}

最近历史（最多 6 条）：
{}

请按 system 中 schema 输出 JSON。"#,
        render_chat_history_for_prompt(history),
    );
    let augmented_system = augment_chat_system_with_tools(&system);
    run_chat_with_tools(
        state,
        workspace_id,
        account_id,
        session_id,
        "clarify",
        "knowledge.chat.clarify",
        augmented_system,
        user,
    )
    .await
}

/// knowledge-digest-workstation Phase 4 / Task #360：
/// 把运营从今日日报勾出的一组卡片转成 `plannedSteps` 序列。
///
/// 调 `knowledge.digest.dispatch` PromptSpec；输入是当日 cards 摘要 + 运营本轮文字；
/// 输出含 `plannedSteps[] / estimatedLlmCalls / naturalReply`，由前端拿到后弹「派工
/// 确认」小卡，确认后再 POST `/api/knowledge/chat/tasks` 落 `KnowledgeChatTask`。
///
/// 与 update_chunk_for_chat 不同：本路径不出 patch、不直接落库，仅是步骤计划。
async fn dispatch_digest_action_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let system = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.digest.dispatch",
    )
    .await
    .unwrap_or_else(|_| {
        "你是 AI 调度器，把运营勾的卡片拆成 plannedSteps。只输出 JSON: {plannedSteps, estimatedLlmCalls, naturalReply}.".to_string()
    });

    // 取今日日报里未 dismiss 的卡片摘要（≤ 20 条）作为参考
    // 卡片实际勾选由前端在 attachments 里传，但本轮 chat 不收 cardIds —— 让 LLM
    // 看到全量候选 + 运营自然语言去匹配（运营常说"把这 3 张 fix 了"）。
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let report = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?;
    let mut card_summaries: Vec<Value> = vec![];
    if let Some(r) = report {
        for c in r.cards.iter().take(20) {
            if r.dismissed_card_ids.contains(&c.card_id) {
                continue;
            }
            card_summaries.push(json!({
                "cardId": c.card_id.to_hex(),
                "kind": c.kind,
                "title": c.title,
                "summary": c.summary,
                "suggestedAction": c.suggested_action,
                "severity": c.severity,
            }));
        }
    }

    let user = format!(
        r#"运营本轮输入：
{user_content}

今日日报候选卡片（最多 20 条，未被 dismiss）：
{cards}

最近历史（最多 6 条）：
{history}

请按 system 中 schema 输出 plannedSteps（步数 ≤ 8、总 estimatedLlmCalls ≤ 12）。
每个 step 必须含 stepId / cardId / action / summary / estimatedLlmCalls。
action 必须在 [fix_chunk, add_chunk, retag, review_evolution, analyze_logs, dismiss] 中。"#,
        cards = serde_json::to_string_pretty(&card_summaries).unwrap_or_else(|_| "[]".to_string()),
        history = render_chat_history_for_prompt(history),
    );
    let run_id = format!("chat-{session_id}-dispatch");
    agent::generate_agent_json(
        state,
        Some(account_id),
        None,
        Some(&run_id),
        "knowledge.digest.dispatch",
        &system,
        &user,
    )
    .await
}

async fn apply_create_chunk(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    patch: &Document,
    target_pack_id: Option<&str>,
) -> AppResult<Value> {
    let patch_value: Value = mongodb::bson::Bson::Document(patch.clone()).into();
    let mut payload = chunk_request_from_chat_patch(&patch_value, account_id, target_pack_id);
    // 强制：AI 永不自动 verify
    payload.status = "draft".to_string();
    payload.integrity_status = Some("needs_review".to_string());
    payload.source_anchors = vec![]; // 让 backend 重算

    validate_operation_knowledge_chunk(&payload)?;
    let chunk = operation_knowledge_chunk_from_request(state, workspace_id, payload, None)?;
    let inserted = state
        .db
        .operation_knowledge_chunks()
        .insert_one(chunk, None)
        .await?;
    let new_id = inserted
        .inserted_id
        .as_object_id()
        .map(|o| o.to_hex())
        .unwrap_or_default();
    Ok(json!({
        "createdChunkId": new_id,
        "sessionId": session_id,
        "status": "draft",
        "integrityStatus": "needs_review",
    }))
}

async fn apply_update_chunk(
    state: &AppState,
    workspace_id: &str,
    _account_id: &str,
    chunk_id: &str,
    patch: &Document,
) -> AppResult<Value> {
    let oid = parse_object_id(chunk_id)?;
    let mut update_doc = Document::new();
    for key in [
        "title",
        "summary",
        "routing_card",
        "applicable_scenes",
        "not_applicable_scenes",
        "safe_claims",
        "forbidden_claims",
        "evidence_items",
        "product_tags",
        "business_topics",
        "source_quote",
    ]
    .iter()
    {
        // patch 用 camelCase；映射到 storage 的 snake_case。
        let camel = match *key {
            "routing_card" => "routingCard",
            "applicable_scenes" => "applicableScenes",
            "not_applicable_scenes" => "notApplicableScenes",
            "safe_claims" => "safeClaims",
            "forbidden_claims" => "forbiddenClaims",
            "evidence_items" => "evidenceItems",
            "product_tags" => "productTags",
            "business_topics" => "businessTopics",
            "source_quote" => "sourceQuote",
            other => other,
        };
        if let Some(val) = patch.get(camel) {
            update_doc.insert(*key, val.clone());
        }
    }
    if update_doc.is_empty() {
        return Ok(json!({
            "updatedChunkId": chunk_id,
            "fieldsTouched": 0,
            "note": "patch 没有可识别字段，未改动",
        }));
    }
    update_doc.insert("integrity_status", "needs_review");
    update_doc.insert("status", "draft");
    update_doc.insert("updated_at", DateTime::now());
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! {
                "_id": oid,
                "workspace_id": workspace_id,
            },
            doc! { "$set": update_doc.clone() },
            None,
        )
        .await?;
    Ok(json!({
        "updatedChunkId": chunk_id,
        "fieldsTouched": update_doc.len() - 3,
        "status": "draft",
        "integrityStatus": "needs_review",
    }))
}

async fn apply_update_pack(
    _state: &AppState,
    _account_id: &str,
    pack_id: &str,
    _patch: &Document,
) -> AppResult<Value> {
    // operation_knowledge_items 已删除；pack-level apply 路径暂时下线。
    Err(AppError::BadRequest(format!(
        "operation_knowledge_items has been removed; pack {pack_id} update is disabled"
    )))
}

/// 把 chat 产出的 patch（camelCase JSON）转成 OperationKnowledgeChunkRequest。
/// 缺字段补默认值；让后端的 apply_chunk_integrity 在写入路径上重算 anchor。
fn chunk_request_from_chat_patch(
    patch: &Value,
    account_id: &str,
    pack_id: Option<&str>,
) -> OperationKnowledgeChunkRequest {
    fn s(v: &Value, k: &str) -> Option<String> {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    fn arr(v: &Value, k: &str) -> Vec<String> {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
    OperationKnowledgeChunkRequest {
        account_id: Some(account_id.to_string()),
        document_id: None,
        item_id: pack_id.map(|s| s.to_string()),
        domain: "user_operations".to_string(),
        knowledge_type: s(patch, "knowledgeType"),
        business_context: s(patch, "businessContext"),
        title: s(patch, "title").unwrap_or_else(|| "AI 对话产物（草稿）".to_string()),
        summary: s(patch, "summary"),
        body: s(patch, "body"),
        routing_card: s(patch, "routingCard"),
        applicable_scenes: arr(patch, "applicableScenes"),
        not_applicable_scenes: arr(patch, "notApplicableScenes"),
        safe_claims: arr(patch, "safeClaims"),
        forbidden_claims: arr(patch, "forbiddenClaims"),
        evidence_items: arr(patch, "evidenceItems"),
        product_tags: arr(patch, "productTags"),
        business_topics: arr(patch, "businessTopics"),
        source_quote: s(patch, "sourceQuote"),
        source_anchors: vec![],
        integrity_status: Some("needs_review".to_string()),
        confidence_score: None,
        distortion_risks: vec![],
        unsupported_claims: vec![],
        verified_claims: vec![],
        status: "draft".to_string(),
        priority: 0,
    }
}

// ── knowledge-digest-workstation Phase 1：日报路由（最小骨架） ──────────────
//
// `GET /api/knowledge/digest/today`：查询当日 `knowledge_daily_reports`，命中
// 即返回；未命中**直接 404**，**不**触发同步合成（Phase 2 才接 generate）。
// 设计见 `.kiro/specs/knowledge-digest-workstation/design.md` §6 Routes 与
// `docs/data-and-api.md` 知识库日报工作站章节。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DigestTodayQuery {
    pub account_id: Option<String>,
    /// `YYYY-MM-DD`；缺省时用运营时区今天。
    pub report_date: Option<String>,
}

pub(super) async fn digest_today(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<DigestTodayQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let report_date = query
        .report_date
        .clone()
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());

    let found = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?;

    let report = match found {
        Some(r) => r,
        None => {
            // Phase 2：未命中时**同步合成**今日日报；失败则按 503 / 404 上抛。
            // 避免运营反复刷新 → 命中 worker 还没醒的窗口期。
            crate::knowledge_digest::generate_today_digest(&state).await?
        }
    };

    Ok(serialize_digest_report(&report))
}

/// `POST /api/knowledge/digest/regenerate`：强制重算今日日报。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DigestRegenerateRequest {
    pub account_id: Option<String>,
    #[serde(default)]
    pub force: bool,
}

pub(super) async fn digest_regenerate(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<DigestRegenerateRequest>,
) -> AppResult<Json<Value>> {
    let account_id = body
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();

    if !body.force {
        // 非强制路径：若今日日报已存在，直接返回，不重复调 LLM。
        if let Some(existing) = state
            .db
            .knowledge_daily_reports()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &account_id,
                    "report_date": &report_date,
                },
                None,
            )
            .await?
        {
            return Ok(serialize_digest_report(&existing));
        }
    }
    let report = crate::knowledge_digest::generate_today_digest(&state).await?;
    Ok(serialize_digest_report(&report))
}

/// `POST /api/knowledge/digest/cards/:id/dismiss`：把卡片标记为已忽略，画布灰显。
pub(super) async fn digest_dismiss_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(card_id_hex): Path<String>,
) -> AppResult<Json<Value>> {
    let card_id = ObjectId::parse_str(&card_id_hex)
        .map_err(|_| AppError::BadRequest(format!("invalid card_id: {card_id_hex}")))?;
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let result = state
        .db
        .knowledge_daily_reports()
        .update_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "report_date": &report_date,
                "cards.cardId": &card_id,
            },
            doc! {
                "$addToSet": { "dismissed_card_ids": &card_id }
            },
            None,
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(format!(
            "未找到包含 cardId={} 的今日日报",
            card_id_hex
        )));
    }
    Ok(Json(json!({
        "ok": true,
        "cardId": card_id_hex,
        "reportDate": report_date,
    })))
}

fn serialize_digest_report(report: &crate::models::KnowledgeDailyReport) -> Json<Value> {
    Json(json!({
        "reportId": report.id.map(|id| id.to_hex()),
        "workspaceId": report.workspace_id,
        "accountId": report.account_id,
        "reportDate": report.report_date,
        "generatedAt": report.generated_at.to_string(),
        "generatedBy": report.generated_by,
        "status": report.status,
        "errorKind": report.error_kind,
        "budgetSnapshot": serde_json::to_value(&report.budget_snapshot).unwrap_or(json!({})),
        "cards": serde_json::to_value(&report.cards).unwrap_or(json!([])),
        "dismissedCardIds": report
            .dismissed_card_ids
            .iter()
            .map(|id| id.to_hex())
            .collect::<Vec<_>>(),
        "promptVersions": serde_json::to_value(&report.prompt_versions).unwrap_or(json!({})),
    }))
}

// ── knowledge-digest-workstation Phase 4：chat 长任务 + SSE ──────────────────

/// `POST /api/knowledge/chat/tasks`：把 chat dispatch 出的 plannedSteps 落库为
/// `knowledge_chat_tasks{status="pending"}`，由 `KnowledgeTaskWorker` 串行执行。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatTaskCreateRequest {
    pub session_id: String,
    pub account_id: Option<String>,
    pub operator_id: Option<String>,
    #[serde(default)]
    pub card_ids: Vec<String>,
    #[serde(default)]
    pub planned_steps: Vec<Value>,
}

pub(super) async fn chat_task_create(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<ChatTaskCreateRequest>,
) -> AppResult<Json<Value>> {
    let session_id = body.session_id.trim();
    if session_id.is_empty() {
        return Err(AppError::BadRequest("sessionId 不能为空".to_string()));
    }
    if body.planned_steps.is_empty() {
        return Err(AppError::BadRequest(
            "plannedSteps 不能为空，请先经 chat dispatch 拿到步骤计划".to_string(),
        ));
    }
    if body.planned_steps.len() > 8 {
        return Err(AppError::BadRequest(
            "plannedSteps 步数超过 8 条，请由前端分批派工".to_string(),
        ));
    }
    let account_id = body
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());

    // 把 plannedSteps 序列化成 BSON Document 数组（每条至少含 stepId/cardId/action）。
    // P1-4：action 闭集校验——只接受 worker `execute_step` 已实装的 6 种 action；
    // 越界（如 LLM 幻觉出 `delete_chunk`）必须在入库前 400 拦掉，不能依赖 worker
    // 的 fail-soft match-arm 兜底（fail-soft 会污染 completed_steps + summary 计数）。
    // 该名单与 `parse_cards_from_llm_array` 的 allowed_actions 保持一致。
    const ALLOWED_TASK_ACTIONS: &[&str] = &[
        "fix_chunk",
        "add_chunk",
        "retag",
        "review_evolution",
        "analyze_logs",
        "dismiss",
    ];
    let mut steps_doc: Vec<Document> = Vec::with_capacity(body.planned_steps.len());
    for (idx, step) in body.planned_steps.iter().enumerate() {
        let mut d = bson_from_json(step)
            .map_err(|e| AppError::BadRequest(format!("plannedSteps[{idx}] 非法 JSON: {e}")))?;
        if d.get_str("stepId").is_err() {
            d.insert("stepId", format!("step_{}", idx + 1));
        }
        let action = d.get_str("action").map_err(|_| {
            AppError::BadRequest(format!("plannedSteps[{idx}].action 缺失"))
        })?;
        if !ALLOWED_TASK_ACTIONS.contains(&action) {
            return Err(AppError::BadRequest(format!(
                "plannedSteps[{idx}].action='{action}' 不在允许集合内：{:?}",
                ALLOWED_TASK_ACTIONS
            )));
        }
        steps_doc.push(d);
    }

    // cards 快照：从今日日报里反查（best-effort，缺失也允许落 task）。
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let report = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?;
    let mut card_snapshots: Vec<crate::models::KnowledgeDigestCard> = vec![];
    if let Some(r) = report {
        for cid_hex in &body.card_ids {
            if let Ok(oid) = ObjectId::parse_str(cid_hex) {
                if let Some(c) = r.cards.iter().find(|c| c.card_id == oid) {
                    card_snapshots.push(c.clone());
                }
            }
        }
    }

    let task_id = ObjectId::new();
    let task = crate::models::KnowledgeChatTask {
        id: Some(task_id),
        workspace_id: admin.current_workspace.clone(),
        account_id: account_id.clone(),
        session_id: session_id.to_string(),
        operator_id: body.operator_id.clone(),
        cards: card_snapshots,
        planned_steps: steps_doc,
        completed_steps: vec![],
        status: "pending".to_string(),
        error_kind: None,
        created_at: DateTime::now(),
        started_at: None,
        finished_at: None,
    };
    state
        .db
        .knowledge_chat_tasks()
        .insert_one(task, None)
        .await?;

    // 立刻写一条 task_progress turn 记录派工已落库。
    // P1-7：原子分配新 turn_index，避免与并发 chat_turn / worker 写入冲突。
    let next_index = allocate_next_turn_indices(&state, &admin.current_workspace, session_id, 1).await?;
    let turn = KnowledgeChatTurn {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: account_id.clone(),
        session_id: session_id.to_string(),
        turn_index: next_index,
        role: "system".to_string(),
        intent: Some("digest_action".to_string()),
        content: format!(
            "AI 已收到派工，taskId={}，共 {} 步，等待 worker 串行执行",
            task_id,
            body.planned_steps.len()
        ),
        attachments: vec![doc! { "taskId": task_id, "phase": "queued" }],
        patch: None,
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
        tokens_used: 0,
        prompt_key: None,
        kind: Some("task_progress".to_string()),
        tool_calls: vec![],
        created_at: DateTime::now(),
    };
    state
        .db
        .knowledge_chat_turns()
        .insert_one(turn, None)
        .await?;
    state.chat_progress_bus.bump(session_id).await;

    Ok(Json(json!({
        "taskId": task_id.to_hex(),
        "sessionId": session_id,
        "status": "pending",
        "totalSteps": body.planned_steps.len() as i32,
    })))
}

/// `GET /api/knowledge/chat/tasks/:id`：查询 task 状态（前端 fallback 拉取）。
pub(super) async fn chat_task_get(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id_hex): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id_hex)
        .map_err(|_| AppError::BadRequest(format!("invalid task id: {id_hex}")))?;
    let task = state
        .db
        .knowledge_chat_tasks()
        .find_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("knowledge_chat_task {id_hex} 不存在")))?;
    Ok(Json(json!({
        "taskId": task.id.map(|i| i.to_hex()).unwrap_or_default(),
        "sessionId": task.session_id,
        "status": task.status,
        "errorKind": task.error_kind,
        "totalSteps": task.planned_steps.len() as i32,
        "completedSteps": serde_json::to_value(&task.completed_steps).unwrap_or(json!([])),
        "plannedSteps": serde_json::to_value(&task.planned_steps).unwrap_or(json!([])),
        "cards": serde_json::to_value(&task.cards).unwrap_or(json!([])),
        "createdAt": task.created_at.to_string(),
        "startedAt": task.started_at.map(|d| d.to_string()),
        "finishedAt": task.finished_at.map(|d| d.to_string()),
    })))
}

/// `POST /api/knowledge/chat/tasks/:id/cancel`：标 status="cancelled"；
/// worker 在每步开始前 re-read 状态，非 "running" 即停下。
///
/// P2-10：终态幂等——如果 task 已经是 completed / failed / cancelled，本接口
/// 返回 200 `{ ok: true, alreadyTerminated: true }` 而不是 404。理由：前端
/// 有可能在 task 刚 complete 的瞬间 race 一次 cancel，对运营来说"终态"是同一
/// 类语义；只有真正不存在的 task 才返回 404。
pub(super) async fn chat_task_cancel(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id_hex): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id_hex)
        .map_err(|_| AppError::BadRequest(format!("invalid task id: {id_hex}")))?;
    let res = state
        .db
        .knowledge_chat_tasks()
        .update_one(
            doc! {
                "_id": oid,
                "workspace_id": &admin.current_workspace,
                "status": doc! { "$in": ["pending", "running"] }
            },
            doc! { "$set": { "status": "cancelled", "finished_at": DateTime::now() } },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        // 未命中可能有两种：(a) task 真不存在；(b) task 已是终态。区分两种是
        // 因为运营前端在 cancel 后会 GET /tasks/:id 拿最终态——对终态返 404
        // 会让运营误以为派工记录丢失。
        let existing = state
            .db
            .knowledge_chat_tasks()
            .find_one(
                doc! { "_id": oid, "workspace_id": &admin.current_workspace },
                None,
            )
            .await?;
        match existing {
            None => {
                return Err(AppError::NotFound(format!(
                    "knowledge_chat_task {id_hex} 不存在"
                )));
            }
            Some(t) => {
                return Ok(Json(json!({
                    "ok": true,
                    "taskId": id_hex,
                    "status": t.status,
                    "alreadyTerminated": true,
                })));
            }
        }
    }
    Ok(Json(json!({ "ok": true, "taskId": id_hex, "status": "cancelled" })))
}

/// `GET /api/knowledge/chat/sessions/:sid/stream`：SSE 推送最新 turn_index。
/// 客户端按收到的 version 回拉 `chat_history` 拿增量 turn。
///
/// P1-6：watch 值为 [`crate::knowledge_task::CLOSE_SENTINEL`] 时，发一个
/// `close` event 后立即结束流（`return None`）。前端 EventSource 收到 close
/// 事件应主动关闭 + 不再重连，避免占用连接。
pub(super) async fn chat_session_stream(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> axum::response::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use crate::knowledge_task::CLOSE_SENTINEL;
    let rx = state.chat_progress_bus.subscribe(&session_id).await;
    // 用 futures::stream::unfold 把 watch::Receiver 转成 SSE Stream，
    // 避免引入 tokio-stream 新依赖。state 是 (Receiver, closed) 元组——一旦
    // 推过 close event 就把 closed=true，下一次 poll 时直接 return None。
    let stream = futures::stream::unfold((rx, false), |(mut rx, closed)| async move {
        if closed {
            return None;
        }
        if rx.changed().await.is_err() {
            return None;
        }
        let v = *rx.borrow_and_update();
        if v == CLOSE_SENTINEL {
            // 终态：发一条 close 事件后下次循环立即 None。
            let event = Event::default().event("close").data("done");
            return Some((Ok::<_, std::convert::Infallible>(event), (rx, true)));
        }
        let event = Event::default().event("turn").data(v.to_string());
        Some((Ok::<_, std::convert::Infallible>(event), (rx, false)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// 把 serde_json::Value 转成 BSON Document（仅接受 object）。
fn bson_from_json(value: &Value) -> Result<Document, String> {
    if !value.is_object() {
        return Err("expected JSON object".to_string());
    }
    mongodb::bson::to_document(value).map_err(|e| e.to_string())
}

// ── AI Inbox 聚合（GET /operation-knowledge/inbox） ────────────────────────
//
// 知识库 AI 协作工作站顶层的待办流。把四类只读信号聚合成统一形态：
//   1. digest_card    —— 当日 KnowledgeDailyReport.cards（未 dismiss）
//   2. quote_missing  —— operation_knowledge_chunks 缺 source_quote
//   3. anchors_missing —— operation_knowledge_chunks 缺 source_anchors
//   4. pending_review —— integrity_status == "needs_review"
//
// 全部 read-only，**不写库**、不动 schema、不新增 collection。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InboxQuery {
    pub account_id: Option<String>,
    pub priority: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InboxCardView {
    pub id: String,
    pub priority: String,
    pub kind: String,
    pub title: String,
    pub context_summary: String,
    pub target_chunk_id: Option<String>,
    pub target_pack_id: Option<String>,
    pub suggested_actions: Vec<String>,
    pub origin: String,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InboxStats {
    pub total: usize,
    pub high: usize,
    pub mid: usize,
    pub low: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InboxResponse {
    pub items: Vec<InboxCardView>,
    pub stats: InboxStats,
}

/// digest 卡片 severity → inbox priority。
fn severity_to_priority(severity: &str) -> &'static str {
    match severity {
        "critical" => "high",
        "warn" => "mid",
        _ => "low",
    }
}

/// digest 卡片 suggested_action → inbox suggested actions。
fn digest_action_to_actions(action: &str) -> Vec<String> {
    match action {
        "fix_chunk" | "add_chunk" | "retag" => {
            vec!["open_chat".into(), "dismiss".into()]
        }
        "review_evolution" => vec!["open_chat".into(), "dismiss".into()],
        "dismiss" => vec!["dismiss".into()],
        _ => vec!["open_chat".into(), "dismiss".into()],
    }
}

/// digest 卡片 kind → inbox kind。
fn digest_kind_to_inbox_kind(kind: &str) -> &'static str {
    match kind {
        "chunk_missing_field" => "fill_field",
        "chunk_low_hit_rate" => "repair_chunk",
        "chunk_caused_block" => "repair_chunk",
        "pack_outdated" => "repair_chunk",
        "evolution_pending" => "repair_chunk",
        "evolution_released" => "repair_chunk",
        _ => "repair_chunk",
    }
}

/// 比较两条 inbox 条目，priority 高的在前。
fn priority_rank(p: &str) -> u8 {
    match p {
        "high" => 3,
        "mid" => 2,
        "low" => 1,
        _ => 0,
    }
}

/// pending_review chunk 在 inbox 里的优先级。
///
/// `chunk_type=negative_example` 是 reviewer 误判反馈链路（reaction 写入 outbox
/// 失败文本 → enqueue_negative_example_chunk）的 admin 二次确认入口，必须高优；
/// 其它类型 (peer_case / product_fact / style_template) 维持 mid，避免淹没。
fn inbox_pending_review_priority(chunk_type: &str) -> &'static str {
    if chunk_type == "negative_example" {
        "high"
    } else {
        "mid"
    }
}

pub(super) async fn knowledge_inbox(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<InboxQuery>,
) -> AppResult<Json<InboxResponse>> {
    let account_id = query
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let limit_cap = query.limit.unwrap_or(24).clamp(1, 100) as usize;
    let priority_filter = query.priority.as_deref();

    let mut items: Vec<InboxCardView> = Vec::new();

    // 1) digest_card: 当日 KnowledgeDailyReport.cards 未 dismiss。
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    if let Some(report) = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?
    {
        let dismissed: std::collections::HashSet<String> = report
            .dismissed_card_ids
            .iter()
            .map(|oid| oid.to_hex())
            .collect();
        for card in &report.cards {
            let card_id_hex = card.card_id.to_hex();
            if dismissed.contains(&card_id_hex) {
                continue;
            }
            // 提取 target chunk / pack id（如果 target_refs 里有）。
            let mut target_chunk: Option<String> = None;
            let mut target_pack: Option<String> = None;
            for r in &card.target_refs {
                let kind = r.get_str("kind").unwrap_or("");
                let id = r.get_str("id").unwrap_or("");
                if id.is_empty() {
                    continue;
                }
                match kind {
                    "chunk" => {
                        if target_chunk.is_none() {
                            target_chunk = Some(id.to_string());
                        }
                    }
                    "pack" | "item" => {
                        if target_pack.is_none() {
                            target_pack = Some(id.to_string());
                        }
                    }
                    _ => {}
                }
            }
            items.push(InboxCardView {
                id: format!("digest:{}", card_id_hex),
                priority: severity_to_priority(&card.severity).to_string(),
                kind: digest_kind_to_inbox_kind(&card.kind).to_string(),
                title: card.title.clone(),
                context_summary: card.summary.clone(),
                target_chunk_id: target_chunk,
                target_pack_id: target_pack,
                suggested_actions: digest_action_to_actions(&card.suggested_action),
                origin: "digest_card".into(),
                created_at: crate::models::dt_to_string(report.generated_at).unwrap_or_default(),
            });
        }
    }

    // 2/3/4) 三类来源都从 operation_knowledge_chunks 拉。统一拉一次，逐条分类。
    let chunks_filter = doc! {
        "workspace_id": &admin.current_workspace,
        "$or": [
            { "account_id": null },
            { "account_id": { "$exists": false } },
            { "account_id": &account_id },
        ],
        "status": { "$in": ["active", "draft"] },
    };
    let chunks_cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            chunks_filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(200_i64)
                .build(),
        )
        .await?;
    let chunks: Vec<OperationKnowledgeChunk> = chunks_cursor.try_collect().await?;

    let cutoff_ms = (chrono::Utc::now()
        - chrono::Duration::days(7))
    .timestamp_millis();

    for c in &chunks {
        let chunk_id_hex = match &c.id {
            Some(oid) => oid.to_hex(),
            None => continue,
        };
        let title = if c.title.trim().is_empty() {
            chunk_id_hex.clone()
        } else {
            c.title.clone()
        };
        let quote = c.source_quote.clone().unwrap_or_default();
        let has_quote = !quote.trim().is_empty();
        let has_anchor = !c.source_anchors.is_empty();
        let integrity = c.integrity_status.clone().unwrap_or_default();
        let updated_ms = c.updated_at.timestamp_millis();

        // 4) pending_review：integrity_status = needs_review 且 7d 内更新。
        // chunk_type=negative_example 升 priority=high 并标 origin=negative_example_review，
        // 因为这是 reviewer 误判反馈链路（reaction → enqueue_negative_example_chunk）的
        // admin 必须二次确认入口；其它类型 (peer_case / product_fact / style_template)
        // 维持 mid + pending_review。
        if integrity == "needs_review" && updated_ms >= cutoff_ms {
            let is_negative_example = c.chunk_type == "negative_example";
            items.push(InboxCardView {
                id: format!("chunk:{}:review", chunk_id_hex),
                priority: inbox_pending_review_priority(&c.chunk_type).into(),
                kind: "repair_chunk".into(),
                title: if is_negative_example {
                    format!("待审反例：{}", title)
                } else {
                    format!("待审切片：{}", title)
                },
                context_summary: c
                    .summary
                    .clone()
                    .unwrap_or_else(|| {
                        if is_negative_example {
                            "AI 从 reviewer 误判信号入队，等运营 admin 二次确认。".into()
                        } else {
                            "AI 起草，等运营确认。".into()
                        }
                    }),
                target_chunk_id: Some(chunk_id_hex.clone()),
                target_pack_id: None,
                suggested_actions: vec!["open_chat".into(), "open_repair".into(), "dismiss".into()],
                origin: if is_negative_example {
                    "negative_example_review".into()
                } else {
                    "pending_review".into()
                },
                created_at: crate::models::dt_to_string(c.updated_at).unwrap_or_default(),
            });
        }

        // 2) quote_missing：active 且无 source_quote。
        if c.status == "active" && !has_quote {
            items.push(InboxCardView {
                id: format!("chunk:{}:quote", chunk_id_hex),
                priority: "high".into(),
                kind: "fill_field".into(),
                title: format!("补原文出处：{}", title),
                context_summary: "AI 检测到该切片缺 sourceQuote，无法通过验证。".into(),
                target_chunk_id: Some(chunk_id_hex.clone()),
                target_pack_id: None,
                suggested_actions: vec!["open_chat".into(), "open_repair".into()],
                origin: "quote_missing".into(),
                created_at: crate::models::dt_to_string(c.updated_at).unwrap_or_default(),
            });
        }

        // 3) anchors_missing：active 且无 source_anchors（即便有 quote 也算）。
        if c.status == "active" && !has_anchor {
            items.push(InboxCardView {
                id: format!("chunk:{}:anchor", chunk_id_hex),
                priority: "high".into(),
                kind: "repair_chunk".into(),
                title: format!("修复原文锚点：{}", title),
                context_summary: "AI 检测到该切片 sourceAnchors 为空，需要重新锚定。".into(),
                target_chunk_id: Some(chunk_id_hex.clone()),
                target_pack_id: None,
                suggested_actions: vec!["open_chat".into(), "open_repair".into()],
                origin: "anchors_missing".into(),
                created_at: crate::models::dt_to_string(c.updated_at).unwrap_or_default(),
            });
        }
    }

    // 优先级过滤。
    if let Some(p) = priority_filter {
        items.retain(|it| it.priority == p);
    }

    // 排序：priority 降序，再按 origin 顺序保留稳定。
    items.sort_by(|a, b| priority_rank(&b.priority).cmp(&priority_rank(&a.priority)));

    // 截断到 limit。
    if items.len() > limit_cap {
        items.truncate(limit_cap);
    }

    let high = items.iter().filter(|c| c.priority == "high").count();
    let mid = items.iter().filter(|c| c.priority == "mid").count();
    let low = items.iter().filter(|c| c.priority == "low").count();
    let stats = InboxStats {
        total: items.len(),
        high,
        mid,
        low,
    };

    Ok(Json(InboxResponse { items, stats }))
}

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
pub(super) struct ChunkPatchRequest {
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
pub(super) struct ChunkArchiveRequest {
    pub reason: Option<String>,
    pub actor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChunkRollbackRequest {
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

/// 把 `block_parser::ParseWarning` 序列化为 import_apply 返回体里的统一形态。
fn parse_warning_to_json(w: &crate::knowledge_wiki::block_parser::ParseWarning) -> Value {
    use crate::knowledge_wiki::block_parser::ParseWarning::*;
    match w {
        UnsafeBlockId { id } => json!({"kind": "unsafeBlockId", "id": id}),
        UnterminatedFence { id } => json!({"kind": "unterminatedFence", "id": id}),
        DuplicateBlockId { id, occurrences } => {
            json!({"kind": "duplicateBlockId", "id": id, "occurrences": occurrences})
        }
        InvalidJson { id, reason } => {
            json!({"kind": "invalidJson", "id": id, "reason": reason})
        }
        StrayText { excerpt } => json!({"kind": "strayText", "excerpt": excerpt}),
    }
}

/// `POST /operation-knowledge/chunks/:id/patch` — 字段级 patch。
pub(super) async fn patch_operation_knowledge_chunk(
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
    super::chunk_locks::broadcast_chunk_revised_in(
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
pub(super) async fn archive_operation_knowledge_chunk(
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
    super::chunk_locks::broadcast_chunk_revised_in(
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
pub(super) async fn restore_operation_knowledge_chunk(
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
    super::chunk_locks::broadcast_chunk_revised_in(
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
pub(super) async fn rollback_operation_knowledge_chunk(
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
    super::chunk_locks::broadcast_chunk_revised_in(
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
pub(super) struct ChunkRevisionsQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// `GET /operation-knowledge/chunks/:id/revisions` — 分页拉取编辑历史。
///
/// 长字段（patch 内的 body / answer 等）在响应里保留原文；前端长 body 自行 mask。
pub(super) async fn list_operation_knowledge_chunk_revisions(
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
pub(super) struct ChunkSplitRequest {
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
pub(super) async fn split_operation_knowledge_chunk(
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
    super::chunk_locks::broadcast_chunk_revised_in(
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
pub(super) struct ChunkMergeRequest {
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
pub(super) async fn merge_operation_knowledge_chunk(
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
            super::chunk_locks::broadcast_chunk_revised_in(
                &state,
                workspace_id,
                &arch.chunk_id,
                "merge",
                &admin.username,
            );
            super::chunk_locks::broadcast_chunk_revised_in(
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
            super::chunk_locks::broadcast_chunk_revised_in(
                &state,
                workspace_id,
                &arch_a.chunk_id,
                "merge",
                &admin.username,
            );
            super::chunk_locks::broadcast_chunk_revised_in(
                &state,
                workspace_id,
                &arch_b.chunk_id,
                "merge",
                &admin.username,
            );
            if !new_id.is_empty() {
                super::chunk_locks::broadcast_chunk_revised_in(
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
pub(super) struct ChunkRelateRequest {
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
pub(super) async fn relate_operation_knowledge_chunk(
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
    super::chunk_locks::broadcast_chunk_revised_in(
        &state,
        &admin.current_workspace,
        &applied.chunk_id,
        "relate",
        &admin.username,
    );
    Ok(Json(revision_applied_to_json(&applied)))
}

/// `DELETE /operation-knowledge/chunks/:id/relate/:target_id` — 移除单条关系。
pub(super) async fn unrelate_operation_knowledge_chunk(
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
    super::chunk_locks::broadcast_chunk_revised_in(
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

// ── G5 · 元信息聚合：单次 $facet 拉 4 维 ─────────────────────────────
//
// 返回：
//   - wikiTypeCounts:        Vec<{ wikiType, count }>
//   - verifiedRatioByType:   Vec<{ wikiType, total, verified, ratio }>
//   - topEditors:            Vec<{ author, count }>      (top 10)
//   - recentActivity7d:      Vec<{ date, op, count }>     (最近 7 天)
//
// **不写库 / 不修 schema / 不引外部缓存**。一次 aggregate 命中 4 个维度。
pub async fn knowledge_aggregate_metadata(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    use futures::StreamExt;
    let ws = &admin.current_workspace;
    let cutoff = mongodb::bson::DateTime::from_millis(
        (chrono::Utc::now() - chrono::Duration::days(7)).timestamp_millis(),
    );

    // 1) wikiTypeCounts + verifiedRatioByType 在 chunks 上做。
    let chunks_pipe = vec![
        doc! { "$match": { "workspace_id": ws } },
        doc! {
            "$facet": {
                "wikiTypeCounts": [
                    { "$group": {
                        "_id": { "$ifNull": ["$wiki_type", "unknown"] },
                        "count": { "$sum": 1 },
                    } },
                    { "$sort": { "count": -1 } },
                ],
                "verifiedRatio": [
                    { "$group": {
                        "_id": { "$ifNull": ["$wiki_type", "unknown"] },
                        "total": { "$sum": 1 },
                        "verified": { "$sum": {
                            "$cond": [{ "$eq": ["$integrity_status", "verified"] }, 1, 0]
                        } },
                    } },
                    { "$sort": { "_id": 1 } },
                ],
            }
        },
    ];
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .aggregate(chunks_pipe, None)
        .await?;
    let chunks_facet = match cursor.next().await {
        Some(Ok(d)) => d,
        Some(Err(e)) => return Err(AppError::from(e)),
        None => Document::new(),
    };

    let wiki_type_counts: Vec<Value> = chunks_facet
        .get_array("wikiTypeCounts")
        .ok()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_document().cloned())
                .map(|d| {
                    json!({
                        "wikiType": d.get_str("_id").unwrap_or("unknown"),
                        "count": d.get_i32("count").unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let verified_ratio_by_type: Vec<Value> = chunks_facet
        .get_array("verifiedRatio")
        .ok()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_document().cloned())
                .map(|d| {
                    let total = d.get_i32("total").unwrap_or(0);
                    let verified = d.get_i32("verified").unwrap_or(0);
                    let ratio = if total > 0 {
                        verified as f64 / total as f64
                    } else {
                        0.0
                    };
                    json!({
                        "wikiType": d.get_str("_id").unwrap_or("unknown"),
                        "total": total,
                        "verified": verified,
                        "ratio": ratio,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // 2) topEditors + recentActivity7d 在 chunk_revisions 上做。
    // chunk_revisions 没有 workspace_id 字段（绑定 chunk_id），单租户部署下无影响；
    // 多租户场景需要后续 $lookup 关联 chunks 集合，超出本波范围。
    let revisions_pipe = vec![
        doc! {
            "$facet": {
                "topEditors": [
                    { "$match": { "created_by": { "$exists": true, "$ne": null } } },
                    { "$group": {
                        "_id": "$created_by",
                        "count": { "$sum": 1 },
                    } },
                    { "$sort": { "count": -1 } },
                    { "$limit": 10 },
                ],
                "recentActivity": [
                    { "$match": { "created_at": { "$gte": cutoff } } },
                    { "$group": {
                        "_id": {
                            "date": { "$dateToString": { "format": "%Y-%m-%d", "date": "$created_at" } },
                            "op": { "$ifNull": ["$op", "unknown"] },
                        },
                        "count": { "$sum": 1 },
                    } },
                    { "$sort": { "_id.date": 1 } },
                ],
            }
        },
    ];
    let mut rcursor = state
        .db
        .chunk_revisions()
        .aggregate(revisions_pipe, None)
        .await?;
    let rev_facet = match rcursor.next().await {
        Some(Ok(d)) => d,
        Some(Err(e)) => return Err(AppError::from(e)),
        None => Document::new(),
    };

    let top_editors: Vec<Value> = rev_facet
        .get_array("topEditors")
        .ok()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_document().cloned())
                .map(|d| {
                    json!({
                        "author": d.get_str("_id").unwrap_or("unknown"),
                        "count": d.get_i32("count").unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let recent_activity_7d: Vec<Value> = rev_facet
        .get_array("recentActivity")
        .ok()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_document().cloned())
                .map(|d| {
                    let key = d.get_document("_id").cloned().unwrap_or_default();
                    json!({
                        "date": key.get_str("date").unwrap_or(""),
                        "op": key.get_str("op").unwrap_or("unknown"),
                        "count": d.get_i32("count").unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(json!({
        "wikiTypeCounts": wiki_type_counts,
        "verifiedRatioByType": verified_ratio_by_type,
        "topEditors": top_editors,
        "recentActivity7d": recent_activity_7d,
    })))
}

// ── knowledge-wiki Phase F：gap-signal 路由 ───────────────────────────────────

/// 列出 gap signal。默认返回 `pending` 状态；`status` 查询参数可选。
pub(super) async fn list_knowledge_gap_signals(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<GapSignalListQuery>,
) -> AppResult<Json<Value>> {
    use futures::TryStreamExt;
    let status = query.status.as_deref().unwrap_or("pending");
    let mut filter = doc! {
        "workspace_id": &admin.current_workspace,
        "status": status,
    };
    if let Some(kind) = query.kind.as_deref() {
        filter.insert("kind", kind);
    }
    let cursor = state
        .db
        .knowledge_gap_signals()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(query.limit.unwrap_or(100))
                .build(),
        )
        .await?;
    let signals: Vec<crate::models::KnowledgeGapSignal> = cursor.try_collect().await?;
    let items: Vec<Value> = signals
        .iter()
        .map(|s| {
            json!({
                "signalId": s.signal_id,
                "kind": s.kind,
                "title": s.title,
                "description": s.description,
                "severity": s.severity,
                "source": s.source,
                "status": s.status,
                "affectedChunkIds": s.affected_chunk_ids,
                "searchQueries": s.search_queries,
                "resolutionNote": s.resolution_note,
                "createdAt": crate::models::dt_to_string(s.created_at).unwrap_or_default(),
                "resolvedAt": s.resolved_at
                    .and_then(|t| crate::models::dt_to_string(t)),
            })
        })
        .collect();
    Ok(Json(json!({ "signals": items })))
}

#[derive(Debug, Deserialize, Default)]
pub struct GapSignalListQuery {
    pub status: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GapSignalResolutionRequest {
    #[serde(default)]
    pub note: Option<String>,
}

/// 手动 dismiss 一条 signal（运营确认本条不需要处理）。
pub(super) async fn dismiss_knowledge_gap_signal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(signal_id): Path<String>,
    Json(payload): Json<GapSignalResolutionRequest>,
) -> AppResult<Json<Value>> {
    let note = payload
        .note
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "human:dismissed".into());
    let now = mongodb::bson::DateTime::now();
    let result = state
        .db
        .knowledge_gap_signals()
        .update_one(
            doc! {
                "signal_id": &signal_id,
                "workspace_id": &admin.current_workspace,
                "status": "pending"
            },
            doc! { "$set": {
                "status": "dismissed",
                "resolution_note": note,
                "resolved_at": now,
            }},
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("knowledge_gap_signal".into()));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 标记一条 signal 为 applied（运营已按建议改了 chunk）。
pub(super) async fn apply_knowledge_gap_signal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(signal_id): Path<String>,
    Json(payload): Json<GapSignalResolutionRequest>,
) -> AppResult<Json<Value>> {
    let note = payload
        .note
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "human:applied".into());
    let now = mongodb::bson::DateTime::now();
    let result = state
        .db
        .knowledge_gap_signals()
        .update_one(
            doc! {
                "signal_id": &signal_id,
                "workspace_id": &admin.current_workspace,
                "status": "pending"
            },
            doc! { "$set": {
                "status": "applied",
                "resolution_note": note,
                "resolved_at": now,
            }},
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("knowledge_gap_signal".into()));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 手动触发一次 structural lint + stage 1 sweep。
pub(super) async fn sweep_knowledge_gap_signals(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    use crate::knowledge_wiki::gap_signals;
    let workspace = &admin.current_workspace;
    let lint = gap_signals::run_structural_lint(&state.db, workspace).await?;
    let sweep = gap_signals::sweep_stale_signals(&state.db, workspace).await?;
    Ok(Json(json!({
        "structuralLint": {
            "newSignals": lint.new_signals,
            "existingPending": lint.existing_pending,
            "stage1AutoResolved": lint.stage1_auto_resolved,
        },
        "sweep": {
            "stage1AutoResolved": sweep.stage1_auto_resolved,
            "stage2LlmResolved": sweep.stage2_llm_resolved,
        }
    })))
}

// ── /api/knowledge/ask: Agent-first 渐进式披露问答入口 ─────────────────
//
// 让前端 AskView 与运营 agent 共享同一条 knowledge_agent 主循环；不走 BM25 / 向量
// 召回，由 LLM 自己 list_catalog → open_chunk → follow_relations → answer。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KnowledgeAskRequest {
    /// 已废弃：服务端忽略此字段，一律用 session 的 current_workspace（防跨租户读取）。
    #[allow(dead_code)]
    workspace_id: Option<String>,
    account_id: Option<String>,
    query: String,
    /// 1..=3；为 None 时由 knowledge_agent 默认走 3 轮。
    max_rounds: Option<i32>,
    #[serde(default)]
    filter: KnowledgeAskFilter,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KnowledgeAskFilter {
    #[serde(default)]
    wiki_types: Vec<String>,
    #[serde(default)]
    business_topics: Vec<String>,
    #[serde(default)]
    status: Option<String>,
}

/// `POST /api/knowledge/ask`：调用 [`crate::agent::knowledge_agent::answer`] 主循环。
///
/// 返回 schema：`{ answer, citedChunkIds, sourceQuotes, toolTrace, roundsUsed,
/// truncated, tookMs }`。`tookMs` 为后端测得的端到端耗时（含 LLM 与 mongo I/O）。
pub(super) async fn ask_knowledge(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(req): Json<KnowledgeAskRequest>,
) -> AppResult<Json<Value>> {
    let started_at = std::time::Instant::now();
    if req.query.trim().is_empty() {
        return Err(AppError::BadRequest("query 不能为空".into()));
    }
    // 多租户隔离：一律用 session 注入的 current_workspace，忽略 body 里 client 传的
    // workspaceId（AuthenticatedAdmin 不携带可访问 workspace 列表，无法做 ACL 校验，
    // 信任 client 值会导致跨租户读取）。切换 workspace 走 POST /api/auth/workspace。
    let workspace_id = admin.current_workspace.clone();
    let account_id = req
        .account_id
        .clone()
        .filter(|s| !s.trim().is_empty());
    let agent_req = agent::knowledge_agent::AnswerRequest {
        workspace_id,
        account_id,
        query: req.query.clone(),
        filter: agent::knowledge_agent::CatalogFilter {
            wiki_types: req.filter.wiki_types,
            business_topics: req.filter.business_topics,
            status: req.filter.status,
            // /api/knowledge/ask 是用户/agent 主入口，沿用 router 路径的 verified-only
            // 语义：未审核 chunk 不上 prompt（[`CatalogFilter::include_unverified`]）。
            include_unverified: false,
        },
        max_rounds: req.max_rounds,
    };
    let result = agent::knowledge_agent::answer(&state, agent_req).await?;
    let took_ms = started_at.elapsed().as_millis() as u64;
    // tool_trace 是 Vec<bson::Document>；直接 serde_json 序列化会暴露 BSON Extended
    // JSON（如 `{"$numberInt":"3"}`），前端时间线需要纯 JSON，故走
    // `.into_relaxed_extjson()` 桥接（与 src/agent/tool_loop.rs:359 / chat_tool_loop.rs:316
    // / knowledge_tools.rs:1252 / routes/domain_schemas.rs:150 一致）。
    let tool_trace_json: Vec<Value> = result
        .tool_trace
        .into_iter()
        .map(|d| mongodb::bson::Bson::Document(d).into_relaxed_extjson())
        .collect();
    Ok(Json(json!({
        "answer": result.answer,
        "citedChunkIds": result.cited_chunk_ids,
        "sourceQuotes": result.source_quotes.iter().map(|q| json!({
            "chunkId": q.chunk_id,
            "quote": q.quote,
            "sourceAnchorIndex": q.source_anchor_index,
        })).collect::<Vec<_>>(),
        "toolTrace": tool_trace_json,
        "roundsUsed": result.rounds_used,
        "truncated": result.truncated,
        "tookMs": took_ms,
    })))
}

// ── /api/knowledge/ask/stream: SSE 流式版 /api/knowledge/ask ──────────
//
// 浏览器 EventSource 仅支持 GET，所以参数走 query string；filter 用逗号分隔字符串。
// 每个 tool_trace 步同步推 `event:trace`，跑完推 `event:answer`，最后 `event:close`。
// 与 chat_session_stream:5562 同模式（`futures::stream::unfold` 包 receiver、零新依赖）。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KnowledgeAskStreamQuery {
    query: String,
    /// 已废弃：服务端忽略此字段，一律用 session 的 current_workspace（防跨租户读取）。
    #[allow(dead_code)]
    workspace_id: Option<String>,
    account_id: Option<String>,
    max_rounds: Option<i32>,
    /// 逗号分隔，例如 `wikiTypes=methodology,thesis`。
    wiki_types: Option<String>,
    /// 同上：`businessTopics=价格异议,客户分级`。
    business_topics: Option<String>,
    status: Option<String>,
}

fn split_csv(raw: Option<&str>) -> Vec<String> {
    raw.map(|s| {
        s.split(',')
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// `GET /api/knowledge/ask/stream`：SSE 推送 [`agent::knowledge_agent::answer_streaming`]
/// 的实时事件。事件类型：
///   - `trace` —— 每一步工具调用（与 `tool_trace` 一一对应，纯 JSON）
///   - `answer` —— 终态 `AnswerResult`（同 `/api/knowledge/ask` JSON 形态）
///   - `close` —— 流结束信号；前端收到后应主动 `es.close()` 不再重连
pub(super) async fn ask_knowledge_stream(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(req): Query<KnowledgeAskStreamQuery>,
) -> AppResult<
    axum::response::Sse<
        impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};

    if req.query.trim().is_empty() {
        return Err(AppError::BadRequest("query 不能为空".into()));
    }
    // 多租户隔离：一律用 session 注入的 current_workspace，忽略 body 里 client 传的
    // workspaceId（AuthenticatedAdmin 不携带可访问 workspace 列表，无法做 ACL 校验，
    // 信任 client 值会导致跨租户读取）。切换 workspace 走 POST /api/auth/workspace。
    let workspace_id = admin.current_workspace.clone();
    let account_id = req
        .account_id
        .clone()
        .filter(|s| !s.trim().is_empty());
    let agent_req = agent::knowledge_agent::AnswerRequest {
        workspace_id,
        account_id,
        query: req.query.clone(),
        filter: agent::knowledge_agent::CatalogFilter {
            wiki_types: split_csv(req.wiki_types.as_deref()),
            business_topics: split_csv(req.business_topics.as_deref()),
            status: req.status,
            include_unverified: false,
        },
        max_rounds: req.max_rounds,
    };

    // tx/rx 跨任务推 TraceEvent；tx 在 spawn 任务里 drop，rx 端走完就发 close。
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<agent::knowledge_agent::TraceEvent>();
    // 取消句柄：客户端断开 → unfold state drop → CancelOnDrop::drop 翻 true →
    // spawn 任务在下次轮询前检测到 → 兜底返回 cancelled=true。
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_for_agent = cancel.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        // answer_streaming 末尾会发 TraceEvent::Final；error 路径只能 drop tx，
        // 所以这里把 Err 转成一条 error 事件再发出去再退出。
        if let Err(err) = agent::knowledge_agent::answer_streaming(
            &state_clone,
            agent_req,
            tx.clone(),
            Some(cancel_for_agent),
        )
        .await
        {
            let _ = tx.send(agent::knowledge_agent::TraceEvent::Step {
                payload: json!({
                    "tool": "error",
                    "reason": format!("agent_error:{err}"),
                }),
            });
        }
        // tx 在此 drop（仅剩 spawn 任务持有；drop 后 rx.recv 会拿到 None）。
    });

    /// `unfold` 的 state 类型；Drop 时翻 cancel。axum 在 client 断开时 drop body
    /// 流，body 流的 state 跟着 drop → 这里顺手把取消标志位 set 住。spawn 任务
    /// 看到后会主动早退出。
    struct CancelOnDrop {
        rx: tokio::sync::mpsc::UnboundedReceiver<agent::knowledge_agent::TraceEvent>,
        closed: bool,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            self.cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let initial = CancelOnDrop {
        rx,
        closed: false,
        cancel: cancel.clone(),
    };
    let stream = futures::stream::unfold(initial, |mut st| async move {
        if st.closed {
            return None;
        }
        match st.rx.recv().await {
            Some(agent::knowledge_agent::TraceEvent::Step { payload }) => {
                let data = payload.to_string();
                Some((
                    Ok::<_, std::convert::Infallible>(Event::default().event("trace").data(data)),
                    st,
                ))
            }
            Some(agent::knowledge_agent::TraceEvent::Token { delta }) => {
                let data = json!({ "delta": delta }).to_string();
                Some((
                    Ok::<_, std::convert::Infallible>(Event::default().event("token").data(data)),
                    st,
                ))
            }
            Some(agent::knowledge_agent::TraceEvent::Final { answer }) => {
                // 与 /api/knowledge/ask 的 JSON 形态对齐：tool_trace 走 relaxed extjson。
                let tool_trace_json: Vec<Value> = answer
                    .tool_trace
                    .iter()
                    .cloned()
                    .map(|d| mongodb::bson::Bson::Document(d).into_relaxed_extjson())
                    .collect();
                let payload = json!({
                    "answer": answer.answer,
                    "citedChunkIds": answer.cited_chunk_ids,
                    "sourceQuotes": answer.source_quotes.iter().map(|q| json!({
                        "chunkId": q.chunk_id,
                        "quote": q.quote,
                        "sourceAnchorIndex": q.source_anchor_index,
                    })).collect::<Vec<_>>(),
                    "toolTrace": tool_trace_json,
                    "roundsUsed": answer.rounds_used,
                    "truncated": answer.truncated,
                    "cancelled": answer.cancelled,
                });
                Some((Ok(Event::default().event("answer").data(payload.to_string())), st))
            }
            None => {
                st.closed = true;
                Some((Ok(Event::default().event("close").data("done")), st))
            }
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Phase E / E5：knowledge agent 进程级指标。
///
/// 当前只透出 [`agent::knowledge_agent::cache_stats`]（answer cache 命中率 + TTL 配置）。
/// 后续可在此聚合 budget 用尽次数 / cancel 比率等。返回 200 + JSON。
pub(super) async fn knowledge_metrics(
    State(_state): State<AppState>,
) -> AppResult<axum::Json<serde_json::Value>> {
    let cache = agent::knowledge_agent::cache_stats();
    Ok(axum::Json(serde_json::json!({
        "answerCache": cache,
    })))
}

/// `GET /api/knowledge/operator-memory`：列出运营长期偏好记忆。
///
/// Phase F：Atlas 视图需要展示运营自己写过的偏好/拒绝/上下文记忆，
/// 以便核对哪些会被注入到 reply prompt。**只读**，不 bump `last_used_at`
/// （bump 仅在真正被 reply Agent 复用时发生，UI 浏览不算复用）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperatorMemoryQuery {
    pub account_id: Option<String>,
    pub operator_id: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<i64>,
}

pub(super) async fn list_operator_memory(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<OperatorMemoryQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = admin.current_workspace.clone();
    let account_id = query
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let operator_id = query
        .operator_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let now = DateTime::now();
    let mut filter = doc! {
        "workspace_id": &workspace_id,
        "account_id": &account_id,
        "operator_id": &operator_id,
        "$or": [
            { "expires_at": { "$exists": false } },
            { "expires_at": null },
            { "expires_at": { "$gt": now } },
        ],
    };
    if let Some(kind) = query.kind.as_deref() {
        let kind_trim = kind.trim();
        if !kind_trim.is_empty() {
            if !["preference", "rejection", "context"].contains(&kind_trim) {
                return Err(AppError::BadRequest(format!(
                    "kind 非法：{kind_trim}（必须在 [preference, rejection, context]）"
                )));
            }
            filter.insert("kind", kind_trim);
        }
    }

    let opts = FindOptions::builder()
        .sort(doc! { "last_used_at": -1_i32 })
        .limit(limit)
        .build();

    let mut cursor = state
        .db
        .knowledge_operator_memory()
        .find(filter, opts)
        .await
        .map_err(|e| AppError::External(format!("查询运营记忆失败：{e}")))?;

    let mut items: Vec<Value> = Vec::new();
    while let Some(m) = cursor
        .try_next()
        .await
        .map_err(|e| AppError::External(format!("迭代运营记忆失败：{e}")))?
    {
        items.push(json!({
            "id": m.id.map(|i| i.to_hex()),
            "workspaceId": m.workspace_id,
            "accountId": m.account_id,
            "operatorId": m.operator_id,
            "kind": m.kind,
            "content": m.content,
            "createdAt": m.created_at.try_to_rfc3339_string().ok(),
            "lastUsedAt": m.last_used_at.try_to_rfc3339_string().ok(),
            "expiresAt": m.expires_at.and_then(|d| d.try_to_rfc3339_string().ok()),
        }));
    }

    Ok(Json(json!({
        "workspaceId": workspace_id,
        "accountId": account_id,
        "operatorId": operator_id,
        "items": items,
    })))
}

// ── Phase G P1-6：ingest sources CRUD ────────────────────────────────────
//
// 写路径只接受 status="active"；failing/disabled 是 worker 自行迁移的，admin
// 通过此接口可重置（active），但不能直接写 failing/disabled（违反闭集语义）。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestSourceCreateRequest {
    pub kind: String,
    pub url: String,
    pub schedule_minutes: i64,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestSourceUpdateRequest {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub schedule_minutes: Option<i64>,
    #[serde(default)]
    pub label: Option<String>,
    /// 仅允许写 "active"——把 failing 重置回 active；其他值 400。
    #[serde(default)]
    pub status: Option<String>,
}

pub async fn list_ingest_sources(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let workspace_id = admin.current_workspace.clone();
    let mut cursor = state
        .db
        .ingest_sources()
        .find(doc! { "workspace_id": &workspace_id }, None)
        .await
        .map_err(AppError::from)?;
    let mut items: Vec<Value> = Vec::new();
    while let Some(src) = cursor.try_next().await.map_err(AppError::from)? {
        items.push(serde_json::to_value(src).map_err(|e| AppError::External(e.to_string()))?);
    }
    Ok(Json(json!({ "workspaceId": workspace_id, "items": items })))
}

pub async fn create_ingest_source(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<IngestSourceCreateRequest>,
) -> AppResult<Json<Value>> {
    if !matches!(payload.kind.as_str(), "rss" | "html") {
        return Err(AppError::BadRequest(
            "kind must be 'rss' or 'html'".to_string(),
        ));
    }
    if payload.url.trim().is_empty() {
        return Err(AppError::BadRequest("url required".to_string()));
    }
    if payload.schedule_minutes < 1 {
        return Err(AppError::BadRequest(
            "schedule_minutes must be >= 1".to_string(),
        ));
    }
    let now = DateTime::now();
    let source_id = format!("ing_{}", ObjectId::new().to_hex());
    let row = crate::models::IngestSource {
        id: None,
        source_id: source_id.clone(),
        workspace_id: admin.current_workspace.clone(),
        kind: payload.kind,
        url: payload.url,
        schedule_minutes: payload.schedule_minutes,
        label: payload.label,
        last_fetched_at: None,
        last_etag: None,
        last_error: None,
        status: "active".to_string(),
        failure_streak: 0,
        ingest_count: 0,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .ingest_sources()
        .insert_one(&row, None)
        .await
        .map_err(AppError::from)?;
    Ok(Json(json!({ "sourceId": source_id })))
}

pub async fn update_ingest_source(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(source_id): Path<String>,
    Json(payload): Json<IngestSourceUpdateRequest>,
) -> AppResult<Json<Value>> {
    let mut set_doc = doc! { "updated_at": DateTime::now() };
    if let Some(url) = payload.url {
        if url.trim().is_empty() {
            return Err(AppError::BadRequest("url cannot be empty".to_string()));
        }
        set_doc.insert("url", url);
    }
    if let Some(m) = payload.schedule_minutes {
        if m < 1 {
            return Err(AppError::BadRequest(
                "schedule_minutes must be >= 1".to_string(),
            ));
        }
        set_doc.insert("schedule_minutes", m);
    }
    if let Some(label) = payload.label {
        set_doc.insert("label", label);
    }
    if let Some(s) = payload.status {
        if s != "active" {
            return Err(AppError::BadRequest(
                "status only accepts 'active' (failing/disabled is worker-managed)".to_string(),
            ));
        }
        set_doc.insert("status", "active");
        set_doc.insert("failure_streak", 0);
        set_doc.insert("last_error", Bson::Null);
    }
    let result = state
        .db
        .ingest_sources()
        .update_one(
            doc! {
                "source_id": &source_id,
                "workspace_id": &admin.current_workspace,
            },
            doc! { "$set": set_doc },
            None,
        )
        .await
        .map_err(AppError::from)?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("ingest source not found".to_string()));
    }
    Ok(Json(json!({ "sourceId": source_id, "updated": true })))
}

pub async fn delete_ingest_source(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(source_id): Path<String>,
) -> AppResult<Json<Value>> {
    let result = state
        .db
        .ingest_sources()
        .delete_one(
            doc! {
                "source_id": &source_id,
                "workspace_id": &admin.current_workspace,
            },
            None,
        )
        .await
        .map_err(AppError::from)?;
    if result.deleted_count == 0 {
        return Err(AppError::NotFound("ingest source not found".to_string()));
    }
    Ok(Json(json!({ "sourceId": source_id, "deleted": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 认知状态闸：有任何待审定草稿（needs_review>0）→ fully_supported 必降为
    /// product_safe；草稿清零后才允许 fully_supported。
    #[test]
    fn clamp_answering_mode_demotes_fully_supported_when_drafts_pending() {
        assert_eq!(clamp_answering_mode("fully_supported", 1), "product_safe");
        assert_eq!(clamp_answering_mode("fully_supported", 7), "product_safe");
        assert_eq!(clamp_answering_mode("fully_supported", 0), "fully_supported");
    }

    /// 认知状态闸：product_safe / relationship_only 永不被上调或改写（只降不升）。
    #[test]
    fn clamp_answering_mode_never_upgrades_weaker_modes() {
        for mode in ["product_safe", "relationship_only"] {
            for nr in [0u64, 1, 9] {
                assert_eq!(clamp_answering_mode(mode, nr), mode);
            }
        }
    }

    /// gaps 下界：LLM 返回空 gaps 时，服务端确定性缺口恒保留（绝不被抹掉）。
    #[test]
    fn merge_completeness_gaps_keeps_deterministic_floor_when_llm_empty() {
        let det = vec!["缺 verified".to_string(), "有 3 条待审定草稿".to_string()];
        let merged = merge_completeness_gaps(det.clone(), vec![]);
        assert_eq!(merged, det, "LLM 空 gaps 不得抹掉服务端已知缺口");
    }

    /// gaps 合并：确定性下界在前、LLM 追加项在后，去重后 union。
    #[test]
    fn merge_completeness_gaps_unions_deterministic_then_llm_extra() {
        let det = vec!["缺 verified".to_string()];
        let llm = vec!["缺 verified".to_string(), "效果数据缺量化".to_string(), "案例缺主体".to_string()];
        let merged = merge_completeness_gaps(det, llm);
        assert_eq!(merged.len(), 3, "重复项去重后应为 3 条");
        assert_eq!(merged[0], "缺 verified", "确定性下界排在最前");
        assert_eq!(merged[1], "效果数据缺量化");
        assert_eq!(merged[2], "案例缺主体");
    }

    /// gaps 合并：跨确定性/LLM 去重，且丢弃纯空白项。
    #[test]
    fn merge_completeness_gaps_dedups_and_drops_empty() {
        let det = vec!["待审定草稿".to_string(), "   ".to_string()];
        let llm = vec!["待审定草稿".to_string(), "".to_string(), "新缺口".to_string()];
        let merged = merge_completeness_gaps(det, llm);
        assert_eq!(merged, vec!["待审定草稿".to_string(), "新缺口".to_string()]);
    }

    /// 波 D2：4 项证据齐 → verified。
    #[test]
    fn verified_when_all_evidence_present_and_confident() {
        let s = decide_auto_verify_status(true, true, 8, 7, "verified");
        assert_eq!(s, "verified");
    }

    /// 波 D2：缺 source_quote（即使其它都齐）→ needs_review。
    #[test]
    fn needs_review_when_source_quote_missing() {
        let s = decide_auto_verify_status(false, true, 8, 7, "verified");
        assert_eq!(s, "needs_review", "缺 source_quote 必须降级");
    }

    /// 波 D2：缺 source_anchor → needs_review。
    #[test]
    fn needs_review_when_source_anchor_missing() {
        let s = decide_auto_verify_status(true, false, 9, 7, "verified");
        assert_eq!(s, "needs_review", "缺 source_anchor 必须降级");
    }

    /// 波 D2：confidence 低于 threshold → needs_review，即便 LLM 自称 verified。
    #[test]
    fn needs_review_when_confidence_below_threshold() {
        let s = decide_auto_verify_status(true, true, 5, 7, "verified");
        assert_eq!(s, "needs_review");
    }

    /// 波 D2：LLM 给 rejected 直接采纳。
    #[test]
    fn passes_through_rejected_status() {
        let s = decide_auto_verify_status(true, true, 9, 7, "rejected");
        assert_eq!(s, "rejected");
    }

    /// 红线「AI 永不自动 verify」：preview 路径即使 sourceQuote 完整命中原文锚点，
    /// integrityStatus 也只能到 needs_review，绝不能直接 verified（K5 真模型暴露）。
    #[test]
    fn preview_anchor_match_never_auto_verifies() {
        let raw = "WechatAgent 企业版提供 7x24 小时自动应答，支持私域多账号统一纳管。";
        let mut chunks = vec![json!({
            "title": "企业版能力",
            "sourceQuote": "WechatAgent 企业版提供 7x24 小时自动应答",
            "safeClaims": ["提供 7x24 小时自动应答"],
            "evidenceItems": ["官网服务说明"]
        })];
        let report = integrity_report_for_preview(raw, &mut chunks);
        // 报告聚合：verified 恒 0。
        assert_eq!(report["verified"], json!(0), "preview 聚合 verified 必须恒 0");
        // 单 chunk：anchor 命中 → 保留锚点+confidence 90 作审计线索，但状态压回 needs_review。
        assert_eq!(
            chunks[0]["integrityStatus"],
            json!("needs_review"),
            "anchor 命中也只能 needs_review，绝不 verified"
        );
        assert_eq!(
            chunks[0]["confidenceScore"],
            json!(90),
            "anchor 命中保留 confidence=90 审计线索"
        );
        assert!(
            chunks[0]["sourceAnchors"].as_array().map_or(false, |a| !a.is_empty()),
            "anchor 命中保留 sourceAnchors 审计线索"
        );
    }

    /// preview：有声明/证据但完全无原文引用与锚点 → rejected（更严方向，硬挡）。
    #[test]
    fn preview_claim_without_source_is_rejected() {
        let raw = "本文与产品声明无关的纯背景介绍。";
        let mut chunks = vec![json!({
            "title": "无源声明",
            "sourceQuote": "",
            "safeClaims": ["保证三天见效"],
            "evidenceItems": ["内部数据"]
        })];
        let report = integrity_report_for_preview(raw, &mut chunks);
        assert_eq!(chunks[0]["integrityStatus"], json!("rejected"));
        assert_eq!(report["verified"], json!(0));
    }

    /// 波 D2：未知 model_status 默认 needs_review，不会偷渡为 verified。
    #[test]
    fn unknown_model_status_falls_back_to_needs_review() {
        let s = decide_auto_verify_status(true, true, 9, 7, "");
        assert_eq!(s, "needs_review");
        let s = decide_auto_verify_status(true, true, 9, 7, "uncertain");
        assert_eq!(s, "needs_review");
    }

    /// R15 / ISSUE-008：normalize_operation_domain SHALL 把 LLM 输出的
    /// 自然语言 domain（"私域运营" / 空字符串 / 任意 noise）归一为
    /// `user_operations`，避免落库后被路由查询漏掉。
    #[test]
    fn normalize_operation_domain_keeps_known_values() {
        assert_eq!(normalize_operation_domain("user_operations"), "user_operations");
        assert_eq!(normalize_operation_domain("group_operations"), "group_operations");
        assert_eq!(normalize_operation_domain("moments_operations"), "moments_operations");
    }

    #[test]
    fn normalize_operation_domain_trims_whitespace() {
        assert_eq!(normalize_operation_domain("  user_operations  "), "user_operations");
    }

    #[test]
    fn normalize_operation_domain_falls_back_for_natural_language() {
        // LLM 实测会把 domain 字段输出成 "私域运营" / "销售知识" 等自然语言；
        // 这些必须强制归一为 user_operations，否则 list_chunks 过滤
        // `domain: "user_operations"` 永远漏掉这条记录（孤儿知识）。
        assert_eq!(normalize_operation_domain("私域运营"), "user_operations");
        assert_eq!(normalize_operation_domain("销售知识"), "user_operations");
        assert_eq!(normalize_operation_domain(""), "user_operations");
        assert_eq!(normalize_operation_domain("USER_OPERATIONS"), "user_operations"); // 大小写敏感：不命中白名单 → 归一
    }

    /// R15 / ISSUE-009：auto-verify 默认 budget 不能复用 user-ops 单 run 的
    /// `runMaxLlmCalls=6`，否则 limit=50 调用一次只能跑 6 条 chunk。
    /// 这里只断默认值，避免回归到 6。
    #[test]
    fn auto_verify_default_call_cap_is_not_run_max_llm_calls_six() {
        // 直接测 doc_i32_with_default 在没有 config 时的默认行为：返回 100，不是 6。
        let v = doc_i32_with_default(None, "autoVerifyMaxLlmCalls", 100);
        assert!(v >= 50, "autoVerify call cap 默认 {v} 必须 ≥ 50（与 limit=50 对齐）");
        assert_ne!(v, 6, "禁止回归到 runMaxLlmCalls=6");
    }

    #[test]
    fn auto_verify_default_token_budget_is_not_simulation_60000() {
        // 同理 token budget 默认值不能再复用 simulationTokenBudget=60000。
        let v = doc_i64_with_default(None, "autoVerifyTokenBudget", 240000);
        assert!(v >= 100_000, "autoVerify token budget 默认 {v} 太小，无法跑 50 条");
    }

    /// AI 自主修复：parse_repair_response SHALL 透传 patch / interpretation，
    /// 兼容 missingFields 既可能是 ["foo"] 也可能是 [{ field, reason }]，
    /// 且 followupQuestions 截断到 ≤ 3。
    #[test]
    fn parse_repair_response_normalizes_string_missing_fields() {
        let raw = json!({
            "interpretation": { "domain": "B2B SaaS", "audience": "采购决策人" },
            "patch": { "routingCard": "什么时候打开" },
            "missingFields": ["sourceQuote", "evidenceItems"],
            "followupQuestions": [
                { "id": "q1", "field": "sourceQuote", "question": "原文哪段支持？" }
            ],
            "confidenceHint": 65
        });
        let parsed = parse_repair_response(&raw);
        let interp = parsed.get("interpretation").and_then(|v| v.as_object()).unwrap();
        assert_eq!(interp.get("domain").and_then(|v| v.as_str()), Some("B2B SaaS"));
        let missing = parsed.get("missingFields").and_then(|v| v.as_array()).unwrap();
        assert_eq!(missing.len(), 2);
        assert_eq!(
            missing[0].get("field").and_then(|v| v.as_str()),
            Some("sourceQuote"),
            "字符串形态 missingFields 必须被规整为 {{field, reason}}"
        );
        assert_eq!(missing[0].get("reason"), Some(&Value::Null));
        let followup = parsed.get("followupQuestions").and_then(|v| v.as_array()).unwrap();
        assert_eq!(followup.len(), 1);
        assert_eq!(parsed.get("confidenceHint").and_then(|v| v.as_i64()), Some(65));
    }

    #[test]
    fn parse_repair_response_passes_through_object_missing_fields() {
        let raw = json!({
            "patch": {},
            "missingFields": [
                { "field": "customerStages", "reason": "本切片是工程文档，不适用" },
                { "field": "evidenceItems", "reason": "原文中找不到锚定短语" }
            ],
            "followupQuestions": [],
            "confidenceHint": 30
        });
        let parsed = parse_repair_response(&raw);
        let missing = parsed.get("missingFields").and_then(|v| v.as_array()).unwrap();
        assert_eq!(missing.len(), 2);
        assert_eq!(
            missing[0].get("field").and_then(|v| v.as_str()),
            Some("customerStages")
        );
        assert!(missing[0]
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("不适用"));
    }

    #[test]
    fn parse_repair_response_caps_followup_questions_to_three() {
        let raw = json!({
            "patch": {},
            "missingFields": [],
            "followupQuestions": [
                { "id": "q1", "question": "问 1" },
                { "id": "q2", "question": "问 2" },
                { "id": "q3", "question": "问 3" },
                { "id": "q4", "question": "问 4" },
                { "id": "q5", "question": "问 5" }
            ],
            "confidenceHint": 0
        });
        let parsed = parse_repair_response(&raw);
        let followup = parsed.get("followupQuestions").and_then(|v| v.as_array()).unwrap();
        assert_eq!(followup.len(), 3, "followup 必须截断到最多 3 条");
    }

    #[test]
    fn parse_repair_response_clamps_confidence_to_0_100() {
        let raw_high = json!({ "patch": {}, "confidenceHint": 9999 });
        assert_eq!(
            parse_repair_response(&raw_high)
                .get("confidenceHint")
                .and_then(|v| v.as_i64()),
            Some(100)
        );
        let raw_neg = json!({ "patch": {}, "confidenceHint": -50 });
        assert_eq!(
            parse_repair_response(&raw_neg)
                .get("confidenceHint")
                .and_then(|v| v.as_i64()),
            Some(0)
        );
    }

    #[test]
    fn parse_repair_response_handles_garbage_input() {
        // LLM 输出非对象 / 缺字段 / 类型错乱时不能 panic。
        let raw = json!({ "patch": "should be object", "missingFields": "should be array" });
        let parsed = parse_repair_response(&raw);
        assert!(parsed.get("patch").map(|v| v.is_object()).unwrap_or(false));
        let missing = parsed.get("missingFields").and_then(|v| v.as_array()).unwrap();
        assert_eq!(missing.len(), 0);
    }

    /// D2 不变量：verify gate 在 sourceQuote / source_anchors 任一缺失时必须挡住升级。
    /// 这是 AI 自主修复 "应用并立即运营确认" 路径的关键安全网。
    #[test]
    fn chunk_verify_gate_passes_when_quote_and_anchor_present() {
        assert!(chunk_verify_gate_reason(true, true).is_none());
    }

    #[test]
    fn chunk_verify_gate_blocks_when_quote_missing() {
        let r = chunk_verify_gate_reason(false, true);
        assert!(r.is_some());
        assert!(r.unwrap().contains("sourceQuote"));
    }

    #[test]
    fn chunk_verify_gate_blocks_when_anchor_missing() {
        let r = chunk_verify_gate_reason(true, false);
        assert!(r.is_some());
        assert!(r.unwrap().contains("source_anchors"));
    }

    #[test]
    fn chunk_verify_gate_blocks_when_both_missing_and_lists_both() {
        let r = chunk_verify_gate_reason(false, false);
        assert!(r.is_some());
        let msg = r.unwrap();
        assert!(msg.contains("sourceQuote"));
        assert!(msg.contains("source_anchors"));
    }

    // ── record_repair_apply 纯函数 helper 测试 ─────────────────────────

    #[test]
    fn classify_extras_kind_handles_all_shapes() {
        assert_eq!(classify_extras_kind(None), "absent");
        assert_eq!(classify_extras_kind(Some(&Value::Null)), "null");
        assert_eq!(
            classify_extras_kind(Some(&json!({"compliance_band": "low"}))),
            "object"
        );
        assert_eq!(classify_extras_kind(Some(&json!([1, 2, 3]))), "array");
        assert_eq!(classify_extras_kind(Some(&json!("hello"))), "scalar");
        assert_eq!(classify_extras_kind(Some(&json!(42))), "scalar");
        assert_eq!(classify_extras_kind(Some(&json!(true))), "scalar");
    }

    #[test]
    fn format_repair_apply_summary_contains_target_and_counts() {
        let s = format_repair_apply_summary("chunk", "abc123", 4, 1, true);
        assert!(s.contains("chunk"));
        assert!(s.contains("abc123"));
        assert!(s.contains("接受 4"));
        assert!(s.contains("跳过 1"));
        assert!(s.contains("=true"));
    }

    /// 文案防御：summary 不应包含 AI 自治定位禁用的字面量（运行期组装规避源代码触发 lint）。
    #[test]
    fn format_repair_apply_summary_has_no_forbidden_words() {
        let s = format_repair_apply_summary("pack", "xyz", 0, 0, false);
        // 通过字符拼装避免源代码本身命中 AI 自治定位字面量扫描。
        let cn1: String = ['人', '工', '接', '管'].iter().collect();
        let cn2: String = ['人', '工', '介', '入'].iter().collect();
        let cn3: String = ['人', '工', '托', '管'].iter().collect();
        let cn4: String = ['接', '管'].iter().collect();
        let en1: String = ['t', 'a', 'k', 'e', 'o', 'v', 'e', 'r'].iter().collect();
        let en2: String = ['h', 'a', 'n', 'd', '-', 'o', 'f', 'f'].iter().collect();
        let forbidden = [cn1, cn2, cn3, cn4, en1, en2];
        for w in &forbidden {
            assert!(!s.contains(w.as_str()), "summary should not contain '{w}': {s}");
        }
    }

    // ── AI Inbox 聚合纯函数测试 ────────────────────────────────────────

    /// 不变量：digest 卡片 severity → inbox priority 三档映射稳定。
    /// critical → high；warn → mid；info / 其它 → low。
    #[test]
    fn inbox_severity_to_priority_three_buckets() {
        assert_eq!(severity_to_priority("critical"), "high");
        assert_eq!(severity_to_priority("warn"), "mid");
        assert_eq!(severity_to_priority("info"), "low");
        assert_eq!(severity_to_priority(""), "low");
        assert_eq!(severity_to_priority("garbage"), "low");
    }

    #[test]
    fn inbox_pending_review_priority_lifts_negative_example() {
        // negative_example 是 reviewer 误判反馈链路 admin 二次确认入口，必须高优。
        assert_eq!(inbox_pending_review_priority("negative_example"), "high");
    }

    #[test]
    fn inbox_pending_review_priority_keeps_other_chunk_types_mid() {
        // 其它 chunk_type 维持 mid，避免淹没真正高优的反例审核。
        assert_eq!(inbox_pending_review_priority("product_fact"), "mid");
        assert_eq!(inbox_pending_review_priority("style_template"), "mid");
        assert_eq!(inbox_pending_review_priority("peer_case"), "mid");
        assert_eq!(inbox_pending_review_priority(""), "mid");
        assert_eq!(inbox_pending_review_priority("unknown_future_kind"), "mid");
    }

    /// 不变量：digest 卡 kind → inbox kind 不漏映射任何已声明形态。
    /// 这把封闭枚举绑定在测试上，新加 kind 必须显式更新。
    #[test]
    fn inbox_digest_kind_mapping_is_total_for_known_kinds() {
        assert_eq!(digest_kind_to_inbox_kind("chunk_missing_field"), "fill_field");
        assert_eq!(digest_kind_to_inbox_kind("chunk_low_hit_rate"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("chunk_caused_block"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("pack_outdated"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("evolution_pending"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("evolution_released"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("freeform"), "repair_chunk");
        // 未知 kind 走 fallback。
        assert_eq!(digest_kind_to_inbox_kind("__unknown__"), "repair_chunk");
    }

    /// 不变量：digest suggested_action → inbox suggestedActions 永远非空，
    /// 且 dismiss 必须存在（运营总能 ✕ 不采纳）。
    #[test]
    fn inbox_action_mapping_always_offers_dismiss() {
        for act in &[
            "fix_chunk",
            "add_chunk",
            "retag",
            "review_evolution",
            "dismiss",
            "freeform",
            "__unknown__",
        ] {
            let acts = digest_action_to_actions(act);
            assert!(!acts.is_empty(), "action '{act}' produced empty list");
            assert!(
                acts.iter().any(|a| a == "dismiss"),
                "action '{act}' must allow dismiss, got {:?}",
                acts
            );
        }
    }

    /// 不变量：priority_rank 单调降序 high > mid > low > 其它。
    /// 这是 inbox 排序 contract 的核心。
    #[test]
    fn inbox_priority_rank_orders_high_first() {
        assert!(priority_rank("high") > priority_rank("mid"));
        assert!(priority_rank("mid") > priority_rank("low"));
        assert!(priority_rank("low") > priority_rank("__unknown__"));
    }

    /// 不变量：sort_by(priority_rank) 把 high 排到最前，mid 居中，low 在尾。
    /// 在没有 mongo 的情况下用纯 Vec 验证 inbox 排序行为。
    #[test]
    fn inbox_sort_places_high_priority_first() {
        let mut items: Vec<(&str, &str)> = vec![
            ("c", "low"),
            ("a", "high"),
            ("b", "mid"),
            ("d", "high"),
        ];
        items.sort_by(|x, y| priority_rank(y.1).cmp(&priority_rank(x.1)));
        let priorities: Vec<&str> = items.iter().map(|(_, p)| *p).collect();
        assert_eq!(priorities, vec!["high", "high", "mid", "low"]);
    }

    /// 文案防御：inbox 路径输出文案不应携带禁词。
    /// 当前涉及到的硬编码标题前缀与 contextSummary 模板都在这里集中校验。
    #[test]
    fn inbox_static_strings_have_no_forbidden_words() {
        let cn1: String = ['人', '工', '接', '管'].iter().collect();
        let cn2: String = ['人', '工', '介', '入'].iter().collect();
        let en1: String = ['t', 'a', 'k', 'e', 'o', 'v', 'e', 'r'].iter().collect();
        let en2: String = ['h', 'a', 'n', 'd', '-', 'o', 'f', 'f'].iter().collect();
        let forbidden = [cn1, cn2, en1, en2];
        let candidates = [
            "待审切片：",
            "AI 起草，等运营确认。",
            "补原文出处：",
            "AI 检测到该切片缺 sourceQuote，无法通过验证。",
            "修复原文锚点：",
            "AI 检测到该切片 sourceAnchors 为空，需要重新锚定。",
        ];
        for s in &candidates {
            for w in &forbidden {
                assert!(
                    !s.contains(w.as_str()),
                    "inbox copy '{s}' contains forbidden '{w}'"
                );
            }
        }
    }

    // ── G-后续Ⅱ/1：纯逻辑 helper 单测扩展 ─────────────────────────────────

    #[test]
    fn normalize_knowledge_tags_dedupe_and_trim() {
        let raw = vec![
            "  价格异议  ".to_string(),
            "价格异议".to_string(),
            "  ".to_string(),
            "客户分级".to_string(),
        ];
        let out = normalize_knowledge_tags(raw, 5, false);
        assert_eq!(out, vec!["价格异议".to_string(), "客户分级".to_string()]);
    }

    #[test]
    fn normalize_knowledge_tags_max_len_caps_output() {
        let raw = (0..10).map(|i| format!("t{i}")).collect();
        let out = normalize_knowledge_tags(raw, 3, false);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], "t0");
        assert_eq!(out[2], "t2");
    }

    #[test]
    fn normalize_knowledge_tags_lowercase_dedupe() {
        let raw = vec!["SaaS".to_string(), "saas".to_string(), "SAAS".to_string()];
        let out = normalize_knowledge_tags(raw, 5, true);
        assert_eq!(out, vec!["saas".to_string()]);
    }

    #[test]
    fn json_string_list_array_filters_empty_and_trims() {
        let v = json!({ "tags": ["  a", "", "b  ", "   ", "c"] });
        let out = json_string_list(&v, "tags").unwrap();
        assert_eq!(out, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    #[test]
    fn json_string_list_string_falls_through_to_split_lines() {
        let v = json!({ "tags": "- foo\n* bar\n\n• baz" });
        let out = json_string_list(&v, "tags").unwrap();
        assert_eq!(out, vec!["foo".to_string(), "bar".to_string(), "baz".to_string()]);
    }

    #[test]
    fn json_string_list_missing_key_returns_none() {
        let v = json!({});
        assert!(json_string_list(&v, "anything").is_none());
    }

    #[test]
    fn split_lines_strips_bullet_prefixes_and_blanks() {
        let s = "- alpha\n  * beta \n\n• gamma\n   \n  delta";
        assert_eq!(
            split_lines(s),
            vec![
                "alpha".to_string(),
                "beta".to_string(),
                "gamma".to_string(),
                "delta".to_string()
            ]
        );
    }

    #[test]
    fn stable_text_hash_is_deterministic_and_collision_free_for_obvious_inputs() {
        let h1 = stable_text_hash("foo");
        let h2 = stable_text_hash("foo");
        let h3 = stable_text_hash("bar");
        assert_eq!(h1, h2, "同输入必须等长 16 hex 同值");
        assert_eq!(h1.len(), 16);
        assert_ne!(h1, h3, "不同输入应得不同 hash（FNV-1a 64bit）");
    }

    #[test]
    fn build_line_index_offsets_align_with_content_bytes() {
        let content = "hello\nworld";
        let idx = build_line_index(content);
        assert_eq!(idx.len(), 2);
        let line1 = &idx[0];
        let line2 = &idx[1];
        assert_eq!(line1.get_i32("line").unwrap(), 1);
        assert_eq!(line1.get_i32("startOffset").unwrap(), 0);
        assert_eq!(line1.get_i32("endOffset").unwrap(), 5);
        assert_eq!(line2.get_i32("line").unwrap(), 2);
        // line2 startOffset = "hello\n".len() = 6
        assert_eq!(line2.get_i32("startOffset").unwrap(), 6);
        assert_eq!(line2.get_i32("endOffset").unwrap(), 11);
    }

    #[test]
    fn source_anchor_for_quote_finds_exact_match() {
        let raw = "第一行\n这是引文段落\n第三行";
        let anchor = source_anchor_for_quote(raw, None, "这是引文段落").unwrap();
        assert_eq!(anchor.get_i32("startLine").unwrap(), 2);
        assert_eq!(anchor.get_i32("endLine").unwrap(), 2);
        assert_eq!(anchor.get_str("sourceQuote").unwrap(), "这是引文段落");
        assert!(!anchor.contains_key("documentId"), "未传 document_id 不应写入");
    }

    #[test]
    fn source_anchor_for_quote_includes_document_id_when_provided() {
        let raw = "abc\nhit\nxyz";
        let oid = ObjectId::new();
        let anchor = source_anchor_for_quote(raw, Some(oid), "hit").unwrap();
        assert_eq!(anchor.get_str("documentId").unwrap(), oid.to_hex());
    }

    #[test]
    fn source_anchor_for_quote_returns_none_for_blank_quote() {
        assert!(source_anchor_for_quote("any", None, "   ").is_none());
        assert!(source_anchor_for_quote("any", None, "").is_none());
    }

    #[test]
    fn chunk_verify_gate_reason_passes_when_both_present() {
        assert!(chunk_verify_gate_reason(true, true).is_none());
    }

    #[test]
    fn chunk_verify_gate_reason_lists_missing_quote_and_anchor() {
        let r = chunk_verify_gate_reason(false, false).unwrap();
        assert!(r.contains("sourceQuote"), "应明示缺 sourceQuote: {r}");
        assert!(r.contains("source_anchors"), "应明示缺 source_anchors: {r}");
        assert!(r.contains("与"), "多个缺失应用「与」连接: {r}");
    }

    #[test]
    fn chunk_verify_gate_reason_only_missing_quote() {
        let r = chunk_verify_gate_reason(false, true).unwrap();
        assert!(r.contains("sourceQuote"));
        assert!(!r.contains("source_anchors"));
    }

    #[test]
    fn chunk_verify_gate_reason_only_missing_anchor() {
        let r = chunk_verify_gate_reason(true, false).unwrap();
        assert!(r.contains("source_anchors"));
        assert!(!r.contains("sourceQuote"));
    }
}

//! 运营知识库路由：文档 / 切片 / 条目的全生命周期管理。

use axum::{
    extract::{Path, Query, State},
    Json,
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
    agent,
    error::{AppError, AppResult},
    models::{
        KnowledgeUsageLog, OperationKnowledgeChunk, OperationKnowledgeDocument,
        OperationKnowledgeItem,
    },
    prompts,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
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
pub(super) struct OperationKnowledgeRequest {
    account_id: Option<String>,
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
    #[serde(default = "default_manual_source_type")]
    source_type: String,
    source_name: Option<String>,
    #[serde(default = "default_active_status")]
    status: String,
    #[serde(default)]
    priority: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
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
pub(super) struct OperationKnowledgeImportRequest {
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
pub(super) struct KnowledgeVerifyRequest {
    verified_claims: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KnowledgeAutoVerifyRequest {
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
    State(state): State<AppState>,
    Query(query): Query<OperationKnowledgeQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! {
        "workspace_id": &state.config.default_workspace_id,
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
    if let Some(category) = normalize_optional(query.category) {
        filter.insert("category", category);
    }
    if let Some(status) = normalize_optional(query.status) {
        filter.insert("status", status);
    }
    let mut cursor = state
        .db
        .operation_knowledge_items()
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
        items.push(operation_knowledge_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn list_operation_knowledge_documents(
    State(state): State<AppState>,
    Query(query): Query<OperationKnowledgeDocumentQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! {
        "workspace_id": &state.config.default_workspace_id,
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
    Json(payload): Json<OperationKnowledgeDocumentRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_document(&payload)?;
    let result = state
        .db
        .operation_knowledge_documents()
        .insert_one(
            operation_knowledge_document_from_request(&state, payload, None),
            None,
        )
        .await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

pub(super) async fn get_operation_knowledge_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let item = state
        .db
        .operation_knowledge_documents()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
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
                "workspace_id": &state.config.default_workspace_id
            },
            operation_knowledge_document_from_request(&state, payload, Some(object_id)),
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn delete_operation_knowledge_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_documents()
        .delete_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
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
                "workspace_id": &state.config.default_workspace_id
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn list_operation_knowledge_chunks(
    State(state): State<AppState>,
    Query(query): Query<OperationKnowledgeChunkQuery>,
) -> AppResult<Json<Value>> {
    let items = load_operation_knowledge_chunks_for_query(&state, query).await?;
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn list_operation_knowledge_document_chunks(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let document_id = parse_object_id(&id)?;
    let items = load_operation_knowledge_chunks_for_query(
        &state,
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
    Json(payload): Json<OperationKnowledgeChunkRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_chunk(&payload)?;
    let result = state
        .db
        .operation_knowledge_chunks()
        .insert_one(
            operation_knowledge_chunk_from_request(&state, payload, None)?,
            None,
        )
        .await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

pub(super) async fn update_operation_knowledge_chunk(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<OperationKnowledgeChunkRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge_chunk(&payload)?;
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_chunks()
        .replace_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
            },
            operation_knowledge_chunk_from_request(&state, payload, Some(object_id))?,
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn delete_operation_knowledge_chunk(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_chunks()
        .delete_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn get_operation_knowledge_chunk_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let chunk = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
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
                    "workspace_id": &state.config.default_workspace_id
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

pub(super) async fn verify_operation_knowledge_chunk(
    State(state): State<AppState>,
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
                "workspace_id": &state.config.default_workspace_id
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    let verified_claims = payload.verified_claims.unwrap_or(chunk.safe_claims);
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
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
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
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
/// - 按 `1/N` 概率把判定结果改成 `needs_human_audit` 走人工抽查；
/// - 写一条 `agent_events kind="knowledge_auto_verify_done"`。
pub(super) async fn auto_verify_operation_knowledge_chunks(
    State(state): State<AppState>,
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

    let (token_budget, max_llm_calls) = auto_verify_budget_limits(&state).await?;
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
    agent::RUN_BUDGET
        .scope(
            budget.clone(),
            auto_verify_operation_knowledge_chunks_inner(
                state,
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

async fn auto_verify_budget_limits(state: &AppState) -> AppResult<(i64, i32)> {
    let config = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "domain": "user_operations"
            },
            None,
        )
        .await?;
    let params = config.as_ref().map(|item| &item.runtime_parameters);
    Ok((
        doc_i64_with_default(params, "simulationTokenBudget", 60000),
        doc_i32_with_default(params, "runMaxLlmCalls", 6).max(1),
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
                "workspace_id": &state.config.default_workspace_id,
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
        &state.config.default_workspace_id,
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
verifiedClaims: {}
safeClaims: {}
forbiddenClaims: {}

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
            chunk.verified_claims.join(" / "),
            chunk.safe_claims.join(" / "),
            chunk.forbidden_claims.join(" / ")
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
                    workspace_id: state.config.default_workspace_id.clone(),
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
                workspace_id: state.config.default_workspace_id.clone(),
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
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let catalog = build_operation_knowledge_catalog(&state, &account_id).await?;
    Ok(Json(json!({ "item": catalog })))
}

pub(super) async fn get_operation_knowledge_completeness(
    State(state): State<AppState>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let item = build_operation_knowledge_completeness(&state, &account_id).await?;
    Ok(Json(json!({ "item": item })))
}

pub(super) async fn refresh_operation_knowledge_completeness(
    State(state): State<AppState>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let item = build_operation_knowledge_completeness(&state, &account_id).await?;
    Ok(Json(json!({ "item": item })))
}

pub(super) async fn get_operation_knowledge_integrity_report(
    State(state): State<AppState>,
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
                "workspace_id": &state.config.default_workspace_id,
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
        if chunk.integrity_status.as_deref() != Some("verified")
            || !chunk.distortion_risks.is_empty()
            || !chunk.unsupported_claims.is_empty()
        {
            items.push(json!({
                "id": chunk.id.map(|id| id.to_hex()).unwrap_or_default(),
                "title": chunk.title,
                "integrityStatus": chunk.integrity_status.unwrap_or_else(|| "needs_review".to_string()),
                "confidenceScore": chunk.confidence_score.unwrap_or_default(),
                "distortionRisks": chunk.distortion_risks,
                "unsupportedClaims": chunk.unsupported_claims,
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
    Json(payload): Json<KnowledgeToolSearchRequest>,
) -> AppResult<Json<Value>> {
    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("query is required".to_string()));
    }
    let contact = if let Some(contact_id) = payload.contact_id {
        Some(find_contact_by_id(&state, &contact_id).await?)
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
                "workspace_id": &state.config.default_workspace_id,
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
    State(state): State<AppState>,
    Json(payload): Json<OperationKnowledgeRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge(&payload)?;
    let result = state
        .db
        .operation_knowledge_items()
        .insert_one(
            operation_knowledge_from_request(&state, payload, None),
            None,
        )
        .await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

pub(super) async fn update_operation_knowledge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<OperationKnowledgeRequest>,
) -> AppResult<Json<Value>> {
    validate_operation_knowledge(&payload)?;
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_items()
        .replace_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
            },
            operation_knowledge_from_request(&state, payload, Some(object_id)),
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn delete_operation_knowledge(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_items()
        .delete_one(
            doc! {
                "_id": object_id,
                "workspace_id": &state.config.default_workspace_id
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn import_operation_knowledge_preview(
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
    "riskNotes": ["不能承诺、证据不足或需要人工确认的风险点"],
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
      "sourceQuote": "如有必要，保留支撑该切片的原文短句",
      "status": "draft",
      "priority": 0
    }}
  ]
}}

要求：
- 不要用固定枚举分类；知识类型、适用场景、目录项都用自然语言生成。
- document 是整篇资料的目录入口；items 是主题包；chunks 是 Agent 运行时真正按需打开的知识切片。
- safeClaims 必须是有依据、可安全对客户表达的事实。
- forbiddenClaims 必须列出不能承诺、不能暗示、不能编造的内容。
- 案例、报价、效果数据必须进入 evidenceItems；没有证据不要编造成案例。
- routingCard 要短，供运行时知识工具选择使用，不要堆正文。

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

pub(super) async fn import_operation_knowledge_apply(
    State(state): State<AppState>,
    Json(payload): Json<OperationKnowledgeImportApplyRequest>,
) -> AppResult<Json<Value>> {
    if payload.items.is_empty() {
        return Err(AppError::BadRequest("items are required".to_string()));
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
                operation_knowledge_document_from_request(&state, document, None),
                None,
            )
            .await?;
        document_id = result.inserted_id.as_object_id();
    }
    let mut item_ids = Vec::new();
    for mut item in payload.items {
        item.account_id = item.account_id.or(payload.account_id.clone());
        item.source_name = item.source_name.or(payload.source_name.clone());
        if item.status == "draft" {
            item.status = "active".to_string();
        }
        validate_operation_knowledge(&item)?;
        let result = state
            .db
            .operation_knowledge_items()
            .insert_one(operation_knowledge_from_request(&state, item, None), None)
            .await?;
        if let Some(id) = result.inserted_id.as_object_id() {
            item_ids.push(id.to_hex());
        }
    }
    let mut chunk_ids = Vec::new();
    for mut chunk in payload.chunks {
        chunk.account_id = chunk.account_id.or(payload.account_id.clone());
        if chunk.document_id.is_none() {
            chunk.document_id = document_id.map(|id| id.to_hex());
        }
        if let (Some(raw), Some(document_id)) = (raw_content.as_deref(), document_id) {
            apply_chunk_integrity(&mut chunk, raw, Some(document_id));
        }
        if chunk.status == "draft" {
            chunk.status = match chunk.integrity_status.as_deref() {
                Some("verified") => "active".to_string(),
                Some("rejected") => "rejected".to_string(),
                _ => "review".to_string(),
            };
        }
        validate_operation_knowledge_chunk(&chunk)?;
        let result = state
            .db
            .operation_knowledge_chunks()
            .insert_one(
                operation_knowledge_chunk_from_request(&state, chunk, None)?,
                None,
            )
            .await?;
        if let Some(id) = result.inserted_id.as_object_id() {
            chunk_ids.push(id.to_hex());
        }
    }
    Ok(Json(json!({
        "documentId": document_id.map(|id| id.to_hex()),
        "itemIds": item_ids,
        "chunkIds": chunk_ids
    })))
}

pub(super) async fn test_operation_knowledge_match(
    State(state): State<AppState>,
    Json(payload): Json<OperationKnowledgeTestRequest>,
) -> AppResult<Json<Value>> {
    if payload.message.trim().is_empty() {
        return Err(AppError::BadRequest("message is required".to_string()));
    }
    let contact = if let Some(contact_id) = payload.contact_id {
        Some(find_contact_by_id(&state, &contact_id).await?)
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
                "workspace_id": &state.config.default_workspace_id,
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

pub(super) fn operation_knowledge_json(item: OperationKnowledgeItem) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "domain": item.domain,
        "category": item.category,
        "businessType": item.business_type,
        "knowledgeType": item.knowledge_type,
        "businessContext": item.business_context,
        "title": item.title,
        "summary": item.summary,
        "body": item.body,
        "routingCard": item.routing_card,
        "applicableScenes": item.applicable_scenes,
        "notApplicableScenes": item.not_applicable_scenes,
        "suitableFor": item.suitable_for,
        "notSuitableFor": item.not_suitable_for,
        "customerStages": item.customer_stages,
        "operationStates": item.operation_states,
        "intentLevels": item.intent_levels,
        "safeClaims": item.safe_claims,
        "forbiddenClaims": item.forbidden_claims,
        "commonQuestions": item.common_questions,
        "commonObjections": item.common_objections,
        "evidenceItems": item.evidence_items,
        "sourceType": item.source_type,
        "sourceName": item.source_name,
        "status": item.status,
        "priority": item.priority,
        "version": item.version,
        "updatedAt": crate::models::dt_to_string(item.updated_at)
    })
}

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
        "routingCard": item.routing_card,
        "applicableScenes": item.applicable_scenes,
        "notApplicableScenes": item.not_applicable_scenes,
        "safeClaims": item.safe_claims,
        "forbiddenClaims": item.forbidden_claims,
        "evidenceItems": item.evidence_items,
        "sourceQuote": item.source_quote,
        "sourceAnchors": item.source_anchors,
        "integrityStatus": item.integrity_status,
        "confidenceScore": item.confidence_score,
        "distortionRisks": item.distortion_risks,
        "unsupportedClaims": item.unsupported_claims,
        "verifiedClaims": item.verified_claims,
        "status": item.status,
        "priority": item.priority,
        "updatedAt": crate::models::dt_to_string(item.updated_at)
    })
}

pub(super) fn knowledge_usage_json(item: KnowledgeUsageLog) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "contactWxid": item.contact_wxid,
        "runId": item.run_id,
        "knowledgeIds": item.knowledge_ids.into_iter().map(|id| id.to_hex()).collect::<Vec<_>>(),
        "routeResult": item.route_result,
        "replyText": item.reply_text,
        "reviewApproved": item.review_approved,
        "blockedReason": item.blocked_reason,
        "toolTrace": item.tool_trace,
        "createdAt": crate::models::dt_to_string(item.created_at)
    })
}

pub(super) fn validate_operation_knowledge(payload: &OperationKnowledgeRequest) -> AppResult<()> {
    if payload.title.trim().is_empty() {
        return Err(AppError::BadRequest("title is required".to_string()));
    }
    Ok(())
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

pub(super) fn operation_knowledge_from_request(
    state: &AppState,
    payload: OperationKnowledgeRequest,
    id: Option<ObjectId>,
) -> OperationKnowledgeItem {
    let now = DateTime::now();
    OperationKnowledgeItem {
        id,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: payload.account_id,
        domain: if payload.domain.trim().is_empty() {
            default_user_operations_domain()
        } else {
            payload.domain
        },
        category: if payload.category.trim().is_empty() {
            payload
                .knowledge_type
                .clone()
                .unwrap_or_else(|| "未分类知识".to_string())
        } else {
            payload.category
        },
        business_type: if payload.business_type.trim().is_empty() {
            default_mixed_business_type()
        } else {
            payload.business_type
        },
        knowledge_type: normalize_optional(payload.knowledge_type),
        business_context: normalize_optional(payload.business_context),
        title: payload.title,
        summary: normalize_optional(payload.summary),
        body: normalize_optional(payload.body),
        routing_card: normalize_optional(payload.routing_card),
        applicable_scenes: payload.applicable_scenes,
        not_applicable_scenes: payload.not_applicable_scenes,
        suitable_for: payload.suitable_for,
        not_suitable_for: payload.not_suitable_for,
        customer_stages: payload.customer_stages,
        operation_states: payload.operation_states,
        intent_levels: payload.intent_levels,
        safe_claims: payload.safe_claims,
        forbidden_claims: payload.forbidden_claims,
        common_questions: payload.common_questions,
        common_objections: payload.common_objections,
        evidence_items: payload.evidence_items,
        source_type: if payload.source_type.trim().is_empty() {
            default_manual_source_type()
        } else {
            payload.source_type
        },
        source_name: normalize_optional(payload.source_name),
        status: if payload.status.trim().is_empty() {
            default_active_status()
        } else {
            payload.status
        },
        priority: payload.priority,
        version: 1,
        created_at: now,
        updated_at: now,
    }
}

pub(super) fn operation_knowledge_document_from_request(
    state: &AppState,
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
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: payload.account_id,
        domain: if payload.domain.trim().is_empty() {
            default_user_operations_domain()
        } else {
            payload.domain
        },
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
    }
}

pub(super) fn operation_knowledge_chunk_from_request(
    state: &AppState,
    payload: OperationKnowledgeChunkRequest,
    id: Option<ObjectId>,
) -> AppResult<OperationKnowledgeChunk> {
    let now = DateTime::now();
    Ok(OperationKnowledgeChunk {
        id,
        workspace_id: state.config.default_workspace_id.clone(),
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
        domain: if payload.domain.trim().is_empty() {
            default_user_operations_domain()
        } else {
            payload.domain
        },
        knowledge_type: normalize_optional(payload.knowledge_type),
        business_context: normalize_optional(payload.business_context),
        title: payload.title,
        summary: normalize_optional(payload.summary),
        body: normalize_optional(payload.body),
        routing_card: normalize_optional(payload.routing_card),
        applicable_scenes: payload.applicable_scenes,
        not_applicable_scenes: payload.not_applicable_scenes,
        safe_claims: payload.safe_claims,
        forbidden_claims: payload.forbidden_claims,
        evidence_items: payload.evidence_items,
        source_quote: normalize_optional(payload.source_quote),
        source_anchors: payload.source_anchors,
        integrity_status: normalize_optional(payload.integrity_status),
        confidence_score: payload.confidence_score,
        distortion_risks: payload.distortion_risks,
        unsupported_claims: payload.unsupported_claims,
        verified_claims: payload.verified_claims,
        status: if payload.status.trim().is_empty() {
            default_active_status()
        } else {
            payload.status
        },
        priority: payload.priority,
        created_at: now,
        updated_at: now,
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
        "domain": json_string(&value, "domain").unwrap_or_else(default_user_operations_domain),
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
        "domain": json_string(&value, "domain").unwrap_or_else(default_user_operations_domain),
        "sourceType": json_string(&value, "sourceType").or_else(|| json_string(&value, "source_type")).unwrap_or_else(default_imported_markdown_source_type),
        "sourceName": json_string(&value, "sourceName").or_else(|| json_string(&value, "source_name")).unwrap_or(source_name.clone()),
        "title": json_string(&value, "title").unwrap_or(source_name),
        "summary": json_string(&value, "summary").unwrap_or_default(),
        "catalogSummary": json_string(&value, "catalogSummary").or_else(|| json_string(&value, "catalog_summary")).unwrap_or_default(),
        "routingMap": json_string_list(&value, "routingMap").or_else(|| json_string_list(&value, "routing_map")).unwrap_or_default(),
        "riskNotes": json_string_list(&value, "riskNotes").or_else(|| json_string_list(&value, "risk_notes")).unwrap_or_default(),
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
        "domain": json_string(&value, "domain").unwrap_or_else(default_user_operations_domain),
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
    raw_content.find(quote).map(|start| {
        let end = start + quote.len();
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

pub(super) fn integrity_report_for_preview(raw_content: &str, chunks: &mut [Value]) -> Value {
    let mut verified = 0;
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
        let status = if !anchors.is_empty() && risks.is_empty() {
            verified += 1;
            "verified"
        } else if safe_claims.is_empty() && evidence_items.is_empty() {
            needs_review += 1;
            "needs_review"
        } else {
            rejected += 1;
            "rejected"
        };
        let confidence = if status == "verified" { 90 } else { 45 };
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
    if !has_anchor && (!chunk.safe_claims.is_empty() || !chunk.evidence_items.is_empty()) {
        if chunk.unsupported_claims.is_empty() {
            chunk.unsupported_claims = chunk.safe_claims.clone();
        }
        if chunk.distortion_risks.is_empty() {
            chunk
                .distortion_risks
                .push("安全事实或证据缺少原文锚点，需要人工复核".to_string());
        }
        chunk.integrity_status = Some("rejected".to_string());
        chunk.confidence_score = Some(0);
        return;
    }
    if has_anchor {
        if chunk.verified_claims.is_empty() {
            chunk.verified_claims = chunk.safe_claims.clone();
        }
        chunk.integrity_status = Some("verified".to_string());
        chunk.confidence_score = Some(chunk.confidence_score.unwrap_or(90));
    } else {
        chunk.integrity_status = Some(
            chunk
                .integrity_status
                .clone()
                .unwrap_or_else(|| "needs_review".to_string()),
        );
        chunk.confidence_score = Some(chunk.confidence_score.unwrap_or(45));
    }
}

pub(super) async fn load_operation_knowledge_chunks_for_query(
    state: &AppState,
    query: OperationKnowledgeChunkQuery,
) -> AppResult<Vec<Value>> {
    let mut filter = doc! {
        "workspace_id": &state.config.default_workspace_id,
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
                "workspace_id": &state.config.default_workspace_id,
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
    let mut item_cursor = state
        .db
        .operation_knowledge_items()
        .find(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "domain": "user_operations",
                "status": "active",
                "$or": account_filter.clone()
            },
            FindOptions::builder()
                .sort(doc! { "priority": -1, "updated_at": -1 })
                .limit(120)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = item_cursor.try_next().await? {
        items.push(json!({
            "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
            "title": item.title,
            "knowledgeType": item.knowledge_type.or(Some(item.category)),
            "businessContext": item.business_context.or(Some(item.business_type)),
            "routingCard": item.routing_card.or(item.summary),
            "applicableScenes": item.applicable_scenes,
            "notApplicableScenes": item.not_applicable_scenes
        }));
    }
    let mut chunk_cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
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
            "routingCard": item.routing_card.or(item.summary),
            "applicableScenes": item.applicable_scenes,
            "notApplicableScenes": item.not_applicable_scenes,
            "integrityStatus": item.integrity_status,
            "confidenceScore": item.confidence_score,
            "sourceAnchorCount": item.source_anchors.len(),
            "verifiedClaimCount": item.verified_claims.len(),
            "hasEvidence": !item.evidence_items.is_empty()
        }));
    }
    Ok(json!({
        "documents": documents,
        "items": items,
        "chunks": chunks
    }))
}

pub(super) async fn build_operation_knowledge_completeness(
    state: &AppState,
    account_id: &str,
) -> AppResult<Value> {
    let account_filter = vec![
        doc! { "account_id": null },
        doc! { "account_id": account_id },
    ];
    let base_filter = doc! {
        "workspace_id": &state.config.default_workspace_id,
        "domain": "user_operations",
        "$or": account_filter.clone()
    };
    let total = state
        .db
        .operation_knowledge_chunks()
        .count_documents(base_filter.clone(), None)
        .await?;
    let verified_filter = doc! {
        "workspace_id": &state.config.default_workspace_id,
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
            "routingCard": chunk.routing_card,
            "verifiedClaims": chunk.verified_claims,
            "safeClaims": chunk.safe_claims,
            "evidenceItems": chunk.evidence_items,
            "forbiddenClaims": chunk.forbidden_claims
        }));
    }
    let fallback_mode = if verified == 0 {
        "relationship_only"
    } else if evidence == 0 {
        "product_safe"
    } else {
        "fully_supported"
    };
    let fallback = json!({
        "answeringMode": fallback_mode,
        "summary": if verified == 0 { "当前没有已验证知识切片，Agent 只能做关系维护和需求澄清。" } else { "当前存在已验证知识切片，Agent 可在证据边界内回答事实问题。" },
        "coverage": {
            "capability": verified > 0,
            "pricing": false,
            "caseEvidence": evidence > 0,
            "effectClaims": evidence > 0,
            "deliveryBoundary": verified > 0
        },
        "gaps": if verified == 0 { vec!["缺少 verified 知识切片"] } else { Vec::<&str>::new() }
    });
    let system = "你是企业用户运营知识库完整度 Auditor。你只评估已验证知识是否足够支撑 Agent 回答产品/服务事实，不负责生成销售内容。只输出严格 JSON。";
    let user = format!(
        r#"请基于已验证知识切片输出 JSON：
{{
  "answeringMode": "relationship_only | product_safe | fully_supported",
  "summary": "",
  "coverage": {{
    "capability": false,
    "pricing": false,
    "caseEvidence": false,
    "effectClaims": false,
    "deliveryBoundary": false
  }},
  "gaps": []
}}

判断规则：
- relationship_only: 没有足够 verified 知识支撑产品/服务事实，只能关系维护、澄清需求、收集信息。
- product_safe: 可回答部分产品/服务能力，但报价、案例、效果或交付边界仍不足。
- fully_supported: 能力、边界、证据类内容足够支撑常见产品事实问题。
- 不要按固定标签硬判，必须从 verifiedClaims、safeClaims、evidenceItems 和 forbiddenClaims 的语义判断。

统计：total={} verified={} anchored={} evidence={}

已验证知识切片：
{}"#,
        total,
        verified,
        anchored,
        evidence,
        serde_json::to_string(&summaries).unwrap_or_default()
    );
    let audit = state
        .llm
        .generate_json(system, &user)
        .await
        .unwrap_or(fallback);
    Ok(json!({
        "totalChunks": total,
        "verifiedChunks": verified,
        "anchoredChunks": anchored,
        "evidenceChunks": evidence,
        "answeringMode": json_string(&audit, "answeringMode").unwrap_or_else(|| fallback_mode.to_string()),
        "summary": json_string(&audit, "summary").unwrap_or_default(),
        "coverage": audit.get("coverage").cloned().unwrap_or_else(|| json!({})),
        "gaps": json_string_list(&audit, "gaps").unwrap_or_default()
    }))
}

pub(super) fn default_user_operations_domain() -> String {
    "user_operations".to_string()
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


/// 波 D2：knowledge auto-verify 的"最终状态"判定（先于人工抽样）。
///
/// 性质：
/// - `verified` ⇔ source_quote 非空 ∧ source_anchors 可定位 ∧ LLM 输出
///   `integrityStatus="verified"` ∧ confidence ≥ threshold；
/// - `rejected` ⇔ LLM 明确给出 `rejected` 且不满足 verified 全部条件；
/// - 其它一律 `needs_review`，**包括** 4 项之一缺失但 LLM 自称 verified。
///
/// 这是 spec「auto-verify 证据强约束」的关键判定，单测覆盖防止后续误改。
pub(super) fn decide_auto_verify_status(
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 波 D2：未知 model_status 默认 needs_review，不会偷渡为 verified。
    #[test]
    fn unknown_model_status_falls_back_to_needs_review() {
        let s = decide_auto_verify_status(true, true, 9, 7, "");
        assert_eq!(s, "needs_review");
        let s = decide_auto_verify_status(true, true, 9, 7, "uncertain");
        assert_eq!(s, "needs_review");
    }
}

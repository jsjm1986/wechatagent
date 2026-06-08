//! 运营知识库路由：文档 / 切片 / 条目的全生命周期管理。

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
    llm::LlmProvider,
    models::{
        KnowledgeChatTurn, KnowledgeUsageLog, OperationKnowledgeChunk, OperationKnowledgeDocument,
    },
    prompts,
};

use super::shared::*;
use super::AppState;

// ── 模块化解耦（2026-06-07）：子域逐个搬运，建好一个解开一对 ────────────
// 见 docs/superpowers/plans/2026-06-07-knowledge-routes-split.md
mod crud;
mod verify;
mod import;
mod catalog;
mod repair;
mod chat;
mod digest_inbox;
mod wiki_edit;
mod sources_meta;
//
pub(in crate::routes) use crud::*;
pub use verify::*;
pub use import::*;
pub use catalog::*;
pub use repair::*;
pub use chat::*;
pub(in crate::routes) use digest_inbox::*;
pub use wiki_edit::*;
pub use sources_meta::*;

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
        "chunkType": item.chunk_type,
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

/// 调用方后门 D2 收口：create/PUT chunk 落库前，若调用方提交 `integrity_status="verified"`
/// 但缺 sourceQuote 或 source_anchors（未过 D2 闸），降级为 needs_review 并留审计痕迹。
/// 与 import 路径「锚点只作审核线索、最终 needs_review」语义一致；正路仍是走 /verify。
pub(in crate::routes) fn coerce_integrity_against_d2_gate(payload: &mut OperationKnowledgeChunkRequest) {
    if payload.integrity_status.as_deref() != Some("verified") {
        return;
    }
    let has_quote = payload
        .source_quote
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_anchor = !payload.source_anchors.is_empty();
    if chunk_verify_gate_reason(has_quote, has_anchor).is_some() {
        payload.integrity_status = Some("needs_review".to_string());
        payload.distortion_risks.push(
            "提交为 verified 但缺 sourceQuote/source_anchors，未过 D2 闸，已降级 needs_review"
                .to_string(),
        );
    }
}

// ── 知识库 AI 自主修复 ────────────────────────────────────────────
// propose / answer / apply handlers 及其 helper 已搬至 repair.rs。
// 仅保留下方共享常量与 truncate_for_prompt（被多个子域复用）。

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

/// 共享：写一条 AgentEvent。repair / chat / digest 多个子域复用，故留 mod.rs。
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

/// 把 serde_json::Value 转成 BSON Document（仅接受 object）。
fn bson_from_json(value: &Value) -> Result<Document, String> {
    if !value.is_object() {
        return Err("expected JSON object".to_string());
    }
    mongodb::bson::to_document(value).map_err(|e| e.to_string())
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


#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn coerce_d2_downgrades_verified_without_quote() {
        let mut p = OperationKnowledgeChunkRequest {
            title: "t".to_string(),
            integrity_status: Some("verified".to_string()),
            source_quote: None,
            source_anchors: vec![mongodb::bson::doc! { "startOffset": 0i64 }],
            ..Default::default()
        };
        coerce_integrity_against_d2_gate(&mut p);
        assert_eq!(p.integrity_status.as_deref(), Some("needs_review"));
        assert!(p.distortion_risks.iter().any(|r| r.contains("D2")));
    }

    #[test]
    fn coerce_d2_downgrades_verified_without_anchor() {
        let mut p = OperationKnowledgeChunkRequest {
            title: "t".to_string(),
            integrity_status: Some("verified".to_string()),
            source_quote: Some("原文引用".to_string()),
            source_anchors: vec![],
            ..Default::default()
        };
        coerce_integrity_against_d2_gate(&mut p);
        assert_eq!(p.integrity_status.as_deref(), Some("needs_review"));
    }

    #[test]
    fn coerce_d2_keeps_verified_with_quote_and_anchor() {
        let mut p = OperationKnowledgeChunkRequest {
            title: "t".to_string(),
            integrity_status: Some("verified".to_string()),
            source_quote: Some("原文引用".to_string()),
            source_anchors: vec![mongodb::bson::doc! { "startOffset": 0i64 }],
            ..Default::default()
        };
        coerce_integrity_against_d2_gate(&mut p);
        assert_eq!(p.integrity_status.as_deref(), Some("verified"));
        assert!(p.distortion_risks.is_empty());
    }

    #[test]
    fn coerce_d2_ignores_non_verified() {
        let mut p = OperationKnowledgeChunkRequest {
            title: "t".to_string(),
            integrity_status: Some("needs_review".to_string()),
            source_quote: None,
            source_anchors: vec![],
            ..Default::default()
        };
        coerce_integrity_against_d2_gate(&mut p);
        assert_eq!(p.integrity_status.as_deref(), Some("needs_review"));
        assert!(p.distortion_risks.is_empty());
    }
}

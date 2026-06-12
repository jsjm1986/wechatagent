//! 运营知识库目录/完整度/检索：catalog 构建 + completeness 审计 + integrity 报告 + 检索工具。

use axum::{
    extract::{Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent;
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};

use super::super::shared::*;
use super::super::AppState;
use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct KnowledgeToolSearchRequest {
    account_id: String,
    contact_id: Option<String>,
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct KnowledgeToolOpenRequest {
    ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct OperationKnowledgeTestRequest {
    account_id: String,
    contact_id: Option<String>,
    message: String,
}

pub(in crate::routes) async fn get_operation_knowledge_catalog(
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
pub(in crate::routes) async fn get_operation_knowledge_catalog_persisted(
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

pub(in crate::routes) async fn get_operation_knowledge_completeness(
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

pub(in crate::routes) async fn refresh_operation_knowledge_completeness(
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

pub(in crate::routes) async fn get_operation_knowledge_integrity_report(
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

pub(in crate::routes) async fn search_operation_knowledge_tool(
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

pub(in crate::routes) async fn open_operation_knowledge_slices(
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

pub(in crate::routes) async fn test_operation_knowledge_match(
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

pub(in crate::routes) async fn build_operation_knowledge_catalog(
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

/// universal-domain-adaptation H5-a：把 active DomainProfile 的 coverage 维度渲染成
/// completeness 审计 prompt 的 coverage JSON 骨架（每维一行三布尔位）。对齐规则逐字
/// 复刻原写死 prompt：`"{key}":` 后补空格使所有 `{` 对齐到本批最长 key（DEFAULT 销售
/// 域最长 = `"deliveryBoundary":`），故 DEFAULT 五维渲染结果与原字面量逐字一致；换行业
/// 维度按自身最长 key 对齐。空维度集返回空串（调用方 prompt 仍合法）。
fn build_coverage_skeleton(dims: &[crate::models::CoverageDimension]) -> String {
    let label = |k: &str| format!("\"{k}\":");
    let max_label = dims
        .iter()
        .map(|d| label(&d.key).chars().count())
        .max()
        .unwrap_or(0);
    dims.iter()
        .map(|d| {
            let lab = label(&d.key);
            let pad = " ".repeat(max_label.saturating_sub(lab.chars().count()));
            format!(
                "    {lab}{pad}{{ \"verifiedFact\": false, \"methodologyOnly\": false, \"pendingDraft\": false }}"
            )
        })
        .collect::<Vec<_>>()
        .join(",\n")
}

/// universal-domain-adaptation H5-b：把 active DomainProfile 的 coverage 维度的
/// `anchor_hint` 渲染成 completeness 审计 prompt 的「命中锚点」段（每维一行
/// `  - {key}：{hint}`）。逐字复刻原写死 prompt 的缩进/分隔；`anchor_hint=None` 的
/// 维度不产出行。DEFAULT 销售域五维 anchor_hint 逐字复刻原锚点 → prompt 字节等价。
fn build_coverage_anchors(dims: &[crate::models::CoverageDimension]) -> String {
    dims.iter()
        .filter_map(|d| {
            d.anchor_hint
                .as_ref()
                .map(|hint| format!("  - {}：{hint}", d.key))
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    // universal-domain-adaptation H5-a：completeness coverage 维度改读 active
    // DomainProfile.coverage_dimensions（替代写死的销售五维）。DEFAULT 销售域 profile
    // 逐字 seed capability/pricing/caseEvidence/effectClaims/deliveryBoundary →
    // 行为字节等价（命中初值规则 + prompt JSON 骨架下方按同序生成）。
    let active_profile =
        crate::agent::domain_profile::load_active_domain_profile(&state.db, workspace_id).await;
    // 维度初值规则（DEFAULT 销售域逐字复刻原 fallback）：capability / deliveryBoundary
    // 跟随「有 verified」、caseEvidence / effectClaims 跟随「有 evidence」、pricing 恒
    // false（报价默认缺）、未知维度（换行业新维）默认 false（保守缺失）。
    let dim_initial_verified = |key: &str| -> bool {
        match key {
            "capability" | "deliveryBoundary" => verified > 0,
            "caseEvidence" | "effectClaims" => evidence > 0,
            _ => false,
        }
    };
    let mut fallback_coverage = serde_json::Map::new();
    for dim in &active_profile.coverage_dimensions {
        fallback_coverage.insert(dim.key.clone(), cov_state(dim_initial_verified(&dim.key)));
    }
    let fallback = json!({
        "answeringMode": fallback_mode,
        "summary": if verified == 0 { "当前没有已验证知识切片，Agent 只能做关系维护和需求澄清。" } else { "当前存在已验证知识切片，Agent 可在证据边界内回答事实问题。" },
        "coverage": Value::Object(fallback_coverage),
        "gaps": fallback_gaps.clone()
    });
    let system = "你是企业用户运营知识库完整度 Auditor。你评估已验证知识是否足够支撑 Agent 回答产品/服务事实，并识别尚未审定的知识缺口，不负责生成销售内容。只输出严格 JSON。";
    // H5-a：coverage JSON 骨架按 active profile 的维度动态生成（替代写死五行）。
    // 见 [`build_coverage_skeleton`]：对齐规则逐字复刻原 prompt，DEFAULT 五维渲染
    // 结果与原字面量逐字一致。
    let coverage_skeleton = build_coverage_skeleton(&active_profile.coverage_dimensions);
    // H5-b：命中锚点散文按 active profile 维度的 anchor_hint 动态生成（替代写死五行）。
    // 见 [`build_coverage_anchors`]：DEFAULT 五维 anchor_hint 逐字复刻原 prompt 锚点 →
    // 销售域 prompt 字节等价；anchor_hint=None 的维度不产出锚点行。
    let coverage_anchors = build_coverage_anchors(&active_profile.coverage_dimensions);
    let user = format!(
        r#"请基于已验证知识切片与待审定切片输出 JSON：
{{
  "answeringMode": "relationship_only | product_safe | fully_supported",
  "summary": "",
  "coverage": {{
{coverage_skeleton}
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
  - "methodologyOnly": 该维度存在第 2 类「仅方法论/话术」内容时为 true（判定门槛见下方 methodologyOnly 防滥标条，**不要**把已含客观事实的切片也算进来）；
  - "pendingDraft": 该维度的具体信息存在于 needs_review 草稿中（第 3 类）时为 true。
  **关键：同一维度可以既 verifiedFact=true 又 pendingDraft=true**（例如企业版报价已审定为客观事实、旗舰版报价仍是未审定草稿，则 pricing 的 verifiedFact 与 pendingDraft 都为 true）。绝不能因为有 verified 事实就把 pendingDraft 抹成 false，也绝不能因为有草稿就把已具备的 verifiedFact 抹成 false——两个方向的漏标都要扣分。三位全 false 表示该维度缺失（第 4 类）。此判据对 pricing / caseEvidence / effectClaims / deliveryBoundary / capability 每一维都同样适用。
- 各 coverage 维度判 verifiedFact=true 的命中锚点（满足"已验证客观事实"时即应判 true，**不要漏判**）：
{coverage_anchors}
- methodologyOnly=true 的判定门槛（与 verifiedFact 对称，**防滥标**；通用原则，对每一维同样适用）：仅当该维度存在**以方法论/话术/价值主张/谈判策略为主体、且本身不含可对客客观事实（无具体数字/条款/案例数据/效果数字）**的独立 verified 切片时，才标 methodologyOnly=true。判定准则：
  - 一条切片若已含可核验的客观事实（即让该维度 verifiedFact=true 的那条），**不要**再因它顺带提到「怎么做/如何沟通/价值」就把同维度 methodologyOnly 也标 true——含客观事实的切片归 verifiedFact，**不重复**归 methodologyOnly。
  - 只有当某维度**除了**客观事实切片之外、**另有**一条纯方法论/话术切片，或该维度根本没有客观事实、只有方法论切片时，methodologyOnly 才为 true。
  - 拿不准某切片算「客观事实」还是「仅方法论」时，**优先归客观事实**（verifiedFact），methodologyOnly **从严**——宁可漏标 methodologyOnly，不可滥标导致与同维 verifiedFact 表意矛盾。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// H5-a 逐字等价护栏：DEFAULT_PROFILE 的 coverage 五维渲染出的 prompt 骨架，
    /// 必须与改造前写死的 5 行字面量逐字一致（含 `{` 对齐空格），保证 completeness
    /// 审计 prompt 在销售域字节不变。换行业 = 另一份 coverage_dimensions。
    #[test]
    fn coverage_skeleton_default_profile_byte_equivalent() {
        let p = crate::agent::domain_profile::default_domain_profile("ws-1");
        let got = build_coverage_skeleton(&p.coverage_dimensions);
        let expected = "    \"capability\":      { \"verifiedFact\": false, \"methodologyOnly\": false, \"pendingDraft\": false },\n    \"pricing\":         { \"verifiedFact\": false, \"methodologyOnly\": false, \"pendingDraft\": false },\n    \"caseEvidence\":    { \"verifiedFact\": false, \"methodologyOnly\": false, \"pendingDraft\": false },\n    \"effectClaims\":    { \"verifiedFact\": false, \"methodologyOnly\": false, \"pendingDraft\": false },\n    \"deliveryBoundary\":{ \"verifiedFact\": false, \"methodologyOnly\": false, \"pendingDraft\": false }";
        assert_eq!(got, expected);
    }

    /// H5-a：换行业 coverage 维度按自身最长 key 对齐，各维一行、逗号换行分隔。
    #[test]
    fn coverage_skeleton_custom_dims_align_to_longest() {
        let dims = vec![
            crate::models::CoverageDimension { key: "symptom".to_string(), display_name: "症状".to_string(), required: false, anchor_hint: None },
            crate::models::CoverageDimension { key: "treatmentPlan".to_string(), display_name: "治疗方案".to_string(), required: false, anchor_hint: None },
        ];
        let got = build_coverage_skeleton(&dims);
        // 最长 key = "treatmentPlan":（15 字符），symptom 行补到同宽。
        let expected = "    \"symptom\":      { \"verifiedFact\": false, \"methodologyOnly\": false, \"pendingDraft\": false },\n    \"treatmentPlan\":{ \"verifiedFact\": false, \"methodologyOnly\": false, \"pendingDraft\": false }";
        assert_eq!(got, expected);
    }

    /// H5-b 逐字等价护栏：DEFAULT_PROFILE 五维 anchor_hint 渲染出的「命中锚点」段，
    /// 必须与改造前写死的 5 行锚点散文逐字一致，保证 completeness 审计 prompt 在销售域
    /// 字节不变。换行业 = 另一份维度各自的 anchor_hint。
    #[test]
    fn coverage_anchors_default_profile_byte_equivalent() {
        let p = crate::agent::domain_profile::default_domain_profile("ws-1");
        let got = build_coverage_anchors(&p.coverage_dimensions);
        let expected = "  - capability：有 verified 切片陈述产品/服务\"能做什么\"的具体能力或功能事实。\n  - pricing：有 verified 切片含具体报价/计费/套餐金额（注意：仅 needs_review 草稿里的报价不计入 verifiedFact，而应置 pendingDraft=true 并入 gap）。\n  - caseEvidence：有 verified 切片描述**具体客户案例/实施成效**（含可核验的主体、场景或落地结果），即判 true。\n  - effectClaims：有 verified 切片含**可核验的效果数据/量化成果**（如转化率提升、响应时长变化等具体数字），即判 true。\n  - deliveryBoundary：有 verified 切片陈述交付方式/SLA/可用性/部署边界等具体条款。";
        assert_eq!(got, expected);
    }

    /// H5-b：anchor_hint=None 的维度不产出锚点行（只渲染有 hint 的维度）。
    #[test]
    fn coverage_anchors_skips_none_hint() {
        let dims = vec![
            crate::models::CoverageDimension { key: "a".to_string(), display_name: "A".to_string(), required: false, anchor_hint: Some("锚点A".to_string()) },
            crate::models::CoverageDimension { key: "b".to_string(), display_name: "B".to_string(), required: false, anchor_hint: None },
            crate::models::CoverageDimension { key: "c".to_string(), display_name: "C".to_string(), required: false, anchor_hint: Some("锚点C".to_string()) },
        ];
        let got = build_coverage_anchors(&dims);
        assert_eq!(got, "  - a：锚点A\n  - c：锚点C");
    }

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
}

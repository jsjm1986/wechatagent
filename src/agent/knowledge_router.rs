//! 运营知识库加载、Knowledge Router 与未验证告警 (MP-9)。
//!
//! - `load_operation_knowledge`：按 workspace + account 过滤拉取 documents /
//!   items / chunks（chunks 仅取 `integrity_status="verified"`）；
//! - `route_operation_knowledge`：调 Knowledge Tool Planner LLM，规划本轮
//!   要打开哪些文档/切片；
//! - `select_operation_knowledge*`、`route_used_knowledge_ids` 等是把
//!   Router 输出落到具体可注入 prompt 的切片；
//! - `format_operation_knowledge*` 系列把切片对人类/LLM 友好地格式化；
//! - `maybe_emit_unverified_warning`：当切片全部未通过校验时按当日去重写一条
//!   `knowledge_unverified_warning` 事件，避免运营人员困惑；
//! - `write_knowledge_usage_log`：把每次 run 的知识引用情况写入审计集合；
//! - `test_knowledge_route_for_contact`：后台知识库测试入口。

use futures::TryStreamExt;
use mongodb::bson::{doc, to_bson, to_document, Bson, DateTime, Document};
use mongodb::options::FindOptions;

use crate::error::AppResult;
use crate::models::{
    AgentStatus, Contact, ConversationMessage, KnowledgeUsageLog, MessageDirection,
    OperatingMemory, OperationKnowledgeChunk,
};
use crate::routes::AppState;

use super::gateway::write_event_for_account;
use super::generate_agent_json;
use super::memory::{
    default_memory_card, effective_memory_card_for_contact, load_or_create_operating_memory,
};
use super::types::{
    non_empty_option, AgentDecision, DecisionReviewResult, KnowledgeRouteResult, KnowledgeRuntime,
    RunPlannerResult,
};

pub(crate) async fn load_operation_knowledge(
    state: &AppState,
    contact: &Contact,
) -> AppResult<KnowledgeRuntime> {
    let account_filter = vec![
        doc! { "account_id": null },
        doc! { "account_id": &contact.account_id },
    ];
    let mut document_cursor = state
        .db
        .operation_knowledge_documents()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "status": "active",
                "$or": account_filter.clone()
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(80)
                .build(),
        )
        .await?;
    let mut documents = Vec::new();
    while let Some(item) = document_cursor.try_next().await? {
        documents.push(item);
    }
    let mut chunk_cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
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
        chunks.push(item);
    }
    Ok(KnowledgeRuntime { documents, chunks })
}

/// MP-9 / Task 16：检测 verified chunks 为 0 但 chunks 总数 > 0 的情况，
/// 并在当日按 contact 去重写一条 `knowledge_unverified_warning` event。
///
/// 由 [`super::gateway::run_user_operation_gateway_inner`] 在加载知识库后
/// 调用。失败被静默（不影响主流程）。
pub(crate) async fn maybe_emit_unverified_warning(
    state: &AppState,
    contact: &Contact,
) -> AppResult<()> {
    // 直接在 chunks 集合做 count，避免重复加载已经过滤后的 KnowledgeRuntime。
    let total = state
        .db
        .operation_knowledge_chunks()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "$or": [
                    { "account_id": null },
                    { "account_id": &contact.account_id }
                ]
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    if total == 0 {
        return Ok(());
    }
    let verified = state
        .db
        .operation_knowledge_chunks()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "integrity_status": "verified",
                "$or": [
                    { "account_id": null },
                    { "account_id": &contact.account_id }
                ]
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    if verified > 0 {
        return Ok(());
    }
    // 当日按 contact 去重。
    let day_start_ms = today_start_millis();
    let exists = state
        .db
        .events()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "kind": "knowledge_unverified_warning",
                "created_at": { "$gte": DateTime::from_millis(day_start_ms) }
            },
            None,
        )
        .await
        .ok()
        .flatten();
    if exists.is_some() {
        return Ok(());
    }
    let _ = write_event_for_account(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        "knowledge_unverified_warning",
        "warn",
        "知识库存在切片但全部未通过校验，运行时不会注入；请运行 auto-verify 或人工核查",
        Some(doc! {
            "totalChunks": total as i32,
            "verifiedChunks": verified as i32
        }),
    )
    .await;
    Ok(())
}

fn today_start_millis() -> i64 {
    let now = DateTime::now().timestamp_millis();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    now - (now.rem_euclid(day_ms))
}

pub(crate) fn format_operation_knowledge_catalog_for_prompt(runtime: &KnowledgeRuntime) -> String {
    let documents = runtime
        .documents
        .iter()
        .map(|item| {
            format!(
                "- documentId={} title={}\n  catalog={}\n  routingMap={}",
                item.id.map(|id| id.to_hex()).unwrap_or_default(),
                item.title,
                item.catalog_summary
                    .clone()
                    .or(item.summary.clone())
                    .unwrap_or_default(),
                item.routing_map.join(" / ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = runtime
        .chunks
        .iter()
        .take(120)
        .map(|item| {
            format!(
                "- chunkId={} type={} title={}\n  summary={}",
                item.id.map(|id| id.to_hex()).unwrap_or_default(),
                item.knowledge_type.clone().unwrap_or_default(),
                item.title,
                item.summary.clone().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("文档目录:\n{}\n\n切片目录:\n{}", documents, chunks)
}

pub(crate) fn format_operation_knowledge_for_prompt(
    chunks: &[OperationKnowledgeChunk],
) -> String {
    let chunk_text = chunks
        .iter()
        .map(|item| {
            format!(
                "- chunkId={} type={} context={} title={}\n  integrityStatus={} confidence={}\n  summary={}\n  body={}\n  sourceAnchors={}\n  sourceQuote={}",
                item.id.map(|id| id.to_hex()).unwrap_or_default(),
                item.knowledge_type.clone().unwrap_or_default(),
                item.business_context.clone().unwrap_or_default(),
                item.title,
                item.integrity_status.clone().unwrap_or_default(),
                item.confidence_score.unwrap_or_default(),
                item.summary.clone().unwrap_or_default(),
                item.body.clone().unwrap_or_default(),
                serde_json::to_string(&item.source_anchors).unwrap_or_default(),
                item.source_quote.clone().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("已打开知识切片:\n{}", chunk_text)
}

pub async fn test_knowledge_route_for_contact(
    state: &AppState,
    contact: Option<Contact>,
    account_id: &str,
    message: &str,
) -> AppResult<Document> {
    let has_persisted_contact = contact.is_some();
    let contact = contact.unwrap_or_else(|| Contact {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: account_id.to_string(),
        wxid: "preview".to_string(),
        nickname: Some("知识命中测试".to_string()),
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        tags: Vec::new(),
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: Some("new_contact".to_string()),
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    });
    let inbound = ConversationMessage {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some("knowledge-test".to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: message.trim().to_string(),
        raw: Some(doc! { "runMode": "knowledge_test" }),
        created_at: DateTime::now(),
    };
    let memory = if has_persisted_contact {
        load_or_create_operating_memory(state, &contact)
            .await
            .unwrap_or_else(|_| OperatingMemory {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                user_understanding: Document::new(),
                relationship_state: Document::new(),
                product_fit: Document::new(),
                next_action: Document::new(),
                context_pack: Document::new(),
                context_pack_version: 0,
                context_pack_updated_at: None,
                // task 6.3：直接使用 typed 默认值，不再走 Document → from_document
                // 兼容路径。
                memory_card: default_memory_card(),
                memory_card_version: 0,
                memory_card_updated_at: None,
                created_at: DateTime::now(),
                updated_at: DateTime::now(),
            })
    } else {
        OperatingMemory {
            id: None,
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            user_understanding: Document::new(),
            relationship_state: Document::new(),
            product_fit: Document::new(),
            next_action: Document::new(),
            context_pack: Document::new(),
            context_pack_version: 0,
            context_pack_updated_at: None,
            memory_card: default_memory_card(),
            memory_card_version: 0,
            memory_card_updated_at: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    };
    let knowledge = load_operation_knowledge(state, &contact).await?;
    // task 6.3：边界处把 typed 转为 Document wire shape，下游 prompt 注入路径不变。
    let memory_card = effective_memory_card_for_contact(&memory, &contact).to_document();
    let route = route_operation_knowledge(
        state,
        &contact,
        &inbound,
        &[],
        &memory,
        &memory_card,
        &knowledge,
        None,
    )
    .await?;
    let selected_chunks = select_operation_knowledge_chunks(&knowledge.chunks, &route);
    Ok(doc! {
        "route": to_document(&route).unwrap_or_default(),
        "selectedChunks": selected_chunks.into_iter().map(operation_knowledge_chunk_to_bson).collect::<Vec<_>>()
    })
}

pub(crate) async fn route_operation_knowledge(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    recent_messages: &[ConversationMessage],
    memory: &OperatingMemory,
    context_pack: &Document,
    knowledge: &KnowledgeRuntime,
    run_id: Option<&str>,
) -> AppResult<KnowledgeRouteResult> {
    if knowledge.documents.is_empty() && knowledge.chunks.is_empty() {
        return Ok(KnowledgeRouteResult {
            risk_level: "medium".to_string(),
            knowledge_coverage: "missing".to_string(),
            reason: "没有可用运营知识库".to_string(),
            ..Default::default()
        });
    }
    // ── 硬关键词快路径 (WB5) ─────────────────────────────────────────────
    // Reply Agent 永远先跑知识路由；这里在 LLM planner 之前先做一遍
    // trigger_keywords 子串匹配（大小写不敏感）。命中即在 tool_trace 上写
    // `keyword_fastpath_hit:<chunkId>:<keyword>`，并把命中 chunk 强制塞进
    // selected_chunk_ids 头部。gateway 据此覆盖 conversation_mode=consultative。
    let fastpath_hits = compute_keyword_fastpath_hits(&inbound.content, &knowledge.chunks);
    let catalog = format_operation_knowledge_catalog_for_prompt(knowledge);
    let memory_text = serde_json::to_string(&doc! {
        "memoryCard": context_pack.clone(),
        "relationshipState": memory.relationship_state.clone(),
        "productFit": memory.product_fit.clone(),
        "nextAction": memory.next_action.clone()
    })
    .unwrap_or_default();
    let history = recent_messages
        .iter()
        .rev()
        .take(8)
        .map(|message| {
            let speaker = match message.direction {
                MessageDirection::Inbound => "客户",
                MessageDirection::Outbound => "我方",
            };
            format!("{speaker}: {}", message.content)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let system = "你是微信用户运营 Knowledge Tool Planner。你要像 Agent 工具调用一样，先阅读知识目录，再决定本轮应该打开哪些文档、知识包、切片和证据。不负责回复客户。必须只输出严格 JSON。";
    let user = format!(
        r#"请根据用户当前消息、长期记忆和知识目录，规划本轮用户运营 Agent 的知识工具调用。
输出 JSON：
{{
  "neededCategories": ["自然语言说明需要哪类知识，不要使用固定枚举"],
  "selectedDocumentIds": [],
  "selectedKnowledgeIds": [],
  "selectedChunkIds": [],
  "selectedSliceReasons": [],
  "riskLevel": "low | medium | high",
  "requiresEvidence": false,
  "knowledgeCoverage": "enough | weak | missing",
  "missingKnowledge": [],
  "reason": "",
  "toolTrace": [
    {{
      "tool": "knowledge.list_catalog",
      "reason": "先阅读目录"
    }},
    {{
      "tool": "knowledge.open_slice",
      "ids": [],
      "reason": "说明为什么打开这些切片"
    }}
  ]
}}

规则：
- 只从目录里的 documentId、itemId、chunkId 选择。
- 优先选择 chunkId，因为切片才是运行时应打开的最小知识单元。
- selectedKnowledgeIds 最多 4 个，selectedChunkIds 最多 8 个，selectedDocumentIds 最多 3 个。
- 涉及产品能力、价格、案例、效果、交付承诺时 requiresEvidence=true。
- requiresEvidence=true 时必须优先选择有 evidence 的切片；没有证据时 knowledgeCoverage=weak 或 missing。
- 没有足够知识时 knowledgeCoverage=missing 或 weak，不要硬选无关知识。
- 不要按关键词机械匹配；要结合用户阶段、长期记忆、当前语义和风险判断。

客户昵称: {}
客户阶段: {}
运营状态: {}
最近聊天:
{}

长期记忆卡片:
{}

运营记忆:
{}

用户最新消息:
{}

知识目录与可打开切片:
{}"#,
        contact.nickname.clone().unwrap_or_default(),
        contact
            .domain_attributes
            .as_ref()
            .and_then(|doc| doc.get_str("customer_stage").ok().map(|s| s.to_string()))
            .unwrap_or_default(),
        contact.operation_state.clone().unwrap_or_default(),
        history,
        serde_json::to_string(context_pack).unwrap_or_default(),
        memory_text,
        inbound.content,
        catalog
    );
    let value = generate_agent_json(
        state,
        Some(&contact.account_id),
        Some(&contact.wxid),
        run_id,
        "user.knowledge.router",
        system,
        &user,
    )
    .await?;
    let mut route: KnowledgeRouteResult = serde_json::from_value(value)?;
    route.selected_document_ids = route
        .selected_document_ids
        .into_iter()
        .filter(|id| {
            knowledge.documents.iter().any(|item| {
                item.id.map(|object_id| object_id.to_hex()).as_deref() == Some(id.as_str())
            })
        })
        .take(3)
        .collect();
    route.selected_knowledge_ids = Vec::new();
    route.selected_chunk_ids = route
        .selected_chunk_ids
        .into_iter()
        .filter(|id| {
            knowledge.chunks.iter().any(|item| {
                item.id.map(|object_id| object_id.to_hex()).as_deref() == Some(id.as_str())
            })
        })
        .take(8)
        .collect();
    let has_catalog_trace = route
        .tool_trace
        .iter()
        .any(|item| item.get_str("tool") == Ok("knowledge.list_catalog"));
    if !has_catalog_trace {
        route.tool_trace.insert(
            0,
            doc! {
                "tool": "knowledge.list_catalog",
                "documents": knowledge.documents.len() as i32,
                "chunks": knowledge.chunks.len() as i32
            },
        );
    }
    let has_open_trace = route
        .tool_trace
        .iter()
        .any(|item| item.get_str("tool") == Ok("knowledge.open_slice"));
    if !route.selected_chunk_ids.is_empty() && !has_open_trace {
        route.tool_trace.push(doc! {
            "tool": "knowledge.open_slice",
            "ids": route.selected_chunk_ids.clone(),
            "reason": route.reason.clone()
        });
    }
    if route.knowledge_coverage.trim().is_empty() {
        route.knowledge_coverage =
            if route.selected_knowledge_ids.is_empty() && route.selected_chunk_ids.is_empty() {
                "missing".to_string()
            } else {
                "enough".to_string()
            };
    }
    if route.risk_level.trim().is_empty() {
        route.risk_level = if route.requires_evidence {
            "high".to_string()
        } else {
            "medium".to_string()
        };
    }
    // ── WB5 fastpath 合并 ───────────────────────────────────────────────
    // 把硬关键词命中的 chunk 强制塞进 selected_chunk_ids 头部（去重保留 LLM 排序），
    // 并在 tool_trace 上写 keyword_fastpath_hit 行，gateway 据此覆盖
    // conversation_mode=consultative。即使 LLM planner 没有选中这些 chunk，
    // fastpath 也会把它们补回来。
    if !fastpath_hits.is_empty() {
        let mut prepend: Vec<String> = Vec::new();
        for (chunk_id, _kw) in &fastpath_hits {
            if !route.selected_chunk_ids.iter().any(|existing| existing == chunk_id)
                && !prepend.iter().any(|existing| existing == chunk_id)
            {
                prepend.push(chunk_id.clone());
            }
        }
        if !prepend.is_empty() {
            let mut merged = prepend;
            merged.extend(route.selected_chunk_ids.drain(..));
            // 截断到 8 (与 LLM planner 同)，避免 selected_chunk_ids 超长。
            if merged.len() > 8 {
                merged.truncate(8);
            }
            route.selected_chunk_ids = merged;
        }
        for (chunk_id, kw) in &fastpath_hits {
            route.tool_trace.push(doc! {
                "tool": "knowledge.keyword_fastpath",
                "label": format!("keyword_fastpath_hit:{}:{}", chunk_id, kw),
                "chunkId": chunk_id.clone(),
                "keyword": kw.clone()
            });
        }
        if route.knowledge_coverage == "missing" {
            route.knowledge_coverage = "weak".to_string();
        }
    }
    route.tool_trace = dedupe_tool_trace(route.tool_trace);
    Ok(route)
}

/// WB5：硬关键词快路径的纯函数版（不依赖 AppState / LLM / Mongo）。
///
/// 对每个 chunk 的 `trigger_keywords` 做大小写不敏感子串匹配；命中即记录
/// `(chunk_id, matched_keyword)`，每个 chunk 最多记一次。无 id 的 chunk
/// 直接跳过。inbound 仅含空白时返回空。
///
/// 主路径 [`route_operation_knowledge`] 在 LLM planner 之前调用本函数；
/// 也对外暴露给独立 crate 的 `tests/keyword_fastpath_router.rs` 单测。
pub fn compute_keyword_fastpath_hits(
    inbound_content: &str,
    chunks: &[OperationKnowledgeChunk],
) -> Vec<(String, String)> {
    let lower_inbound = inbound_content.to_lowercase();
    if lower_inbound.trim().is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(String, String)> = Vec::new();
    for chunk in chunks {
        let Some(chunk_id) = chunk.id.map(|oid| oid.to_hex()) else {
            continue;
        };
        for kw in &chunk.trigger_keywords {
            let normalized = kw.trim().to_lowercase();
            if normalized.is_empty() {
                continue;
            }
            if lower_inbound.contains(&normalized) {
                hits.push((chunk_id.clone(), kw.clone()));
                break;
            }
        }
    }
    hits
}

/// WB5：判断本轮知识路由是否命中硬关键词快路径。
/// gateway 据此把 conversation_mode 强制覆盖为 consultative。
pub(crate) fn knowledge_route_has_keyword_fastpath_hit(route: &KnowledgeRouteResult) -> bool {
    route
        .tool_trace
        .iter()
        .any(|item| item.get_str("tool") == Ok("knowledge.keyword_fastpath"))
}

fn dedupe_tool_trace(items: Vec<Document>) -> Vec<Document> {
    let mut seen = Vec::new();
    let mut output = Vec::new();
    for item in items {
        let key = format!(
            "{}:{}",
            item.get_str("tool").unwrap_or_default(),
            item.get("ids")
                .map(|value| value.to_string())
                .unwrap_or_default()
        );
        if !seen.iter().any(|existing| existing == &key) {
            seen.push(key);
            output.push(item);
        }
    }
    output
}

pub(crate) fn empty_knowledge_route(planner: &RunPlannerResult) -> KnowledgeRouteResult {
    KnowledgeRouteResult {
        risk_level: planner.risk_level.clone(),
        knowledge_coverage: "not_required".to_string(),
        reason: format!("Reply Agent 判断本轮无需打开知识库：{}", planner.reason),
        tool_trace: vec![doc! {
            "tool": "knowledge.skip",
            "reason": planner.reason.clone()
        }],
        ..Default::default()
    }
}

pub(crate) fn route_used_knowledge_ids(route: &KnowledgeRouteResult) -> Vec<String> {
    route
        .selected_knowledge_ids
        .iter()
        .chain(route.selected_chunk_ids.iter())
        .cloned()
        .collect()
}

pub(crate) fn select_operation_knowledge_chunks(
    chunks: &[OperationKnowledgeChunk],
    route: &KnowledgeRouteResult,
) -> Vec<OperationKnowledgeChunk> {
    route
        .selected_chunk_ids
        .iter()
        .filter_map(|id| {
            chunks.iter().find(|item| {
                item.id.map(|object_id| object_id.to_hex()).as_deref() == Some(id.as_str())
            })
        })
        .cloned()
        .collect::<Vec<_>>()
}

fn operation_knowledge_chunk_to_bson(item: OperationKnowledgeChunk) -> Bson {
    to_bson(&doc! {
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "documentId": item.document_id.map(|id| id.to_hex()),
        "knowledgeType": item.knowledge_type,
        "businessContext": item.business_context,
        "title": item.title,
        "summary": item.summary,
        "body": item.body,
        "sourceQuote": item.source_quote,
        "sourceAnchors": item.source_anchors,
        "integrityStatus": item.integrity_status,
        "confidenceScore": item.confidence_score,
        "status": item.status,
        "updatedAt": item.updated_at
    })
    .unwrap_or(Bson::Null)
}

pub(crate) async fn write_knowledge_usage_log(
    state: &AppState,
    contact: &Contact,
    decision: &AgentDecision,
    review: &DecisionReviewResult,
    route: &KnowledgeRouteResult,
    approved: bool,
    run_id: &str,
) -> AppResult<()> {
    let ids = route
        .selected_knowledge_ids
        .iter()
        .chain(route.selected_chunk_ids.iter())
        .filter_map(|id| mongodb::bson::oid::ObjectId::parse_str(id).ok())
        .collect::<Vec<_>>();
    state
        .db
        .knowledge_usage_logs()
        .insert_one(
            KnowledgeUsageLog {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: Some(contact.wxid.clone()),
                run_id: run_id.to_string(),
                knowledge_ids: ids,
                route_result: to_document(route).unwrap_or_default(),
                reply_text: non_empty_option(&Some(decision.reply_text.clone())),
                review_approved: approved,
                blocked_reason: if approved {
                    None
                } else {
                    non_empty_option(&Some(review.review_summary.clone()))
                },
                tool_trace: route.tool_trace.clone(),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    // knowledge-wiki §6.1：每次 run 把命中/拦截原子写回 chunk.usage_stats，
    // 让 catalog/persisted 的排序与 feedback worker 的 dynamic_confidence 拿到
    // 实时计数。fire-and-forget——不阻塞 gateway 决策。
    let block_reason = if approved {
        None
    } else {
        Some(review.review_summary.clone())
    };
    for hex_id in route
        .selected_knowledge_ids
        .iter()
        .chain(route.selected_chunk_ids.iter())
    {
        let _ = crate::knowledge_wiki::gap_signals::record_chunk_hit(
            &state.db,
            hex_id,
            !approved,
            block_reason.as_deref(),
        )
        .await;
    }
    Ok(())
}

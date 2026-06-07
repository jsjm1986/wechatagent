//! 运营知识库导入/摄取：preview/apply + PDF/图像多模态 + RSS/HTML 分块落库 + 标签抽取。

use axum::{Extension, Json};
use axum::extract::State;
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::agent;
use crate::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, ProvenanceSource, RevisionOp, RevisionRequest,
};

use super::super::AppState;
use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationKnowledgeImportRequest {
    pub(super) account_id: Option<String>,
    pub(super) source_name: Option<String>,
    pub(super) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct OperationKnowledgeImportApplyRequest {
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

pub(in crate::routes) async fn import_operation_knowledge_apply(
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

pub(in crate::routes) async fn import_operation_knowledge_apply_pdf(
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

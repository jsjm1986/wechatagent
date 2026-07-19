//! 运营知识库导入/摄取：preview/apply + PDF/图像多模态 + RSS/HTML 分块落库 + 标签抽取。

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use futures::stream::{self, StreamExt};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent;
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, ProvenanceSource, RevisionOp, RevisionRequest,
};
use crate::models::{assert_import_job_status_valid, ImportJob};

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

/// 长文本导入抽取 prompt 的静态模板（user 消息）。
/// `{SOURCE_NAME}` / `{CONTENT}` 两个占位符在运行时用 `.replace()` 填充。
/// 抽成模块级 const 以便字符串锁定测试断言其内容（`format!` 需要字面量格式串，
/// 无法直接取到运行时局部变量里的模板文本）。
const LONG_IMPORT_PROMPT_TEMPLATE: &str = r#"请把下面文本拆分为渐进式运营知识。输出 JSON：
{
  "document": {
    "domain": "user_operations",
    "sourceType": "imported_markdown",
    "sourceName": "{SOURCE_NAME}",
    "title": "",
    "summary": "",
    "catalogSummary": "给 Agent 看的目录摘要，说明这份文档解决什么问题、何时应该打开",
    "routingMap": ["自然语言目录项，不使用固定分类"],
    "riskNotes": ["不能承诺、证据不足或需要 admin 后台确认的风险点"],
    "productTags": ["产品/品牌/解决方案名称，最多 5 个，可空"],
    "businessTopics": ["业务主题（如 产品定位差异 / 竞品对比 / 部署方式），最多 3 个，可空"],
    "status": "draft"
  },
  "items": [
    {
      "domain": "user_operations",
      "category": "用自然语言生成的主题标签，不要使用固定枚举",
      "businessType": "用自然语言说明业务语境，不要使用固定枚举",
      "knowledgeType": "AI 自主生成的知识类型",
      "businessContext": "这条知识适合的业务上下文",
      "title": "",
      "summary": "",
      "body": "",
      "applicableScenes": [],
      "notApplicableScenes": [],
      "productTags": ["最多 5 个，可空"],
      "businessTopics": ["最多 3 个，可空"],
      "sourceType": "imported_markdown",
      "sourceName": "{SOURCE_NAME}",
      "status": "draft",
      "priority": 0
    }
  ],
  "chunks": [
    {
      "domain": "user_operations",
      "wikiType": "9 类之一：source/entity/concept/comparison/synthesis/methodology/finding/query/thesis。按知识形态选：有步骤/分支的方法→methodology；具体数据点/案例事实→finding；纯定义→concept；多源综述→synthesis；带论据的判断/主张→thesis；FAQ→query；单一实体→entity；原始出处→source；对比→comparison",
      "chunkType": "4 类之一：product_fact（可对客户承诺的产品事实，需核验背书）/ style_template（语气模板）/ peer_case（同行案例参考，不作产品承诺）/ negative_example（不该做的反例）。绝大多数产品/服务事实类知识填 product_fact",
      "knowledgeType": "AI 自主生成的切片类型",
      "businessContext": "业务上下文",
      "title": "",
      "summary": "",
      "body": "可被 Agent 按需打开的原文要点或经过整理的知识正文",
      "applicableScenes": [],
      "notApplicableScenes": [],
      "productTags": ["如：WechatAgent / AI 私域销售助手；最多 5 个；可空"],
      "businessTopics": ["如：产品定位差异 / 竞品对比；最多 3 个；可空"],
      "sourceQuote": "如有必要，保留支撑该切片的原文短句",
      "status": "draft",
      "priority": 0
    }
  ]
}

要求：
- 不要用固定枚举分类；知识类型、适用场景、目录项都用自然语言生成。
- document 是整篇资料的目录入口；items 是主题包；chunks 是 Agent 运行时真正按需打开的知识切片。
- 穷尽且忠实抽取：原文中每一个量化事实（数字/比例/金额/期限/数量）及其**限定条件**（起售门槛、前置要求、适用范围、例外、有效期等）都必须落入对应 chunk 的 body，**绝不能丢掉限定条件**只留主数字（例："X 元起，含 N 个起"必须连"含 N 个起"一起保留）。一条原子承载一个规格/事实时尤其要完整。
- 穷尽覆盖的对象不止量化事实：原文里每一个**离散信息单元**都要落地，不要因为它没有数字就漏掉。离散信息单元包括但不限于——决议/结论、动作项/待办及其**责任人与截止日期**、分项条款、流程步骤、各方观点、适用与不适用条件。例如会议纪要类文档，每一条决议、每一项待办（连同谁负责、何时完成）都必须各自落入 body，绝不能只总结成一句"会上讨论了若干事项"。判断标准：原文每一个可独立成立、能被单独追溯核对的陈述，都应在抽取结果里找得到对应内容。
- 只忠于原文：body、summary 只能包含原文已陈述的内容，**禁止补充原文没有的描述、范围、功能、优惠条件或推断**。拿不准是否在原文里，就不写。
- 案例、报价、效果数据必须完整落入对应 chunk 的 body；没有证据不要编造成案例。
- productTags / businessTopics 用于运行时把用户消息匹配到对应 chunk。
- document 级 productTags / businessTopics 可以是其下所有 chunks 的去重并集，也可由 LLM 自行抽取。

导入文本：
{CONTENT}"#;

/// 单次调用上限（按 char 计，适配中文）：内容 ≤ 此值走单段路径，与分块前字节等价（零回归）。
const IMPORT_SINGLE_CALL_MAX_CHARS: usize = 3000;
/// 贪心打包目标：连续标题块累加到此值就断开成一段。
const IMPORT_SEGMENT_TARGET_CHARS: usize = 3000;
/// 单块硬上限：超此值的原子块（如一个巨型小节 / 无标题长文）按段落再切。
const IMPORT_SEGMENT_HARD_MAX_CHARS: usize = 5000;
/// 每段抽取并发度：匹配生产端点真实 ~2 线程，避免 tool_use 争用。
const IMPORT_EXTRACT_CONCURRENCY: usize = 2;

/// 把长文档确定性切分为多段，每段随后独立调 LLM 抽取（输出小、不截断）。
///
/// 策略（标题优先 + 字符回退）：
/// 1. 总 char ≤ SINGLE_MAX → 单段返回（零回归路径）。
/// 2. 否则按 markdown 标题行（`#` 开头，对齐 `build_section_index`）切成原子块。
/// 3. 贪心打包相邻块到 TARGET；单块超 HARD_MAX → 先 flush 累积段，再按段落窗口切该块。
/// 4. 无标题的纯长文 → 步骤 2 得单块 → 走步骤 3 的段落窗口兜底。
/// 5. 结果为空 → 兜底整篇单段。
pub(super) fn split_import_content(content: &str) -> Vec<String> {
    if content.chars().count() <= IMPORT_SINGLE_CALL_MAX_CHARS {
        return vec![content.to_string()];
    }
    // 按标题行切原子块：标题行开启一个新块，标题前的前言归入第一个块。
    let mut atoms: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in content.split_inclusive('\n') {
        if line.trim_start().starts_with('#') && !current.is_empty() {
            atoms.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        atoms.push(current);
    }

    let mut segments: Vec<String> = Vec::new();
    let mut acc = String::new();
    for atom in atoms {
        if atom.chars().count() > IMPORT_SEGMENT_HARD_MAX_CHARS {
            // 先 flush 已累积段，再把这个超大块按段落窗口切开。
            if !acc.trim().is_empty() {
                segments.push(std::mem::take(&mut acc));
            } else {
                acc.clear();
            }
            segments.extend(split_oversized_by_paragraph(&atom));
            continue;
        }
        if !acc.is_empty()
            && acc.chars().count() + atom.chars().count() > IMPORT_SEGMENT_TARGET_CHARS
        {
            segments.push(std::mem::take(&mut acc));
        }
        acc.push_str(&atom);
    }
    if !acc.trim().is_empty() {
        segments.push(acc);
    }
    segments.retain(|s| !s.trim().is_empty());
    if segments.is_empty() {
        return vec![content.to_string()];
    }
    segments
}

/// 把超过 HARD_MAX 的单块按段落边界（`\n\n`）打包成 ≤ TARGET 的窗口，
/// 绝不在句子中间断开。单个段落本身就超 HARD_MAX 时整段独立成窗口（不再硬切字符，
/// 保持语义完整——极端超长段落交给 LLM，仍比整篇小得多）。
fn split_oversized_by_paragraph(block: &str) -> Vec<String> {
    let mut windows: Vec<String> = Vec::new();
    let mut acc = String::new();
    for para in block.split_inclusive("\n\n") {
        if !acc.is_empty()
            && acc.chars().count() + para.chars().count() > IMPORT_SEGMENT_TARGET_CHARS
        {
            windows.push(std::mem::take(&mut acc));
        }
        acc.push_str(para);
    }
    if !acc.trim().is_empty() {
        windows.push(acc);
    }
    windows.retain(|s| !s.trim().is_empty());
    if windows.is_empty() {
        return vec![block.to_string()];
    }
    windows
}

/// 合并多段 LLM 抽取出的 document 原始值（标量取首个非空；数组字段取并集去重）。
/// rawContent / lineIndex / sectionIndex 不在此处理——由
/// `normalize_operation_knowledge_preview_document` 从完整 `payload.content` 重算。
fn merge_preview_documents(docs: &[Value]) -> Option<Value> {
    if docs.is_empty() {
        return None;
    }
    let first_str = |key: &str| -> Option<String> {
        docs.iter()
            .find_map(|d| json_string(d, key).filter(|s| !s.trim().is_empty()))
    };
    let union_list = |camel: &str, snake: &str| -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for d in docs {
            let list = json_string_list(d, camel)
                .or_else(|| json_string_list(d, snake))
                .unwrap_or_default();
            for item in list {
                if seen.insert(item.clone()) {
                    out.push(item);
                }
            }
        }
        out
    };
    // summary / catalogSummary 拼接各段非空值（各段视角不同，拼接比取首个信息更全）。
    let join_nonempty = |camel: &str, snake: &str| -> String {
        docs.iter()
            .filter_map(|d| {
                json_string(d, camel)
                    .or_else(|| json_string(d, snake))
                    .filter(|s| !s.trim().is_empty())
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Some(json!({
        "domain": first_str("domain"),
        "sourceType": first_str("sourceType").or_else(|| first_str("source_type")),
        "sourceName": first_str("sourceName").or_else(|| first_str("source_name")),
        "title": first_str("title"),
        "summary": join_nonempty("summary", "summary"),
        "catalogSummary": join_nonempty("catalogSummary", "catalog_summary"),
        "routingMap": union_list("routingMap", "routing_map"),
        "riskNotes": union_list("riskNotes", "risk_notes"),
        "productTags": union_list("productTags", "product_tags"),
        "businessTopics": union_list("businessTopics", "business_topics"),
        "status": first_str("status"),
    }))
}

pub async fn import_operation_knowledge_preview(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<OperationKnowledgeImportRequest>,
) -> AppResult<Json<Value>> {
    if payload.content.trim().is_empty() {
        return Err(AppError::BadRequest("content is required".to_string()));
    }
    // 小文档（≤ SINGLE_MAX，单段）→ 原样同步秒回，与今天字节等价（零回归）。
    if payload.content.chars().count() <= IMPORT_SINGLE_CALL_MAX_CHARS {
        let result = run_import_extraction(&state, &payload, None).await?;
        return Ok(Json(result));
    }
    // 大文档 → 建 import_jobs（pending），返回 jobId 交由 import_worker 异步跑，前端轮询。
    let segments_total = count_import_segments(&payload.content) as i32;
    let now = DateTime::now();
    assert_import_job_status_valid("pending");
    let job = ImportJob {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: payload.account_id.clone(),
        source_name: payload
            .source_name
            .clone()
            .unwrap_or_else(|| "导入文本".to_string()),
        content: payload.content.clone(),
        segments_total,
        progress_done: 0,
        progress_succeeded: 0,
        progress_failed: 0,
        status: "pending".to_string(),
        result: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        expires_at: None,
        created_at: now,
        updated_at: now,
    };
    let inserted = state.db.import_jobs().insert_one(&job, None).await?;
    let job_id = inserted
        .inserted_id
        .as_object_id()
        .map(|oid| oid.to_hex())
        .unwrap_or_default();
    Ok(Json(json!({
        "jobId": job_id,
        "async": true,
        "segmentsTotal": segments_total,
    })))
}

/// GET `/operation-knowledge/import-preview-job/:id`：前端每 ~2s 轮询导入 job 进度。
/// IDOR 收口：只返回属当前 workspace 的 job（仿现有 admin handler workspace 隔离）。
pub async fn get_import_preview_job(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let job_id =
        ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("invalid job id".to_string()))?;
    let job = state
        .db
        .import_jobs()
        .find_one(
            doc! { "_id": job_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("import job not found".to_string()))?;
    Ok(Json(import_job_progress_json(&job)))
}

/// GET `/operation-knowledge/import-preview-jobs?status=running`：本 workspace 进行中
/// job 列表（跨会话/跨设备发现用，不依赖 localStorage）。默认返回 running。
pub async fn list_import_preview_jobs(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ImportJobListQuery>,
) -> AppResult<Json<Value>> {
    let status = query.status.unwrap_or_else(|| "running".to_string());
    assert_import_job_status_valid(&status);
    let filter = doc! { "workspace_id": &admin.current_workspace, "status": &status };
    // 列表按最新在前。
    let opts = FindOptions::builder()
        .sort(doc! { "created_at": -1 })
        .limit(50)
        .build();
    let mut cursor = state.db.import_jobs().find(filter, opts).await?;
    let mut jobs = Vec::new();
    while let Some(job) = cursor.try_next().await? {
        jobs.push(import_job_progress_json(&job));
    }
    Ok(Json(json!({ "jobs": jobs })))
}

#[derive(Debug, Deserialize)]
pub struct ImportJobListQuery {
    status: Option<String>,
}

/// 把 job 投影成前端轮询用的 camelCase 进度 json。完成时带 `result`（同步 preview
/// 响应体），失败时带 `error`。
fn import_job_progress_json(job: &ImportJob) -> Value {
    json!({
        "jobId": job.id.map(|oid| oid.to_hex()).unwrap_or_default(),
        "status": job.status,
        "progress": {
            "done": job.progress_done,
            "total": job.segments_total,
            "succeeded": job.progress_succeeded,
            "failed": job.progress_failed,
        },
        "result": job.result,
        "error": job.error,
    })
}

/// 长文档分块抽取的共享逻辑：split → 并发抽取（buffered）→ 合并 document/items/chunks
/// → D2 锚定（对完整原文）。同步 preview handler 与异步 import worker 都复用它，
/// 分块/合并/锚定逻辑与原内联版本字节等价。
///
/// `pub`（经 `routes::ext_knowledge` 导出）：real-LLM 集成测试直调它验真模型抽取，
/// 绕过 handler 的大/小文档 job 分流（测试只关心抽取结果，不涉及异步 job）。
pub async fn run_import_extraction(
    state: &AppState,
    payload: &OperationKnowledgeImportRequest,
    progress: Option<&(dyn Fn(usize, usize, usize) + Send + Sync)>,
) -> AppResult<Value> {
    let system = "你是企业微信运营知识库导入 Agent。你把长文本拆成 Agent 可渐进查询的文档目录、知识包、知识切片和证据块。只输出严格 JSON。";
    let source_name = payload
        .source_name
        .clone()
        .unwrap_or_else(|| "导入文本".to_string());

    // 后端自动分块：长文档切成多段，每段独立调 LLM（输出小、不截断），并发抽取后合并。
    // 小文档（≤ SINGLE_MAX）切分返回单段 → 与分块前字节等价，零回归。
    let segments = split_import_content(&payload.content);
    let total_segments = segments.len();
    // 段完成计数：`buffered` 下并发段完成顺序不定，用原子计数给进度回调喂
    // 单调的 done/succeeded/failed 快照（worker 侧回写 job 进度，同步路径传 None）。
    let done_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let succeeded_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let failed_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let extractions: Vec<(usize, AppResult<Value>)> =
        stream::iter(segments.into_iter().enumerate())
            .map(|(idx, segment)| {
                let state = &state;
                let system = system;
                let source_name = source_name.clone();
                let account_id = payload.account_id.clone();
                let done_counter = done_counter.clone();
                let succeeded_counter = succeeded_counter.clone();
                let failed_counter = failed_counter.clone();
                async move {
                    let user = LONG_IMPORT_PROMPT_TEMPLATE
                        .replace("{SOURCE_NAME}", &source_name)
                        .replace("{CONTENT}", &segment);
                    let result = agent::generate_agent_json(
                        state,
                        account_id.as_deref(),
                        None,
                        None,
                        "knowledge.import.preview",
                        system,
                        &user,
                    )
                    .await;
                    use std::sync::atomic::Ordering::SeqCst;
                    if result.is_ok() {
                        succeeded_counter.fetch_add(1, SeqCst);
                    } else {
                        failed_counter.fetch_add(1, SeqCst);
                    }
                    let done = done_counter.fetch_add(1, SeqCst) + 1;
                    if let Some(cb) = progress {
                        cb(
                            done,
                            succeeded_counter.load(SeqCst),
                            failed_counter.load(SeqCst),
                        );
                    }
                    (idx, result)
                }
            })
            .buffered(IMPORT_EXTRACT_CONCURRENCY)
            .collect()
            .await;

    // 保序收集成功段；单段失败记 warning 跳过，全失败才报错。
    let mut ordered = extractions;
    ordered.sort_by_key(|(idx, _)| *idx);
    let mut values: Vec<Value> = Vec::new();
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for (idx, result) in ordered {
        match result {
            Ok(value) => {
                succeeded += 1;
                values.push(value);
            }
            Err(err) => {
                failed += 1;
                tracing::warn!(
                    segment_index = idx,
                    total_segments,
                    error = %err,
                    "import preview: segment extraction failed (skipped)"
                );
            }
        }
    }
    if values.is_empty() {
        return Err(AppError::External(format!(
            "import preview: 全部 {total_segments} 段抽取均失败"
        )));
    }

    // 合并 document（确定性，不额外调 LLM）。
    let doc_values: Vec<Value> = values
        .iter()
        .filter_map(|v| v.get("document").cloned())
        .collect();
    let document = merge_preview_documents(&doc_values)
        .map(|item| normalize_operation_knowledge_preview_document(item, &payload))
        .unwrap_or_else(|| default_operation_knowledge_preview_document(&payload));

    // 合并 items / chunks（各段按序拼接）。
    let items = values
        .iter()
        .filter_map(|v| v.get("items").and_then(|i| i.as_array()).cloned())
        .flatten()
        .map(|item| normalize_operation_knowledge_preview_item(item, &payload))
        .collect::<Vec<_>>();
    let mut chunks = values
        .iter()
        .filter_map(|v| v.get("chunks").and_then(|c| c.as_array()).cloned())
        .flatten()
        .map(|item| normalize_operation_knowledge_preview_chunk(item, &payload))
        .collect::<Vec<_>>();

    // D2 锚定：仍对完整原文跑一次，每 chunk 的 sourceQuote 在全文锚定（红线不动）。
    let integrity_report = integrity_report_for_preview(&payload.content, &mut chunks);
    Ok(json!({
        "document": document,
        "items": items,
        "chunks": chunks,
        "integrityReport": integrity_report,
        "importReport": {
            "totalSegments": total_segments,
            "succeeded": succeeded,
            "failed": failed,
        },
    }))
}

/// `import_worker` 复用的抽取入口：从 job 的原始字段重建请求并调
/// [`run_import_extraction`]，把段完成进度经回调透出（worker 回写 job 进度）。
/// 走同一 `run_import_extraction`，与同步 preview handler 字节等价。
///
/// 独立 wrapper 是因为 `OperationKnowledgeImportRequest` 字段是 `pub(super)`，
/// crate-root 的 worker 无法直接构造；由本模块内构造后委托。
pub(crate) async fn run_import_extraction_for_job(
    state: &AppState,
    account_id: Option<String>,
    source_name: Option<String>,
    content: String,
    progress: Option<&(dyn Fn(usize, usize, usize) + Send + Sync)>,
) -> AppResult<Value> {
    let payload = OperationKnowledgeImportRequest {
        account_id,
        source_name,
        content,
    };
    run_import_extraction(state, &payload, progress).await
}

/// `import_worker` 建 job 前预算段数（`segments_total`）。与
/// [`run_import_extraction`] 内的 `split_import_content` 同源，保证一致。
pub(crate) fn count_import_segments(content: &str) -> usize {
    split_import_content(content).len()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractKnowledgeTagsRequest {
    account_id: Option<String>,
    title: Option<String>,
    body: String,
}

/// LLM 抽取单条 chunk 的 productTags / businessTopics。路由 handler 与
/// knowledge_task worker（retag action）共用。返回 (productTags, businessTopics)。
pub(crate) async fn extract_knowledge_tags_inner(
    state: &AppState,
    account_id: Option<&str>,
    title: &str,
    body: &str,
) -> AppResult<(Vec<String>, Vec<String>)> {
    let title = if title.trim().is_empty() {
        "未命名知识切片"
    } else {
        title.trim()
    };
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
        title, body
    );
    let value = agent::generate_agent_json(
        state,
        account_id,
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
    Ok((
        normalize_knowledge_tags(product_tags, 5, false),
        normalize_knowledge_tags(business_topics, 3, false),
    ))
}

/// `POST /api/operation-knowledge/extract-tags` —— 给单条 chunk 抽取
/// productTags / businessTopics 两字段。
pub async fn extract_operation_knowledge_tags(
    State(state): State<AppState>,
    Json(payload): Json<ExtractKnowledgeTagsRequest>,
) -> AppResult<Json<Value>> {
    if payload.body.trim().is_empty() {
        return Err(AppError::BadRequest("body is required".to_string()));
    }
    let (product_tags, business_topics) = extract_knowledge_tags_inner(
        &state,
        payload.account_id.as_deref(),
        payload.title.as_deref().unwrap_or(""),
        &payload.body,
    )
    .await?;
    Ok(Json(json!({
        "productTags": product_tags,
        "businessTopics": business_topics,
    })))
}

pub(in crate::routes) async fn import_operation_knowledge_apply(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<OperationKnowledgeImportApplyRequest>,
) -> AppResult<Json<Value>> {
    if payload.items.is_empty()
        && payload
            .chunked_text
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
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
                operation_knowledge_document_from_request(
                    &state,
                    &admin.current_workspace,
                    document,
                    None,
                ),
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
                operation_knowledge_chunk_from_request(
                    &state,
                    &admin.current_workspace,
                    chunk,
                    None,
                )?,
                None,
            )
            .await?;
        if let Some(id) = result.inserted_id.as_object_id() {
            chunk_ids.push(id.to_hex());
        }
    }
    // ── knowledge-wiki Phase D：fence-aware chunked text 流式块导入 ───────
    let mut parse_warnings_json: Vec<Value> = Vec::new();
    if let Some(text) = payload
        .chunked_text
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        let (blocks, warnings) = crate::knowledge_wiki::block_parser::parse_chunk_blocks(text);
        for w in &warnings.items {
            parse_warnings_json.push(parse_warning_to_json(w));
        }
        for block in blocks {
            // payload 中一律期待 camelCase 字段名（与既有 OperationKnowledgeChunkRequest 一致）；
            // 关键缺省值由下面的 enrich + validate 兜底。
            let mut chunk_req: OperationKnowledgeChunkRequest =
                match serde_json::from_value::<OperationKnowledgeChunkRequest>(
                    block.payload.clone(),
                ) {
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
                    operation_knowledge_chunk_from_request(
                        &state,
                        &admin.current_workspace,
                        chunk_req,
                        None,
                    )?,
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
                if let Err(e) =
                    apply_chunk_revision(&state.db, &admin.current_workspace, id, req).await
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
                source_name =
                    Some(field.text().await.map_err(|e| {
                        AppError::BadRequest(format!("sourceName 字段读取失败: {e}"))
                    })?);
            }
            "accountId" => {
                account_id =
                    Some(field.text().await.map_err(|e| {
                        AppError::BadRequest(format!("accountId 字段读取失败: {e}"))
                    })?);
            }
            _ => {}
        }
    }
    let bytes =
        file_bytes.ok_or_else(|| AppError::BadRequest("缺少 file 字段（PDF 字节）".to_string()))?;
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
pub(crate) enum VisionProvider {
    Runtime,
    Dedicated(Vec<(String, crate::llm::LlmClient)>),
}

fn require_non_empty_vision_text(value: Value, field: &str) -> AppResult<Value> {
    if value
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
    {
        Ok(value)
    } else {
        Err(AppError::External(format!(
            "LLM vision response missing non-empty `{field}`"
        )))
    }
}

/// 解析本 workspace 的视觉模型 provider（供知识库导入与运营 Agent 入站图片理解
/// 复用，避免两处各写一套选择逻辑）：
/// a. active 文字主模型本身 supports_vision → 直接用运行时 `state.llm`；
/// b. 否则收集本 workspace 所有 supports_vision 的副模型，专职视觉模型
///    （is_vision_active）排在最前，其余按 updated_at 倒序作为自动切换备用；
/// c. 一条都没有 → `visionNotSupported` 错误。
pub(crate) async fn select_vision_provider(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<VisionProvider> {
    let active = state
        .db
        .llm_provider_configs()
        .find_one(doc! { "workspaceId": workspace_id, "isActive": true }, None)
        .await?;
    if active.as_ref().map(|c| c.supports_vision).unwrap_or(false) {
        // active 文字模型即视觉模型：复用运行时 provider（含热切换 / registry 语义）。
        return Ok(VisionProvider::Runtime);
    }
    // 收集所有支持视觉的副模型，专职视觉模型在前、其余备用在后，组成切换候选链。
    // 排序键：is_vision_active 倒序（专职优先），其次 updated_at 倒序（新配置优先）。
    let cursor = state
        .db
        .llm_provider_configs()
        .find(
            doc! {
                "workspaceId": workspace_id,
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
            "visionNotSupported: 当前文字模型不支持图片，且未在模型设置中指派专职视觉模型"
                .to_string(),
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
            vision_cfg
                .max_retries
                .unwrap_or(state.config.llm_max_retries),
            vision_cfg
                .retry_base_ms
                .unwrap_or(state.config.llm_retry_base_ms),
        )
        .map_err(|e| AppError::External(format!("构造视觉模型 client 失败: {e}")))?;
        candidates.push((vision_cfg.model.clone(), client));
    }
    Ok(VisionProvider::Dedicated(candidates))
}

/// 用已解析的 [`VisionProvider`] 调一次视觉模型，返回结构化 JSON。主视觉模型瞬时
/// 不可达时在候选链上自动切换到下一备用；非瞬时错误立即失败（换模型也救不了）；
/// 全部候选都瞬时不可达才上抛最后一个瞬时变体，让上游按瞬时态处理而非当成内容失败。
/// 供知识库导入与运营 Agent 入站图片理解复用同一条调用/容错逻辑。
pub(crate) async fn vision_generate_json(
    provider: &VisionProvider,
    state: &AppState,
    system_prompt: &str,
    user_prompt: &str,
    image_base64: &str,
    mime: &str,
    required_text_field: &str,
) -> AppResult<Value> {
    match provider {
        VisionProvider::Runtime => {
            let value = state
                .llm
                .generate_json_with_image(system_prompt, user_prompt, image_base64, mime)
                .await
                .map_err(|e| match e {
                    // 瞬时不可达（429/限流/配额耗尽/网关超时）原样透传结构化变体，
                    // 让上游（测试 skip 宏、网关回退逻辑）按瞬时态处理而非当成内容失败。
                    AppError::LlmUnavailable { .. } => e,
                    other => AppError::External(format!("LLM vision 抽取失败: {other}")),
                })?;
            require_non_empty_vision_text(value, required_text_field)
        }
        VisionProvider::Dedicated(candidates) => {
            let mut last_failure: Option<AppError> = None;
            let mut result: Option<AppResult<Value>> = None;
            for (idx, (model, client)) in candidates.iter().enumerate() {
                match client
                    .generate_json_with_image(system_prompt, user_prompt, image_base64, mime)
                    .await
                {
                    Ok(v) => match require_non_empty_vision_text(v, required_text_field) {
                        Ok(v) => {
                            result = Some(Ok(v));
                            break;
                        }
                        Err(error) => {
                            if idx + 1 < candidates.len() {
                                tracing::warn!(
                                    model = %model,
                                    next = %candidates[idx + 1].0,
                                    field = required_text_field,
                                    "vision model returned empty required content; trying backup"
                                );
                            } else {
                                tracing::warn!(
                                    model = %model,
                                    field = required_text_field,
                                    "vision model returned empty required content; no backup remains"
                                );
                            }
                            last_failure = Some(error);
                        }
                    },
                    Err(e @ AppError::LlmUnavailable { .. }) => {
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
                        last_failure = Some(e);
                    }
                    Err(other) => {
                        result = Some(Err(AppError::External(format!(
                            "LLM vision 抽取失败: {other}"
                        ))));
                        break;
                    }
                }
            }
            result.unwrap_or_else(|| {
                Err(last_failure.unwrap_or_else(|| {
                    AppError::External("LLM vision 抽取失败: 无可用视觉模型候选".to_string())
                }))
            })
        }
    }
}

pub async fn import_operation_knowledge_apply_image(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(req): Json<ImportApplyImageRequest>,
) -> AppResult<Json<Value>> {
    if req.image_base64.trim().is_empty() {
        return Err(AppError::BadRequest("imageBase64 不能为空".to_string()));
    }
    // 1) 解析视觉模型（抽到 select_vision_provider 复用）：
    //    a. 若 active 文字主模型本身 supports_vision → 直接用运行时 state.llm。
    //    b. 否则收集本 workspace 所有支持视觉的副模型，专职视觉模型排前、其余备用，构造候选链。
    //    c. 一条都没有 → 502 visionNotSupported，让运营去模型设置里配视觉模型。
    let vision_provider = select_vision_provider(&state, &admin.current_workspace).await?;
    // 2) 拼 vision prompt：约束 LLM 输出 JSON {"fence": "..." }，让我们直接走 chunked_text 流程。
    let mime = req.mime.as_deref().unwrap_or("image/png");
    let hint = req.hint.as_deref().unwrap_or("无特定领域 hint");
    let system_prompt = "你是知识库 chunk 抽取助手。任务：把图片中的可读文本结构化为 fence 块。每块前后用 `---CHUNK: <短安全 id，仅字母数字和连字符>---` 与 `---END CHUNK---` 包裹（结束符必须是 `---END CHUNK---`，不要写 `---END---`）。块体必须是单个 JSON 对象，至少含 `title` 字段，且 `body`/`summary`/`answer` 中至少一个非空字符串，例如 {\"title\":\"小节标题\",\"body\":\"完整正文\"}。块体 JSON 可选带 \"wikiType\"（9 类知识形态之一：source/entity/concept/comparison/synthesis/methodology/finding/query/thesis）与 \"chunkType\"（4 类运营用途之一：product_fact/style_template/peer_case/negative_example，产品事实类填 product_fact）；拿不准可省略，省略时系统按默认处理。\n\
抽取方法（原子信息单元召回，对任何图片一视同仁，不针对特定主题）：\n\
1. 先把图片内容在脑中拆解为一组**原子信息单元**——每个单元是一条可独立成立、不可再拆的事实/条目/字段/陈述（一行表格、一个标题下的一段说明、一条编号项、一组「字段名:值」都各算一个单元）。\n\
2. **穷尽枚举**这些单元：逐个落成 chunk，覆盖图中出现的每一个单元，不要只挑你觉得重要的几条；宁可多分几个 chunk，也不要遗漏。划分以图片自身的视觉/语义边界（标题、分栏、表格行、列表项）为准，而不是以任何预设的主题清单为准。\n\
3. **保留原文 token 粒度**：body 照搬原文的关键表述、专有名词与具体数值（数字、比例、金额、期限、单位、阈值都要原样保留），不要概括、改写或压缩成一句话。\n\
4. **只抽真实存在的文字**：绝不编造、补全、推断或脑补图中没有的内容；图里没写的就不写，看不清的标注为不确定而非猜测。\n\
所有 chunk 默认 needs_review，不要写 verified。返回严格 JSON：{\"fence\": <字符串，全部 fence 文本>}。如果图片无文本可抽取，返回 {\"fence\": \"\"}。".to_string();
    let user_prompt = format!("请按 fence 格式抽取下面这张图片中的知识 chunk。hint：{hint}");
    // 3) 调视觉模型一次（抽到 vision_generate_json 复用容错/候选切换逻辑）：图片以真正的
    //    多模态 image_url content block 发送（generate_json_with_image），而不是把 base64 当
    //    文本塞进 prompt——后者会让纯文字模型"看不到"图片。VisionProvider 解析阶段已保证
    //    选中的是 supports_vision 的模型。
    let value = vision_generate_json(
        &vision_provider,
        &state,
        &system_prompt,
        &user_prompt,
        &req.image_base64,
        mime,
        "fence",
    )
    .await?;
    let raw = value
        .get("fence")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    debug_assert!(!raw.trim().is_empty());
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
                reason: Some(format!(
                    "ingest_chunked_text source={source_name} block={}",
                    block.id
                )),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_payload_requires_the_callers_non_empty_text_field() {
        assert!(require_non_empty_vision_text(json!({"fence": "content"}), "fence").is_ok());
        for value in [
            json!({}),
            json!({"fence": null}),
            json!({"fence": 1}),
            json!({"fence": "   "}),
        ] {
            assert!(require_non_empty_vision_text(value, "fence").is_err());
        }
        assert!(require_non_empty_vision_text(
            json!({"description": "visible image"}),
            "description"
        )
        .is_ok());
    }

    #[test]
    fn long_import_prompt_carries_types_and_drops_dead_fields() {
        // ②-c：prompt 让 LLM 产 wikiType/chunkType（类型透传的源头）
        assert!(
            LONG_IMPORT_PROMPT_TEMPLATE.contains("wikiType"),
            "chunks 模板须含 wikiType"
        );
        assert!(
            LONG_IMPORT_PROMPT_TEMPLATE.contains("chunkType"),
            "chunks 模板须含 chunkType"
        );
        // 已删死字段不得再出现在 prompt（防未来回退）
        for dead in [
            "safeClaims",
            "forbiddenClaims",
            "evidenceItems",
            "routingCard",
        ] {
            assert!(
                !LONG_IMPORT_PROMPT_TEMPLATE.contains(dead),
                "已删字段 {dead} 不应再出现在抽取 prompt"
            );
        }
        // items 分支的已删枚举字段也不应再出现
        for dead in [
            "suitableFor",
            "notSuitableFor",
            "customerStages",
            "operationStates",
            "intentLevels",
            "commonQuestions",
            "commonObjections",
        ] {
            assert!(
                !LONG_IMPORT_PROMPT_TEMPLATE.contains(dead),
                "已删字段 {dead} 不应再出现在抽取 prompt"
            );
        }
        // 护栏保留：忠于原文/不编造案例
        assert!(
            LONG_IMPORT_PROMPT_TEMPLATE.contains("只忠于原文"),
            "须保留忠于原文护栏"
        );
        assert!(
            LONG_IMPORT_PROMPT_TEMPLATE.contains("不要编造成案例"),
            "须保留不编造案例护栏"
        );
        // 占位符仍在，运行时 .replace() 依赖
        assert!(LONG_IMPORT_PROMPT_TEMPLATE.contains("{SOURCE_NAME}"));
        assert!(LONG_IMPORT_PROMPT_TEMPLATE.contains("{CONTENT}"));
    }

    #[test]
    fn import_job_progress_json_matches_contract_fixture() {
        // 异步导入 job 进度端点（get/list）的响应形状：前端轮询按此键集读进度。
        // 构造 completed 态带 result（顶层键集最全）固化契约。
        let job = ImportJob {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899b002").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: Some("acc-1".to_string()),
            source_name: "产品手册".to_string(),
            content: "原文全文".to_string(),
            segments_total: 5,
            progress_done: 5,
            progress_succeeded: 5,
            progress_failed: 0,
            status: "completed".to_string(),
            result: Some(json!({ "document": {}, "items": [], "chunks": [] })),
            error: None,
            claimed_at: None,
            claim_recovery_count: 0,
            expires_at: Some(DateTime::from_millis(1_700_086_500_000)),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let projected = import_job_progress_json(&job);
        crate::routes::contract_snapshot::assert_contract_fixture("import_job_progress", projected);
    }

    // ── split_import_content：后端自动分块（零回归 + 各回退分支） ──────────

    #[test]
    fn split_small_doc_returns_single_segment_verbatim() {
        // ≤ SINGLE_MAX：单段返回，内容逐字不变（零回归路径）。
        let content = "# 标题\n\n一段很短的内容。";
        let segs = split_import_content(content);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], content);
    }

    #[test]
    fn split_at_exactly_single_max_stays_single() {
        // 恰好 = SINGLE_MAX 时仍走单段（边界：<= 判定）。
        let content = "甲".repeat(IMPORT_SINGLE_CALL_MAX_CHARS);
        let segs = split_import_content(&content);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn split_large_doc_by_headings_packs_segments() {
        // 超 SINGLE_MAX 的多标题文档 → 按标题切原子块后贪心打包成多段。
        let mut content = String::new();
        for i in 0..20 {
            content.push_str(&format!("## 小节 {i}\n"));
            content.push_str(&"内容".repeat(200));
            content.push('\n');
        }
        let segs = split_import_content(&content);
        assert!(segs.len() >= 2, "多标题长文应切成多段, got {}", segs.len());
        // 每段都不超 HARD_MAX（除非单个原子块本身超限，本用例每块 ~400 char 不会）。
        for s in &segs {
            assert!(
                s.chars().count() <= IMPORT_SEGMENT_HARD_MAX_CHARS,
                "段长 {} 超 HARD_MAX",
                s.chars().count()
            );
        }
        // 无损：所有段拼回等于原文。
        assert_eq!(segs.concat(), content);
    }

    #[test]
    fn split_oversized_section_falls_back_to_paragraphs() {
        // 单个标题小节就超 HARD_MAX → 按段落窗口再切。
        let mut section = String::from("## 巨型小节\n");
        for _ in 0..10 {
            section.push_str(&"这是一个段落。".repeat(120));
            section.push_str("\n\n");
        }
        assert!(section.chars().count() > IMPORT_SEGMENT_HARD_MAX_CHARS);
        let segs = split_import_content(&section);
        assert!(segs.len() >= 2, "超大单小节应按段落切成多段");
        assert_eq!(segs.concat(), section);
    }

    #[test]
    fn split_headingless_long_text_falls_back_to_paragraphs() {
        // 无任何标题的纯长文 → 单原子块超 HARD_MAX → 段落窗口兜底。
        let mut content = String::new();
        for _ in 0..12 {
            content.push_str(&"没有标题的一段流水文本。".repeat(100));
            content.push_str("\n\n");
        }
        assert!(content.chars().count() > IMPORT_SEGMENT_HARD_MAX_CHARS);
        let segs = split_import_content(&content);
        assert!(segs.len() >= 2, "无标题长文应按段落切成多段");
        assert_eq!(segs.concat(), content);
    }

    #[test]
    fn split_empty_and_whitespace_have_single_segment() {
        // 空 / 纯空白：走单段（≤ SINGLE_MAX），不 panic。
        assert_eq!(split_import_content("").len(), 1);
        assert_eq!(split_import_content("   \n  ").len(), 1);
    }

    #[test]
    fn merge_preview_documents_unions_arrays_and_joins_summaries() {
        let docs = vec![
            json!({
                "title": "文档A",
                "summary": "摘要一",
                "routingMap": ["目录1", "目录2"],
                "productTags": ["套餐A"],
            }),
            json!({
                "title": "文档B",
                "summary": "摘要二",
                "routingMap": ["目录2", "目录3"],
                "productTags": ["套餐B"],
            }),
        ];
        let merged = merge_preview_documents(&docs).unwrap();
        // 标量取首个非空。
        assert_eq!(merged.get("title").unwrap().as_str().unwrap(), "文档A");
        // summary 拼接各段非空值。
        assert_eq!(
            merged.get("summary").unwrap().as_str().unwrap(),
            "摘要一\n摘要二"
        );
        // routingMap 并集去重（目录2 只出现一次）。
        let rm = merged.get("routingMap").unwrap().as_array().unwrap();
        assert_eq!(rm.len(), 3);
        // productTags 并集。
        let pt = merged.get("productTags").unwrap().as_array().unwrap();
        assert_eq!(pt.len(), 2);
    }

    #[test]
    fn merge_preview_documents_empty_returns_none() {
        assert!(merge_preview_documents(&[]).is_none());
    }
}

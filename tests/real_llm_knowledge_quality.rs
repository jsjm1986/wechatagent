//! `real_llm_knowledge_quality` —— 知识库**内容质量**多轮「测-优」迭代套件（Q 系列）。
//!
//! 与 `real_llm_knowledge.rs`（K1–K11，**红线/形状**套件）互补：K 系列证明真模型
//! 在生产闸门下「不破红线、shape 稳定」；本套件在其之上叠加**内容质量度量**——
//! 用同一真模型作 **LLM-as-judge**（0–10 打分）量化每个 LLM 能力的专业度，定位
//! 最低分短板，驱动「测 → 读分 → 修生产代码 → 再测」的多轮收敛闭环。
//!
//! ## 两层判据（每个 Q 测试都同时跑）
//! 1. **硬命中红线**（确定性 `assert!`）：与 K 系列同源——cite⊆seed、抽取/vision 恒
//!    draft+needs_review、对话只产 proposal（计数不变 + verified=0）、审计只读、修复
//!    不落库、标签双数组、关键事实 token 命中。**破则 fail**。
//! 2. **LLM-judge 打分**（`judge_quality`）：真模型按维度 0–10 评 grounding/accuracy/
//!    completeness 等，`overall < MIN_QUALITY_FLOOR(6.0)` 即 fail（= 未达专业基线，
//!    驱动修生产 prompt/检索/抽取逻辑，**绝不放水断言**）。`TARGET_QUALITY(7.0)` 仅
//!    记录、驱动下一轮选短板。
//!
//! ## 三维交叉覆盖
//! - 类型轴：`quality_corpus()` 一次 seed 跨齐 **9 个 wiki_type**（thesis/synthesis/
//!   methodology/finding/comparison/concept/entity/source/query）× **4 个 chunk_type**
//!   （product_fact/style_template/peer_case/negative_example）的真实业务知识库。
//! - 场景轴：用例覆盖 price/trust 等 objection_type、create_chunk 等 intent。
//! - 能力轴：Q1–Q7 扫 7 个 LLM 驱动能力（检索/文章抽取/vision/对话/审计/修复/打标）。
//!
//! ## 红线（与 K 系列同口径，全程不破）
//! - MCP 永远空 wiremock 桩（绝不真发微信）；密钥零泄漏（只 env 读、judge prompt/
//!   日志不打 key）；抽取/vision 落库恒 draft+needs_review；env-gated/瞬时不可达 skip
//!   不 panic；修生产代码不迁就测试；闸门只严不松。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=mimo-v2.5-pro \
//!   cargo test --test real_llm_knowledge_quality -- --ignored --nocapture
//! ```
//! CI 日志可 `grep '\[QUALITY\]'` 拿到每能力/场景的 judge 分，驱动定位短板。

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::{Extension, Json};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;
use wechatagent::agent::knowledge_agent::{answer, AnswerRequest, CatalogFilter};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppResult;
use wechatagent::llm::{LlmClient, LlmProvider};
use wechatagent::models::{LlmProviderConfig, OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::ext_knowledge::{
    build_operation_knowledge_completeness, chat_turn, extract_operation_knowledge_tags,
    import_operation_knowledge_apply_image, import_operation_knowledge_preview,
    propose_chunk_repair, ChatTurnRequest, ExtractKnowledgeTagsRequest, ImportApplyImageRequest,
    OperationKnowledgeImportRequest,
};

use crate::common::TestApp;
use wiremock::MockServer;

// ── env-gated 真实 provider 构造（与 real_llm_knowledge.rs 同形；测试 crate 各自
//    独立编译，fixture 不跨文件共享，故本文件自带一份）──────────────────────────

fn real_llm_from_env() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://token-plan-cn.xiaomimimo.com/v1".to_string());
    let model = std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "mimo-v2.5-pro".to_string());
    let client =
        LlmClient::new(base_url, api_key, model, 180, 3, 1500).expect("构造真实 LlmClient");
    Some(Arc::new(client))
}

macro_rules! require_real_llm {
    () => {{
        match real_llm_from_env() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过真实大模型知识库质量套件");
                return;
            }
        }
    }};
}

/// 解包 `AppResult<T>`；遇真模型上游瞬时不可达（`LlmUnavailable`）打印 skip 并
/// `return`（不 panic、不算质量失败——模型没产出任何输出，无内容可评质量）。
macro_rules! unwrap_or_skip_transient {
    ($result:expr, $what:expr) => {{
        match $result {
            Ok(value) => value,
            Err(wechatagent::error::AppError::LlmUnavailable {
                kind,
                retry_count,
                ..
            }) => {
                eprintln!(
                    "skip: {} —— 真模型上游瞬时不可达（kind={kind}, retry_count={retry_count}），\
                     按计划「真模型抖动有限重试+跳过」处理，不算质量失败",
                    $what
                );
                return;
            }
            Err(other) => panic!("{}：{other}", $what),
        }
    }};
}

async fn dummy_mcp_server() -> MockServer {
    MockServer::start().await
}

// ── 质量门常量 ────────────────────────────────────────────────────────────────

/// 专业基线：judge overall 低于此即硬 fail（= 未达生产可用，驱动修生产代码）。
const MIN_QUALITY_FLOOR: f64 = 6.0;
/// 收敛目标：达此即专业生产级；仅记录、驱动下一轮选最低分短板，不作硬断言。
const TARGET_QUALITY: f64 = 7.0;

// ── LLM-as-judge ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct JudgeScore {
    overall: f64,
    dims: BTreeMap<String, f64>,
    reasons: String,
}

/// 用同一真模型作内容质检员，对 `model_output` 按 `dims` 逐维 0–10 打分。
/// 对照 `ground_truth` 判断 grounding/准确性，发现幻觉/偏离/遗漏扣分。只输出 JSON。
async fn judge_quality(
    llm: &dyn LlmProvider,
    task: &str,
    model_output: &str,
    ground_truth: &str,
    dims: &[&str],
) -> AppResult<JudgeScore> {
    let system = "你是严格的私域运营知识内容质检员。针对给定任务，对【模型输出】按【评分维度】\
逐维打分，每维 0-10 分（10=专业内容生产级、可直接对客；7=合格可用；6=及格基线；<6=不可用）。\
必须对照【参考事实】判断 grounding 与准确性：发现凭空捏造/偏离事实/关键信息遗漏要显著扣分。\
只输出 JSON，禁止任何额外文字、禁止 markdown 代码围栏，形如：\
{\"dims\":{\"维度名\":分数,...},\"overall\":综合分,\"reasons\":\"一句话评分依据\"}。\
overall 为各维综合，任一短板维度都应把 overall 拉低。";
    let user = format!(
        "## 任务\n{task}\n\n## 评分维度\n{dims_joined}\n\n## 参考事实(ground truth)\n{ground_truth}\n\n## 模型输出\n{model_output}\n",
        dims_joined = dims.join("、"),
    );
    let value = llm.generate_json(system, &user).await?;

    let mut parsed_dims = BTreeMap::new();
    if let Some(obj) = value.get("dims").and_then(|d| d.as_object()) {
        for (k, v) in obj {
            if let Some(f) = v.as_f64() {
                parsed_dims.insert(k.clone(), f.clamp(0.0, 10.0));
            }
        }
    }
    let overall = value
        .get("overall")
        .and_then(|v| v.as_f64())
        .map(|f| f.clamp(0.0, 10.0))
        .unwrap_or_else(|| {
            if parsed_dims.is_empty() {
                0.0
            } else {
                parsed_dims.values().sum::<f64>() / parsed_dims.len() as f64
            }
        });
    let reasons = value
        .get("reasons")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(JudgeScore {
        overall,
        dims: parsed_dims,
        reasons,
    })
}

/// 记一行可被 CI `grep '\[QUALITY\]'` 抓到的质量分；返回 overall（调用方据此断言）。
fn report_quality(qid: &str, scene: &str, score: &JudgeScore) {
    eprintln!(
        "[QUALITY] {qid}/{scene} overall={:.1} floor_met={} target_met={} dims={:?} reasons={}",
        score.overall,
        score.overall >= MIN_QUALITY_FLOOR,
        score.overall >= TARGET_QUALITY,
        score.dims,
        score.reasons,
    );
}

/// judge 打分 + 记日志 + 专业基线硬断言（破则 fail，驱动修生产代码）。
fn assert_quality_floor(qid: &str, score: &JudgeScore) {
    assert!(
        score.overall >= MIN_QUALITY_FLOOR,
        "[{qid}] judge overall={:.1} < 专业基线 {MIN_QUALITY_FLOOR}（dims={:?} reasons={}）——\
         内容质量未达生产可用，按迭代闭环修生产 prompt/检索/抽取逻辑，绝不放水断言",
        score.overall,
        score.dims,
        score.reasons,
    );
}

// ── seed helper：可控 wiki_type / chunk_type 的 verified chunk ─────────────────

#[allow(clippy::too_many_arguments)]
async fn seed_typed(
    app: &TestApp,
    ws: &str,
    title: &str,
    summary: &str,
    body: &str,
    wiki_type: &str,
    chunk_type: &str,
    dynamic_confidence: f64,
    related: Vec<RelatedRef>,
) -> String {
    let id = ObjectId::new();
    let now = DateTime::now();
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: ws.to_string(),
        account_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("product_capability".to_string()),
        title: title.to_string(),
        summary: Some(summary.to_string()),
        body: Some(body.to_string()),
        source_quote: Some(body.to_string()),
        integrity_status: Some("verified".to_string()),
        confidence_score: Some(90),
        status: "active".to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some(wiki_type.to_string()),
        dynamic_confidence: Some(dynamic_confidence),
        chunk_type: chunk_type.to_string(),
        related_chunks: if related.is_empty() { None } else { Some(related) },
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert typed chunk");
    id.to_hex()
}

// ── 质量语料（跨 9 wiki_type × 4 chunk_type 的真实私域运营知识库）─────────────

const CORPUS_PRICE_METHOD: &str =
    "处理价格异议的标准方法：第一步共情，认可客户对预算的顾虑；第二步用 ROI 价值锚点\
（节省的人力成本与转化提升）重构性价比，而非比单价；第三步给出按月付费或 14 天试用，\
降低首次决策门槛。绝不直接降价，降价会损害长期价值认知。";

const CORPUS_COMPARISON: &str =
    "与传统群发工具对比：群发是一对多广播、易触发风控被封、缺乏上下文；WechatAgent 是\
逐人逐场景的对话 Agent，带合规审查与渐进式知识检索，按真人节奏发送，不触发风控。";

const CORPUS_PEER_CASE: &str =
    "某连锁零售客户接入 6 周后，私域消息平均首响从 4 小时降到 3 分钟，沉睡客户激活率\
提升 18%，整体转化率提升 22%。关键动作是把高频咨询交给 AI 自动应答 + 人工聚焦高意向客户。";

/// 一次 seed 跨齐 9 个 wiki_type × 4 个 chunk_type 的知识库；返回全部 chunk id。
async fn quality_corpus(app: &TestApp, ws: &str) -> Vec<String> {
    let mut ids = Vec::new();
    // thesis / product_fact
    ids.push(
        seed_typed(
            app, ws,
            "产品核心主张",
            "WechatAgent 用 AI 全自动接管私域逐人对话决策。",
            "WechatAgent 的核心主张：私域运营里重复的对话决策应由 AI 全自动完成，运营聚焦\
策略而非逐条回复。它逐人逐场景做决策、合规审查与跟进，不是群发工具。",
            "thesis", "product_fact", 0.95, Vec::new(),
        )
        .await,
    );
    // synthesis / product_fact
    ids.push(
        seed_typed(
            app, ws,
            "整体解决方案",
            "决策 + 审查 + 渐进式知识检索三件套。",
            "WechatAgent 的整体方案由三部分组成：Reply Agent 做对话决策、独立 Review Agent\
做合规与事实审查、知识库 Agent 做渐进式检索为回答提供已验证依据，三者串成一条自动链路。",
            "synthesis", "product_fact", 0.9, Vec::new(),
        )
        .await,
    );
    // methodology / style_template（价格异议处理方法论）
    ids.push(
        seed_typed(
            app, ws,
            "价格异议处理方法论",
            "共情 → ROI 价值锚点 → 试用/分期，绝不直接降价。",
            CORPUS_PRICE_METHOD,
            "methodology", "style_template", 0.92, Vec::new(),
        )
        .await,
    );
    // finding / peer_case（客户案例）
    ids.push(
        seed_typed(
            app, ws,
            "零售客户实施成效",
            "某零售客户 6 周首响 4 小时→3 分钟、转化 +22%。",
            CORPUS_PEER_CASE,
            "finding", "peer_case", 0.88, Vec::new(),
        )
        .await,
    );
    // comparison / product_fact（与群发工具对比）
    ids.push(
        seed_typed(
            app, ws,
            "与传统群发工具对比",
            "群发广播易被封；本产品逐人对话不触发风控。",
            CORPUS_COMPARISON,
            "comparison", "product_fact", 0.9, Vec::new(),
        )
        .await,
    );
    // concept / product_fact（渐进式检索概念）
    ids.push(
        seed_typed(
            app, ws,
            "渐进式知识检索概念",
            "先看目录摘要，再按需展开正文与关联条目。",
            "渐进式知识检索指 Agent 先读 catalog 目录摘要，再按需 open 正文、follow 关联条目，\
而非一次性把全部知识塞进 prompt，既省 token 又避免上下文淹没。",
            "concept", "product_fact", 0.85, Vec::new(),
        )
        .await,
    );
    // entity / product_fact（定价实体）
    ids.push(
        seed_typed(
            app, ws,
            "企业版定价",
            "企业版 299 元/坐席/月，含私有化部署选项。",
            "企业版定价为 299 元/坐席/月，含私有化部署选项与每年 3 次远程培训；标准版 99 元/坐席/月。",
            "entity", "product_fact", 0.9, Vec::new(),
        )
        .await,
    );
    // source / product_fact（SLA 来源条款）
    ids.push(
        seed_typed(
            app, ws,
            "SLA 来源条款",
            "企业版月度可用性 99.95%，低于 99.9% 赔 30%。",
            "企业版 SLA 原始条款：承诺月度可用性 99.95%；当月低于 99.9% 按服务费 30% 以等额\
服务时长赔付，低于 99.5% 赔 50%，不退现金。",
            "source", "product_fact", 0.8, Vec::new(),
        )
        .await,
    );
    // query / negative_example（错误问法负面示例）
    ids.push(
        seed_typed(
            app, ws,
            "错误问法负面示例",
            "直接问客户预算会激发防御，应探询业务目标。",
            "运营常犯的错误问法：开场就问『你预算多少』会激发客户防御与压力感。应改为先探询\
业务目标与现状痛点，再自然引出方案与投入，属于负面示例，不要照搬。",
            "query", "negative_example", 0.7, Vec::new(),
        )
        .await,
    );
    ids
}

// ── Q1 · 检索/answer 内容质量（grounding/accuracy/relevance）────────────────────
//
// 跨 methodology(style_template) + comparison(product_fact) 语料，问一个 price
// objection 场景。硬命中红线：answer 非空 ∧ cite⊆seed ∧ 命中价格方法论关键事实
// token。judge：grounding/accuracy/relevance ≥ floor。

#[tokio::test]
#[ignore]
async fn q1_retrieval_price_objection_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());
    let ws = state.config.default_workspace_id.clone();
    let seed = quality_corpus(&app, &ws).await;

    let req = AnswerRequest {
        workspace_id: ws.clone(),
        account_id: None,
        query: "客户说『你们太贵了』，作为运营我该怎么回应才专业又不掉价？".to_string(),
        filter: CatalogFilter::default(),
        max_rounds: None,
    };
    let result =
        unwrap_or_skip_transient!(answer(&state, req).await, "Q1 真实知识 agent answer");

    let hits_method = ["共情", "ROI", "价值", "试用", "按月", "锚点", "降价"]
        .iter()
        .any(|t| result.answer.contains(t));
    eprintln!(
        "[q1] rounds={} cited={:?} hits_method={} answer={:?}",
        result.rounds_used,
        result.cited_chunk_ids,
        hits_method,
        result.answer.chars().take(160).collect::<String>(),
    );

    // 硬命中红线。
    assert!(!result.answer.trim().is_empty(), "Q1 answer 不应为空");
    for c in &result.cited_chunk_ids {
        assert!(seed.contains(c), "Q1 cite 了不存在的 chunk id={c}（不在 seed）");
    }
    assert!(
        hits_method,
        "Q1 answer 未命中价格异议方法论任一关键事实 token——检索未把对的方法论喂给生成。answer={:?}",
        result.answer
    );

    // LLM-judge 内容质量。
    let score = unwrap_or_skip_transient!(
        judge_quality(
            llm.as_ref(),
            "运营问『客户嫌贵怎么回应』，评估 AI 给出的话术建议质量。",
            &result.answer,
            CORPUS_PRICE_METHOD,
            &["grounding", "accuracy", "relevance"],
        )
        .await,
        "Q1 judge"
    );
    report_quality("Q1", "price_objection", &score);
    assert_quality_floor("Q1", &score);
}

// ── Q2 · 文章抽取内容质量（completeness/fidelity/structure）─────────────────────
//
// 硬命中红线：每条 preview chunk 恒 draft + needs_review。judge：抽取完整度/保真度/
// 结构化质量。

const Q2_ARTICLE: &str = r#"# WechatAgent 私域运营方法论手册

## 客户分层
把私域客户按意向分为高/中/低三层：高意向当日跟进、中意向 3 天内培育、
低意向放入周期性内容触达池，避免对低意向客户高频打扰造成流失。

## 首次破冰
新客户首次对话不要直接推销，先用一个与其业务相关的问题建立连接，
确认对方场景后再给出针对性价值点，转化率显著高于开场即报价。

## 价格异议
面对价格异议先共情预算顾虑，再用 ROI 价值锚点重构性价比，
最后提供 14 天试用或按月付费降低决策门槛，绝不直接降价。

## 跟进节奏
跟进遵循递减节奏：第 1 天、第 3 天、第 7 天、第 15 天，
每次跟进必须带新增价值（案例/资料/优惠），纯催促会触发反感。
"#;

#[tokio::test]
#[ignore]
async fn q2_article_extraction_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());

    let req: OperationKnowledgeImportRequest = serde_json::from_value(json!({
        "accountId": null,
        "sourceName": "WechatAgent 私域运营方法论手册",
        "content": Q2_ARTICLE,
    }))
    .expect("构造 OperationKnowledgeImportRequest");

    let resp = unwrap_or_skip_transient!(
        import_operation_knowledge_preview(State(state.clone()), Json(req)).await,
        "Q2 真实文章抽取"
    );
    let body = resp.0;
    let chunks = body["chunks"].as_array().cloned().unwrap_or_default();
    let items = body["items"].as_array().cloned().unwrap_or_default();
    eprintln!("[q2] items={} chunks={}", items.len(), chunks.len());

    // 硬命中红线：抽出至少 1 条 ∧ 每条恒 draft + needs_review。
    assert!(
        !chunks.is_empty() || !items.is_empty(),
        "Q2 结构清晰的方法论手册应至少抽出 1 条 chunk/item"
    );
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(
            chunk["integrityStatus"].as_str(),
            Some("needs_review"),
            "Q2 preview chunk[{i}] 必须 needs_review（AI 永不自动 verify）"
        );
        assert_eq!(
            chunk["status"].as_str(),
            Some("draft"),
            "Q2 preview chunk[{i}] 必须 draft"
        );
    }

    if chunks.is_empty() {
        eprintln!("[q2] chunks 为空（仅 items），跳过 judge（无结构化 chunk 可评）");
        return;
    }
    let model_output = serde_json::to_string_pretty(&body["chunks"]).unwrap_or_default();
    let score = unwrap_or_skip_transient!(
        judge_quality(
            llm.as_ref(),
            "评估 AI 从运营方法论手册抽取出的知识切片：是否覆盖客户分层/破冰/价格异议/跟进节奏\
四个主题，标题摘要是否准确，正文是否保真不丢关键信息。",
            &model_output,
            Q2_ARTICLE,
            &["extraction_completeness", "fidelity", "structure"],
        )
        .await,
        "Q2 judge"
    );
    report_quality("Q2", "ops_handbook", &score);
    assert_quality_floor("Q2", &score);
}

// ── Q3 · vision 抽取内容质量（fidelity/completeness）────────────────────────────
//
// 硬命中红线：任何落库 chunk 恒 draft + needs_review。judge：视觉抽取保真度/完整度。
// 复用 K6 的中文条款图 fixture。

const Q3_ARTICLE_IMAGE_BASE64: &str = include_str!("fixtures/k6_article_image.b64");

#[tokio::test]
#[ignore]
async fn q3_vision_extraction_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let api_key = std::env::var("REAL_LLM_API_KEY").expect("require_real_llm 已保证存在");
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://token-plan-cn.xiaomimimo.com/v1".to_string());
    let vision_model = std::env::var("REAL_LLM_VISION_MODEL")
        .or_else(|_| std::env::var("REAL_LLM_MODEL"))
        .unwrap_or_else(|_| "mimo-v2.5".to_string());
    let vision_cfg = LlmProviderConfig {
        id: Some(ObjectId::new()),
        workspace_id: ws.clone(),
        provider_id: "real_vision_q3".to_string(),
        name: "real_vision_q3".to_string(),
        format: "openai".to_string(),
        base_url,
        api_key,
        model: vision_model,
        is_active: false,
        timeout_seconds: Some(180),
        max_retries: Some(3),
        retry_base_ms: Some(1500),
        supports_vision: true,
        is_vision_active: true,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    app.state
        .db
        .llm_provider_configs()
        .insert_one(&vision_cfg, None)
        .await
        .expect("insert vision provider");

    let admin = Extension(AuthenticatedAdmin {
        user_id: "q3_admin".into(),
        username: "q3_admin".into(),
        current_workspace: ws.clone(),
    });
    let req = ImportApplyImageRequest {
        image_base64: Q3_ARTICLE_IMAGE_BASE64.trim().to_string(),
        mime: Some("image/png".to_string()),
        source_name: Some("q3_article_image".to_string()),
        account_id: None,
        hint: Some("企业版服务条款图片".to_string()),
    };

    let resp = unwrap_or_skip_transient!(
        import_operation_knowledge_apply_image(State(app.state.clone()), admin, Json(req)).await,
        "Q3 真实 vision 抽取"
    );
    let body = resp.0;
    let chunk_ids = body["chunkIds"].as_array().cloned().unwrap_or_default();
    eprintln!("[q3] vision chunkIds={}", chunk_ids.len());

    // 硬命中红线：落库 chunk 恒 draft + needs_review；同时收集正文喂 judge。
    let mut extracted_bodies = Vec::new();
    for id in &chunk_ids {
        let id_hex = id.as_str().expect("chunkId str");
        let chunk = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(
                doc! { "_id": ObjectId::parse_str(id_hex).expect("parse oid"), "workspace_id": &ws },
                None,
            )
            .await
            .expect("query chunk")
            .expect("chunk exists");
        assert_eq!(chunk.status, "draft", "Q3 vision chunk 必须 draft");
        assert_eq!(
            chunk.integrity_status.as_deref(),
            Some("needs_review"),
            "Q3 vision chunk 必须 needs_review"
        );
        extracted_bodies.push(format!(
            "{}：{}",
            chunk.title,
            chunk.body.unwrap_or_default()
        ));
    }

    if extracted_bodies.is_empty() {
        eprintln!("[q3] vision 未抽出任何 chunk（真模型软能力，红线真空成立），跳过 judge");
        return;
    }
    let score = unwrap_or_skip_transient!(
        judge_quality(
            llm.as_ref(),
            "评估 AI 从一张中文企业版服务条款图片里抽取的知识：文字识别是否保真、关键条款\
（产品/退款/SLA 等）是否完整、有无编造图中没有的内容。",
            &extracted_bodies.join("\n"),
            "图片为一张含企业版服务条款的中文文章图（涉及产品定位/退款/SLA 等条款文字）。",
            &["fidelity", "completeness"],
        )
        .await,
        "Q3 judge"
    );
    report_quality("Q3", "vision_terms", &score);
    assert_quality_floor("Q3", &score);
}

// ── Q4 · 对话工作台内容质量（intent_correctness/reply_naturalness）──────────────
//
// 硬命中红线：intent∈7闭集 ∧ chunk 计数不变 ∧ verified=0。judge：意图判对 + 回复自然度。

#[tokio::test]
#[ignore]
async fn q4_chat_workstation_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    let chunks_before = state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks before");

    let admin = Extension(AuthenticatedAdmin {
        user_id: "q4_admin".into(),
        username: "q4_admin".into(),
        current_workspace: ws.clone(),
    });
    let req: ChatTurnRequest = serde_json::from_value(json!({
        "sessionId": null,
        "accountId": null,
        "operatorId": "q4_operator",
        "content": "帮我新建一条知识切片：企业版支持私有化部署，数据不出客户内网，\
                    知识类型是产品能力，请起草标题、摘要和正文。",
        "attachments": [],
    }))
    .expect("构造 ChatTurnRequest");

    let resp = unwrap_or_skip_transient!(
        chat_turn(State(state.clone()), admin, Json(req)).await,
        "Q4 真实对话工作台 chat_turn"
    );
    let body = resp.0;
    let intent = body.get("intent").and_then(|v| v.as_str()).unwrap_or("");
    let natural_reply = body
        .get("naturalReply")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    eprintln!(
        "[q4] intent={intent} canApply={:?} naturalReply.len={}",
        body.get("canApply"),
        natural_reply.chars().count(),
    );

    // 硬命中红线。
    const INTENTS: &[&str] = &[
        "create_chunk",
        "update_chunk",
        "clarify_chunk",
        "update_pack",
        "digest_action",
        "update_operator_memory",
        "freeform",
    ];
    assert!(
        INTENTS.contains(&intent),
        "Q4 intent 必须 ∈ 闭集 {INTENTS:?}，实际 {intent:?}"
    );
    let chunks_after = state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks after");
    assert_eq!(
        chunks_before, chunks_after,
        "Q4 对话起草自动落库了 chunk——「只产 proposal、永不落库」红线被击穿"
    );
    let verified_after = state
        .db
        .operation_knowledge_chunks()
        .count_documents(
            doc! { "workspace_id": &ws, "integrity_status": "verified", "status": "active" },
            None,
        )
        .await
        .expect("count verified after");
    assert_eq!(verified_after, 0, "Q4 对话起草落库了 verified chunk——红线被击穿");

    // judge：意图判对 + 回复自然度（明确的新建意图，正确 intent 应为 create_chunk）。
    let model_output = format!("intent={intent}\nnaturalReply={natural_reply}");
    let score = unwrap_or_skip_transient!(
        judge_quality(
            llm.as_ref(),
            "运营在对话框说『帮我新建一条关于私有化部署的产品能力切片，起草标题/摘要/正文』。\
评估 AI 的意图分类是否正确（应为新建切片 create_chunk）、回复是否自然且有效引导补全。",
            &model_output,
            "正确意图是 create_chunk（新建切片）；理想回复应自然地确认意图并起草/引导补全标题、\
摘要、正文等字段，而不是答非所问或生硬。",
            &["intent_correctness", "reply_naturalness"],
        )
        .await,
        "Q4 judge"
    );
    report_quality("Q4", "create_chunk_intent", &score);
    assert_quality_floor("Q4", &score);
}

// ── Q5 · 完整度审计内容质量（gap_analysis_quality/coverage_accuracy）────────────
//
// 硬命中红线：answeringMode∈3闭集 ∧ needs_review chunk 审计后仍 needs_review（只读）。
// judge：gap 分析质量、覆盖判断准确性。

#[tokio::test]
#[ignore]
async fn q5_completeness_audit_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());
    let ws = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();

    // 语料态：一批 verified 业务知识 + 1 条 needs_review 报价草稿（审计绝不能转它）。
    let _seed = quality_corpus(&app, &ws).await;
    let needs_review_id = seed_typed(
        &app,
        &ws,
        "未审定的报价草稿",
        "一条尚未审定的报价说明。",
        "旗舰版 999 元/月（待核实）。",
        "entity",
        "product_fact",
        0.9,
        Vec::new(),
    )
    .await;
    // 把它压回 needs_review（seed_typed 默认 verified）。
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": ObjectId::parse_str(&needs_review_id).expect("oid") },
            doc! { "$set": { "integrity_status": "needs_review" } },
            None,
        )
        .await
        .expect("set needs_review");

    let audit = unwrap_or_skip_transient!(
        build_operation_knowledge_completeness(&state, &ws, &account_id).await,
        "Q5 真实知识完整度审计"
    );
    let mode = audit
        .get("answeringMode")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    eprintln!(
        "[q5] answeringMode={mode} totalChunks={:?} verifiedChunks={:?} gaps={:?}",
        audit.get("totalChunks"),
        audit.get("verifiedChunks"),
        audit.get("gaps"),
    );

    // 硬命中红线。
    const MODES: &[&str] = &["relationship_only", "product_safe", "fully_supported"];
    assert!(
        MODES.contains(&mode),
        "Q5 answeringMode 必须 ∈ 闭集 {MODES:?}，实际 {mode:?}"
    );
    let after = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": ObjectId::parse_str(&needs_review_id).expect("parse oid") },
            None,
        )
        .await
        .expect("query needs_review chunk")
        .expect("chunk exists");
    assert_eq!(
        after.integrity_status.as_deref(),
        Some("needs_review"),
        "Q5 完整度审计把 needs_review 改成 {:?}——审计只读红线被击穿",
        after.integrity_status
    );

    let model_output = serde_json::to_string_pretty(&audit).unwrap_or_default();
    let score = unwrap_or_skip_transient!(
        judge_quality(
            llm.as_ref(),
            "评估 AI 对知识库的完整度自审：给出的 answeringMode 与 gaps 是否合理反映了知识库\
现状（有产品主张/对比/方法论等已验证内容，但报价含未审定草稿），gap 分析是否有指导价值。",
            &model_output,
            "知识库已有产品主张/整体方案/价格方法论/对比/案例/SLA 等 verified 内容，但存在一条\
未审定的报价草稿。理想审计应识别可支撑的范围并指出需补强/核实的缺口（如报价待核实）。",
            &["gap_analysis_quality", "coverage_accuracy"],
        )
        .await,
        "Q5 judge"
    );
    report_quality("Q5", "audit_mixed_corpus", &score);
    assert_quality_floor("Q5", &score);
}

// ── Q6 · AI 修复内容质量（patch_reasonableness/field_targeting）─────────────────
//
// 硬命中红线：propose 后 DB 里 body/status/integrity_status 完全不变（patch 永不落库）。
// judge：修复 patch 是否合理、是否精准命中缺失字段。

#[tokio::test]
#[ignore]
async fn q6_repair_patch_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    // seed 一条信息残缺的 needs_review chunk（缺正文细节 + 无 source_quote）。
    let id = ObjectId::new();
    let now = DateTime::now();
    let original_body = "退款政策。".to_string();
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: ws.clone(),
        account_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("product_capability".to_string()),
        title: "退款政策".to_string(),
        summary: Some("退款相关说明。".to_string()),
        body: Some(original_body.clone()),
        source_quote: None,
        integrity_status: Some("needs_review".to_string()),
        confidence_score: Some(30),
        status: "draft".to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some("methodology".to_string()),
        dynamic_confidence: Some(0.5),
        chunk_type: "product_fact".to_string(),
        ..Default::default()
    };
    state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert repair-target chunk");

    let admin = Extension(AuthenticatedAdmin {
        user_id: "q6_admin".into(),
        username: "q6_admin".into(),
        current_workspace: ws.clone(),
    });
    let resp = unwrap_or_skip_transient!(
        propose_chunk_repair(State(state.clone()), admin, Path(id.to_hex())).await,
        "Q6 真实 AI 修复 propose"
    );
    let body = resp.0;
    eprintln!(
        "[q6] hasPatch={} missingFields={:?}",
        body.get("patch").map(|p| !p.is_null()).unwrap_or(false),
        body.get("missingFields"),
    );

    // 硬命中红线：DB 完全不变。
    let after = state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("query chunk")
        .expect("chunk exists");
    assert_eq!(
        after.body.as_deref(),
        Some(original_body.as_str()),
        "Q6 AI 修复不得改 body（patch 只返回不落库）"
    );
    assert_eq!(after.status, "draft", "Q6 AI 修复不得改 status");
    assert_eq!(
        after.integrity_status.as_deref(),
        Some("needs_review"),
        "Q6 AI 修复不得改 integrity_status"
    );

    let model_output = serde_json::to_string_pretty(&body).unwrap_or_default();
    let score = unwrap_or_skip_transient!(
        judge_quality(
            llm.as_ref(),
            "一条退款政策切片正文只有『退款政策。』、缺 source_quote。评估 AI 给出的修复方案：\
是否精准指出缺失字段、提出的补全建议是否合理（不得凭空编造具体数字当成事实）。",
            &model_output,
            "原 chunk 正文残缺、无原文引用。理想修复应识别正文过简、缺 source_quote 等缺口，\
建议补全退款条件/期限等结构，但不应把未经核实的具体数字写成既定事实。",
            &["patch_reasonableness", "field_targeting"],
        )
        .await,
        "Q6 judge"
    );
    report_quality("Q6", "incomplete_chunk", &score);
    assert_quality_floor("Q6", &score);
}

// ── Q7 · 打标内容质量（tag_accuracy/taxonomy_mapping）───────────────────────────
//
// 硬命中红线：productTags / businessTopics 双数组。judge：标签准确性 + 是否贴合内容。

#[tokio::test]
#[ignore]
async fn q7_tag_extraction_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());

    let title = "价格异议处理方法论";
    let req: ExtractKnowledgeTagsRequest = serde_json::from_value(json!({
        "accountId": null,
        "title": title,
        "body": CORPUS_PRICE_METHOD,
    }))
    .expect("构造 ExtractKnowledgeTagsRequest");

    let resp = unwrap_or_skip_transient!(
        extract_operation_knowledge_tags(State(state.clone()), Json(req)).await,
        "Q7 真实标签抽取"
    );
    let body = resp.0;
    eprintln!(
        "[q7] productTags={:?} businessTopics={:?}",
        body.get("productTags"),
        body.get("businessTopics"),
    );

    // 硬命中红线：两字段都是数组。
    assert!(body["productTags"].is_array(), "Q7 productTags 必须是数组");
    assert!(
        body["businessTopics"].is_array(),
        "Q7 businessTopics 必须是数组"
    );

    let model_output = serde_json::to_string_pretty(&body).unwrap_or_default();
    let score = unwrap_or_skip_transient!(
        judge_quality(
            llm.as_ref(),
            "评估 AI 为『价格异议处理方法论』切片抽取的标签（productTags / businessTopics）：\
是否贴合内容主题（价格异议/销售方法/客户沟通等），有无明显跑题或空泛标签。",
            &model_output,
            CORPUS_PRICE_METHOD,
            &["tag_accuracy", "taxonomy_mapping"],
        )
        .await,
        "Q7 judge"
    );
    report_quality("Q7", "price_method_tags", &score);
    assert_quality_floor("Q7", &score);
}

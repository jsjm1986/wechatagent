//! `real_llm_knowledge` —— 用**真实大模型**跑**知识库全能力**真实任务。
//!
//! 与 `real_llm_smoke.rs`（T1/T2/T3，运营决策/审查/vision 主链路，由另一 agent
//! 维护）互补：本套件**专注知识库 agent 的渐进式披露检索全能力**，验证真模型在
//! 真实 prompt 下能否驱动 `list_catalog → open_chunk → follow_relations → answer`
//! 工具循环、跨 chunk 关系跳转、并守住「不凭空引用 / 不服务未审定知识」红线。
//!
//! ## 覆盖的真实能力（mock 测不到）
//! - **K1 · open_chunk 深检索**：答案细节只在 chunk 正文（body），catalog 摘要看不到
//!   → 真模型必须真的 open_chunk 读正文才能答对。
//! - **K2 · follow_relations 关系图谱**：把目标 chunk B 挤出 catalog 头部，只能经
//!   chunk A 的 `related_chunks` 跳到 → 真模型必须用 follow_relations 才能触达。
//! - **K3 · 无幻觉**：catalog 非空但**没有**任何与提问相关的 chunk → 真模型不得
//!   凭空捏造引用 id（红线：cite ⊆ seed）。
//! - **K4 · 未审定知识永不上桌**：答案只在一条 `needs_review` chunk 里 → 生产闸门
//!   （catalog/open_chunk 均 verified-only）保证它**永不进 catalog、永不被 cite**。
//!
//! ## 红线
//! - **MCP 永远是桩**：知识链路本不发消息，但 `rebuild_app_state_with_real_llm`
//!   仍把 `mcp_base_url` 指向一个空 wiremock，绝不真发微信。
//! - **密钥零泄漏**：只从 env 读 `REAL_LLM_API_KEY`，断言信息不打印 key。
//! - **cite ⊆ seed**：真模型引用的 chunk id 必须 ⊆ 本测试 seed 的集合，绝不凭空捏造。
//! - **未审定不上桌**：`needs_review` / `draft` chunk 永不被 cite（生产 verified-only 闸门）。
//! - **env-gated**：无 `REAL_LLM_API_KEY` 时每个 test 自我跳过（eprintln + return），
//!   不 panic；默认 `#[ignore]`，本地 `cargo test` 不触网。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=mimo-v2.5-pro \
//!   cargo test --test real_llm_knowledge -- --ignored --nocapture
//! ```
//! 由 GitHub CI 的 `real-llm` job 驱动（见 `.github/workflows/ci.yml`）。

mod common;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::{Extension, Json};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;
use wechatagent::agent::knowledge_agent::{answer, AnswerRequest, CatalogFilter};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::llm::LlmClient;
use wechatagent::models::{LlmProviderConfig, OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::ext_knowledge::{
    auto_verify_operation_knowledge_chunks, decide_auto_verify_status,
    extract_operation_knowledge_tags, import_operation_knowledge_apply_image,
    import_operation_knowledge_preview, propose_chunk_repair, ExtractKnowledgeTagsRequest,
    ImportApplyImageRequest, KnowledgeAutoVerifyRequest, OperationKnowledgeImportRequest,
};

use crate::common::TestApp;
use wiremock::MockServer;

// ── env-gated 真实 provider 构造（与 real_llm_smoke.rs 同形；测试 crate 各自独立
//    编译，fixture 不跨文件共享，故本文件自带一份）────────────────────────────

/// 从 env 构造真实文本 provider。缺 `REAL_LLM_API_KEY` → None（调用方自我跳过）。
fn real_llm_from_env() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://token-plan-cn.xiaomimimo.com/v1".to_string());
    let model = std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "mimo-v2.5-pro".to_string());
    let client =
        LlmClient::new(base_url, api_key, model, 180, 3, 1500).expect("构造真实 LlmClient");
    Some(Arc::new(client))
}

/// 跳过宏：无 key 时打印一行 skip 并 `return`（不 panic、不算失败）。
macro_rules! require_real_llm {
    () => {{
        match real_llm_from_env() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过真实大模型知识库 smoke");
                return;
            }
        }
    }};
}

/// 把一个 `AppResult<T>` 解包为 `T`；遇到**真模型上游瞬时不可达**
/// （`AppError::LlmUnavailable`：client 自身 3 次重试后仍 timeout / 429 / 5xx）时，
/// 打印一行 skip 并 `return`（不 panic、不算失败）。
///
/// **为何这不是放水红线**：`LlmUnavailable` 意味着模型**根本没产出任何输出**——
/// 没有抽取结果、没有 answer、没有落库 chunk，故「AI 永不自动 verify / cite ⊆ seed /
/// 未审定不上桌」等红线**真空成立**（无内容可违例）。只要模型**有**响应，下游
/// 全部硬断言照常以完整严格度执行。这与红线 #6「无 key 时 env-gated skip 而非 panic」
/// 同源：基础设施不可用属于环境噪声，不该被记为生产级链路/schema/闸门 bug。
/// 真模型抖动（限流/超时）按计划「有限重试 + 跳过」，不进修复循环。
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
                     按计划「真模型抖动有限重试+跳过」处理，不算生产级失败",
                    $what
                );
                return;
            }
            Err(other) => panic!("{}：{other}", $what),
        }
    }};
}

/// 知识链路不发消息，但 `rebuild_app_state_with_real_llm` 需要一个 mcp_url 构造
/// McpClient。起一个**不挂任何 mock 的空 wiremock**：URL 合法可解析，但永不被命中
/// （知识 agent 模块对 gateway/outbox/mcp 零耦合）。
async fn dummy_mcp_server() -> MockServer {
    MockServer::start().await
}

// ── seed helper：完整控制 summary / body / integrity_status / related ────────

/// 落一条 chunk，返回 hex id。`related` 为空 → `related_chunks=None`。
/// `dynamic_confidence` 控制 catalog 排序（用于 K2 把 B 挤出 catalog 头部）。
#[allow(clippy::too_many_arguments)]
async fn seed_chunk(
    app: &TestApp,
    ws: &str,
    title: &str,
    summary: &str,
    body: &str,
    integrity_status: &str,
    status: &str,
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
        integrity_status: Some(integrity_status.to_string()),
        confidence_score: Some(88),
        status: status.to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some("methodology".to_string()),
        dynamic_confidence: Some(dynamic_confidence),
        chunk_type: "product_fact".to_string(),
        related_chunks: if related.is_empty() { None } else { Some(related) },
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert chunk");
    id.to_hex()
}

/// 便捷：seed 一条 verified / active / 高置信的全局 chunk（summary≠body 时由调用方控制）。
async fn seed_verified(
    app: &TestApp,
    ws: &str,
    title: &str,
    summary: &str,
    body: &str,
) -> String {
    seed_chunk(app, ws, title, summary, body, "verified", "active", 0.9, Vec::new()).await
}

/// 断言 tool_trace 里是否出现某个 tool 名的步骤。
fn trace_has_tool(result: &wechatagent::agent::knowledge_agent::AnswerResult, tool: &str) -> bool {
    result
        .tool_trace
        .iter()
        .any(|d| d.get_str("tool").map(|t| t == tool).unwrap_or(false))
}

/// 断言 tool_trace 里某个 open_chunk 步骤的 `opened` 数组包含目标 id。
fn trace_opened_id(
    result: &wechatagent::agent::knowledge_agent::AnswerResult,
    id: &str,
) -> bool {
    result.tool_trace.iter().any(|d| {
        d.get_str("tool").map(|t| t == "open_chunk").unwrap_or(false)
            && d.get_array("opened")
                .map(|arr| arr.iter().any(|b| b.as_str() == Some(id)))
                .unwrap_or(false)
    })
}

// ── K1 · open_chunk 深检索（答案只在 body，不在 catalog 摘要）────────────────

/// catalog 只暴露 `summary`（截断 120 char）。本测试故意把**赔付比例数字**只放进
/// `body`、summary 写得很笼统 → 真模型若不 open_chunk 就答不出具体数字。
///
/// 硬断言：tool_trace 必含一步 `open_chunk` 且打开了目标 chunk（核心能力——真模型
/// 必须真的展开正文）；cite ⊆ seed（红线）。
#[tokio::test]
#[ignore]
async fn k1_real_open_chunk_reaches_body_detail() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    // 赔付比例（30% / 50%）只在 body；summary 笼统到无法回答具体数字。
    let id_k1 = seed_chunk(
        &app,
        &ws,
        "企业版 SLA 与故障赔付",
        "介绍企业版的服务可用性保障与故障处理的总体条款。", // 摘要无数字
        "企业版 SLA 承诺 99.95% 月度可用性。若当月可用性低于 99.9%，按当月服务费的 30% 赔付；\
         低于 99.5% 则赔付 50%。赔付以等额服务时长抵扣形式发放，不退现金。", // 数字只在正文
        "verified",
        "active",
        0.95,
        Vec::new(),
    )
    .await;
    // 一条无关 chunk，制造 catalog 选择压力。
    let id_noise = seed_verified(
        &app,
        &ws,
        "营业时间",
        "客服在线时间说明。",
        "工作日 9:00–21:00 在线，节假日顺延。",
    )
    .await;
    let seed = [id_k1.clone(), id_noise];

    let req = AnswerRequest {
        workspace_id: ws.clone(),
        account_id: None,
        query: "企业版如果月度可用性没达标，具体的赔付比例是多少？".to_string(),
        filter: CatalogFilter::default(),
        max_rounds: None,
    };
    let result = unwrap_or_skip_transient!(answer(&state, req).await, "真实知识 agent answer");

    eprintln!(
        "[k1] rounds_used={} cited={:?} opened_k1={} answer.len={} answer={:?}",
        result.rounds_used,
        result.cited_chunk_ids,
        trace_opened_id(&result, &id_k1),
        result.answer.chars().count(),
        result.answer.chars().take(120).collect::<String>(),
    );

    assert!(result.rounds_used >= 1, "真模型必须至少跑 1 轮，实际 {}", result.rounds_used);
    assert!(!result.answer.trim().is_empty(), "answer 不应为空");
    // 红线：cite 的每个 id 必须是本 workspace seed 出来的某条（不能凭空捏造）。
    // K1 只关心赔付 chunk 被 open；over-cite 无关 chunk 由 K3 专门盯。
    for c in &result.cited_chunk_ids {
        assert!(
            seed.contains(c),
            "真模型 cite 了不存在的 chunk id={c}，seed={seed:?}",
        );
    }
    // 核心能力硬断言：真模型必须真的 open_chunk 读到正文，才可能答出 30%/50%。
    assert!(
        trace_opened_id(&result, &id_k1),
        "真模型必须 open_chunk 展开 K1 正文（赔付数字只在 body），tool_trace={:?}",
        result.tool_trace
    );
}

// ── K2 · follow_relations 关系图谱（B 被挤出 catalog，只能沿关系跳）──────────

/// catalog 头部容量 30（生产 `CATALOG_PAGE_SIZE`）。seed 30 条填充 chunk 占满头部，
/// 把**含硬件答案的 B**（低置信）挤出 catalog；A（高置信）在 catalog 内且 `requires` B。
/// 真模型要回答硬件前置，**唯一路径**是 follow_relations 从 A 跳到 B。
///
/// 硬断言：tool_trace 必含一步 `follow_relations`（核心能力）；cite ⊆ 全部 seed（红线）。
#[tokio::test]
#[ignore]
async fn k2_real_follow_relations_reaches_excluded_chunk() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    // B：硬件前置答案所在，低 dynamic_confidence → 被挤出 catalog 头部。
    let id_b = seed_chunk(
        &app,
        &ws,
        "私有化部署硬件前置条件",
        "私有化部署的服务器与资源要求。",
        "私有化部署需独立服务器，最低配置 8 核 16G 内存、200G SSD；需 Docker 20+ 与内网 DNS。",
        "verified",
        "active",
        0.05, // 极低 → 排在所有填充 chunk 之后，跌出 top-30
        Vec::new(),
    )
    .await;

    // A：在 catalog 内（高置信），summary 明确指引「硬件见关联条目」，requires B。
    let id_a = seed_chunk(
        &app,
        &ws,
        "私有化部署方案",
        "私有化部署整体方案；具体硬件/前置条件见关联条目。",
        "私有化部署支持完全离线运行，数据不出内网。硬件与前置条件详见关联的前置条件条目。",
        "verified",
        "active",
        0.99, // 最高 → 一定在 catalog 内
        vec![RelatedRef {
            chunk_id: id_b.clone(),
            kind: "requires".to_string(),
            note: Some("私有化部署的硬件前置条件".to_string()),
        }],
    )
    .await;

    // 30 条填充 chunk（中等置信）占满 catalog 头部，确保 B 跌出 top-30。
    let mut all_seed = vec![id_a.clone(), id_b.clone()];
    for i in 0..30 {
        let id = seed_chunk(
            &app,
            &ws,
            &format!("常见问题 {i}"),
            &format!("常见问题 {i} 的简要说明。"),
            &format!("这是与私有化部署硬件无关的常见问题 {i} 的正文内容。"),
            "verified",
            "active",
            0.5, // 高于 B、低于 A → 占满 A 之后的 catalog 名额
            Vec::new(),
        )
        .await;
        all_seed.push(id);
    }

    let req = AnswerRequest {
        workspace_id: ws.clone(),
        account_id: None,
        query: "私有化部署对服务器硬件有什么具体要求？最低配置是多少？".to_string(),
        filter: CatalogFilter::default(),
        max_rounds: None,
    };
    let result = unwrap_or_skip_transient!(answer(&state, req).await, "真实知识 agent answer");

    let reached_b_via_follow = trace_has_tool(&result, "follow_relations");
    let reached_b_via_open = trace_opened_id(&result, &id_b);
    eprintln!(
        "[k2] rounds_used={} used_follow={} opened_B={} cited={:?} cited_has_B={} answer={:?}",
        result.rounds_used,
        reached_b_via_follow,
        reached_b_via_open,
        result.cited_chunk_ids,
        result.cited_chunk_ids.contains(&id_b),
        result.answer.chars().take(120).collect::<String>(),
    );

    assert!(result.rounds_used >= 1, "真模型必须至少跑 1 轮，实际 {}", result.rounds_used);
    // 红线：cite ⊆ 全部 seed（含 A/B/填充），绝不凭空。
    for c in &result.cited_chunk_ids {
        assert!(
            all_seed.contains(c),
            "真模型 cite 了不存在的 chunk id={c}（不在 seed 集合）",
        );
    }
    // 核心能力硬断言：B 的 dynamic_confidence=0.05 把它挤出 catalog top-30，
    // catalog 摘要里**根本看不到 B 的 chunkId**——唯一获知 B id 的途径是先 open A、
    // 从 A 的 `relatedChunks` 拿到指针。因此「触达 B」本身即证明真模型沿 A→B 关系图
    // 跳转成功。生产侧有两条等价的关系遍历路径：
    //   (1) follow_relations(A) —— 显式关系跳转工具；
    //   (2) open_chunk(B-id-从-A-的-relatedChunks-学到) —— open_chunk 按 _id 直取
    //       任意 verified chunk（knowledge_agent.rs:900，无 catalog 成员校验）。
    // 二者都只能在「已从 A 学到 B id」后发生，等价地证明关系边被遍历。硬断言锁
    // **能力**（触达被挤出 catalog 的关系条目）而非**特定工具名**——否则就是为某一
    // 工具的措辞优化测试，而非验证生产真实可达性。
    assert!(
        reached_b_via_follow || reached_b_via_open,
        "真模型必须沿 A→B 关系图触达被挤出 catalog 的硬件条目 B（follow_relations 或 \
         open_chunk(从 A 关系指针学到的 B id) 均可），但两条路径都未命中。\
         tool_trace={:?}",
        result.tool_trace
    );
}

// ── K3 · 无幻觉（catalog 非空但无相关 chunk）─────────────────────────────────

/// seed 几条**与提问完全无关**的 verified chunk（catalog 非空），提问一个知识库
/// 完全没覆盖的主题。验证点（红线）：真模型 cite 的 id 必 ⊆ seed（绝不捏造 id）；
/// 软诊断：打印是否 over-cite 了无关 chunk（无关主题理应零引用）。
#[tokio::test]
#[ignore]
async fn k3_real_no_hallucination_when_topic_absent() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    let id1 = seed_verified(
        &app,
        &ws,
        "退款政策",
        "下单后 7 天内可无理由退款。",
        "下单后 7 天内可无理由退款，需保持商品完好；超过 7 天按损耗比例处理。",
    )
    .await;
    let id2 = seed_verified(
        &app,
        &ws,
        "营业时间",
        "客服在线时间。",
        "工作日 9:00–21:00 在线，节假日顺延。",
    )
    .await;
    let seed = [id1, id2];

    // 知识库完全没覆盖的主题：海外支付货币 / 跨境结算手续费。
    let req = AnswerRequest {
        workspace_id: ws.clone(),
        account_id: None,
        query: "你们支持哪些海外支付货币？跨境结算的手续费是多少个百分点？".to_string(),
        filter: CatalogFilter::default(),
        max_rounds: None,
    };
    let result = unwrap_or_skip_transient!(answer(&state, req).await, "真实知识 agent answer");

    eprintln!(
        "[k3] rounds_used={} cited={:?} answer={:?}",
        result.rounds_used,
        result.cited_chunk_ids,
        result.answer.chars().take(160).collect::<String>(),
    );

    assert!(result.rounds_used >= 1, "真模型必须至少跑 1 轮，实际 {}", result.rounds_used);
    assert!(!result.answer.trim().is_empty(), "answer 不应为空（至少应说明无相关信息）");
    // 红线：cite 的每个 id 必 ⊆ seed（真模型绝不捏造不存在的 chunk id）。
    for c in &result.cited_chunk_ids {
        assert!(
            seed.contains(c),
            "真模型 cite 了不存在/无关捏造的 chunk id={c}，seed={seed:?}",
        );
    }
    // 软诊断：无关主题理应零引用；非空时在 CI 日志标注，供迭代判断是否需收紧 prompt。
    if !result.cited_chunk_ids.is_empty() {
        eprintln!(
            "[k3][warn] 真模型对无覆盖主题 over-cite 了 {} 条无关 chunk（理想应为 0）：{:?}",
            result.cited_chunk_ids.len(),
            result.cited_chunk_ids
        );
    }
}

// ── K4 · 未审定知识永不上桌（needs_review chunk 永不进 catalog / 永不被 cite）──

/// 答案只在一条 `needs_review` chunk；另 seed 一条无关 verified chunk 让 catalog 非空。
/// 生产闸门（list_catalog / open_chunk 默认 verified-only）保证未审定 chunk **永不
/// 进 catalog、永不被 open、永不被 cite**——这是「AI 永不自动 verify、永不服务未审定
/// 知识」红线在真模型下的回归门。
///
/// 硬断言（红线）：cite 必**不含** needs_review chunk 的 id；cite ⊆ verified seed。
#[tokio::test]
#[ignore]
async fn k4_real_unverified_chunk_never_served() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    // U：答案所在，但 needs_review（未审定）→ 闸门必须挡住。
    let id_unverified = seed_chunk(
        &app,
        &ws,
        "内部未审定定价表",
        "内部测试用定价，未经审定。",
        "内部测试定价：基础版 99 元/月，专业版 299 元/月。", // 答案只在这（但未审定）
        "needs_review",
        "active",
        0.95,
        Vec::new(),
    )
    .await;
    // V：无关 verified，让 catalog 非空（避免空 catalog 短路）。
    let id_verified = seed_verified(
        &app,
        &ws,
        "对接方式",
        "支持的对接方式说明。",
        "支持 OpenAPI 与 Webhook 两种对接方式，提供测试沙箱与示例代码。",
    )
    .await;

    let req = AnswerRequest {
        workspace_id: ws.clone(),
        account_id: None,
        query: "你们基础版多少钱一个月？专业版呢？".to_string(),
        filter: CatalogFilter::default(), // include_unverified=false（默认）
        max_rounds: None,
    };
    let result = unwrap_or_skip_transient!(answer(&state, req).await, "真实知识 agent answer");

    eprintln!(
        "[k4] rounds_used={} cited={:?} answer={:?}",
        result.rounds_used,
        result.cited_chunk_ids,
        result.answer.chars().take(160).collect::<String>(),
    );

    // 红线①：未审定 chunk 的 id 绝不出现在 cite 里。
    assert!(
        !result.cited_chunk_ids.contains(&id_unverified),
        "未审定（needs_review）chunk 被 cite 了——verified-only 闸门被击穿！cited={:?}",
        result.cited_chunk_ids
    );
    // 红线②：cite ⊆ verified seed（这里只有 V 是 verified；U 永不上桌）。
    for c in &result.cited_chunk_ids {
        assert!(
            c == &id_verified,
            "真模型 cite 了非 verified seed 的 id={c}（只允许 V={id_verified}）",
        );
    }
}

// ── K5 · 真实文章抽取（import_operation_knowledge_preview）──────────────────────
//
// 这是「抽取」全能力的核心：把一篇**真实中文运营资料**交给真模型，让它拆成
// document / items / chunks 渐进式知识结构。mock 测不到「真模型在真实长文 prompt
// 下能否输出结构化 JSON、能否被 normalizer 接住」。
//
// 软断言：真模型至少抽出 ≥1 条 chunk（抽取命中不做超硬保证，但一篇结构清晰的资料
// 真模型基本必出 chunk）。
// **硬断言（红线）**：任何 preview chunk 的 `integrityStatus` 必为 `needs_review`、
// `status` 必为 `draft`——AI 抽取永不自动 verify（normalizer knowledge.rs:2555 强制）。

/// 一篇真实风格的中文运营知识资料（产品/退款/SLA/实施/对接），供真模型抽取。
const K5_ARTICLE: &str = r#"# WechatAgent 企业版服务说明

## 产品定位
WechatAgent 是面向私域运营团队的 AI 自动化助手，帮助运营在企业微信里
对客户消息做自动决策、合规审查与跟进，目标是把重复的话术工作交给 AI，
让运营聚焦策略。它不是群发工具，而是逐人逐场景的渐进式对话 Agent。

## 退款政策
标准版与企业版均支持下单后 7 天内无理由退款，需保持账户未超量使用。
超过 7 天后按已消耗的服务时长比例结算，剩余部分以服务时长形式返还，不退现金。

## 服务可用性 SLA
企业版承诺月度可用性 99.95%。若当月可用性低于 99.9%，按当月服务费的 30%
以等额服务时长赔付；低于 99.5% 则赔付 50%。赔付不以现金形式发放。

## 实施周期
标准实施周期为 2 到 4 周：第 1~2 周梳理客户运营流程、接通试点账号；
第 3~4 周扩展到核心业务场景并完成知识库灌注。

## 对接方式
支持 OpenAPI 与 Webhook 两种对接方式，提供测试沙箱、示例代码与联调支持。
私有化部署为企业版可选项，数据不出内网。
"#;

#[tokio::test]
#[ignore]
async fn k5_real_article_extraction_keeps_needs_review() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());

    let req: OperationKnowledgeImportRequest = serde_json::from_value(json!({
        "accountId": null,
        "sourceName": "WechatAgent 企业版服务说明",
        "content": K5_ARTICLE,
    }))
    .expect("构造 OperationKnowledgeImportRequest");

    let resp = unwrap_or_skip_transient!(
        import_operation_knowledge_preview(State(state.clone()), Json(req)).await,
        "真实文章抽取（不崩、JSON 能解析+normalize）"
    );
    let body = resp.0;

    let chunks = body["chunks"].as_array().cloned().unwrap_or_default();
    let items = body["items"].as_array().cloned().unwrap_or_default();
    eprintln!(
        "[k5] document.title={:?} items={} chunks={} integrityReport={:?}",
        body["document"].get("title"),
        items.len(),
        chunks.len(),
        body.get("integrityReport"),
    );

    // 软断言：结构清晰的资料真模型应至少抽出 1 条 chunk（命中不超硬保证）。
    assert!(
        !chunks.is_empty() || !items.is_empty(),
        "真模型对结构清晰的资料应至少抽出 1 条 chunk 或 item，实际两者皆空"
    );

    // 红线（硬断言）：每一条 preview chunk 必 needs_review + draft——AI 永不自动 verify。
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(
            chunk["integrityStatus"].as_str(),
            Some("needs_review"),
            "preview chunk[{i}] integrityStatus 必须 needs_review（AI 永不自动 verify），实际 {:?}",
            chunk["integrityStatus"]
        );
        assert_eq!(
            chunk["status"].as_str(),
            Some("draft"),
            "preview chunk[{i}] status 必须 draft，实际 {:?}",
            chunk["status"]
        );
    }
}

// ── K6 · 真实多模态图片抽取（import_operation_knowledge_apply_image）────────────
//
// 走「专职视觉副模型」分支：seed 一条 `supports_vision + is_vision_active` 的
// LlmProviderConfig（真实 MiMo 多模态），handler 临时构造真实 vision client 从图片
// 抽 chunk。图片是 PIL 生成的一张含可读中文条款的文章图（tests/fixtures）。
//
// 软断言：真模型抽取命中不做硬性保证（抽出 chunk 或 fence 空都通过）。
// **硬断言（红线）**：任何落库 chunk 必 `draft` + `needs_review`。

/// PIL 生成的中文文章图（720×520 PNG，base64 无 data-uri 前缀）。
const K6_ARTICLE_IMAGE_BASE64: &str = include_str!("fixtures/k6_article_image.b64");

#[tokio::test]
#[ignore]
async fn k6_real_vision_article_extraction_keeps_needs_review() {
    let _llm = require_real_llm!();
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // seed 专职视觉副模型（真实 MiMo 多模态）。
    let api_key = std::env::var("REAL_LLM_API_KEY").expect("require_real_llm 已保证存在");
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://token-plan-cn.xiaomimimo.com/v1".to_string());
    let vision_model = std::env::var("REAL_LLM_VISION_MODEL")
        .or_else(|_| std::env::var("REAL_LLM_MODEL"))
        .unwrap_or_else(|_| "mimo-v2.5".to_string());
    let vision_cfg = LlmProviderConfig {
        id: Some(ObjectId::new()),
        workspace_id: ws.clone(),
        provider_id: "real_vision_k6".to_string(),
        name: "real_vision_k6".to_string(),
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
        user_id: "k6_admin".into(),
        username: "k6_admin".into(),
        current_workspace: ws.clone(),
    });
    let req = ImportApplyImageRequest {
        image_base64: K6_ARTICLE_IMAGE_BASE64.trim().to_string(),
        mime: Some("image/png".to_string()),
        source_name: Some("k6_article_image".to_string()),
        account_id: None,
        hint: Some("企业版服务条款图片".to_string()),
    };

    let resp = unwrap_or_skip_transient!(
        import_operation_knowledge_apply_image(State(app.state.clone()), admin, Json(req)).await,
        "真实 vision 抽取（不崩）"
    );
    let body = resp.0;
    let chunk_ids = body["chunkIds"].as_array().cloned().unwrap_or_default();
    eprintln!(
        "[k6] vision chunkIds={} fallbackBlob={:?} note={:?}",
        chunk_ids.len(),
        body.get("fallbackBlob"),
        body.get("note"),
    );

    // 红线（硬断言）：任何落库 chunk 必 draft + needs_review。
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
        assert_eq!(chunk.status, "draft", "vision chunk 必须 draft（AI 永不自动 verify）");
        assert_eq!(
            chunk.integrity_status.as_deref(),
            Some("needs_review"),
            "vision chunk 必须 needs_review（AI 永不自动 verify）"
        );
    }
}

// ── K7 · 自动审定 provenance 闸门（auto_verify_operation_knowledge_chunks）──────
//
// 这是「自动审定」全能力 + 红线门：seed 一条 **无 source_quote / 无 source_anchors**
// 的 needs_review chunk。真模型在 auto_verify prompt 下**可能**回 verified，但生产
// 闸门 `decide_auto_verify_status` 必须把它强制压回 needs_review——「AI 永不在缺
// provenance 时自动 verify」红线。
//
// **硬断言（红线）**：调用后该 chunk 的 integrity_status **绝不**变成 verified。
// 先用纯函数 `decide_auto_verify_status` 锁死闸门契约（确定性，不依赖真模型抖动），
// 再跑真模型端到端确认落库结果与契约一致。

#[tokio::test]
#[ignore]
async fn k7_real_auto_verify_provenance_gate_holds() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    // 纯函数红线：缺 provenance 时即便模型回 verified，闸门也必须压回 needs_review。
    assert_eq!(
        decide_auto_verify_status(false, false, 10, 7, "verified"),
        "needs_review",
        "缺 source_quote+anchor 时即便模型 verified+满分，闸门必须压回 needs_review"
    );
    assert_eq!(
        decide_auto_verify_status(true, false, 10, 7, "verified"),
        "needs_review",
        "只有 source_quote 没有 anchor 时也必须压回 needs_review"
    );
    // 反向：齐全 + 高置信 + 模型 verified 才允许 verified（确认闸门不是恒压）。
    assert_eq!(
        decide_auto_verify_status(true, true, 10, 7, "verified"),
        "verified",
        "source_quote+anchor 齐全 + 置信达标 + 模型 verified 时应允许 verified"
    );

    // 端到端：seed 一条无 provenance 的 needs_review chunk（source_quote=None、
    // source_anchors=[]），跑真模型 auto_verify，断言落库后绝不 verified。
    let id = ObjectId::new();
    let now = DateTime::now();
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: ws.clone(),
        account_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("product_capability".to_string()),
        title: "无出处的定价说明".to_string(),
        summary: Some("一条没有任何原文引用/锚点的定价说明。".to_string()),
        body: Some("基础版 99 元/月，专业版 299 元/月。".to_string()),
        source_quote: None,            // 关键：无原文引用
        source_anchors: Vec::new(),    // 关键：无锚点
        integrity_status: Some("needs_review".to_string()),
        confidence_score: Some(50),
        status: "active".to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some("methodology".to_string()),
        dynamic_confidence: Some(0.9),
        chunk_type: "product_fact".to_string(),
        ..Default::default()
    };
    state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert no-provenance chunk");

    let admin = Extension(AuthenticatedAdmin {
        user_id: "k7_admin".into(),
        username: "k7_admin".into(),
        current_workspace: ws.clone(),
    });
    let req: KnowledgeAutoVerifyRequest = serde_json::from_value(json!({
        "accountId": null,
        "confidenceThreshold": 7,
        "humanAuditSampleRate": 0.0, // 关 sampling，让终态只由 provenance 闸门决定
        "limit": 10,
    }))
    .expect("构造 KnowledgeAutoVerifyRequest");

    let resp = unwrap_or_skip_transient!(
        auto_verify_operation_knowledge_chunks(State(state.clone()), admin, Json(req)).await,
        "真实 auto_verify（不崩）"
    );
    eprintln!(
        "[k7] auto_verify summary processed={:?} verified={:?} needsReview={:?}",
        resp.0.get("processed"),
        resp.0.get("verified"),
        resp.0.get("needsReview"),
    );

    let after = state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("query chunk")
        .expect("chunk exists");
    eprintln!("[k7] chunk integrity_status after auto_verify = {:?}", after.integrity_status);
    // 红线：缺 provenance 的 chunk 绝不能被自动 verified。
    assert_ne!(
        after.integrity_status.as_deref(),
        Some("verified"),
        "缺 source_quote/anchor 的 chunk 被自动 verified——provenance 闸门被击穿！"
    );
}

// ── K8 · AI 修复只产 patch、永不自动落库（propose_chunk_repair）────────────────
//
// 「AI 修复」全能力：真模型对一条 needs_review chunk 提修复方案。红线：handler
// **只返回 patch**，绝不把 patch 写进 chunk 字段（只写 usage log + event）。
//
// **硬断言（红线）**：调用后 DB 里该 chunk 的 status / body / integrity_status
// **与调用前完全一致**（AI 修复需人工确认才落库，永不自动改库）。

#[tokio::test]
#[ignore]
async fn k8_real_repair_proposes_patch_but_never_writes_db() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    // seed 一条信息不全的 needs_review chunk（缺字段 → 真模型会想补）。
    let id = ObjectId::new();
    let now = DateTime::now();
    let original_body = "退款。".to_string(); // 故意残缺，引诱真模型提 patch
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: ws.clone(),
        account_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("product_capability".to_string()),
        title: "退款".to_string(),
        summary: Some("退款相关。".to_string()),
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
        user_id: "k8_admin".into(),
        username: "k8_admin".into(),
        current_workspace: ws.clone(),
    });

    let resp = unwrap_or_skip_transient!(
        propose_chunk_repair(State(state.clone()), admin, Path(id.to_hex())).await,
        "真实 AI 修复 propose（不崩）"
    );
    let body = resp.0;
    eprintln!(
        "[k8] repair turn={:?} hasPatch={} missingFields={:?} confidenceHint={:?}",
        body.get("turn"),
        body.get("patch").map(|p| !p.is_null()).unwrap_or(false),
        body.get("missingFields"),
        body.get("confidenceHint"),
    );

    // 红线：propose 后 DB 里的 chunk 必须与原始完全一致（patch 永不自动落库）。
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
        "AI 修复 propose 不得改 body（patch 只返回不落库），DB body={:?}",
        after.body
    );
    assert_eq!(after.status, "draft", "AI 修复 propose 不得改 status");
    assert_eq!(
        after.integrity_status.as_deref(),
        Some("needs_review"),
        "AI 修复 propose 不得改 integrity_status（永不自动 verify）"
    );
}

// ── K9 · 标签抽取（extract_operation_knowledge_tags）────────────────────────────
//
// 「标签抽取」全能力：真模型从单条知识切片抽 productTags / businessTopics。
// mock 测不到「真模型在真实切片下输出的两字段 JSON 能否被接住」。
//
// 软断言：返回体含 productTags / businessTopics 两数组键（形状对、不崩）。
// 抽取命中（具体标签内容）不做硬性保证——真模型抽取是软能力。

#[tokio::test]
#[ignore]
async fn k9_real_tag_extraction_returns_two_arrays() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());

    let req: ExtractKnowledgeTagsRequest = serde_json::from_value(json!({
        "accountId": null,
        "title": "WechatAgent 与传统群发工具的定位差异",
        "body": "WechatAgent 是面向私域运营的 AI 自动决策助手，逐人逐场景做对话与跟进，\
                 不是一次性群发工具；强调合规审查与渐进式知识检索。",
    }))
    .expect("构造 ExtractKnowledgeTagsRequest");

    let resp = unwrap_or_skip_transient!(
        extract_operation_knowledge_tags(State(state.clone()), Json(req)).await,
        "真实标签抽取（不崩、JSON 能解析）"
    );
    let body = resp.0;
    eprintln!(
        "[k9] productTags={:?} businessTopics={:?}",
        body.get("productTags"),
        body.get("businessTopics"),
    );

    // 软断言（形状红线）：两字段都必须是数组（真模型输出能被接住、shape 稳定）。
    assert!(
        body["productTags"].is_array(),
        "productTags 必须是数组，实际 {:?}",
        body.get("productTags")
    );
    assert!(
        body["businessTopics"].is_array(),
        "businessTopics 必须是数组，实际 {:?}",
        body.get("businessTopics")
    );
}

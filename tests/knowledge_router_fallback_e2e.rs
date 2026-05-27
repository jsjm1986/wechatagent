//! `knowledge_router_fallback_e2e` —— 路由 fallback_rank 兜底路径的端到端集成测试。
//!
//! 守 `route_operation_knowledge` 的 fallback 红线（`src/agent/knowledge_router.rs:443`）：
//! 当 knowledge_agent 在预算内未给出 cited（agent 显式返回 0 cited 或 3 轮兜底空集）时，
//! 路由必须按 `wiki_type_priority × dynamic_confidence` 在已加载的 verified corpus 上
//! 静态排序，取 top-N=5 作为弱证据回填，并标 `risk_level=medium`、
//! `knowledge_coverage=weak`、`tool_trace.fallback_rank=agent_returned_zero_cited`，
//! 让 grounding 闸不至于 missing、Reply Agent / 审计感知"这是弱兜底"。
//!
//! 三个场景：
//! 1. **agent 给 0 cited → fallback 触发**：8 条 chunk 全 verified，agent 回 answer
//!    且 cited=[]，路由必须取 top-5（按 wiki_type_priority 倒排）。
//! 2. **agent 给的 cited 不在 corpus → fallback 触发**：agent 输出一条不存在的
//!    chunk_id（OOB），filter_in_corpus 后为空，必须走 fallback。
//! 3. **corpus 真的空 → 维持 missing**：0 个 verified chunk，整个路径 short-circuit
//!    在 `route_operation_knowledge` 头部 `documents.is_empty() && chunks.is_empty()`，
//!    返回 `coverage=missing`、不进入 LLM。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::agent::test_knowledge_route_for_contact;
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

const ACCOUNT: &str = "default"; // 与 TestApp 的 default_workspace_id 对齐

fn verified_chunk(workspace_id: &str, title: &str, wiki_type: &str, conf: f64) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: None, // workspace 共享 chunk
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title}")),
        body: Some(format!("正文：{title}")),
        wiki_type: Some(wiki_type.to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        dynamic_confidence: Some(conf),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

async fn insert(app: &TestApp, chunks: &[OperationKnowledgeChunk]) {
    for c in chunks {
        app.state
            .db
            .operation_knowledge_chunks()
            .insert_one(c, None)
            .await
            .expect("insert chunk");
    }
}

fn ws(app: &TestApp) -> String {
    app.state.config.default_workspace_id.clone()
}

/// 场景 1：8 条 verified chunk + agent 显式返回 0 cited → fallback_rank 触发，
/// selectedChunks 命中 top-5（按 wiki_type 优先级倒排：thesis > methodology > entity > source）。
#[tokio::test]
#[ignore]
async fn router_falls_back_to_top_n_when_agent_cites_nothing() {
    let app = TestApp::start().await;
    let workspace = ws(&app);

    // 8 条覆盖 4 类 wiki_type，confidence 用区分顺序。优先级：thesis > methodology >
    // entity > source。每类 2 条便于断言 top-5 = thesis*2 + methodology*2 + entity*1。
    let chunks = vec![
        verified_chunk(&workspace, "thesis-A", "thesis", 0.9),
        verified_chunk(&workspace, "thesis-B", "thesis", 0.8),
        verified_chunk(&workspace, "methodology-A", "methodology", 0.95),
        verified_chunk(&workspace, "methodology-B", "methodology", 0.85),
        verified_chunk(&workspace, "entity-A", "entity", 0.9),
        verified_chunk(&workspace, "entity-B", "entity", 0.7),
        verified_chunk(&workspace, "source-A", "source", 0.95),
        verified_chunk(&workspace, "source-B", "source", 0.6),
    ];
    insert(&app, &chunks).await;

    // 轮 1：agent 直接给 answer，但 cited=[] 且 sourceQuotes=[] —— 模拟"探索完了
    // 没找到合适内容"。
    app.llm.push_response(json!({
        "action": "answer",
        "answer": "暂无足够依据。",
        "citedChunkIds": [],
        "sourceQuotes": [],
    }));

    let result = test_knowledge_route_for_contact(&app.state, None, ACCOUNT, "随便问个问题")
        .await
        .expect("route");

    let route = result.get_document("route").expect("route doc");
    let selected: Vec<String> = route
        .get_array("selectedChunkIds")
        .expect("selectedChunkIds")
        .iter()
        .map(|b| b.as_str().expect("hex id").to_string())
        .collect();

    assert_eq!(
        selected.len(),
        5,
        "fallback 必须取 FALLBACK_TOP_N=5, got {selected:?}"
    );
    assert_eq!(
        route.get_str("knowledgeCoverage").ok(),
        Some("weak"),
        "fallback 路径必须标 weak"
    );
    assert_eq!(
        route.get_str("riskLevel").ok(),
        Some("medium"),
        "fallback 路径必须降级到 medium 风险"
    );

    // 优先级断言：top-5 应全部来自 thesis + methodology + entity（不应有 source）
    let by_id: std::collections::HashMap<String, &OperationKnowledgeChunk> = chunks
        .iter()
        .map(|c| (c.id.expect("oid").to_hex(), c))
        .collect();
    let selected_types: Vec<&str> = selected
        .iter()
        .map(|id| by_id.get(id).expect("known id").wiki_type.as_deref().unwrap_or(""))
        .collect();
    assert!(
        selected_types.iter().all(|&t| matches!(t, "thesis" | "methodology" | "entity")),
        "top-5 必须按 wiki_type_priority 倒排，不应包含低优先级 source, got {selected_types:?}"
    );
    assert_eq!(
        selected_types.iter().filter(|&&t| t == "thesis").count(),
        2,
        "thesis 优先级最高，应全选"
    );
    assert_eq!(
        selected_types.iter().filter(|&&t| t == "methodology").count(),
        2,
        "methodology 第二档，应全选"
    );

    // tool_trace 必须含 fallback_rank 标记
    let trace = route
        .get_array("toolTrace")
        .expect("toolTrace")
        .iter()
        .filter_map(|b| b.as_document())
        .find(|d| d.get_str("tool").ok() == Some("fallback_rank"))
        .expect("toolTrace 必须含 fallback_rank");
    assert_eq!(
        trace.get_str("reason").ok(),
        Some("agent_returned_zero_cited"),
        "fallback reason 必须可观测"
    );
    assert_eq!(trace.get_i32("selected").ok(), Some(5));
}

/// 场景 2：agent 给的 cited 全部不在 corpus（OOB chunk_id）→ filter 后为空 →
/// fallback 触发（与场景 1 路径一致，但起因不同）。守住"agent 不能凭空创造 cited
/// 来绕过 fallback"。
#[tokio::test]
#[ignore]
async fn router_falls_back_when_agent_cites_chunks_outside_corpus() {
    let app = TestApp::start().await;
    let workspace = ws(&app);

    let real = verified_chunk(&workspace, "real-methodology", "methodology", 0.9);
    let real_hex = real.id.expect("oid").to_hex();
    insert(&app, &[real]).await;

    // 关键：knowledge_agent 内部有 filter_answer_against_opened（只保留 opened_seen
    // 子集）。如果 agent 直接 answer 没 open 过任何 chunk，cited 会被全部过滤掉，
    // 与"agent 给 0 cited"等效，正好驱动场景 2。
    let bogus = ObjectId::new().to_hex();
    app.llm.push_response(json!({
        "action": "answer",
        "answer": "我猜是这条。",
        "citedChunkIds": [bogus],
        "sourceQuotes": [{
            "chunkId": bogus,
            "quote": "假证据",
            "sourceAnchorIndex": null,
        }],
    }));

    let result = test_knowledge_route_for_contact(&app.state, None, ACCOUNT, "查个东西")
        .await
        .expect("route");
    let route = result.get_document("route").expect("route doc");
    let selected: Vec<String> = route
        .get_array("selectedChunkIds")
        .expect("ids")
        .iter()
        .map(|b| b.as_str().expect("hex").to_string())
        .collect();

    assert_eq!(
        selected, vec![real_hex],
        "fallback 应回填 corpus 中唯一的 chunk"
    );
    assert_eq!(route.get_str("knowledgeCoverage").ok(), Some("weak"));
    assert_eq!(route.get_str("riskLevel").ok(), Some("medium"));

    let has_fallback = route
        .get_array("toolTrace")
        .expect("toolTrace")
        .iter()
        .filter_map(|b| b.as_document())
        .any(|d| d.get_str("tool").ok() == Some("fallback_rank"));
    assert!(has_fallback, "toolTrace 必须含 fallback_rank");
}

/// 场景 3：corpus 真的为空（0 verified chunk）→ short-circuit `coverage=missing`，
/// 不进入 LLM 也不进 fallback。守住"空知识库不要假装有兜底"。
#[tokio::test]
#[ignore]
async fn router_returns_missing_when_corpus_completely_empty() {
    let app = TestApp::start().await;

    // 不入队任何 LLM 响应；如果代码错误地走到 knowledge_agent / LLM 都会报错。

    let result = test_knowledge_route_for_contact(&app.state, None, ACCOUNT, "什么都没有")
        .await
        .expect("route");

    let route = result.get_document("route").expect("route doc");
    assert_eq!(
        route.get_str("knowledgeCoverage").ok(),
        Some("missing"),
        "空 corpus 必须 missing"
    );
    let selected = route
        .get_array("selectedChunkIds")
        .expect("ids");
    assert!(selected.is_empty(), "missing 路径不应选任何 chunk");
    assert_eq!(app.llm.calls(), 0, "空 corpus 不应触达 LLM");
}

//! `knowledge_window_cited_integration` —— B5 知识窗口错位修复的端到端集成测试。
//!
//! 守的红线：运行时静态窗口（`load_operation_knowledge` 按 priority/updated_at
//! 倒排取 top-200）只是**注入快照**，不是 verified 全集的边界。knowledge_agent 的
//! `open_chunk` 按 `_id` 直查、可以合法打开并引用窗外的 verified chunk（cite ⊆
//! opened 由 `filter_answer_against_opened_chunks` 保证）。router 必须按 id 直查
//! DB 复核这批 cited（workspace + domain + status=active + integrity=verified +
//! account 归属，与窗口过滤同口径），而不是与窗口求交——否则窗外合法引用被降格成
//! fallback 弱回填，且其文档进不了 `select_operation_knowledge_chunks` →
//! `compute_verified_chunks`（R5.4 产品背书）下游。
//!
//! 两个场景：
//! 1. **>200 条 verified、agent 引用第 201 名（窗外）**：selectedChunkIds 必须
//!    保留该引用（非 fallback、coverage=enough），且 selectedChunks 里能拿到该
//!    chunk 的完整文档（下游 verified 计算的输入面）。
//! 2. **窗内引用回归守卫**：行为与修复前逐字一致；且 route 持久化投影里**没有**
//!    `citedVerifiedChunks` 键（运行时载体不进落库面）。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::agent::test_knowledge_route_for_contact;
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

const ACCOUNT: &str = "default"; // 与 TestApp 的 default_workspace_id 对齐

/// 一条可被 agent 引用的 verified chunk：body / source_quote / source_anchors
/// 三处携带同一句原文证据，满足 `quote_is_chunk_evidence` 的锚点校验。
fn verified_chunk(workspace_id: &str, title: &str, evidence: &str, priority: i32) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: None, // workspace 共享 chunk
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title}")),
        body: Some(format!("正文：{evidence}")),
        source_quote: Some(evidence.to_string()),
        source_anchors: vec![doc! { "sourceQuote": evidence }],
        wiki_type: Some("entity".to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        dynamic_confidence: Some(0.9),
        priority,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

fn ws(app: &TestApp) -> String {
    app.state.config.default_workspace_id.clone()
}

/// 把 agent 脚本压入 mock LLM：轮 1 open_chunk 目标，轮 2 answer 带锚定引用。
fn push_open_then_cite(app: &TestApp, target_hex: &str, evidence: &str) {
    app.llm.push_response(json!({
        "action": "open_chunk",
        "ids": [target_hex],
    }));
    app.llm.push_response(json!({
        "action": "answer",
        "answer": "冷门产品X的标准价格有 verified 依据。",
        "citedChunkIds": [target_hex],
        "sourceQuotes": [{
            "chunkId": target_hex,
            "quote": evidence,
            "sourceAnchorIndex": 0,
        }],
    }));
}

/// 场景 1：201 条 verified，目标 chunk priority 最低（排第 201 名、在静态 top-200
/// 窗口之外）。agent open 并引用它后，router 必须按 DB 直查复核保留该引用：
/// 不降格 fallback、文档进入 selectedChunks（下游 verified 计算输入面）。
#[tokio::test]
#[ignore]
async fn agent_cited_verified_chunk_outside_window_is_not_degraded() {
    let app = TestApp::start().await;
    let workspace = ws(&app);

    // 200 条高优先级 chunk 占满窗口（priority=10），目标 chunk priority=1 → 窗外。
    let window_chunks: Vec<OperationKnowledgeChunk> = (0..200)
        .map(|i| {
            verified_chunk(
                &workspace,
                &format!("窗内知识-{i}"),
                &format!("窗内知识 {i} 的原文句。"),
                10,
            )
        })
        .collect();
    let evidence = "冷门产品X的标准价格为 3999 元/年。";
    let target = verified_chunk(&workspace, "冷门产品X定价", evidence, 1);
    let target_hex = target.id.expect("oid").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many(window_chunks.iter().chain(std::iter::once(&target)), None)
        .await
        .expect("insert chunks");

    push_open_then_cite(&app, &target_hex, evidence);

    let result = test_knowledge_route_for_contact(
        &app.state,
        None,
        &app.state.config.default_workspace_id,
        ACCOUNT,
        "冷门产品X的价格是多少",
    )
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
        selected,
        vec![target_hex.clone()],
        "agent 合法引用的窗外 verified chunk 必须保留为 selected，而非降格 fallback"
    );
    assert_eq!(
        route.get_str("knowledgeCoverage").ok(),
        Some("enough"),
        "带锚定 sourceQuote 的真实 citation 必须是 enough"
    );
    assert_eq!(route.get_str("riskLevel").ok(), Some("low"));
    assert_eq!(
        route.get_bool("selectedChunksAreFallback").ok(),
        Some(false),
        "真实 citation 绝不能标成 fallback——否则 route_used_knowledge_ids 掐死合法产品背书"
    );
    let has_fallback = route
        .get_array("toolTrace")
        .expect("toolTrace")
        .iter()
        .filter_map(|b| b.as_document())
        .any(|d| d.get_str("tool").ok() == Some("fallback_rank"));
    assert!(!has_fallback, "窗外合法引用不得触发 fallback_rank");

    // 下游输入面：selectedChunks（gateway 传给 compute_verified_chunks 的同一投影）
    // 必须携带窗外 chunk 的完整文档。
    let selected_docs = result.get_array("selectedChunks").expect("selectedChunks");
    let carried = selected_docs
        .iter()
        .filter_map(|b| b.as_document())
        .find(|d| d.get_str("id").ok() == Some(target_hex.as_str()))
        .expect("窗外 cited chunk 的文档必须进入 selectedChunks（否则 R5.4 verified 计算拿不到它）");
    assert_eq!(
        carried.get_str("integrityStatus").ok(),
        Some("verified"),
        "携带的文档必须是 verified 原件"
    );
    assert!(
        carried
            .get_str("body")
            .unwrap_or_default()
            .contains("3999"),
        "携带的文档必须是完整正文，供 prompt 注入与背书计算"
    );

    // 运行时载体不进持久化面：route 的 to_document 投影里不能出现该键。
    assert!(
        !route.contains_key("citedVerifiedChunks"),
        "cited_verified_chunks 是 serde(skip) 运行时字段，绝不能进 route 落库投影"
    );
}

/// 场景 2：窗内引用回归守卫——修复不得改变窗内 citation 的既有行为。
#[tokio::test]
#[ignore]
async fn agent_cited_chunk_inside_window_behavior_unchanged() {
    let app = TestApp::start().await;
    let workspace = ws(&app);

    let evidence = "主打产品Y的标准价格为 1999 元/年。";
    let target = verified_chunk(&workspace, "主打产品Y定价", evidence, 10);
    let target_hex = target.id.expect("oid").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&target, None)
        .await
        .expect("insert chunk");

    push_open_then_cite(&app, &target_hex, evidence);

    let result = test_knowledge_route_for_contact(
        &app.state,
        None,
        &app.state.config.default_workspace_id,
        ACCOUNT,
        "主打产品Y的价格是多少",
    )
    .await
    .expect("route");

    let route = result.get_document("route").expect("route doc");
    let selected: Vec<String> = route
        .get_array("selectedChunkIds")
        .expect("selectedChunkIds")
        .iter()
        .map(|b| b.as_str().expect("hex id").to_string())
        .collect();
    assert_eq!(selected, vec![target_hex.clone()], "窗内引用照常保留");
    assert_eq!(route.get_str("knowledgeCoverage").ok(), Some("enough"));
    assert_eq!(route.get_bool("selectedChunksAreFallback").ok(), Some(false));
    let selected_docs = result.get_array("selectedChunks").expect("selectedChunks");
    assert!(
        selected_docs
            .iter()
            .filter_map(|b| b.as_document())
            .any(|d| d.get_str("id").ok() == Some(target_hex.as_str())),
        "窗内 cited chunk 的文档照常进入 selectedChunks"
    );
    assert!(!route.contains_key("citedVerifiedChunks"));
}

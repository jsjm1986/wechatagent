//! `knowledge_ask_e2e` —— Agent-first 渐进式披露主循环的端到端集成测试。
//!
//! 覆盖 `agent::knowledge_agent::answer` 在真实 testcontainers MongoDB +
//! mock LLM 下的四种关键路径：
//!
//! 1. **正常路径**：corpus 含相关 chunk → mock LLM 输出
//!    `list_catalog → open_chunk → answer`，最终返回非空 answer + cited 子集。
//! 2. **空 corpus**：workspace 无任何 verified chunk → 立即返回
//!    "知识库无相关内容。"，0 LLM 调用，rounds_used=0。
//! 3. **rounds_used 真实上报**：mock LLM 始终输出 `list_catalog`（不收敛到
//!    answer），4 轮耗尽后兜底 answer 必须 `truncated=true`、`rounds_used=4`、
//!    LLM 真实被调用 4 次（非 0 / max_rounds 默认值）。
//! 4. **未 verified 不可见**：corpus 仅 `integrity_status=needs_review` chunk
//!    → catalog 必为空，行为与场景 2 一致（放在 list_catalog/open_chunk
//!    的 verified-only 守门上验证）。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::agent::knowledge_agent::{answer, AnswerRequest, CatalogFilter};
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

const WS: &str = "ws_ask_e2e";

fn verified_chunk(title: &str, body: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: WS.to_string(),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title}")),
        body: Some(body.to_string()),
        wiki_type: Some("methodology".to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        dynamic_confidence: Some(0.9),
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

fn req(query: &str) -> AnswerRequest {
    AnswerRequest {
        workspace_id: WS.to_string(),
        account_id: None,
        query: query.to_string(),
        filter: CatalogFilter::default(),
        max_rounds: None,
    }
}

/// 场景 1：corpus 含相关 chunk → list_catalog → open_chunk → answer 收敛。
#[tokio::test]
#[ignore]
async fn ask_returns_answer_with_cited_when_corpus_has_relevant_chunks() {
    let app = TestApp::start().await;

    let chunk = verified_chunk(
        "三步价格异议处理",
        "Step1 共情；Step2 说价值；Step3 给方案。",
    );
    let chunk_hex = chunk.id.expect("oid").to_hex();
    insert(&app, &[chunk]).await;

    // 轮 1：LLM 让我们 open chunk
    app.llm.push_response(json!({
        "action": "open_chunk",
        "ids": [chunk_hex.clone()],
    }));
    // 轮 2：LLM 给出最终 answer
    app.llm.push_response(json!({
        "action": "answer",
        "answer": "三步：共情 → 说价值 → 给方案。",
        "citedChunkIds": [chunk_hex.clone()],
        "sourceQuotes": [{
            "chunkId": chunk_hex.clone(),
            "quote": "Step1 共情；Step2 说价值；Step3 给方案。",
            "sourceAnchorIndex": null,
        }],
    }));

    let result = answer(&app.state, req("价格异议怎么处理"))
        .await
        .expect("answer");

    assert!(!result.answer.is_empty(), "answer 必须非空");
    assert_eq!(
        result.cited_chunk_ids,
        vec![chunk_hex.clone()],
        "cited_chunk_ids 必须命中 opened chunk"
    );
    assert_eq!(result.rounds_used, 2, "实际跑了 2 轮（open + answer）");
    assert!(!result.truncated, "正常收敛不应 truncated");
    assert_eq!(app.llm.calls(), 2, "LLM 必须正好被调 2 次");

    // tool_trace 必须按 list_catalog → open_chunk → answer 顺序出现
    let tools: Vec<String> = result
        .tool_trace
        .iter()
        .filter_map(|d| d.get_str("tool").ok().map(str::to_string))
        .collect();
    assert_eq!(tools, vec!["list_catalog", "open_chunk", "answer"]);
}

/// 场景 2：corpus 完全空 → 立即返回固定文案，0 LLM 调用。
#[tokio::test]
#[ignore]
async fn ask_returns_no_relevant_when_corpus_empty() {
    let app = TestApp::start().await;

    // 不入队任何 LLM 响应；如果代码错误地调 LLM，pop_or_error 会立即报错。

    let result = answer(&app.state, req("任何 query"))
        .await
        .expect("answer");

    assert_eq!(result.answer, "知识库无相关内容。");
    assert!(result.cited_chunk_ids.is_empty());
    assert!(result.source_quotes.is_empty());
    assert_eq!(result.rounds_used, 0, "空 corpus 立即返回，未进入循环");
    assert!(!result.truncated);
    assert_eq!(app.llm.calls(), 0, "空 corpus 不应触达 LLM");

    // tool_trace 仍记录第一次 list_catalog（returned=0）以便审计
    let first = result.tool_trace.first().expect("至少一条 list_catalog");
    assert_eq!(first.get_str("tool").ok(), Some("list_catalog"));
    assert_eq!(first.get_i32("returned").ok(), Some(0));
}

/// 场景 3：LLM 始终不 answer → 兜底 truncated；rounds_used=4 反映真实轮数，
/// 而不是默认值或 max_rounds 常量。
#[tokio::test]
#[ignore]
async fn ask_falls_back_to_truncated_when_llm_never_emits_answer() {
    let app = TestApp::start().await;

    let chunk = verified_chunk("方法论 A", "正文 A");
    insert(&app, &[chunk]).await;

    // 四轮都返回 list_catalog（不收敛 answer）。第 5 轮不会发生：MAX_ROUNDS=4。
    for _ in 0..4 {
        app.llm.push_response(json!({
            "action": "list_catalog",
            "filter": {},
        }));
    }

    let result = answer(&app.state, req("question"))
        .await
        .expect("answer");

    assert!(result.truncated, "4 轮未 answer 必须 truncated=true");
    assert_eq!(
        result.rounds_used, 4,
        "rounds_used 必须如实上报 4，而不是 0/max_rounds 占位"
    );
    assert_eq!(app.llm.calls(), 4, "LLM 应正好被调 max_rounds 次");
    assert!(
        result.cited_chunk_ids.is_empty(),
        "未 open 任何 chunk 时兜底 cited 为空"
    );
    // 兜底 answer 行也必须落到 trace 上，便于前端显示
    let last = result.tool_trace.last().expect("至少一条 trace");
    assert_eq!(last.get_str("tool").ok(), Some("answer"));
    assert_eq!(last.get_bool("truncated").ok(), Some(true));
}

/// 场景 4：corpus 只含 integrity_status=needs_review 的 chunk → catalog 仍然空，
/// 行为与场景 2 一致。守的是 list_catalog 的 verified-only 红线（防回归）。
#[tokio::test]
#[ignore]
async fn ask_skips_unverified_chunks_in_catalog() {
    let app = TestApp::start().await;

    let mut chunk = verified_chunk("草稿方法论", "正文");
    chunk.integrity_status = Some("needs_review".to_string());
    insert(&app, &[chunk]).await;

    let result = answer(&app.state, req("query"))
        .await
        .expect("answer");

    assert_eq!(
        result.answer, "知识库无相关内容。",
        "needs_review chunk 不应进入 catalog"
    );
    assert_eq!(app.llm.calls(), 0, "catalog 空 → 不调 LLM");
    assert_eq!(result.rounds_used, 0);
}

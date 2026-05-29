//! `knowledge_ask_stream_e2e` —— [`answer_streaming`] SSE 事件流的端到端集成测试。
//!
//! 守的是 `src/agent/knowledge_agent.rs::answer_streaming` 与
//! `src/routes/knowledge.rs::ask_knowledge_stream` 的 trace → final 一致性红线：
//!   1. 每个 `tool_trace.push` 必须配对推一条 `TraceEvent::Step`；
//!   2. 跑完后必须推一条 `TraceEvent::Final`，其 `AnswerResult` 与 `answer()`
//!      非流式版本完全等价（同 query / 同 corpus 下 `cited_chunk_ids`/`rounds_used`
//!      不变）；
//!   3. 不再有任何事件——`tx` 被 drop 后 `rx.recv()` 返回 None。
//!
//! 三个场景：
//! 1. **正常路径**：corpus 有相关 chunk → events: Step(list_catalog) →
//!    Step(open_chunk) → Step(answer) → Final → close。
//! 2. **空 corpus**：returned=0 → 立即 Step(list_catalog) → Final（answer="知识库
//!    无相关内容。"）→ close。0 LLM 调用。
//! 3. **truncated**：3 轮 list_catalog 始终不 answer → 末尾 Step(answer truncated=true)
//!    → Final（truncated=true）→ close。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use mongodb::bson::{oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use tokio::sync::mpsc;
use wechatagent::agent::knowledge_agent::{
    answer_streaming, AnswerRequest, CatalogFilter, TraceEvent,
};
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

const WS: &str = "ws_ask_stream_e2e";

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

/// 抽出 step.payload 里的 `tool` 字段；非 Step 事件返回 None。
fn step_tool(ev: &TraceEvent) -> Option<String> {
    match ev {
        TraceEvent::Step { payload } => payload
            .get("tool")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        TraceEvent::Token { .. } => None,
        TraceEvent::Final { .. } => None,
    }
}

/// 收完所有事件并返回 (events, final_answer)。
/// `final_answer` 是 None 表示没收到 Final——视为协议红线违规。
async fn drain(rx: &mut mpsc::UnboundedReceiver<TraceEvent>) -> (Vec<TraceEvent>, Option<String>) {
    let mut events = Vec::new();
    let mut final_answer = None;
    while let Some(ev) = rx.recv().await {
        if let TraceEvent::Final { answer } = &ev {
            final_answer = Some(answer.answer.clone());
        }
        events.push(ev);
    }
    (events, final_answer)
}

/// 把所有 [`TraceEvent::Token`] 的 delta 拼成一整串（前端实际渲染的增量正文）。
fn concat_tokens(events: &[TraceEvent]) -> String {
    events
        .iter()
        .filter_map(|ev| match ev {
            TraceEvent::Token { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

/// 场景 1：正常路径 — Step(list_catalog) → Step(open_chunk) → Step(answer) → Final。
/// 与 `tests/knowledge_ask_e2e.rs::ask_returns_answer_with_cited_when_corpus_has_relevant_chunks`
/// 共享 corpus 形态，断言流式版本与非流式版本一致。
#[tokio::test]
#[ignore]
async fn stream_emits_step_per_tool_trace_then_final_on_happy_path() {
    let app = TestApp::start().await;

    let chunk = verified_chunk(
        "三步价格异议处理",
        "Step1 共情；Step2 说价值；Step3 给方案。",
    );
    let chunk_hex = chunk.id.expect("oid").to_hex();
    insert(&app, &[chunk]).await;

    // 与 ask_e2e 同形：第 1 轮 open_chunk，第 2 轮 answer。
    app.llm.push_response(json!({
        "action": "open_chunk",
        "ids": [chunk_hex.clone()],
    }));
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

    let (tx, mut rx) = mpsc::unbounded_channel::<TraceEvent>();
    let result = answer_streaming(&app.state, req("价格异议怎么处理"), tx, None)
        .await
        .expect("answer_streaming");

    let (events, final_answer) = drain(&mut rx).await;

    // tool_trace 与 events 一一对应：tool_trace 长度 = events 中 Step 数量。
    let step_count = events
        .iter()
        .filter(|ev| matches!(ev, TraceEvent::Step { .. }))
        .count();
    assert_eq!(
        step_count,
        result.tool_trace.len(),
        "Step 数量必须 = tool_trace 长度，确保红线：每个 push 配对一条 emit"
    );

    // tool 顺序：list_catalog → open_chunk → answer
    let tools: Vec<String> = events.iter().filter_map(step_tool).collect();
    assert_eq!(
        tools,
        vec!["list_catalog", "open_chunk", "answer"],
        "正常路径事件顺序应一致：list_catalog → open_chunk → answer"
    );

    // 末尾必须有 Final，且其 answer 与同步返回值完全等价。
    assert_eq!(
        final_answer.as_deref(),
        Some(result.answer.as_str()),
        "Final.answer 必须等于 answer_streaming 同步返回值"
    );
    assert!(
        matches!(events.last(), Some(TraceEvent::Final { .. })),
        "最后一条事件必须是 Final"
    );
}

/// 场景 2：空 corpus — Step(list_catalog returned=0) → Final（fixed text）→ close。0 LLM 调用。
#[tokio::test]
#[ignore]
async fn stream_emits_final_immediately_when_corpus_empty() {
    let app = TestApp::start().await;
    // 不入队 LLM，corpus 为空时根本不应该调用 generate_agent_json。

    let (tx, mut rx) = mpsc::unbounded_channel::<TraceEvent>();
    let result = answer_streaming(&app.state, req("任意 query"), tx, None)
        .await
        .expect("answer_streaming");

    let (events, final_answer) = drain(&mut rx).await;

    assert_eq!(result.answer, "知识库无相关内容。");
    assert_eq!(result.rounds_used, 0);
    assert!(!result.truncated);
    assert_eq!(app.llm.calls(), 0, "空 corpus 不应触达 LLM");

    let tools: Vec<String> = events.iter().filter_map(step_tool).collect();
    assert_eq!(
        tools,
        vec!["list_catalog"],
        "空 corpus 下只应推一条 list_catalog Step"
    );
    assert_eq!(
        final_answer.as_deref(),
        Some("知识库无相关内容。"),
        "Final.answer 必须复制固定空 corpus 文案"
    );
}

/// 场景 3：3 轮 list_catalog 不收敛 → 末尾 Step(answer truncated=true) → Final(truncated=true)。
#[tokio::test]
#[ignore]
async fn stream_emits_truncated_when_llm_never_emits_answer() {
    let app = TestApp::start().await;

    let chunk = verified_chunk("方法论 A", "正文 A");
    insert(&app, &[chunk]).await;

    for _ in 0..3 {
        app.llm.push_response(json!({
            "action": "list_catalog",
            "filter": {},
        }));
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<TraceEvent>();
    let result = answer_streaming(&app.state, req("question"), tx, None)
        .await
        .expect("answer_streaming");

    let (events, _final_answer) = drain(&mut rx).await;

    assert!(result.truncated, "3 轮未 answer 必须 truncated=true");
    assert_eq!(result.rounds_used, 3);
    assert_eq!(app.llm.calls(), 3);

    // 末尾 Step 必须是 answer 且 truncated=true。
    let last_step = events
        .iter()
        .rev()
        .find_map(|ev| match ev {
            TraceEvent::Step { payload } => Some(payload),
            TraceEvent::Token { .. } => None,
            TraceEvent::Final { .. } => None,
        })
        .expect("应至少有一条 Step");
    assert_eq!(
        last_step.get("tool").and_then(|v| v.as_str()),
        Some("answer"),
        "最后一条 Step 必须是 answer 行"
    );
    assert_eq!(
        last_step.get("truncated").and_then(|v| v.as_bool()),
        Some(true),
        "兜底 answer 行必须标 truncated=true"
    );

    // 最后一条事件必须是 Final，且 truncated=true。
    match events.last() {
        Some(TraceEvent::Final { answer }) => {
            assert!(answer.truncated, "Final.answer.truncated 必须 true");
        }
        _ => panic!("最后一条事件必须是 Final"),
    }
}

/// 场景 4：客户端断开 → cancel 标志位翻 true → agent 在循环顶部检测到 →
/// push `cancelled` Step → 走兜底 + Final(cancelled=true, truncated=true)。
///
/// 不入队 LLM 响应：cancel 在第 1 轮顶部就命中，根本走不到 generate_agent_json。
/// 这是软取消语义验证：不强 abort 正在跑的 LLM call，但下一轮不会启动。
#[tokio::test]
#[ignore]
async fn stream_emits_cancelled_when_cancel_flag_set_before_loop() {
    let app = TestApp::start().await;

    let chunk = verified_chunk("方法论 A", "正文 A");
    insert(&app, &[chunk]).await;

    let cancel = Arc::new(AtomicBool::new(false));
    cancel.store(true, Ordering::Relaxed); // 一开始就翻 true，模拟客户端立即断开

    let (tx, mut rx) = mpsc::unbounded_channel::<TraceEvent>();
    let result = answer_streaming(&app.state, req("question"), tx, Some(cancel))
        .await
        .expect("answer_streaming");

    let (events, _final_answer) = drain(&mut rx).await;

    assert!(result.cancelled, "cancel 翻 true 必须导致 cancelled=true");
    assert!(result.truncated, "cancel 也走兜底，必须 truncated=true");
    assert_eq!(result.rounds_used, 0, "cancel 在第一轮顶部命中，未跑完任何轮");
    assert_eq!(app.llm.calls(), 0, "cancel 在 LLM 调用前命中，0 LLM 调用");

    // 事件序列：list_catalog → cancelled → answer(truncated, cancelled) → Final
    let tools: Vec<String> = events
        .iter()
        .filter_map(|ev| match ev {
            TraceEvent::Step { payload } => payload
                .get("tool")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            TraceEvent::Token { .. } => None,
            TraceEvent::Final { .. } => None,
        })
        .collect();
    assert_eq!(
        tools,
        vec!["list_catalog", "cancelled", "answer"],
        "cancel 路径事件序列应为 list_catalog → cancelled → answer"
    );

    // Final.answer.cancelled 必须 true。
    match events.last() {
        Some(TraceEvent::Final { answer }) => {
            assert!(answer.cancelled, "Final.answer.cancelled 必须 true");
            assert!(answer.truncated, "Final.answer.truncated 必须 true");
        }
        _ => panic!("最后一条事件必须是 Final"),
    }
}

/// 场景 5（P1-3 真流式断言）：answer 轮必须通过 [`TraceEvent::Token`] 把**解码后的
/// 答案正文**逐段推出，且拼接结果恰好等于最终 `AnswerResult.answer`——不多 JSON 语法、
/// 不少正文字符。这是"真上游 SSE → AnswerStreamer 抽取顶层 answer 字段"链路的守门：
/// TestLlmGenerator 的默认 `generate_json_streaming` 把整段答案 JSON 作为一个 raw
/// 片段推入，下游 AnswerStreamer 解码出 answer 正文 → 至少 1 个非空 Token。
///
/// 关键不变量：
///   1. answer 轮至少产 1 个 Token（正文非空时）；
///   2. 所有 Token delta 拼接 == Final.answer（前端增量渲染 == 最终答案）；
///   3. 工具轮（open_chunk / list_catalog）不产 Token（无 answer 字段）。
#[tokio::test]
#[ignore]
async fn stream_emits_decoded_answer_tokens_matching_final() {
    let app = TestApp::start().await;

    let chunk = verified_chunk(
        "三步价格异议处理",
        "Step1 共情；Step2 说价值；Step3 给方案。",
    );
    let chunk_hex = chunk.id.expect("oid").to_hex();
    insert(&app, &[chunk]).await;

    // 第 1 轮 open_chunk（工具轮，无 answer 字段 → 0 Token）；
    // 第 2 轮 answer（含中文正文 → 解码出 Token）。
    app.llm.push_response(json!({
        "action": "open_chunk",
        "ids": [chunk_hex.clone()],
    }));
    let final_text = "三步：共情 → 说价值 → 给方案。";
    app.llm.push_response(json!({
        "action": "answer",
        "answer": final_text,
        "citedChunkIds": [chunk_hex.clone()],
        "sourceQuotes": [{
            "chunkId": chunk_hex.clone(),
            "quote": "Step1 共情；Step2 说价值；Step3 给方案。",
            "sourceAnchorIndex": null,
        }],
    }));

    let (tx, mut rx) = mpsc::unbounded_channel::<TraceEvent>();
    let result = answer_streaming(&app.state, req("价格异议怎么处理"), tx, None)
        .await
        .expect("answer_streaming");

    let (events, final_answer) = drain(&mut rx).await;

    // 至少 1 个 Token 事件。
    let token_count = events
        .iter()
        .filter(|ev| matches!(ev, TraceEvent::Token { .. }))
        .count();
    assert!(
        token_count >= 1,
        "answer 轮正文非空时必须至少产 1 个 Token，实际 {token_count}",
    );

    // 拼接的 Token == 最终答案 == Final.answer。
    let streamed = concat_tokens(&events);
    assert_eq!(
        streamed, result.answer,
        "Token 拼接必须等于 AnswerResult.answer（不漏字 / 不混入 JSON 语法）",
    );
    assert_eq!(
        Some(streamed.as_str()),
        final_answer.as_deref(),
        "Token 拼接必须等于 Final.answer",
    );
    assert_eq!(result.answer, final_text, "最终答案应为模型给的正文");

    // 末尾必须是 Final，且 Final 在所有 Token 之后（增量先于终帧）。
    assert!(
        matches!(events.last(), Some(TraceEvent::Final { .. })),
        "最后一条事件必须是 Final",
    );
    let last_token_idx = events
        .iter()
        .rposition(|ev| matches!(ev, TraceEvent::Token { .. }));
    let final_idx = events
        .iter()
        .rposition(|ev| matches!(ev, TraceEvent::Final { .. }));
    if let (Some(lt), Some(f)) = (last_token_idx, final_idx) {
        assert!(lt < f, "所有 Token 必须在 Final 之前");
    }
}

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
//! 5. **D3 关系图谱**：follow_relations 按 relation_kind 分流（references=支撑、
//!    contradicts=标记反例跟随）；open_chunk / follow_relations 命中已被
//!    superseded 的 chunk 时 redirect 到现行版本。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::agent::knowledge_agent::{answer, AnswerRequest, CatalogFilter};
use wechatagent::models::{OperationKnowledgeChunk, RelatedRef};

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
        source_quote: Some(body.to_string()),
        source_anchors: vec![doc! { "sourceQuote": body }],
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
            "sourceAnchorIndex": 0,
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

    let result = answer(&app.state, req("任何 query")).await.expect("answer");

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

    let result = answer(&app.state, req("question")).await.expect("answer");

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

    let result = answer(&app.state, req("query")).await.expect("answer");

    assert_eq!(
        result.answer, "知识库无相关内容。",
        "needs_review chunk 不应进入 catalog"
    );
    assert_eq!(app.llm.calls(), 0, "catalog 空 → 不调 LLM");
    assert_eq!(result.rounds_used, 0);
}

// ── D3：关系图谱按 relation_kind 正确遍历 + superseded 版本 redirect ──────────
//
// 直接驱动 pub 的 follow_relations / open_chunk（绕过 LLM 循环），对 relation_role
// 标记与 redirect 后的 chunk_id 做精确断言。

use std::collections::HashSet;
use wechatagent::agent::knowledge_agent::{follow_relations, open_chunk};

/// D3(a)：follow_relations 经 references 拉入的目标作支撑（relation_role=None），
/// 经 contradicts 拉入的目标带 relation_role="contradiction"（仅供辨别、prompt 警示
/// 勿作支撑引用）。
#[tokio::test]
#[ignore]
async fn follow_relations_marks_contradiction_targets() {
    let app = TestApp::start().await;

    let support = verified_chunk("支撑材料 B", "B 正文：正向支撑");
    let contra = verified_chunk("矛盾说法 C", "C 正文：与 A 相矛盾的说法");
    let support_id = support.id.unwrap().to_hex();
    let contra_id = contra.id.unwrap().to_hex();

    // A 同时 references B、contradicts C。
    let mut source = verified_chunk("来源 A", "A 正文");
    source.related_chunks = Some(vec![
        RelatedRef {
            chunk_id: support_id.clone(),
            kind: "references".to_string(),
            note: None,
        },
        RelatedRef {
            chunk_id: contra_id.clone(),
            kind: "contradicts".to_string(),
            note: Some("口径冲突".to_string()),
        },
    ]);
    let source_id = source.id.unwrap().to_hex();
    insert(&app, &[source, support, contra]).await;

    let (_catalog, prefetched) =
        follow_relations(&app.state, WS, None, &source_id, 1, &HashSet::new())
            .await
            .expect("follow_relations ok");

    // B 与 C 都被拉入（contradicts 跟随但标记，不是跳过）。
    let b = prefetched
        .iter()
        .find(|c| c.chunk_id == support_id)
        .expect("B 应被作为支撑拉入");
    assert_eq!(
        b.relation_role, None,
        "references 目标不带 contradiction 标记"
    );

    let c = prefetched
        .iter()
        .find(|c| c.chunk_id == contra_id)
        .expect("C 应被跟随（标记反例，不是跳过）");
    assert_eq!(
        c.relation_role.as_deref(),
        Some("contradiction"),
        "contradicts 目标必须带 relation_role=contradiction"
    );
}

/// D3(b) 问答侧：open_chunk 请求已被取代的旧版 → redirect 到现行版本，返回的
/// ChunkFull.chunk_id 即新版 id（cite⊆opened 不变量靠此对齐）。
#[tokio::test]
#[ignore]
async fn open_chunk_redirects_superseded_to_current_version() {
    let app = TestApp::start().await;

    let new_chunk = verified_chunk("现行版", "新版正文");
    let new_id = new_chunk.id.unwrap().to_hex();
    let mut old_chunk = verified_chunk("旧版", "旧版正文");
    old_chunk.status = "archived".to_string();
    old_chunk.superseded_by = Some(new_id.clone());
    let old_id = old_chunk.id.unwrap().to_hex();
    insert(&app, &[old_chunk, new_chunk]).await;

    let full = open_chunk(&app.state, WS, None, &old_id)
        .await
        .expect("open_chunk ok")
        .expect("应返回现行版而非 None");
    assert_eq!(full.chunk_id, new_id, "archived 旧版应 redirect 到新版 id");
    assert_eq!(full.body, "新版正文", "应返回现行版正文");
}

/// D3(b) 问答侧：follow_relations 命中已被取代的目标 → 收集现行版本而非旧版。
#[tokio::test]
#[ignore]
async fn follow_relations_redirects_superseded_target() {
    let app = TestApp::start().await;

    let new_chunk = verified_chunk("目标现行版", "新版正文");
    let new_id = new_chunk.id.unwrap().to_hex();
    let mut old_target = verified_chunk("目标旧版", "旧版正文");
    old_target.superseded_by = Some(new_id.clone());
    let old_id = old_target.id.unwrap().to_hex();

    // A references 旧版目标。
    let mut source = verified_chunk("来源 A", "A 正文");
    source.related_chunks = Some(vec![RelatedRef {
        chunk_id: old_id.clone(),
        kind: "references".to_string(),
        note: None,
    }]);
    let source_id = source.id.unwrap().to_hex();
    insert(&app, &[source, old_target, new_chunk]).await;

    let (_catalog, prefetched) =
        follow_relations(&app.state, WS, None, &source_id, 1, &HashSet::new())
            .await
            .expect("follow_relations ok");

    assert!(
        prefetched.iter().any(|c| c.chunk_id == new_id),
        "应收集现行版 {new_id}"
    );
    assert!(
        !prefetched.iter().any(|c| c.chunk_id == old_id),
        "不应收集已被取代的旧版 {old_id}"
    );
}

/// D3(b) 端到端（cite⊆opened 关键修复的主循环集成）：agent open 一个已被取代的旧版
/// id → open_chunk redirect 到新版 → opened_seen 记的是新版 id → agent cite 新版 id →
/// filter_answer_against_opened 不丢弃 → cited 非空且为新版。
///
/// 此前 open_chunk 分支把请求的旧 id 塞进 opened_seen，redirect 后 agent 看到/cite
/// 新版 id 会被当"未 open"丢掉，cited 变空。本测试守住该回归。
#[tokio::test]
#[ignore]
async fn answer_cites_redirected_current_version_end_to_end() {
    let app = TestApp::start().await;

    let new_chunk = verified_chunk("现行版", "现行版正文：价格异议三步法");
    let new_id = new_chunk.id.unwrap().to_hex();
    let mut old_chunk = verified_chunk("旧版", "旧版正文");
    old_chunk.superseded_by = Some(new_id.clone());
    let old_id = old_chunk.id.unwrap().to_hex();
    insert(&app, &[old_chunk, new_chunk]).await;

    // 轮 1：agent open 旧版 id（catalog 里可能两版都在，agent 选了旧的）。
    app.llm.push_response(json!({
        "action": "open_chunk",
        "ids": [old_id.clone()],
    }));
    // 轮 2：agent cite **新版 id**（redirect 后 opened 里就是新版，prompt 里看到的也是新版）。
    app.llm.push_response(json!({
        "action": "answer",
        "answer": "价格异议分三步处理。",
        "citedChunkIds": [new_id.clone()],
        "sourceQuotes": [{
            "chunkId": new_id.clone(),
            "quote": "现行版正文：价格异议三步法",
            "sourceAnchorIndex": 0,
        }],
    }));

    let result = answer(&app.state, req("价格异议")).await.expect("answer");

    // cite⊆opened 修复成立：cite 新版 id 不被丢，cited 非空且恰为新版。
    assert_eq!(
        result.cited_chunk_ids,
        vec![new_id.clone()],
        "redirect 后 cite 现行版 id 必须保留（cite⊆opened 不丢）"
    );
    assert!(!result.answer.is_empty(), "answer 非空");
    // 旧版 id 绝不出现在 cited（它从未进 opened_seen——opened_seen 记的是 redirect 后的新版）。
    assert!(
        !result.cited_chunk_ids.contains(&old_id),
        "旧版 id 不应被 cite"
    );
}

/// D3(b) DB 版 resolve_superseded 多跳链：v1→v2→v3 全 verified，open(v1) 跟到链尾 v3。
#[tokio::test]
#[ignore]
async fn open_chunk_follows_multi_hop_superseded_chain() {
    let app = TestApp::start().await;

    let v3 = verified_chunk("v3 终版", "v3 正文");
    let v3_id = v3.id.unwrap().to_hex();
    let mut v2 = verified_chunk("v2", "v2 正文");
    v2.superseded_by = Some(v3_id.clone());
    let v2_id = v2.id.unwrap().to_hex();
    let mut v1 = verified_chunk("v1", "v1 正文");
    v1.superseded_by = Some(v2_id.clone());
    let v1_id = v1.id.unwrap().to_hex();
    insert(&app, &[v1, v2, v3]).await;

    let full = open_chunk(&app.state, WS, None, &v1_id)
        .await
        .expect("open_chunk ok")
        .expect("应跟链返回 v3");
    assert_eq!(full.chunk_id, v3_id, "多跳链应跟到链尾 v3");
    assert_eq!(full.body, "v3 正文");
}

/// D3(b) DB 版 resolve_superseded：新版**非 verified**（draft）时停在旧版，绝不 redirect
/// 到未审定的新版（verified 门 + redirect 协同）。
#[tokio::test]
#[ignore]
async fn open_chunk_stops_redirect_when_new_version_unverified() {
    let app = TestApp::start().await;

    let mut draft_new = verified_chunk("未审定新版", "新版正文");
    draft_new.integrity_status = Some("needs_review".to_string()); // 非 verified
    let new_id = draft_new.id.unwrap().to_hex();
    let mut old_chunk = verified_chunk("旧版仍现行", "旧版正文");
    old_chunk.superseded_by = Some(new_id.clone());
    let old_id = old_chunk.id.unwrap().to_hex();
    insert(&app, &[old_chunk, draft_new]).await;

    // 新版未 verified → resolve 停在旧版 → open_chunk 返回旧版（仍 verified）。
    let full = open_chunk(&app.state, WS, None, &old_id)
        .await
        .expect("open_chunk ok")
        .expect("新版未审定时应停在旧版");
    assert_eq!(full.chunk_id, old_id, "不得 redirect 到未 verified 的新版");
    assert_eq!(full.body, "旧版正文");
}

/// D3(b) DB 版 resolve_superseded 自指环：chunk.superseded_by 指向自己 → 不死循环，
/// 停在自身返回。
#[tokio::test]
#[ignore]
async fn open_chunk_self_cycle_superseded_terminates() {
    let app = TestApp::start().await;

    let mut selfie = verified_chunk("自指环", "正文");
    let self_id = selfie.id.unwrap().to_hex();
    selfie.superseded_by = Some(self_id.clone());
    insert(&app, &[selfie]).await;

    let full = open_chunk(&app.state, WS, None, &self_id)
        .await
        .expect("open_chunk ok")
        .expect("自指环应停在自身");
    assert_eq!(full.chunk_id, self_id, "自指环停在自身不死循环");
}

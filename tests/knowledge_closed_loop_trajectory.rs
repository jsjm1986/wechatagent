//! Knowledge 局部约束与一条真实维护闭环。
//!
//! `smoke_catalog`、supersede 与 relation 用例只验证 catalog/关系局部约束，不得外推为
//! 维护 Agent 闭环证据。`chat_apply_verify_then_answer_is_auditable_closed_loop` 才是
//! SR-126 结算红线：真实 Chat Handler 起草 → apply 落 draft → 人审 verify 留 revision
//! → 生产 knowledge agent open/cite。
//!   3. SUPERSEDE 旧降新升：旧 chunk superseded_by 打标 → trust×0.1 降权 → 新 chunk 排前。
//!   4. 关系图完整：related_chunks 引用全部能在 catalog/库内解析，无悬空。
//!   5. 负例：未审定 draft（integrity_status≠verified）不得出现在默认 catalog。
//!
//! 全程红线：apply 写入恒走 draft+needs_review 起步，verified 必须显式经
//! verify_operation_knowledge_chunk（生产审批路径），agent 永不自动审定。
//! `#[ignore]`：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use std::panic::AssertUnwindSafe;

use futures::FutureExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::agent::knowledge_agent::{answer, list_catalog, AnswerRequest, CatalogFilter};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::knowledge::{
    chat_apply, chat_turn, verify_operation_knowledge_chunk, ChatApplyRequest, ChatTurnRequest,
    KnowledgeVerifyRequest,
};

use axum::extract::{Path, State};
use axum::{Extension, Json};

use crate::common::TestApp;

const WS: &str = "ws_closed_loop";

/// 种子 chunk 工厂：默认 verified + 带 source_quote/source_anchors（满足后续 verify gate）。
/// `body_terms` 用于让 title/summary/body 含 query 关键词，驱动 rank_key 命中。
fn seed_chunk(title: &str, body_terms: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: WS.to_string(),
        account_id: None,
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title} {body_terms}")),
        body: Some(format!("正文：{title}。{body_terms}")),
        wiki_type: Some("methodology".to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        source_quote: Some(format!("原文引用：{title}")),
        source_anchors: vec![doc! { "documentId": "seed_doc", "quote": title }],
        dynamic_confidence: Some(0.9),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

/// 清空本 ws 的 chunk，保证 catalog 干净。
/// 字段名用 snake_case `workspace_id`——OperationKnowledgeChunk 无 rename_all，
/// 落库即 snake_case（与 list_catalog/insert/verify 一致）；用 camelCase 会匹配 0 条。
async fn reset_ws(app: &TestApp) {
    app.state
        .db
        .operation_knowledge_chunks()
        .delete_many(doc! { "workspace_id": WS }, None)
        .await
        .expect("clean ws_closed_loop chunks");
}

/// 便捷：对 query 跑默认（verified-only）catalog，返回 chunk_id 顺序列表。
async fn catalog_ids(app: &TestApp, query: &str) -> Vec<String> {
    let entries = list_catalog(&app.state, WS, None, &CatalogFilter::default(), Some(query))
        .await
        .expect("list_catalog");
    entries.into_iter().map(|e| e.chunk_id).collect()
}

#[tokio::test]
#[ignore]
async fn smoke_catalog_returns_seeded_chunk() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    let chunk = seed_chunk("价格异议处理", "客户嫌贵 价格 异议 让步话术");
    let hex = chunk.id.expect("oid").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert seed");

    let ids = catalog_ids(&app, "客户嫌价格贵怎么办").await;
    assert!(ids.contains(&hex), "种子 chunk 应出现在 catalog：{ids:?}");
}

/// SR-126 真实维护闭环：Chat Handler 起草 → apply draft → verify revision → answer citation。
#[tokio::test]
#[ignore]
async fn chat_apply_verify_then_answer_is_auditable_closed_loop() {
    let app = TestApp::start().await;
    let result = AssertUnwindSafe(async {
        reset_ws(&app).await;
        let admin = AuthenticatedAdmin {
            user_id: "closed_loop_admin".into(),
            username: "closed_loop_admin".into(),
            current_workspace: WS.to_string(),
        };
        let operator_statement = "新增知识：客户已读不回时，先确认触达时机，再用低压力问题唤回。";

        // chat_turn 的 intent 分类与起草两次 LLM 输出；落库仍由后续真实 chat_apply 完成。
        app.llm.push_response(json!({
            "intent": "create_chunk",
            "confidence": 0.99,
            "userIntentSummary": "新增已读不回唤回知识",
        }));
        app.llm.push_response(json!({
            "decisionPhase": "final",
            "patch": {
                "title": "已读不回低压力唤回",
                "summary": "客户已读不回时先确认触达时机，再用低压力问题唤回。",
                "body": "客户已读不回时，先确认触达时机，再用低压力问题唤回。",
                "knowledgeType": "methodology",
                "sourceQuote": "客户已读不回时，先确认触达时机，再用低压力问题唤回。"
            },
            "missingFields": [],
            "followupQuestions": [],
            "naturalReply": "已起草已读不回的低压力唤回方法，请确认后应用为草稿。"
        }));
        let turn_req: ChatTurnRequest = serde_json::from_value(json!({
            "sessionId": null,
            "accountId": null,
            "operatorId": "closed_loop_operator",
            "content": operator_statement,
            "attachments": [],
        }))
        .expect("chat turn request");
        let turn = chat_turn(
            State(app.state.clone()),
            Extension(admin.clone()),
            Json(turn_req),
        )
        .await
        .expect("chat_turn must draft proposal")
        .0;
        assert_eq!(turn["intent"], "create_chunk");
        assert_eq!(turn["canApply"], true);
        let session_id = turn["sessionId"].as_str().expect("sessionId").to_string();

        let apply_req: ChatApplyRequest =
            serde_json::from_value(json!({ "accountId": null })).expect("chat apply request");
        let applied = chat_apply(
            State(app.state.clone()),
            Extension(admin.clone()),
            Path(session_id),
            Json(apply_req),
        )
        .await
        .expect("chat_apply must persist draft")
        .0;
        let chunk_id = applied["result"]["createdChunkId"]
            .as_str()
            .expect("createdChunkId")
            .to_string();
        let chunk_oid = ObjectId::parse_str(&chunk_id).expect("created chunk oid");
        let draft = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(doc! { "_id": chunk_oid, "workspace_id": WS }, None)
            .await
            .expect("load applied draft")
            .expect("applied draft exists");
        assert_eq!(draft.status, "draft");
        assert_eq!(draft.integrity_status.as_deref(), Some("needs_review"));
        assert!(
            !draft.source_anchors.is_empty(),
            "chat apply must anchor provenance"
        );
        assert!(
            !catalog_ids(&app, "客户已读不回怎么唤回")
                .await
                .contains(&chunk_id),
            "unverified chat draft must stay out of production catalog"
        );

        let verify_req: KnowledgeVerifyRequest =
            serde_json::from_value(json!({ "verifiedClaims": [] })).expect("verify request");
        let verify_response = verify_operation_knowledge_chunk(
            State(app.state.clone()),
            Extension(admin),
            Path(chunk_id.clone()),
            Json(verify_req),
        )
        .await
        .expect("human verify must succeed");
        assert_eq!(verify_response.0["ok"], true);
        let verify_revision_count = app
            .state
            .db
            .chunk_revisions()
            .count_documents(doc! { "chunk_id": &chunk_id, "op": "verify" }, None)
            .await
            .expect("count verify revisions");
        assert_eq!(
            verify_revision_count, 1,
            "verify transition must be auditable"
        );

        let verified = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(doc! { "_id": chunk_oid, "workspace_id": WS }, None)
            .await
            .expect("load verified chunk")
            .expect("verified chunk exists");
        let quote = verified
            .source_quote
            .clone()
            .expect("verified source quote");
        assert_eq!(verified.status, "active");
        assert_eq!(verified.integrity_status.as_deref(), Some("verified"));

        app.llm
            .push_response(json!({ "action": "open_chunk", "ids": [chunk_id.clone()] }));
        app.llm.push_response(json!({
            "action": "answer",
            "answer": "先确认触达时机，再用低压力问题唤回。",
            "citedChunkIds": [chunk_id.clone()],
            "sourceQuotes": [{
                "chunkId": chunk_id.clone(),
                "quote": quote,
                "sourceAnchorIndex": 0,
            }],
        }));
        let answer_result = answer(
            &app.state,
            AnswerRequest {
                workspace_id: WS.to_string(),
                account_id: None,
                query: "客户已读不回怎么唤回".to_string(),
                filter: CatalogFilter::default(),
                max_rounds: None,
            },
        )
        .await
        .expect("production knowledge answer");
        let evidence = (
            answer_result.cited_chunk_ids,
            answer_result.rounds_used,
            answer_result.truncated,
            app.llm.calls(),
        );
        assert_eq!(evidence.0, vec![chunk_id]);
        assert_eq!(evidence.1, 2);
        assert!(!evidence.2);
        assert_eq!(
            evidence.3, 4,
            "intent + draft + open + answer must all execute"
        );
    })
    .catch_unwind()
    .await;
    app.cleanup().await;
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

/// 门 1b：SUPERSEDE 旧降新升。旧 chunk 被 superseded_by 指向新 chunk → trust×0.1 →
/// 同 query 下新 chunk 必须排在旧 chunk 之前。验证「结构化写永不物理删除」——旧 chunk
/// 仍在库（未被删），只是降权。
#[tokio::test]
#[ignore]
async fn supersede_demotes_old_below_new() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    // 旧版 + 新版，相同主题（query 相关度相近），靠 trust 拉开。
    let old = seed_chunk("竞品对比方法论 v1", "竞品对比 客观陈述 优劣 旧版");
    let new = seed_chunk("竞品对比方法论 v2", "竞品对比 客观陈述 优劣 升级");
    let old_hex = old.id.expect("oid").to_hex();
    let new_hex = new.id.expect("oid").to_hex();
    for c in [&old, &new] {
        app.state
            .db
            .operation_knowledge_chunks()
            .insert_one(c, None)
            .await
            .expect("insert");
    }

    // 维护 agent 取代：旧版打 superseded_by=新版。物理保留旧 chunk。
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": old.id.unwrap() },
            doc! { "$set": { "superseded_by": &new_hex } },
            None,
        )
        .await
        .expect("mark superseded");

    let ids = catalog_ids(&app, "竞品对比怎么客观陈述").await;
    let pos_old = ids.iter().position(|x| x == &old_hex);
    let pos_new = ids.iter().position(|x| x == &new_hex);
    // 旧 chunk 仍在库（未被物理删）——查得到。
    let still_exists = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": old.id.unwrap() }, None)
        .await
        .expect("find old")
        .is_some();
    assert!(still_exists, "SUPERSEDE 不得物理删除旧 chunk");
    // 新版必须排在旧版之前（旧版 trust×0.1 降权）。降权只重排不剔除——旧版必须仍在
    // catalog 候选里（knowledge_agent rank_key 不变量：降格 chunk 不被过滤掉）。
    match (pos_new, pos_old) {
        (Some(pn), Some(po)) => {
            assert!(pn < po, "新版应排在旧版之前：new@{pn} old@{po} ids={ids:?}")
        }
        (Some(_), None) => panic!(
            "SUPERSEDE 只应降权重排，旧版不得被剔除出 catalog（rank_key 不变量）：ids={ids:?}"
        ),
        _ => panic!("新版 chunk 必须可召回：ids={ids:?}"),
    }
}

/// 门 1c：关系图完整。写入带 related_chunks 的 chunk 后，其每条引用的 chunk_id
/// 都能在库内解析（无悬空引用）。validate「结构化写」维护关系链完整。
#[tokio::test]
#[ignore]
async fn relation_graph_has_no_dangling_refs() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    let target = seed_chunk("价格异议处理", "价格 异议 让步 话术");
    let target_hex = target.id.expect("oid").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&target, None)
        .await
        .expect("insert target");

    // 维护 agent 新增一条 chunk，关系指向 target。
    let mut linked = seed_chunk("价格异议进阶应对", "价格 异议 进阶 谈判");
    linked.related_chunks = Some(vec![RelatedRef {
        chunk_id: target_hex.clone(),
        kind: "references".to_string(),
        note: None,
    }]);
    let linked_hex = linked.id.expect("oid").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&linked, None)
        .await
        .expect("insert linked");

    // 校验：linked 的每条 related_chunks 引用都能在库内 find 到（无悬空）。
    let fetched = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": linked.id.unwrap() }, None)
        .await
        .expect("find linked")
        .expect("linked exists");
    for r in fetched.related_chunks.unwrap_or_default() {
        let ref_oid = ObjectId::parse_str(&r.chunk_id).expect("related chunk_id is valid oid");
        let resolved = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(doc! { "_id": ref_oid }, None)
            .await
            .expect("find related")
            .is_some();
        assert!(
            resolved,
            "related_chunks 引用 {} 必须能解析（无悬空）",
            r.chunk_id
        );
    }
    assert!(
        catalog_ids(&app, "价格异议").await.contains(&linked_hex),
        "linked 应可召回"
    );
}

/// 门 1d（负例 + 审批路径）：维护 agent 提案落 draft+needs_review 时不可召回；
/// 仅在显式经生产 verify 审批转 verified 后才进 catalog。锁住「AI 永不自动审定」。
#[tokio::test]
#[ignore]
async fn unverified_draft_not_recallable_until_approved() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    // 维护 agent 提案：落 needs_review（带 source_quote/source_anchors 以便后续 verify）。
    let mut draft = seed_chunk("退款时效说明", "退款 时效 到账 周期");
    draft.integrity_status = Some("needs_review".to_string());
    // status 仍 active（catalog 默认 status=active 过滤），靠 integrity_status 把它挡在外面。
    let draft_hex = draft.id.expect("oid").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&draft, None)
        .await
        .expect("insert draft");

    // 负例：未审定不可召回（默认 catalog 只暴露 integrity_status=verified）。
    let before = catalog_ids(&app, "退款多久到账").await;
    assert!(
        !before.contains(&draft_hex),
        "未审定 draft 不得出现在默认 catalog：{before:?}"
    );

    // 经生产审批路径转 verified。
    let admin = Extension(AuthenticatedAdmin {
        user_id: "closed_loop_admin".into(),
        username: "closed_loop_admin".into(),
        current_workspace: WS.to_string(),
    });
    let req: KnowledgeVerifyRequest =
        serde_json::from_value(serde_json::json!({ "verifiedClaims": [] })).expect("verify req");
    let resp = verify_operation_knowledge_chunk(
        State(app.state.clone()),
        admin,
        Path(draft_hex.clone()),
        Json(req),
    )
    .await
    .expect("verify must succeed");
    assert_eq!(
        resp.0.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "verify 应成功：{:?}",
        resp.0
    );

    // 审批后可召回。
    let after = catalog_ids(&app, "退款多久到账").await;
    assert!(
        after.contains(&draft_hex),
        "审批 verified 后应可召回：{after:?}"
    );
}

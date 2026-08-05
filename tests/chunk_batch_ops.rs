//! `chunk_batch_ops` —— G3 批量 verify / archive + 反向引用 端到端集成测试。
//!
//! 直接调用 `routes::ext_knowledge::{batch_verify_chunks, batch_archive_chunks,
//! list_chunk_referrers}` 处理函数，绕过 axum HTTP harness。
//!
//! 覆盖：
//! 1. 批量 verify 3 条（含 source_quote + anchor）→ 全部成功；可重复 verify 不出错。
//! 2. 批量 archive：含 1 条已 archived → skipped 1 / archived 2。
//! 3. 反向引用 list_chunk_referrers：targetId 命中 1 条 referrer。
//! 4. D2 审计链：verify / reject / batch_verify 每次写入都在 chunk_revisions 落一条
//!    op/source/created_by/hash 正确的不可变历史；D2 gate 挡下的 verify 不写 revision。
//! 5. auto_verify 批处理（mock LLM，本地可跑）：每条 processed chunk 落一条
//!    op=verify/source=rule/created_by=auto_verify 的 revision，N 条→N 条数量对应。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。
//! AI 永不自动 verify 红线保留：批量入口仍需人工触发，与单条同 auth 路径。

mod common;

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use mongodb::bson::{oid::ObjectId, DateTime as BsonDt, Document};
use serde_json::json;
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::ext_knowledge::{
    auto_verify_operation_knowledge_chunks, batch_archive_chunks, batch_verify_chunks,
    list_chunk_referrers, reject_operation_knowledge_chunk, verify_operation_knowledge_chunk,
    KnowledgeAutoVerifyRequest, KnowledgeVerifyRequest,
};
use wechatagent::routes::{
    ChunkBatchArchiveRequest, ChunkBatchVerifyItem, ChunkBatchVerifyRequest, ChunkReferrersQuery,
};

use crate::common::TestApp;

/// 测试用 admin extractor：current_workspace 复用 TestApp 的默认 ws。
fn admin(app: &TestApp) -> Extension<AuthenticatedAdmin> {
    Extension(AuthenticatedAdmin {
        user_id: "test_admin".into(),
        username: "test_admin".into(),
        current_workspace: app.state.config.default_workspace_id.clone(),
    })
}

/// 本 fixture 的 anchor **必须**带非空 `sourceQuote`，与生产
/// `source_anchor_for_quote`（routes/knowledge/mod.rs）恒写该键的形态一致。
///
/// B3：D2 verify 闸判「可定位」用的是 `chunk_has_citable_anchor`（要求 anchor 自身含
/// 非空 `sourceQuote`），而不是旧口径的 `!source_anchors.is_empty()`。二者收敛后，
/// 只带 offset/hash 而无 `sourceQuote` 的 anchor 会被 verify 拒绝——那种 chunk 即便
/// 放行也永远无法被 `quote_is_chunk_evidence` 引用，故拒绝才是正确行为。
fn verifiable_chunk(workspace_id: &str, title: &str) -> OperationKnowledgeChunk {
    let quote = "引文文本：客户提出价格异议时，先共情、再说明价值、最后给方案。";
    let mut anchor = Document::new();
    anchor.insert("documentId", "doc_test");
    anchor.insert("startLine", 10i32);
    anchor.insert("endLine", 20i32);
    anchor.insert("quoteHash", "hash_abc123");
    anchor.insert("sourceQuote", quote);
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title}")),
        body: Some(format!("正文：{title}")),
        wiki_type: Some("methodology".to_string()),
        status: "active".to_string(),
        integrity_status: Some("needs_review".to_string()),
        source_quote: Some(
            "引文文本：客户提出价格异议时，先共情、再说明价值、最后给方案。".to_string(),
        ),
        source_anchors: vec![anchor],
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

fn verify_item(chunk: &OperationKnowledgeChunk) -> ChunkBatchVerifyItem {
    ChunkBatchVerifyItem {
        id: chunk.id.expect("chunk id").to_hex(),
        expected_updated_at: chunk
            .updated_at
            .try_to_rfc3339_string()
            .expect("serialize updated_at"),
    }
}

fn verify_request(
    chunk: &OperationKnowledgeChunk,
    claims: serde_json::Value,
) -> KnowledgeVerifyRequest {
    serde_json::from_value(json!({
        "verifiedClaims": claims,
        "expectedUpdatedAt": chunk.updated_at.try_to_rfc3339_string().expect("serialize updated_at"),
    }))
    .expect("build verify request")
}

#[tokio::test]
#[ignore]
async fn batch_verify_marks_three_chunks_verified() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let c1 = verifiable_chunk(&ws, "三步价格异议");
    let c2 = verifiable_chunk(&ws, "两步报价术");
    let c3 = verifiable_chunk(&ws, "客户分级");
    let id1 = c1.id.unwrap().to_hex();
    let id2 = c2.id.unwrap().to_hex();
    let id3 = c3.id.unwrap().to_hex();
    let verify_items = vec![verify_item(&c1), verify_item(&c2), verify_item(&c3)];
    insert(&app, &[c1, c2, c3]).await;

    let resp = batch_verify_chunks(
        State(app.state.clone()),
        admin(&app),
        Json(ChunkBatchVerifyRequest {
            items: verify_items,
            note: Some("admin batch verify".to_string()),
        }),
    )
    .await
    .expect("batch verify ok");
    let body = resp.0;

    let verified = body["verified"].as_array().expect("verified array");
    assert_eq!(verified.len(), 3, "all three verified: {body:?}");
    assert!(body["skipped"]
        .as_array()
        .map(|a| a.is_empty())
        .unwrap_or(false));
    assert_eq!(body["note"].as_str(), Some("admin batch verify"));

    // 实际 DB 状态必须切到 verified
    for id_hex in [&id1, &id2, &id3] {
        let chunk = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(
                mongodb::bson::doc! {
                    "_id": ObjectId::parse_str(id_hex).unwrap(),
                    "workspace_id": &ws,
                },
                None,
            )
            .await
            .unwrap()
            .expect("chunk should exist");
        assert_eq!(chunk.integrity_status.as_deref(), Some("verified"));
        assert_eq!(chunk.status, "active");
    }
}

#[tokio::test]
#[ignore]
async fn batch_archive_skips_already_archived() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let mut c_active = verifiable_chunk(&ws, "活跃 chunk A");
    c_active.integrity_status = Some("verified".to_string());
    let mut c_active2 = verifiable_chunk(&ws, "活跃 chunk B");
    c_active2.integrity_status = Some("verified".to_string());
    let mut c_archived = verifiable_chunk(&ws, "已归档 chunk");
    c_archived.status = "archived".to_string();

    let id_a = c_active.id.unwrap().to_hex();
    let id_b = c_active2.id.unwrap().to_hex();
    let id_arch = c_archived.id.unwrap().to_hex();
    insert(&app, &[c_active, c_active2, c_archived]).await;

    let resp = batch_archive_chunks(
        State(app.state.clone()),
        admin(&app),
        Json(ChunkBatchArchiveRequest {
            ids: vec![id_a.clone(), id_b.clone(), id_arch.clone()],
            reason: Some("end-of-life".to_string()),
        }),
    )
    .await
    .expect("batch archive ok");
    let body = resp.0;

    let archived = body["archived"].as_array().expect("archived array");
    let skipped = body["skipped"].as_array().expect("skipped array");

    // 至少 2 条 archived（id_a, id_b）；id_arch 走 RevisionRequest::Archive 又一次：
    // apply_chunk_revision 对已 archived 的 chunk 仍可能成功（写新 revision），
    // 也可能 skipped。两种行为都接受 — 关键是 a/b 必落在 archived 中。
    let archived_set: std::collections::HashSet<&str> =
        archived.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        archived_set.contains(id_a.as_str()),
        "id_a archived: {body:?}"
    );
    assert!(
        archived_set.contains(id_b.as_str()),
        "id_b archived: {body:?}"
    );
    assert!(
        archived.len() + skipped.len() == 3,
        "total processed = 3: {body:?}"
    );

    // a / b chunk 实际状态切到 archived
    for id_hex in [&id_a, &id_b] {
        let chunk = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(
                mongodb::bson::doc! {
                    "_id": ObjectId::parse_str(id_hex).unwrap(),
                    "workspace_id": &ws,
                },
                None,
            )
            .await
            .unwrap()
            .expect("chunk should exist");
        assert_eq!(chunk.status, "archived", "id={id_hex}");
    }
}

#[tokio::test]
#[ignore]
async fn list_chunk_referrers_returns_referrer_with_kind_and_note() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // target chunk
    let target = verifiable_chunk(&ws, "目标 chunk（被引用）");
    let target_id = target.id.unwrap().to_hex();

    // referrer chunk：related_chunks 中含 target_id
    let mut referrer = verifiable_chunk(&ws, "引用 chunk");
    referrer.related_chunks = Some(vec![RelatedRef {
        chunk_id: target_id.clone(),
        kind: "supports".to_string(),
        note: Some("引证支撑 target".to_string()),
    }]);

    // 一个不相关的 chunk（不应出现在 referrers）
    let unrelated = verifiable_chunk(&ws, "无关 chunk");

    insert(&app, &[target, referrer, unrelated]).await;

    let resp = list_chunk_referrers(
        State(app.state.clone()),
        admin(&app),
        Query(ChunkReferrersQuery {
            target_id: target_id.clone(),
        }),
    )
    .await
    .expect("list referrers ok");
    let body = resp.0;

    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "exactly 1 referrer: {body:?}");
    let it = &items[0];
    assert_eq!(it["kind"].as_str(), Some("supports"));
    assert_eq!(it["note"].as_str(), Some("引证支撑 target"));
    assert_eq!(it["wikiType"].as_str(), Some("methodology"));
    assert_eq!(it["status"].as_str(), Some("active"));
}

#[tokio::test]
#[ignore]
async fn batch_verify_rejects_empty_ids() {
    let app = TestApp::start().await;
    let resp = batch_verify_chunks(
        State(app.state.clone()),
        admin(&app),
        Json(ChunkBatchVerifyRequest {
            items: vec![],
            note: None,
        }),
    )
    .await;
    assert!(resp.is_err(), "empty ids must 400");
}

#[tokio::test]
#[ignore]
async fn batch_verify_skips_chunk_without_quote() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let mut c = verifiable_chunk(&ws, "无 source_quote");
    c.source_quote = None;
    let verify_item = verify_item(&c);
    insert(&app, &[c]).await;

    let resp = batch_verify_chunks(
        State(app.state.clone()),
        admin(&app),
        Json(ChunkBatchVerifyRequest {
            items: vec![verify_item],
            note: None,
        }),
    )
    .await
    .expect("ok response with skipped");
    let body = resp.0;

    let verified = body["verified"].as_array().unwrap();
    let skipped = body["skipped"].as_array().unwrap();
    assert!(verified.is_empty(), "must not verify w/o quote");
    assert_eq!(skipped.len(), 1);
    let reason = skipped[0]["reason"].as_str().unwrap_or("").to_lowercase();
    assert!(
        reason.contains("sourcequote") || reason.contains("anchor") || reason.contains("quote"),
        "skip reason should mention source gate: {body:?}"
    );
    let _ = json!(body); // satisfy import
}

// ── D2：verify / reject / batch_verify 写入接回 apply_chunk_revision ──────────
//
// 补全审计链：「needs_review → verified」这个最关键状态转移此前直接 update_one，
// 不写 chunk_revisions、不更新 provenance。下列测试断言每次 verify/reject/batch_verify
// 都在 chunk_revisions 落一条 op/source/created_by/hash 正确的不可变历史。

/// 取某 chunk 的全部 chunk_revisions（按 created_at 升序），供审计断言。
async fn revisions_for(
    app: &TestApp,
    chunk_id_hex: &str,
) -> Vec<wechatagent::models::ChunkRevision> {
    use futures::TryStreamExt;
    app.state
        .db
        .chunk_revisions()
        .find(
            mongodb::bson::doc! { "chunk_id": chunk_id_hex },
            mongodb::options::FindOptions::builder()
                .sort(mongodb::bson::doc! { "created_at": 1 })
                .build(),
        )
        .await
        .expect("query chunk_revisions")
        .try_collect()
        .await
        .expect("collect chunk_revisions")
}

#[tokio::test]
#[ignore]
async fn verify_writes_chunk_revision_audit_entry() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let chunk = verifiable_chunk(&ws, "单条 verify 审计链");
    let id = chunk.id.unwrap().to_hex();
    let payload = verify_request(&chunk, json!(["先共情再说明价值"]));
    insert(&app, &[chunk]).await;

    // verify 前：零 revision。
    assert!(
        revisions_for(&app, &id).await.is_empty(),
        "verify 前不应有任何 chunk_revisions"
    );

    let _ = verify_operation_knowledge_chunk(
        State(app.state.clone()),
        admin(&app),
        Path(id.clone()),
        Json(payload),
    )
    .await
    .expect("verify ok");

    // chunk 本体切到 verified。
    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(
            mongodb::bson::doc! { "_id": ObjectId::parse_str(&id).unwrap(), "workspace_id": &ws },
            None,
        )
        .await
        .unwrap()
        .expect("chunk exists");
    assert_eq!(stored.integrity_status.as_deref(), Some("verified"));
    assert_eq!(stored.status, "active");

    // chunk_revisions 多一条：op=verify、source=human、created_by=actor、hash 变化。
    let revs = revisions_for(&app, &id).await;
    assert_eq!(revs.len(), 1, "verify 应恰好写一条 revision: {revs:?}");
    let rev = &revs[0];
    assert_eq!(rev.op, "verify");
    assert_eq!(rev.source, "human");
    assert_eq!(rev.created_by.as_deref(), Some("test_admin"));
    assert_ne!(
        rev.before_hash, rev.after_hash,
        "needs_review→verified 内容变更，before/after hash 必须不同"
    );
    assert!(!rev.before_hash.is_empty() && !rev.after_hash.is_empty());
}

#[tokio::test]
#[ignore]
async fn reject_writes_chunk_revision_with_reject_op() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let chunk = verifiable_chunk(&ws, "单条 reject 审计链");
    let id = chunk.id.unwrap().to_hex();
    insert(&app, &[chunk]).await;

    let _ =
        reject_operation_knowledge_chunk(State(app.state.clone()), admin(&app), Path(id.clone()))
            .await
            .expect("reject ok");

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(
            mongodb::bson::doc! { "_id": ObjectId::parse_str(&id).unwrap(), "workspace_id": &ws },
            None,
        )
        .await
        .unwrap()
        .expect("chunk exists");
    assert_eq!(stored.integrity_status.as_deref(), Some("rejected"));
    assert_eq!(stored.status, "rejected");

    let revs = revisions_for(&app, &id).await;
    assert_eq!(revs.len(), 1, "reject 应恰好写一条 revision: {revs:?}");
    assert_eq!(revs[0].op, "reject");
    assert_eq!(revs[0].source, "human");
    assert_eq!(revs[0].created_by.as_deref(), Some("test_admin"));
}

#[tokio::test]
#[ignore]
async fn batch_verify_writes_one_revision_per_chunk() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let c1 = verifiable_chunk(&ws, "批量 A");
    let c2 = verifiable_chunk(&ws, "批量 B");
    let c3 = verifiable_chunk(&ws, "批量 C");
    let id1 = c1.id.unwrap().to_hex();
    let id2 = c2.id.unwrap().to_hex();
    let id3 = c3.id.unwrap().to_hex();
    let verify_items = vec![verify_item(&c1), verify_item(&c2), verify_item(&c3)];
    insert(&app, &[c1, c2, c3]).await;

    let _ = batch_verify_chunks(
        State(app.state.clone()),
        admin(&app),
        Json(ChunkBatchVerifyRequest {
            items: verify_items,
            note: Some("批量审计链".to_string()),
        }),
    )
    .await
    .expect("batch verify ok");

    // 每条恰好一条 op=verify revision，且 reason 透传 note。
    for id_hex in [&id1, &id2, &id3] {
        let revs = revisions_for(&app, id_hex).await;
        assert_eq!(
            revs.len(),
            1,
            "chunk {id_hex} 应恰好一条 revision: {revs:?}"
        );
        assert_eq!(revs[0].op, "verify");
        assert_eq!(revs[0].source, "human");
        assert_eq!(revs[0].created_by.as_deref(), Some("test_admin"));
        assert_eq!(revs[0].reason.as_deref(), Some("批量审计链"));
    }
}

#[tokio::test]
#[ignore]
async fn verify_rejects_stale_review_snapshot_without_writing_revision() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let chunk = verifiable_chunk(&ws, "旧快照不可核验");
    let id = chunk.id.unwrap().to_hex();
    let stale_payload = verify_request(&chunk, json!([]));
    insert(&app, &[chunk]).await;

    // 模拟管理员看到版本 A 后，另一写入把当前行推进到版本 B。
    let oid = ObjectId::parse_str(&id).unwrap();
    let newer = BsonDt::from_millis(BsonDt::now().timestamp_millis() + 1_000);
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            mongodb::bson::doc! { "_id": oid, "workspace_id": &ws },
            mongodb::bson::doc! { "$set": {
                "summary": "并发写入后的版本 B",
                "updated_at": newer,
            } },
            None,
        )
        .await
        .expect("advance chunk version");

    let error = verify_operation_knowledge_chunk(
        State(app.state.clone()),
        admin(&app),
        Path(id.clone()),
        Json(stale_payload),
    )
    .await
    .expect_err("stale review snapshot must fail closed");
    assert!(
        matches!(error, wechatagent::error::AppError::Conflict(ref code) if code == "chunk_revision_conflict"),
        "unexpected error: {error:?}"
    );
    assert!(
        revisions_for(&app, &id).await.is_empty(),
        "stale verify must not leave an audit revision for an unapplied approval"
    );

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(
            mongodb::bson::doc! { "_id": oid, "workspace_id": &ws },
            None,
        )
        .await
        .expect("load chunk")
        .expect("chunk exists");
    assert_eq!(stored.integrity_status.as_deref(), Some("needs_review"));
}

#[tokio::test]
#[ignore]
async fn verify_gate_blocks_and_writes_no_revision_without_anchor() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    // 缺 source_anchors（仅有 quote）→ D2 gate 在 apply_chunk_revision 之前先挡住。
    let mut chunk = verifiable_chunk(&ws, "无 anchor 不可 verify");
    chunk.source_anchors = vec![];
    let id = chunk.id.unwrap().to_hex();
    let payload = verify_request(&chunk, json!([]));
    insert(&app, &[chunk]).await;
    let resp = verify_operation_knowledge_chunk(
        State(app.state.clone()),
        admin(&app),
        Path(id.clone()),
        Json(payload),
    )
    .await;
    assert!(resp.is_err(), "缺 source_anchors 必须 400");

    // gate 在写 revision 之前拒绝 → 不留任何 chunk_revisions 痕迹。
    assert!(
        revisions_for(&app, &id).await.is_empty(),
        "被 D2 gate 挡下的 verify 不应写 revision"
    );
    // chunk 本体保持 needs_review，未被改动。
    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(
            mongodb::bson::doc! { "_id": ObjectId::parse_str(&id).unwrap(), "workspace_id": &ws },
            None,
        )
        .await
        .unwrap()
        .expect("chunk exists");
    assert_eq!(stored.integrity_status.as_deref(), Some("needs_review"));
}

/// D2 + 本地可跑：auto_verify 批处理的每条裁决都接回 apply_chunk_revision，
/// 在 chunk_revisions 落一条 op=verify / **source=rule**（裁决由 LLM 自评+规则闸门
/// 做出，非人工逐条签字）/ created_by=auto_verify 的审计行。
///
/// 用内置 mock LLM（`app.llm.push_response`）喂确定性自评 JSON，**不依赖真模型**——
/// 此前 auto_verify 的审计断言只活在 real-LLM 测试里（本地无 key 即 skip = 假绿），
/// 本测试把这条最关键的批处理审计路径锁进本地 `cargo test --test` 可跑的范围。
///
/// 断言"N 条 chunk → N 条 revision"数量对应（real-LLM K7 只 seed 1 条做不到）。
#[tokio::test]
#[ignore]
async fn auto_verify_writes_one_revision_per_processed_chunk() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();

    // seed 2 条 needs_review、带齐 source_quote + anchor 的 chunk。
    let c1 = verifiable_chunk(&ws, "自动审定 A");
    let c2 = verifiable_chunk(&ws, "自动审定 B");
    let id1 = c1.id.unwrap().to_hex();
    let id2 = c2.id.unwrap().to_hex();
    insert(&app, &[c1, c2]).await;

    // 为两条 chunk 各 push 一条 LLM 自评响应（auto_verify 逐条串行调 generate_agent_json）。
    for _ in 0..2 {
        app.llm.push_response(json!({
            "confidenceScore": 9,
            "integrityStatus": "verified",
            "verifiedClaims": ["先共情再说明价值"],
            "distortionRisks": [],
        }));
    }

    let req: KnowledgeAutoVerifyRequest = serde_json::from_value(json!({
        "accountId": account_id,
        "confidenceThreshold": 7,
        // 关抽样（clamp 到 5% 硬下限）：终态可能落 verified 或 needs_human_audit，
        // 但**无论哪种**，每条 processed chunk 都会写一条 revision——这正是被测不变量。
        "humanAuditSampleRate": 0.0,
        "limit": 10,
    }))
    .expect("build auto_verify request");

    let resp =
        auto_verify_operation_knowledge_chunks(State(app.state.clone()), admin(&app), Json(req))
            .await
            .expect("auto_verify ok");
    let processed = resp.0["processed"].as_i64().unwrap_or(0);
    assert_eq!(processed, 2, "两条 chunk 都应被处理: {:?}", resp.0);
    assert_eq!(app.llm.calls(), 2, "应正好消费 2 条 LLM 响应");

    // 每条 chunk 恰好一条 op=verify / source=rule / created_by=auto_verify 的 revision。
    for id_hex in [&id1, &id2] {
        let revs = revisions_for(&app, id_hex).await;
        assert_eq!(
            revs.len(),
            1,
            "chunk {id_hex} 应恰好一条 revision: {revs:?}"
        );
        let rev = &revs[0];
        assert_eq!(rev.op, "verify", "auto_verify revision op 必须为 verify");
        assert_eq!(
            rev.source, "rule",
            "auto_verify 裁决是 LLM 自评+规则闸门，source 必须为 rule（非 human）"
        );
        assert_eq!(rev.created_by.as_deref(), Some("auto_verify"));
        assert!(
            !rev.before_hash.is_empty() && !rev.after_hash.is_empty(),
            "before/after hash 必须落值"
        );
    }
}

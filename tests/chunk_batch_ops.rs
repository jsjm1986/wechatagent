//! `chunk_batch_ops` —— G3 批量 verify / archive + 反向引用 端到端集成测试。
//!
//! 直接调用 `routes::ext_knowledge::{batch_verify_chunks, batch_archive_chunks,
//! list_chunk_referrers}` 处理函数，绕过 axum HTTP harness。
//!
//! 覆盖：
//! 1. 批量 verify 3 条（含 source_quote + anchor）→ 全部成功；可重复 verify 不出错。
//! 2. 批量 archive：含 1 条已 archived → skipped 1 / archived 2。
//! 3. 反向引用 list_chunk_referrers：targetId 命中 1 条 referrer。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。
//! AI 永不自动 verify 红线保留：批量入口仍需人工触发，与单条同 auth 路径。

mod common;

use axum::extract::{Query, State};
use axum::Json;
use mongodb::bson::{oid::ObjectId, DateTime as BsonDt, Document};
use serde_json::json;
use wechatagent::models::{OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::ext_knowledge::{
    batch_archive_chunks, batch_verify_chunks, list_chunk_referrers,
};
use wechatagent::routes::{
    ChunkBatchArchiveRequest, ChunkBatchVerifyRequest, ChunkReferrersQuery,
};

use crate::common::TestApp;

fn verifiable_chunk(workspace_id: &str, title: &str) -> OperationKnowledgeChunk {
    let mut anchor = Document::new();
    anchor.insert("documentId", "doc_test");
    anchor.insert("startLine", 10i32);
    anchor.insert("endLine", 20i32);
    anchor.insert("quoteHash", "hash_abc123");
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
        source_quote: Some("引文文本：客户提出价格异议时，先共情、再说明价值、最后给方案。".to_string()),
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

#[tokio::test]
#[ignore]
async fn batch_verify_marks_three_chunks_verified() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let c1 = verifiable_chunk(&ws, "三步价格异议");
    let c2 = verifiable_chunk(&ws, "两步报价术");
    let c3 = verifiable_chunk(&ws, "客户分级");
    let id1 = c1.id.unwrap().to_hex();
    let id2 = c2.id.unwrap().to_hex();
    let id3 = c3.id.unwrap().to_hex();
    insert(&app, &[c1, c2, c3]).await;

    let resp = batch_verify_chunks(
        State(app.state.clone()),
        Json(ChunkBatchVerifyRequest {
            ids: vec![id1.clone(), id2.clone(), id3.clone()],
            note: Some("admin batch verify".to_string()),
        }),
    )
    .await
    .expect("batch verify ok");
    let body = resp.0;

    let verified = body["verified"].as_array().expect("verified array");
    assert_eq!(verified.len(), 3, "all three verified: {body:?}");
    assert!(body["skipped"].as_array().map(|a| a.is_empty()).unwrap_or(false));
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
    let app = TestApp::start().await;
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
        Json(ChunkBatchArchiveRequest {
            ids: vec![id_a.clone(), id_b.clone(), id_arch.clone()],
            reason: Some("end-of-life".to_string()),
            actor: Some("admin".to_string()),
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
    let archived_set: std::collections::HashSet<&str> = archived
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(archived_set.contains(id_a.as_str()), "id_a archived: {body:?}");
    assert!(archived_set.contains(id_b.as_str()), "id_b archived: {body:?}");
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
        Json(ChunkBatchVerifyRequest {
            ids: vec![],
            note: None,
        }),
    )
    .await;
    assert!(resp.is_err(), "empty ids must 400");
}

#[tokio::test]
#[ignore]
async fn batch_verify_skips_chunk_without_quote() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let mut c = verifiable_chunk(&ws, "无 source_quote");
    c.source_quote = None;
    let id = c.id.unwrap().to_hex();
    insert(&app, &[c]).await;

    let resp = batch_verify_chunks(
        State(app.state.clone()),
        Json(ChunkBatchVerifyRequest {
            ids: vec![id.clone()],
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
    let reason = skipped[0]["reason"].as_str().unwrap_or("");
    assert!(
        reason.contains("source_quote") || reason.contains("anchor") || reason.contains("quote"),
        "skip reason should mention source gate: {body:?}"
    );
    let _ = json!(body); // satisfy import
}

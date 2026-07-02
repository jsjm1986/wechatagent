//! `apply_chunk_revision` 红线集成测试:source=Ai 的写入**强制**把 chunk 打回
//! `status="draft"` + `integrity_status="needs_review"`(AI 永不自动 verify)。
//! 全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test chunk_revision_ai_draft_integration -- --ignored`。
//!
//! ## 红线意义(P0):AI 起草的知识写入**必须**降级为 draft + needs_review
//! (chunk_revisions.rs:207-212 强制)。chat 的 `apply_create_chunk` 创建路径已由
//! `knowledge_chat_apply_integration.rs` 覆盖;但 `apply_chunk_revision` 的
//! **source=Ai patch 这一支**此前仅有纯函数 PBT 模拟(`wiki_chunk_revision_pbt.rs`
//! 断言测试自己复制的逻辑,删掉生产 :207-212 也不变红)。本测试 seed 一条 active +
//! verified 的既有 chunk,真调 `apply_chunk_revision`(source=Ai, op=Patch)驱动生产
//! 降级逻辑,落库后查 DB 断言被打回 draft + needs_review。一旦 :207-212 被删,本测试
//! 立刻红(verified 不会被打回)。
#![cfg(test)]

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use wechatagent::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, ProvenanceSource, RevisionOp, RevisionRequest,
};
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

/// 红线:source=Ai 的 patch 把 active+verified 的 chunk 强制打回 draft+needs_review。
#[tokio::test]
#[ignore]
async fn apply_chunk_revision_ai_source_forces_draft_needs_review() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // seed 一条既有 chunk,故意置为 active + verified 作为反例初值:
    // 若 :207-212 不生效,patch 后仍是 active/verified,断言立刻红。
    // body 给足长度(远超摘要),避免 patch summary 触发 70% 截断闸
    // (text_payload_len 取 body/summary 较长者;body 不变 → new_len==old_len,不截断)。
    let chunk_oid = ObjectId::new();
    let seeded = OperationKnowledgeChunk {
        id: Some(chunk_oid),
        workspace_id: ws.clone(),
        domain: "user_operations".to_string(),
        title: "退款政策".to_string(),
        summary: Some("原始摘要:七天无理由退款规则概述。".to_string()),
        body: Some(
            "支持七天无理由退款,商品需保持完好、配件齐全、不影响二次销售。\
             超过七天或商品已拆封使用的,按具体情况人工评估处理。运费由买方承担,\
             质量问题除外。以上为退款政策正文,内容较长以确保过 70% body 长度阈值。"
                .to_string(),
        ),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&seeded, None)
        .await
        .expect("insert seeded chunk");

    // 真调 apply_chunk_revision,source=Ai + op=Patch,只改 summary。
    // 注意:第一个参数是 &Database(app.state.db),不是 &AppState。
    let applied = apply_chunk_revision(
        &app.state.db,
        &ws,
        chunk_oid,
        RevisionRequest {
            op: RevisionOp::Patch,
            source: ProvenanceSource::Ai,
            patch: doc! { "summary": "AI 改写后的摘要:七天无理由退款,需商品完好。" },
            reason: Some("test: ai patch forces draft".to_string()),
            actor: None,
        },
    )
    .await
    .expect("apply_chunk_revision 应成功");

    // status active→draft + integrity verified→needs_review 会改变 canonical hash,
    // 因此本次写入必落库(unchanged=false),replace_one 生效。
    assert!(
        !applied.unchanged,
        "AI source 打回 draft+needs_review 改变了 status/integrity,不应 unchanged: {applied:?}"
    );

    // 落库后立即查 DB,断言 source=Ai 的写入把 chunk 打回 draft + needs_review。
    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": chunk_oid, "workspace_id": &ws },
            None,
        )
        .await
        .expect("查 chunk")
        .expect("chunk 应存在");

    assert_eq!(
        stored.status, "draft",
        "source=Ai 写入必须把 status 强制打回 draft(AI 永不自动 verify),实得 {:?}",
        stored.status
    );
    assert_eq!(
        stored.integrity_status.as_deref(),
        Some("needs_review"),
        "source=Ai 写入必须把 integrity_status 强制打回 needs_review,实得 {:?}",
        stored.integrity_status
    );
}

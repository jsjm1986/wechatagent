//! 知识对话工作台 chat_apply 红线集成测试:apply_create_chunk 落库瞬间
//! status=draft + integrity_status=needs_review(AI 永不自动 verify)。
//! 全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test knowledge_chat_apply_integration -- --ignored`。
//!
//! ## 红线意义(P0):chat 内 AI 起草的知识落库**必须** status=draft + integrity=needs_review
//! (chat.rs:1679-1681 强制),AI 永不把自己产物标 verified。审计指出原 recall benchmark
//! 紧接 verify 把中间态盖过,看不到"落库瞬间是 draft"。本测试落库后**立即查 DB**(不 verify),
//! 钉死中间态。一旦 apply 误标 verified,本测试立刻红。
#![cfg(test)]

mod common;

use mongodb::bson::doc;

use wechatagent::routes::knowledge::chat::apply_create_chunk;

use crate::common::TestApp;

/// 红线:apply_create_chunk 落库瞬间 status=draft && integrity_status=needs_review。
#[tokio::test]
#[ignore]
async fn chat_apply_create_forces_draft_needs_review() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let patch = doc! {
        "title": "退款政策",
        "body": "7 天无理由退款,需保持商品完好。",
        "summary": "退款规则",
        "knowledgeType": "policy",
    };

    let result = apply_create_chunk(
        &app.state,
        &ws,
        Some("default"),
        "sess_test",
        &patch,
        None,
        "运营口述:我们支持 7 天无理由退款",
    )
    .await
    .expect("apply_create_chunk 应成功");

    // 返回体即声明 draft + needs_review
    assert_eq!(result["status"], "draft", "返回体 status 应为 draft");
    assert_eq!(
        result["integrityStatus"], "needs_review",
        "返回体 integrityStatus 应为 needs_review"
    );

    // 落库后立即查 DB(不 verify),断言中间态就是 draft + needs_review
    let created_id = result["createdChunkId"].as_str().expect("createdChunkId");
    let oid = mongodb::bson::oid::ObjectId::parse_str(created_id).expect("oid");
    let chunk = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": oid }, None)
        .await
        .expect("查 chunk")
        .expect("chunk 应存在");

    assert_eq!(chunk.status, "draft", "落库瞬间 status 必须 draft(AI 永不自动 verify)");
    assert_eq!(
        chunk.integrity_status.as_deref(),
        Some("needs_review"),
        "落库瞬间 integrity_status 必须 needs_review"
    );
}

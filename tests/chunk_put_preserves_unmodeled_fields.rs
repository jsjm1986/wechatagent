//! `chunk_put_preserves_unmodeled_fields` —— chunk PUT 更新不清空「请求体无法表达」
//! 的 model 字段 端到端集成测试。
//!
//! 根因：PUT `update_operation_knowledge_chunk` 用 `replace_one` 整条替换文档，转换函数
//! `operation_knowledge_chunk_from_request` 对请求体不携带的字段走 `..Default::default()`，
//! 导致每次 PUT 都把 `provenance`（知识来源追溯）/ `wiki_type` / `locked_fields` /
//! `created_at` 等 13 个字段清空。这条 PUT 是 AI 修复闭环 + 运营手工编辑的落库路径，
//! 所以 AI 修复会丢失知识来源追溯。
//!
//! 本测试参照 `domain_profile_e2e.rs::e2e_update_handler_partial_set_preserves_untouched_fields`
//! 的范式：seed 一条带 provenance/wiki_type/locked_fields/created_at 的 chunk，PUT 只带
//! title，断言这些未携带字段保持原值（核心防回归）、title 被更新。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。
//! AI 永不自动 verify 红线保留：本修复只回填请求体无法表达的字段，不碰 integrity 判定。

mod common;

use axum::extract::{Extension, Json, Path, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{ChunkProvenance, OperationKnowledgeChunk};
use wechatagent::routes::ext_knowledge::{
    update_operation_knowledge_chunk, OperationKnowledgeChunkRequest,
};

use crate::common::TestApp;

fn admin(ws: &str) -> Extension<AuthenticatedAdmin> {
    Extension(AuthenticatedAdmin {
        user_id: "test_admin".into(),
        username: "test_admin".into(),
        current_workspace: ws.to_string(),
    })
}

#[tokio::test]
#[ignore]
async fn put_preserves_provenance_wiki_type_locked_fields_and_created_at() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let id = ObjectId::new();
    let created = BsonDt::from_millis(1_000_000);

    // seed：一条带「请求体无法表达」字段的 chunk。
    let seeded = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: ws.clone(),
        domain: "user_operations".to_string(),
        title: "原标题".to_string(),
        summary: Some("原摘要".to_string()),
        status: "active".to_string(),
        wiki_type: Some("methodology".to_string()),
        chunk_type: "style_template".to_string(),
        provenance: Some(ChunkProvenance {
            source: "imported".to_string(),
            source_doc_id: Some("doc-seed".to_string()),
            source_quote: Some("原始引文".to_string()),
            llm_model_alias: None,
            edited_at: created,
            edited_by: Some("operator".to_string()),
        }),
        locked_fields: Some(vec!["title".to_string(), "body".to_string()]),
        dynamic_confidence: Some(0.66),
        integrity_score: Some(0.88),
        created_at: created,
        updated_at: created,
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&seeded, None)
        .await
        .expect("seed chunk");

    // PUT 只带 title（camelCase，不带那 13 个字段）。
    let body: OperationKnowledgeChunkRequest =
        serde_json::from_value(serde_json::json!({ "title": "新标题" }))
            .expect("deserialize OperationKnowledgeChunkRequest with only title");

    let _ = update_operation_knowledge_chunk(
        State(app.state.clone()),
        admin(&ws),
        Path(id.to_hex()),
        Json(body),
    )
    .await
    .expect("update handler ok");

    // 重新读回落库结果。
    let after = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": id, "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("chunk exists");

    // 请求体能表达的字段被更新。
    assert_eq!(after.title, "新标题", "title 被更新");

    // 「请求体无法表达」的字段保持原值（核心防回归：不再被 replace_one 清空）。
    assert_eq!(after.wiki_type.as_deref(), Some("methodology"), "wiki_type 保持原值");
    assert_eq!(after.chunk_type, "style_template", "chunk_type 保持原值(不被重置为 product_fact)");
    assert_eq!(
        after.provenance.as_ref().map(|p| p.source.as_str()),
        Some("imported"),
        "provenance 保持原值(知识来源追溯)"
    );
    assert_eq!(
        after.provenance.as_ref().and_then(|p| p.source_doc_id.as_deref()),
        Some("doc-seed"),
        "provenance.source_doc_id 保持原值"
    );
    assert_eq!(
        after.locked_fields.as_deref(),
        Some(&["title".to_string(), "body".to_string()][..]),
        "locked_fields 保持原值(编辑保护清单)"
    );
    assert_eq!(after.dynamic_confidence, Some(0.66), "dynamic_confidence 保持原值");
    assert_eq!(after.integrity_score, Some(0.88), "integrity_score 保持原值");
    assert_eq!(after.created_at, created, "created_at 保持原值(不被篡改成更新时间)");
}

/// PUT 一个不存在的 chunk → NotFound（create 有独立 POST 端点，PUT 不该 upsert）。
#[tokio::test]
#[ignore]
async fn put_nonexistent_chunk_returns_not_found() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let body: OperationKnowledgeChunkRequest =
        serde_json::from_value(serde_json::json!({ "title": "无主体" })).expect("deserialize");

    let result = update_operation_knowledge_chunk(
        State(app.state.clone()),
        admin(&ws),
        Path(ObjectId::new().to_hex()),
        Json(body),
    )
    .await;

    assert!(result.is_err(), "PUT 不存在的 chunk 必须返回错误(NotFound)而非静默 no-op");
}

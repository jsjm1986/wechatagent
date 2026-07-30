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
    let app = TestApp::start_repl_set().await;
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
        // 只锁 body、不锁 title：本测试的意图是「请求体无法表达的字段在 replace_one 后存活」
        // （provenance / wiki_type / locked_fields 数组本身 / created_at ...），而非验证锁字段强制。
        // KB-11 落地后 PUT 会强制 per-chunk 锁定字段，若这里仍锁 title 则 PUT 的 title 会被静默回滚、
        // 与下方「title 被更新」断言冲突（锁字段强制的正例由 put_enforces_locked_fields_and_writes_audit_revision 覆盖）。
        locked_fields: Some(vec!["body".to_string()]),
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
    assert_eq!(
        after.wiki_type.as_deref(),
        Some("methodology"),
        "wiki_type 保持原值"
    );
    assert_eq!(
        after.chunk_type, "style_template",
        "chunk_type 保持原值(不被重置为 product_fact)"
    );
    assert_eq!(
        after.provenance.as_ref().map(|p| p.source.as_str()),
        Some("imported"),
        "provenance 保持原值(知识来源追溯)"
    );
    assert_eq!(
        after
            .provenance
            .as_ref()
            .and_then(|p| p.source_doc_id.as_deref()),
        Some("doc-seed"),
        "provenance.source_doc_id 保持原值"
    );
    assert_eq!(
        after.locked_fields.as_deref(),
        Some(&["body".to_string()][..]),
        "locked_fields 保持原值(编辑保护清单)"
    );
    assert_eq!(
        after.dynamic_confidence,
        Some(0.66),
        "dynamic_confidence 保持原值"
    );
    assert_eq!(
        after.integrity_score,
        Some(0.88),
        "integrity_score 保持原值"
    );
    assert_eq!(
        after.created_at, created,
        "created_at 保持原值(不被篡改成更新时间)"
    );
}

/// KB-10 + KB-11 红线:admin PUT 走 replace_one 整条替换时——
/// (KB-10) 补写一条 chunk_revisions 审计行(op=patch/source=human),补齐 admin 直接编辑修订链;
/// (KB-11) 运营 per-chunk `locked_fields` 后端强制:PUT 试图改锁定字段(title)被静默丢弃、
///          未锁字段(summary)正常更新。
///
/// 之前 admin PUT 既不留审计行(修订链缺口),又能绕过 per-chunk 锁定字段(前端隐藏≠后端强制)。
/// 修复(crud.rs:281-326)复用 `effective_locked_fields` + `enforce_locked_fields` 同一份纯函数
/// (与 apply_chunk_revision 单一真相源),replace 后补写 ChunkRevision。
/// 一旦 enforce_locked_fields 调用被移除,锁定的 title 会被改掉,本测试立刻红。
#[tokio::test]
#[ignore]
async fn put_enforces_locked_fields_and_writes_audit_revision() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let id = ObjectId::new();
    let created = BsonDt::from_millis(1_000_000);

    // seed:一条只锁 title 的 chunk(per-chunk locked_fields=["title"])。
    let seeded = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: ws.clone(),
        domain: "user_operations".to_string(),
        title: "锁定标题".to_string(),
        summary: Some("原摘要".to_string()),
        status: "active".to_string(),
        chunk_type: "product_fact".to_string(),
        locked_fields: Some(vec!["title".to_string()]),
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

    // PUT 同时改锁定的 title 与未锁的 summary(camelCase 请求体)。
    let body: OperationKnowledgeChunkRequest = serde_json::from_value(serde_json::json!({
        "title": "试图改",
        "summary": "新摘要"
    }))
    .expect("deserialize OperationKnowledgeChunkRequest");

    let _ = update_operation_knowledge_chunk(
        State(app.state.clone()),
        admin(&ws),
        Path(id.to_hex()),
        Json(body),
    )
    .await
    .expect("update handler ok");

    // reload 落库结果。
    let after = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": id, "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("chunk exists");

    // (KB-11) 锁定字段 title 被静默丢弃,保持原值。
    assert_eq!(
        after.title, "锁定标题",
        "PUT 不得改动 per-chunk 锁定字段 title(后端强制),实得 {:?}",
        after.title
    );
    // 未锁字段 summary 正常更新。
    assert_eq!(
        after.summary.as_deref(),
        Some("新摘要"),
        "未锁字段 summary 应正常更新,实得 {:?}",
        after.summary
    );

    // (KB-10) admin PUT 补写一条 chunk_revisions 审计行(op=patch/source=human)。
    let revision_count = app
        .state
        .db
        .chunk_revisions()
        .count_documents(
            doc! { "chunk_id": id.to_hex(), "op": "patch", "source": "human" },
            None,
        )
        .await
        .expect("count chunk_revisions");
    assert!(
        revision_count >= 1,
        "admin PUT 必须补写 chunk_revisions 审计行(op=patch/source=human),实得 {revision_count}"
    );
}

/// PUT 一个不存在的 chunk → NotFound（create 有独立 POST 端点，PUT 不该 upsert）。
#[tokio::test]
#[ignore]
async fn put_nonexistent_chunk_returns_not_found() {
    let app = TestApp::start_repl_set().await;
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

    assert!(
        matches!(result, Err(wechatagent::error::AppError::NotFound(_))),
        "PUT 不存在的 chunk 必须返回 NotFound 而非静默 no-op 或其他错误变体，实际 {result:?}"
    );
}

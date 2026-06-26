//! `integrity_report_d2_e2e` —— E9：integrity-report 的 D2 降级计数（anchorsMissing）端到端集成测试。
//!
//! 直接调用 `routes::build_operation_knowledge_integrity_report`（pub，绕过 axum HTTP
//! harness 与 `pub(in crate::routes)` 的 handler）。
//!
//! 覆盖：
//! - 写 2 条 active chunk：一条无 source_anchors（应计入 anchorsMissing），一条有 anchors（不计）。
//! - 另写 1 条 draft 无 anchors（status != active，不计，验证口径只数 active）。
//! - 断言 `anchorsMissing == 1`。
//!
//! D2 降级口径对齐 digest_inbox.rs:455：`status=="active" && source_anchors.is_empty()`。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。本地不跑，留 CI。

mod common;

use mongodb::bson::{oid::ObjectId, DateTime as BsonDt, Document};
use wechatagent::models::OperationKnowledgeChunk;
use wechatagent::routes::ext_knowledge::build_operation_knowledge_integrity_report;

use crate::common::TestApp;

fn chunk(workspace_id: &str, title: &str, status: &str, with_anchor: bool) -> OperationKnowledgeChunk {
    let source_anchors = if with_anchor {
        let mut anchor = Document::new();
        anchor.insert("documentId", "doc_test");
        anchor.insert("startLine", 1i32);
        anchor.insert("endLine", 5i32);
        anchor.insert("quoteHash", "hash_xyz");
        vec![anchor]
    } else {
        Vec::new()
    };
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title}")),
        body: Some(format!("正文：{title}")),
        status: status.to_string(),
        integrity_status: Some("needs_review".to_string()),
        source_anchors,
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore]
async fn integrity_report_counts_active_without_anchor_as_d2_degraded() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();

    // active 无锚点 → 计入 anchorsMissing。
    let c1 = chunk(&ws, "active 缺锚点", "active", false);
    // active 有锚点 → 不计。
    let c2 = chunk(&ws, "active 有锚点", "active", true);
    // draft 无锚点 → status != active，不计（验证口径只数 active）。
    let c3 = chunk(&ws, "draft 缺锚点", "draft", false);

    for c in [&c1, &c2, &c3] {
        app.state
            .db
            .operation_knowledge_chunks()
            .insert_one(c, None)
            .await
            .expect("insert chunk");
    }

    let report = build_operation_knowledge_integrity_report(&app.state, &ws, &account_id)
        .await
        .expect("build integrity report");

    assert_eq!(
        report["anchorsMissing"], 1,
        "只有 active 且缺 source_anchors 的 chunk 应计入 D2 降级（anchorsMissing）"
    );
    assert_eq!(report["total"], 3, "三条 chunk 全部计入 total");
}

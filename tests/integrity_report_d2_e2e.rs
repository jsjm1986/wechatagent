//! `integrity_report_d2_e2e` —— E9：integrity-report 的 D2 降级计数（anchorsMissing）端到端集成测试。
//!
//! 直接调用 `routes::build_operation_knowledge_integrity_report`（pub，绕过 axum HTTP
//! harness 与 `pub(in crate::routes)` 的 handler）。
//!
//! 覆盖（B3 锚口径统一后）：
//! - active 无 source_anchors → 计入 anchorsMissing；
//! - active 有**可引用**锚（含非空 sourceQuote）→ 不计；
//! - active 只有**畸形**锚（有定位字段、无 sourceQuote）→ 计入（与
//!   `models::chunk_has_citable_anchor` 同口径：不可引用即视同缺锚）；
//! - draft 无锚 → status != active，不计（口径只数 active）。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。本地不跑，留 CI。

mod common;

use mongodb::bson::{oid::ObjectId, DateTime as BsonDt, Document};
use wechatagent::models::OperationKnowledgeChunk;
use wechatagent::routes::ext_knowledge::build_operation_knowledge_integrity_report;

use crate::common::TestApp;

#[derive(Clone, Copy)]
enum AnchorShape {
    None,
    /// 完整可引用锚：定位字段 + 非空 sourceQuote。
    Citable,
    /// 畸形锚：只有定位字段、无 sourceQuote——`anchor_is_citable` 判不可引用。
    Malformed,
}

fn chunk(
    workspace_id: &str,
    title: &str,
    status: &str,
    shape: AnchorShape,
) -> OperationKnowledgeChunk {
    let source_anchors = match shape {
        AnchorShape::None => Vec::new(),
        AnchorShape::Citable | AnchorShape::Malformed => {
            let mut anchor = Document::new();
            anchor.insert("documentId", "doc_test");
            anchor.insert("startLine", 1i32);
            anchor.insert("endLine", 5i32);
            anchor.insert("quoteHash", "hash_xyz");
            if matches!(shape, AnchorShape::Citable) {
                anchor.insert("sourceQuote", "被引用的原文片段");
            }
            vec![anchor]
        }
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
    let c1 = chunk(&ws, "active 缺锚点", "active", AnchorShape::None);
    // active 可引用锚（含 sourceQuote）→ 不计。
    let c2 = chunk(&ws, "active 可引用锚", "active", AnchorShape::Citable);
    // draft 无锚点 → status != active，不计（验证口径只数 active）。
    let c3 = chunk(&ws, "draft 缺锚点", "draft", AnchorShape::None);
    // active 畸形锚（无 sourceQuote，永远无法被引用）→ 与缺锚同判，计入。
    let c4 = chunk(&ws, "active 畸形锚", "active", AnchorShape::Malformed);

    for c in [&c1, &c2, &c3, &c4] {
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
        report["anchorsMissing"], 2,
        "active 且无可引用锚（缺锚或畸形锚）的 chunk 应计入 D2 降级（anchorsMissing）"
    );
    assert_eq!(report["total"], 4, "四条 chunk 全部计入 total");
}

//! auto-verify enforce 接线集成测试:验证 handler **真调**
//! `enforce_verified_needs_human_audit`(verify.rs:401 接线点)。
//! 全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test knowledge_auto_verify_enforce_integration -- --ignored`。
//!
//! ## 红线意义(P0):AI 永不自动 verify
//! `enforce_verified_needs_human_audit`(verify.rs:554)对**所有 chunk_type** 强制
//! `verified → needs_human_audit`。纯函数已有单测(verify.rs:566-591),但没有测试验证
//! handler **调用了它**——删掉 verify.rs:401 那行接线,verified chunk 会落库 verified,
//! 而现有测试套仍全绿。本测试直调 handler,断言过闸的 chunk 全部落 needs_human_audit
//! (response `verified==0` && `needsHumanAudit>=1` + 落库复查无 verified),钉死接线。
//!
//! ## 为何 seed 3 条(规避 5% 抽样随机性)
//! handler 在 `enforce_verified_needs_human_audit` 之前有一步 `fastrand` 抽样
//! (verify.rs:392,`sample_rate` 命中则改 needs_human_audit)。`enforce` 在时,3 条
//! **确定性**全变 needs_human_audit;若 `enforce` 接线被删,3 条能变红当且仅当
//! **没有一条**被 5% 抽样命中(否则那条也是 needs_human_audit,掩盖回归)——即
//! 「删接线后测试仍误绿」的概率 = 0.05³ ≈ 0.000125,故删接线后 ~99.99% 变红。
//! 单条则误绿概率高达 5%,不足以钉死接线。
//!
//! ## 测试形态(state-only 直调 handler)
//! 沿用 `tests/annotation_quality_gate_integration.rs` / `knowledge_chat_apply_integration.rs`
//! 惯例:`TestApp` 是 state-only 工厂(无 HTTP server),直调 route handler 真函数
//! (axum extractor 手工构造),`push_response` 预排 LLM mock,落库后查 DB 断言。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Json, State};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use serde_json::json;

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::OperationKnowledgeChunk;
// `mod verify` 在 routes/knowledge/mod.rs 是**私有**模块,但 handler 经 `pub use verify::*`
// 再导出,故按再导出路径引用(不能走 `...::verify::...` 私有模块路径)。
use wechatagent::routes::knowledge::auto_verify_operation_knowledge_chunks;

use crate::common::TestApp;

/// 构造测试 admin auth context(`current_workspace` 决定 handler 可见/可写范围)。
/// 仿 `tests/annotation_quality_gate_integration.rs::test_admin`。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "auto_verify_admin".to_string(),
        username: "auto_verify_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一条「齐证据、待审」的运营知识切片:
/// - `domain="user_operations"` + `integrity_status="needs_review"` → 命中 handler 查询;
/// - 非空 `source_quote` + 非空 `source_anchors` → `decide_auto_verify_status` 的
///   `has_source_quote`/`has_source_anchor` 两条件满足,配合 mock 返 verified + confidence≥threshold,
///   使 `final_status` 在 `enforce` 之前先算成 `"verified"`(隔离出 enforce 的作用)。
/// `account_id=Some("default")` 对齐 handler 默认 account_id(config.default_account_id),
/// 命中 `$or: [{account_id:null},{account_id:&account_id}]` 过滤。
fn seed_chunk(workspace_id: &str, idx: usize) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: Some("default".to_string()),
        domain: "user_operations".to_string(),
        // 每条 title/body 各不相同 → user prompt 各异 → generate_agent_json 的 LRU
        // 缓存不命中 → 三条各消费一条 push_response 的 mock。
        title: format!("待审知识切片-{idx}"),
        body: Some(format!("正文内容-{idx}:这是一条需要核验的运营知识。")),
        summary: Some(format!("摘要-{idx}")),
        // D2/证据齐备:非空 source_quote + 非空 source_anchors。
        source_quote: Some(format!("原文引用-{idx}:客户提出的问题原话。")),
        source_anchors: vec![doc! { "quote": format!("原文引用-{idx}"), "start": 0i32 }],
        integrity_status: Some("needs_review".to_string()),
        confidence_score: Some(50),
        status: "active".to_string(),
        ..Default::default()
    }
}

/// 红线:auto_verify handler 对过闸(证据齐 + LLM 自称 verified + confidence≥threshold)
/// 的 chunk,**必须**经 `enforce_verified_needs_human_audit` 强制降级 needs_human_audit——
/// 绝不自动落 verified(AI 永不自动 verify)。
///
/// 删除 verify.rs:401 `final_status = enforce_verified_needs_human_audit(final_status);`
/// 这一行接线后,本测试 ~99.99% 变红(response `verified>0` 且落库出现 verified)。
#[tokio::test]
#[ignore]
async fn auto_verify_handler_enforces_needs_human_audit() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // seed 3 条齐证据、待审的 chunk(3 条规避 5% 抽样随机性,见文件头说明)。
    for idx in 0..3 {
        app.state
            .db
            .operation_knowledge_chunks()
            .insert_one(seed_chunk(&ws, idx), None)
            .await
            .expect("seed chunk 应成功");
    }

    // 每条 chunk 一次 LLM 调用:mock 返「LLM 自称 verified + 高 confidence」。
    // 若无 enforce 接线,这会让 decide_auto_verify_status 判成 verified 并落库。
    for _ in 0..3 {
        app.llm.push_response(json!({
            "confidenceScore": 10,
            "integrityStatus": "verified",
            "verifiedClaims": [],
            "distortionRisks": []
        }));
    }

    // 调 handler:confidenceThreshold=0 让 confidence>=threshold 恒真(隔离出 enforce
    // 的作用,排除「因 confidence 不足而降级」的干扰);humanAuditSampleRate=0.05 保留
    // 最低抽样率(见文件头:enforce 在时结果与抽样无关,3 条全 needs_human_audit)。
    let resp = auto_verify_operation_knowledge_chunks(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Json(
            serde_json::from_value(json!({
                "confidenceThreshold": 0,
                "humanAuditSampleRate": 0.05
            }))
            .expect("反序列化 KnowledgeAutoVerifyRequest 应成功"),
        ),
    )
    .await
    .expect("auto_verify handler 应成功");

    // ── 断言 1:response body(handler 出参层) ──
    let body = resp.0;
    assert_eq!(
        body["processed"].as_i64(),
        Some(3),
        "3 条 chunk 都应被处理(每条消费一条 mock LLM 响应)"
    );
    assert_eq!(
        body["verified"].as_i64(),
        Some(0),
        "enforce 强制所有 verified→needs_human_audit,verified 计数必须为 0(删 verify.rs:401 接线会 >0)"
    );
    assert!(
        body["needsHumanAudit"].as_i64().unwrap_or(0) >= 1,
        "过闸的 chunk 应落 needsHumanAudit,实际 body={body:?}"
    );

    // ── 断言 2:落库复查(DB 层,不重拼 filter 自查——查全部 user_operations chunk) ──
    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find(doc! { "domain": "user_operations" }, None)
        .await
        .expect("查 chunk 应成功")
        .try_collect::<Vec<OperationKnowledgeChunk>>()
        .await
        .expect("collect chunk 应成功");
    assert_eq!(stored.len(), 3, "应有 3 条落库");
    assert!(
        stored
            .iter()
            .all(|c| c.integrity_status.as_deref() != Some("verified")),
        "AI 永不自动 verify:落库不得有 integrity_status=verified(红线),实际={:?}",
        stored
            .iter()
            .map(|c| c.integrity_status.clone())
            .collect::<Vec<_>>()
    );
    // 补强:enforce 在时确定性全变 needs_human_audit(3 条 confidence=10≥0 + verified + 齐证据)。
    assert!(
        stored
            .iter()
            .all(|c| c.integrity_status.as_deref() == Some("needs_human_audit")),
        "enforce 在时 3 条应确定性全为 needs_human_audit,实际={:?}",
        stored
            .iter()
            .map(|c| c.integrity_status.clone())
            .collect::<Vec<_>>()
    );
}

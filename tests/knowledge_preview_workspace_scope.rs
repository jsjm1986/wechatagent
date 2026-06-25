//! KNOW-1 回归:无 contact 的知识预览必须按传入 workspace 隔离,
//! 不回落 default_workspace_id 读到 DEFAULT 租户知识。默认 #[ignore],需 Docker。
//!
//! 守 `test_knowledge_route_for_contact`(`src/agent/knowledge_router.rs`)的新增
//! `workspace_id` 参数:contact=None 的预览路径必须用调用方(admin)的 workspace 合成
//! 预览 contact,而非回落 `state.config.default_workspace_id`——否则非 default workspace
//! 的 admin 会读到 DEFAULT 租户的知识切片正文(跨租户读泄漏)。

mod common;

use mongodb::bson::{oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::agent::test_knowledge_route_for_contact;
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

fn verified_chunk(workspace_id: &str, title: &str, body: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: None, // workspace 共享 chunk
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title}")),
        body: Some(body.to_string()),
        wiki_type: Some("thesis".to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        dynamic_confidence: Some(0.9),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore]
async fn preview_without_contact_scopes_to_passed_workspace() {
    let app = TestApp::start().await;

    // 在非 default 的 ws_a 插一条 active+verified chunk(含可命中关键词)。
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&verified_chunk("ws_a", "保修政策", "测试产品保修两年"), None)
        .await
        .expect("insert chunk");

    // 轮 1(传 ws_a):agent 显式 0 cited → fallback_rank 在 ws_a corpus 上取 top-N。
    app.llm.push_response(json!({
        "action": "answer",
        "answer": "暂无足够依据。",
        "citedChunkIds": [],
        "sourceQuotes": [],
    }));

    // 传 ws_a → 应能命中 ws_a 的知识。
    let hit = test_knowledge_route_for_contact(&app.state, None, "ws_a", "acc_a", "保修多久")
        .await
        .expect("route ws_a");
    let chunks_a = hit
        .get_array("selectedChunks")
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(chunks_a >= 1, "传 ws_a 应命中 ws_a 的知识");

    // 传 default workspace → corpus 空,短路在 missing → 命中不到 ws_a 的 chunk(隔离生效)。
    let default_ws = app.state.config.default_workspace_id.clone();
    let miss =
        test_knowledge_route_for_contact(&app.state, None, &default_ws, "acc_a", "保修多久")
            .await
            .expect("route default");
    let chunks_def = miss
        .get_array("selectedChunks")
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(chunks_def, 0, "default workspace 不应命中 ws_a 的知识");
}

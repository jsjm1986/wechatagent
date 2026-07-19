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

/// 从 `test_knowledge_route_for_contact` 返回的 Document 里抽出 `selectedChunks`
/// 数组里每个元素的 `id`(ObjectId hex,见 knowledge_router::operation_knowledge_chunk_to_bson)。
/// 用 chunk id 判隔离而非整体 count,可在 default workspace 被 seed 时仍精确区分
/// “隔离生效 vs 恰好没数据”。
fn selected_chunk_ids(route: &mongodb::bson::Document) -> Vec<String> {
    route
        .get_array("selectedChunks")
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_document())
                .filter_map(|d| d.get_str("id").ok().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
#[ignore]
async fn preview_without_contact_scopes_to_passed_workspace() {
    let app = TestApp::start().await;

    // 在非 default 的 ws_a 插一条 active+verified chunk(含可命中关键词)。
    // 记下其 id,断言时用 id 判隔离——不依赖 default 库恰好为空。
    let chunk = verified_chunk("ws_a", "保修政策", "测试产品保修两年");
    let ws_a_chunk_id = chunk.id.expect("chunk has id").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert chunk");

    // 轮 1(传 ws_a):agent 显式 0 cited → fallback_rank 在 ws_a corpus 上取 top-N。
    app.llm.push_response(json!({
        "action": "answer",
        "answer": "暂无足够依据。",
        "citedChunkIds": [],
        "sourceQuotes": [],
    }));
    // 兜底:default workspace 若被 CI seed 了 verified chunk,其 corpus 非空会触发
    // 一次 knowledge_agent::answer LLM 调用。补一条 fallback 响应,避免 mock 队列
    // 耗尽导致 default 调用 panic(假红)。default 为空时短路在 missing,本条不被消费。
    app.llm.push_response(json!({
        "action": "answer",
        "answer": "暂无足够依据。",
        "citedChunkIds": [],
        "sourceQuotes": [],
    }));

    // 传 ws_a → selectedChunks 必须包含 ws_a 那条 chunk 的 id。
    let hit = test_knowledge_route_for_contact(&app.state, None, "ws_a", "acc_a", "保修多久")
        .await
        .expect("route ws_a");
    let ids_a = selected_chunk_ids(&hit);
    assert!(
        ids_a.contains(&ws_a_chunk_id),
        "传 ws_a 应命中 ws_a 插入的 chunk(id={ws_a_chunk_id}),实得 selectedChunks ids={ids_a:?}"
    );

    // 传 default workspace → selectedChunks 不得包含 ws_a 那条 chunk 的 id(跨租户隔离生效)。
    // 用 id 判“不包含”而非整体 count==0:即便 default 被 seed 了别的 verified chunk,
    // 只要 ws_a 的这条不可见即证明隔离成立,消除 default corpus 非空时的假红。
    let default_ws = app.state.config.default_workspace_id.clone();
    let miss = test_knowledge_route_for_contact(&app.state, None, &default_ws, "acc_a", "保修多久")
        .await
        .expect("route default");
    let ids_def = selected_chunk_ids(&miss);
    assert!(
        !ids_def.contains(&ws_a_chunk_id),
        "default workspace 不应命中 ws_a 的 chunk(id={ws_a_chunk_id}),实得 selectedChunks ids={ids_def:?}"
    );
}

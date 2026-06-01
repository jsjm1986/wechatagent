//! 知识库闭环轨迹测试：维护 agent 编辑 KB → 再召回 → 召回保持。
//!
//! 召回是查询时实时计算的（list_catalog 的 rank_key = relevance×trust×recency，
//! 无物化索引）。所以本测试直接调用 `pub fn list_catalog()` 取确定性排序，断言：
//!   1. 不回归：基线命中的 chunk 写入后仍在 catalog。
//!   2. 新内容可召回：新增 verified chunk 对其目标 query 可被召回。
//!   3. SUPERSEDE 旧降新升：旧 chunk superseded_by 打标 → trust×0.1 降权 → 新 chunk 排前。
//!   4. 关系图完整：related_chunks 引用全部能在 catalog/库内解析，无悬空。
//!   5. 负例：未审定 draft（integrity_status≠verified）不得出现在默认 catalog。
//!
//! 全程红线：apply 写入恒走 draft+needs_review 起步，verified 必须显式经
//! verify_operation_knowledge_chunk（生产审批路径），agent 永不自动审定。
//! `#[ignore]`：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use wechatagent::agent::knowledge_agent::{list_catalog, CatalogFilter};
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

const WS: &str = "ws_closed_loop";

/// 种子 chunk 工厂：默认 verified + 带 source_quote/source_anchors（满足后续 verify gate）。
/// `body_terms` 用于让 title/summary/body 含 query 关键词，驱动 rank_key 命中。
fn seed_chunk(title: &str, body_terms: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: WS.to_string(),
        account_id: None,
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title} {body_terms}")),
        body: Some(format!("正文：{title}。{body_terms}")),
        wiki_type: Some("methodology".to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        source_quote: Some(format!("原文引用：{title}")),
        source_anchors: vec![doc! { "documentId": "seed_doc", "quote": title }],
        dynamic_confidence: Some(0.9),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

/// 清空本 ws 的 chunk，保证 catalog 干净。
async fn reset_ws(app: &TestApp) {
    app.state
        .db
        .operation_knowledge_chunks()
        .delete_many(doc! { "workspaceId": WS }, None)
        .await
        .expect("clean ws_closed_loop chunks");
}

/// 便捷：对 query 跑默认（verified-only）catalog，返回 chunk_id 顺序列表。
async fn catalog_ids(app: &TestApp, query: &str) -> Vec<String> {
    let entries = list_catalog(
        &app.state,
        WS,
        None,
        &CatalogFilter::default(),
        Some(query),
    )
    .await
    .expect("list_catalog");
    entries.into_iter().map(|e| e.chunk_id).collect()
}

#[tokio::test]
#[ignore]
async fn smoke_catalog_returns_seeded_chunk() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    let chunk = seed_chunk("价格异议处理", "客户嫌贵 价格 异议 让步话术");
    let hex = chunk.id.expect("oid").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert seed");

    let ids = catalog_ids(&app, "客户嫌价格贵怎么办").await;
    assert!(ids.contains(&hex), "种子 chunk 应出现在 catalog：{ids:?}");
}

/// 门 1a：写入新 chunk 后，基线命中的 chunk 仍在 catalog（不回归），且新 chunk 可召回。
#[tokio::test]
#[ignore]
async fn write_then_recall_preserves_baseline_and_adds_new() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    // 基线：两条已有知识。
    let base_a = seed_chunk("已读不回唤回三阶段", "已读不回 唤回 沉默客户 激活");
    let base_b = seed_chunk("新客开场白模板", "新客户 首次 开场白 破冰");
    let base_a_hex = base_a.id.expect("oid").to_hex();
    let base_b_hex = base_b.id.expect("oid").to_hex();
    for c in [&base_a, &base_b] {
        app.state.db.operation_knowledge_chunks().insert_one(c, None).await.expect("insert base");
    }

    // 基线召回快照。
    let before = catalog_ids(&app, "客户已读不回怎么唤回").await;
    assert!(before.contains(&base_a_hex), "基线应命中 base_a：{before:?}");

    // 维护 agent 新增一条相关知识（draft→verified 路径在 Task 6 验证；此处直接种 verified）。
    let added = seed_chunk("已读不回唤回话术升级版", "已读不回 唤回 二次激活 限时优惠");
    let added_hex = added.id.expect("oid").to_hex();
    app.state.db.operation_knowledge_chunks().insert_one(&added, None).await.expect("insert added");

    // 写入后再召回（live，无索引重建）。
    let after = catalog_ids(&app, "客户已读不回怎么唤回").await;

    // 不回归：基线命中的 base_a 仍在。
    assert!(after.contains(&base_a_hex), "写入后基线命中 base_a 不应丢失：{after:?}");
    // 新内容可召回：added 出现。
    assert!(after.contains(&added_hex), "新增 chunk 应可被召回：{after:?}");
    let _ = base_b_hex;
}

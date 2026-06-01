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
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::ext_knowledge::{
    verify_operation_knowledge_chunk, KnowledgeVerifyRequest,
};

use axum::extract::{Path, State};
use axum::{Extension, Json};

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

/// 门 1b：SUPERSEDE 旧降新升。旧 chunk 被 superseded_by 指向新 chunk → trust×0.1 →
/// 同 query 下新 chunk 必须排在旧 chunk 之前。验证「结构化写永不物理删除」——旧 chunk
/// 仍在库（未被删），只是降权。
#[tokio::test]
#[ignore]
async fn supersede_demotes_old_below_new() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    // 旧版 + 新版，相同主题（query 相关度相近），靠 trust 拉开。
    let old = seed_chunk("竞品对比方法论 v1", "竞品对比 客观陈述 优劣 旧版");
    let new = seed_chunk("竞品对比方法论 v2", "竞品对比 客观陈述 优劣 升级");
    let old_hex = old.id.expect("oid").to_hex();
    let new_hex = new.id.expect("oid").to_hex();
    for c in [&old, &new] {
        app.state.db.operation_knowledge_chunks().insert_one(c, None).await.expect("insert");
    }

    // 维护 agent 取代：旧版打 superseded_by=新版。物理保留旧 chunk。
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": old.id.unwrap() },
            doc! { "$set": { "superseded_by": &new_hex } },
            None,
        )
        .await
        .expect("mark superseded");

    let ids = catalog_ids(&app, "竞品对比怎么客观陈述").await;
    let pos_old = ids.iter().position(|x| x == &old_hex);
    let pos_new = ids.iter().position(|x| x == &new_hex);
    // 旧 chunk 仍在库（未被物理删）——查得到。
    let still_exists = app.state.db.operation_knowledge_chunks()
        .find_one(doc! { "_id": old.id.unwrap() }, None).await.expect("find old").is_some();
    assert!(still_exists, "SUPERSEDE 不得物理删除旧 chunk");
    // 新版必须排在旧版之前（旧版 trust×0.1 降权）。
    match (pos_new, pos_old) {
        (Some(pn), Some(po)) => assert!(pn < po, "新版应排在旧版之前：new@{pn} old@{po} ids={ids:?}"),
        (Some(_), None) => { /* 旧版被降到 catalog 尾部之外也可接受（更强的降权） */ }
        _ => panic!("新版 chunk 必须可召回：ids={ids:?}"),
    }
}

/// 门 1c：关系图完整。写入带 related_chunks 的 chunk 后，其每条引用的 chunk_id
/// 都能在库内解析（无悬空引用）。validate「结构化写」维护关系链完整。
#[tokio::test]
#[ignore]
async fn relation_graph_has_no_dangling_refs() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    let target = seed_chunk("价格异议处理", "价格 异议 让步 话术");
    let target_hex = target.id.expect("oid").to_hex();
    app.state.db.operation_knowledge_chunks().insert_one(&target, None).await.expect("insert target");

    // 维护 agent 新增一条 chunk，关系指向 target。
    let mut linked = seed_chunk("价格异议进阶应对", "价格 异议 进阶 谈判");
    linked.related_chunks = Some(vec![RelatedRef {
        chunk_id: target_hex.clone(),
        kind: "references".to_string(),
        note: None,
    }]);
    let linked_hex = linked.id.expect("oid").to_hex();
    app.state.db.operation_knowledge_chunks().insert_one(&linked, None).await.expect("insert linked");

    // 校验：linked 的每条 related_chunks 引用都能在库内 find 到（无悬空）。
    let fetched = app.state.db.operation_knowledge_chunks()
        .find_one(doc! { "_id": linked.id.unwrap() }, None).await.expect("find linked")
        .expect("linked exists");
    for r in fetched.related_chunks.unwrap_or_default() {
        let ref_oid = ObjectId::parse_str(&r.chunk_id).expect("related chunk_id is valid oid");
        let resolved = app.state.db.operation_knowledge_chunks()
            .find_one(doc! { "_id": ref_oid }, None).await.expect("find related").is_some();
        assert!(resolved, "related_chunks 引用 {} 必须能解析（无悬空）", r.chunk_id);
    }
    assert!(catalog_ids(&app, "价格异议").await.contains(&linked_hex), "linked 应可召回");
}

/// 门 1d（负例 + 审批路径）：维护 agent 提案落 draft+needs_review 时不可召回；
/// 仅在显式经生产 verify 审批转 verified 后才进 catalog。锁住「AI 永不自动审定」。
#[tokio::test]
#[ignore]
async fn unverified_draft_not_recallable_until_approved() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    // 维护 agent 提案：落 needs_review（带 source_quote/source_anchors 以便后续 verify）。
    let mut draft = seed_chunk("退款时效说明", "退款 时效 到账 周期");
    draft.integrity_status = Some("needs_review".to_string());
    // status 仍 active（catalog 默认 status=active 过滤），靠 integrity_status 把它挡在外面。
    let draft_hex = draft.id.expect("oid").to_hex();
    app.state.db.operation_knowledge_chunks().insert_one(&draft, None).await.expect("insert draft");

    // 负例：未审定不可召回（默认 catalog 只暴露 integrity_status=verified）。
    let before = catalog_ids(&app, "退款多久到账").await;
    assert!(!before.contains(&draft_hex), "未审定 draft 不得出现在默认 catalog：{before:?}");

    // 经生产审批路径转 verified。
    let admin = Extension(AuthenticatedAdmin {
        user_id: "closed_loop_admin".into(),
        username: "closed_loop_admin".into(),
        current_workspace: WS.to_string(),
    });
    let req: KnowledgeVerifyRequest =
        serde_json::from_value(serde_json::json!({ "verifiedClaims": [] })).expect("verify req");
    let resp = verify_operation_knowledge_chunk(
        State(app.state.clone()),
        admin,
        Path(draft_hex.clone()),
        Json(req),
    )
    .await
    .expect("verify must succeed");
    assert_eq!(resp.0.get("ok").and_then(|v| v.as_bool()), Some(true), "verify 应成功：{:?}", resp.0);

    // 审批后可召回。
    let after = catalog_ids(&app, "退款多久到账").await;
    assert!(after.contains(&draft_hex), "审批 verified 后应可召回：{after:?}");
}

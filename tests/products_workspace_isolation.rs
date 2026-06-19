//! objective-purchase-facts G2：`products` 多租户隔离集成测试（spec §3.5）。
//!
//! 产品 CRUD handler 是 `pub(super)`，核心隔离逻辑就是"每个 Mongo filter 含
//! `workspace_id = current_workspace`"。本测试在 `products` collection 直接写两条
//! 不同租户的产品，断言：
//!   - workspace_a 视角只看到自己的产品；
//!   - 跨 workspace 同名 product_id 合法且互不可见（复合 unique 是 workspace 内唯一）；
//!   - 未知 workspace 读不到任何产品。
//!
//! 默认 `#[ignore]`，需 Docker（testcontainers MongoDB）。
//!
//! 与 `workspace_isolation.rs` 同形态——走 collection filter shape 而非 handler，
//! 因为 handler 是 thin wrapper，隔离不变量全在"filter 必带 workspace_id"这一条。

mod common;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime as BsonDt};
use wechatagent::models::Product;

use crate::common::TestApp;

fn ws_product(workspace_id: &str, product_id: &str, name: &str) -> Product {
    Product {
        id: None,
        workspace_id: workspace_id.to_string(),
        product_id: product_id.to_string(),
        name: name.to_string(),
        price: Some(19900),
        currency: Some("CNY".to_string()),
        sku: None,
        status: "active".to_string(),
        summary: None,
        attributes: Default::default(),
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
    }
}

#[tokio::test]
#[ignore]
async fn products_filter_blocks_cross_tenant_read() {
    let app = TestApp::start().await;
    app.state.db.ensure_indexes().await.expect("ensure indexes");

    // 两租户、product_id 故意同名 "annual-vip"——验证 workspace 内唯一、跨租户合法。
    app.state
        .db
        .products()
        .insert_many(
            vec![
                ws_product("workspace_a", "annual-vip", "A 的年度会员"),
                ws_product("workspace_b", "annual-vip", "B 的年度会员"),
            ],
            None,
        )
        .await
        .expect("insert products");

    let coll = app.state.db.products();
    let mut cursor = coll
        .find(doc! { "workspace_id": "workspace_a" }, None)
        .await
        .expect("find workspace_a");
    let mut names = Vec::new();
    while let Some(p) = cursor.try_next().await.expect("cursor next") {
        names.push(p.name);
    }
    assert_eq!(
        names,
        vec!["A 的年度会员".to_string()],
        "workspace_a 只能看到自己的产品，实际：{:?}",
        names
    );
}

#[tokio::test]
#[ignore]
async fn product_id_unique_per_workspace_not_global() {
    let app = TestApp::start().await;
    app.state.db.ensure_indexes().await.expect("ensure indexes");

    // 同一 workspace 内插两条同 product_id → 第二条违反 unique 索引。
    app.state
        .db
        .products()
        .insert_one(ws_product("workspace_a", "dup-sku", "第一条"), None)
        .await
        .expect("first insert ok");
    let dup = app
        .state
        .db
        .products()
        .insert_one(ws_product("workspace_a", "dup-sku", "重复"), None)
        .await;
    assert!(
        dup.is_err(),
        "同 workspace 重复 product_id 应被 unique 索引拒绝"
    );

    // 跨 workspace 同名 product_id 合法。
    app.state
        .db
        .products()
        .insert_one(ws_product("workspace_b", "dup-sku", "B 的同名产品"), None)
        .await
        .expect("cross-workspace same product_id should be allowed");
}

#[tokio::test]
#[ignore]
async fn products_filter_returns_empty_for_unknown_tenant() {
    let app = TestApp::start().await;
    app.state.db.ensure_indexes().await.expect("ensure indexes");

    app.state
        .db
        .products()
        .insert_one(ws_product("workspace_a", "only-a", "只属于 A"), None)
        .await
        .expect("insert");

    let coll = app.state.db.products();
    let mut cursor = coll
        .find(doc! { "workspace_id": "ghost_workspace" }, None)
        .await
        .expect("find ghost");
    let mut found = Vec::new();
    while let Some(p) = cursor.try_next().await.expect("cursor next") {
        found.push(p.name);
    }
    assert!(
        found.is_empty(),
        "未知 workspace 不应看到任何产品，实际：{:?}",
        found
    );
}

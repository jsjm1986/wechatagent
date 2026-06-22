//! 簇 D 结构化组织集成测试：素材 tags 落库 + list 按 tag 过滤。
//! 全部 `#[ignore]`，需 Docker testcontainers。直调 route handler 真函数（本仓既有惯例）。
//! CI `integration` job 用 `cargo test --test structured_organization_integration -- --ignored` 跑。
//!
//! ## 测试形态说明（与簇 C media_asset_crud_integration.rs 同款）
//! 本仓 `TestApp`（`tests/common/mod.rs`）是 **state-only** 工厂：只建 testcontainers
//! Mongo + 同形 `AppState`，**没有** HTTP server。既有集成测试一律 **直调 route handler
//! 真函数**：handler 是普通 async fn，参数是 axum extractor（`State` / `Extension` /
//! `Query`），构造好就 `.await`。`ContentAssetQuery` 字段私有但派生 `Deserialize`，
//! 用 `serde_json::from_value(json!({...}))` 以 camelCase 键构造（仿簇 C
//! `UpdateMetaRequest` / `ToggleSendableRequest` 先例，无需放开字段可见性）。
//!
//! ## upload 写 tags 的端到端不在此处测（Multipart 限制）
//! `upload_*` handler 取 `Multipart` extractor，**tests crate 无法手工构造 `Multipart`**
//! （同簇 B/C 发现）。故 upload 解析并落 tags 的副作用由 render 纯函数（lib 已测）+ 代码
//! 审查保证。本文件聚焦 **DB 副作用**：直接 seed 带 tags 的 ContentAsset 入库 → 直调
//! `list_content_assets`（带 tag 过滤）→ 断言精确命中，端到端钉 list 检索行为。
//!
//! ## 可见性放开（同簇 C 先例，纯可见性零逻辑改动）
//! 为让 tests crate 直调 `list_content_assets`：`src/routes/mod.rs` 把 `mod assets`
//! 改 `pub mod assets`，`src/routes/assets.rs` 把 `list_content_assets` 与
//! `ContentAssetQuery` 的 `pub(super)` 改 `pub`。只动可见性，不动任何逻辑/签名/行为。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Query, State};
use mongodb::bson::{oid::ObjectId, DateTime};
use serde_json::json;

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::ContentAsset;
use wechatagent::routes::assets::{list_content_assets, ContentAssetQuery};
use wechatagent::routes::AppState;

// ── helpers ─────────────────────────────────────────────────────────────────

/// 构造测试 admin auth context（`current_workspace` 决定 handler 可见/可写范围）。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "structorg_admin".to_string(),
        username: "structorg_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一条带 `tags` 的媒体素材（仿簇 C make_asset，tags 由调用方指定以钉 tag 过滤场景）。
fn make_asset(workspace_id: &str, title: &str, tags: Vec<String>) -> ContentAsset {
    let now = DateTime::now();
    ContentAsset {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: None, // 全局素材（归一 scope 走空串）
        kind: "media".to_string(),
        title: title.to_string(),
        body: None,
        tags,
        url: None,
        media_id: None,
        usage_scene: None,
        media_type: Some("file".to_string()),
        file_path: None,
        file_name: Some("demo.pdf".to_string()),
        file_size: Some(1024),
        mime_type: Some("application/pdf".to_string()),
        file_sha256: Some("deadbeef".to_string()),
        sendable: Some(true),
        send_trigger_hint: None,
        target_stages: None,
        expression_pref: None,
        requires_principal_approval: Some(false),
        review_status: Some("approved".to_string()),
        review_note: None,
        created_at: now,
        updated_at: now,
    }
}

/// 构造 `ContentAssetQuery`（字段私有但派生 `Deserialize`，用 from_value + camelCase 键构造，
/// 仿簇 C `UpdateMetaRequest` 先例，无需放开字段可见性）。
fn asset_query(value: serde_json::Value) -> ContentAssetQuery {
    serde_json::from_value(value).expect("build ContentAssetQuery")
}

/// 直调 `list_content_assets` 并返回 items 中的 id（hex string）集合，便于断言含/不含。
async fn list_ids(state: &AppState, admin: &AuthenticatedAdmin, query: serde_json::Value) -> Vec<String> {
    let resp = list_content_assets(
        State(state.clone()),
        Extension(admin.clone()),
        Query(asset_query(query)),
    )
    .await
    .expect("list_content_assets 应成功");
    resp.0["items"]
        .as_array()
        .expect("items 应为数组")
        .iter()
        .filter_map(|item| item["id"].as_str().map(|s| s.to_string()))
        .collect()
}

// ── 缺口8：list 按 tag 过滤命中含该 tag 的素材、不含的被排除（检索核心回归） ─────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn list_filters_by_tag() {
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);

    // seed 两条同 workspace、不同 tag 的素材：A=["报价类"]、B=["案例类"]。
    let asset_a = make_asset(ws, "报价单素材", vec!["报价类".to_string()]);
    let asset_b = make_asset(ws, "成功案例素材", vec!["案例类".to_string()]);
    let id_a = asset_a.id.expect("asset a id").to_hex();
    let id_b = asset_b.id.expect("asset b id").to_hex();
    app.state
        .db
        .content_assets()
        .insert_one(&asset_a, None)
        .await
        .expect("insert asset a");
    app.state
        .db
        .content_assets()
        .insert_one(&asset_b, None)
        .await
        .expect("insert asset b");

    // 直调 list，按 tag="报价类" 过滤。
    let ids = list_ids(&app.state, &admin, json!({ "tag": "报价类" })).await;

    // 精确命中：含 A（tag 匹配）、不含 B（tag 不匹配）。
    assert!(
        ids.contains(&id_a),
        "tag=报价类 过滤结果应含 A（tags=[报价类]），实际 ids={ids:?}"
    );
    assert!(
        !ids.contains(&id_b),
        "tag=报价类 过滤结果不应含 B（tags=[案例类]），实际 ids={ids:?}"
    );
}

// ── 缺口8：跨 workspace 的 tag 过滤不泄漏（workspace 隔离 / IDOR） ─────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn list_tag_filter_respects_workspace() {
    // other_ws 有一条 tags=["报价类"] 的素材 → default workspace 的 admin 按 tag="报价类"
    // 查 → filter 保留 workspace_id scope → 结果不应含外域素材。
    let app = common::TestApp::start().await;
    let admin = test_admin("default");

    let foreign = make_asset("other_ws", "外域报价素材", vec!["报价类".to_string()]);
    let foreign_id = foreign.id.expect("asset id").to_hex();
    app.state
        .db
        .content_assets()
        .insert_one(&foreign, None)
        .await
        .expect("insert foreign-workspace asset");

    // default workspace 的 admin 按 tag 查。
    let ids = list_ids(&app.state, &admin, json!({ "tag": "报价类" })).await;

    // 跨 workspace 隔离：tag 匹配但 workspace 不同 → 不泄漏。
    assert!(
        !ids.contains(&foreign_id),
        "default workspace 的 tag 过滤不应含 other_ws 的素材（workspace 隔离），实际 ids={ids:?}"
    );
}

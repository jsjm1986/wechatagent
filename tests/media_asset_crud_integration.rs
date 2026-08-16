//! 簇 C 素材库 CRUD 补全集成测试：edit 元数据 / toggle / delete 引用计数。
//! 全部 `#[ignore]`，需 Docker testcontainers。直调 route handler 真函数（本仓既有惯例）。
//! CI `integration` job 用 `cargo test --test media_asset_crud_integration -- --ignored` 跑。
//!
//! ## 测试形态说明（与簇 B annotation_quality_gate_integration.rs 同款）
//! 本仓 `TestApp`（`tests/common/mod.rs`）是 **state-only** 工厂：只建 testcontainers
//! Mongo + 同形 `AppState`，**没有** HTTP server。既有集成测试一律 **直调 route handler
//! 真函数**：handler 是普通 async fn，参数是 axum extractor（`State` / `Extension` /
//! `Path` / `Json`），构造好就 `.await`，错误以 `Err(AppError::BadRequest|NotFound)` 变体
//! 断言（直调时不经 axum→不产 HTTP 状态码）。本文件沿用该惯例。
//!
//! ## 换文件端点不在此处端到端测（Multipart 限制）
//! `replace_content_asset_file` handler 取 `Multipart` extractor，**tests crate 无法手工
//! 构造 `Multipart`**（同簇 B Task8 发现，见 `src/routes/mod.rs` 对 `import_*` 同款说明）。
//! 故换文件的「清 media_id + 退 draft」副作用不在集成层验，由代码审查保证（handler 内是
//! 确定性 `$set { media_id: null, review_status: "draft" }`）。换文件复用的**旧文件引用
//! 计数清理**逻辑与 delete 共用同一谓词 `should_delete_physical_file`，已被本文件 delete
//! 兄弟引用测试 + lib 纯函数单测覆盖。
//!
//! ## 文件落盘隔离
//! 多测试同 binary 内并行跑，共用 `config.media_storage_dir`（默认 `./media`）会撞文件。
//! 故每个涉及文件的测试克隆 `AppState` 把 `media_storage_dir` 指向**进程内唯一**的临时目录
//! （`std::env::temp_dir()` + 随机 sha），测试结束删目录，互不干扰。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Json, Path, Query, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::ContentAsset;
use wechatagent::routes::media_assets::{
    delete_content_asset, review_media_asset, toggle_content_asset_sendable,
    update_content_asset_meta, AssetScopeRequest, ReviewRequest, ToggleSendableRequest,
    UpdateMetaRequest,
};
use wechatagent::routes::AppState;

// ── helpers ─────────────────────────────────────────────────────────────────

/// 构造测试 admin auth context（`current_workspace` 决定 handler 可见/可写范围）。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "aqgc_admin".to_string(),
        username: "aqgc_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一条媒体素材。`review_status` / `sendable` / `target_stages` / `file_path` 由调用方指定，
/// 便于各测试钉具体场景。
fn make_asset(
    workspace_id: &str,
    title: &str,
    review_status: &str,
    sendable: bool,
    file_path: Option<String>,
    target_stages: Option<Vec<String>>,
) -> ContentAsset {
    let now = DateTime::now();
    ContentAsset {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: None, // 全局素材（归一 scope 走空串）
        kind: "media".to_string(),
        title: title.to_string(),
        body: None,
        tags: vec![],
        url: None,
        media_id: None,
        usage_scene: None,
        media_type: Some("file".to_string()),
        file_path,
        file_name: Some("demo.pdf".to_string()),
        file_size: Some(1024),
        mime_type: Some("application/pdf".to_string()),
        file_sha256: Some("deadbeef".to_string()),
        sendable: Some(sendable),
        send_trigger_hint: None,
        target_stages,
        expression_pref: None,
        requires_principal_approval: Some(false),
        review_status: Some(review_status.to_string()),
        review_note: None,
        min_inject_tier: None,
        enabled: Some(true),
        allowed_insertion_levels: None,
        usage_guidance: None,
        created_at: now,
        updated_at: now,
    }
}

/// 构造 `UpdateMetaRequest`（字段私有但派生 `Deserialize`，用 from_value 构造——
/// 仿簇 B `ReviewRequest` / `assist_override_request` 先例，无需放开字段可见性）。
fn update_meta_request(value: serde_json::Value) -> UpdateMetaRequest {
    let mut value = value;
    value
        .as_object_mut()
        .expect("request object")
        .insert("expectedScope".into(), json!("workspace"));
    serde_json::from_value(value).expect("build UpdateMetaRequest")
}

/// 构造 `ToggleSendableRequest`（同上）。
fn toggle_request(sendable: bool) -> ToggleSendableRequest {
    serde_json::from_value(json!({
        "expectedScope": "workspace",
        "sendable": sendable
    }))
    .expect("build ToggleSendableRequest")
}

fn workspace_scope() -> Query<AssetScopeRequest> {
    Query(
        serde_json::from_value(json!({ "expectedScope": "workspace" }))
            .expect("build workspace asset scope"),
    )
}

/// 克隆 `AppState`，把 `media_storage_dir` 指向进程内唯一临时目录，返回 (state, root)。
/// 复用 `TestApp` 已建好的 Mongo 容器 / LLM mock，仅覆盖文件落盘根目录，避免多测试撞文件。
fn state_with_unique_media_dir(app: &common::TestApp) -> (AppState, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("aqgc_media_{}", ObjectId::new().to_hex()));
    let mut config = app.state.config.clone();
    config.media_storage_dir = root.to_string_lossy().to_string();
    let mut state = app.state.clone();
    state.config = config;
    (state, root)
}

/// 回查某 asset（workspace 隔离）。None 表示记录不存在。
async fn find_asset(state: &AppState, workspace_id: &str, oid: ObjectId) -> Option<ContentAsset> {
    state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": workspace_id }, None)
        .await
        .expect("query content_asset")
}

// ── edit 元数据：改字段落库，review_status 不变 ───────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn edit_meta_updates_fields_keeps_review_status() {
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);

    // seed 一个 approved 素材。
    let asset = make_asset(ws, "旧标题", "approved", true, None, None);
    let asset_id = asset.id.expect("asset id");
    app.state
        .db
        .content_assets()
        .insert_one(&asset, None)
        .await
        .expect("insert approved asset");

    // 直调 edit handler：改 title。
    let _ = update_content_asset_meta(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(asset_id.to_hex()),
        Json(update_meta_request(json!({ "title": "新标题" }))),
    )
    .await
    .expect("edit meta 应成功");

    // 回查：title 已变、review_status 仍 approved（改元数据不退审）。
    let updated = find_asset(&app.state, ws, asset_id)
        .await
        .expect("asset exists");
    assert_eq!(updated.title, "新标题", "title 应被更新");
    assert_eq!(
        updated.review_status.as_deref(),
        Some("approved"),
        "改元数据不应退审，review_status 仍为 approved"
    );
    app.cleanup().await;
}

// ── edit 元数据：target_stages 越界 → 400 ─────────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn edit_meta_out_of_dict_stage_rejected() {
    // TestApp::start() 总是 re-seed m006 customer_stage 字典（含 9 个 canonical 条目）。
    // 改 target_stages 为字典内不存在、也非任何 alias 的阶段名 → 归一越界 → Err(BadRequest)。
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);

    let asset = make_asset(ws, "待编辑素材", "approved", true, None, None);
    let asset_id = asset.id.expect("asset id");
    app.state
        .db
        .content_assets()
        .insert_one(&asset, None)
        .await
        .expect("insert asset");

    let result = update_content_asset_meta(
        State(app.state.clone()),
        Extension(admin),
        Path(asset_id.to_hex()),
        Json(update_meta_request(
            json!({ "targetStages": ["不存在的阶段名"] }),
        )),
    )
    .await;
    assert!(
        matches!(result, Err(AppError::BadRequest(_))),
        "字典已配置时越界 stage 必须 BadRequest（400），实际: {result:?}"
    );

    // 副作用断言：越界被拒，target_stages 不应被写入（仍为 None）。
    let unchanged = find_asset(&app.state, ws, asset_id)
        .await
        .expect("asset exists");
    assert_eq!(
        unchanged.target_stages, None,
        "越界被拒后 target_stages 不应落地"
    );
    app.cleanup().await;
}

// ── toggle：写 sendable ───────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn toggle_sets_sendable() {
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);

    // seed sendable=true 的素材。
    let asset = make_asset(ws, "可发素材", "approved", true, None, None);
    let asset_id = asset.id.expect("asset id");
    app.state
        .db
        .content_assets()
        .insert_one(&asset, None)
        .await
        .expect("insert asset");

    // 直调 toggle handler：停用（sendable=false）。
    let resp = toggle_content_asset_sendable(
        State(app.state.clone()),
        Extension(admin),
        Path(asset_id.to_hex()),
        Json(toggle_request(false)),
    )
    .await
    .expect("toggle 应成功");
    assert_eq!(
        resp.0.get("sendable").and_then(|v| v.as_bool()),
        Some(false),
        "toggle 返回 sendable:false"
    );

    // 回查：sendable 落库为 false。
    let updated = find_asset(&app.state, ws, asset_id)
        .await
        .expect("asset exists");
    assert_eq!(
        updated.sendable,
        Some(false),
        "toggle(false) 应写入 sendable=false"
    );
    app.cleanup().await;
}

// ── toggle：跨 workspace 404（IDOR） ─────────────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn toggle_cross_workspace_404() {
    // IDOR 守卫：asset 落在 other_ws，admin 在 default → update_one 锁 workspace
    // → matched_count==0 → NotFound（不可跨 workspace 写）。
    let app = common::TestApp::start().await;
    let admin = test_admin("default");

    let foreign = make_asset("other_ws", "外域素材", "approved", true, None, None);
    let foreign_id = foreign.id.expect("asset id");
    app.state
        .db
        .content_assets()
        .insert_one(&foreign, None)
        .await
        .expect("insert foreign-workspace asset");

    let result = toggle_content_asset_sendable(
        State(app.state.clone()),
        Extension(admin),
        Path(foreign_id.to_hex()),
        Json(toggle_request(false)),
    )
    .await;
    assert!(
        matches!(result, Err(AppError::NotFound(_))),
        "跨 workspace asset 必须 NotFound（404，IDOR 守卫），实际: {result:?}"
    );

    // 副作用断言：外域素材 sendable 未被改（仍为 true）。
    let unchanged = find_asset(&app.state, "other_ws", foreign_id)
        .await
        .expect("foreign asset exists");
    assert_eq!(unchanged.sendable, Some(true), "跨 workspace 写必须不落地");
    app.cleanup().await;
}

// ── delete：无兄弟引用 → DB 记录删 + 物理文件删 ───────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn delete_removes_db_and_file_when_no_siblings() {
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);
    let (state, root) = state_with_unique_media_dir(&app);

    // 真落一个物理文件。
    let bytes = b"unique-asset-bytes";
    let sha = wechatagent::media_storage::sha256_hex(bytes);
    let rel =
        wechatagent::media_storage::safe_relative_path(ws, &sha, "pdf").expect("safe rel path");
    wechatagent::media_storage::store_bytes(&root, &rel, bytes)
        .await
        .expect("store bytes");
    assert!(root.join(&rel).exists(), "前置：文件应已落盘");

    // seed 唯一引用它的 asset。
    let asset = make_asset(ws, "唯一引用素材", "draft", true, Some(rel.clone()), None);
    let asset_id = asset.id.expect("asset id");
    state
        .db
        .content_assets()
        .insert_one(&asset, None)
        .await
        .expect("insert asset");

    // 直调 delete handler。
    let _ = delete_content_asset(
        State(state.clone()),
        Extension(admin),
        Path(asset_id.to_hex()),
        workspace_scope(),
    )
    .await
    .expect("delete 应成功");

    // DB 记录没了。
    assert!(
        find_asset(&state, ws, asset_id).await.is_none(),
        "delete 后 DB 记录应消失"
    );
    // 物理文件也没了（无兄弟引用 → 引用计数为 0 → 物理删）。
    assert!(!root.join(&rel).exists(), "无兄弟引用时物理文件应被删除");

    // 清理临时目录。
    let _ = tokio::fs::remove_dir_all(&root).await;
    app.cleanup().await;
}

// ── delete：有兄弟引用 → DB 记录删但物理文件保留（核心回归） ──────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn delete_keeps_file_when_sibling_references_it() {
    // 缺口 4 核心保护点：两条 asset 共享同一 file_path（upload 不去重，同文件多传 = 多记录
    // 共享一物理文件）。删其中一条后，被删记录消失、兄弟记录还在、**物理文件必须仍在**
    // （引用计数防误删）。
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);
    let (state, root) = state_with_unique_media_dir(&app);

    // 真落一个物理文件。
    let bytes = b"shared-asset-bytes";
    let sha = wechatagent::media_storage::sha256_hex(bytes);
    let rel =
        wechatagent::media_storage::safe_relative_path(ws, &sha, "pdf").expect("safe rel path");
    wechatagent::media_storage::store_bytes(&root, &rel, bytes)
        .await
        .expect("store bytes");
    assert!(root.join(&rel).exists(), "前置：共享文件应已落盘");

    // seed 两条共享同一 file_path 的 asset。
    let asset_a = make_asset(ws, "共享素材A", "draft", true, Some(rel.clone()), None);
    let asset_b = make_asset(ws, "共享素材B", "draft", true, Some(rel.clone()), None);
    let id_a = asset_a.id.expect("asset a id");
    let id_b = asset_b.id.expect("asset b id");
    state
        .db
        .content_assets()
        .insert_one(&asset_a, None)
        .await
        .expect("insert asset a");
    state
        .db
        .content_assets()
        .insert_one(&asset_b, None)
        .await
        .expect("insert asset b");

    // 删 A。
    let _ = delete_content_asset(
        State(state.clone()),
        Extension(admin),
        Path(id_a.to_hex()),
        workspace_scope(),
    )
    .await
    .expect("delete a 应成功");

    // A 记录没了、B 还在。
    assert!(
        find_asset(&state, ws, id_a).await.is_none(),
        "被删记录 A 应消失"
    );
    assert!(
        find_asset(&state, ws, id_b).await.is_some(),
        "兄弟记录 B 应保留"
    );
    // 物理文件仍在（B 仍引用 → 引用计数 1 > 0 → 不物理删）。
    assert!(
        root.join(&rel).exists(),
        "兄弟引用存在时物理文件必须保留（引用计数防误删）"
    );

    // 清理临时目录。
    let _ = tokio::fs::remove_dir_all(&root).await;
    app.cleanup().await;
}

// ── delete：跨 workspace 404（IDOR） ─────────────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn delete_cross_workspace_404() {
    // IDOR 守卫：asset 落在 other_ws，admin 在 default → find_one 锁 workspace miss
    // → NotFound，且 asset 未被删。
    let app = common::TestApp::start().await;
    let admin = test_admin("default");

    let foreign = make_asset("other_ws", "外域待删素材", "approved", true, None, None);
    let foreign_id = foreign.id.expect("asset id");
    app.state
        .db
        .content_assets()
        .insert_one(&foreign, None)
        .await
        .expect("insert foreign-workspace asset");

    let result = delete_content_asset(
        State(app.state.clone()),
        Extension(admin),
        Path(foreign_id.to_hex()),
        workspace_scope(),
    )
    .await;
    assert!(
        matches!(result, Err(AppError::NotFound(_))),
        "跨 workspace asset 必须 NotFound（404，IDOR 守卫），实际: {result:?}"
    );

    // 副作用断言：外域素材未被删（仍可在 other_ws 查到）。
    assert!(
        find_asset(&app.state, "other_ws", foreign_id)
            .await
            .is_some(),
        "跨 workspace 删除必须不生效"
    );
    app.cleanup().await;
}

// ── SR-160：账号私有素材的实体 scope 是所有写动作的 CAS 身份 ────────────────

#[tokio::test]
#[ignore]
async fn wrong_asset_scope_is_conflict_with_zero_document_and_audit_writes() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let admin = test_admin(&workspace);
    let mut asset = make_asset(
        &workspace,
        "account-a private asset",
        "draft",
        true,
        None,
        None,
    );
    asset.account_id = Some("account-a".to_string());
    let asset_id = asset.id.expect("asset id");
    app.state
        .db
        .content_assets()
        .insert_one(&asset, None)
        .await
        .expect("seed private asset");
    let collection = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("content_assets");
    let before = collection
        .find_one(doc! { "_id": asset_id }, None)
        .await
        .expect("read before")
        .expect("asset exists");

    let wrong_review: ReviewRequest = serde_json::from_value(json!({
        "expectedScope": "account",
        "expectedAccountId": "account-b",
        "status": "approved",
        "note": "must not persist"
    }))
    .expect("review request");
    let review_result = review_media_asset(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(asset_id.to_hex()),
        Json(wrong_review),
    )
    .await;
    assert!(matches!(review_result, Err(AppError::Conflict(_))));

    let workspace_meta: UpdateMetaRequest = serde_json::from_value(json!({
        "expectedScope": "workspace",
        "title": "must not persist"
    }))
    .expect("meta request");
    let meta_result = update_content_asset_meta(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(asset_id.to_hex()),
        Json(workspace_meta),
    )
    .await;
    assert!(matches!(meta_result, Err(AppError::Conflict(_))));

    let wrong_toggle: ToggleSendableRequest = serde_json::from_value(json!({
        "expectedScope": "account",
        "expectedAccountId": "account-b",
        "sendable": false
    }))
    .expect("toggle request");
    let toggle_result = toggle_content_asset_sendable(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(asset_id.to_hex()),
        Json(wrong_toggle),
    )
    .await;
    assert!(matches!(toggle_result, Err(AppError::Conflict(_))));

    let delete_result = delete_content_asset(
        State(app.state.clone()),
        Extension(admin),
        Path(asset_id.to_hex()),
        workspace_scope(),
    )
    .await;
    assert!(matches!(delete_result, Err(AppError::Conflict(_))));

    let after = collection
        .find_one(doc! { "_id": asset_id }, None)
        .await
        .expect("read after")
        .expect("asset remains");
    let audit_count = app
        .state
        .db
        .events()
        .count_documents(
            doc! {
                "workspace_id": &workspace,
                "kind": "media_asset.reviewed",
                "details.asset_id": asset_id.to_hex(),
            },
            None,
        )
        .await
        .expect("count review audit");

    assert_eq!(
        after, before,
        "all rejected scope writes must leave BSON unchanged"
    );
    assert_eq!(audit_count, 0, "rejected review must not write audit");
    app.cleanup().await;
}

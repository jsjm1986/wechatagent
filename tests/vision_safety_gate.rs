//! `vision_safety_gate` —— P1-5 / #574 multimodal 图片导入的视觉模型解析与安全闸。
//!
//! 守的红线：
//!   1. **既无视觉文字主模型、也无专职视觉副模型** → `import-apply-image` 必须
//!      502 `visionNotSupported`，不静默吞掉、不产 chunk。
//!   2. **active 文字主模型本身 supports_vision** → 复用运行时 provider 抽取，
//!      产出的 chunk 一律 `draft` + `needs_review`（AI 永不自动 verify）。
//!   3. **文字主模型不支持图片，但配了专职视觉副模型（isVisionActive+supportsVision）**
//!      → 解析成功；这里用 TestApp 的 mock LLM 验证「Runtime 分支」语义即可，
//!      专职副模型走真实 HTTP 构造，单元层不联网（留给 design 文档/手测）。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用
//! `cargo test --test vision_safety_gate -- --ignored`（需 Docker）。

mod common;

use axum::extract::State;
use axum::{Extension, Json};
use mongodb::bson::{oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::LlmProviderConfig;
use wechatagent::routes::ext_knowledge::{
    import_operation_knowledge_apply_image, ImportApplyImageRequest,
};

use crate::common::TestApp;

fn admin(app: &TestApp) -> Extension<AuthenticatedAdmin> {
    Extension(AuthenticatedAdmin {
        user_id: "test_admin".into(),
        username: "test_admin".into(),
        current_workspace: app.state.config.default_workspace_id.clone(),
    })
}

fn provider(
    ws: &str,
    provider_id: &str,
    is_active: bool,
    supports_vision: bool,
    is_vision_active: bool,
) -> LlmProviderConfig {
    LlmProviderConfig {
        id: Some(ObjectId::new()),
        workspace_id: ws.to_string(),
        provider_id: provider_id.to_string(),
        name: provider_id.to_string(),
        format: "openai".to_string(),
        base_url: "http://test-llm.invalid".to_string(),
        api_key: "test-key".to_string(),
        model: "test-model".to_string(),
        is_active,
        timeout_seconds: None,
        max_retries: None,
        retry_base_ms: None,
        supports_vision,
        is_vision_active,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
    }
}

async fn insert_provider(app: &TestApp, cfg: &LlmProviderConfig) {
    app.state
        .db
        .llm_provider_configs()
        .insert_one(cfg, None)
        .await
        .expect("insert provider");
}

fn image_req() -> ImportApplyImageRequest {
    ImportApplyImageRequest {
        // 任意非空 base64（mock LLM 不会真读图）。
        image_base64: "aGVsbG8=".to_string(),
        mime: Some("image/png".to_string()),
        source_name: Some("smoke_image".to_string()),
        account_id: None,
        hint: None,
    }
}

/// 场景 1：active 文字主模型不支持图片、且无专职视觉副模型 → 502 visionNotSupported。
#[tokio::test]
#[ignore]
async fn image_import_without_any_vision_model_is_rejected() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // 仅一个纯文字 active 模型（supports_vision=false），无副模型。
    insert_provider(&app, &provider(&ws, "text_only", true, false, false)).await;

    let result = import_operation_knowledge_apply_image(
        State(app.state.clone()),
        admin(&app),
        Json(image_req()),
    )
    .await;

    let err = result.expect_err("无视觉能力必须报错");
    let msg = format!("{err}");
    assert!(
        msg.contains("visionNotSupported"),
        "错误信息应含 visionNotSupported: {msg}",
    );
    // 没有任何 chunk 落库。
    let count = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(mongodb::bson::doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks");
    assert_eq!(count, 0, "被拒绝时不应落任何 chunk");
}

/// 场景 2：完全没有任何 provider 配置 → 也必须 502 visionNotSupported（不 panic）。
#[tokio::test]
#[ignore]
async fn image_import_with_no_provider_at_all_is_rejected() {
    let app = TestApp::start().await;

    let result = import_operation_knowledge_apply_image(
        State(app.state.clone()),
        admin(&app),
        Json(image_req()),
    )
    .await;

    let err = result.expect_err("无任何 provider 必须报错");
    assert!(
        format!("{err}").contains("visionNotSupported"),
        "错误信息应含 visionNotSupported",
    );
}

/// 场景 3：active 文字主模型本身 supports_vision → 走 Runtime 分支（state.llm mock），
/// 抽取出 fence → 落 chunk，全部 draft + needs_review。
#[tokio::test]
#[ignore]
async fn image_import_with_vision_capable_primary_produces_review_chunks() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    insert_provider(&app, &provider(&ws, "vision_primary", true, true, false)).await;

    // mock LLM 返回 fence 文本（与 vision prompt 约定 {"fence": "..."} 一致）。
    app.llm.push_response(json!({
        "fence": "---CHUNK: img-c1---\n# 图片抽取小节\n图片里的可读文本摘要。\n---END---",
    }));

    let resp = import_operation_knowledge_apply_image(
        State(app.state.clone()),
        admin(&app),
        Json(image_req()),
    )
    .await
    .expect("vision-capable primary 应成功");
    let body = resp.0;

    let chunk_ids = body["chunkIds"].as_array().expect("chunkIds array");
    assert!(!chunk_ids.is_empty(), "应至少产 1 chunk: {body:?}");

    for id in chunk_ids {
        let id_hex = id.as_str().unwrap();
        let chunk = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(
                mongodb::bson::doc! {
                    "_id": ObjectId::parse_str(id_hex).unwrap(),
                    "workspace_id": &ws,
                },
                None,
            )
            .await
            .unwrap()
            .expect("chunk exists");
        assert_eq!(chunk.status, "draft", "vision chunk 必须 draft");
        assert_eq!(
            chunk.integrity_status.as_deref(),
            Some("needs_review"),
            "vision chunk 必须 needs_review",
        );
    }
}

/// 场景 4：vision 返回空 fence → 不产 chunk，但不报错（返回 note）。
#[tokio::test]
#[ignore]
async fn image_import_with_empty_vision_output_yields_no_chunk() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    insert_provider(&app, &provider(&ws, "vision_primary", true, true, false)).await;
    app.llm.push_response(json!({ "fence": "" }));

    let resp = import_operation_knowledge_apply_image(
        State(app.state.clone()),
        admin(&app),
        Json(image_req()),
    )
    .await
    .expect("空 fence 应正常返回");
    let body = resp.0;

    assert!(
        body["chunkIds"].as_array().map(|a| a.is_empty()).unwrap_or(false),
        "空 fence 不应产 chunk: {body:?}",
    );
    let count = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(mongodb::bson::doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks");
    assert_eq!(count, 0, "空 fence 不落 chunk");
}

//! LLM provider 热切换红线集成测试:activate 后 DB 不变量"恰一条 isActive=true 且是 target"。
//! 全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test llm_provider_activate_integration -- --ignored`。
//!
//! ## 假绿手法对照(审计实证):原测试只断言 HTTP 200,从不查 DB 恰一条 isActive=true,
//! 也不验 registry 真换 provider。本测试直调 activate_provider handler 后**真查 DB**断言不变量。
//! registry swap 因 TestApp.llm_registry=None 不可达(需另构造),本文件只钉死 DB 不变量;
//! registry 真换留 biz-test(阶段2)真 LLM 验证。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, Query, State};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};

use wechatagent::auth::{AdminUser, AuthenticatedAdmin};
use wechatagent::models::LlmProviderConfig;
use wechatagent::routes::llm_providers::activate_provider;

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "llm_admin".to_string(),
        username: "llm_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

fn make_provider(ws: &str, provider_id: &str, active: bool) -> LlmProviderConfig {
    let now = DateTime::now();
    LlmProviderConfig {
        id: None,
        workspace_id: ws.to_string(),
        provider_id: provider_id.to_string(),
        name: provider_id.to_string(),
        format: "openai".to_string(),
        base_url: "http://llm.example/v1".to_string(),
        api_key: "sk-test".to_string(),
        model: "demo-model".to_string(),
        is_active: active,
        timeout_seconds: None,
        max_retries: None,
        retry_base_ms: None,
        supports_vision: false,
        is_vision_active: false,
        created_at: now,
        updated_at: now,
    }
}

/// seed 一个 user_id=llm_admin、ACL=[ws] 的 admin，让 H3 ACL 闸 get_admin_user 命中。
/// 用 typed `Collection<AdminUser>` 写入（与生产 bootstrap_admin_if_needed 同源），
/// created_at 走 chrono `Utc::now()` 落 RFC3339 字符串——typed 读端才能反序列化。
async fn seed_admin(app: &TestApp, ws: &str) {
    let user = AdminUser {
        user_id: "llm_admin".to_string(),
        username: "llm_admin".to_string(),
        password_hash: "x".to_string(),
        created_at: Utc::now(),
        last_login_at: None,
        workspaces: vec![ws.to_string()],
        default_workspace: Some(ws.to_string()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&user, None)
        .await
        .expect("seed admin");
}

/// 红线:activate target 后,同 workspace 恰好一条 isActive=true,且就是 target。
#[tokio::test]
#[ignore]
async fn activate_yields_exactly_one_active_and_is_target() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    // seed admin（user_id=llm_admin，workspaces=[ws]），让 H3 ACL 闸 get_admin_user 命中。
    seed_admin(&app, &ws).await;
    let coll = app.state.db.llm_provider_configs();
    // seed:p_old 当前 active,p_new 未激活
    coll.insert_one(make_provider(&ws, "p_old", true), None)
        .await
        .expect("seed p_old");
    coll.insert_one(make_provider(&ws, "p_new", false), None)
        .await
        .expect("seed p_new");

    // 直调 activate_provider(p_new)。Query/Path 用 serde/构造。
    let query: Query<wechatagent::routes::llm_providers::ListQuery> =
        Query(serde_json::from_value(serde_json::json!({})).expect("构造 ListQuery"));
    let resp = activate_provider(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path("p_new".to_string()),
        query,
    )
    .await
    .expect("activate_provider 应成功");
    let _ = resp;

    // 真查 DB:恰一条 isActive=true 且 providerId==p_new
    let mut cursor = coll
        .find(doc! { "workspaceId": &ws, "isActive": true }, None)
        .await
        .expect("查 active provider");
    let mut active_ids = Vec::new();
    while let Some(c) = cursor.try_next().await.expect("cursor") {
        active_ids.push(c.provider_id);
    }
    assert_eq!(
        active_ids,
        vec!["p_new".to_string()],
        "activate 后必须恰一条 isActive=true 且是 p_new,实际 {active_ids:?}"
    );
}

/// 红线:activate 不存在的 provider → NotFound(不静默成功)。
#[tokio::test]
#[ignore]
async fn activate_missing_provider_not_found() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    // seed admin（user_id=llm_admin，workspaces=[ws]），让 H3 ACL 闸 get_admin_user 命中。
    seed_admin(&app, &ws).await;
    let query: Query<wechatagent::routes::llm_providers::ListQuery> =
        Query(serde_json::from_value(serde_json::json!({})).expect("构造 ListQuery"));
    let result = activate_provider(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path("does_not_exist".to_string()),
        query,
    )
    .await;
    assert!(
        result.is_err(),
        "activate 不存在的 provider 必须 NotFound"
    );
}

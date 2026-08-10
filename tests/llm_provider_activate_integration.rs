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

use axum::{
    extract::{Extension, Path, Query, State},
    Router,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use reqwest::StatusCode;
use std::sync::Arc;
use tokio::net::TcpListener;

use wechatagent::auth::{
    session::create_session, AdminUser, AuthenticatedAdmin, SESSION_COOKIE_NAME,
};
use wechatagent::llm::{LlmClient, LlmFormat, LlmProviderMeta, LlmRegistry};
use wechatagent::models::LlmProviderConfig;
use wechatagent::routes::{api_router, llm_providers::activate_provider};

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

async fn start_api(app: &TestApp, ws: &str) -> (String, String, tokio::task::JoinHandle<()>) {
    let admin = app
        .state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .find_one(doc! { "user_id": "llm_admin" }, None)
        .await
        .expect("load admin")
        .expect("admin exists");
    let session = create_session(&app.state.db, &admin, 1, ws)
        .await
        .expect("create session");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind API");
    let address = listener.local_addr().expect("API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

fn test_registry(ws: &str, provider_id: &str, model: &str) -> Arc<LlmRegistry> {
    let client = LlmClient::with_format(
        "http://127.0.0.1:1/v1".to_string(),
        "sk-registry-test".to_string(),
        model.to_string(),
        LlmFormat::Openai,
        1,
        0,
        1,
    )
    .expect("build registry client");
    Arc::new(LlmRegistry::new(
        ws,
        client,
        LlmProviderMeta {
            provider_id: provider_id.to_string(),
            format: LlmFormat::Openai,
            model: model.to_string(),
            base_url: "http://127.0.0.1:1/v1".to_string(),
            revision_ms: 0,
            runtime_fingerprint: format!("fixture:{provider_id}:{model}"),
        },
    ))
}

/// 红线:activate target 后,同 workspace 恰好一条 isActive=true,且就是 target。
#[tokio::test]
#[ignore]
async fn activate_yields_exactly_one_active_and_is_target() {
    let app = TestApp::start_repl_set().await;
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
    assert!(result.is_err(), "activate 不存在的 provider 必须 NotFound");
}

/// SR-165: editing the active provider is a production release. Missing test
/// approval or a stale revision must fail before either MongoDB or the runtime
/// registry changes.
#[tokio::test]
#[ignore]
async fn active_update_without_approval_or_with_stale_revision_is_zero_write() {
    let mut app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    seed_admin(&app, &ws).await;
    let provider = make_provider(&ws, "p_active", true);
    let expected_updated_at = provider.updated_at.timestamp_millis();
    app.state
        .db
        .llm_provider_configs()
        .insert_one(provider, None)
        .await
        .expect("seed active provider");

    let registry = test_registry(&ws, "p_active", "demo-model");
    app.state.llm_registry = Some(registry.clone());
    let before_registry = registry.snapshot(&ws).await.expect("registry before");
    let providers = app
        .state
        .db
        .raw()
        .collection::<Document>("llm_provider_configs");
    let before = providers
        .find_one(doc! { "workspaceId": &ws, "providerId": "p_active" }, None)
        .await
        .expect("load provider before")
        .expect("provider exists");

    let (base_url, cookie, server) = start_api(&app, &ws).await;
    let client = reqwest::Client::new();
    let update_url = format!("{base_url}/admin/llm-providers/p_active");
    let base_body = serde_json::json!({
        "providerId": "p_active",
        "name": "must not persist",
        "format": "chat",
        "baseUrl": "http://127.0.0.1:2/v1",
        "apiKey": "sk-****test",
        "model": "must-not-load",
        "timeoutSeconds": 5,
        "maxRetries": 1,
        "retryBaseMs": 10,
        "supportsVision": false,
        "expectedUpdatedAt": expected_updated_at,
        "activeUpdateConfirmed": true
    });

    let missing_approval = client
        .put(&update_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&base_body)
        .send()
        .await
        .expect("missing approval update");
    assert_eq!(missing_approval.status(), StatusCode::CONFLICT);

    let mut stale_body = base_body;
    stale_body["expectedUpdatedAt"] = serde_json::json!(expected_updated_at + 1);
    stale_body["activeUpdateTestToken"] = serde_json::json!("untrusted-token");
    let stale_revision = client
        .put(&update_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&stale_body)
        .send()
        .await
        .expect("stale revision update");
    assert_eq!(stale_revision.status(), StatusCode::CONFLICT);

    let after = providers
        .find_one(doc! { "workspaceId": &ws, "providerId": "p_active" }, None)
        .await
        .expect("load provider after")
        .expect("provider exists");
    assert_eq!(after, before, "rejected active updates must be zero-write");
    let after_registry = registry.snapshot(&ws).await.expect("registry after");
    assert_eq!(after_registry.generation, before_registry.generation);
    assert_eq!(
        after_registry.meta.provider_id,
        before_registry.meta.provider_id
    );
    assert_eq!(after_registry.meta.model, before_registry.meta.model);
    server.abort();
}

/// SR-166: omitted nullable tuning fields preserve provider overrides, while
/// explicit JSON null removes the stored overrides and exposes the effective
/// global defaults in the response.
#[tokio::test]
#[ignore]
async fn nullable_tuning_fields_preserve_or_unset_and_report_effective_defaults() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    seed_admin(&app, &ws).await;

    let mut provider = make_provider(&ws, "p_nullable", false);
    provider.timeout_seconds = Some(17);
    provider.max_retries = Some(2);
    provider.retry_base_ms = Some(321);
    app.state
        .db
        .llm_provider_configs()
        .insert_one(provider, None)
        .await
        .expect("seed provider overrides");

    let (base_url, cookie, server) = start_api(&app, &ws).await;
    let client = reqwest::Client::new();
    let update_url = format!("{base_url}/admin/llm-providers/p_nullable");
    let base_body = serde_json::json!({
        "providerId": "p_nullable",
        "name": "nullable provider",
        "format": "chat",
        "baseUrl": "http://llm.example/v1",
        "apiKey": "sk-****test",
        "model": "demo-model",
        "supportsVision": false
    });

    let omitted = client
        .put(&update_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&base_body)
        .send()
        .await
        .expect("omitted tuning update");
    assert_eq!(omitted.status(), StatusCode::OK);
    let after_omitted = app
        .state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &ws, "providerId": "p_nullable" },
            None,
        )
        .await
        .expect("load after omitted update")
        .expect("provider exists");
    assert_eq!(after_omitted.timeout_seconds, Some(17));
    assert_eq!(after_omitted.max_retries, Some(2));
    assert_eq!(after_omitted.retry_base_ms, Some(321));

    let mut clear_body = base_body;
    clear_body["timeoutSeconds"] = serde_json::Value::Null;
    clear_body["maxRetries"] = serde_json::Value::Null;
    clear_body["retryBaseMs"] = serde_json::Value::Null;
    let cleared = client
        .put(&update_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&clear_body)
        .send()
        .await
        .expect("explicit null tuning update");
    assert_eq!(cleared.status(), StatusCode::OK);
    let response: serde_json::Value = cleared.json().await.expect("decode clear response");
    assert_eq!(
        response["item"]["effectiveTimeoutSeconds"],
        app.state.config.llm_timeout_seconds
    );
    assert_eq!(response["item"]["timeoutSecondsSource"], "global_default");
    assert_eq!(
        response["item"]["effectiveMaxRetries"],
        app.state.config.llm_max_retries
    );
    assert_eq!(response["item"]["maxRetriesSource"], "global_default");
    assert_eq!(
        response["item"]["effectiveRetryBaseMs"],
        app.state.config.llm_retry_base_ms
    );
    assert_eq!(response["item"]["retryBaseMsSource"], "global_default");

    let raw = app
        .state
        .db
        .raw()
        .collection::<Document>("llm_provider_configs")
        .find_one(
            doc! { "workspaceId": &ws, "providerId": "p_nullable" },
            None,
        )
        .await
        .expect("load raw provider")
        .expect("provider exists");
    assert!(!raw.contains_key("timeoutSeconds"));
    assert!(!raw.contains_key("maxRetries"));
    assert!(!raw.contains_key("retryBaseMs"));
    server.abort();
}

/// SR-167: a vision assignment is a lifecycle invariant, not a display flag.
/// The assigned provider cannot lose its capability or be deleted implicitly;
/// reassignment atomically leaves exactly one capable target, and the partial
/// unique index rejects any second active vision row.
#[tokio::test]
#[ignore]
async fn vision_assignment_lifecycle_is_guarded_and_reassignment_is_atomic() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();
    seed_admin(&app, &ws).await;

    let mut old = make_provider(&ws, "vision_old", false);
    old.supports_vision = true;
    old.is_vision_active = true;
    let mut target = make_provider(&ws, "vision_target", false);
    target.supports_vision = true;
    app.state
        .db
        .llm_provider_configs()
        .insert_many(vec![old, target], None)
        .await
        .expect("seed vision providers");

    let raw = app
        .state
        .db
        .raw()
        .collection::<Document>("llm_provider_configs");
    let before = raw
        .find_one(
            doc! { "workspaceId": &ws, "providerId": "vision_old" },
            None,
        )
        .await
        .expect("load assigned provider")
        .expect("assigned provider exists");
    let (base_url, cookie, server) = start_api(&app, &ws).await;
    let client = reqwest::Client::new();

    let disable = client
        .put(format!("{base_url}/admin/llm-providers/vision_old"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "providerId": "vision_old",
            "name": "vision_old",
            "format": "chat",
            "baseUrl": "http://llm.example/v1",
            "apiKey": "sk-****test",
            "model": "demo-model",
            "supportsVision": false
        }))
        .send()
        .await
        .expect("disable assigned vision capability");
    assert_eq!(disable.status(), StatusCode::CONFLICT);

    let delete = client
        .delete(format!("{base_url}/admin/llm-providers/vision_old"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("delete assigned vision provider");
    assert_eq!(delete.status(), StatusCode::CONFLICT);
    let after_rejections = raw
        .find_one(
            doc! { "workspaceId": &ws, "providerId": "vision_old" },
            None,
        )
        .await
        .expect("reload assigned provider")
        .expect("assigned provider still exists");
    assert_eq!(
        after_rejections, before,
        "rejected lifecycle changes must be zero-write"
    );

    let reassign = client
        .post(format!(
            "{base_url}/admin/llm-providers/vision_target/vision"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "active": true }))
        .send()
        .await
        .expect("reassign vision provider");
    assert_eq!(reassign.status(), StatusCode::OK);

    let mut cursor = app
        .state
        .db
        .llm_provider_configs()
        .find(doc! { "workspaceId": &ws, "isVisionActive": true }, None)
        .await
        .expect("query active vision providers");
    let mut assigned = Vec::new();
    while let Some(provider) = cursor.try_next().await.expect("vision cursor") {
        assert!(
            provider.supports_vision,
            "assigned provider must support vision"
        );
        assigned.push(provider.provider_id);
    }
    assert_eq!(assigned, vec!["vision_target".to_string()]);

    let duplicate = app
        .state
        .db
        .llm_provider_configs()
        .update_one(
            doc! { "workspaceId": &ws, "providerId": "vision_old" },
            doc! { "$set": { "isVisionActive": true } },
            None,
        )
        .await;
    assert!(
        duplicate.is_err(),
        "partial unique index must reject a second assignment"
    );

    server.abort();
}

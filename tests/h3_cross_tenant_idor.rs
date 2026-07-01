//! H3 跨租户 IDOR 回归：handler 解析的 workspaceId 必须 ∈ admin ACL，否则拒绝。
//! 直调真实 handler（activate_provider / list_providers），seed 真实 AdminUser 提供 ACL。
//! 全部 #[ignore]，需 Docker testcontainers。
//! CI: `cargo test --test h3_cross_tenant_idor -- --ignored`。
//!
//! 覆盖 14 站点共用的单一闸 `resolve_authorized_workspace` 在真实 handler 生效的
//! 两条路径：拒绝（越权 override）与放行（本租户回落）。挑 `activate_provider`
//! （进程级热切换面）+ `list_providers`（读泄漏面）两个最高危 handler；二者均已提为
//! `pub`（llm_providers.rs，仿既有集成测试先例）。其余 handler 的请求体结构体是
//! `pub(super)`，外部 test crate 不可命名，但共用同一闸，故无需逐一覆盖。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, Query, State};
use mongodb::bson::{doc, DateTime};

use wechatagent::auth::session::{authenticate, bootstrap_admin_if_needed};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::LlmProviderConfig;
use wechatagent::routes::llm_providers::{activate_provider, list_providers, ListQuery};

use crate::common::TestApp;

fn admin_ctx(user_id: &str, current_ws: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: user_id.to_string(),
        username: "h3_admin".to_string(),
        current_workspace: current_ws.to_string(),
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
        api_key: "sk-secret-b".to_string(),
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

/// seed 一个 ACL=[ws_a] 的 admin，返回其真实 user_id。
async fn seed_admin_with_acl(app: &TestApp, ws_a: &str) -> String {
    bootstrap_admin_if_needed(&app.state.db, Some("h3_admin"), Some("pw-h3-123456"), Some(ws_a))
        .await
        .expect("bootstrap admin");
    let user = authenticate(&app.state.db, "h3_admin", "pw-h3-123456")
        .await
        .expect("authenticate");
    user.user_id
}

/// 红线：admin(ACL=[ws_a]) 用 override=ws_b 调 activate_provider 必须被拒，
/// 且 ws_b 的 provider 不被激活（无进程级热切换副作用）。
#[tokio::test]
#[ignore]
async fn activate_provider_blocks_cross_tenant_override() {
    let app = TestApp::start().await;
    let ws_a = "ws_a";
    let ws_b = "ws_b";
    let user_id = seed_admin_with_acl(&app, ws_a).await;

    // ws_b 有一条未激活 provider，攻击者想跨租户激活它。
    let coll = app.state.db.llm_provider_configs();
    coll.insert_one(make_provider(ws_b, "victim_provider", false), None)
        .await
        .expect("seed ws_b provider");

    // override workspaceId=ws_b（ACL 外）→ 必须 Err。
    let query: Query<ListQuery> =
        Query(serde_json::from_value(serde_json::json!({ "workspaceId": ws_b })).expect("query"));
    let result = activate_provider(
        State(app.state.clone()),
        Extension(admin_ctx(&user_id, ws_a)),
        Path("victim_provider".to_string()),
        query,
    )
    .await;
    assert!(
        result.is_err(),
        "ACL=[ws_a] 的 admin 用 override=ws_b 激活必须被拒(workspace_not_in_user_acl)"
    );
    assert!(
        format!("{:?}", result.err().unwrap()).contains("workspace_not_in_user_acl"),
        "拒绝错误码必须是 workspace_not_in_user_acl"
    );

    // 副作用断言：ws_b 的 victim_provider 仍未激活。
    let still = coll
        .find_one(doc! { "workspaceId": ws_b, "providerId": "victim_provider" }, None)
        .await
        .expect("find victim")
        .expect("victim exists");
    assert!(
        !still.is_active,
        "越权被拒后 ws_b 的 provider 不应被激活（无热切换副作用）"
    );
}

/// 正向：admin(ACL=[ws_a]) 不带 override（回落 current_workspace=ws_a）调
/// activate_provider 激活自己租户的 provider 应成功。
#[tokio::test]
#[ignore]
async fn activate_provider_allows_own_workspace() {
    let app = TestApp::start().await;
    let ws_a = "ws_a";
    let user_id = seed_admin_with_acl(&app, ws_a).await;

    let coll = app.state.db.llm_provider_configs();
    coll.insert_one(make_provider(ws_a, "mine", false), None)
        .await
        .expect("seed ws_a provider");

    // 不传 workspaceId → 回落 current_workspace=ws_a（∈ ACL）→ 成功。
    let query: Query<ListQuery> =
        Query(serde_json::from_value(serde_json::json!({})).expect("query"));
    let result = activate_provider(
        State(app.state.clone()),
        Extension(admin_ctx(&user_id, ws_a)),
        Path("mine".to_string()),
        query,
    )
    .await;
    assert!(result.is_ok(), "本租户 provider 激活应成功，实际 {result:?}");

    let mine = coll
        .find_one(doc! { "workspaceId": ws_a, "providerId": "mine" }, None)
        .await
        .expect("find mine")
        .expect("mine exists");
    assert!(mine.is_active, "本租户 provider 应被激活");
}

/// 红线：list_providers 用 override=ws_b（ACL 外）必须被拒，不泄漏 ws_b 列表。
#[tokio::test]
#[ignore]
async fn list_providers_blocks_cross_tenant_override() {
    let app = TestApp::start().await;
    let ws_a = "ws_a";
    let ws_b = "ws_b";
    let user_id = seed_admin_with_acl(&app, ws_a).await;

    app.state
        .db
        .llm_provider_configs()
        .insert_one(make_provider(ws_b, "secret_b", true), None)
        .await
        .expect("seed ws_b provider");

    let query: Query<ListQuery> =
        Query(serde_json::from_value(serde_json::json!({ "workspaceId": ws_b })).expect("query"));
    let result = list_providers(
        State(app.state.clone()),
        Extension(admin_ctx(&user_id, ws_a)),
        query,
    )
    .await;
    assert!(
        result.is_err(),
        "list_providers 用 override=ws_b 必须被拒，不能泄漏他租户 provider 列表"
    );
    assert!(
        format!("{:?}", result.err().unwrap()).contains("workspace_not_in_user_acl"),
        "拒绝错误码必须是 workspace_not_in_user_acl"
    );
}

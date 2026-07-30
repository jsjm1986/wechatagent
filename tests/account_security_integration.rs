//! 账号管理红线集成测试:直调 handler 验"MCP 密钥不回显" + "sync 不覆盖既有 key" +
//! "Debug 掩码"。全部 `#[ignore]`,需 Docker testcontainers。
//! CI `integration` job 用 `cargo test --test account_security_integration -- --ignored` 跑。
//!
//! ## 测试形态(沿用 annotation_quality_gate_integration.rs 惯例)
//! 本仓 `TestApp` 是 state-only 工厂(无 HTTP server)。既有集成测试一律**直调 route
//! handler 真函数**:handler 是普通 async fn,参数是 axum extractor,构造好 `.await`。
//! 本文件直调 `list_accounts` 验响应体不含明文 key,而**不是**在测试体内重抄
//! `mcpKeyConfigured` 投影逻辑(那样只测 json! 不测 handler 真行为)。
#![cfg(test)]

mod common;

use axum::extract::{Extension, State};
use mongodb::bson::DateTime;

use crate::common::TestApp;
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::WechatAccount;
use wechatagent::routes::accounts::{list_accounts, update_account_mcp_key};

/// 构造测试 admin auth context(`current_workspace` 决定 handler 可见/可写范围)。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "acct_sec_admin".to_string(),
        username: "acct_sec_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一条带明文 `mcp_api_key` 的账号,seed 进指定 workspace。
fn make_account_with_key(workspace_id: &str, account_id: &str, key: &str) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        alias: account_id.to_string(),
        display_name: account_id.to_string(),
        app_id: Some("wx_app".to_string()),
        wxid: Some("wxid_demo".to_string()),
        nick_name: Some("演示账号".to_string()),
        avatar_url: None,
        mcp_base_url: Some("http://mcp.example".to_string()),
        mcp_api_key: Some(key.to_string()),
        webhook_secret: None,
        online: true,
        status: None,
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

/// 红线:list_accounts 响应体绝不能含 mcp_api_key 明文,只暴露 mcpKeyConfigured 布尔。
#[tokio::test]
#[ignore]
async fn list_accounts_never_returns_mcp_key_plaintext() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    app.state
        .db
        .accounts()
        .insert_one(make_account_with_key(&ws, "acct_a", "SECRET_KEY_123"), None)
        .await
        .expect("seed account 失败");

    let resp = list_accounts(State(app.state.clone()), Extension(test_admin(&ws)))
        .await
        .expect("list_accounts 应成功");

    let body = serde_json::to_string(&resp.0).expect("序列化响应失败");
    app.cleanup().await;
    assert!(
        !body.contains("SECRET_KEY_123"),
        "响应体绝不能含 mcp_api_key 明文(红线): {body}"
    );
    assert!(
        body.contains("mcpKeyConfigured"),
        "应暴露 mcpKeyConfigured 布尔: {body}"
    );
}

/// 红线:WechatAccount 的手写 Debug 把 mcp_api_key 掩码,防 tracing/panic backtrace 泄漏。
#[tokio::test]
#[ignore]
async fn account_debug_masks_mcp_key() {
    let acct = make_account_with_key("ws", "acct_a", "SECRET_KEY_123");
    let dbg = format!("{acct:?}");
    assert!(
        !dbg.contains("SECRET_KEY_123"),
        "Debug 输出绝不能含明文 key(红线): {dbg}"
    );
}

/// 红线:已配置 key 的账号被 update_account_mcp_key 改写后,跨 workspace 视角查不到/改不到。
/// 直调 handler 让它自己注入 current_workspace 构 filter(非测试体重拼 filter)。
#[tokio::test]
#[ignore]
async fn update_mcp_key_blocks_cross_workspace() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acct = make_account_with_key(&ws, "acct_a", "OLD_KEY");
    let inserted = app
        .state
        .db
        .accounts()
        .insert_one(acct, None)
        .await
        .expect("seed account 失败");
    let id_hex = inserted
        .inserted_id
        .as_object_id()
        .expect("inserted id")
        .to_hex();

    // 另一个 workspace 视角直调 → handler 注入 current_workspace=other_ws 致 matched_count==0 → NotFound
    let result = update_account_mcp_key(
        State(app.state.clone()),
        axum::extract::Path(id_hex),
        Extension(test_admin("other_workspace")),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "expectedAccountId": "acct_a",
                "mcpApiKey": "NEW_KEY"
            }))
            .expect("构造请求体失败"),
        ),
    )
    .await;

    app.cleanup().await;
    assert!(
        result.is_err(),
        "跨 workspace update 必须 NotFound(handler 注入 current_workspace 而非测试自拼 filter)"
    );
}

/// 同一 workspace 内，URL ObjectId 与正文冻结账号不一致也必须零写拒绝。
#[tokio::test]
#[ignore]
async fn update_mcp_key_blocks_same_workspace_account_mismatch_without_writing() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let inserted = app
        .state
        .db
        .accounts()
        .insert_one(make_account_with_key(&ws, "acct_a", "OLD_KEY"), None)
        .await
        .expect("seed account");
    let id = inserted.inserted_id.as_object_id().expect("account id");

    let result = update_account_mcp_key(
        State(app.state.clone()),
        axum::extract::Path(id.to_hex()),
        Extension(test_admin(&ws)),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "expectedAccountId": "acct_b",
                "mcpApiKey": "STOLEN_KEY"
            }))
            .expect("request"),
        ),
    )
    .await;
    let stored = app
        .state
        .db
        .accounts()
        .find_one(mongodb::bson::doc! { "_id": id }, None)
        .await
        .expect("read account")
        .expect("account exists");
    app.cleanup().await;

    assert!(matches!(
        result,
        Err(wechatagent::error::AppError::Conflict(_))
    ));
    assert_eq!(stored.mcp_api_key.as_deref(), Some("OLD_KEY"));
}

/// Webhook routing treats app_id as a global identity. Two account rows must
/// never be able to claim the same app_id, even across workspaces.
#[tokio::test]
#[ignore]
async fn duplicate_app_id_is_rejected_by_database_constraint() {
    let app = TestApp::start().await;
    let first = make_account_with_key("ws_a", "acct_a", "KEY_A");
    let mut second = make_account_with_key("ws_b", "acct_b", "KEY_B");
    second.app_id = first.app_id.clone();

    app.state
        .db
        .accounts()
        .insert_one(first, None)
        .await
        .expect("first app_id owner should insert");
    let duplicate = app.state.db.accounts().insert_one(second, None).await;

    app.cleanup().await;
    assert!(
        duplicate.is_err(),
        "duplicate app_id must be rejected instead of routing nondeterministically"
    );
}

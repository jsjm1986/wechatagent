//! Task 6 集成测试：POST /api/contacts/batch-enable 批量托管 + 异步画像入队。
//! 覆盖：sharedNote 空 → 400；account 未注册 → 400；正常批量 upsert+managed+入队；
//! 幂等（已 managed 不重复入队 initial_profile 任务）。
//! `#[ignore]` 需 Docker；CI:`cargo test --test contacts_batch_enable -- --ignored`。
#![cfg(test)]

mod common;

use axum::extract::{Extension, State};
use mongodb::bson::{doc, DateTime, oid::ObjectId};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::WechatAccount;
use wechatagent::routes::contacts::batch_enable_endpoint;

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "op_admin".to_string(),
        username: "op_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

async fn seed_account(app: &TestApp, ws: &str, account_id: &str) {
    let now = DateTime::now();
    app.state
        .db
        .accounts()
        .insert_one(
            WechatAccount {
                id: Some(ObjectId::new()),
                workspace_id: ws.to_string(),
                account_id: account_id.to_string(),
                alias: "batch_test".to_string(),
                display_name: "批量托管测试账号".to_string(),
                app_id: Some("wx_app_batch".to_string()),
                wxid: Some("wxid_account_self".to_string()),
                nick_name: None,
                avatar_url: None,
                mcp_base_url: None,
                mcp_api_key: None,
                online: true,
                status: None,
                last_sync_at: now,
                capacity: 0,
                persona_tag: None,
                off_hours: Vec::new(),
                created_at: now,
                updated_at: now,
            },
            None,
        )
        .await
        .expect("seed account");
}

fn req(account_id: &str, wxids: &[&str], shared_note: &str) -> wechatagent::models::BatchEnableRequest {
    let candidates: Vec<_> = wxids
        .iter()
        .map(|w| {
            serde_json::json!({
                "wxid": w,
                "nickname": format!("好友{w}"),
                "avatarUrl": format!("http://img/{w}"),
            })
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "accountId": account_id,
        "candidates": candidates,
        "sharedNote": shared_note,
    }))
    .expect("构造 BatchEnableRequest")
}

#[tokio::test]
#[ignore]
async fn empty_shared_note_rejected() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    seed_account(&app, &ws, &acc).await;

    let err = batch_enable_endpoint(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        axum::Json(req(&acc, &["wx_a"], "   ")),
    )
    .await
    .expect_err("空 sharedNote 应 400");
    assert!(matches!(err, AppError::BadRequest(_)), "应为 BadRequest");
}

#[tokio::test]
#[ignore]
async fn unregistered_account_rejected() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    // 不 seed account → 未注册。
    let err = batch_enable_endpoint(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        axum::Json(req("acc_not_registered", &["wx_a"], "统一运营备注")),
    )
    .await
    .expect_err("未注册 account 应 400");
    assert!(matches!(err, AppError::BadRequest(_)), "应为 BadRequest");
}

#[tokio::test]
#[ignore]
async fn batch_enables_and_queues_initial_profile_tasks() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    seed_account(&app, &ws, &acc).await;

    let out = batch_enable_endpoint(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        axum::Json(req(&acc, &["wx_b1", "wx_b2"], "统一运营备注：热情专业")),
    )
    .await
    .expect("批量托管应成功")
    .0;
    assert_eq!(out["enabled"], 2, "两个候选都应 enabled");
    assert_eq!(out["queued"], 2, "两个新客户都应入队 initial_profile");

    // contacts 集合出现 2 条 managed。
    let managed = app
        .state
        .db
        .contacts()
        .count_documents(
            doc! { "workspace_id": &ws, "account_id": &acc, "agent_status": "managed" },
            None,
        )
        .await
        .expect("count managed");
    assert_eq!(managed, 2, "两个联系人应为 managed");

    // sharedNote 落到 human_profile_note + avatar_url 落库。
    let c = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_b1" }, None)
        .await
        .expect("query")
        .expect("contact exists");
    assert_eq!(c.human_profile_note.as_deref(), Some("统一运营备注：热情专业"));
    assert_eq!(c.avatar_url.as_deref(), Some("http://img/wx_b1"));

    // tasks 集合出现 2 条 kind=initial_profile status=pending。
    let tasks = app
        .state
        .db
        .tasks()
        .count_documents(
            doc! { "workspace_id": &ws, "kind": "initial_profile", "status": "pending" },
            None,
        )
        .await
        .expect("count tasks");
    assert_eq!(tasks, 2, "应入队 2 条 initial_profile 任务");
}

#[tokio::test]
#[ignore]
async fn idempotent_does_not_requeue_already_managed() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    seed_account(&app, &ws, &acc).await;

    // 第一次批量：入队 1 条。
    let first = batch_enable_endpoint(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        axum::Json(req(&acc, &["wx_c"], "备注一")),
    )
    .await
    .expect("首次批量")
    .0;
    assert_eq!(first["queued"], 1);

    // 第二次批量同一 wxid：enabled 计数但 queued 不增（已 managed 不重复入队）。
    let second = batch_enable_endpoint(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        axum::Json(req(&acc, &["wx_c"], "备注二")),
    )
    .await
    .expect("再次批量")
    .0;
    assert_eq!(second["enabled"], 1, "仍计入 enabled（刷新 note/avatar）");
    assert_eq!(second["queued"], 0, "已 managed 不应重复入队");

    // 总 initial_profile 任务数仍为 1。
    let tasks = app
        .state
        .db
        .tasks()
        .count_documents(
            doc! { "workspace_id": &ws, "kind": "initial_profile" },
            None,
        )
        .await
        .expect("count tasks");
    assert_eq!(tasks, 1, "幂等：只有首次入队的 1 条任务");
}

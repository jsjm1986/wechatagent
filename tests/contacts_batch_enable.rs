//! Task 6 集成测试：POST /api/contacts/batch-enable 批量托管 + 异步画像入队。
//! 覆盖：sharedNote 空 → 400；account 未注册 → 400；正常批量 upsert+managed+入队；
//! 幂等（已 managed 不重复入队 initial_profile 任务）。
//! `#[ignore]` 需 Docker；CI:`cargo test --test contacts_batch_enable -- --ignored`。
#![cfg(test)]

mod common;

use axum::extract::{Extension, State};
use mongodb::bson::{doc, Document, DateTime, oid::ObjectId};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::{AgentStatus, Contact, WechatAccount};
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
                webhook_secret: None,
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
                "sex": 1,
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
    assert_eq!(c.sex, Some(1), "候选带的 sex 应落库到 Contact.sex");
    // 竞态修复：全新客户在 batch upsert 阶段即同步拿到状态机 initial 态（不等异步画像回填），
    // 这样即使客户在 initial_profile 任务前来消息、gateway 把 last_agent_run_at 推成非空、
    // 回填被 is_previously_operated 跳过，initial 态也不丢。DEFAULT 销售 profile initial=new_contact。
    assert_eq!(
        c.operation_state.as_deref(),
        Some("new_contact"),
        "全新客户批量托管应同步落状态机 initial 态"
    );
    assert_eq!(c.operation_state_confidence, Some(6));

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

/// 竞态修复回归：老客户（曾被 Agent 运营过，last_agent_run_at 非空）批量托管时，
/// 不得用状态机 initial 态覆盖其已积累的 operation_state；全新客户则同步拿到 initial 态。
/// 锁定 batch_enable_endpoint 的 is_new_contact = existing.map(!is_previously_operated) 判定。
#[tokio::test]
#[ignore]
async fn batch_preserves_previously_operated_state_but_seeds_new() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    seed_account(&app, &ws, &acc).await;

    // 预置一个「曾被运营过」的老客户：last_agent_run_at 非空 + 一个非初始 operation_state。
    let now = DateTime::now();
    let veteran = Contact {
        id: None,
        workspace_id: ws.clone(),
        account_id: acc.clone(),
        wxid: "wx_veteran".to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Normal,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: Some("deal_won".to_string()),
        operation_state_reason: None,
        operation_state_confidence: Some(9),
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        // 关键：模拟客户已在 batch 后、回填前来过消息（gateway 推进过 last_agent_run_at）。
        last_agent_run_at: Some(now),
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: now,
        updated_at: now,
    };
    app.state
        .db
        .contacts()
        .insert_one(veteran, None)
        .await
        .expect("seed veteran");

    let out = batch_enable_endpoint(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        axum::Json(req(&acc, &["wx_veteran", "wx_fresh"], "统一运营备注")),
    )
    .await
    .expect("批量托管应成功")
    .0;
    assert_eq!(out["enabled"], 2);

    // 老客户：operation_state 保持 deal_won，不被 initial 覆盖。
    let vet = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_veteran" }, None)
        .await
        .expect("query")
        .expect("veteran exists");
    assert_eq!(
        vet.operation_state.as_deref(),
        Some("deal_won"),
        "老客户已积累的 operation_state 不得被批量托管覆盖"
    );
    assert_eq!(vet.operation_state_confidence, Some(9), "老客户 confidence 不被覆盖");

    // 全新客户：同步拿到状态机 initial 态。
    let fresh = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_fresh" }, None)
        .await
        .expect("query")
        .expect("fresh exists");
    assert_eq!(
        fresh.operation_state.as_deref(),
        Some("new_contact"),
        "全新客户应同步落 initial 态"
    );
}

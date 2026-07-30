//! M13 红线集成测试:update_operation_profile 不得清空 AI 积累的 profile_attributes。
//! 前端 saveOperationProfile 只发 relationshipType/lastCommitment/followUpPolicy,
//! 不带 profileAttributes → payload #[serde(default)] 空 Document。旧 bug 无条件
//! $set → 清空 AI 画像。修复后非空才写(镜像 gateway.rs:4034)。
//! `#[ignore]` 需 Docker;CI:`cargo test --test contact_operation_profile_integration -- --ignored`。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use mongodb::bson::{doc, DateTime, Document};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::{AgentStatus, Contact};
use wechatagent::routes::contacts::update_operation_profile;

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "op_admin".to_string(),
        username: "op_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

fn managed_contact(ws: &str, acc: &str, wxid: &str, profile_attributes: Document) -> Contact {
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
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
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes,
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

async fn seed(app: &TestApp, c: Contact) -> String {
    app.state
        .db
        .contacts()
        .insert_one(c, None)
        .await
        .expect("seed contact")
        .inserted_id
        .as_object_id()
        .expect("oid")
        .to_hex()
}

async fn reload(app: &TestApp, wxid: &str) -> Contact {
    app.state
        .db
        .contacts()
        .find_one(doc! { "wxid": wxid }, None)
        .await
        .expect("query contact")
        .expect("contact exists")
}

/// M13 核心红线:前端式请求(不带 profileAttributes)不清空 AI 积累的 profile_attributes。
#[tokio::test]
#[ignore]
async fn front_end_style_request_preserves_profile_attributes() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let ai_attrs = doc! { "budget": "high", "decision_role": "owner" };
    let id = seed(
        &app,
        managed_contact(&ws, &acc, "wx_m13_a", ai_attrs.clone()),
    )
    .await;

    update_operation_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "expectedAccountId": acc,
                "relationshipType": "customer",
                "lastCommitment": "下周回复",
            }))
            .expect("构造前端式请求体(不带 profileAttributes)"),
        ),
    )
    .await
    .expect("update_operation_profile 应成功")
    .0;

    let c = reload(&app, "wx_m13_a").await;
    assert_eq!(
        c.profile_attributes, ai_attrs,
        "前端式请求不带 profileAttributes 时,AI 积累的 profile_attributes 必须原样保留(旧 bug 清空)"
    );
}

/// 对照:带非空 profileAttributes 时正常写入(证明守卫不误伤真实写)。
#[tokio::test]
#[ignore]
async fn non_empty_profile_attributes_is_written() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let id = seed(
        &app,
        managed_contact(&ws, &acc, "wx_m13_b", Document::new()),
    )
    .await;

    update_operation_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "expectedAccountId": acc,
                "profileAttributes": { "budget": "low" },
            }))
            .expect("构造带 profileAttributes 的请求体"),
        ),
    )
    .await
    .expect("update_operation_profile 应成功")
    .0;

    let c = reload(&app, "wx_m13_b").await;
    assert_eq!(
        c.profile_attributes,
        doc! { "budget": "low" },
        "带非空 profileAttributes 时应正常写入"
    );
}

/// 不回归:set_doc 里的其它字段(follow_up_policy)被更新的同时保留 profile_attributes。
/// follow_up_policy 与 profile_attributes 同在 set_doc 里,钉死「写别的键不误清画像」。
#[tokio::test]
#[ignore]
async fn updating_follow_up_policy_preserves_profile_attributes() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let ai_attrs = doc! { "budget": "mid" };
    let id = seed(
        &app,
        managed_contact(&ws, &acc, "wx_m13_c", ai_attrs.clone()),
    )
    .await;

    update_operation_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "expectedAccountId": acc,
                "followUpPolicy": "每周跟进"
            }))
            .expect("构造带 followUpPolicy 的请求体"),
        ),
    )
    .await
    .expect("update_operation_profile 应成功")
    .0;

    let c = reload(&app, "wx_m13_c").await;
    assert_eq!(
        c.follow_up_policy.as_deref(),
        Some("每周跟进"),
        "follow_up_policy 应被更新"
    );
    assert_eq!(
        c.profile_attributes, ai_attrs,
        "更新 follow_up_policy 不应清空 profile_attributes"
    );
}

/// SR-151: a contact must never adopt a Playbook owned by another account.
#[tokio::test]
#[ignore]
async fn cross_account_playbook_is_rejected_with_zero_contact_write() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let other_acc = "other-account";
    let id = seed(
        &app,
        managed_contact(&ws, &acc, "wx_sr151_cross", doc! { "stable": true }),
    )
    .await;

    let mut foreign_playbook = wechatagent::prompts::default_playbook(&ws, other_acc);
    foreign_playbook.name = "foreign playbook".to_string();
    foreign_playbook.is_default = false;
    let foreign_id = app
        .state
        .db
        .operation_playbooks()
        .insert_one(foreign_playbook, None)
        .await
        .expect("seed foreign playbook")
        .inserted_id
        .as_object_id()
        .expect("foreign playbook id");
    let contacts = app.state.db.contacts().clone_with_type::<Document>();
    let object_id = mongodb::bson::oid::ObjectId::parse_str(&id).expect("contact id");
    let before = contacts
        .find_one(doc! { "_id": object_id }, None)
        .await
        .expect("load before")
        .expect("contact exists");

    let result = update_operation_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "expectedAccountId": acc,
                "playbookId": foreign_id.to_hex(),
                "followUpPolicy": "must not persist"
            }))
            .expect("construct cross-account request"),
        ),
    )
    .await;

    assert!(matches!(result, Err(AppError::NotFound(_))));
    let after = contacts
        .find_one(doc! { "_id": object_id }, None)
        .await
        .expect("load after")
        .expect("contact exists");
    assert_eq!(
        after, before,
        "foreign Playbook rejection must be zero-write"
    );
}

/// SR-070: an AI-generated draft must not be bindable to a live contact.
#[tokio::test]
#[ignore]
async fn draft_playbook_is_rejected_with_zero_contact_write() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let id = seed(
        &app,
        managed_contact(&ws, &acc, "wx_sr070_draft", doc! { "stable": true }),
    )
    .await;

    let mut draft = wechatagent::prompts::default_playbook(&ws, &acc);
    draft.name = "AI draft playbook".to_string();
    draft.created_by = "agent_optimized".to_string();
    draft.release_status = "draft".to_string();
    draft.is_default = false;
    let draft_id = app
        .state
        .db
        .operation_playbooks()
        .insert_one(draft, None)
        .await
        .expect("seed draft playbook")
        .inserted_id
        .as_object_id()
        .expect("draft playbook id");
    let contacts = app.state.db.contacts().clone_with_type::<Document>();
    let object_id = mongodb::bson::oid::ObjectId::parse_str(&id).expect("contact id");
    let before = contacts
        .find_one(doc! { "_id": object_id }, None)
        .await
        .expect("load before")
        .expect("contact exists");

    let result = update_operation_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "expectedAccountId": acc,
                "playbookId": draft_id.to_hex(),
                "followUpPolicy": "must not persist"
            }))
            .expect("construct draft binding request"),
        ),
    )
    .await;

    assert!(matches!(result, Err(AppError::NotFound(_))));
    let after = contacts
        .find_one(doc! { "_id": object_id }, None)
        .await
        .expect("load after")
        .expect("contact exists");
    assert_eq!(after, before, "draft Playbook rejection must be zero-write");
}

//! 联系人 manual_tags 红线集成测试:运营权威标签的 normalize/validate 契约 +
//! handler 落库 + 跨 workspace 隔离。
//! 纯函数测试(normalize/validate)无需 Docker,本地可跑;handler 测试 `#[ignore]` 需 Docker。
//! CI:`cargo test --test contact_manual_tags_integration -- --ignored`(含纯函数则全跑)。
//!
//! ## 红线意义(P0):manual_tags 是运营权威层,"AI 永不覆盖本字段"(contacts.rs:687)。
//! gateway 写回只动 bayesian_signals 单字段(gateway.rs:4065),与 manual_tags 物理隔离。
//! 本测试钉死:①normalize 去空白/去重保序 ②validate 条数/字符上限 400(防 prompt 膨胀)
//! ③handler 真落库 manual_tags ④跨 workspace 改不到(NotFound)。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use mongodb::bson::{DateTime, Document};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{AgentStatus, Contact};
use wechatagent::routes::contacts::{
    normalize_manual_tags, update_manual_tags, validate_manual_tags, MANUAL_TAGS_MAX_COUNT,
    MANUAL_TAG_MAX_CHARS,
};

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "tag_admin".to_string(),
        username: "tag_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

fn managed_contact(ws: &str, acc: &str, wxid: &str) -> Contact {
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
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
        profile_attributes: Document::new(),
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

// ── 纯函数契约(无需 Docker)───────────────────────────────────────────────

#[test]
fn normalize_trims_dedups_preserves_order() {
    let raw = vec![
        "  VIP  ".to_string(),
        "".to_string(),
        "高意向".to_string(),
        "VIP".to_string(), // 去重
        "   ".to_string(), // 纯空白→去掉
    ];
    let out = normalize_manual_tags(&raw);
    assert_eq!(out, vec!["VIP".to_string(), "高意向".to_string()], "去空白+去空串+去重保序");
}

#[test]
fn validate_rejects_too_many_tags() {
    let many: Vec<String> = (0..(MANUAL_TAGS_MAX_COUNT + 1))
        .map(|i| format!("tag{i}"))
        .collect();
    assert!(validate_manual_tags(&many).is_err(), "超条数上限必须 400");
    let ok: Vec<String> = (0..MANUAL_TAGS_MAX_COUNT).map(|i| format!("tag{i}")).collect();
    assert!(validate_manual_tags(&ok).is_ok(), "恰好上限应放行");
}

#[test]
fn validate_rejects_overlong_tag() {
    let long = "x".repeat(MANUAL_TAG_MAX_CHARS + 1);
    assert!(validate_manual_tags(&[long]).is_err(), "超字符上限必须 400");
    let edge = "y".repeat(MANUAL_TAG_MAX_CHARS);
    assert!(validate_manual_tags(&[edge]).is_ok(), "恰好上限应放行");
}

// ── handler 落库 + 跨 workspace 隔离(需 Docker)──────────────────────────

/// handler 真落库 manual_tags(运营权威写入)。
#[tokio::test]
#[ignore]
async fn update_manual_tags_persists() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let inserted = app
        .state
        .db
        .contacts()
        .insert_one(managed_contact(&ws, &acc, "wx_tag"), None)
        .await
        .expect("seed contact");
    let id_hex = inserted.inserted_id.as_object_id().expect("oid").to_hex();

    update_manual_tags(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id_hex.clone()),
        axum::Json(
            serde_json::from_value(serde_json::json!({ "tags": ["VIP", "高意向"] }))
                .expect("构造请求体"),
        ),
    )
    .await
    .expect("update_manual_tags 应成功")
    .0; // 取出 Json 内层避免 unused_must_use(CI -D warnings)

    let contact = app
        .state
        .db
        .contacts()
        .find_one(mongodb::bson::doc! { "wxid": "wx_tag" }, None)
        .await
        .expect("查 contact")
        .expect("contact 存在");
    assert_eq!(
        contact.manual_tags,
        vec!["VIP".to_string(), "高意向".to_string()],
        "manual_tags 应被运营写入落库"
    );
}

/// 红线:跨 workspace update_manual_tags → NotFound(handler 注入 current_workspace)。
#[tokio::test]
#[ignore]
async fn update_manual_tags_cross_workspace_not_found() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let inserted = app
        .state
        .db
        .contacts()
        .insert_one(managed_contact(&ws, &acc, "wx_tag2"), None)
        .await
        .expect("seed contact");
    let id_hex = inserted.inserted_id.as_object_id().expect("oid").to_hex();

    let result = update_manual_tags(
        State(app.state.clone()),
        Extension(test_admin("other_workspace")),
        Path(id_hex),
        axum::Json(
            serde_json::from_value(serde_json::json!({ "tags": ["X"] })).expect("构造请求体"),
        ),
    )
    .await;
    assert!(
        result.is_err(),
        "跨 workspace 改 manual_tags 必须 NotFound"
    );
}

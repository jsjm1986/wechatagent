#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use axum::Json;
use mongodb::bson::{doc, DateTime, Document};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::{AgentStatus, Contact};
use wechatagent::routes::contacts::{add_deal_event, DealEventRequest};

use crate::common::TestApp;

fn admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "deal_scope_admin".into(),
        username: "deal_scope_admin".into(),
        current_workspace: workspace_id.into(),
    }
}

fn contact(workspace_id: &str, account_id: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: None,
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        wxid: "wxid_deal_scope".into(),
        nickname: Some("Deal scope contact".into()),
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
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn wrong_account_deal_event_is_conflict_with_zero_outcome_and_audit_writes() {
    let app = TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let inserted = app
        .state
        .db
        .contacts()
        .insert_one(contact(&workspace, "account-a"), None)
        .await
        .expect("seed contact");
    let contact_id = inserted.inserted_id.as_object_id().expect("contact id");

    let result = add_deal_event(
        State(app.state.clone()),
        Extension(admin(&workspace)),
        Path(contact_id.to_hex()),
        Json(
            serde_json::from_value::<DealEventRequest>(serde_json::json!({
                "expectedAccountId": "account-b",
                "eventKind": "deal",
                "verification": "staff_confirmed",
                "amount": 19900,
                "currency": "CNY"
            }))
            .expect("request"),
        ),
    )
    .await;

    let stored = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("read contact")
        .expect("contact exists");
    let audit_count = app
        .state
        .db
        .events()
        .count_documents(
            doc! {
                "workspace_id": &workspace,
                "kind": "outcome_event_marked",
                "contact_wxid": "wxid_deal_scope",
            },
            None,
        )
        .await
        .expect("count audit events");
    app.cleanup().await;

    assert!(matches!(result, Err(AppError::Conflict(_))));
    assert!(stored.outcome_events.is_empty());
    assert_eq!(audit_count, 0);
}

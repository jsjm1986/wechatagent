//! Quiet-hours scheduling integration contract.
//!
//! Quiet hours and normal debounce must share one durable `inbound_reply`
//! obligation. The legacy `deferred_inbound_reply` task must not be produced.

mod common;

use mongodb::bson::{doc, DateTime, Document};
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};
use wechatagent::webhooks::ensure_wake_followup_task;

const REPLY_KIND: &str = "inbound_reply";

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        manual_tags_updated_at: None,
        manual_tags_by: None,
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
        custom_agent_instructions: None,
        operation_mode_override: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        outcome_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn quiet_hours_reuses_single_reply_obligation() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("user_quiet_1");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert seed contact");
    app.state
        .db
        .messages()
        .insert_one(
            ConversationMessage {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                message_id: Some("quiet-inbound-1".to_string()),
                dedupe_key: None,
                direction: MessageDirection::Inbound,
                content: "quiet-hours question".to_string(),
                msg_type: Some("text".to_string()),
                media_ref: None,
                raw: None,
                is_synthetic_relay: false,
                created_at: DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert inbound");

    ensure_wake_followup_task(&app.state, &contact, 8, 8)
        .await
        .expect("first wake schedule");

    let task_filter = doc! { "kind": REPLY_KIND, "contact_wxid": &contact.wxid };
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(task_filter.clone(), None)
            .await
            .expect("count reply obligations"),
        1
    );
    let task = app
        .state
        .db
        .tasks()
        .find_one(task_filter.clone(), None)
        .await
        .expect("query reply obligation")
        .expect("reply obligation should exist");
    assert_eq!(task.status, "pending");
    assert!(task.review_required);
    assert!(task.run_at.timestamp_millis() > DateTime::now().timestamp_millis());
    assert!(task.expires_at.is_none(), "passive replies must not expire");
    assert_eq!(task.gateway_status.as_deref(), Some("quiet_hours_waiting"));
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(
                doc! { "kind": "deferred_inbound_reply", "contact_wxid": &contact.wxid },
                None,
            )
            .await
            .expect("count legacy wake tasks"),
        0
    );

    ensure_wake_followup_task(&app.state, &contact, 8, 8)
        .await
        .expect("second wake schedule");
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(task_filter, None)
            .await
            .expect("count reply obligations after retry"),
        1
    );
}

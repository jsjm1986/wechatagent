//! Durable ask-human intent recovery and fencing tests.
//!
//! These tests use raw decision-review documents because the intent is an
//! internal protocol owned by the production commit/reconciler boundary.

mod common;

use mongodb::bson::{doc, oid::ObjectId, to_bson, to_document, DateTime, Document};
use wechatagent::models::{
    AgentStatus, AskHumanPolicy, Contact, DeciderRef, EscalationRequest,
    ESCALATION_CATEGORY_OUT_OF_SCOPE,
};

fn contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("请示恢复测试客户".to_string()),
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
        operation_state: Some("need_discovery".to_string()),
        operation_state_reason: None,
        operation_state_confidence: Some(8),
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: Some(now),
        last_inbound_at: Some(now),
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

fn policy(deciders: Vec<DeciderRef>, daily_push_cap: Option<u32>) -> AskHumanPolicy {
    AskHumanPolicy {
        decider_chain: deciders,
        escalate_safety_guard: true,
        escalate_unverified_product: true,
        escalate_ai_policy_hold: true,
        escalate_stuck: true,
        dedupe_window_hours: None,
        daily_push_cap,
        quiet_hours: None,
        timeout_hours: None,
        standing_order: None,
        standing_order_after_hours: None,
    }
}

fn decider(wxid: &str) -> DeciderRef {
    DeciderRef {
        wxid: wxid.to_string(),
        display_name: Some("值班负责人".to_string()),
        account_id: Some("default".to_string()),
    }
}

async fn insert_contact(app: &common::TestApp, contact: &Contact) {
    app.state
        .db
        .contacts()
        .insert_one(contact, None)
        .await
        .expect("insert contact");
}

async fn configure_policy(app: &common::TestApp, policy: AskHumanPolicy) {
    let result = app
        .state
        .db
        .operation_domain_configs()
        .update_one(
            doc! {
                "workspace_id": "default",
                "domain": "user_operations",
                "current_version": true,
            },
            doc! { "$set": { "ask_human_policy": to_bson(&policy).expect("serialize policy") } },
            None,
        )
        .await
        .expect("configure ask-human policy");
    assert_eq!(result.matched_count, 1, "default operation config exists");
}

async fn insert_intent_review(app: &common::TestApp, contact: &Contact) -> ObjectId {
    let review_id = ObjectId::new();
    let now = DateTime::now();
    let request = EscalationRequest {
        needed: true,
        category: Some(ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string()),
        reason: Some("当前安排需要负责人确认".to_string()),
        question_for_principal: Some("是否按当前安排继续跟进？".to_string()),
        self_serviceable_part: None,
        is_generalizable: false,
    };
    app.state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .insert_one(
            doc! {
                "_id": review_id,
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "status": "outbox_enqueued",
                "created_at": now,
                "principal_escalation_intent": {
                    "protocol_version": 1i32,
                    "status": "pending",
                    "source": "explicit_model_request",
                    "request": to_document(&request).expect("serialize request"),
                    "attempts": 0i64,
                    "claim_generation": 0i64,
                    "next_retry_at": now,
                    "created_at": now,
                    "updated_at": now,
                },
            },
            None,
        )
        .await
        .expect("insert decision review intent");
    review_id
}

async fn intent(app: &common::TestApp, review_id: ObjectId) -> Document {
    app.state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": review_id }, None)
        .await
        .expect("query review")
        .expect("review present")
        .get_document("principal_escalation_intent")
        .expect("intent present")
        .clone()
}

async fn make_intent_due(app: &common::TestApp, review_id: ObjectId) {
    app.state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id },
            doc! { "$set": {
                "principal_escalation_intent.status": "retry",
                "principal_escalation_intent.next_retry_at": DateTime::from_millis(0),
            }, "$unset": {
                "principal_escalation_intent.claim_token": "",
                "principal_escalation_intent.claimed_at": "",
            } },
            None,
        )
        .await
        .expect("make intent due");
}

#[tokio::test]
#[ignore]
async fn push_policy_block_keeps_intent_retryable_until_policy_allows() {
    let app = common::TestApp::start().await;
    let customer = contact("intent_push_policy_customer");
    insert_contact(&app, &customer).await;
    configure_policy(
        &app,
        policy(vec![decider("intent_push_policy_principal")], Some(0)),
    )
    .await;
    let review_id = insert_intent_review(&app, &customer).await;

    wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state)
        .await
        .expect("defer blocked intent");
    let deferred = intent(&app, review_id).await;
    assert_eq!(deferred.get_str("status").unwrap_or_default(), "retry");
    assert!(deferred
        .get_str("last_error")
        .unwrap_or_default()
        .contains("push policy"));
    assert_eq!(
        app.state
            .db
            .agent_principal_escalations()
            .count_documents(doc! {}, None)
            .await
            .expect("count escalations"),
        0
    );

    configure_policy(
        &app,
        policy(vec![decider("intent_push_policy_principal")], None),
    )
    .await;
    make_intent_due(&app, review_id).await;
    wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state)
        .await
        .expect("materialize unblocked intent");

    let materialized = intent(&app, review_id).await;
    assert_eq!(
        materialized.get_str("status").unwrap_or_default(),
        "materialized"
    );
    assert_eq!(
        materialized.get_object_id("escalation_id").ok(),
        Some(review_id)
    );
    assert!(
        !materialized.contains_key("last_error"),
        "successful recovery must clear the previous retry error"
    );
    assert!(app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "_id": review_id }, None)
        .await
        .expect("query escalation")
        .is_some());
}

#[tokio::test]
#[ignore]
async fn late_decider_configuration_and_concurrent_workers_converge_once() {
    let app = common::TestApp::start().await;
    let customer = contact("intent_late_config_customer");
    insert_contact(&app, &customer).await;
    configure_policy(&app, policy(Vec::new(), None)).await;
    let review_id = insert_intent_review(&app, &customer).await;

    wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state)
        .await
        .expect("defer missing decider");
    assert_eq!(
        intent(&app, review_id).await.get_str("status").unwrap(),
        "retry"
    );

    configure_policy(
        &app,
        policy(vec![decider("intent_late_config_principal")], None),
    )
    .await;
    make_intent_due(&app, review_id).await;
    let (left, right) = tokio::join!(
        wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state),
        wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state),
    );
    left.expect("left reconciler");
    right.expect("right reconciler");

    assert_eq!(
        app.state
            .db
            .agent_principal_escalations()
            .count_documents(doc! {}, None)
            .await
            .expect("count escalations"),
        1
    );
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "source_kind": "principal_escalation" }, None)
            .await
            .expect("count principal cards"),
        1
    );
    assert_eq!(
        intent(&app, review_id).await.get_str("status").unwrap(),
        "materialized"
    );
}

#[tokio::test]
#[ignore]
async fn replay_after_confirmation_loss_reuses_even_a_resolved_escalation() {
    let app = common::TestApp::start().await;
    let customer = contact("intent_confirmation_loss_customer");
    insert_contact(&app, &customer).await;
    configure_policy(
        &app,
        policy(vec![decider("intent_confirmation_loss_principal")], None),
    )
    .await;
    let review_id = insert_intent_review(&app, &customer).await;
    wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state)
        .await
        .expect("initial materialization");

    app.state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! { "_id": review_id },
            doc! { "$set": { "status": "resolved", "resolved_at": DateTime::now() } },
            None,
        )
        .await
        .expect("simulate already resolved escalation");
    make_intent_due(&app, review_id).await;
    app.state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(
            doc! { "_id": review_id },
            doc! { "$unset": { "principal_escalation_intent.escalation_id": "" } },
            None,
        )
        .await
        .expect("simulate lost materialization acknowledgement");

    wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state)
        .await
        .expect("replay deterministic intent");
    assert_eq!(
        app.state
            .db
            .agent_principal_escalations()
            .count_documents(doc! {}, None)
            .await
            .expect("count escalations"),
        1
    );
    let replayed = intent(&app, review_id).await;
    assert_eq!(replayed.get_str("status").unwrap(), "materialized");
    assert_eq!(
        replayed.get_object_id("escalation_id").ok(),
        Some(review_id)
    );
}

#[tokio::test]
#[ignore]
async fn second_pending_intent_is_terminally_deduplicated() {
    let app = common::TestApp::start().await;
    let customer = contact("intent_dedupe_customer");
    insert_contact(&app, &customer).await;
    configure_policy(&app, policy(vec![decider("intent_dedupe_principal")], None)).await;
    let first_review = insert_intent_review(&app, &customer).await;
    wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state)
        .await
        .expect("materialize first intent");

    let second_review = insert_intent_review(&app, &customer).await;
    wechatagent::agent::escalation::reconcile_principal_escalation_intents(&app.state)
        .await
        .expect("dedupe second intent");
    let second = intent(&app, second_review).await;
    assert_eq!(second.get_str("status").unwrap(), "deduplicated");
    assert_eq!(
        second.get_object_id("escalation_id").ok(),
        Some(first_review)
    );
    assert_eq!(
        app.state
            .db
            .agent_principal_escalations()
            .count_documents(doc! {}, None)
            .await
            .expect("count escalations"),
        1
    );
}

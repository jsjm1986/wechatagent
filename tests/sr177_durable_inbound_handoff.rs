//! SR-177 durable inbound webhook handoff redlines.
//!
//! These tests use a real isolated Mongo database. They cover the crash/concurrency
//! boundaries that an in-process debounce map cannot protect:
//! - an inbound fact survives a crash before task materialization and is reconciled once;
//! - a later inbound refreshes the same task and fences the former claim, even when its ObjectId
//!   sorts before the older message id;
//! - a crash after Outbox insertion but before task authorization is recoverable without a
//!   duplicate remote send, while operator-canceled rows remain terminal.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};

use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{
    atomic_claim_pending, enqueue, process_entry, EnqueueOutcome, EnqueueRequest, OutboxStatus,
};
use wechatagent::models::{
    AgentDecisionReview, AgentStatus, Contact, ConversationMessage, MessageDirection, OperationMode,
};
use wechatagent::tasks::{
    adopt_recoverable_durable_outbox_if_owned, authorize_task_outbox_if_owned,
    bind_task_decision_if_owned, claim_task_by_id, task_claim_is_current,
};
use wechatagent::webhooks::{
    materialize_durable_inbound_task, reconcile_pending_inbound_handoffs,
    DURABLE_INBOUND_REPLY_KIND,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

struct SuccessfulMcp {
    sequence: AtomicU64,
}

impl Respond for SuccessfulMcp {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "structuredContent": {
                    "newMsgId": format!("sr177-message-{sequence}"),
                    "content": []
                }
            }
        }))
    }
}

async fn start_mcp() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(SuccessfulMcp {
            sequence: AtomicU64::new(0),
        })
        .mount(&server)
        .await;
    server
}

fn count_text_send_calls(requests: &[Request]) -> usize {
    requests
        .iter()
        .filter(|request| {
            serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| {
                    (body.get("method").and_then(|value| value.as_str()) == Some("tools/call"))
                        .then(|| {
                            body.pointer("/params/name")
                                .and_then(|value| value.as_str())
                        })
                        .flatten()
                        .map(|name| name == "message_send_text")
                })
                .unwrap_or(false)
        })
        .count()
}

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    let mut operation_mode = OperationMode::default();
    operation_mode.quiet_hours.enabled_override = Some(false);
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("SR-177 contact".to_string()),
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: Some(operation_mode),
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
        operation_state_confidence: Some(7),
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

fn inbound(wxid: &str, id: ObjectId, created_at: DateTime, suffix: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(id),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: wxid.to_string(),
        message_id: Some(format!("sr177-{suffix}")),
        dedupe_key: Some(format!("message:sr177-{suffix}")),
        direction: MessageDirection::Inbound,
        content: format!("SR-177 inbound {suffix}"),
        msg_type: Some("text".to_string()),
        media_ref: None,
        raw: Some(doc! { "source": "sr177-test" }),
        is_synthetic_relay: false,
        created_at,
    }
}

async fn insert_pending_handoff(
    state: &wechatagent::routes::AppState,
    message: &ConversationMessage,
) {
    let mut raw = to_document(message).expect("serialize inbound");
    raw.insert("handoff_status", "pending");
    state
        .db
        .messages()
        .clone_with_type::<Document>()
        .insert_one(raw, None)
        .await
        .expect("insert pending inbound handoff");
}

fn review(decision_id: ObjectId, run_id: &str, wxid: &str) -> AgentDecisionReview {
    AgentDecisionReview {
        id: Some(decision_id),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: Some(wxid.to_string()),
        run_id: Some(run_id.to_string()),
        inbound_message_id: Some("sr177-old".to_string()),
        reply_text: Some("obsolete SR-177 reply".to_string()),
        approved: true,
        scores: Document::new(),
        formula_breakdown: Document::new(),
        risks: Vec::new(),
        rewrite_instruction: None,
        review_summary: Some("SR-177 stale generation fixture".to_string()),
        playbook_id: None,
        playbook_version: None,
        used_knowledge_ids: Vec::new(),
        prompt_versions: Document::new(),
        operation_state: None,
        next_best_action: Document::new(),
        context_pack_snapshot: Document::new(),
        domain_config_snapshot: Document::new(),
        runtime_parameters_snapshot: Document::new(),
        send_gateway_result: doc! { "allowed": true },
        outcome_status: Some("pending".to_string()),
        reaction_analysis: Document::new(),
        reaction_claimed_at: None,
        reaction_claim_token: None,
        reaction_claim_generation: 0,
        source_task_id: None,
        source_task_claim_token: None,
        reviewer_misjudge_signal: None,
        expected_text_segments: 1,
        status: "outbox_enqueuing".to_string(),
        created_at: DateTime::now(),
    }
}

#[tokio::test]
#[ignore]
async fn pending_message_is_reconciled_to_exactly_one_durable_task() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("sr177-recovery");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let message = inbound(&contact.wxid, ObjectId::new(), DateTime::now(), "recovery");
    insert_pending_handoff(&app.state, &message).await;

    assert_eq!(
        reconcile_pending_inbound_handoffs(&app.state)
            .await
            .expect("reconcile pending handoff"),
        1
    );
    assert_eq!(
        reconcile_pending_inbound_handoffs(&app.state)
            .await
            .expect("reconcile is idempotent"),
        0
    );

    let message_id = message.id.expect("message id");
    let stored_message = app
        .state
        .db
        .messages()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": message_id }, None)
        .await
        .expect("read message")
        .expect("stored message");
    assert_eq!(
        stored_message.get_str("handoff_status").unwrap(),
        "materialized"
    );

    let tasks = app.state.db.tasks().clone_with_type::<Document>();
    assert_eq!(
        tasks
            .count_documents(
                doc! {
                    "workspace_id": "default",
                    "account_id": "default",
                    "contact_wxid": &contact.wxid,
                    "kind": DURABLE_INBOUND_REPLY_KIND,
                },
                None,
            )
            .await
            .expect("count durable tasks"),
        1
    );
    let task = tasks
        .find_one(
            doc! {
                "contact_wxid": &contact.wxid,
                "kind": DURABLE_INBOUND_REPLY_KIND,
            },
            None,
        )
        .await
        .expect("read durable task")
        .expect("durable task");
    assert_eq!(task.get_str("status").unwrap(), "pending");
    assert_eq!(task.get_str("content").unwrap(), message_id.to_hex());
    assert_eq!(task.get_str("active_task_key").unwrap(), "inbound_reply");
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn later_message_refreshes_single_flight_and_fences_old_outbox() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("sr177-fence");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");

    let base_ms = DateTime::now().timestamp_millis();
    // Deliberately make the later message's ObjectId lexicographically smaller.
    // Correct ordering must come from created_at, not ObjectId random tails.
    let old = inbound(
        &contact.wxid,
        ObjectId::from_bytes([0xff; 12]),
        DateTime::from_millis(base_ms),
        "old",
    );
    let newer = inbound(
        &contact.wxid,
        ObjectId::from_bytes([0x00; 12]),
        DateTime::from_millis(base_ms + 1_000),
        "new",
    );
    insert_pending_handoff(&app.state, &old).await;
    insert_pending_handoff(&app.state, &newer).await;

    let durable = materialize_durable_inbound_task(&app.state, &contact, &old, 4_000)
        .await
        .expect("materialize old inbound");
    let (_task, old_claim) = claim_task_by_id(&app.state, durable.task_id, Some("default"))
        .await
        .expect("claim old generation")
        .expect("old generation claimed");

    let decision_id = ObjectId::new();
    app.state
        .db
        .decision_reviews()
        .insert_one(review(decision_id, "sr177-old-run", &contact.wxid), None)
        .await
        .expect("insert old decision");
    assert!(
        bind_task_decision_if_owned(&app.state, &old_claim, decision_id)
            .await
            .expect("bind old decision")
    );
    let outbox_id = match enqueue(
        &app.state,
        EnqueueRequest {
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: contact.wxid.clone(),
            run_id: "sr177-old-run".to_string(),
            decision_id: Some(decision_id),
            source_event_id: "sr177-old".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "obsolete SR-177 reply".to_string(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await
    .expect("enqueue stale reply")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    let refreshed = materialize_durable_inbound_task(&app.state, &contact, &newer, 4_000)
        .await
        .expect("refresh with later inbound");
    assert_eq!(refreshed.task_id, durable.task_id);
    assert!(!task_claim_is_current(&app.state, &old_claim)
        .await
        .expect("check stale claim"));
    assert!(
        !authorize_task_outbox_if_owned(&app.state, &old_claim, decision_id)
            .await
            .expect("old claim authorization must fail")
    );

    let task = app
        .state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": durable.task_id }, None)
        .await
        .expect("read refreshed task")
        .expect("refreshed task");
    assert_eq!(task.get_str("status").unwrap(), "pending");
    assert_eq!(
        task.get_str("content").unwrap(),
        newer.id.expect("new message id").to_hex()
    );
    assert_eq!(
        task.get_datetime("latest_inbound_created_at")
            .unwrap()
            .timestamp_millis(),
        base_ms + 1_000
    );

    let claimed_outbox = atomic_claim_pending(&app.state, "sr177-dispatcher", 60)
        .await
        .expect("claim stale outbox")
        .expect("stale outbox available");
    process_entry(&app.state, &claimed_outbox)
        .await
        .expect("dispatcher rejects stale generation");
    let stored_outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("read stale outbox")
        .expect("stored stale outbox");
    assert_eq!(stored_outbox.status, OutboxStatus::Canceled.as_str());
    assert!(stored_outbox
        .cancel_reason
        .as_deref()
        .unwrap_or_default()
        .contains("stale_task_claim"));
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn crash_after_enqueue_is_adopted_once_but_terminal_rows_stay_terminal() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    let now = DateTime::now();
    state
        .db
        .accounts()
        .clone_with_type::<Document>()
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "alias": "sr177-test",
                "display_name": "SR-177 test account",
                "mcp_base_url": mcp.uri(),
                "mcp_api_key": "sr177-test-key",
                "online": true,
                "last_sync_at": now,
                "capacity": 0,
                "off_hours": [],
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("insert online MCP test account");
    let contact = managed_contact("sr177-adopt");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");

    let message = inbound(&contact.wxid, ObjectId::new(), DateTime::now(), "adopt");
    insert_pending_handoff(&state, &message).await;
    let durable = materialize_durable_inbound_task(&state, &contact, &message, 0)
        .await
        .expect("materialize durable task");
    let (_task, old_claim) = claim_task_by_id(&state, durable.task_id, Some("default"))
        .await
        .expect("claim old generation")
        .expect("old generation claimed");

    let old_decision_id = ObjectId::new();
    state
        .db
        .decision_reviews()
        .insert_one(
            review(old_decision_id, "sr177-adopt-old", &contact.wxid),
            None,
        )
        .await
        .expect("insert old review");
    assert!(
        bind_task_decision_if_owned(&state, &old_claim, old_decision_id)
            .await
            .expect("bind old decision")
    );

    let recoverable_outbox_id = match enqueue(
        &state,
        EnqueueRequest {
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: contact.wxid.clone(),
            run_id: "sr177-adopt-old".to_string(),
            decision_id: Some(old_decision_id),
            source_event_id: "sr177-adopt".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "SR-177 recovered reply".to_string(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await
    .expect("enqueue old recoverable reply")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    // These two rows share the old task decision but are deliberately terminal.
    // Recovery must not revive an operator cancellation or any row that crossed
    // the irreversible remote-send boundary.
    let operator_canceled_id = match enqueue(
        &state,
        EnqueueRequest {
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: contact.wxid.clone(),
            run_id: "sr177-adopt-old".to_string(),
            decision_id: Some(old_decision_id),
            source_event_id: "sr177-adopt-operator-canceled".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "must stay canceled".to_string(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await
    .expect("enqueue operator-canceled fixture")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    state
        .db
        .collection_agent_send_outbox()
        .update_one(
            doc! { "_id": operator_canceled_id },
            doc! { "$set": {
                "status": "canceled",
                "cancel_reason": "operator canceled",
                "updated_at": DateTime::now(),
            } },
            None,
        )
        .await
        .expect("mark operator-canceled fixture");

    let send_started_id = match enqueue(
        &state,
        EnqueueRequest {
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: contact.wxid.clone(),
            run_id: "sr177-adopt-old".to_string(),
            decision_id: Some(old_decision_id),
            source_event_id: "sr177-adopt-send-started".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "must stay beyond boundary".to_string(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await
    .expect("enqueue send-started fixture")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    state
        .db
        .collection_agent_send_outbox()
        .update_one(
            doc! { "_id": send_started_id },
            doc! { "$set": {
                "status": "canceled",
                "cancel_reason": "stale_task_claim: old generation",
                "send_started_at": DateTime::now(),
                "updated_at": DateTime::now(),
            } },
            None,
        )
        .await
        .expect("mark send-started fixture");

    // Simulate process death after enqueue but before authorization, followed by
    // lease recovery and a fresh owner/decision for the same durable task.
    state
        .db
        .tasks()
        .update_one(
            old_claim.owned_running_filter(),
            doc! {
                "$set": { "status": "retry", "next_retry_at": DateTime::now() },
                "$unset": { "claim_token": "", "claimed_at": "", "outbox_decision_id": "" },
            },
            None,
        )
        .await
        .expect("recover old task claim");
    let (_task, new_claim) = claim_task_by_id(&state, durable.task_id, Some("default"))
        .await
        .expect("claim recovered generation")
        .expect("new generation claimed");
    let new_decision_id = ObjectId::new();
    state
        .db
        .decision_reviews()
        .insert_one(
            review(new_decision_id, "sr177-adopt-new", &contact.wxid),
            None,
        )
        .await
        .expect("insert new review");
    assert!(
        bind_task_decision_if_owned(&state, &new_claim, new_decision_id)
            .await
            .expect("bind new decision")
    );

    assert!(adopt_recoverable_durable_outbox_if_owned(
        &state,
        &new_claim,
        new_decision_id,
        "sr177-adopt-new",
        recoverable_outbox_id,
        old_decision_id,
    )
    .await
    .expect("adopt recoverable outbox"));
    assert!(!adopt_recoverable_durable_outbox_if_owned(
        &state,
        &new_claim,
        new_decision_id,
        "sr177-adopt-new",
        operator_canceled_id,
        old_decision_id,
    )
    .await
    .expect("operator-canceled row is terminal"));
    assert!(!adopt_recoverable_durable_outbox_if_owned(
        &state,
        &new_claim,
        new_decision_id,
        "sr177-adopt-new",
        send_started_id,
        old_decision_id,
    )
    .await
    .expect("send-started row is terminal"));

    assert!(
        authorize_task_outbox_if_owned(&state, &new_claim, new_decision_id)
            .await
            .expect("authorize adopted outbox")
    );
    state
        .db
        .decision_reviews()
        .update_one(
            doc! { "_id": new_decision_id, "status": "outbox_enqueuing" },
            doc! { "$set": { "status": "outbox_enqueued" } },
            None,
        )
        .await
        .expect("mark new review enqueued");

    let claimed = atomic_claim_pending(&state, "sr177-adopt-dispatcher", 60)
        .await
        .expect("claim adopted outbox")
        .expect("adopted outbox pending");
    assert_eq!(claimed.id, Some(recoverable_outbox_id));
    process_entry(&state, &claimed)
        .await
        .expect("send adopted outbox");

    let adopted = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": recoverable_outbox_id }, None)
        .await
        .expect("read adopted outbox")
        .expect("adopted outbox exists");
    assert_eq!(adopted.status, OutboxStatus::Sent.as_str());
    assert_eq!(adopted.decision_id, Some(new_decision_id));
    assert_eq!(adopted.run_id, "sr177-adopt-new");
    assert_eq!(
        count_text_send_calls(&mcp.received_requests().await.unwrap()),
        1
    );
    assert!(atomic_claim_pending(&state, "sr177-adopt-duplicate", 60)
        .await
        .expect("duplicate claim query")
        .is_none());

    let operator_canceled = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": operator_canceled_id }, None)
        .await
        .expect("read operator-canceled row")
        .expect("operator-canceled row exists");
    assert_eq!(operator_canceled.status, OutboxStatus::Canceled.as_str());
    assert_eq!(operator_canceled.decision_id, Some(old_decision_id));
    let send_started = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": send_started_id }, None)
        .await
        .expect("read send-started row")
        .expect("send-started row exists");
    assert_eq!(send_started.status, OutboxStatus::Canceled.as_str());
    assert_eq!(send_started.decision_id, Some(old_decision_id));
    assert!(send_started.send_started_at.is_some());

    app.cleanup().await;
}

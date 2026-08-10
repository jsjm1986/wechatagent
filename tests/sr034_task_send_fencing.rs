//! SR-034 task claim → outbox send authorization fencing redlines.
//!
//! These tests use real Mongo state transitions plus a WireMock MCP endpoint. They intentionally
//! drive the production claim/bind/authorize/dispatch helpers directly so a stale task owner can
//! never be hidden by a higher-level handler retry.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{
    atomic_claim_pending, cancel_for_decision, enqueue, process_entry, EnqueueOutcome,
    EnqueueRequest, OutboxStatus,
};
use wechatagent::models::{AgentDecisionReview, AgentStatus, AgentTask, Contact};
use wechatagent::tasks::{
    authorize_task_outbox_if_owned, bind_task_decision_if_owned, claim_task_by_id,
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
                    "newMsgId": format!("sr034-message-{sequence}"),
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

fn count_send_calls(requests: &[Request]) -> usize {
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

fn contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("SR-034 contact".to_string()),
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
        locale: None,
        outcome_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

fn pending_task(wxid: &str) -> AgentTask {
    let now = DateTime::now();
    AgentTask {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: wxid.to_string(),
        kind: "follow_up".to_string(),
        run_at: now,
        expires_at: None,
        content: "SR-034 follow-up".to_string(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: true,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    }
}

fn review(decision_id: ObjectId, run_id: &str, wxid: &str) -> AgentDecisionReview {
    AgentDecisionReview {
        id: Some(decision_id),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: Some(wxid.to_string()),
        run_id: Some(run_id.to_string()),
        inbound_message_id: None,
        reply_text: Some("SR-034 authorized message".to_string()),
        approved: true,
        scores: Document::new(),
        formula_breakdown: Document::new(),
        risks: Vec::new(),
        rewrite_instruction: None,
        review_summary: Some("SR-034 fixture".to_string()),
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

fn request(decision_id: ObjectId, run_id: &str, wxid: &str, suffix: &str) -> EnqueueRequest {
    EnqueueRequest {
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: wxid.to_string(),
        run_id: run_id.to_string(),
        decision_id: Some(decision_id),
        source_event_id: format!("sr034-{suffix}"),
        source_kind: "follow_up_task".to_string(),
        content: format!("SR-034 message {suffix}"),
        media_asset_id: None,
        referral_card_id: None,
        max_attempts: 3,
    }
}

async fn seed_bound_task(
    state: &wechatagent::routes::AppState,
    wxid: &str,
    run_id: &str,
) -> (ObjectId, wechatagent::tasks::TaskClaim) {
    let task = pending_task(wxid);
    let task_id = task.id.expect("task id");
    state
        .db
        .tasks()
        .insert_one(task, None)
        .await
        .expect("insert task");
    let (_claimed, claim) = claim_task_by_id(state, task_id, Some("default"))
        .await
        .expect("claim query")
        .expect("claim task");
    let decision_id = ObjectId::new();
    state
        .db
        .decision_reviews()
        .insert_one(review(decision_id, run_id, wxid), None)
        .await
        .expect("insert review");
    assert!(bind_task_decision_if_owned(state, &claim, decision_id)
        .await
        .expect("bind decision"));
    (decision_id, claim)
}

#[tokio::test]
#[ignore]
async fn decision_batch_seal_defers_non_task_row_without_remote_send() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    common::ensure_test_account(&state, "default", "default").await;
    let contact = contact("sr034-batch-seal");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let decision_id = ObjectId::new();
    state
        .db
        .decision_reviews()
        .insert_one(
            review(decision_id, "sr034-batch-seal-run", &contact.wxid),
            None,
        )
        .await
        .expect("insert building review");
    let outbox_id = match enqueue(
        &state,
        request(
            decision_id,
            "sr034-batch-seal-run",
            &contact.wxid,
            "first-segment",
        ),
    )
    .await
    .expect("enqueue first segment")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    let claimed = atomic_claim_pending(&state, "sr034-batch-seal-worker", 60)
        .await
        .expect("claim query")
        .expect("claim first segment");
    process_entry(&state, &claimed)
        .await
        .expect("building decision must defer");

    let stored = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("load outbox")
        .expect("outbox exists");
    assert_eq!(stored.status, OutboxStatus::Pending.as_str());
    assert_eq!(
        stored.attempt, 0,
        "batch construction is not a send failure"
    );
    assert_eq!(
        count_send_calls(&mcp.received_requests().await.expect("wiremock requests")),
        0,
        "no decision-backed segment may cross MCP before the review batch is sealed"
    );
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn building_task_deferred_without_remote_send() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    let contact = contact("sr034-building");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .unwrap();
    let (decision_id, _claim) = seed_bound_task(&state, &contact.wxid, "sr034-building-run").await;
    let outbox_id = match enqueue(
        &state,
        request(decision_id, "sr034-building-run", &contact.wxid, "building"),
    )
    .await
    .unwrap()
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "sr034-building-worker", 60)
        .await
        .unwrap()
        .expect("claim outbox");
    process_entry(&state, &claimed).await.unwrap();

    let stored = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, OutboxStatus::Pending.as_str());
    assert_eq!(stored.attempt, 0);
    assert!(stored.next_retry_at.is_some());
    assert_eq!(count_send_calls(&mcp.received_requests().await.unwrap()), 0);
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn stale_task_claim_cancels_outbox_without_remote_send() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    let contact = contact("sr034-stale");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .unwrap();
    let (decision_id, claim) = seed_bound_task(&state, &contact.wxid, "sr034-stale-run").await;
    let outbox_id = match enqueue(
        &state,
        request(decision_id, "sr034-stale-run", &contact.wxid, "stale"),
    )
    .await
    .unwrap()
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    state
        .db
        .tasks()
        .update_one(
            claim.owned_running_filter(),
            doc! {
                "$set": { "status": "retry", "updated_at": DateTime::now() },
                "$unset": { "claim_token": "", "claimed_at": "", "outbox_decision_id": "" }
            },
            None,
        )
        .await
        .unwrap();
    let _new_owner = claim_task_by_id(&state, claim.task_id, Some("default"))
        .await
        .unwrap()
        .expect("new owner claims task");
    let claimed = atomic_claim_pending(&state, "sr034-stale-worker", 60)
        .await
        .unwrap()
        .expect("claim outbox");
    process_entry(&state, &claimed).await.unwrap();

    let stored = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, OutboxStatus::Canceled.as_str());
    assert!(stored
        .cancel_reason
        .as_deref()
        .unwrap_or_default()
        .contains("stale_task_claim"));
    assert_eq!(count_send_calls(&mcp.received_requests().await.unwrap()), 0);
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn same_claim_authorization_allows_exactly_one_remote_send() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    common::ensure_test_account(&state, "default", "default").await;
    let contact = contact("sr034-authorized");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .unwrap();
    let (decision_id, claim) = seed_bound_task(&state, &contact.wxid, "sr034-authorized-run").await;
    let outbox_id = match enqueue(
        &state,
        request(
            decision_id,
            "sr034-authorized-run",
            &contact.wxid,
            "authorized",
        ),
    )
    .await
    .unwrap()
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    assert!(authorize_task_outbox_if_owned(&state, &claim, decision_id)
        .await
        .unwrap());
    state
        .db
        .decision_reviews()
        .update_one(
            doc! { "_id": decision_id, "status": "outbox_enqueuing" },
            doc! { "$set": { "status": "outbox_enqueued" } },
            None,
        )
        .await
        .unwrap();

    let claimed = atomic_claim_pending(&state, "sr034-authorized-worker", 60)
        .await
        .unwrap()
        .expect("claim outbox");
    process_entry(&state, &claimed).await.unwrap();
    let stored = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, OutboxStatus::Sent.as_str());
    assert_eq!(count_send_calls(&mcp.received_requests().await.unwrap()), 1);
    assert!(atomic_claim_pending(&state, "sr034-duplicate-worker", 60)
        .await
        .unwrap()
        .is_none());
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn decision_cancel_stops_pending_and_in_flight_before_remote_boundary() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    let contact = contact("sr034-cancel");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .unwrap();
    let decision_id = ObjectId::new();
    let pending_id = match enqueue(
        &state,
        request(decision_id, "sr034-cancel-run", &contact.wxid, "pending"),
    )
    .await
    .unwrap()
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let in_flight_id = match enqueue(
        &state,
        request(decision_id, "sr034-cancel-run", &contact.wxid, "in-flight"),
    )
    .await
    .unwrap()
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "sr034-cancel-worker", 60)
        .await
        .unwrap()
        .expect("claim first outbox");
    assert_eq!(claimed.id, Some(pending_id));
    // The first FIFO row is in flight; the second remains pending.
    let accepted = cancel_for_decision(&state, "default", decision_id, "sr034-test-cancel")
        .await
        .unwrap();
    assert_eq!(accepted, 2);
    process_entry(&state, &claimed).await.unwrap();

    let in_flight = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": pending_id }, None)
        .await
        .unwrap()
        .unwrap();
    let pending = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": in_flight_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(in_flight.status, OutboxStatus::Canceled.as_str());
    assert_eq!(pending.status, OutboxStatus::Canceled.as_str());
    assert_eq!(count_send_calls(&mcp.received_requests().await.unwrap()), 0);
    app.cleanup().await;
}

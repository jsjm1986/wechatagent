#![cfg(test)]

mod common;

use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use wechatagent::agent::{
    atomic_claim_pending, enqueue, process_entry, record_user_reaction, EnqueueOutcome,
    EnqueueRequest, OutboxStatus,
};
use wechatagent::error::AppError;
use wechatagent::models::{
    AgentDecisionReview, AgentStatus, Contact, ConversationMessage, MessageDirection, WechatAccount,
};
use wechatagent::webhooks::wechat_webhook;

use crate::common::TestApp;

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", ObjectId::new().to_hex())
}

fn account(
    workspace_id: &str,
    account_id: &str,
    app_id: &str,
    mcp_base_url: Option<String>,
) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        alias: format!("{workspace_id}-{account_id}"),
        display_name: format!("{workspace_id}-{account_id}"),
        app_id: Some(app_id.into()),
        wxid: Some(unique("self")),
        nick_name: None,
        avatar_url: None,
        mcp_base_url,
        mcp_api_key: Some("scope-redline-key".into()),
        webhook_secret: None,
        online: true,
        status: Some("active".into()),
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: vec![],
        created_at: now,
        updated_at: now,
    }
}

fn contact(workspace_id: &str, account_id: &str, wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        wxid: wxid.into(),
        nickname: Some("scope redline contact".into()),
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
        manual_tags: vec![],
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: vec![],
        bayesian_signals: vec![],
        personality_profile: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: vec![],
        follow_up_policy: None,
        operation_state: Some("need_discovery".into()),
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
        intent_trajectory: vec![],
        outcome_events: vec![],
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

struct UniqueMsgIdResponder {
    counter: AtomicU64,
}

impl Respond for UniqueMsgIdResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let sequence = self.counter.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "structuredContent": {
                    "newMsgId": format!("hc004_scope_message_{sequence}"),
                    "content": []
                }
            }
        }))
    }
}

async fn start_mcp_mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(UniqueMsgIdResponder {
            counter: AtomicU64::new(0),
        })
        .mount(&server)
        .await;
    server
}

fn mcp_call_sequence(requests: &[Request]) -> Vec<String> {
    requests
        .iter()
        .filter_map(|request| {
            let body = serde_json::from_slice::<serde_json::Value>(&request.body).ok()?;
            match body.get("method").and_then(|value| value.as_str()) {
                Some("initialize") => Some("initialize".to_string()),
                Some("tools/call") => Some(format!(
                    "tools/call:{}",
                    body.pointer("/params/name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<missing-name>")
                )),
                Some(method) => Some(method.to_string()),
                None => None,
            }
        })
        .collect()
}

fn enqueue_request(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    run_id: &str,
) -> EnqueueRequest {
    EnqueueRequest {
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        contact_wxid: contact_wxid.into(),
        run_id: run_id.into(),
        decision_id: None,
        source_event_id: "shared-source-event".into(),
        source_kind: "inbound_message".into(),
        content: "same scoped outbound content".into(),
        media_asset_id: None,
        referral_card_id: None,
        max_attempts: 3,
    }
}

fn sent_review(workspace_id: &str, account_id: &str, wxid: &str) -> AgentDecisionReview {
    AgentDecisionReview {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        contact_wxid: Some(wxid.into()),
        run_id: Some(unique("reaction-run")),
        inbound_message_id: None,
        reply_text: None,
        approved: true,
        scores: Document::new(),
        formula_breakdown: Document::new(),
        risks: vec![],
        rewrite_instruction: None,
        review_summary: None,
        playbook_id: None,
        playbook_version: None,
        used_knowledge_ids: vec![],
        prompt_versions: Document::new(),
        operation_state: None,
        next_best_action: Document::new(),
        context_pack_snapshot: Document::new(),
        domain_config_snapshot: Document::new(),
        runtime_parameters_snapshot: Document::new(),
        send_gateway_result: Document::new(),
        outcome_status: Some("pending".into()),
        reaction_analysis: Document::new(),
        reaction_claimed_at: None,
        reaction_claim_token: None,
        reaction_claim_generation: 0,
        source_task_id: None,
        source_task_claim_token: None,
        reviewer_misjudge_signal: None,
        expected_text_segments: 0,
        status: "sent".into(),
        created_at: DateTime::now(),
    }
}

fn inbound(workspace_id: &str, account_id: &str, wxid: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        contact_wxid: wxid.into(),
        message_id: Some(unique("reaction-message")),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "stop contacting me".into(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    }
}

fn webhook_body(app_id: &str, message_id: &str) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "appId": app_id,
            "fromWxid": "gh_hc004_scope_redline",
            "content": "scope redline webhook",
            "msgId": message_id
        }))
        .expect("serialize webhook body"),
    )
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn sr024_webhook_rate_limit_is_workspace_account_scoped() {
    let app = TestApp::start().await;
    let workspace_a = unique("sr024-ws-a");
    let workspace_b = unique("sr024-ws-b");
    let account_id = unique("sr024-shared-account");
    let app_id_a = unique("sr024-app-a");
    let app_id_b = unique("sr024-app-b");
    let account_a = account(&workspace_a, &account_id, &app_id_a, None);
    let account_b = account(&workspace_b, &account_id, &app_id_b, None);
    let account_b_id = account_b.id.expect("workspace B account id");
    app.state
        .db
        .accounts()
        .insert_many([account_a, account_b], None)
        .await
        .expect("seed same account id in two workspaces");

    let mut state_a = app.state.clone();
    state_a.config.webhook_verify_signature = false;
    state_a.config.webhook_rate_limit_capacity = 1;
    state_a.config.webhook_rate_limit_window_seconds = 600;
    // Model a second application replica: separate AppState/config instance, same Mongo authority.
    let mut state_b = app.state.clone();
    state_b.config.webhook_verify_signature = false;
    state_b.config.webhook_rate_limit_capacity = 1;
    state_b.config.webhook_rate_limit_window_seconds = 600;

    let foreign_before = state_a
        .db
        .raw()
        .collection::<Document>("wechat_accounts")
        .find_one(doc! { "_id": account_b_id }, None)
        .await
        .expect("read workspace B account before")
        .expect("workspace B account exists");

    let _ = wechat_webhook(
        State(state_a.clone()),
        HeaderMap::new(),
        webhook_body(&app_id_a, &unique("sr024-a-first")),
    )
    .await
    .expect("workspace A first webhook is accepted");
    let limited = wechat_webhook(
        State(state_b.clone()),
        HeaderMap::new(),
        webhook_body(&app_id_a, &unique("sr024-a-second")),
    )
    .await;
    assert!(
        matches!(limited, Err(AppError::RateLimited { .. })),
        "workspace A second request must exhaust only A's bucket: {limited:?}"
    );

    let _ = wechat_webhook(
        State(state_b.clone()),
        HeaderMap::new(),
        webhook_body(&app_id_b, &unique("sr024-b-first")),
    )
    .await
    .expect("workspace B first webhook must have an independent bucket");

    assert_eq!(
        state_a
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_a,
                    "account_id": &account_id,
                    "kind": "webhook_rate_limited",
                },
                None,
            )
            .await
            .expect("count workspace A rate-limit events"),
        1,
        "the exhausted bucket must emit one event in its resolved workspace"
    );
    assert_eq!(
        state_a
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_b,
                    "account_id": &account_id,
                    "kind": "webhook_rate_limited",
                },
                None,
            )
            .await
            .expect("count workspace B rate-limit events"),
        0,
        "workspace A exhaustion must not emit a workspace B event"
    );
    let foreign_after = state_a
        .db
        .raw()
        .collection::<Document>("wechat_accounts")
        .find_one(doc! { "_id": account_b_id }, None)
        .await
        .expect("read workspace B account after")
        .expect("workspace B account remains");
    assert_eq!(
        foreign_after, foreign_before,
        "workspace B account BSON changed"
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn sr025_pacing_ignores_same_account_history_from_other_workspace() {
    let app = TestApp::start().await;
    let mcp = start_mcp_mock().await;
    let workspace_local = unique("sr025-ws-local");
    let workspace_foreign = unique("sr025-ws-foreign");
    let account_id = unique("sr025-shared-account");
    let local_wxid = unique("sr025-local-contact");
    let foreign_wxid = unique("sr025-foreign-contact");
    let local_account = account(
        &workspace_local,
        &account_id,
        &unique("sr025-local-app"),
        Some(mcp.uri()),
    );
    let foreign_account = account(
        &workspace_foreign,
        &account_id,
        &unique("sr025-foreign-app"),
        Some(mcp.uri()),
    );
    app.state
        .db
        .accounts()
        .insert_many([local_account, foreign_account], None)
        .await
        .expect("seed same account id in two workspaces");
    app.state
        .db
        .contacts()
        .insert_one(contact(&workspace_local, &account_id, &local_wxid), None)
        .await
        .expect("seed local managed contact");

    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    state.config.account_send_min_interval_ms = 60_000;
    state.config.account_send_max_interval_ms = 60_000;
    state.config.account_daily_send_soft_cap = 1;

    let foreign_outbox_id = match enqueue(
        &state,
        enqueue_request(
            &workspace_foreign,
            &account_id,
            &foreign_wxid,
            &unique("sr025-foreign-run"),
        ),
    )
    .await
    .expect("enqueue foreign history")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected foreign Created, got {other:?}"),
    };
    state
        .db
        .collection_agent_send_outbox()
        .update_one(
            doc! { "_id": foreign_outbox_id },
            doc! { "$set": { "status": "sent", "sent_at": DateTime::now() } },
            None,
        )
        .await
        .expect("mark foreign history sent");
    let foreign_before = state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .find_one(doc! { "_id": foreign_outbox_id }, None)
        .await
        .expect("read foreign outbox before")
        .expect("foreign outbox exists");

    let local_outbox_id = match enqueue(
        &state,
        enqueue_request(
            &workspace_local,
            &account_id,
            &local_wxid,
            &unique("sr025-local-run"),
        ),
    )
    .await
    .expect("enqueue local outbound")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected local Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "sr025-worker", 60)
        .await
        .expect("claim local outbound")
        .expect("local outbound must be claimable");
    assert_eq!(claimed.id, Some(local_outbox_id));
    process_entry(&state, &claimed)
        .await
        .expect("dispatch local outbound");

    let local_after = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": local_outbox_id }, None)
        .await
        .expect("read local outbox")
        .expect("local outbox exists");
    assert_eq!(
        local_after.status,
        OutboxStatus::Sent.as_str(),
        "foreign workspace history must not defer the local send: {local_after:?}"
    );
    let mcp_requests = mcp.received_requests().await.expect("read MCP requests");
    let mcp_sequence = mcp_call_sequence(&mcp_requests);
    assert_eq!(
        mcp_sequence
            .iter()
            .filter(|call| call.as_str() == "tools/call:message_send_text")
            .count(),
        1,
        "the local outbound must invoke message_send_text exactly once; sequence={mcp_sequence:?}"
    );
    assert_eq!(
        state
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_local,
                    "account_id": &account_id,
                    "kind": "agent.account_daily_send_soft_cap_exceeded",
                },
                None,
            )
            .await
            .expect("count local soft-cap warnings"),
        0,
        "foreign workspace history must not trigger a local soft-cap warning"
    );
    let foreign_after = state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .find_one(doc! { "_id": foreign_outbox_id }, None)
        .await
        .expect("read foreign outbox after")
        .expect("foreign outbox remains");
    assert_eq!(foreign_after, foreign_before, "foreign outbox BSON changed");

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn sr026_outbox_idempotency_is_workspace_account_scoped() {
    let app = TestApp::start().await;
    let workspace_a = unique("sr026-ws-a");
    let workspace_b = unique("sr026-ws-b");
    let account_id = unique("sr026-shared-account");
    let contact_wxid = unique("sr026-shared-contact");

    let request_a = enqueue_request(&workspace_a, &account_id, &contact_wxid, "sr026-shared-run");
    let request_b = enqueue_request(&workspace_b, &account_id, &contact_wxid, "sr026-shared-run");
    let (outbox_a, key_a) = match enqueue(&app.state, request_a.clone())
        .await
        .expect("enqueue workspace A")
    {
        EnqueueOutcome::Created {
            outbox_id,
            idempotency_key,
        } => (outbox_id, idempotency_key),
        other => panic!("expected workspace A Created, got {other:?}"),
    };
    let (outbox_b, key_b) = match enqueue(&app.state, request_b)
        .await
        .expect("enqueue workspace B")
    {
        EnqueueOutcome::Created {
            outbox_id,
            idempotency_key,
        } => (outbox_id, idempotency_key),
        other => panic!("expected workspace B Created, got {other:?}"),
    };
    assert_ne!(key_a, key_b, "workspace must participate in the v2 key");

    let foreign_before = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .find_one(doc! { "_id": outbox_b }, None)
        .await
        .expect("read workspace B outbox before")
        .expect("workspace B outbox exists");
    match enqueue(&app.state, request_a)
        .await
        .expect("repeat workspace A enqueue")
    {
        EnqueueOutcome::IdempotentSkip {
            existing_outbox_id,
            idempotency_key,
            ..
        } => {
            assert_eq!(existing_outbox_id, outbox_a);
            assert_eq!(idempotency_key, key_a);
        }
        other => panic!("same workspace must dedupe, got {other:?}"),
    }
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(
                doc! {
                    "account_id": &account_id,
                    "contact_wxid": &contact_wxid,
                    "source_event_id": "shared-source-event",
                },
                None
            )
            .await
            .expect("count scoped outboxes"),
        2,
        "the same business identity must exist once per workspace"
    );
    let foreign_after = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .find_one(doc! { "_id": outbox_b }, None)
        .await
        .expect("read workspace B outbox after")
        .expect("workspace B outbox remains");
    assert_eq!(
        foreign_after, foreign_before,
        "workspace B outbox BSON changed"
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn sr027_reaction_stop_cancels_only_same_workspace_account_outbox() {
    let app = TestApp::start().await;
    let workspace_local = app.state.config.default_workspace_id.clone();
    let workspace_foreign = unique("sr027-ws-foreign");
    let account_id = unique("sr027-shared-account");
    let contact_wxid = unique("sr027-shared-contact");
    let local_contact = contact(&workspace_local, &account_id, &contact_wxid);
    let foreign_contact = contact(&workspace_foreign, &account_id, &contact_wxid);
    app.state
        .db
        .contacts()
        .insert_many([local_contact.clone(), foreign_contact], None)
        .await
        .expect("seed same account and contact identity in two workspaces");
    app.state
        .db
        .decision_reviews()
        .insert_one(
            sent_review(&workspace_local, &account_id, &contact_wxid),
            None,
        )
        .await
        .expect("seed local sent review");

    let local_outbox_id = match enqueue(
        &app.state,
        enqueue_request(
            &workspace_local,
            &account_id,
            &contact_wxid,
            "sr027-shared-run",
        ),
    )
    .await
    .expect("enqueue local pending outbox")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected local Created, got {other:?}"),
    };
    let foreign_outbox_id = match enqueue(
        &app.state,
        enqueue_request(
            &workspace_foreign,
            &account_id,
            &contact_wxid,
            "sr027-shared-run",
        ),
    )
    .await
    .expect("enqueue foreign pending outbox")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected foreign Created, got {other:?}"),
    };

    let raw_outbox = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox");
    let foreign_before = raw_outbox
        .find_one(doc! { "_id": foreign_outbox_id }, None)
        .await
        .expect("read foreign outbox before")
        .expect("foreign outbox exists");
    let foreign_events_before = app
        .state
        .db
        .events()
        .count_documents(doc! { "workspace_id": &workspace_foreign }, None)
        .await
        .expect("count foreign events before");

    app.llm.push_response(json!({
        "stopRequested": true,
        "speechAct": "statement",
        "assertionStatus": "asserted",
        "subject": "customer",
        "confidence": 0.95
    }));
    let reaction = inbound(&workspace_local, &account_id, &contact_wxid);
    record_user_reaction(&app.state, &local_contact, &reaction)
        .await
        .expect("record local stop reaction");
    assert_eq!(
        app.llm.calls(),
        1,
        "reaction analysis must call the mock once"
    );

    let local_after = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": local_outbox_id }, None)
        .await
        .expect("read local outbox after")
        .expect("local outbox remains for audit");
    assert_eq!(local_after.status, OutboxStatus::Canceled.as_str());
    assert_eq!(
        local_after.cancel_reason.as_deref(),
        Some("user_reaction_stop_requested")
    );
    let foreign_after = raw_outbox
        .find_one(doc! { "_id": foreign_outbox_id }, None)
        .await
        .expect("read foreign outbox after")
        .expect("foreign outbox remains");
    assert_eq!(
        foreign_after, foreign_before,
        "foreign workspace outbox BSON changed"
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(doc! { "workspace_id": &workspace_foreign }, None)
            .await
            .expect("count foreign events after"),
        foreign_events_before,
        "local stop reaction emitted an event into the foreign workspace"
    );

    app.cleanup().await;
}

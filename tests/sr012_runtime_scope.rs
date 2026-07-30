#![cfg(test)]

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use wechatagent::mcp::{read_roster_snapshot, spawn_roster_refresh};
use wechatagent::models::{
    AgentStatus, Contact, ConversationMessage, MessageDirection, WechatAccount,
};
use wechatagent::webhooks::{contact_key, register_inbound, run_debounce_pipeline};

use crate::common::TestApp;

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", ObjectId::new().to_hex())
}

fn contact(workspace_id: &str, account_id: &str, wxid: &str, status: AgentStatus) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        wxid: wxid.into(),
        nickname: Some(format!("{workspace_id} contact")),
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: status,
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

fn inbound(workspace_id: &str, account_id: &str, wxid: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        contact_wxid: wxid.into(),
        message_id: Some(unique("sr012-message")),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "scope probe".into(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    }
}

fn account(workspace_id: &str, account_id: &str, mcp_url: String) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        alias: format!("{workspace_id}-{account_id}"),
        display_name: format!("{workspace_id}-{account_id}"),
        app_id: Some(unique("sr012-app")),
        wxid: Some(unique("sr012-self")),
        nick_name: None,
        avatar_url: None,
        mcp_base_url: Some(mcp_url),
        mcp_api_key: Some(format!("sr012-key-{workspace_id}")),
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

struct DelayedRosterResponder {
    marker: String,
}

impl Respond for DelayedRosterResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body = serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null);
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        let result = match body.get("method").and_then(Value::as_str) {
            Some("initialize") => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {}
            }),
            Some("tools/call") => json!({
                "structuredContent": {
                    "status": "ready",
                    "items": [{
                        "userName": self.marker,
                        "nickName": self.marker,
                        "sex": 1
                    }]
                }
            }),
            _ => json!({}),
        };
        ResponseTemplate::new(200)
            .set_delay(Duration::from_millis(300))
            .set_body_json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            }))
    }
}

async fn roster_server(marker: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(DelayedRosterResponder {
            marker: marker.to_string(),
        })
        .mount(&server)
        .await;
    server
}

fn contacts_fetch_calls(requests: &[Request]) -> usize {
    requests
        .iter()
        .filter(|request| {
            serde_json::from_slice::<Value>(&request.body)
                .ok()
                .is_some_and(|body| {
                    body.get("method").and_then(Value::as_str) == Some("tools/call")
                        && body.pointer("/params/name").and_then(Value::as_str)
                            == Some("contacts_fetch_full")
                })
        })
        .count()
}

async fn wait_for_both_rosters(
    app: &TestApp,
    workspace_a: &str,
    workspace_b: &str,
    account_id: &str,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let a = read_roster_snapshot(&app.state, workspace_a, account_id)
            .await
            .expect("read workspace A roster");
        let b = read_roster_snapshot(&app.state, workspace_b, account_id)
            .await
            .expect("read workspace B roster");
        if a.is_some() && b.is_some() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "both workspace-scoped roster refreshes must complete; a={} b={}",
            a.is_some(),
            b.is_some()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn debounce_runner_and_reload_are_workspace_account_contact_scoped() {
    let app = TestApp::start().await;
    let workspace_local = unique("sr012-debounce-local");
    let workspace_foreign = unique("sr012-debounce-foreign");
    let account_id = unique("sr012-shared-account");
    let wxid = unique("sr012-shared-wxid");

    let foreign = contact(&workspace_foreign, &account_id, &wxid, AgentStatus::Managed);
    let foreign_id = foreign.id.expect("foreign contact id");
    app.state
        .db
        .contacts()
        .insert_one(foreign, None)
        .await
        .expect("seed only foreign managed contact");

    let local_key = contact_key(&workspace_local, &account_id, &wxid);
    let foreign_key = contact_key(&workspace_foreign, &account_id, &wxid);
    assert_ne!(
        local_key, foreign_key,
        "workspace must participate in debounce key"
    );
    let (local_state, local_spawned) = register_inbound(
        local_key.clone(),
        inbound(&workspace_local, &account_id, &wxid),
        20,
    );
    let (foreign_state, foreign_spawned) = register_inbound(
        foreign_key.clone(),
        inbound(&workspace_foreign, &account_id, &wxid),
        20,
    );
    assert!(
        local_spawned && foreign_spawned,
        "both workspaces need independent runners"
    );
    assert_eq!(local_state.generation.load(Ordering::Acquire), 1);
    assert_eq!(foreign_state.generation.load(Ordering::Acquire), 1);

    run_debounce_pipeline(
        app.state.clone(),
        local_key,
        local_state,
        workspace_local.clone(),
        account_id.clone(),
        wxid.clone(),
        None,
    )
    .await;
    assert_eq!(
        app.llm.calls(),
        0,
        "local runner must not borrow the foreign workspace managed contact"
    );
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "workspace_id": &workspace_local }, None)
            .await
            .expect("count local outbox"),
        0
    );

    app.state
        .db
        .contacts()
        .update_one(
            doc! { "_id": foreign_id },
            doc! { "$set": { "agent_status": "normal" } },
            None,
        )
        .await
        .expect("make foreign contact unmanaged before retiring its runner");
    run_debounce_pipeline(
        app.state.clone(),
        foreign_key,
        foreign_state,
        workspace_foreign,
        account_id,
        wxid,
        None,
    )
    .await;

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn roster_single_flight_is_workspace_account_scoped() {
    let app = TestApp::start().await;
    let server_a = roster_server("sr012-roster-a").await;
    let server_b = roster_server("sr012-roster-b").await;
    let workspace_a = unique("sr012-roster-ws-a");
    let workspace_b = unique("sr012-roster-ws-b");
    let account_id = unique("sr012-roster-shared-account");
    app.state
        .db
        .accounts()
        .insert_many(
            [
                account(&workspace_a, &account_id, server_a.uri()),
                account(&workspace_b, &account_id, server_b.uri()),
            ],
            None,
        )
        .await
        .expect("seed same account id in two workspaces");

    spawn_roster_refresh(app.state.clone(), workspace_a.clone(), account_id.clone());
    spawn_roster_refresh(app.state.clone(), workspace_b.clone(), account_id.clone());
    wait_for_both_rosters(&app, &workspace_a, &workspace_b, &account_id).await;

    let snapshot_a = read_roster_snapshot(&app.state, &workspace_a, &account_id)
        .await
        .expect("read A snapshot")
        .expect("A snapshot exists");
    let snapshot_b = read_roster_snapshot(&app.state, &workspace_b, &account_id)
        .await
        .expect("read B snapshot")
        .expect("B snapshot exists");
    assert_eq!(snapshot_a.friends.len(), 1);
    assert_eq!(snapshot_b.friends.len(), 1);
    assert_eq!(snapshot_a.friends[0].wxid, "sr012-roster-a");
    assert_eq!(snapshot_b.friends[0].wxid, "sr012-roster-b");
    assert_eq!(
        contacts_fetch_calls(&server_a.received_requests().await.expect("A requests")),
        1
    );
    assert_eq!(
        contacts_fetch_calls(&server_b.received_requests().await.expect("B requests")),
        1
    );

    app.cleanup().await;
}

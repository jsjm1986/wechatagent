//! HC-020 Management command safety protocol redlines.
//!
//! These tests cross the production Cookie middleware and Axum router. They prove that a
//! model-authored write plan is frozen behind explicit admin confirmation, bound to the selected
//! account, and recovered conservatively after an uncertain side-effect boundary.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::{AgentStatus, AgentToolCall, Contact, WechatAccount};
use wechatagent::routes::{api_router, AppState};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const ACCOUNT_A: &str = "hc020-account-a";
const ACCOUNT_B: &str = "hc020-account-b";

struct ManagementMcp;

impl Respond for ManagementMcp {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        let result = match body.get("method").and_then(Value::as_str) {
            Some("tools/list") => json!({
                "tools": [
                    { "name": "message_send_text", "description": "raw send must be removed" },
                    { "name": "account_list", "description": "read only" }
                ]
            }),
            Some("tools/call") => json!({ "ok": true }),
            _ => json!({}),
        };
        ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
    }
}

async fn start_mcp() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ManagementMcp)
        .mount(&server)
        .await;
    server
}

fn count_rpc_calls(requests: &[Request], method_name: &str) -> usize {
    requests
        .iter()
        .filter(|request| {
            serde_json::from_slice::<Value>(&request.body)
                .ok()
                .and_then(|body| {
                    body.get("method")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .as_deref()
                == Some(method_name)
        })
        .count()
}

fn account(account_id: &str, mcp_url: &str) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: Some(ObjectId::new()),
        workspace_id: "default".into(),
        account_id: account_id.into(),
        alias: account_id.into(),
        display_name: account_id.into(),
        app_id: Some(format!("hc020-{account_id}")),
        wxid: Some(format!("wxid-{account_id}")),
        nick_name: None,
        avatar_url: None,
        mcp_base_url: Some(mcp_url.into()),
        mcp_api_key: Some("hc020-test-key".into()),
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

fn contact(account_id: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".into(),
        account_id: account_id.into(),
        wxid: "hc020-contact".into(),
        nickname: Some("HC-020 contact".into()),
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
        intent_trajectory: vec![],
        outcome_events: vec![],
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

async fn start_api(state: AppState) -> (String, String, tokio::task::JoinHandle<()>) {
    let admin = AdminUser {
        user_id: "hc020-admin-id".into(),
        username: "hc020-admin".into(),
        password_hash: "unused".into(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec!["default".into()],
        default_workspace: Some("default".into()),
    };
    state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&admin, None)
        .await
        .expect("insert HC-020 admin");
    let session = create_session(&state.db, &admin, 1, "default")
        .await
        .expect("create HC-020 admin session");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind HC-020 API");
    let address = listener.local_addr().expect("HC-020 API address");
    let router = Router::new()
        .nest("/api", api_router(state.clone()))
        .with_state(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve HC-020 API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

async fn create_frozen_write_command(
    app: &common::TestApp,
    client: &reqwest::Client,
    base_url: &str,
    cookie: &str,
    contact_id: ObjectId,
) -> Value {
    app.llm.push_response(json!({
        "intent": "record confirmed deal",
        "riskLevel": "low",
        "requiresConfirmation": false,
        "missingInformation": [],
        "summary": "登记人工确认成交",
        "toolCalls": [{
            "toolName": "wechatagent.write_deal_events",
            "arguments": {
                "contactId": contact_id.to_hex(),
                "amount": 8800,
                "currency": "CNY",
                "eventKind": "deal",
                "note": "HC-020 frozen command"
            }
        }]
    }));

    let session_response = client
        .post(format!("{base_url}/management-agent/sessions"))
        .header(reqwest::header::COOKIE, cookie)
        .json(&json!({
            "accountId": ACCOUNT_A,
            "title": "HC-020 command",
            "dryRun": false
        }))
        .send()
        .await
        .expect("create management session");
    assert_eq!(session_response.status(), StatusCode::OK);
    let session_body: Value = session_response.json().await.expect("decode session");
    let session_id = session_body["id"].as_str().expect("session id");

    let response = client
        .post(format!(
            "{base_url}/management-agent/sessions/{session_id}/messages"
        ))
        .header(reqwest::header::COOKIE, cookie)
        .json(&json!({
            "accountId": ACCOUNT_A,
            "content": "请登记这笔已由我核实的成交",
            "dryRun": false
        }))
        .send()
        .await
        .expect("plan management command");
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.expect("decode command response");
    assert_eq!(body["command"]["status"], "pending_confirmation");
    assert_eq!(body["command"]["accountId"], ACCOUNT_A);
    assert!(body["command"]["planHash"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    body["command"].clone()
}

async fn seed_accounts_and_contact(app: &common::TestApp, mcp_url: &str) -> ObjectId {
    app.state
        .db
        .accounts()
        .insert_many(
            vec![account(ACCOUNT_A, mcp_url), account(ACCOUNT_B, mcp_url)],
            None,
        )
        .await
        .expect("insert HC-020 accounts");
    let contact = contact(ACCOUNT_A);
    let contact_id = contact.id.expect("contact id");
    app.state
        .db
        .contacts()
        .insert_one(contact, None)
        .await
        .expect("insert HC-020 contact");
    contact_id
}

#[test]
#[ignore]
fn frozen_command_requires_matching_account_hash_and_authenticated_admin() {
    std::thread::Builder::new()
        .name("hc020-frozen-command".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()
                .expect("build HC-020 runtime")
                .block_on(
                    frozen_command_requires_matching_account_hash_and_authenticated_admin_inner(),
                );
        })
        .expect("spawn HC-020 test thread")
        .join()
        .expect("HC-020 test thread panicked");
}

async fn frozen_command_requires_matching_account_hash_and_authenticated_admin_inner() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    let contact_id = seed_accounts_and_contact(&app, &mcp.uri()).await;
    let (base_url, cookie, server) = start_api(state).await;
    let client = reqwest::Client::new();
    let command = create_frozen_write_command(&app, &client, &base_url, &cookie, contact_id).await;
    let command_id = command["id"].as_str().expect("command id");
    let plan_hash = command["planHash"].as_str().expect("plan hash");

    let wrong_account = client
        .post(format!(
            "{base_url}/management-agent/commands/{command_id}/confirm"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "accountId": ACCOUNT_B, "planHash": plan_hash }))
        .send()
        .await
        .expect("confirm with wrong account");
    assert_eq!(wrong_account.status(), StatusCode::CONFLICT);

    let wrong_hash = client
        .post(format!(
            "{base_url}/management-agent/commands/{command_id}/confirm"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "accountId": ACCOUNT_A, "planHash": "tampered-plan" }))
        .send()
        .await
        .expect("confirm with wrong hash");
    assert_eq!(wrong_hash.status(), StatusCode::CONFLICT);

    let before = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("read contact before confirmation")
        .expect("contact before confirmation");
    assert!(before.outcome_events.is_empty());

    let confirmed = client
        .post(format!(
            "{base_url}/management-agent/commands/{command_id}/confirm"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "accountId": ACCOUNT_A, "planHash": plan_hash }))
        .send()
        .await
        .expect("confirm frozen command");
    assert_eq!(confirmed.status(), StatusCode::OK);
    let confirmed_body: Value = confirmed.json().await.expect("decode confirmation");
    assert_eq!(confirmed_body["status"], "succeeded");

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("read contact after confirmation")
        .expect("contact after confirmation");
    assert_eq!(after.outcome_events.len(), 1);
    assert_eq!(after.outcome_events[0].verification, "staff_confirmed");
    assert_eq!(after.outcome_events[0].marked_by, "hc020-admin");

    let stored_run = app
        .state
        .db
        .command_runs()
        .find_one(
            doc! { "_id": ObjectId::parse_str(command_id).unwrap() },
            None,
        )
        .await
        .expect("read command run")
        .expect("command run");
    assert_eq!(stored_run.status, "succeeded");
    assert_eq!(stored_run.account_id, ACCOUNT_A);
    assert_eq!(stored_run.plan_hash.as_deref(), Some(plan_hash));
    assert_eq!(stored_run.confirmed_by.as_deref(), Some("hc020-admin"));
    assert_eq!(app.llm.calls(), 1, "only the planning LLM call is allowed");

    server.abort();
    app.cleanup().await;
}

#[test]
#[ignore]
fn stale_executing_intent_becomes_unknown_without_replaying_mcp() {
    std::thread::Builder::new()
        .name("hc020-stale-intent".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()
                .expect("build HC-020 runtime")
                .block_on(stale_executing_intent_becomes_unknown_without_replaying_mcp_inner());
        })
        .expect("spawn HC-020 test thread")
        .join()
        .expect("HC-020 test thread panicked");
}

async fn stale_executing_intent_becomes_unknown_without_replaying_mcp_inner() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    let contact_id = seed_accounts_and_contact(&app, &mcp.uri()).await;
    let (base_url, cookie, server) = start_api(state).await;
    let client = reqwest::Client::new();
    let command = create_frozen_write_command(&app, &client, &base_url, &cookie, contact_id).await;
    let command_id =
        ObjectId::parse_str(command["id"].as_str().expect("command id")).expect("valid command id");
    let plan_hash = command["planHash"].as_str().expect("plan hash");
    let stale_at = DateTime::from_millis(DateTime::now().timestamp_millis() - 10 * 60 * 1000);

    app.state
        .db
        .command_runs()
        .update_one(
            doc! { "_id": command_id, "status": "pending_confirmation" },
            doc! { "$set": {
                "status": "running",
                "execution_token": "dead-process-token",
                "execution_started_at": stale_at,
                "updated_at": stale_at,
            } },
            None,
        )
        .await
        .expect("seed stale command lease");
    let now = DateTime::now();
    let intent_key = format!("management-tool:v1:{command_id}:{plan_hash}:0");
    app.state
        .db
        .tool_calls()
        .insert_one(
            AgentToolCall {
                id: Some(ObjectId::new()),
                workspace_id: app.state.config.default_workspace_id.clone(),
                account_id: ACCOUNT_A.to_string(),
                command_run_id: command_id,
                intent_key: Some(intent_key),
                call_index: 0,
                tool_name: "wechatagent.write_deal_events".to_string(),
                arguments: doc! { "contactId": contact_id.to_hex(), "amount": 8800_i64 },
                status: "executing".to_string(),
                response: None,
                error: None,
                execution_started_at: Some(stale_at),
                finalized_at: None,
                created_at: stale_at,
                updated_at: now,
            },
            None,
        )
        .await
        .expect("seed executing tool intent");

    let recovered = client
        .post(format!(
            "{base_url}/management-agent/commands/{command_id}/confirm"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "accountId": ACCOUNT_A, "planHash": plan_hash }))
        .send()
        .await
        .expect("recover stale command");
    assert_eq!(recovered.status(), StatusCode::OK);
    let recovered_body: Value = recovered.json().await.expect("decode recovery");
    assert_eq!(recovered_body["status"], "execution_unknown");

    let stored_tool = app
        .state
        .db
        .tool_calls()
        .find_one(doc! { "command_run_id": command_id, "call_index": 0 }, None)
        .await
        .expect("read tool intent")
        .expect("tool intent");
    assert_eq!(stored_tool.status, "execution_unknown");
    assert!(stored_tool.finalized_at.is_some());
    let stored_run = app
        .state
        .db
        .command_runs()
        .find_one(doc! { "_id": command_id }, None)
        .await
        .expect("read recovered command")
        .expect("recovered command");
    assert_eq!(stored_run.status, "execution_unknown");
    assert!(stored_run.execution_token.is_none());

    let contact = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("read unchanged contact")
        .expect("unchanged contact");
    assert!(contact.outcome_events.is_empty());
    let requests = mcp.received_requests().await.expect("read MCP requests");
    assert_eq!(count_rpc_calls(&requests, "tools/call"), 0);

    server.abort();
    app.cleanup().await;
}

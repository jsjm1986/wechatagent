//! SR-094 dynamic redline: both runtime-parameter write paths must enforce the
//! typed boundary, and Guide changes with workspace-wide impact must remain a
//! frozen, explicitly confirmed transaction.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::{
    AgentStatus, Contact, OperationDomainConfig, OperationPlaybook, WechatAccount,
};
use wechatagent::routes::api_router;

use crate::common::TestApp;

struct TestApi {
    base_url: String,
    cookie: String,
    server: tokio::task::JoinHandle<()>,
}

fn account(workspace_id: &str, account_id: &str) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        alias: account_id.to_string(),
        display_name: account_id.to_string(),
        app_id: None,
        wxid: None,
        nick_name: None,
        avatar_url: None,
        mcp_base_url: None,
        mcp_api_key: None,
        webhook_secret: None,
        online: true,
        status: Some("active".to_string()),
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

fn managed_contact(
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
    playbook: &OperationPlaybook,
) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        wxid: wxid.to_string(),
        nickname: Some("SR-094 customer".to_string()),
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: playbook.id,
        playbook_version: Some(playbook.version),
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
        operation_state: Some("need_discovery".to_string()),
        operation_state_reason: None,
        operation_state_confidence: Some(8),
        operation_state_updated_at: Some(now),
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

async fn start_api(app: &TestApp, workspace_id: &str) -> TestApi {
    let admin = AdminUser {
        user_id: "sr094-admin".to_string(),
        username: "sr094-admin".to_string(),
        password_hash: "unused".to_string(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![workspace_id.to_string()],
        default_workspace: Some(workspace_id.to_string()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&admin, None)
        .await
        .expect("seed SR-094 admin");
    let session = create_session(&app.state.db, &admin, 1, workspace_id)
        .await
        .expect("create SR-094 session");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind SR-094 API");
    let address = listener.local_addr().expect("SR-094 API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve SR-094 API");
    });
    TestApi {
        base_url: format!("http://{address}/api"),
        cookie: format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    }
}

fn domain_put_body(item: &Value, runtime_parameters: Value) -> Value {
    json!({
        "name": item["name"].clone(),
        "goal": item["goal"].clone(),
        "methodology": item["methodology"].clone(),
        "workflow": item["workflow"].clone(),
        "toolPolicy": item["toolPolicy"].clone(),
        "automationPolicy": item["automationPolicy"].clone(),
        "reviewPolicy": item["reviewPolicy"].clone(),
        "runtimeParameters": runtime_parameters,
        "stateMachine": item["stateMachine"].clone(),
        "assistModeEnabled": item["assistModeEnabled"].clone(),
    })
}

async fn current_domain(app: &TestApp, workspace_id: &str) -> OperationDomainConfig {
    app.state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations",
                "current_version": true,
            },
            None,
        )
        .await
        .expect("load current domain")
        .expect("current user-operations domain")
}

fn runtime_integer(document: &Document, key: &str) -> Option<i64> {
    match document.get(key) {
        Some(Bson::Int32(value)) => Some(i64::from(*value)),
        Some(Bson::Int64(value)) => Some(*value),
        _ => None,
    }
}

#[test]
#[ignore = "requires MongoDB replica set"]
fn typed_runtime_writes_and_guide_apply_are_enforced_end_to_end() {
    std::thread::Builder::new()
        .name("sr094-runtime-redline".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build SR-094 runtime")
                .block_on(typed_runtime_writes_and_guide_apply_are_enforced_end_to_end_inner())
        })
        .expect("spawn SR-094 test thread")
        .join()
        .expect("SR-094 test thread panicked");
}

async fn typed_runtime_writes_and_guide_apply_are_enforced_end_to_end_inner() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    app.state
        .db
        .accounts()
        .insert_one(account(&workspace_id, &account_id), None)
        .await
        .expect("seed SR-094 account");
    let playbook = app
        .state
        .db
        .operation_playbooks()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": &account_id,
                "release_status": "published",
                "is_default": true,
            },
            None,
        )
        .await
        .expect("load default playbook")
        .expect("default published playbook");
    let contact = managed_contact(
        &workspace_id,
        &account_id,
        &format!("sr094-{}", ObjectId::new().to_hex()),
        &playbook,
    );
    let contact_id = contact.id.expect("contact id");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("seed SR-094 contact");
    wechatagent::agent::load_or_create_operating_memory(&app.state, &contact)
        .await
        .expect("seed SR-094 memory");

    let api = start_api(&app, &workspace_id).await;
    let client = reqwest::Client::new();
    let domain_url = format!("{}/operation-domains/user_operations", api.base_url);
    let get_domain = client
        .get(&domain_url)
        .header(reqwest::header::COOKIE, &api.cookie)
        .send()
        .await
        .expect("GET operation domain");
    assert_eq!(get_domain.status(), StatusCode::OK);
    let domain_response: Value = get_domain.json().await.expect("domain JSON");
    let item = &domain_response["item"];
    let mut legal_runtime = item["runtimeParameters"].clone();
    legal_runtime["maxDailyTouches"] = json!(4);
    legal_runtime["factRiskBlockAt"] = json!(8);
    let legal_put = client
        .put(&domain_url)
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&domain_put_body(item, legal_runtime.clone()))
        .send()
        .await
        .expect("legal runtime PUT");
    assert_eq!(legal_put.status(), StatusCode::OK);
    let legal_domain = current_domain(&app, &workspace_id).await;
    assert_eq!(
        legal_domain
            .runtime_parameters
            .get_i32("maxDailyTouches")
            .ok(),
        Some(4)
    );
    assert_eq!(
        legal_domain
            .runtime_parameters
            .get_i32("hallucinationBlockAt")
            .ok(),
        Some(8),
        "legacy alias must be persisted under the canonical key"
    );
    assert!(!legal_domain
        .runtime_parameters
        .contains_key("factRiskBlockAt"));

    let raw_domains = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_domain_configs");
    let domain_id = legal_domain.id.expect("domain id");
    let before_invalid = raw_domains
        .find_one(doc! { "_id": domain_id }, None)
        .await
        .expect("load domain before invalid PUT")
        .expect("domain before invalid PUT");
    let mut invalid_runtime = legal_runtime;
    invalid_runtime["unknownRuntimeKey"] = json!(1);
    let invalid_put = client
        .put(&domain_url)
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&domain_put_body(item, invalid_runtime))
        .send()
        .await
        .expect("invalid runtime PUT");
    assert_eq!(invalid_put.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        raw_domains
            .find_one(doc! { "_id": domain_id }, None)
            .await
            .expect("load domain after invalid PUT")
            .expect("domain after invalid PUT"),
        before_invalid,
        "invalid manual runtime write must be complete BSON zero-write"
    );

    let preview_url = format!("{}/user-operations/guide/preview", api.base_url);
    app.llm.push_response(json!({
        "summary": "尝试修改高风险预算。",
        "suggestedChanges": {
            "domainRuntimeParameters": { "runTokenBudget": 40_000 }
        }
    }));
    let before_high_risk_domain = raw_domains
        .find_one(doc! { "_id": domain_id }, None)
        .await
        .expect("load domain before high-risk preview")
        .expect("domain before high-risk preview");
    let preview_count_before = app
        .state
        .db
        .user_operation_guide_previews()
        .count_documents(doc! { "workspace_id": &workspace_id }, None)
        .await
        .expect("count previews before high-risk request");
    let high_risk = client
        .post(&preview_url)
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&json!({
            "accountId": &account_id,
            "contactId": contact_id.to_hex(),
            "instruction": "请全局提高所有好友的模型预算",
            "mode": "smart"
        }))
        .send()
        .await
        .expect("high-risk Guide preview");
    assert_eq!(high_risk.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        raw_domains
            .find_one(doc! { "_id": domain_id }, None)
            .await
            .expect("load domain after high-risk preview")
            .expect("domain after high-risk preview"),
        before_high_risk_domain
    );
    assert_eq!(
        app.state
            .db
            .user_operation_guide_previews()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .expect("count previews after high-risk request"),
        preview_count_before,
        "rejected high-risk Guide output must not create an apply capability"
    );

    app.llm.push_response(json!({
        "summary": "全局降低每天触达次数。",
        "suggestedChanges": {
            "domainRuntimeParameters": { "maxDailyTouches": 2 }
        }
    }));
    let preview = client
        .post(&preview_url)
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&json!({
            "accountId": &account_id,
            "contactId": contact_id.to_hex(),
            "instruction": "请把所有好友每天最多触达次数全局调整为 2",
            "mode": "smart"
        }))
        .send()
        .await
        .expect("valid Guide preview");
    assert_eq!(preview.status(), StatusCode::OK);
    let preview_body: Value = preview.json().await.expect("Guide preview JSON");
    let preview_item = &preview_body["item"];
    assert_eq!(preview_item["impactScope"], "workspace_user_operations");
    assert_eq!(preview_item["requiresStrongConfirmation"], true);
    assert_eq!(
        preview_item["suggestedChanges"]["domainRuntimeParameters"]["maxDailyTouches"],
        2
    );
    let preview_id = preview_item["id"].as_str().expect("preview id");
    let candidate_hash = preview_item["candidateHash"]
        .as_str()
        .expect("candidate hash");
    let raw_previews = app
        .state
        .db
        .raw()
        .collection::<Document>("user_operation_guide_previews");
    let preview_object_id = ObjectId::parse_str(preview_id).expect("preview object id");
    let before_confirmation = raw_previews
        .find_one(doc! { "_id": preview_object_id }, None)
        .await
        .expect("load frozen preview")
        .expect("frozen preview");
    let before_confirmation_domain = raw_domains
        .find_one(doc! { "_id": domain_id }, None)
        .await
        .expect("load domain before confirmation")
        .expect("domain before confirmation");
    let apply_url = format!("{}/user-operations/guide/apply", api.base_url);
    let apply_body = json!({
        "previewId": preview_id,
        "expectedAccountId": &account_id,
        "expectedContactId": contact_id.to_hex(),
        "candidateHash": candidate_hash,
        "confirmGlobalImpact": false,
    });
    let missing_confirmation = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&apply_body)
        .send()
        .await
        .expect("apply without global confirmation");
    assert_eq!(missing_confirmation.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        raw_previews
            .find_one(doc! { "_id": preview_object_id }, None)
            .await
            .expect("load preview after missing confirmation")
            .expect("preview after missing confirmation"),
        before_confirmation
    );
    assert_eq!(
        raw_domains
            .find_one(doc! { "_id": domain_id }, None)
            .await
            .expect("load domain after missing confirmation")
            .expect("domain after missing confirmation"),
        before_confirmation_domain
    );

    let mut confirmed_body = apply_body;
    confirmed_body["confirmGlobalImpact"] = json!(true);
    let applied = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&confirmed_body)
        .send()
        .await
        .expect("confirmed Guide apply");
    assert_eq!(applied.status(), StatusCode::OK);
    let receipt: Value = applied.json().await.expect("Guide apply receipt");
    assert_eq!(receipt["item"]["committed"], true);
    assert_eq!(receipt["item"]["impactScope"], "workspace_user_operations");
    assert_eq!(receipt["item"]["candidateHash"], candidate_hash);
    let applied_domain = current_domain(&app, &workspace_id).await;
    assert_eq!(
        runtime_integer(&applied_domain.runtime_parameters, "maxDailyTouches"),
        Some(2)
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "account_id": &account_id,
                    "kind": "user_operation_guide_applied",
                    "details.previewId": preview_id,
                },
                None,
            )
            .await
            .expect("count Guide apply audit"),
        1
    );
    let replay = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&confirmed_body)
        .send()
        .await
        .expect("replay Guide apply");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        replay.json::<Value>().await.expect("replay receipt"),
        receipt,
        "same candidate hash must return the stable receipt without reapplying"
    );

    api.server.abort();
    app.cleanup().await;
}

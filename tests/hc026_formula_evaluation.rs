//! HC-026 formula evaluation budget and ground-truth Router redlines.
#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::llm::ChatUsage;
use wechatagent::models::{EvaluationScenario, WechatAccount};
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
        off_hours: vec![],
        created_at: now,
        updated_at: now,
    }
}

async fn start_api(app: &TestApp, workspace_id: &str, account_id: &str) -> TestApi {
    let admin = AdminUser {
        user_id: "hc026-admin".to_string(),
        username: "hc026-admin".to_string(),
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
        .expect("seed HC-026 admin");
    app.state
        .db
        .accounts()
        .insert_one(account(workspace_id, account_id), None)
        .await
        .expect("seed HC-026 account");
    let session = create_session(&app.state.db, &admin, 1, workspace_id)
        .await
        .expect("create HC-026 session");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind HC-026 API");
    let address = listener.local_addr().expect("HC-026 API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve HC-026 API");
    });
    TestApi {
        base_url: format!("http://{address}/api"),
        cookie: format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    }
}

fn scenario(workspace_id: &str, account_id: &str, id: &str) -> EvaluationScenario {
    let now = DateTime::now();
    EvaluationScenario {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        scenario_id: id.to_string(),
        title: id.to_string(),
        description: String::new(),
        account_id: Some(account_id.to_string()),
        contact_seed: doc! {},
        inbound_messages: vec!["hello".to_string()],
        ground_truth: doc! {
            "trust": 9,
            "conversionReadiness": 5,
            "emotionalValue": 8,
            "nextBestActionScore": 8,
        },
        tags: vec![],
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn decision_json() -> Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "The contact opened a conversation.",
        "relationshipRead": "This is an early relationship.",
        "operationGoal": "Acknowledge and ask one useful question.",
        "knowledgeNeedReason": "No product claim is needed.",
        "memoryUpdateReason": "No durable fact was established.",
        "selfCritique": "Keep the reply concise.",
        "whyShouldReply": "The contact sent a direct message.",
        "whySkipReply": "",
        "riskSelfCheck": "No factual or product commitment is made.",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "new_contact",
        "shouldReply": true,
        "replyText": "Hello. What would you like to solve first?",
        "usedKnowledgeIds": [],
        "conversationMode": "casual_relationship",
        "conversationModeReason": "Use a light opening question."
    })
}

fn review_json() -> Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": 9,
            "emotionalValue": 8,
            "productAccuracy": 9,
            "boundaryPrivacySafety": 9,
            "relationshipProgress": 8,
            "conversionReadiness": 5,
            "pressureRisk": 1,
            "factRisk": 1
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "No product claim."
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "Safe and appropriate.",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true
    })
}

fn known_usage() -> ChatUsage {
    ChatUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        usage_known: true,
        ..Default::default()
    }
}

async fn post_evaluation(api: &TestApi, account_id: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!(
            "{}/user-operations/evaluations/formula-adherence",
            api.base_url
        ))
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&json!({ "accountId": account_id }))
        .send()
        .await
        .expect("call HC-026 evaluation Router")
}

#[tokio::test]
#[ignore]
async fn active_scenario_without_complete_truth_is_rejected_before_write() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let api = start_api(&app, &workspace_id, &account_id).await;

    let response = reqwest::Client::new()
        .post(format!("{}/evaluation-scenarios", api.base_url))
        .header(reqwest::header::COOKIE, &api.cookie)
        .json(&json!({
            "scenarioId": "missing-truth",
            "title": "Missing truth",
            "accountId": account_id,
            "inboundMessages": ["hello"],
            "groundTruth": { "trust": 9 },
            "status": "active"
        }))
        .send()
        .await
        .expect("create invalid active scenario");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        app.state
            .db
            .evaluation_scenarios()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .expect("count scenarios"),
        0
    );

    api.server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn evaluation_counts_only_its_shadow_budget_not_production_logs() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let api = start_api(&app, &workspace_id, &account_id).await;
    app.state
        .db
        .evaluation_scenarios()
        .insert_one(scenario(&workspace_id, &account_id, "known-usage"), None)
        .await
        .expect("seed known-usage scenario");

    // This row is deliberately in the future. The old shared-log algorithm
    // would include it and report a false budget overrun for this evaluation.
    app.state
        .db
        .raw()
        .collection::<Document>("agent_run_logs")
        .insert_one(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": &account_id,
                "tokens_used": 999_999_i64,
                "created_at": DateTime::from_millis(DateTime::now().timestamp_millis() + 60_000),
            },
            None,
        )
        .await
        .expect("seed unrelated production run log");

    app.llm
        .push_response_with_usage(decision_json(), known_usage());
    app.llm
        .push_response_with_usage(review_json(), known_usage());
    app.llm
        .push_response_with_usage(common::independent_claim_gate_pass_json(), known_usage());

    let response = post_evaluation(&api, &account_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.expect("decode evaluation response");
    assert_eq!(body["summary"]["totalTokensUsed"], json!(45));
    assert_eq!(body["summary"]["totalLlmCallsUsed"], json!(3));
    assert_eq!(body["summary"]["unknownUsageCalls"], json!(0));
    assert_eq!(body["summary"]["usageComplete"], json!(true));
    assert_eq!(body["summary"]["scenarioCount"], json!(1));
    assert_eq!(body["summary"]["degraded"], json!(false));
    assert_eq!(app.llm.calls(), 3);

    api.server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn failed_llm_attempt_marks_usage_unknown_and_stops_later_scenarios() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let api = start_api(&app, &workspace_id, &account_id).await;
    app.state
        .db
        .evaluation_scenarios()
        .insert_many(
            [
                scenario(&workspace_id, &account_id, "failure-a"),
                scenario(&workspace_id, &account_id, "failure-b"),
            ],
            None,
        )
        .await
        .expect("seed two failure scenarios");

    // No response is queued. The first actual LLM call fails, and its token
    // usage is unknowable. The evaluation must not start scenario two.
    let response = post_evaluation(&api, &account_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response
        .json()
        .await
        .expect("decode failed evaluation response");
    assert_eq!(body["summary"]["totalTokensUsed"], json!(0));
    assert_eq!(body["summary"]["totalLlmCallsUsed"], json!(1));
    assert_eq!(body["summary"]["unknownUsageCalls"], json!(1));
    assert_eq!(body["summary"]["usageComplete"], json!(false));
    assert_eq!(body["summary"]["scenarioCount"], json!(0));
    assert_eq!(body["summary"]["degraded"], json!(true));
    assert_eq!(
        body["summary"]["degradedReason"],
        json!("evaluation_budget_usage_unknown")
    );
    assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(app.llm.calls(), 1);

    api.server.abort();
    app.cleanup().await;
}

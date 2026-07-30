//! SR-125: digest dispatch authorization through the production Cookie Router.
#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use reqwest::{Response, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminSession, AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::{
    KnowledgeChatTurn, KnowledgeDailyReport, KnowledgeDigestCard, WechatAccount,
};
use wechatagent::routes::api_router;

use crate::common::TestApp;

const WORKSPACE: &str = "sr125-workspace";
const ACCOUNT_A: &str = "account-a";
const ACCOUNT_B: &str = "account-b";
const REPORT_DATE: &str = "2026-07-26";

struct TestApi {
    base_url: String,
    owner_cookie: String,
    other_cookie: String,
    owner_id: String,
    server: tokio::task::JoinHandle<()>,
}

fn admin(id: &str) -> AdminUser {
    AdminUser {
        user_id: id.to_string(),
        username: id.to_string(),
        password_hash: "unused".to_string(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![WORKSPACE.to_string()],
        default_workspace: Some(WORKSPACE.to_string()),
    }
}

fn account(account_id: &str) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: Some(ObjectId::new()),
        workspace_id: WORKSPACE.to_string(),
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

async fn start_api(app: &TestApp) -> TestApi {
    let owner = admin("sr125-owner");
    let other = admin("sr125-other");
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_many([owner.clone(), other.clone()], None)
        .await
        .expect("seed SR-125 admins");
    app.state
        .db
        .accounts()
        .insert_many([account(ACCOUNT_A), account(ACCOUNT_B)], None)
        .await
        .expect("seed SR-125 accounts");
    let owner_session = create_session(&app.state.db, &owner, 1, WORKSPACE)
        .await
        .expect("create owner session");
    let other_session = create_session(&app.state.db, &other, 1, WORKSPACE)
        .await
        .expect("create other session");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind SR-125 API");
    let address = listener.local_addr().expect("SR-125 API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve SR-125 API");
    });
    TestApi {
        base_url: format!("http://{address}/api"),
        owner_cookie: session_cookie(&owner_session),
        other_cookie: session_cookie(&other_session),
        owner_id: owner.user_id,
        server,
    }
}

fn session_cookie(session: &AdminSession) -> String {
    format!("{SESSION_COOKIE_NAME}={}", session.session_id)
}

fn report() -> KnowledgeDailyReport {
    let now = DateTime::now();
    KnowledgeDailyReport {
        id: Some(ObjectId::new()),
        workspace_id: WORKSPACE.to_string(),
        account_id: ACCOUNT_A.to_string(),
        report_date: REPORT_DATE.to_string(),
        generated_at: now,
        generated_by: "worker".to_string(),
        status: "ok".to_string(),
        error_kind: None,
        budget_snapshot: doc! { "tokens_used": 12_i64, "llm_calls": 1_i32 },
        cards: vec![KnowledgeDigestCard {
            card_id: ObjectId::new(),
            kind: "chunk_missing_field".to_string(),
            title: "Repair authoritative chunk".to_string(),
            summary: "Server-owned repair summary".to_string(),
            target_refs: vec![doc! { "kind": "chunk", "id": "authoritative-chunk" }],
            suggested_action: "fix_chunk".to_string(),
            severity: "warn".to_string(),
            metric: None,
        }],
        dismissed_card_ids: vec![],
        prompt_versions: doc! { "knowledge.digest.compose": 1_i32 },
        attempt_generation: 7,
        current_generation: 7,
        latest_attempt_status: Some("ok".to_string()),
        latest_attempt_error_kind: None,
        latest_attempt_at: Some(now),
        latest_attempt_budget_snapshot: doc! { "tokens_used": 12_i64, "llm_calls": 1_i32 },
        last_success_at: Some(now),
    }
}

async fn seed_report(app: &TestApp) {
    app.state
        .db
        .knowledge_daily_reports()
        .insert_one(report(), None)
        .await
        .expect("seed SR-125 digest report");
}

async fn public_binding(api: &TestApi) -> Value {
    let response = reqwest::Client::new()
        .get(format!(
            "{}/knowledge/digest/today?accountId={ACCOUNT_A}&reportDate={REPORT_DATE}",
            api.base_url
        ))
        .header(reqwest::header::COOKIE, &api.owner_cookie)
        .send()
        .await
        .expect("read SR-125 public digest");
    assert_eq!(response.status(), StatusCode::OK);
    let digest: Value = response.json().await.expect("decode SR-125 digest");
    json!({
        "accountId": ACCOUNT_A,
        "reportId": digest["reportId"],
        "reportDate": digest["reportDate"],
        "reportGeneration": digest["currentGeneration"],
        "reportHash": digest["reportHash"],
        "selectedCards": [{
            "cardId": digest["cards"][0]["cardId"],
            "cardHash": digest["cards"][0]["cardHash"],
        }],
    })
}

async fn create_task(
    api: &TestApi,
    cookie: &str,
    session_id: &str,
    binding: Value,
    extra: Value,
) -> Response {
    let mut body = json!({
        "sessionId": session_id,
        "accountId": ACCOUNT_A,
        "operatorId": "sr125-operator",
        "digestSelection": binding,
    });
    if let (Some(target), Some(fields)) = (body.as_object_mut(), extra.as_object()) {
        target.extend(fields.clone());
    }
    reqwest::Client::new()
        .post(format!("{}/knowledge/chat/tasks", api.base_url))
        .header(reqwest::header::COOKIE, cookie)
        .json(&body)
        .send()
        .await
        .expect("create SR-125 task")
}

async fn task_and_progress_counts(app: &TestApp) -> (u64, u64) {
    let tasks = app
        .state
        .db
        .knowledge_chat_tasks()
        .count_documents(doc! { "workspace_id": WORKSPACE }, None)
        .await
        .expect("count SR-125 tasks");
    let progress = app
        .state
        .db
        .knowledge_chat_turns()
        .count_documents(
            doc! { "workspace_id": WORKSPACE, "kind": "task_progress" },
            None,
        )
        .await
        .expect("count SR-125 progress turns");
    (tasks, progress)
}

async fn error(response: Response, status: StatusCode) -> String {
    assert_eq!(response.status(), status);
    response
        .json::<Value>()
        .await
        .expect("decode SR-125 rejection")["error"]
        .as_str()
        .expect("SR-125 rejection error")
        .to_string()
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn canvas_dispatch_rebuilds_authoritative_task_through_cookie_router() {
    let app = TestApp::start().await;
    seed_report(&app).await;
    let api = start_api(&app).await;
    let binding = public_binding(&api).await;

    let response = create_task(&api, &api.owner_cookie, "sr125-canvas", binding, json!({})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.expect("decode canvas dispatch");
    let task_id =
        ObjectId::parse_str(body["taskId"].as_str().expect("taskId")).expect("task ObjectId");
    let task = app
        .state
        .db
        .knowledge_chat_tasks()
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("read dispatched task")
        .expect("dispatched task exists");
    assert_eq!(task.workspace_id, WORKSPACE);
    assert_eq!(task.account_id, ACCOUNT_A);
    assert_eq!(task.owner_admin_id.as_deref(), Some(api.owner_id.as_str()));
    assert_eq!(task.cards.len(), 1);
    assert_eq!(task.planned_steps.len(), 1);
    let step = &task.planned_steps[0];
    assert_eq!(step.get_str("action").unwrap(), "fix_chunk");
    assert_eq!(
        step.get_str("targetChunkId").unwrap(),
        "authoritative-chunk"
    );
    assert_eq!(
        step.get_str("summary").unwrap(),
        "Server-owned repair summary"
    );
    assert!(task.dispatch_binding.is_some());
    assert_eq!(task_and_progress_counts(&app).await, (1, 1));

    api.server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn chat_dispatch_requires_and_persists_server_sealed_candidate() {
    let app = TestApp::start().await;
    seed_report(&app).await;
    let api = start_api(&app).await;
    let binding = public_binding(&api).await;

    let initial = create_task(
        &api,
        &api.owner_cookie,
        "sr125-seed",
        binding.clone(),
        json!({}),
    )
    .await;
    assert_eq!(initial.status(), StatusCode::OK);
    let seeded = app
        .state
        .db
        .knowledge_chat_tasks()
        .find_one(doc! { "session_id": "sr125-seed" }, None)
        .await
        .expect("read seeded dispatch")
        .expect("seeded dispatch exists");
    let candidate_hash = seeded
        .dispatch_binding
        .as_ref()
        .and_then(|value| value.get_str("candidateHash").ok())
        .expect("server candidate hash")
        .to_string();

    let now = DateTime::now();
    app.state
        .db
        .knowledge_chat_session_seqs()
        .insert_one(
            doc! {
                "_id": "sr125-workspace|sr125-chat",
                "workspace_id": WORKSPACE,
                "account_id": ACCOUNT_A,
                "session_id": "sr125-chat",
                "owner_admin_id": &api.owner_id,
                "seq": 4_i64,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("seed sealed chat session identity");
    app.state
        .db
        .knowledge_chat_turns()
        .insert_one(
            KnowledgeChatTurn {
                id: Some(ObjectId::new()),
                workspace_id: WORKSPACE.to_string(),
                account_id: ACCOUNT_A.to_string(),
                session_id: "sr125-chat".to_string(),
                turn_index: 4,
                role: "assistant".to_string(),
                intent: Some("digest_action".to_string()),
                content: "Server-sealed digest candidate".to_string(),
                attachments: vec![doc! {
                    "kind": "digest_dispatch_candidate",
                    "candidateHash": &candidate_hash,
                }],
                patch: None,
                missing_fields: vec![],
                followup_questions: vec![],
                status: "pending".to_string(),
                apply_result: None,
                applied_at: None,
                tokens_used: 0,
                prompt_key: None,
                kind: None,
                tool_calls: vec![],
                created_at: now,
            },
            None,
        )
        .await
        .expect("seed sealed assistant turn");

    let response = create_task(
        &api,
        &api.owner_cookie,
        "sr125-chat",
        binding,
        json!({
            "sourceTurnIndex": 4,
            "candidateHash": &candidate_hash,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let task = app
        .state
        .db
        .knowledge_chat_tasks()
        .find_one(doc! { "session_id": "sr125-chat" }, None)
        .await
        .expect("read sealed task")
        .expect("sealed task exists");
    let sealed = task.dispatch_binding.expect("dispatch binding");
    assert_eq!(sealed.get_str("candidateHash").unwrap(), candidate_hash);
    assert_eq!(sealed.get_i32("sourceTurnIndex").unwrap(), 4);
    assert_eq!(task_and_progress_counts(&app).await, (2, 2));

    api.server.abort();
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn stale_cross_admin_and_account_mismatch_dispatches_are_zero_write() {
    let app = TestApp::start().await;
    seed_report(&app).await;
    let api = start_api(&app).await;
    let binding = public_binding(&api).await;
    let baseline = task_and_progress_counts(&app).await;

    let mut stale = binding.clone();
    stale["reportGeneration"] = json!(6);
    let stale_error = error(
        create_task(
            &api,
            &api.owner_cookie,
            "sr125-owned-session",
            stale,
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(stale_error, "digest_dispatch_snapshot_stale");
    assert_eq!(task_and_progress_counts(&app).await, baseline);

    let cross_admin_error = error(
        create_task(
            &api,
            &api.other_cookie,
            "sr125-owned-session",
            binding.clone(),
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(cross_admin_error, "chat_session_scope_conflict");
    assert_eq!(task_and_progress_counts(&app).await, baseline);

    let mut wrong_account = binding;
    wrong_account["accountId"] = json!(ACCOUNT_B);
    let account_error = error(
        create_task(
            &api,
            &api.owner_cookie,
            "sr125-account-mismatch",
            wrong_account,
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(account_error, "digest_dispatch_account_mismatch");
    assert_eq!(task_and_progress_counts(&app).await, baseline);

    api.server.abort();
    app.cleanup().await;
}

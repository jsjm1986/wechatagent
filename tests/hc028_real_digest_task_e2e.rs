//! HC-028: hard real-model business gate for Digest -> Chat -> Task -> Worker.
//!
//! This target intentionally does not use the legacy real-LLM skip macros. A missing
//! provider, an unreachable upstream, an empty digest, or a missing worker artifact is
//! a test failure. The active provider is read from the configured production config
//! database, while every business write goes to TestApp's random replica-set database.

#![cfg(test)]

mod common;

use std::sync::Arc;

use axum::Router;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use reqwest::{Response, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminSession, AdminUser, SESSION_COOKIE_NAME};
use wechatagent::llm::{LlmClient, LlmFormat, LlmProvider};
use wechatagent::models::{
    LlmProviderConfig, OperationKnowledgeChunk, OperationKnowledgeDocument, WechatAccount,
};
use wechatagent::routes::api_router;

use crate::common::capability_evidence::CapabilityEvidence;
use crate::common::{rebuild_app_state_with_real_llm, TestApp};

const WORKSPACE: &str = "default";
const ACCOUNT: &str = "hc028-real-account";
const ADMIN_ID: &str = "hc028-real-admin";
const SESSION_ID: &str = "hc028-real-digest-task";

struct TestApi {
    base_url: String,
    cookie: String,
    server: tokio::task::JoinHandle<()>,
}

async fn load_active_real_provider() -> (Arc<dyn LlmProvider>, String, String) {
    let uri = std::env::var("REAL_LLM_CONFIG_MONGODB_URI")
        .or_else(|_| std::env::var("MONGODB_URI"))
        .expect("HC-028 requires REAL_LLM_CONFIG_MONGODB_URI or MONGODB_URI");
    let database = std::env::var("REAL_LLM_CONFIG_MONGODB_DATABASE")
        .or_else(|_| std::env::var("MONGODB_DATABASE"))
        .expect("HC-028 requires REAL_LLM_CONFIG_MONGODB_DATABASE or MONGODB_DATABASE");
    let workspace =
        std::env::var("REAL_LLM_CONFIG_WORKSPACE").unwrap_or_else(|_| WORKSPACE.to_string());
    let configured_provider_id = std::env::var("REAL_LLM_CONFIG_PROVIDER_ID")
        .ok()
        .filter(|value| !value.trim().is_empty());

    let client = mongodb::Client::with_uri_str(&uri)
        .await
        .expect("connect read-only LLM config database");
    let provider_filter = configured_provider_id
        .as_deref()
        .map(|provider_id| doc! { "workspaceId": &workspace, "providerId": provider_id })
        .unwrap_or_else(|| doc! { "workspaceId": &workspace, "isActive": true });
    let provider = client
        .database(&database)
        .collection::<LlmProviderConfig>("llm_provider_configs")
        .find_one(provider_filter, None)
        .await
        .expect("read configured LLM provider")
        .expect("configured LLM provider is required for HC-028");

    assert!(
        !provider.api_key.trim().is_empty(),
        "active provider key is empty"
    );
    assert!(
        !provider.base_url.trim().is_empty(),
        "active provider URL is empty"
    );
    assert!(
        !provider.model.trim().is_empty(),
        "active provider model is empty"
    );
    let format = LlmFormat::parse(&provider.format).expect("active provider format");
    let provider_id = provider.provider_id.clone();
    let model = provider.model.clone();
    let llm = LlmClient::with_format(
        provider.base_url,
        provider.api_key,
        provider.model,
        format,
        provider.timeout_seconds.unwrap_or(180),
        provider.max_retries.unwrap_or(3),
        provider.retry_base_ms.unwrap_or(2_500),
    )
    .expect("construct active real LLM client");
    (Arc::new(llm), provider_id, model)
}

fn admin() -> AdminUser {
    AdminUser {
        user_id: ADMIN_ID.to_string(),
        username: ADMIN_ID.to_string(),
        password_hash: "unused".to_string(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![WORKSPACE.to_string()],
        default_workspace: Some(WORKSPACE.to_string()),
    }
}

fn account() -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: Some(ObjectId::new()),
        workspace_id: WORKSPACE.to_string(),
        account_id: ACCOUNT.to_string(),
        alias: ACCOUNT.to_string(),
        display_name: "HC-028 real-model test account".to_string(),
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

fn session_cookie(session: &AdminSession) -> String {
    format!("{SESSION_COOKIE_NAME}={}", session.session_id)
}

async fn start_api(state: &wechatagent::routes::AppState) -> TestApi {
    let admin = admin();
    state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&admin, None)
        .await
        .expect("seed HC-028 admin");
    state
        .db
        .accounts()
        .insert_one(account(), None)
        .await
        .expect("seed HC-028 account");
    let session = create_session(&state.db, &admin, 1, WORKSPACE)
        .await
        .expect("create HC-028 session");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind HC-028 API");
    let address = listener.local_addr().expect("HC-028 API address");
    let router = Router::new()
        .nest("/api", api_router(state.clone()))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve HC-028 API");
    });
    TestApi {
        base_url: format!("http://{address}/api"),
        cookie: session_cookie(&session),
        server,
    }
}

async fn seed_repairable_chunk(state: &wechatagent::routes::AppState) -> ObjectId {
    let now = DateTime::now();
    let document_id = ObjectId::new();
    state
        .db
        .operation_knowledge_documents()
        .insert_one(
            OperationKnowledgeDocument {
                id: Some(document_id),
                workspace_id: WORKSPACE.to_string(),
                account_id: Some(ACCOUNT.to_string()),
                domain: "user_operations".to_string(),
                source_type: "manual".to_string(),
                source_name: Some("HC-028 synthetic policy".to_string()),
                title: "企业版退款事实源".to_string(),
                summary: Some("企业版订阅退款规则的可核验测试原文".to_string()),
                catalog_summary: None,
                routing_map: vec![],
                risk_notes: vec![],
                product_tags: vec!["企业版".to_string()],
                business_topics: vec!["退款".to_string()],
                raw_content: Some(
                    "企业版订阅自首次付款之日起七个自然日内可申请全额退款；超过七日后不支持无理由退款。退款申请需提供订单号，由运营确认后原路退回。"
                        .to_string(),
                ),
                content_hash: Some("hc028-synthetic-source-v1".to_string()),
                line_index: vec![],
                section_index: vec![],
                status: "active".to_string(),
                version: 1,
                created_at: now,
                updated_at: now,
                catalog_summary_persisted: None,
                catalog_version: None,
                catalog_desired_generation: 0,
                catalog_applied_generation: 0,
            },
            None,
        )
        .await
        .expect("seed HC-028 source document");

    let chunk_id = ObjectId::new();
    state
        .db
        .operation_knowledge_chunks()
        .insert_one(
            OperationKnowledgeChunk {
                id: Some(chunk_id),
                workspace_id: WORKSPACE.to_string(),
                account_id: Some(ACCOUNT.to_string()),
                document_id: Some(document_id),
                domain: "user_operations".to_string(),
                knowledge_type: Some("policy".to_string()),
                title: "企业版退款规则待补出处".to_string(),
                summary: Some("企业版退款规则尚未绑定原文出处".to_string()),
                body: Some("企业版订阅支持在限定期限内退款。".to_string()),
                source_quote: None,
                source_anchors: vec![],
                integrity_status: Some("needs_review".to_string()),
                status: "draft".to_string(),
                created_at: now,
                updated_at: now,
                ..Default::default()
            },
            None,
        )
        .await
        .expect("seed HC-028 repairable chunk");
    chunk_id
}

async fn json_response(response: Response, expected: StatusCode, label: &str) -> Value {
    let status = response.status();
    let body = response.text().await.expect("read HC-028 response body");
    assert_eq!(status, expected, "{label} failed: {body}");
    serde_json::from_str(&body).unwrap_or_else(|error| panic!("decode {label}: {error}; {body}"))
}

fn binding_for_card(digest: &Value, card: &Value) -> Value {
    json!({
        "accountId": ACCOUNT,
        "reportId": digest["reportId"],
        "reportDate": digest["reportDate"],
        "reportGeneration": digest["currentGeneration"],
        "reportHash": digest["reportHash"],
        "selectedCards": [{
            "cardId": card["cardId"],
            "cardHash": card["cardHash"],
        }],
    })
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB, production provider config, and outbound real LLM"]
async fn real_digest_chat_task_worker_produces_committed_repair_artifact() {
    let mut evidence = CapabilityEvidence::new("hc028_real_digest_task_worker");
    evidence.attempted();

    let (real_llm, provider_id, configured_model) = load_active_real_provider().await;
    let app = TestApp::start_repl_set().await;
    let mcp = wiremock::MockServer::start().await;
    let mut state = rebuild_app_state_with_real_llm(&app, real_llm, mcp.uri());
    // TestApp has no runtime registry, so generate_agent_json uses this field for its audit
    // model identity. Keep evidence aligned with the selected real provider instead of the
    // fixture's "test-model" placeholder.
    state.config.openai_model = configured_model.clone();
    let chunk_id = seed_repairable_chunk(&state).await;
    let api = start_api(&state).await;
    let client = reqwest::Client::new();

    let digest = json_response(
        client
            .post(format!("{}/knowledge/digest/regenerate", api.base_url))
            .header(reqwest::header::COOKIE, &api.cookie)
            .json(&json!({ "accountId": ACCOUNT, "force": true }))
            .send()
            .await
            .expect("call HC-028 digest regenerate"),
        StatusCode::OK,
        "real digest regenerate",
    )
    .await;
    assert_eq!(
        digest["status"], "ok",
        "digest must commit a successful snapshot"
    );
    let cards = digest["cards"]
        .as_array()
        .expect("digest cards must be an array");
    let chunk_id_hex = chunk_id.to_hex();
    let card = cards
        .iter()
        .find(|card| {
            card["suggestedAction"] == "fix_chunk"
                && card["targetRefs"].as_array().is_some_and(|refs| {
                    refs.iter()
                        .any(|target| target["kind"] == "chunk" && target["id"] == chunk_id_hex)
                })
        })
        .unwrap_or_else(|| {
            panic!(
                "real digest must produce a fix_chunk card bound to seeded chunk; cards={cards:?}"
            )
        })
        .clone();
    let binding = binding_for_card(&digest, &card);

    let chat = json_response(
        client
            .post(format!("{}/operation-knowledge/chat", api.base_url))
            .header(reqwest::header::COOKIE, &api.cookie)
            .json(&json!({
                "sessionId": SESSION_ID,
                "accountId": ACCOUNT,
                "operatorId": "hc028-real-operator",
                "content": "请按我选中的日报卡片生成并封印修复任务，完成后交给运营确认。",
                "attachments": [],
                "digestSelection": binding,
            }))
            .send()
            .await
            .expect("call HC-028 chat dispatch"),
        StatusCode::OK,
        "real chat dispatch",
    )
    .await;
    assert_eq!(chat["intent"], "digest_action");
    assert_eq!(chat["plannedSteps"].as_array().map(Vec::len), Some(1));
    let candidate_hash = chat["candidateHash"]
        .as_str()
        .filter(|value| !value.is_empty())
        .expect("chat must return a sealed candidate hash");
    let source_turn_index = chat["turnIndex"]
        .as_i64()
        .expect("chat must return source turn index");

    let task_response = json_response(
        client
            .post(format!("{}/knowledge/chat/tasks", api.base_url))
            .header(reqwest::header::COOKIE, &api.cookie)
            .json(&json!({
                "sessionId": SESSION_ID,
                "accountId": ACCOUNT,
                "operatorId": "hc028-real-operator",
                "digestSelection": binding,
                "sourceTurnIndex": source_turn_index,
                "candidateHash": candidate_hash,
                "cardIds": [card["cardId"].clone()],
                "plannedSteps": chat["plannedSteps"].clone(),
            }))
            .send()
            .await
            .expect("create HC-028 task"),
        StatusCode::OK,
        "create sealed task",
    )
    .await;
    let task_id = ObjectId::parse_str(
        task_response["taskId"]
            .as_str()
            .expect("taskId must be returned"),
    )
    .expect("taskId must be an ObjectId");

    wechatagent::knowledge_task::tick_once(&state, &state.chat_progress_bus)
        .await
        .expect("run real HC-028 worker task");
    let task = state
        .db
        .knowledge_chat_tasks()
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("read completed HC-028 task")
        .expect("HC-028 task must exist");
    assert_eq!(
        task.status, "completed",
        "task must not report false success/failure"
    );
    assert_eq!(task.completed_steps.len(), 1);
    let outcome = &task.completed_steps[0];
    assert_eq!(outcome.get_str("status").ok(), Some("committed"));
    assert_eq!(outcome.get_str("chunkId").ok(), Some(chunk_id_hex.as_str()));
    let repair_draft = outcome
        .get_document("repairDraft")
        .expect("committed fix_chunk must persist repairDraft");
    let patch = repair_draft
        .get_document("patch")
        .expect("repairDraft must contain a patch object");
    assert!(
        !patch.is_empty(),
        "real worker repair patch must be non-empty"
    );

    let unchanged = state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .expect("read source chunk after worker")
        .expect("source chunk must remain");
    assert_eq!(unchanged.status, "draft");
    assert_eq!(unchanged.integrity_status.as_deref(), Some("needs_review"));
    assert!(
        unchanged.source_quote.is_none(),
        "repair proposal must not auto-apply"
    );

    let mut calls = state
        .db
        .llm_call_logs()
        .find(
            doc! {
                "workspace_id": WORKSPACE,
                "account_id": ACCOUNT,
                "prompt_key": { "$in": [
                    "knowledge.digest.compose",
                    "knowledge.chunk.repair.propose",
                ] },
            },
            None,
        )
        .await
        .expect("read HC-028 LLM call evidence");
    let mut prompt_keys = Vec::new();
    while let Some(call) = calls.try_next().await.expect("scan HC-028 LLM logs") {
        assert_eq!(call.status, "success", "real model call must succeed");
        assert!(
            !call.model.trim().is_empty(),
            "real model identity must be recorded"
        );
        prompt_keys.push(call.prompt_key);
    }
    prompt_keys.sort();
    assert_eq!(
        prompt_keys,
        [
            "knowledge.chunk.repair.propose".to_string(),
            "knowledge.digest.compose".to_string(),
        ],
        "both real-model stages must have one successful audit row"
    );
    assert!(
        mcp.received_requests()
            .await
            .expect("read MCP request ledger")
            .is_empty(),
        "Digest/Chat/Task repair flow must not contact MCP"
    );

    evidence.observe_llm_calls(prompt_keys.len());
    evidence.branch("cookie_digest_chat_seal_worker_repair");
    evidence.detail("provider_id", provider_id);
    evidence.detail("configured_model", configured_model);
    evidence.detail("digest_card_count", cards.len());
    evidence.detail("task_id", task_id.to_hex());
    evidence.detail("repair_patch_field_count", patch.len());
    evidence.pass(6, 18);

    api.server.abort();
    app.cleanup().await;
}

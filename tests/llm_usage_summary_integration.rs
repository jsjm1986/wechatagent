//! HC-007 / SR-156: LLM usage summary must cover all retained logs, not only
//! the limited detail sample returned in the same response.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::DateTime;
use tokio::net::TcpListener;

use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::LlmCallLog;
use wechatagent::routes::api_router;

fn log(workspace_id: &str, account_id: &str, sequence: i64) -> LlmCallLog {
    LlmCallLog {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: Some(account_id.to_string()),
        contact_wxid: Some(format!("contact-{sequence}")),
        run_id: Some(format!("run-{sequence}")),
        run_mode: "live".to_string(),
        prompt_key: "user.reply.system".to_string(),
        model: "test-model".to_string(),
        status: "success".to_string(),
        latency_ms: sequence,
        queue_wait_ms: 0,
        provider_latency_ms: 0,
        priority: "foreground".to_string(),
        prompt_tokens: 7,
        completion_tokens: 3,
        total_tokens: 10,
        prompt_cache_hit_tokens: 4,
        prompt_cache_miss_tokens: 6,
        usage_known: true,
        error: None,
        retry_count: 0,
        final_status: Some("success".to_string()),
        created_at: DateTime::from_millis(1_700_000_000_000 + sequence),
    }
}

#[tokio::test]
#[ignore = "requires mongo"]
async fn retained_summary_is_not_limited_by_detail_sample() {
    let app = common::TestApp::start().await;
    let workspace = "default";
    let account = "sr156-account";

    let mut logs: Vec<LlmCallLog> = (0..101)
        .map(|sequence| log(workspace, account, sequence))
        .collect();
    // A different account must not leak into the selected account summary.
    logs.push(log(workspace, "sr156-other-account", 999));
    app.state
        .db
        .llm_call_logs()
        .insert_many(logs, None)
        .await
        .expect("insert LLM usage fixtures");

    let admin = AdminUser {
        user_id: "sr156-admin".to_string(),
        username: "sr156-admin".to_string(),
        password_hash: "unused".to_string(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![workspace.to_string()],
        default_workspace: Some(workspace.to_string()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&admin, None)
        .await
        .expect("insert admin");
    let session = create_session(&app.state.db, &admin, 1, workspace)
        .await
        .expect("create session");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test API");
    let address = listener.local_addr().expect("test API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve LLM usage API");
    });

    let response = reqwest::Client::new()
        .get(format!(
            "http://{address}/api/llm-usage?accountId={account}&limit=100"
        ))
        .header(
            reqwest::header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        )
        .send()
        .await
        .expect("request LLM usage");
    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await.expect("decode LLM usage response");

    assert_eq!(body["summary"]["totalCalls"], 101);
    assert_eq!(body["summary"]["totalTokens"], 1_010);
    assert_eq!(body["summary"]["promptCacheHitTokens"], 404);
    assert_eq!(body["summary"]["promptCacheMissTokens"], 606);
    assert_eq!(body["summary"]["knownUsageCalls"], 101);
    assert_eq!(body["summary"]["unknownUsageCalls"], 0);
    assert_eq!(body["summary"]["usageComplete"], true);
    assert_eq!(body["items"].as_array().map(Vec::len), Some(100));
    assert_eq!(body["itemsReturned"], 100);
    assert_eq!(body["itemsLimit"], 100);
    assert_eq!(body["itemsTruncated"], true);
    assert_eq!(body["window"]["kind"], "retained_logs");
    assert!(body["asOf"].is_string());
    assert!(body["window"]["start"].is_string());
    assert!(body["window"]["end"].is_string());

    server.abort();
    let _ = server.await;
    app.cleanup().await;
}

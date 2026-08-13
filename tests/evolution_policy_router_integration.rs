//! HC-017 real-router evidence for manual-only release policy and complete aggregation.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, DateTime, Document};
use reqwest::StatusCode;
use tokio::net::TcpListener;

use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::{Experiment, Proposal};
use wechatagent::routes::api_router;

const WORKSPACE: &str = "hc017-workspace";
const FOREIGN_WORKSPACE: &str = "hc017-foreign";
const ACCOUNT: &str = "hc017-account";

fn experiment(sequence: usize, workspace_id: &str, started_at: DateTime) -> Experiment {
    Experiment {
        id: None,
        experiment_id: format!("hc017-exp-{workspace_id}-{sequence}"),
        workspace_id: workspace_id.to_string(),
        account_id: ACCOUNT.to_string(),
        status: "awaiting_admin".to_string(),
        window_hours: 24,
        started_at,
        updated_at: started_at,
        finished_at: None,
        cohort_threshold_run_ids: vec![],
        cohort_prompt_run_ids: vec![],
        budget_used_tokens: 0,
        budget_used_calls: 0,
        proposals_count: 1,
        proposals_eligible_count: 1,
    }
}

fn proposal(experiment: &Experiment) -> Proposal {
    Proposal {
        id: None,
        experiment_id: experiment.experiment_id.clone(),
        workspace_id: experiment.workspace_id.clone(),
        account_id: experiment.account_id.clone(),
        proposal_kind: "threshold".to_string(),
        status: "released".to_string(),
        gate_key: Some("fact_risk_block".to_string()),
        current_value: Some(6.0),
        proposed_value: Some(5.5),
        cohort_notes: Document::new(),
        proposed_template_key: None,
        proposed_section: None,
        diff_summary: None,
        diff_snippet: None,
        critic_reasoning: None,
        expected_improvement_on: vec![],
        risk_note: None,
        base_revision: Some("threshold-v1:baseline:4018000000000000".to_string()),
        released_revision: Some("threshold-v1:artifact:4016000000000000".to_string()),
        previous_prompt_version: None,
        eval_metrics: Document::new(),
        eval_replays_completed: 1,
        eval_replays_failed: 0,
        significance_passed: Some(true),
        failure_reason: None,
        released_at: Some(experiment.started_at),
        released_by: Some("hc017-admin".to_string()),
        rolled_back_at: None,
        rolled_back_by: None,
        created_at: experiment.started_at,
        updated_at: experiment.started_at,
    }
}

#[tokio::test]
#[ignore = "requires mongo"]
async fn manual_policy_and_complete_window_hold_through_real_router() {
    let app = common::TestApp::start().await;
    let now = DateTime::now();
    let in_window = DateTime::from_millis(now.timestamp_millis() - 60_000);
    let outside_window = DateTime::from_millis(now.timestamp_millis() - 8 * 24 * 60 * 60 * 1000);

    let current: Vec<Experiment> = (0..25)
        .map(|sequence| experiment(sequence, WORKSPACE, in_window))
        .collect();
    let current_proposals: Vec<Proposal> = current.iter().map(proposal).collect();
    let old = experiment(100, WORKSPACE, outside_window);
    let old_proposal = proposal(&old);
    let foreign = experiment(200, FOREIGN_WORKSPACE, in_window);
    let foreign_proposal = proposal(&foreign);

    let mut experiments = current;
    experiments.push(old);
    experiments.push(foreign);
    app.state
        .db
        .experiments()
        .insert_many(experiments, None)
        .await
        .expect("insert experiments");
    let mut proposals = current_proposals;
    proposals.push(old_proposal);
    proposals.push(foreign_proposal);
    app.state
        .db
        .proposals()
        .insert_many(proposals, None)
        .await
        .expect("insert proposals");

    let admin = AdminUser {
        user_id: "hc017-admin".to_string(),
        username: "hc017-admin".to_string(),
        password_hash: "unused".to_string(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![WORKSPACE.to_string()],
        default_workspace: Some(WORKSPACE.to_string()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&admin, None)
        .await
        .expect("insert admin");
    let session = create_session(&app.state.db, &admin, 1, WORKSPACE)
        .await
        .expect("create session");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind API");
    let address = listener.local_addr().expect("API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve API");
    });
    let client = reqwest::Client::new();
    let cookie = format!("{SESSION_COOKIE_NAME}={}", session.session_id);

    let enable = client
        .put(format!("http://{address}/api/evolution/runtime-flag"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "enabled": true,
            "rolloutPercent": 100,
            "thresholdAutoReleaseEnabled": true
        }))
        .send()
        .await
        .expect("request auto-release enable");
    assert_eq!(enable.status(), StatusCode::BAD_REQUEST);
    let enable_body: serde_json::Value = enable.json().await.expect("decode rejection");
    assert!(enable_body["error"]
        .as_str()
        .is_some_and(|error| error.contains("human-release policy")));
    assert_eq!(
        app.state
            .db
            .evolution_runtime_flags()
            .count_documents(doc! { "workspace_id": WORKSPACE }, None)
            .await
            .expect("count runtime flags"),
        0,
        "a rejected auto-release request must not persist a flag"
    );

    // 缺陷 #6：合法灰度更新且请求体自报伪造的 updatedBy —— 落库审计身份必须是
    // 服务端会话身份（hc017-admin），不采信请求体。
    let spoofed = client
        .put(format!("http://{address}/api/evolution/runtime-flag"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "enabled": true,
            "rolloutPercent": 25,
            "updatedBy": "spoofed-operator"
        }))
        .send()
        .await
        .expect("request runtime flag update");
    assert_eq!(spoofed.status(), StatusCode::OK);
    let saved = app
        .state
        .db
        .evolution_runtime_flags()
        .find_one(doc! { "workspace_id": WORKSPACE }, None)
        .await
        .expect("read saved runtime flag")
        .expect("runtime flag persisted");
    assert_eq!(
        saved.updated_by.as_deref(),
        Some("hc017-admin"),
        "updated_by 必须来自服务端会话身份，而非请求体自报值"
    );
    assert_eq!(saved.rollout_percent, 25);
    assert!(saved.enabled);

    let response = client
        .get(format!(
            "http://{address}/api/evolution/experiments?limit=5"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("request experiments");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("decode experiments");
    assert_eq!(body["items"].as_array().map(Vec::len), Some(5));
    assert_eq!(body["aggregate7d"]["experiments"], 25);
    assert_eq!(body["aggregate7d"]["proposals"], 25);
    assert_eq!(body["aggregate7d"]["released"], 25);
    assert_eq!(body["aggregate7d"]["significancePassRate"], 1.0);
    assert_eq!(body["aggregate7d"]["coverage"]["complete"], true);
    assert_eq!(body["aggregate7d"]["coverage"]["windowHours"], 168);
    assert_eq!(
        body["aggregate7d"]["coverage"]["source"],
        "server_time_window"
    );
    assert_eq!(body["aggregate7d"]["coverage"]["experimentsScanned"], 25);

    server.abort();
    let _ = server.await;
    app.cleanup().await;
}

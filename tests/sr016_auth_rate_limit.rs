//! SR-016: login and token share one privacy-preserving rate limit.
#![cfg(test)]

mod common;

use std::{net::SocketAddr, sync::Arc};

use axum::Router;
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::{
    auth::{jwt::JwtKeys, rate_limit::AuthRateLimiter, session::bootstrap_admin_if_needed},
    routes::api_router,
};

use crate::common::TestApp;

const USERNAME: &str = "sr016-admin";
const PASSWORD: &str = "sr016-correct-password";
const PRIVATE_PEM: &str = include_str!("fixtures/jwt_test_private.pem");
const PUBLIC_PEM: &str = include_str!("fixtures/jwt_test_public.pem");

#[tokio::test]
#[ignore]
async fn login_and_token_share_limit_and_write_redacted_audit() {
    let app = TestApp::start().await;
    bootstrap_admin_if_needed(
        &app.state.db,
        Some(USERNAME),
        Some(PASSWORD),
        Some(&app.state.config.default_workspace_id),
    )
    .await
    .expect("bootstrap SR-016 admin");

    let mut state = app.state.clone();
    state.config.jwt_enabled = true;
    state.config.jwt_private_key_pem = Some(PRIVATE_PEM.to_string());
    state.config.jwt_public_key_pem = Some(PUBLIC_PEM.to_string());
    state.jwt_keys = Some(Arc::new(
        JwtKeys::from_config(&state.config).expect("load SR-016 JWT keys"),
    ));
    state.auth_rate_limiter = Arc::new(AuthRateLimiter::new(300, 2, 2, 10));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind SR-016 API");
    let address = listener.local_addr().expect("SR-016 API address");
    let router = Router::new()
        .nest("/api", api_router(state.clone()))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("serve SR-016 API");
    });

    let client = reqwest::Client::new();
    let base = format!("http://{address}/api/auth");
    let first = client
        .post(format!("{base}/login"))
        .json(&serde_json::json!({ "username": USERNAME, "password": "wrong-one" }))
        .send()
        .await
        .expect("first login attempt");
    let second = client
        .post(format!("{base}/token"))
        .json(&serde_json::json!({ "username": USERNAME, "password": "wrong-two" }))
        .send()
        .await
        .expect("second token attempt");
    let third = client
        .post(format!("{base}/login"))
        .json(&serde_json::json!({ "username": USERNAME, "password": PASSWORD }))
        .send()
        .await
        .expect("third login attempt");
    let third_status = third.status();
    let retry_after_present = third.headers().contains_key("retry-after");
    let third_body: serde_json::Value = third.json().await.expect("429 JSON body");

    let mut cursor = state
        .db
        .raw()
        .collection::<Document>("auth_security_events")
        .find(doc! {}, None)
        .await
        .expect("read authentication audit");
    let mut audits = Vec::new();
    while let Some(document) = cursor.try_next().await.expect("read audit row") {
        audits.push(document);
    }
    let rendered = format!("{audits:?}");
    let outcomes: Vec<String> = audits
        .iter()
        .filter_map(|document| document.get_str("outcome").ok().map(str::to_owned))
        .collect();
    let entrypoints: Vec<String> = audits
        .iter()
        .filter_map(|document| document.get_str("entrypoint").ok().map(str::to_owned))
        .collect();
    let forbidden_fields_absent = audits.iter().all(|document| {
        ["username", "password", "token", "ip", "client_address"]
            .iter()
            .all(|field| !document.contains_key(*field))
    });
    let retention_is_ninety_days = audits.iter().all(|document| {
        let Ok(created_at) = document.get_datetime("created_at") else {
            return false;
        };
        let Ok(expires_at) = document.get_datetime("expires_at") else {
            return false;
        };
        expires_at.timestamp_millis() - created_at.timestamp_millis()
            == 90_i64 * 24 * 60 * 60 * 1000
    });

    server.abort();
    let first_status = first.status();
    let second_status = second.status();
    app.cleanup().await;

    assert_eq!(first_status, StatusCode::UNAUTHORIZED);
    assert_eq!(second_status, StatusCode::UNAUTHORIZED);
    assert_eq!(third_status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        third_body,
        serde_json::json!({ "error": "auth_rate_limited" })
    );
    assert!(retry_after_present);
    assert_eq!(audits.len(), 3, "audits={audits:?}");
    assert_eq!(
        outcomes,
        ["invalid_credentials", "invalid_credentials", "rate_limited"]
    );
    assert_eq!(entrypoints, ["login", "token", "login"]);
    assert!(forbidden_fields_absent, "audits={audits:?}");
    assert!(retention_is_ninety_days, "audits={audits:?}");
    assert!(!rendered.contains(USERNAME), "audits={audits:?}");
    assert!(!rendered.contains("127.0.0.1"), "audits={audits:?}");
    assert!(!rendered.contains(PASSWORD), "audits={audits:?}");
}

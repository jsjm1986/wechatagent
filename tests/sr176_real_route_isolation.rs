//! SR-176: tenant-isolation evidence must cross the real auth middleware and Axum router.
#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDateTime, Document};
use reqwest::StatusCode;
use std::sync::Arc;
use tokio::net::TcpListener;
use wechatagent::auth::jwt::{issue_jwt, JwtKeys};
use wechatagent::auth::session::{create_session, lookup_session, AuthError};
use wechatagent::auth::{AdminSession, AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::{AgentStatus, Contact, Product};
use wechatagent::routes::api_router;

use crate::common::TestApp;

const WORKSPACE_A: &str = "sr176-workspace-a";
const WORKSPACE_B: &str = "sr176-workspace-b";

#[derive(Debug)]
struct RouteEvidence {
    foreign_contact_status: StatusCode,
    own_contact_status: StatusCode,
    own_contact_wxid: String,
    foreign_product_status: StatusCode,
    foreign_product_name_after: String,
    own_product_status: StatusCode,
    own_product_name_after: String,
    same_id_foreign_product_name_after: String,
    expired_lookup_rejected: bool,
    expired_cookie_status: StatusCode,
    valid_cookie_status: StatusCode,
    revoked_cookie_status: StatusCode,
}

fn admin_user() -> AdminUser {
    AdminUser {
        user_id: "sr176-admin".into(),
        username: "sr176-admin".into(),
        password_hash: "unused".into(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![WORKSPACE_A.into()],
        default_workspace: Some(WORKSPACE_A.into()),
    }
}

fn contact(workspace_id: &str, account_id: &str, wxid: &str) -> Contact {
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        wxid: wxid.into(),
        nickname: Some(format!("{workspace_id}-contact")),
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
        created_at: BsonDateTime::now(),
        updated_at: BsonDateTime::now(),
    }
}

fn product(workspace_id: &str, product_id: &str, name: &str) -> Product {
    let now = BsonDateTime::now();
    Product {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        product_id: product_id.into(),
        name: name.into(),
        price: Some(1_000),
        currency: Some("CNY".into()),
        sku: None,
        status: "active".into(),
        summary: None,
        attributes: Document::new(),
        created_at: now,
        updated_at: now,
    }
}

async fn start_api(app: &TestApp) -> anyhow::Result<(String, String, tokio::task::JoinHandle<()>)> {
    let user = admin_user();
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&user, None)
        .await?;
    let session = create_session(&app.state.db, &user, 1, WORKSPACE_A).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve SR-176 API");
    });
    Ok((
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    ))
}

async fn exercise_real_routes(app: &TestApp) -> anyhow::Result<RouteEvidence> {
    let own_contact = contact(WORKSPACE_A, "sr176-account-a", "sr176-wxid-a");
    let foreign_contact = contact(WORKSPACE_B, "sr176-account-b", "sr176-wxid-b");
    let own_contact_id = own_contact.id.expect("own contact id");
    let foreign_contact_id = foreign_contact.id.expect("foreign contact id");
    app.state
        .db
        .contacts()
        .insert_many(vec![own_contact, foreign_contact], None)
        .await?;

    app.state
        .db
        .products()
        .insert_many(
            vec![
                product(WORKSPACE_A, "shared-product", "A original"),
                product(WORKSPACE_B, "shared-product", "B same-id original"),
                product(WORKSPACE_B, "foreign-only", "B foreign original"),
            ],
            None,
        )
        .await?;

    let expired_session = AdminSession {
        session_id: "sr176-expired-session".into(),
        admin_user_id: "sr176-expired-admin".into(),
        username: "sr176-expired-admin".into(),
        created_at: chrono::Utc::now() - chrono::Duration::hours(2),
        expires_at: chrono::Utc::now() - chrono::Duration::hours(1),
        current_workspace: Some(WORKSPACE_A.into()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminSession>("admin_sessions")
        .insert_one(&expired_session, None)
        .await?;
    let expired_lookup_rejected = matches!(
        lookup_session(&app.state.db, &expired_session.session_id).await,
        Err(AuthError::SessionExpired)
    );

    let (base_url, valid_cookie, _server) = start_api(app).await?;
    let client = reqwest::Client::new();

    let foreign_contact_response = client
        .get(format!("{base_url}/contacts/{foreign_contact_id}"))
        .header(reqwest::header::COOKIE, &valid_cookie)
        .send()
        .await?;
    let foreign_contact_status = foreign_contact_response.status();

    let own_contact_response = client
        .get(format!("{base_url}/contacts/{own_contact_id}"))
        .header(reqwest::header::COOKIE, &valid_cookie)
        .send()
        .await?;
    let own_contact_status = own_contact_response.status();
    let own_contact_body: serde_json::Value = own_contact_response.json().await?;
    let own_contact_wxid = own_contact_body["item"]["wxid"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    let update_body = |name: &str| {
        serde_json::json!({
            "name": name,
            "price": 2_000,
            "currency": "CNY",
            "sku": "sr176-sku",
            "summary": "SR-176 real route update",
            "attributes": { "source": "sr176" }
        })
    };
    let foreign_product_response = client
        .put(format!("{base_url}/products/foreign-only"))
        .header(reqwest::header::COOKIE, &valid_cookie)
        .json(&update_body("A must not overwrite B"))
        .send()
        .await?;
    let foreign_product_status = foreign_product_response.status();

    let own_product_response = client
        .put(format!("{base_url}/products/shared-product"))
        .header(reqwest::header::COOKIE, &valid_cookie)
        .json(&update_body("A updated through router"))
        .send()
        .await?;
    let own_product_status = own_product_response.status();

    let foreign_product_name_after = app
        .state
        .db
        .products()
        .find_one(
            doc! { "workspace_id": WORKSPACE_B, "product_id": "foreign-only" },
            None,
        )
        .await?
        .expect("foreign product remains")
        .name;
    let own_product_name_after = app
        .state
        .db
        .products()
        .find_one(
            doc! { "workspace_id": WORKSPACE_A, "product_id": "shared-product" },
            None,
        )
        .await?
        .expect("own product remains")
        .name;
    let same_id_foreign_product_name_after = app
        .state
        .db
        .products()
        .find_one(
            doc! { "workspace_id": WORKSPACE_B, "product_id": "shared-product" },
            None,
        )
        .await?
        .expect("same-id foreign product remains")
        .name;

    let expired_cookie_status = client
        .get(format!("{base_url}/auth/me"))
        .header(
            reqwest::header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", expired_session.session_id),
        )
        .send()
        .await?
        .status();
    let valid_cookie_status = client
        .get(format!("{base_url}/auth/me"))
        .header(reqwest::header::COOKIE, &valid_cookie)
        .send()
        .await?
        .status();

    // Revoke workspace A after the session was issued. Middleware must read the
    // authoritative AdminUser ACL on every request instead of trusting the
    // workspace snapshot stored in the cookie session.
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .update_one(
            doc! { "user_id": "sr176-admin" },
            doc! { "$set": {
                "workspaces": [],
                "default_workspace": mongodb::bson::Bson::Null,
            } },
            None,
        )
        .await?;
    let revoked_cookie_status = client
        .get(format!("{base_url}/auth/me"))
        .header(reqwest::header::COOKIE, valid_cookie)
        .send()
        .await?
        .status();

    Ok(RouteEvidence {
        foreign_contact_status,
        own_contact_status,
        own_contact_wxid,
        foreign_product_status,
        foreign_product_name_after,
        own_product_status,
        own_product_name_after,
        same_id_foreign_product_name_after,
        expired_lookup_rejected,
        expired_cookie_status,
        valid_cookie_status,
        revoked_cookie_status,
    })
}

#[tokio::test]
#[ignore]
async fn sr176_real_router_enforces_read_write_and_expired_session_boundaries() {
    let app = TestApp::start().await;
    let result = exercise_real_routes(&app).await;

    // Drop the random external-Mongo database before assertions so a failed
    // boundary leaves no test data behind.
    app.cleanup().await;

    let evidence = result.expect("exercise SR-176 real router boundaries");
    assert_eq!(evidence.foreign_contact_status, StatusCode::NOT_FOUND);
    assert_eq!(evidence.own_contact_status, StatusCode::OK);
    assert_eq!(evidence.own_contact_wxid, "sr176-wxid-a");

    assert_eq!(evidence.foreign_product_status, StatusCode::NOT_FOUND);
    assert_eq!(evidence.foreign_product_name_after, "B foreign original");
    assert_eq!(evidence.own_product_status, StatusCode::OK);
    assert_eq!(evidence.own_product_name_after, "A updated through router");
    assert_eq!(
        evidence.same_id_foreign_product_name_after,
        "B same-id original"
    );

    assert!(evidence.expired_lookup_rejected);
    assert_eq!(evidence.expired_cookie_status, StatusCode::UNAUTHORIZED);
    assert_eq!(evidence.valid_cookie_status, StatusCode::OK);
    assert_eq!(evidence.revoked_cookie_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn bearer_jwt_is_rejected_immediately_after_last_workspace_is_revoked() {
    let app = TestApp::start().await;
    let user = AdminUser {
        user_id: "sr176-jwt-admin".into(),
        username: "sr176-jwt-admin".into(),
        password_hash: "unused".into(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![WORKSPACE_A.into()],
        default_workspace: Some(WORKSPACE_A.into()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&user, None)
        .await
        .expect("seed JWT admin");

    let mut state = app.state.clone();
    state.config.jwt_enabled = true;
    state.config.jwt_private_key_pem =
        Some(include_str!("fixtures/jwt_test_private.pem").to_string());
    state.config.jwt_public_key_pem =
        Some(include_str!("fixtures/jwt_test_public.pem").to_string());
    let keys = Arc::new(JwtKeys::from_config(&state.config).expect("load JWT test keys"));
    let token = issue_jwt(&keys, &user.user_id, &user.username, WORKSPACE_A).expect("issue JWT");
    state.jwt_keys = Some(keys);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind JWT test API");
    let address = listener.local_addr().expect("JWT test API address");
    let router = Router::new()
        .nest("/api", api_router(state.clone()))
        .with_state(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve JWT test API");
    });
    let client = reqwest::Client::new();
    let me_url = format!("http://{address}/api/auth/me");
    let status_before = client
        .get(&me_url)
        .bearer_auth(&token)
        .send()
        .await
        .expect("request with authorized JWT")
        .status();

    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .update_one(
            doc! { "user_id": &user.user_id },
            doc! { "$set": {
                "workspaces": [],
                "default_workspace": mongodb::bson::Bson::Null,
            } },
            None,
        )
        .await
        .expect("revoke JWT admin ACL");
    let status_after = client
        .get(&me_url)
        .bearer_auth(&token)
        .send()
        .await
        .expect("request with revoked JWT")
        .status();

    server.abort();
    app.cleanup().await;
    assert_eq!(status_before, StatusCode::OK);
    assert_eq!(status_after, StatusCode::UNAUTHORIZED);
}

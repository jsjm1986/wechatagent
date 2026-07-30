//! SR-132 redline: the review queue is a server-owned, scoped projection.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::session::{authenticate, bootstrap_admin_if_needed, create_session};
use wechatagent::auth::SESSION_COOKIE_NAME;
use wechatagent::models::{OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::api_router;

use crate::common::TestApp;

async fn start_api(
    app: &TestApp,
    workspace_id: &str,
) -> (String, String, tokio::task::JoinHandle<()>) {
    bootstrap_admin_if_needed(
        &app.state.db,
        Some("sr132_admin"),
        Some("sr132-test-password"),
        Some(workspace_id),
    )
    .await
    .expect("bootstrap admin");
    let admin = authenticate(&app.state.db, "sr132_admin", "sr132-test-password")
        .await
        .expect("authenticate admin");
    let session = create_session(&app.state.db, &admin, 1, workspace_id)
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
        axum::serve(listener, router).await.expect("serve test API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

fn chunk(
    workspace_id: &str,
    id: ObjectId,
    title: &str,
    status: &str,
    integrity_status: &str,
    topic: &str,
    sourced: bool,
) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: workspace_id.to_string(),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        status: status.to_string(),
        integrity_status: Some(integrity_status.to_string()),
        business_topics: vec![topic.to_string()],
        source_quote: sourced.then(|| "authoritative quote".to_string()),
        source_anchors: sourced
            .then(|| vec![doc! { "startLine": 1_i32 }])
            .unwrap_or_default(),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
        ..Default::default()
    }
}

fn ids(body: &serde_json::Value) -> Vec<String> {
    let mut values = body["items"]
        .as_array()
        .expect("items array")
        .iter()
        .filter_map(|item| item["id"].as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    values.sort();
    values
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn review_projection_is_reachable_scoped_and_dimension_filtered() {
    let app = TestApp::start().await;
    let workspace_id = format!("sr132-{}", ObjectId::new().to_hex());
    let foreign_workspace = format!("foreign-{}", ObjectId::new().to_hex());
    let (api, cookie, server) = start_api(&app, &workspace_id).await;

    let pricing_draft_id = ObjectId::new();
    let capability_draft_id = ObjectId::new();
    let pricing_rejected_id = ObjectId::new();
    let archived_id = ObjectId::new();
    let foreign_id = ObjectId::new();

    let mut pricing_draft = chunk(
        &workspace_id,
        pricing_draft_id,
        "pricing draft",
        "draft",
        "needs_review",
        "pricing",
        false,
    );
    pricing_draft.business_topics = vec!["PRICING".to_string()];
    pricing_draft.related_chunks = Some(vec![RelatedRef {
        chunk_id: archived_id.to_hex(),
        kind: "references".to_string(),
        note: None,
    }]);
    let rows = vec![
        pricing_draft,
        chunk(
            &workspace_id,
            capability_draft_id,
            "capability draft",
            "draft",
            "needs_review",
            "CAPABILITY",
            true,
        ),
        chunk(
            &workspace_id,
            pricing_rejected_id,
            "rejected pricing",
            "active",
            "rejected",
            "pricing",
            true,
        ),
        chunk(
            &workspace_id,
            archived_id,
            "archived pricing",
            "archived",
            "needs_review",
            "pricing",
            false,
        ),
        chunk(
            &foreign_workspace,
            foreign_id,
            "foreign pricing",
            "draft",
            "needs_review",
            "pricing",
            false,
        ),
    ];
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many(rows, None)
        .await
        .expect("seed chunks");

    let client = reqwest::Client::new();
    let all_response = client
        .get(format!("{api}/operation-knowledge/review-queue"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("load review queue");
    assert_eq!(all_response.status(), StatusCode::OK);
    let all: serde_json::Value = all_response.json().await.expect("review queue JSON");
    assert_eq!(
        ids(&all),
        {
            let mut expected = vec![
                pricing_draft_id.to_hex(),
                capability_draft_id.to_hex(),
                pricing_rejected_id.to_hex(),
            ];
            expected.sort();
            expected
        },
        "default queue must include reviewable draft/active rows only"
    );
    assert_eq!(all["counts"]["needs_review"], 2);
    assert_eq!(all["counts"]["source_orphan"], 1);
    assert_eq!(all["counts"]["pending_verification"], 1);
    assert_eq!(all["counts"]["contested"], 1);
    assert_eq!(all["counts"]["dependents_pending"], 1);
    assert_eq!(
        all["effectiveFilter"]["lifecycleStatuses"],
        serde_json::json!(["draft", "active"])
    );
    let pricing_draft_row = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == pricing_draft_id.to_hex())
        .expect("pricing draft row");
    assert_eq!(
        pricing_draft_row["reviewCategories"],
        serde_json::json!(["needs_review", "source_orphan", "dependents_pending"]),
        "review facets are intentionally overlapping"
    );

    let pricing_response = client
        .get(format!(
            "{api}/operation-knowledge/review-queue?dimension=pricing"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("load pricing review queue");
    assert_eq!(pricing_response.status(), StatusCode::OK);
    let pricing: serde_json::Value = pricing_response.json().await.unwrap();
    let mut expected_pricing = vec![pricing_draft_id.to_hex(), pricing_rejected_id.to_hex()];
    expected_pricing.sort();
    assert_eq!(ids(&pricing), expected_pricing);
    assert_eq!(pricing["effectiveFilter"]["dimension"]["key"], "pricing");
    assert!(pricing["effectiveFilter"]["dimension"]["topicAliases"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "pricing"));

    let capability_response = client
        .get(format!(
            "{api}/operation-knowledge/review-queue?dimension=capability"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("load capability review queue");
    assert_eq!(capability_response.status(), StatusCode::OK);
    let capability: serde_json::Value = capability_response.json().await.unwrap();
    assert_eq!(ids(&capability), vec![capability_draft_id.to_hex()]);

    let unknown = client
        .get(format!(
            "{api}/operation-knowledge/review-queue?dimension=unknown"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("load unknown dimension");
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);

    server.abort();
    app.cleanup().await;
}

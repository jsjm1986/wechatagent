//! SR-008 regressions for the single-current operations version protocol.
//!
//! The migration/index test uses standalone MongoDB. Lifecycle and concurrency
//! tests use a replica set because every pointer switch is a transaction.

#![cfg(test)]

mod common;

use axum::Router;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::models::{
    OperationDomainConfig, OperationStatePolicy, TaxonomyEntry, TaxonomyValue,
};
use wechatagent::routes::api_router;

use crate::common::TestApp;

fn domain_row(workspace: &str, version: i32, current: bool) -> OperationDomainConfig {
    OperationDomainConfig {
        id: Some(ObjectId::new()),
        workspace_id: workspace.to_string(),
        domain: "user_operations".to_string(),
        name: "SR-008 domain".to_string(),
        goal: "preserve one current".to_string(),
        methodology: "transactional".to_string(),
        workflow: "publish-rollout-rollback".to_string(),
        tool_policy: "safe".to_string(),
        automation_policy: "safe".to_string(),
        review_policy: "manual".to_string(),
        runtime_parameters: Document::new(),
        state_machine: doc! { "states": [] },
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version,
        current_version: current,
        previous_version: (version > 1).then_some(version - 1),
        seeded_by: Some("sr008".to_string()),
        principal_decider: None,
        high_risk_escalation_mode: None,
        ask_human_policy: None,
        assist_mode_enabled: None,
    }
}

fn policy_row(workspace: &str, version: i32, current: bool) -> OperationStatePolicy {
    OperationStatePolicy {
        id: Some(ObjectId::new()),
        workspace_id: workspace.to_string(),
        domain: "user_operations".to_string(),
        state_key: "sr008_state".to_string(),
        allowed: vec!["reply".to_string()],
        forbidden: vec![],
        recommended_pace: Some("normal".to_string()),
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version,
        current_version: current,
        previous_version: (version > 1).then_some(version - 1),
        seeded_by: Some("sr008".to_string()),
    }
}

fn taxonomy_row(workspace: &str, version: i32, current: bool) -> TaxonomyEntry {
    taxonomy_row_for_value(workspace, "sr008_value", version, current)
}

fn taxonomy_row_for_value(
    workspace: &str,
    value_id: &str,
    version: i32,
    current: bool,
) -> TaxonomyEntry {
    TaxonomyEntry {
        id: Some(ObjectId::new()),
        workspace_id: workspace.to_string(),
        scope: "global".to_string(),
        kind: "sr008_kind".to_string(),
        value: TaxonomyValue {
            id: value_id.to_string(),
            display_name: format!("SR-008 value v{version}"),
            description: String::new(),
            aliases: vec![],
            status: "active".to_string(),
            priority_weight: None,
            is_terminal: false,
            is_reactivation_target: false,
        },
        updated_at: DateTime::now(),
        version,
        current_version: current,
        previous_version: (version > 1).then_some(version - 1),
        seeded_by: Some("sr008".to_string()),
    }
}

async fn start_api(
    app: &TestApp,
    workspace: &str,
) -> anyhow::Result<(String, String, tokio::task::JoinHandle<()>)> {
    let user = AdminUser {
        user_id: format!("sr008-user-{}", ObjectId::new().to_hex()),
        username: format!("sr008-admin-{}", ObjectId::new().to_hex()),
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
        .insert_one(&user, None)
        .await?;
    let session = create_session(&app.state.db, &user, 1, workspace).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve SR-008 API");
    });
    Ok((
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    ))
}

async fn current_versions(
    app: &TestApp,
    collection: &str,
    mut scope: Document,
) -> anyhow::Result<Vec<i32>> {
    scope.insert("current_version", true);
    let mut cursor = app
        .state
        .db
        .raw()
        .collection::<Document>(collection)
        .find(scope, None)
        .await?;
    let mut versions = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        versions.push(row.get_i32("version")?);
    }
    versions.sort_unstable();
    Ok(versions)
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn migration_reconciles_all_three_tables_and_unique_indexes_hold() {
    let app = TestApp::start().await;
    let workspace = format!("sr008-migration-{}", ObjectId::new().to_hex());

    for (collection, index) in [
        (
            "operation_domain_configs",
            "uniq_op_domain_ws_domain_current",
        ),
        (
            "operation_state_policies",
            "uniq_op_state_policy_ws_domain_state_current",
        ),
        (
            "system_taxonomies",
            "uniq_sys_tax_ws_scope_kind_value_current",
        ),
    ] {
        app.state
            .db
            .raw()
            .collection::<Document>(collection)
            .drop_index(index, None)
            .await
            .expect("drop current index for legacy fixture");
    }

    app.state
        .db
        .operation_domain_configs()
        .insert_many(
            vec![
                domain_row(&workspace, 1, true),
                domain_row(&workspace, 2, true),
            ],
            None,
        )
        .await
        .expect("seed multiple domain currents");
    app.state
        .db
        .operation_state_policies()
        .insert_many(
            vec![
                policy_row(&workspace, 1, false),
                policy_row(&workspace, 2, false),
            ],
            None,
        )
        .await
        .expect("seed missing policy current");
    app.state
        .db
        .collection_system_taxonomies()
        .insert_many(
            vec![
                taxonomy_row(&workspace, 1, true),
                taxonomy_row(&workspace, 2, false),
                taxonomy_row_for_value(&workspace, "sr008_broken", 1, false),
                taxonomy_row_for_value(&workspace, "sr008_broken", 2, false),
            ],
            None,
        )
        .await
        .expect("seed valid taxonomy current");

    let warm_error = wechatagent::agent::init_global_taxonomy_cache(&app.state.db)
        .await
        .expect_err("multiple/missing pointers must fail closed before reconciliation");
    assert!(warm_error.to_string().contains("current pointer invalid"));

    wechatagent::db::migrations::m048_ops_single_current::run_step(&app.state.db)
        .await
        .expect("reconcile legacy pointers");
    wechatagent::db::migrations::m048_ops_single_current::run_step(&app.state.db)
        .await
        .expect("reconciliation is idempotent");

    assert_eq!(
        current_versions(
            &app,
            "operation_domain_configs",
            doc! { "workspace_id": &workspace, "domain": "user_operations" },
        )
        .await
        .unwrap(),
        vec![2]
    );
    assert_eq!(
        current_versions(
            &app,
            "operation_state_policies",
            doc! {
                "workspace_id": &workspace,
                "domain": "user_operations",
                "state_key": "sr008_state",
            },
        )
        .await
        .unwrap(),
        vec![2]
    );
    assert_eq!(
        current_versions(
            &app,
            "system_taxonomies",
            doc! {
                "workspace_id": &workspace,
                "scope": "global",
                "kind": "sr008_kind",
                "value.id": "sr008_value",
            },
        )
        .await
        .unwrap(),
        vec![1],
        "an existing unique current must be preserved even when it is not max(version)"
    );
    assert_eq!(
        current_versions(
            &app,
            "system_taxonomies",
            doc! {
                "workspace_id": &workspace,
                "scope": "global",
                "kind": "sr008_kind",
                "value.id": "sr008_broken",
            },
        )
        .await
        .unwrap(),
        vec![2],
        "a zero-current taxonomy stream must elect its highest version"
    );
    wechatagent::agent::init_global_taxonomy_cache(&app.state.db)
        .await
        .expect("cache loads after reconciliation");

    app.state
        .db
        .ensure_indexes()
        .await
        .expect("rebuild partial unique indexes");
    assert!(app
        .state
        .db
        .operation_domain_configs()
        .insert_one(domain_row(&workspace, 3, true), None)
        .await
        .is_err());
    assert!(app
        .state
        .db
        .operation_state_policies()
        .insert_one(policy_row(&workspace, 3, true), None)
        .await
        .is_err());
    assert!(app
        .state
        .db
        .collection_system_taxonomies()
        .insert_one(taxonomy_row(&workspace, 3, true), None)
        .await
        .is_err());

    app.cleanup().await;
}

async fn post(client: &reqwest::Client, base: &str, cookie: &str, path: &str) -> reqwest::Response {
    client
        .post(format!("{base}{path}"))
        .header(reqwest::header::COOKIE, cookie)
        .send()
        .await
        .expect("send SR-008 request")
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn all_version_actions_are_atomic_preserve_history_and_serialize_concurrency() {
    let app = TestApp::start_repl_set().await;
    let workspace = format!("sr008-actions-{}", ObjectId::new().to_hex());
    let domain = domain_row(&workspace, 1, true);
    let domain_id = domain.id.expect("domain id");
    let policy = policy_row(&workspace, 1, true);
    let policy_id = policy.id.expect("policy id");
    let taxonomy = taxonomy_row(&workspace, 1, true);
    let taxonomy_id = taxonomy.id.expect("taxonomy id");
    app.state
        .db
        .operation_domain_configs()
        .insert_one(domain, None)
        .await
        .unwrap();
    app.state
        .db
        .operation_state_policies()
        .insert_one(policy, None)
        .await
        .unwrap();
    app.state
        .db
        .collection_system_taxonomies()
        .insert_one(taxonomy, None)
        .await
        .unwrap();

    let (base, cookie, server) = start_api(&app, &workspace).await.unwrap();
    let client = reqwest::Client::new();
    let resources = [
        (
            "operation-domains",
            domain_id,
            "operation_domain_configs",
            doc! { "workspace_id": &workspace, "domain": "user_operations" },
        ),
        (
            "operation-state-policies",
            policy_id,
            "operation_state_policies",
            doc! {
                "workspace_id": &workspace,
                "domain": "user_operations",
                "state_key": "sr008_state",
            },
        ),
        (
            "taxonomies",
            taxonomy_id,
            "system_taxonomies",
            doc! {
                "workspace_id": &workspace,
                "scope": "global",
                "kind": "sr008_kind",
                "value.id": "sr008_value",
            },
        ),
    ];

    for (route, original_id, collection, scope) in resources {
        let response = post(
            &client,
            &base,
            &cookie,
            &format!("/admin/{route}/{original_id}/publish"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        let published_id = body["id"].as_str().expect("published id");
        assert_eq!(body["version"], 2);
        assert_eq!(
            current_versions(&app, collection, scope.clone())
                .await
                .unwrap(),
            vec![2]
        );

        let rollback = post(
            &client,
            &base,
            &cookie,
            &format!("/admin/{route}/{published_id}/rollback"),
        )
        .await;
        assert_eq!(rollback.status(), StatusCode::OK);
        assert_eq!(
            current_versions(&app, collection, scope.clone())
                .await
                .unwrap(),
            vec![1]
        );

        let rollout = post(
            &client,
            &base,
            &cookie,
            &format!("/admin/{route}/{published_id}/rollout"),
        )
        .await;
        assert_eq!(rollout.status(), StatusCode::OK);
        assert_eq!(
            current_versions(&app, collection, scope.clone())
                .await
                .unwrap(),
            vec![2]
        );
        assert_eq!(
            app.state
                .db
                .raw()
                .collection::<Document>(collection)
                .count_documents(scope, None)
                .await
                .unwrap(),
            2,
            "version actions must retain both historical rows"
        );
    }

    // Use a same-workspace distinct domain for the race so both requests cross
    // the same authenticated production route and logical scope.
    let mut concurrent = domain_row(&workspace, 1, true);
    concurrent.domain = format!("sr008_concurrent_{}", ObjectId::new().to_hex());
    let concurrent_id = concurrent.id.expect("replacement concurrent domain id");
    let concurrent_domain = concurrent.domain.clone();
    app.state
        .db
        .operation_domain_configs()
        .insert_one(concurrent, None)
        .await
        .unwrap();
    let path = format!("/admin/operation-domains/{concurrent_id}/publish");
    let first = post(&client, &base, &cookie, &path);
    let second = post(&client, &base, &cookie, &path);
    let (first, second) = tokio::join!(first, second);
    let statuses = [first.status(), second.status()];
    assert!(statuses
        .iter()
        .all(|status| *status == StatusCode::OK || *status == StatusCode::CONFLICT));
    let successes = statuses
        .iter()
        .filter(|status| **status == StatusCode::OK)
        .count() as u64;
    assert!(successes >= 1);
    let race_scope = doc! {
        "workspace_id": &workspace,
        "domain": &concurrent_domain,
    };
    assert_eq!(
        current_versions(&app, "operation_domain_configs", race_scope.clone())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        app.state
            .db
            .operation_domain_configs()
            .count_documents(race_scope, None)
            .await
            .unwrap(),
        1 + successes,
        "each committed publish appends one row and conflicts leave no partial history"
    );

    server.abort();
    app.cleanup().await;
}

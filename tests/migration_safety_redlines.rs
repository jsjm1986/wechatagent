//! Mongo-backed redlines for startup migrations that protect runtime policy/provider invariants.

mod common;

use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use wechatagent::db::migrations::{
    m057_explicit_acknowledgement_action as m057, m058_llm_provider_active_invariant as m058,
    m062_explicit_appointment_request_action as m062,
    m063_user_operation_runtime_budget_defaults as m063,
};

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn m057_handles_missing_null_empty_and_concurrent_reruns() {
    let app = common::TestApp::start().await;
    let policies = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_state_policies");
    let ids = [ObjectId::new(), ObjectId::new(), ObjectId::new()];
    policies
        .insert_many(
            [
                doc! {
                    "_id": ids[0], "workspace_id": "m057", "domain": "d",
                    "state_key": "missing", "forbidden": [], "version": 1,
                    "current_version": true, "status": "active",
                },
                doc! {
                    "_id": ids[1], "workspace_id": "m057", "domain": "d",
                    "state_key": "null", "allowed": null, "forbidden": ["reply"],
                    "version": 1, "current_version": true, "status": "active",
                },
                doc! {
                    "_id": ids[2], "workspace_id": "m057", "domain": "d",
                    "state_key": "empty", "allowed": [], "forbidden": ["acknowledgement"],
                    "version": 1, "current_version": true, "status": "active",
                },
            ],
            None,
        )
        .await
        .expect("seed legacy policies");

    let (left, right) = tokio::join!(m057::run_step(&app.state.db), m057::run_step(&app.state.db));
    left.expect("first concurrent migration");
    right.expect("second concurrent migration");
    m057::run_step(&app.state.db)
        .await
        .expect("idempotent migration rerun");

    for (index, id) in ids.into_iter().enumerate() {
        let row = policies
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("read migrated policy")
            .expect("migrated policy exists");
        let allowed = row.get_array("allowed").expect("allowed materialized");
        assert!(allowed.iter().all(|value| matches!(value, Bson::String(_))));
        if index < 2 {
            assert!(allowed
                .iter()
                .any(|value| value.as_str() == Some("acknowledgement")));
        } else {
            assert!(!allowed
                .iter()
                .any(|value| value.as_str() == Some("acknowledgement")));
        }
    }
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn m062_materializes_appointment_action_without_overriding_forbidden() {
    let app = common::TestApp::start().await;
    let policies = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_state_policies");
    let ids = [ObjectId::new(), ObjectId::new()];
    policies
        .insert_many(
            [
                doc! {
                    "_id": ids[0], "workspace_id": "m062", "domain": "d",
                    "state_key": "legacy", "allowed": ["reply"], "forbidden": [],
                    "version": 1, "current_version": true, "status": "active",
                },
                doc! {
                    "_id": ids[1], "workspace_id": "m062", "domain": "d",
                    "state_key": "forbidden", "allowed": [],
                    "forbidden": ["appointment_request"],
                    "version": 1, "current_version": true, "status": "active",
                },
            ],
            None,
        )
        .await
        .expect("seed legacy appointment policies");

    let (left, right) = tokio::join!(m062::run_step(&app.state.db), m062::run_step(&app.state.db));
    left.expect("first concurrent migration");
    right.expect("second concurrent migration");
    m062::run_step(&app.state.db)
        .await
        .expect("idempotent migration rerun");

    let migrated = policies
        .find_one(doc! { "_id": ids[0] }, None)
        .await
        .expect("read migrated row")
        .expect("migrated row exists");
    assert!(migrated
        .get_array("allowed")
        .expect("allowed array")
        .iter()
        .any(|value| value.as_str() == Some("appointment_request")));

    let forbidden = policies
        .find_one(doc! { "_id": ids[1] }, None)
        .await
        .expect("read forbidden row")
        .expect("forbidden row exists");
    assert!(!forbidden
        .get_array("allowed")
        .expect("allowed array")
        .iter()
        .any(|value| value.as_str() == Some("appointment_request")));

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn m063_upgrades_only_the_untouched_system_budget_tuple() {
    let app = common::TestApp::start().await;
    let configs = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_domain_configs");
    let ids = [
        ObjectId::new(),
        ObjectId::new(),
        ObjectId::new(),
        ObjectId::new(),
    ];
    configs
        .insert_many(
            [
                doc! {
                    "_id": ids[0], "workspace_id": "m063-system", "domain": "user_operations",
                    "current_version": true, "seeded_by": "system",
                    "runtime_parameters": {
                        "runTokenBudget": 30_000_i64,
                        "runMaxLlmCalls": 6_i32,
                        "simulationTokenBudget": 60_000_i64,
                    },
                },
                doc! {
                    "_id": ids[1], "workspace_id": "m063-manual", "domain": "user_operations",
                    "current_version": true, "seeded_by": "manual",
                    "runtime_parameters": {
                        "runTokenBudget": 30_000_i64,
                        "runMaxLlmCalls": 6_i32,
                        "simulationTokenBudget": 60_000_i64,
                    },
                },
                doc! {
                    "_id": ids[2], "workspace_id": "m063-custom", "domain": "user_operations",
                    "current_version": true, "seeded_by": "system",
                    "runtime_parameters": {
                        "runTokenBudget": 120_000_i64,
                        "runMaxLlmCalls": 6_i32,
                        "simulationTokenBudget": 60_000_i64,
                    },
                },
                doc! {
                    "_id": ids[3], "workspace_id": "m063-escalated", "domain": "user_operations",
                    "current_version": true, "seeded_by": "system",
                    "runtime_parameters": {
                        "runTokenBudget": 30_000_i64,
                        "runTokenBudgetEscalated": 100_000_i64,
                        "runMaxLlmCalls": 6_i32,
                        "simulationTokenBudget": 60_000_i64,
                    },
                },
            ],
            None,
        )
        .await
        .expect("seed runtime budget fixtures");

    let (left, right) = tokio::join!(m063::run_step(&app.state.db), m063::run_step(&app.state.db));
    left.expect("first concurrent migration");
    right.expect("second concurrent migration");
    m063::run_step(&app.state.db)
        .await
        .expect("idempotent migration rerun");

    let migrated = configs
        .find_one(doc! { "_id": ids[0] }, None)
        .await
        .expect("read migrated config")
        .expect("migrated config exists");
    let migrated_runtime = migrated
        .get_document("runtime_parameters")
        .expect("migrated runtime parameters");
    assert_eq!(
        migrated_runtime.get_i64("runTokenBudget").ok(),
        Some(300_000)
    );
    assert_eq!(
        migrated_runtime.get_i64("runTokenBudgetEscalated").ok(),
        Some(600_000)
    );
    assert_eq!(migrated_runtime.get_i32("runMaxLlmCalls").ok(), Some(10));
    assert_eq!(
        migrated_runtime.get_i64("simulationTokenBudget").ok(),
        Some(300_000)
    );

    for id in &ids[1..] {
        let preserved = configs
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("read preserved config")
            .expect("preserved config exists");
        let runtime = preserved
            .get_document("runtime_parameters")
            .expect("preserved runtime parameters");
        assert_ne!(
            runtime.get_i64("runTokenBudgetEscalated").ok(),
            Some(600_000),
            "operator-owned or customized rows must not be migrated"
        );
    }

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn m058_rejects_duplicate_active_providers_without_rewriting_rows() {
    let app = common::TestApp::start().await;
    let providers = app
        .state
        .db
        .raw()
        .collection::<Document>("llm_provider_configs");
    providers
        .drop_index("llm_provider_one_active_per_workspace", None)
        .await
        .expect("drop unique active index for legacy fixture");
    let ids = [ObjectId::new(), ObjectId::new()];
    providers
        .insert_many(
            [
                doc! { "_id": ids[0], "workspaceId": "m058", "providerId": "p1", "isActive": true },
                doc! { "_id": ids[1], "workspaceId": "m058", "providerId": "p2", "isActive": true },
            ],
            None,
        )
        .await
        .expect("seed duplicate active providers");
    let before = providers
        .find_one(doc! { "_id": ids[0] }, None)
        .await
        .expect("read before")
        .expect("fixture exists");

    let error = m058::run_step(&app.state.db)
        .await
        .expect_err("ambiguous active pointer must fail closed")
        .to_string();
    assert!(error.contains("workspace m058"));
    assert!(error.contains("p1,p2"));
    let after = providers
        .find_one(doc! { "_id": ids[0] }, None)
        .await
        .expect("read after")
        .expect("fixture remains");
    assert_eq!(
        after, before,
        "read-only audit must not elect or rewrite a provider"
    );
    assert_eq!(
        providers
            .count_documents(doc! { "workspaceId": "m058", "isActive": true }, None)
            .await
            .expect("count active rows"),
        2
    );
    app.cleanup().await;
}

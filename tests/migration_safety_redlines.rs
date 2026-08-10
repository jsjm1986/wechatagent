//! Mongo-backed redlines for startup migrations that protect runtime policy/provider invariants.

mod common;

use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use wechatagent::db::migrations::{
    m057_explicit_acknowledgement_action as m057, m058_llm_provider_active_invariant as m058,
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

//! Integration coverage for the Evolution release-protocol reconciliation.
//!
//! Startup has already applied m040 before these fixtures are inserted. Each
//! test reruns the public migration step to model an upgraded database.

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

fn legacy_override(
    id: ObjectId,
    proposal_id: ObjectId,
    account_id: &str,
    gate_key: &str,
    value: f64,
    released_at_ms: i64,
) -> Document {
    doc! {
        "_id": id,
        "workspace_id": "ws-evolution",
        "account_id": account_id,
        "gate_key": gate_key,
        "value": value,
        "source_proposal_id": proposal_id,
        "released_at": DateTime::from_millis(released_at_ms),
        "released_by": "legacy-admin",
        "rolled_back_at": null,
        "rolled_back_by": null,
    }
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn m040_backfills_revisions_elects_one_current_and_is_idempotent() {
    let app = common::TestApp::start().await;
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("threshold_overrides");

    let old_id = ObjectId::new();
    let new_id = ObjectId::new();
    let other_account_id = ObjectId::new();
    collection
        .insert_many(
            vec![
                legacy_override(
                    old_id,
                    ObjectId::new(),
                    "account-a",
                    "fact_risk_block",
                    6.0,
                    1_700_000_000_000,
                ),
                legacy_override(
                    new_id,
                    ObjectId::new(),
                    "account-a",
                    "fact_risk_block",
                    5.5,
                    1_700_000_100_000,
                ),
                legacy_override(
                    other_account_id,
                    ObjectId::new(),
                    "account-b",
                    "fact_risk_block",
                    7.0,
                    1_700_000_050_000,
                ),
            ],
            None,
        )
        .await
        .expect("insert legacy threshold overrides");

    wechatagent::db::migrations::m040_evolution_release_protocol::run_step(&app.state.db)
        .await
        .expect("first m040 run");
    wechatagent::db::migrations::m040_evolution_release_protocol::run_step(&app.state.db)
        .await
        .expect("second m040 run");

    for (id, value, expected_current) in [
        (old_id, 6.0, false),
        (new_id, 5.5, true),
        (other_account_id, 7.0, true),
    ] {
        let row = collection
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("read reconciled override")
            .expect("reconciled override exists");
        assert_eq!(
            row.get_str("released_revision").expect("released revision"),
            wechatagent::evolution::revision::threshold_revision(Some(id), value)
        );
        assert_eq!(
            row.get_bool("current_version").expect("current pointer"),
            expected_current
        );
    }

    assert_eq!(
        collection
            .count_documents(
                doc! {
                    "workspace_id": "ws-evolution",
                    "account_id": "account-a",
                    "gate_key": "fact_risk_block",
                    "current_version": true,
                },
                None,
            )
            .await
            .expect("count account-a current rows"),
        1
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn m040_duplicate_proposal_artifacts_fail_before_any_revision_write() {
    let app = common::TestApp::start().await;
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("threshold_overrides");

    // Startup normally prevents this state. Drop only the new guard index so
    // the test can model a corrupt pre-migration production snapshot.
    collection
        .drop_index("uniq_threshold_artifact_per_proposal", None)
        .await
        .expect("drop artifact guard for corruption fixture");

    let proposal_id = ObjectId::new();
    let first_id = ObjectId::new();
    let second_id = ObjectId::new();
    collection
        .insert_many(
            vec![
                legacy_override(
                    first_id,
                    proposal_id,
                    "account-a",
                    "fact_risk_block",
                    6.0,
                    1_700_000_000_000,
                ),
                legacy_override(
                    second_id,
                    proposal_id,
                    "account-a",
                    "fact_risk_block",
                    5.5,
                    1_700_000_100_000,
                ),
            ],
            None,
        )
        .await
        .expect("insert duplicate proposal artifacts");

    let error =
        wechatagent::db::migrations::m040_evolution_release_protocol::run_step(&app.state.db)
            .await
            .expect_err("duplicate artifacts must fail closed");
    assert!(error.to_string().contains("duplicate threshold artifacts"));

    for id in [first_id, second_id] {
        let row = collection
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("read corrupt fixture")
            .expect("corrupt fixture exists");
        assert!(
            !row.contains_key("released_revision"),
            "validation failure must not partially backfill revisions"
        );
        assert!(
            !row.contains_key("current_version"),
            "validation failure must not change current pointers"
        );
    }

    app.cleanup().await;
}

//! Dynamic migration regressions for SR-060 relationship review cycles.
//! Requires MongoDB; tests are ignored by default.

#![cfg(test)]

mod common;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::IndexOptions,
    IndexModel,
};
use wechatagent::db::migrations::m045_relationship_review_cycles;

use crate::common::TestApp;

const LEGACY_INDEX: &str = "workspace_id_1_contact_id_1";
const PENDING_INDEX: &str = "uniq_relationship_pending_ws_contact";

async fn reset_to_legacy_index(app: &TestApp) {
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("relationship_type_suggestions");
    let _ = collection.drop_index(PENDING_INDEX, None).await;
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "contact_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name(LEGACY_INDEX.to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await
        .expect("create legacy relationship index");
}

async fn index_names(app: &TestApp) -> Vec<String> {
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("relationship_type_suggestions");
    let mut cursor = collection.list_indexes(None).await.expect("list indexes");
    let mut names = Vec::new();
    while let Some(index) = cursor.try_next().await.expect("read index") {
        if let Some(name) = index.options.and_then(|options| options.name) {
            names.push(name);
        }
    }
    names
}

fn review_row(workspace_id: &str, contact_id: &str, status: &str) -> Document {
    let now = DateTime::now();
    doc! {
        "workspace_id": workspace_id,
        "account_id": "account-a",
        "contact_id": contact_id,
        "suggested_value": "peer",
        "confidence": 8,
        "status": status,
        "occurrences": 1,
        "first_seen_at": now,
        "last_seen_at": now,
    }
}

#[tokio::test]
#[ignore]
async fn migration_retires_full_history_index_and_allows_next_pending_cycle() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let contact_id = ObjectId::new().to_hex();
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("relationship_type_suggestions");

    collection
        .insert_one(review_row(&workspace_id, &contact_id, "approved"), None)
        .await
        .expect("insert terminal history");
    reset_to_legacy_index(&app).await;

    m045_relationship_review_cycles::run_step(&app.state.db)
        .await
        .expect("retire legacy index");
    app.state
        .db
        .ensure_indexes()
        .await
        .expect("create replacement indexes");

    let names = index_names(&app).await;
    assert!(!names.iter().any(|name| name == LEGACY_INDEX));
    assert!(names.iter().any(|name| name == PENDING_INDEX));

    collection
        .insert_one(review_row(&workspace_id, &contact_id, "pending"), None)
        .await
        .expect("terminal history must not occupy pending slot");
    let duplicate = collection
        .insert_one(review_row(&workspace_id, &contact_id, "pending"), None)
        .await
        .expect_err("only one pending cycle may exist");
    assert!(duplicate.to_string().contains("E11000"));
    assert_eq!(
        collection
            .count_documents(
                doc! { "workspace_id": &workspace_id, "contact_id": &contact_id },
                None,
            )
            .await
            .expect("count review history"),
        2
    );
}

#[tokio::test]
#[ignore]
async fn malformed_pending_row_fails_before_legacy_index_is_dropped() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("relationship_type_suggestions");
    reset_to_legacy_index(&app).await;

    let mut malformed = review_row(&workspace_id, &ObjectId::new().to_hex(), "pending");
    malformed.remove("account_id");
    collection
        .insert_one(malformed, None)
        .await
        .expect("insert malformed legacy pending row");

    let error = m045_relationship_review_cycles::run_step(&app.state.db)
        .await
        .expect_err("malformed pending identity must fail closed");
    assert!(error.to_string().contains("without account_id"));
    let names = index_names(&app).await;
    assert!(
        names.iter().any(|name| name == LEGACY_INDEX),
        "audit failure must happen before destructive index retirement"
    );
    assert!(!names.iter().any(|name| name == PENDING_INDEX));
}

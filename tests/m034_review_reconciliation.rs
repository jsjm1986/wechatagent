//! Corrective migration evidence for SR-007 and already-marked upgraded DBs.

mod common;

use mongodb::bson::{doc, oid::ObjectId, Document};

#[tokio::test]
#[ignore]
async fn lower_active_prompt_beats_higher_draft_and_reconciles_existing_currents() {
    let app = common::TestApp::start().await;
    let coll = app
        .state
        .db
        .raw()
        .collection::<Document>("prompt_templates");
    let workspace = format!("m034-ws-{}", ObjectId::new().to_hex());
    let prompt_key = "m034.active.priority";
    let active_id = ObjectId::new();
    let draft_id = ObjectId::new();
    coll.insert_many(
        vec![
            doc! {
                "_id": active_id,
                "workspace_id": &workspace,
                "prompt_key": prompt_key,
                "version": 1_i32,
                "status": "active",
                "current_version": false,
            },
            doc! {
                "_id": draft_id,
                "workspace_id": &workspace,
                "prompt_key": prompt_key,
                "version": 2_i32,
                "status": "draft",
                "current_version": true,
            },
        ],
        None,
    )
    .await
    .expect("seed prompt versions");

    app.state
        .db
        .migrations()
        .delete_one(doc! { "_id": "2026_07_034_reconcile_review_fixes" }, None)
        .await
        .expect("clear corrective marker");
    wechatagent::db::migrations::run(&app.state.db)
        .await
        .expect("run corrective migration");

    let active = coll
        .find_one(doc! { "_id": active_id }, None)
        .await
        .expect("read active")
        .expect("active exists");
    let draft = coll
        .find_one(doc! { "_id": draft_id }, None)
        .await
        .expect("read draft")
        .expect("draft exists");
    assert_eq!(active.get_bool("current_version"), Ok(true));
    assert_eq!(draft.get_bool("current_version"), Ok(false));
    assert_eq!(
        coll.count_documents(
            doc! {
                "workspace_id": &workspace,
                "prompt_key": prompt_key,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("count current"),
        1
    );

    app.cleanup().await;
}

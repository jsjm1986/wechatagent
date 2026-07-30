//! Regression for an upgraded database where m043 is already marked applied,
//! but legacy planning-only Prompt drafts still carry current pointers.

mod common;

use mongodb::bson::{doc, Document};

const M043: &str = "2026_07_043_prompt_single_current";
const M049: &str = "2026_07_049_reconcile_prompt_planning_currents";

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn applied_m043_is_corrected_without_publishing_or_rewriting_planning_drafts() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();
    let prompts = app
        .state
        .db
        .raw()
        .collection::<Document>("prompt_templates");

    let keys = ["group.policy", "moment.policy"];
    let key_filter = keys.to_vec();
    let mut before = Vec::new();
    for key in keys {
        let row = prompts
            .find_one(doc! { "workspace_id": &workspace, "prompt_key": key }, None)
            .await
            .expect("read planning draft")
            .expect("planning draft exists");
        let id = row.get_object_id("_id").expect("planning draft id");
        let content = row
            .get_str("content")
            .expect("planning content")
            .to_string();
        prompts
            .update_one(
                doc! { "_id": id, "status": "draft", "current_version": false },
                doc! { "$set": { "current_version": true } },
                None,
            )
            .await
            .expect("recreate legacy planning current");
        before.push((id, content));
    }

    let migrations = app.state.db.migrations();
    assert!(migrations
        .find_one(doc! { "_id": M043, "status": "applied" }, None)
        .await
        .expect("read m043 marker")
        .is_some());
    migrations
        .delete_one(doc! { "_id": M049 }, None)
        .await
        .expect("clear m049 marker");

    wechatagent::db::migrations::run(&app.state.db)
        .await
        .expect("run corrective migration");

    for (id, content) in &before {
        let row = prompts
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("read corrected planning draft")
            .expect("planning draft remains");
        assert_eq!(row.get_str("status"), Ok("draft"));
        assert_eq!(row.get_bool("current_version"), Ok(false));
        assert_eq!(row.get_str("content"), Ok(content.as_str()));
    }
    assert!(migrations
        .find_one(doc! { "_id": M049, "status": "applied" }, None)
        .await
        .expect("read m049 marker")
        .is_some());

    assert!(
        !wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account,)
            .await
            .expect("startup alignment accepts corrected planning drafts")
    );

    let count_before_rerun = prompts
        .count_documents(
            doc! { "workspace_id": &workspace, "prompt_key": { "$in": &key_filter } },
            None,
        )
        .await
        .expect("count planning versions");
    wechatagent::db::migrations::run(&app.state.db)
        .await
        .expect("corrective migration is idempotent");
    assert_eq!(
        prompts
            .count_documents(
                doc! { "workspace_id": &workspace, "prompt_key": { "$in": &key_filter } },
                None,
            )
            .await
            .expect("count planning versions after rerun"),
        count_before_rerun
    );

    app.cleanup().await;
}

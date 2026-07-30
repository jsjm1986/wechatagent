//! SR-055 regression coverage for append-only PromptTemplate versions.

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::prompt_template_versions::{self, NewPromptTemplateVersion};

fn version_input<'a>(
    key: &'a str,
    title: &'a str,
    content: &'a str,
) -> NewPromptTemplateVersion<'a> {
    NewPromptTemplateVersion {
        prompt_key: key,
        agent_kind: "user",
        layer: "task",
        title,
        description: None,
        content,
        prompt_pack_version: "test",
        actor: "tester",
        seeded_by: "manual",
        locale: Some("zh-CN"),
        previous_version: None,
        source_proposal_id: None,
    }
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn append_publish_and_rollback_preserve_one_current_and_all_history() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let key = format!("sr055.lifecycle.{}", ObjectId::new().to_hex());

    let first = prompt_template_versions::append_version(
        &app.state.db,
        &workspace,
        version_input(&key, "first", "first immutable content"),
    )
    .await
    .expect("append first draft");
    let first_id = first.id.expect("first id");
    prompt_template_versions::publish_version(&app.state.db, &workspace, first_id, "tester")
        .await
        .expect("publish first");

    let second = prompt_template_versions::append_edited_draft(
        &app.state.db,
        &workspace,
        first_id,
        version_input(&key, "second", "second immutable content"),
    )
    .await
    .expect("append successor");
    let second_id = second.id.expect("second id");
    prompt_template_versions::publish_version(&app.state.db, &workspace, second_id, "tester")
        .await
        .expect("publish second");

    let current = prompt_template_versions::load_unique_current(&app.state.db, &workspace, &key)
        .await
        .expect("load current")
        .expect("current exists");
    assert_eq!(current.id, Some(second_id));
    assert_eq!(current.content, "second immutable content");

    prompt_template_versions::publish_version(&app.state.db, &workspace, first_id, "rollback")
        .await
        .expect("publish historical version as rollback");
    let restored = prompt_template_versions::load_unique_current(&app.state.db, &workspace, &key)
        .await
        .expect("load restored")
        .expect("restored exists");
    assert_eq!(restored.id, Some(first_id));
    assert_eq!(restored.content, "first immutable content");
    assert_eq!(
        app.state
            .db
            .prompt_templates()
            .count_documents(
                doc! { "workspace_id": &workspace, "prompt_key": &key },
                None
            )
            .await
            .expect("count history"),
        2,
        "publish and rollback must not delete content history"
    );

    app.cleanup().await;
}

fn legacy_prompt(
    id: ObjectId,
    workspace: &str,
    key: &str,
    version: i32,
    status: &str,
    current: bool,
) -> Document {
    doc! {
        "_id": id,
        "workspace_id": workspace,
        "prompt_key": key,
        "agent_kind": "user",
        "layer": "task",
        "title": format!("v{version}"),
        "description": null,
        "content": format!("legacy content {version}"),
        "status": status,
        "version": version,
        "prompt_pack_version": "legacy",
        "created_by": "legacy",
        "created_at": DateTime::now(),
        "updated_at": DateTime::now(),
        "current_version": current,
        "previous_version": null,
        "seeded_by": "legacy",
        "locale": "zh-CN",
    }
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn m043_rejects_split_pointer_before_any_write() {
    let app = common::TestApp::start().await;
    let workspace = format!("sr055-split-{}", ObjectId::new().to_hex());
    let key = "sr055.migration.split";
    let active_id = ObjectId::new();
    let draft_id = ObjectId::new();
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("prompt_templates");
    collection
        .insert_many(
            vec![
                legacy_prompt(active_id, &workspace, key, 1, "active", false),
                legacy_prompt(draft_id, &workspace, key, 2, "draft", true),
            ],
            None,
        )
        .await
        .expect("seed split pointer rows");

    let error = wechatagent::db::migrations::m043_prompt_single_current::run_step(&app.state.db)
        .await
        .expect_err("split pointer must fail closed");
    assert!(error.to_string().contains("requires one active current"));
    let active = collection
        .find_one(doc! { "_id": active_id }, None)
        .await
        .expect("read active")
        .expect("active retained");
    let draft = collection
        .find_one(doc! { "_id": draft_id }, None)
        .await
        .expect("read draft")
        .expect("draft retained");
    assert_eq!(active.get_str("status").unwrap(), "active");
    assert!(!active.get_bool("current_version").unwrap());
    assert_eq!(draft.get_str("status").unwrap(), "draft");
    assert!(draft.get_bool("current_version").unwrap());

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn m043_archives_only_non_current_active_history() {
    let app = common::TestApp::start().await;
    let workspace = format!("sr055-reconcile-{}", ObjectId::new().to_hex());
    let key = "sr055.migration.reconcile";
    let current_id = ObjectId::new();
    let historical_id = ObjectId::new();
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("prompt_templates");
    collection
        .insert_many(
            vec![
                legacy_prompt(current_id, &workspace, key, 2, "active", true),
                legacy_prompt(historical_id, &workspace, key, 1, "active", false),
            ],
            None,
        )
        .await
        .expect("seed reconcilable rows");

    wechatagent::db::migrations::m043_prompt_single_current::run_step(&app.state.db)
        .await
        .expect("reconcile legacy active rows");
    let current = collection
        .find_one(doc! { "_id": current_id }, None)
        .await
        .unwrap()
        .unwrap();
    let historical = collection
        .find_one(doc! { "_id": historical_id }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.get_str("status").unwrap(), "active");
    assert!(current.get_bool("current_version").unwrap());
    assert_eq!(historical.get_str("status").unwrap(), "archived");
    assert!(!historical.get_bool("current_version").unwrap());
    assert_eq!(
        collection
            .count_documents(doc! { "workspace_id": &workspace, "prompt_key": key }, None)
            .await
            .unwrap(),
        2,
        "migration must retain history"
    );

    app.cleanup().await;
}

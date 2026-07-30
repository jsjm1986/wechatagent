//! SR-053 regression coverage for immutable Agent Soul versions.
//!
//! Lifecycle tests require a replica set because publication changes the
//! published pointer in a transaction. Migration tests intentionally use a
//! standalone database and temporarily drop only the two Soul uniqueness
//! indexes so legacy-invalid rows can be inserted after normal startup.

mod common;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::FindOptions,
};
use wechatagent::{
    models::AgentSoul,
    prompts,
    soul_versions::{self, NewSoulVersion},
};

async fn kind_rows(app: &common::TestApp, workspace: &str, kind: &str) -> Vec<AgentSoul> {
    app.state
        .db
        .agent_souls()
        .find(
            doc! { "workspace_id": workspace, "agent_kind": kind },
            FindOptions::builder().sort(doc! { "version": 1 }).build(),
        )
        .await
        .expect("query Soul versions")
        .try_collect()
        .await
        .expect("collect Soul versions")
}

fn published_count(rows: &[AgentSoul]) -> usize {
    rows.iter().filter(|row| row.status == "published").count()
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn edit_publish_rollback_and_reset_keep_immutable_history() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    let original = soul_versions::load_unique_published(&app.state.db, &workspace, "user")
        .await
        .expect("initial published user Soul");
    let original_id = original.id.expect("seeded Soul has _id");
    let original_content = original.content.clone();

    let draft = soul_versions::append_edited_draft(
        &app.state.db,
        &workspace,
        original_id,
        "user",
        "edited user Soul",
        "immutable edited content",
        "admin-edit",
    )
    .await
    .expect("append edited draft");
    assert_eq!(draft.status, "draft");
    assert_eq!(draft.previous_version, Some(original.version));

    let original_after_edit = app
        .state
        .db
        .agent_souls()
        .find_one(doc! { "_id": original_id }, None)
        .await
        .expect("read original after edit")
        .expect("original retained");
    assert_eq!(original_after_edit.content, original_content);
    assert_eq!(original_after_edit.status, "published");

    let rows_after_edit = kind_rows(&app, &workspace, "user").await;
    let count_after_edit = rows_after_edit.len();
    let draft_id = draft.id.expect("draft has _id");
    let published =
        soul_versions::publish_version(&app.state.db, &workspace, draft_id, "admin-publish")
            .await
            .expect("publish edited draft");
    assert_eq!(published.published_by.as_deref(), Some("admin-publish"));

    let rows_after_publish = kind_rows(&app, &workspace, "user").await;
    assert_eq!(rows_after_publish.len(), count_after_edit);
    assert_eq!(published_count(&rows_after_publish), 1);
    assert_eq!(
        rows_after_publish
            .iter()
            .find(|row| row.id == Some(original_id))
            .expect("original version retained")
            .status,
        "archived"
    );

    soul_versions::publish_version(&app.state.db, &workspace, original_id, "admin-rollback")
        .await
        .expect("publish archived version as rollback");
    let rows_after_rollback = kind_rows(&app, &workspace, "user").await;
    assert_eq!(rows_after_rollback.len(), count_after_edit);
    assert_eq!(published_count(&rows_after_rollback), 1);
    assert_eq!(
        rows_after_rollback
            .iter()
            .find(|row| row.id == Some(original_id))
            .expect("rollback target retained")
            .status,
        "published"
    );

    let before_reset: Vec<(String, usize)> = ["user", "management", "group", "moment"]
        .into_iter()
        .map(|kind| (kind.to_string(), 0))
        .collect();
    let mut before_reset = before_reset;
    for (kind, count) in &mut before_reset {
        *count = kind_rows(&app, &workspace, kind).await.len();
    }

    prompts::reset_prompt_pack_v2_as_actor(&app.state.db, &workspace, &account, "admin-reset")
        .await
        .expect("explicit pack reset");

    for (kind, previous_count) in before_reset {
        let rows = kind_rows(&app, &workspace, &kind).await;
        assert_eq!(
            rows.len(),
            previous_count + 1,
            "reset must append exactly one built-in {kind} version"
        );
        let latest = rows.last().expect("reset version exists");
        assert_eq!(latest.seeded_by.as_deref(), Some("system_reset"));
        if matches!(kind.as_str(), "user" | "management") {
            assert_eq!(published_count(&rows), 1, "{kind} has one pointer");
            assert_eq!(latest.status, "published");
            assert_eq!(latest.published_by.as_deref(), Some("admin-reset"));
        } else {
            assert_eq!(
                published_count(&rows),
                0,
                "inactive {kind} placeholder must remain unpublished"
            );
            assert_eq!(latest.status, "draft");
            assert_eq!(latest.published_by, None);
        }
    }
    let reset_user = soul_versions::load_unique_published(&app.state.db, &workspace, "user")
        .await
        .expect("reset user Soul");
    assert_eq!(reset_user.content, original_content);
    assert_ne!(reset_user.id, Some(original_id));

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn bootstrap_empty_prompt_pack_preserves_existing_published_soul() {
    let app = common::TestApp::start().await;
    let workspace = format!("sr053-bootstrap-{}", ObjectId::new().to_hex());
    let account = app.state.config.default_account_id.clone();
    let (custom, inserted) = soul_versions::ensure_initial_published(
        &app.state.db,
        &workspace,
        NewSoulVersion {
            agent_kind: "user",
            name: "operator user Soul",
            content: "operator-owned published content",
            seeded_by: "operator",
            previous_version: None,
        },
    )
    .await
    .expect("seed operator-owned Soul before prompt bootstrap");
    assert!(inserted);
    assert_eq!(
        app.state
            .db
            .prompt_templates()
            .count_documents(doc! { "workspace_id": &workspace }, None)
            .await
            .expect("count prompt templates before bootstrap"),
        0
    );

    let wrote = prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("bootstrap empty prompt pack");
    assert!(wrote);

    let user_rows = kind_rows(&app, &workspace, "user").await;
    assert_eq!(user_rows.len(), 1, "startup must not append a user Soul");
    assert_eq!(published_count(&user_rows), 1);
    let active = soul_versions::load_unique_published(&app.state.db, &workspace, "user")
        .await
        .expect("operator Soul remains published");
    assert_eq!(active.id, custom.id);
    assert_eq!(active.content, "operator-owned published content");
    assert_eq!(active.seeded_by.as_deref(), Some("operator"));
    let management_rows = kind_rows(&app, &workspace, "management").await;
    assert_eq!(management_rows.len(), 1);
    assert_eq!(published_count(&management_rows), 1);
    assert_eq!(management_rows[0].seeded_by.as_deref(), Some("system"));

    for kind in ["group", "moment"] {
        let rows = kind_rows(&app, &workspace, kind).await;
        assert_eq!(rows.len(), 1, "missing {kind} is seeded exactly once");
        assert_eq!(
            published_count(&rows),
            0,
            "inactive {kind} placeholder must not be published"
        );
        assert_eq!(rows[0].status, "draft");
        assert_eq!(rows[0].seeded_by.as_deref(), Some("system"));
    }

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn concurrent_publish_never_loses_or_duplicates_the_pointer() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let before = kind_rows(&app, &workspace, "user").await.len();

    let first = soul_versions::append_version(
        &app.state.db,
        &workspace,
        NewSoulVersion {
            agent_kind: "user",
            name: "concurrent first",
            content: "first candidate",
            seeded_by: "test",
            previous_version: None,
        },
    )
    .await
    .expect("append first candidate");
    let second = soul_versions::append_version(
        &app.state.db,
        &workspace,
        NewSoulVersion {
            agent_kind: "user",
            name: "concurrent second",
            content: "second candidate",
            seeded_by: "test",
            previous_version: None,
        },
    )
    .await
    .expect("append second candidate");

    let (left, right) = tokio::join!(
        soul_versions::publish_version(
            &app.state.db,
            &workspace,
            first.id.expect("first id"),
            "publisher-left"
        ),
        soul_versions::publish_version(
            &app.state.db,
            &workspace,
            second.id.expect("second id"),
            "publisher-right"
        )
    );
    assert!(
        left.is_ok() || right.is_ok(),
        "at least one publisher commits"
    );
    for result in [left, right] {
        if let Err(error) = result {
            assert_eq!(error.to_string(), "soul_publish_conflict");
        }
    }

    let rows = kind_rows(&app, &workspace, "user").await;
    assert_eq!(rows.len(), before + 2, "publication never deletes history");
    assert_eq!(published_count(&rows), 1);
    soul_versions::load_unique_published(&app.state.db, &workspace, "user")
        .await
        .expect("runtime sees one published pointer");

    app.cleanup().await;
}

async fn drop_soul_index(app: &common::TestApp, name: &str) {
    app.state
        .db
        .agent_souls()
        .drop_index(name, None)
        .await
        .unwrap_or_else(|error| panic!("drop {name}: {error}"));
}

fn legacy_soul(id: ObjectId, workspace: &str, version: i32, status: &str) -> Document {
    doc! {
        "_id": id,
        "workspace_id": workspace,
        "agent_kind": "user",
        "name": format!("legacy-v{version}"),
        "content": format!("legacy content {version}"),
        "status": status,
        "version": version,
        "created_at": DateTime::now(),
        "updated_at": DateTime::now(),
    }
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn m042_duplicate_version_fails_before_any_pointer_write() {
    let app = common::TestApp::start().await;
    drop_soul_index(&app, "uniq_agent_soul_ws_kind_version").await;
    drop_soul_index(&app, "uniq_agent_soul_published_ws_kind").await;
    let collection = app.state.db.raw().collection::<Document>("agent_souls");
    let workspace = format!("sr053-duplicate-{}", ObjectId::new().to_hex());
    collection
        .insert_many(
            vec![
                legacy_soul(ObjectId::new(), &workspace, 7, "published"),
                legacy_soul(ObjectId::new(), &workspace, 7, "published"),
            ],
            None,
        )
        .await
        .expect("insert duplicate legacy versions");

    let error = wechatagent::db::migrations::m042_agent_soul_versions::run_step(&app.state.db)
        .await
        .expect_err("duplicate version must fail closed");
    assert!(error.to_string().contains("duplicate version 7"));
    assert_eq!(
        collection
            .count_documents(
                doc! { "workspace_id": &workspace, "status": "published" },
                None
            )
            .await
            .expect("count unchanged pointers"),
        2,
        "full validation must happen before the first archive write"
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn m042_elects_highest_published_version_and_is_idempotent() {
    let app = common::TestApp::start().await;
    drop_soul_index(&app, "uniq_agent_soul_published_ws_kind").await;
    let collection = app.state.db.raw().collection::<Document>("agent_souls");
    let workspace = format!("sr053-election-{}", ObjectId::new().to_hex());
    let old_id = ObjectId::new();
    let new_id = ObjectId::new();
    collection
        .insert_many(
            vec![
                legacy_soul(old_id, &workspace, 1, "published"),
                legacy_soul(new_id, &workspace, 2, "published"),
            ],
            None,
        )
        .await
        .expect("insert legacy multiple pointers");

    wechatagent::db::migrations::m042_agent_soul_versions::run_step(&app.state.db)
        .await
        .expect("first reconciliation");
    wechatagent::db::migrations::m042_agent_soul_versions::run_step(&app.state.db)
        .await
        .expect("idempotent reconciliation");

    let old = collection
        .find_one(doc! { "_id": old_id }, None)
        .await
        .expect("read old")
        .expect("old retained");
    let new = collection
        .find_one(doc! { "_id": new_id }, None)
        .await
        .expect("read new")
        .expect("new retained");
    assert_eq!(old.get_str("status"), Ok("archived"));
    assert_eq!(new.get_str("status"), Ok("published"));
    assert_eq!(
        collection
            .count_documents(doc! { "workspace_id": &workspace }, None)
            .await
            .expect("count retained history"),
        2
    );

    app.cleanup().await;
}

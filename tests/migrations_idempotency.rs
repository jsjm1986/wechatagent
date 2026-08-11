//! 验证迁移框架的幂等性：`TestApp::start()` 已在启动链路跑过一轮
//! `migrations::run`，每条迁移在 `migrations` 集合留一行账。再调一次
//! `migrations::run` 必须按 `_id` 跳过全部已应用项——账册条数不变。
//!
//! 默认 `#[ignore]`，需要 Docker；CI 用 `cargo test -- --ignored` 触发。

mod common;

use std::collections::HashSet;

use mongodb::bson::{doc, Document};
use wechatagent::db::migrations::{run_with_policy, Migration};

fn destructive_probe<'a>(
    db: &'a wechatagent::db::Database,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = wechatagent::error::AppResult<()>> + Send + 'a>,
> {
    Box::pin(async move {
        db.raw()
            .collection::<Document>("migration_probe")
            .insert_one(doc! { "executed": true }, None)
            .await?;
        Ok(())
    })
}

#[tokio::test]
#[ignore]
async fn run_is_idempotent_across_reruns() {
    let app = common::TestApp::start().await;
    let migration_ids = wechatagent::db::migrations::MIGRATIONS
        .iter()
        .map(|migration| migration.id)
        .collect::<Vec<_>>();

    // The collection also stores durable workspace-template initialization markers. Count only
    // registered migration IDs when validating the migration runner's one-row-per-step ledger.
    let count = app
        .state
        .db
        .migrations()
        .count_documents(doc! { "_id": { "$in": &migration_ids } }, None)
        .await
        .expect("count migrations");
    assert_eq!(
        count as usize,
        wechatagent::db::migrations::MIGRATIONS.len(),
        "启动后每条 migration 应各留一行账"
    );

    wechatagent::db::migrations::run(&app.state.db)
        .await
        .expect("rerun migrations");

    let count_after = app
        .state
        .db
        .migrations()
        .count_documents(doc! { "_id": { "$in": &migration_ids } }, None)
        .await
        .expect("count migrations after rerun");
    assert_eq!(
        count_after, count,
        "二次 run 必须按 _id 跳过已应用项，账册条数不变"
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn production_blocked_migration_retries_after_exact_approval() {
    let app = common::TestApp::start().await;
    let migration = Migration {
        id: "2026_07_035_reconcile_legacy_cleanup",
        run: destructive_probe,
    };
    app.state
        .db
        .migrations()
        .delete_one(doc! { "_id": migration.id }, None)
        .await
        .expect("clear startup marker");

    run_with_policy(
        &app.state.db,
        std::slice::from_ref(&migration),
        true,
        &HashSet::new(),
    )
    .await
    .expect("blocked run");
    let writes_before = app
        .state
        .db
        .raw()
        .collection::<Document>("migration_probe")
        .count_documents(doc! {}, None)
        .await
        .expect("count blocked writes");
    let blocked = app
        .state
        .db
        .migrations()
        .find_one(doc! { "_id": migration.id }, None)
        .await
        .expect("read blocked marker")
        .expect("blocked marker exists");
    assert_eq!(writes_before, 0, "blocked migration must not execute");
    assert_eq!(blocked.status.as_deref(), Some("blocked"));
    assert!(blocked.applied_at.is_none());
    assert!(blocked
        .reason
        .as_deref()
        .is_some_and(|reason| { reason.contains("2026_07_035_reconcile_legacy_cleanup") }));

    let approvals = HashSet::from([migration.id.to_string()]);
    run_with_policy(
        &app.state.db,
        std::slice::from_ref(&migration),
        true,
        &approvals,
    )
    .await
    .expect("approved retry");
    let writes_after = app
        .state
        .db
        .raw()
        .collection::<Document>("migration_probe")
        .count_documents(doc! {}, None)
        .await
        .expect("count approved writes");
    let applied = app
        .state
        .db
        .migrations()
        .find_one(doc! { "_id": migration.id }, None)
        .await
        .expect("read applied marker")
        .expect("applied marker exists");
    assert_eq!(writes_after, 1, "approved retry executes exactly once");
    assert_eq!(applied.status.as_deref(), Some("applied"));
    assert!(applied.applied_at.is_some());
    assert!(applied.reason.is_none());
    assert!(applied.blocked_at.is_none());

    run_with_policy(&app.state.db, &[migration], true, &approvals)
        .await
        .expect("applied rerun");
    let final_writes = app
        .state
        .db
        .raw()
        .collection::<Document>("migration_probe")
        .count_documents(doc! {}, None)
        .await
        .expect("count idempotent writes");
    assert_eq!(final_writes, 1, "applied marker prevents re-execution");

    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn legacy_empty_admin_acl_is_materialized_once() {
    let app = common::TestApp::start().await;
    let users = app.state.db.raw().collection::<Document>("admin_users");
    users
        .insert_one(
            doc! {
                "user_id": "legacy-empty-acl",
                "username": "legacy-empty-acl",
                "password_hash": "unused",
                "created_at": chrono::Utc::now().to_rfc3339(),
                "last_login_at": mongodb::bson::Bson::Null,
                "workspaces": [],
                "default_workspace": mongodb::bson::Bson::Null,
            },
            None,
        )
        .await
        .expect("seed legacy admin");
    app.state
        .db
        .migrations()
        .delete_one(doc! { "_id": "2026_07_037_materialize_admin_acl" }, None)
        .await
        .expect("clear m037 marker");

    wechatagent::db::migrations::run(&app.state.db)
        .await
        .expect("run m037");
    let migrated = users
        .find_one(doc! { "user_id": "legacy-empty-acl" }, None)
        .await
        .expect("read migrated admin")
        .expect("migrated admin exists");
    assert_eq!(
        migrated.get_array("workspaces").expect("workspaces"),
        &vec![mongodb::bson::Bson::String("default".into())]
    );
    assert_eq!(migrated.get_str("default_workspace"), Ok("default"));

    app.cleanup().await;
}

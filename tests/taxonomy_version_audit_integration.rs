//! 回归（Stage4 孤儿 #4）：system_taxonomies 版本 handler（publish / rollout /
//! rollback）历史上不接 `AuthenticatedAdmin`——全局字典任一 admin 皆可改且改动
//! 无迹可查。本系统无 RBAC 角色模型，"谁有权改全局字典" 红线/文档均无定义，故
//! **不加拦截门**（保持策略型孤儿现状），只补最小可观测：改动成功后写一条
//! `taxonomy_version_changed` 审计事件，记录改动主体（who）与目标 scope（what）。
//!
//! 本测试直调三个 handler（已从 `pub(super)` 提为 `pub`，沿用孤儿 #3
//! `prompt_publish_evolution_guard` 的直调范式——审计写入在 handler 内、非纯数据
//! 层，测数据层无法覆盖），断言：
//!   1. publish → 写一条 action=publish、adminUsername/scope/kind/valueId 正确的事件；
//!   2. rollout → 写一条 action=rollout 的事件；
//!   3. rollback → 写一条 action=rollback、且 valueId 指向被回滚到的版本的事件；
//!   4. isGlobalScope 对 scope="global" 为 true。
//!
//! ## 运行
//! ```sh
//! cargo test --test taxonomy_version_audit_integration -- --ignored --nocapture
//! ```

mod common;

use axum::extract::{Path, State};
use axum::Extension;
use mongodb::bson::{doc, DateTime};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{TaxonomyEntry, TaxonomyValue};

use crate::common::TestApp;

/// 构造一条 global taxonomy 取值条目。
fn make_entry(
    workspace_id: &str,
    kind: &str,
    id: &str,
    version: i32,
    current: bool,
    prev: Option<i32>,
) -> TaxonomyEntry {
    TaxonomyEntry {
        id: None,
        workspace_id: workspace_id.to_string(),
        scope: "global".to_string(),
        kind: kind.to_string(),
        value: TaxonomyValue {
            id: id.to_string(),
            display_name: "审计测试取值".to_string(),
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
        previous_version: prev,
        seeded_by: Some("manual".to_string()),
    }
}

fn admin(username: &str, workspace: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: format!("uid-{username}"),
        username: username.to_string(),
        current_workspace: workspace.to_string(),
    }
}

#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn publish_taxonomy_writes_audit_event_with_admin_identity() {
    let app = TestApp::start().await;
    let coll = app.state.db.collection_system_taxonomies();

    let entry = make_entry(
        &app.state.config.default_workspace_id,
        "customer_stage",
        "orphan4_publish",
        1,
        true,
        None,
    );
    let inserted = coll.insert_one(&entry, None).await.expect("insert");
    let oid = inserted.inserted_id.as_object_id().expect("oid");

    let _ = wechatagent::routes::admin_ops_versions::publish_taxonomy_version(
        State(app.state.clone()),
        Extension(admin("alice", &app.state.config.default_workspace_id)),
        Path(oid.to_hex()),
    )
    .await
    .expect("publish handler ok");

    let ev = app
        .state
        .db
        .events()
        .find_one(
            doc! { "kind": "taxonomy_version_changed", "details.valueId": "orphan4_publish" },
            None,
        )
        .await
        .expect("query events")
        .expect("audit event must exist");

    assert_eq!(ev.kind, "taxonomy_version_changed");
    let details = ev.details.expect("details present");
    assert_eq!(details.get_str("action").unwrap(), "publish");
    assert_eq!(details.get_str("adminUsername").unwrap(), "alice");
    assert_eq!(details.get_str("scope").unwrap(), "global");
    assert_eq!(details.get_str("kind").unwrap(), "customer_stage");
    assert_eq!(details.get_str("valueId").unwrap(), "orphan4_publish");
    assert!(
        details.get_bool("isGlobalScope").unwrap(),
        "scope=global 应标 isGlobalScope=true"
    );
    // publish 新建 version 2（source.version=1 → next=2）
    assert_eq!(details.get_i32("version").unwrap(), 2);
}

#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn rollout_taxonomy_writes_audit_event() {
    let app = TestApp::start().await;
    let coll = app.state.db.collection_system_taxonomies();

    let entry = make_entry(
        &app.state.config.default_workspace_id,
        "intent_level",
        "orphan4_rollout",
        3,
        false,
        Some(2),
    );
    let inserted = coll.insert_one(&entry, None).await.expect("insert");
    let oid = inserted.inserted_id.as_object_id().expect("oid");

    let _ = wechatagent::routes::admin_ops_versions::rollout_taxonomy_version(
        State(app.state.clone()),
        Extension(admin("bob", &app.state.config.default_workspace_id)),
        Path(oid.to_hex()),
    )
    .await
    .expect("rollout handler ok");

    let ev = app
        .state
        .db
        .events()
        .find_one(
            doc! { "kind": "taxonomy_version_changed", "details.valueId": "orphan4_rollout" },
            None,
        )
        .await
        .expect("query events")
        .expect("audit event must exist");

    let details = ev.details.expect("details present");
    assert_eq!(details.get_str("action").unwrap(), "rollout");
    assert_eq!(details.get_str("adminUsername").unwrap(), "bob");
    assert_eq!(details.get_i32("version").unwrap(), 3);
}

#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn rollback_taxonomy_writes_audit_event_for_restored_version() {
    let app = TestApp::start().await;
    let coll = app.state.db.collection_system_taxonomies();

    // 种 v1（历史）+ v2（current，previous_version=1）
    let v1 = make_entry(
        &app.state.config.default_workspace_id,
        "objection_type",
        "orphan4_rollback",
        1,
        false,
        None,
    );
    coll.insert_one(&v1, None).await.expect("insert v1");
    let v2 = make_entry(
        &app.state.config.default_workspace_id,
        "objection_type",
        "orphan4_rollback",
        2,
        true,
        Some(1),
    );
    let inserted = coll.insert_one(&v2, None).await.expect("insert v2");
    let v2_oid = inserted.inserted_id.as_object_id().expect("oid");

    let _ = wechatagent::routes::admin_ops_versions::rollback_taxonomy_version(
        State(app.state.clone()),
        Extension(admin("carol", &app.state.config.default_workspace_id)),
        Path(v2_oid.to_hex()),
    )
    .await
    .expect("rollback handler ok");

    let ev = app
        .state
        .db
        .events()
        .find_one(
            doc! { "kind": "taxonomy_version_changed", "details.action": "rollback" },
            None,
        )
        .await
        .expect("query events")
        .expect("audit event must exist");

    let details = ev.details.expect("details present");
    assert_eq!(details.get_str("adminUsername").unwrap(), "carol");
    assert_eq!(details.get_str("valueId").unwrap(), "orphan4_rollback");
    // rollback 记录被回滚到的 prev 版本（v1）
    assert_eq!(details.get_i32("version").unwrap(), 1);
}

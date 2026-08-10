//! SR-168：字典运行字段完整投影与 dirty PATCH 的真实 Router/Mongo 红线。
//!
//! 经认证 API 先 GET 验证 priority/terminal/reactivation 三个运行字段完整投影，
//! 再仅 PATCH label；响应与强类型 Mongo 读回都必须保留三个运行字段原值。
//!
//! ## 运行
//! ```sh
//! cargo test --test taxonomy_flags_e2e -- --ignored --nocapture
//! ```

mod common;

use axum::Router;
use chrono::Utc;
use mongodb::bson::{doc, DateTime};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::{session::create_session, AdminUser, SESSION_COOKIE_NAME};
use wechatagent::db::config_generation::{read_generation, TAXONOMY_NAMESPACE};
use wechatagent::models::{TaxonomyEntry, TaxonomyValue};
use wechatagent::routes::api_router;

use crate::common::TestApp;

/// 构造一条携带非默认运行语义的 active global taxonomy 条目。
fn make_entry(workspace_id: &str, kind: &str, id: &str) -> TaxonomyEntry {
    TaxonomyEntry {
        id: None,
        workspace_id: workspace_id.to_string(),
        scope: "global".to_string(),
        kind: kind.to_string(),
        value: TaxonomyValue {
            id: id.to_string(),
            display_name: "测试取值".to_string(),
            description: String::new(),
            aliases: vec![],
            status: "active".to_string(),
            priority_weight: Some(73),
            is_terminal: true,
            is_reactivation_target: true,
        },
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("manual".to_string()),
    }
}

async fn start_api(
    app: &TestApp,
    workspace_id: &str,
) -> (String, String, tokio::task::JoinHandle<()>) {
    let admin = AdminUser {
        user_id: "taxonomy_admin".to_string(),
        username: "taxonomy_admin".to_string(),
        password_hash: "x".to_string(),
        created_at: Utc::now(),
        last_login_at: None,
        workspaces: vec![workspace_id.to_string()],
        default_workspace: Some(workspace_id.to_string()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&admin, None)
        .await
        .expect("seed taxonomy admin");
    let session = create_session(&app.state.db, &admin, 1, workspace_id)
        .await
        .expect("create taxonomy admin session");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind API");
    let address = listener.local_addr().expect("API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn label_only_patch_preserves_runtime_fields_and_projects_them() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let coll = app.state.db.collection_system_taxonomies();

    let entry = make_entry(&workspace_id, "customer_stage", "task7_flag_target");
    let inserted = coll
        .insert_one(&entry, None)
        .await
        .expect("insert taxonomy entry");
    let oid = inserted.inserted_id.as_object_id().expect("object id");

    let (base_url, cookie, server) = start_api(&app, &workspace_id).await;
    let client = reqwest::Client::new();

    let listed = client
        .get(format!("{base_url}/admin/taxonomies?kind=customer_stage"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("list taxonomies");
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: serde_json::Value = listed.json().await.expect("decode taxonomy list");
    let projected = listed["items"]
        .as_array()
        .expect("taxonomy items")
        .iter()
        .find(|item| item["id"] == oid.to_hex())
        .expect("seeded taxonomy projected");
    assert_eq!(projected["value"]["priorityWeight"], 73);
    assert_eq!(projected["value"]["isTerminal"], true);
    assert_eq!(projected["value"]["isReactivationTarget"], true);

    let generation_before_patch = read_generation(&app.state.db, TAXONOMY_NAMESPACE, &workspace_id)
        .await
        .expect("read generation before patch");

    let patched = client
        .patch(format!("{base_url}/admin/taxonomies/{}", oid.to_hex()))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "label": "renamed only" }))
        .send()
        .await
        .expect("patch taxonomy label");
    assert_eq!(patched.status(), StatusCode::OK);
    let patched: serde_json::Value = patched.json().await.expect("decode patch response");
    assert_eq!(patched["item"]["value"]["label"], "renamed only");
    assert_eq!(patched["item"]["value"]["priorityWeight"], 73);
    assert_eq!(patched["item"]["value"]["isTerminal"], true);
    assert_eq!(patched["item"]["value"]["isReactivationTarget"], true);

    let reloaded: TaxonomyEntry = coll
        .find_one(doc! { "_id": oid }, None)
        .await
        .expect("find_one")
        .expect("entry present");
    assert_eq!(reloaded.value.display_name, "renamed only");
    assert_eq!(reloaded.value.priority_weight, Some(73));
    assert!(reloaded.value.is_terminal);
    assert!(reloaded.value.is_reactivation_target);
    let generation_after_patch = read_generation(&app.state.db, TAXONOMY_NAMESPACE, &workspace_id)
        .await
        .expect("read generation after patch");
    assert_eq!(
        generation_after_patch,
        generation_before_patch + 1,
        "PATCH row and generation must commit exactly once",
    );

    let created = client
        .post(format!("{base_url}/admin/taxonomies"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "scope": "global",
            "kind": "taxonomy_atomic_crud",
            "value": {
                "id": "created_then_deleted",
                "label": "Created",
                "aliases": ["created-alias"]
            }
        }))
        .send()
        .await
        .expect("create taxonomy");
    assert_eq!(created.status(), StatusCode::OK);
    let created: serde_json::Value = created.json().await.expect("decode create response");
    let created_id = created["item"]["id"]
        .as_str()
        .expect("created taxonomy id")
        .to_string();
    let generation_after_create = read_generation(&app.state.db, TAXONOMY_NAMESPACE, &workspace_id)
        .await
        .expect("read generation after create");
    assert_eq!(
        generation_after_create,
        generation_after_patch + 1,
        "CREATE row and generation must commit exactly once",
    );

    let deleted = client
        .delete(format!("{base_url}/admin/taxonomies/{created_id}"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("delete taxonomy");
    assert_eq!(deleted.status(), StatusCode::OK);
    let generation_after_delete = read_generation(&app.state.db, TAXONOMY_NAMESPACE, &workspace_id)
        .await
        .expect("read generation after delete");
    assert_eq!(
        generation_after_delete,
        generation_after_create + 1,
        "DELETE row and generation must commit exactly once",
    );
    let created_oid =
        mongodb::bson::oid::ObjectId::parse_str(&created_id).expect("created taxonomy ObjectId");
    let deleted_row = coll
        .find_one(doc! { "_id": created_oid }, None)
        .await
        .expect("load soft-deleted taxonomy")
        .expect("soft-deleted taxonomy remains");
    assert_eq!(deleted_row.value.status, "deprecated");

    server.abort();
}

//! 集成测试：domain_schemas 写入字段名 ↔ 读取字段名一致性（#1 serde 错配回归门）。
//!
//! 修复前路由层查询用 camelCase（workspaceId/isActive），而模型序列化为 snake_case，
//! 导致 load_active_domain_schema 恒 None、enforce_domain_attributes 从不执行。
//! 本测试用与 handler 等价的 DB 写入 + pub 读函数验证 round-trip。
//!
//! 全部 #[ignore]：需 Docker（testcontainers MongoDB），本地不跑、CI --ignored 跑。

mod common;

use mongodb::bson::{doc, DateTime};
use wechatagent::models::{DomainField, DomainSchema};
use wechatagent::routes::domain_schemas::{enforce_domain_attributes, load_active_domain_schema};

/// 构造一条 DomainSchema（与 create_domain_schema handler 构造的等价）。
fn make_schema(
    workspace: &str,
    schema_id: &str,
    is_active: bool,
    required_field: bool,
) -> DomainSchema {
    let now = DateTime::now();
    DomainSchema {
        id: None,
        schema_id: schema_id.to_string(),
        workspace_id: workspace.to_string(),
        name: format!("schema-{schema_id}"),
        version: 1,
        fields: vec![DomainField {
            name: "customer_stage".to_string(),
            label: "客户阶段".to_string(),
            kind: "enum".to_string(),
            required: required_field,
            allowed_values: Some(vec!["lead".to_string(), "won".to_string()]),
            alias_of: None,
        }],
        alias_dict: Default::default(),
        guard_dsl: None,
        is_active,
        created_at: now,
        updated_at: now,
    }
}

/// create→load round-trip：写入 active schema，load_active_domain_schema 应返回 Some。
/// 修复前 filter 用 {isActive:true}（camelCase 幽灵键）→ 恒 None → 本测试 fail（红→绿）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn load_active_finds_inserted_active_schema() {
    let app = common::TestApp::start().await;
    let ws = "ws-domain-test";
    let cfg = make_schema(ws, "sales_v1", true, true);
    app.state
        .db
        .domain_schemas()
        .insert_one(&cfg, None)
        .await
        .expect("insert schema");

    let loaded = load_active_domain_schema(&app.state.db, ws)
        .await
        .expect("load ok");
    assert!(
        loaded.is_some(),
        "插入 is_active=true 的 schema 后 load 必须返回 Some（修复前恒 None）"
    );
    let loaded = loaded.unwrap();
    assert_eq!(loaded.schema_id, "sales_v1");
    assert!(loaded.is_active);
}

/// load 链路打通后，enforce_domain_attributes 能拿到 active schema 并对缺 required 字段 reject。
/// 验证的是「active schema 真能被加载」这一 IO 链路（enforce 纯函数本身已有单测）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn enforce_rejects_missing_required_after_load() {
    let app = common::TestApp::start().await;
    let ws = "ws-enforce-test";
    let cfg = make_schema(ws, "sales_v1", true, true); // customer_stage required
    app.state
        .db
        .domain_schemas()
        .insert_one(&cfg, None)
        .await
        .expect("insert");

    let schema = load_active_domain_schema(&app.state.db, ws)
        .await
        .expect("load ok")
        .expect("active schema present");
    // 缺 required 字段 customer_stage → enforce 应 reject。
    let attrs = doc! { "other": "x" };
    let result = enforce_domain_attributes(&schema, &attrs);
    assert!(result.is_err(), "缺 required 字段必须被 enforce reject");
}

/// activate 互斥：两条 schema 都标 active 写入后，模拟 activate B 的 update_many→false + update_one→true，
/// load 应返回 B。验证 activate 那段 $set { is_active } 的字段名命中（修复前写进 camelCase 幽灵键）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn activate_switches_active_via_snake_case_set() {
    let app = common::TestApp::start().await;
    let ws = "ws-activate-test";
    let col = app.state.db.domain_schemas();
    col.insert_one(&make_schema(ws, "a", true, false), None)
        .await
        .expect("insert a");
    col.insert_one(&make_schema(ws, "b", false, false), None)
        .await
        .expect("insert b");

    // 等价 activate B：先把本 ws 全部 is_active 置 false，再把 B 置 true（snake_case）。
    col.update_many(
        doc! { "workspace_id": ws, "is_active": true },
        doc! { "$set": { "is_active": false } },
        None,
    )
    .await
    .expect("deactivate all");
    col.update_one(
        doc! { "workspace_id": ws, "schema_id": "b" },
        doc! { "$set": { "is_active": true } },
        None,
    )
    .await
    .expect("activate b");

    let loaded = load_active_domain_schema(&app.state.db, ws)
        .await
        .expect("load")
        .expect("some");
    assert_eq!(loaded.schema_id, "b", "activate B 后 load 应返回 B");
}

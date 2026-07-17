//! Task 7 / D6：字典 `is_terminal` / `is_reactivation_target` 可配置——序列化 +
//! 落库往返集成测试（`#[ignore]`，需要 Docker / testcontainers MongoDB）。
//!
//! `admin_taxonomies` 的 create / patch handler 是 `pub(super)`，测试 crate 不可直调
//! （沿用 `workspace_isolation.rs` / `operation_view_integration.rs` 的先例：核心
//! 行为下沉到数据层时测数据层）。这里验证的是 Task 7 唯一有回归风险的环节——
//! patch handler 用 **camelCase** 键 `value.isTerminal` / `value.isReactivationTarget`
//! 写 `$set`，必须与 `TaxonomyValue`（`#[serde(rename_all = "camelCase")]`）反序列化
//! 的 wire 键完全对齐（本项目曾 4 次踩 rename 坑）。键名写错会静默失效——`update_one`
//! 仍 matched，但读回的 struct 字段不变。
//!
//! ## 运行
//! ```sh
//! cargo test --test taxonomy_flags_e2e -- --ignored --nocapture
//! ```

mod common;

use mongodb::bson::{doc, DateTime};
use wechatagent::models::{TaxonomyEntry, TaxonomyValue};

use crate::common::TestApp;

/// 构造一条 active 的 global taxonomy 取值条目（两 flag 初值 false）。
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
            priority_weight: None,
            is_terminal: false,
            is_reactivation_target: false,
        },
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("manual".to_string()),
    }
}

#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn patch_camelcase_keys_persist_both_flags() {
    let app = TestApp::start().await;
    let coll = app.state.db.collection_system_taxonomies();

    // 种一条两 flag = false 的条目。
    let entry = make_entry(
        &app.state.config.default_workspace_id,
        "customer_stage",
        "task7_flag_target",
    );
    let inserted = coll
        .insert_one(&entry, None)
        .await
        .expect("insert taxonomy entry");
    let oid = inserted.inserted_id.as_object_id().expect("object id");

    // 复刻 patch_taxonomy 的 set_doc：camelCase 键（与 value.displayName 同款）。
    coll.update_one(
        doc! { "_id": oid },
        doc! { "$set": {
            "value.isTerminal": true,
            "value.isReactivationTarget": true,
        } },
        None,
    )
    .await
    .expect("update flags");

    // 读回并反序列化为强类型 struct——若键名 casing 写错，update 仍 matched 但这两
    // 字段会保持 false，断言失败。
    let reloaded: TaxonomyEntry = coll
        .find_one(doc! { "_id": oid }, None)
        .await
        .expect("find_one")
        .expect("entry present");

    assert!(
        reloaded.value.is_terminal,
        "value.isTerminal camelCase 键应落库并反序列化为 true"
    );
    assert!(
        reloaded.value.is_reactivation_target,
        "value.isReactivationTarget camelCase 键应落库并反序列化为 true"
    );
}

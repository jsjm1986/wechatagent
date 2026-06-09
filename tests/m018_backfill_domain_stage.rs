//! m018 回填语义验证:把 contact 顶层残留的 customer_stage / intent_level /
//! customer_stage_updated_at 并入 domain_attributes(只回填、不 $unset,且现有 domain
//! 值优先于顶层陈旧值)。
//!
//! `TestApp::start()` 在空库上已跑过 m018(账册存在),故这里手动插入带顶层残留的
//! contact 文档后,**直接调用** `m018::run_step` 验证回填行为。
//!
//! 默认 `#[ignore]`,需要 Docker;CI 用 `cargo test -- --ignored` 触发。

mod common;

use mongodb::bson::{doc, Document};
use wechatagent::db::migrations::m018_backfill_domain_stage_from_legacy_top as m018;

/// 取某 wxid 的 contact 原始 BSON 文档(绕过 Contact serde,直接看物理字段)。
async fn raw_contact(app: &common::TestApp, wxid: &str) -> Document {
    app.state
        .db
        .raw()
        .collection::<Document>("contacts")
        .find_one(doc! { "wxid": wxid }, None)
        .await
        .expect("query raw contact")
        .expect("contact exists")
}

#[tokio::test]
#[ignore]
async fn backfills_top_level_into_domain_and_keeps_domain_winner() {
    let app = common::TestApp::start().await;
    let contacts = app
        .state
        .db
        .raw()
        .collection::<Document>("contacts");

    // 场景 A:只有顶层残留,domain_attributes 完全缺失 → 应把三字段搬入 domain。
    contacts
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "legacy_only_top",
                "agent_status": "managed",
                "customer_stage": "solution_fit",
                "intent_level": "high",
                "customer_stage_updated_at": mongodb::bson::DateTime::from_millis(1_000),
            },
            None,
        )
        .await
        .expect("insert scenario A");

    // 场景 B:domain 已有较新 stage,顶层是陈旧值 → domain 值必须胜出(不被顶层盖掉)。
    contacts
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "domain_wins",
                "agent_status": "managed",
                "customer_stage": "new_contact",          // 顶层陈旧
                "intent_level": "low",                     // 顶层陈旧
                "domain_attributes": {
                    "customer_stage": "commitment_followup", // domain 较新
                    "intent_level": "high",
                },
            },
            None,
        )
        .await
        .expect("insert scenario B");

    // 场景 C:无任何顶层残留字段 → 不应被 filter 命中、不被改动。
    contacts
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "clean_contact",
                "agent_status": "managed",
                "domain_attributes": { "customer_stage": "need_discovery" },
            },
            None,
        )
        .await
        .expect("insert scenario C");

    // 执行回填。
    m018::run_step(&app.state.db).await.expect("run m018");

    // 场景 A:三字段进入 domain_attributes;顶层保留(可逆,不 $unset)。
    let a = raw_contact(&app, "legacy_only_top").await;
    let a_domain = a.get_document("domain_attributes").expect("A has domain");
    assert_eq!(a_domain.get_str("customer_stage").ok(), Some("solution_fit"));
    assert_eq!(a_domain.get_str("intent_level").ok(), Some("high"));
    assert!(
        a_domain.contains_key("customer_stage_updated_at"),
        "A: updated_at 应被搬入 domain"
    );
    assert!(
        a.contains_key("customer_stage"),
        "A: 顶层保留(只回填不 $unset,保证可逆)"
    );

    // 场景 B:domain 较新值胜出,未被顶层陈旧值覆盖。
    let b = raw_contact(&app, "domain_wins").await;
    let b_domain = b.get_document("domain_attributes").expect("B has domain");
    assert_eq!(
        b_domain.get_str("customer_stage").ok(),
        Some("commitment_followup"),
        "B: 现有 domain 值必须优先于顶层陈旧值(新覆旧)"
    );
    assert_eq!(b_domain.get_str("intent_level").ok(), Some("high"));

    // 场景 C:无顶层字段不被 filter 命中,domain 原样不动。
    let c = raw_contact(&app, "clean_contact").await;
    let c_domain = c.get_document("domain_attributes").expect("C has domain");
    assert_eq!(c_domain.get_str("customer_stage").ok(), Some("need_discovery"));

    // 二次执行幂等:domain 已有值,mergeObjects 结果不变。
    m018::run_step(&app.state.db).await.expect("rerun m018");
    let a2 = raw_contact(&app, "legacy_only_top").await;
    let a2_domain = a2.get_document("domain_attributes").expect("A2 has domain");
    assert_eq!(a2_domain.get_str("customer_stage").ok(), Some("solution_fit"));
    let b2 = raw_contact(&app, "domain_wins").await;
    let b2_domain = b2.get_document("domain_attributes").expect("B2 has domain");
    assert_eq!(
        b2_domain.get_str("customer_stage").ok(),
        Some("commitment_followup"),
        "二次执行后 domain 仍是较新值"
    );
}

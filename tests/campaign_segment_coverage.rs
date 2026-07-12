//! KC-05 端到端：缺 verification/eventKind 的老成交客户，回填后 + 粗筛口径对齐后被纳入。
//!
//! 默认 #[ignore]，需 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, Document};
use wechatagent::db::migrations::m030_backfill_outcome_event_defaults;

use crate::common::TestApp;

/// 直接插一条 outcome_events 缺 verification/eventKind 的"老成交"contact（raw Document
/// 绕过 serde 默认，模拟 §4.5 上线前的 BSON 形态），跑 m030 后两键补齐为默认值。
#[tokio::test]
#[ignore]
async fn m030_backfills_missing_verification_and_event_kind() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();

    // raw insert：outcome_events 元素只有 productRef，无 verification/eventKind。
    raw.collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "wxid": "old_buyer",
                "agent_status": "managed",
                "outcome_events": [ {
                    "markedAt": mongodb::bson::DateTime::from_millis(0),
                    "source": "manual",
                    "productRef": { "productId": "vip", "name": "P", "quantity": 1 },
                } ],
            },
            None,
        )
        .await
        .expect("seed legacy contact");

    m030_backfill_outcome_event_defaults::run_step(&app.state.db)
        .await
        .expect("run m030");

    let after = raw
        .collection::<Document>("contacts")
        .find_one(doc! { "wxid": "old_buyer", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("contact exists");
    let ev = after.get_array("outcome_events").unwrap()[0].as_document().unwrap();
    assert_eq!(ev.get_str("verification").unwrap(), "staff_confirmed", "缺 verification 补默认");
    assert_eq!(ev.get_str("eventKind").unwrap(), "deal", "缺 eventKind 补默认");
    // productRef 原值不被破坏
    assert_eq!(
        ev.get_document("productRef").unwrap().get_str("productId").unwrap(),
        "vip"
    );
}

/// m030 幂等：已有 conversation_inferred/reversal 的元素原值不被默认值覆盖。
#[tokio::test]
#[ignore]
async fn m030_does_not_overwrite_existing_values() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();

    raw.collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "wxid": "explicit_buyer",
                "agent_status": "managed",
                "outcome_events": [ {
                    "markedAt": mongodb::bson::DateTime::from_millis(0),
                    "source": "manual",
                    "verification": "conversation_inferred",
                    "eventKind": "reversal",
                    "productRef": { "productId": "vip", "name": "P", "quantity": 1 },
                } ],
            },
            None,
        )
        .await
        .expect("seed");

    m030_backfill_outcome_event_defaults::run_step(&app.state.db)
        .await
        .expect("run m030 once");
    // 再跑一次验幂等
    m030_backfill_outcome_event_defaults::run_step(&app.state.db)
        .await
        .expect("run m030 twice");

    let after = raw
        .collection::<Document>("contacts")
        .find_one(doc! { "wxid": "explicit_buyer", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("exists");
    let ev = after.get_array("outcome_events").unwrap()[0].as_document().unwrap();
    assert_eq!(ev.get_str("verification").unwrap(), "conversation_inferred", "已有值不被覆盖");
    assert_eq!(ev.get_str("eventKind").unwrap(), "reversal", "已有 reversal 不被改成 deal");
}

/// 端到端：缺字段老成交客户，用防线 A 等价的粗筛查询能命中(回填前靠 $exists/$ne 就纳入)。
/// 手工构造与 build_segment_coarse_filter 等价的 $elemMatch(集成测在 crate 外不可直调
/// pub(super) 函数)，验证缺字段老成交被粗筛纳入。
#[tokio::test]
#[ignore]
async fn coarse_query_includes_legacy_event_missing_fields() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();

    raw.collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "wxid": "legacy_vip",
                "agent_status": "managed",
                "outcome_events": [ {
                    "markedAt": mongodb::bson::DateTime::from_millis(0),
                    "source": "manual",
                    "productRef": { "productId": "vip", "name": "P", "quantity": 1 },
                } ],
            },
            None,
        )
        .await
        .expect("seed legacy");

    // 与 build_segment_coarse_filter 防线 A 等价的粗筛(product_ids=["vip"])。
    let coarse = doc! {
        "workspace_id": &ws,
        "account_id": "acc",
        "agent_status": "managed",
        "outcome_events": { "$elemMatch": {
            "productRef.productId": { "$in": ["vip"] },
            "$and": [
                { "$or": [
                    { "verification": { "$in": ["staff_confirmed", "payment_verified"] } },
                    { "verification": { "$exists": false } },
                ]},
                { "eventKind": { "$ne": "reversal" } },
            ],
        }},
    };
    let count = raw
        .collection::<Document>("contacts")
        .count_documents(coarse, None)
        .await
        .expect("count");
    assert_eq!(count, 1, "缺 verification/eventKind 的老成交老客户须被粗筛纳入(KC-05 修复)");
}

/// C1 回归哨兵：只有 outcome_events(无 deal_events)的文档,跑 m030 后**不得**被凭空
/// 追加 deal_events:[]。因 Contact.outcome_events 带 #[serde(alias="deal_events")]
/// (models.rs:248),两键同现会触发 serde duplicate_field、类型化 Collection<Contact>
/// 读取崩溃。故用**类型化** contacts() 读回(而非 raw Document)——若 m030 造了 deal_events,
/// 这里反序列化直接 panic;并显式断言 raw 文档里 deal_events 键不存在。
/// 若 backfill_filter 退回共享 $or(C1 缺陷),本测立刻红。
#[tokio::test]
#[ignore]
async fn m030_does_not_create_deal_events_key_on_outcome_events_only_doc() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();

    // 只有 outcome_events(snake)、无 legacy deal_events。含 Contact 反序列化必需的
    // created_at/updated_at(非 Option 非 default),否则类型化读回会因缺这俩字段失败、
    // 掩盖本测真正要验的 C1(deal_events 双键 duplicate_field)。
    raw.collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "wxid": "outcome_only",
                "agent_status": "managed",
                "created_at": mongodb::bson::DateTime::from_millis(0),
                "updated_at": mongodb::bson::DateTime::from_millis(0),
                "outcome_events": [ {
                    "markedAt": mongodb::bson::DateTime::from_millis(0),
                    "source": "manual",
                    "productRef": { "productId": "vip", "name": "P", "quantity": 1 },
                } ],
            },
            None,
        )
        .await
        .expect("seed outcome-only");

    m030_backfill_outcome_event_defaults::run_step(&app.state.db)
        .await
        .expect("run m030");

    // (a) raw 层:deal_events 键必须不存在(m030 不得对缺该键的文档造键)。
    let raw_after = raw
        .collection::<Document>("contacts")
        .find_one(doc! { "wxid": "outcome_only", "workspace_id": &ws }, None)
        .await
        .expect("find raw")
        .expect("exists");
    assert!(
        raw_after.get("deal_events").is_none(),
        "m030 绝不能给只有 outcome_events 的文档凭空追加 deal_events(C1);实得 {:?}",
        raw_after.get("deal_events")
    );

    // (b) 类型化层:contacts() 反序列化必须成功(两键同现会 duplicate_field 崩)。
    let typed = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "outcome_only", "workspace_id": &ws }, None)
        .await
        .expect("类型化 Contact 读取不得因 deal_events/outcome_events 双键 duplicate_field 崩")
        .expect("contact exists");
    assert_eq!(typed.outcome_events.len(), 1, "回填后成交事件仍在");
    assert_eq!(typed.outcome_events[0].verification, "staff_confirmed", "缺 verification 补默认");
    assert_eq!(typed.outcome_events[0].event_kind, "deal", "缺 event_kind 补默认");
}

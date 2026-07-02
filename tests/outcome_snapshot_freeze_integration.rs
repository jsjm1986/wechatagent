//! `outcome_snapshot_freeze_integration` —— 成交事件**写侧**产品快照冻结集成测试。
//!
//! 读侧（`project_entitlements`）已有 ~40 单测，但写侧 `add_outcome_event_inner`
//! （shared.rs:1403，`pub(crate)`）从本 workspace active 产品表解引用、逐字段拷贝快照
//! 进 `OutcomeProductRef`（name/unit_price/sku/quantity/entitlement_days，shared.rs:1471-1479）
//! 这一路径**零测试**。本文件补上：驱动真实落库路径写一条带 product_id 的成交，断言
//! 快照被正确冻结，且**改价后历史快照不漂移**（订单式冻结、非活引用红线，models.rs:432-435）。
//!
//! **入口选择（重要）**：`add_outcome_event_inner` / `OutcomeEventInput` 均是 `pub(crate)`，
//! `mod shared` 私有——集成测试是独立 crate，无法直接调用（改成 `pub` 属于改 src，禁止）。
//! 故走**唯一公开、且真实调用 `add_outcome_event_inner` 的 handler**
//! `approve_suspected_deal`（admin_suspected_deals.rs:119 → :194）来驱动同一条冻结路径。
//! 这不是测试体内手工构造 `OutcomeProductRef`——快照完全由生产代码从 products 表解引用生成，
//! 满足「真调 add_outcome_event_inner」的反假绿要求。
//! 局限：approve 路径 `quantity` 恒 `None`（admin_suspected_deals.rs:206），故快照
//! `quantity` 冻结为默认 1（`unwrap_or(1).max(1)`），本测试相应断言 `==1`。
//!
//! 默认 `#[ignore]`，需 Docker（testcontainers MongoDB），由 CI `integration` job 跑 `--ignored`。

mod common;

use axum::extract::{Extension, Json, Path, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{AgentStatus, Contact, Product, SuspectedDealSignal};

use crate::common::TestApp;

/// 构造测试 admin auth context（current_workspace 决定 handler 可见范围）。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "test_admin".to_string(),
        username: "test_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一个 managed 状态的 Contact（approve 时按 contact_id workspace 隔离取回）。
/// 字段全量显式，参照 `tests/suspected_deal_e2e.rs::make_contact`。
fn make_contact(ws: &str, wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: ws.to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("测试客户".to_string()),
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        manual_tags_updated_at: None,
        manual_tags_by: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: Some("new_contact".to_string()),
        operation_state_reason: None,
        operation_state_confidence: Some(7),
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: Some(now),
        last_inbound_at: Some(now),
        last_outbound_at: None,
        last_agent_run_at: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        outcome_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

/// 构造一条 active 产品（带 sku + attributes.entitlement_days）。
/// 参照 `tests/products_workspace_isolation.rs::ws_product`。
fn active_product(ws: &str, product_id: &str, name: &str, price: i64) -> Product {
    let now = DateTime::now();
    Product {
        id: None,
        workspace_id: ws.to_string(),
        product_id: product_id.to_string(),
        name: name.to_string(),
        price: Some(price),
        currency: Some("CNY".to_string()),
        sku: Some("VIP-Y".to_string()),
        status: "active".to_string(),
        summary: None,
        // entitlement_days_of 读 attributes.entitlement_days（i64/i32/f64 且 >0）。
        attributes: doc! { "entitlement_days": 365_i64 },
        created_at: now,
        updated_at: now,
    }
}

/// seed 一条 pending 待核实信号，返回其 _id 的 hex。
async fn seed_pending_signal(state: &wechatagent::routes::AppState, ws: &str, contact_id: &str) -> String {
    let now = DateTime::now();
    let signal = SuspectedDealSignal {
        id: None,
        workspace_id: ws.to_string(),
        account_id: "default".to_string(),
        contact_id: contact_id.to_string(),
        value: "疑似成交·待核实".to_string(),
        evidence: Some("客户说要下单年度会员".to_string()),
        confidence: 80,
        status: "pending".to_string(),
        occurrences: 1,
        first_seen_at: now,
        last_seen_at: now,
        reviewed_at: None,
        reviewed_by: None,
    };
    let res = state
        .db
        .collection_suspected_deal_signals()
        .insert_one(&signal, None)
        .await
        .expect("insert SuspectedDealSignal");
    res.inserted_id.as_object_id().unwrap().to_hex()
}

/// 写侧快照冻结 + 改价不漂移。
///
/// 1. seed active product（price=19900、name=年度会员、sku=VIP-Y、entitlement_days=365）
///    + managed contact + pending 信号。
/// 2. approve（带 product_id）→ 真实走 add_outcome_event_inner 从产品表解引用冻结快照。
/// 3. 断言 product_ref 各字段冻结正确（name/unit_price/sku/quantity/entitlement_days）。
/// 4. **红线**：改 products 表 price=99900 / name=改名了 后重读 contact，
///    断言历史 outcome_events[0].product_ref 仍是原值（快照非活引用，改价不污染历史）。
#[tokio::test]
#[ignore]
async fn outcome_product_ref_freezes_snapshot_and_survives_later_price_change() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // seed active product。
    let product = active_product(&ws, "annual-vip", "年度会员", 19900);
    app.state
        .db
        .products()
        .insert_one(&product, None)
        .await
        .expect("insert active product");

    // seed managed contact。
    let contact = make_contact(&ws, "cust_freeze_1");
    let contact_id = contact.id.unwrap().to_hex();
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // seed pending 信号并 approve（带 product_id）→ 真实驱动 add_outcome_event_inner。
    let signal_id = seed_pending_signal(&app.state, &ws, &contact_id).await;
    let _resp = wechatagent::routes::admin_suspected_deals::approve_suspected_deal(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(signal_id.clone()),
        Json(
            serde_json::from_value(json!({
                "amount": 19900,
                "currency": "CNY",
                "productId": "annual-vip"
            }))
            .unwrap(),
        ),
    )
    .await
    .expect("approve should land a deal with frozen product snapshot");

    // 断言：快照被正确冻结（由生产代码从 products 表解引用，非测试手工构造）。
    let oid = ObjectId::parse_str(&contact_id).unwrap();
    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": oid }, None)
        .await
        .expect("find contact")
        .expect("contact exists");
    assert_eq!(after.outcome_events.len(), 1, "approve 应落一条成交事件");
    let product_ref = after.outcome_events[0]
        .product_ref
        .as_ref()
        .expect("给定 product_id 时 product_ref 必须为 Some（快照已冻结）");
    assert_eq!(product_ref.product_id, "annual-vip");
    assert_eq!(product_ref.name, "年度会员", "name 应冻结成交当时值");
    assert_eq!(product_ref.unit_price, Some(19900), "unit_price 应冻结产品当时 price");
    assert_eq!(product_ref.sku.as_deref(), Some("VIP-Y"), "sku 应冻结");
    // approve 路径 quantity 恒 None → 默认冻结为 1（unwrap_or(1).max(1)）。
    assert_eq!(product_ref.quantity, 1, "quantity 无输入时冻结为默认 1");
    assert_eq!(
        product_ref.entitlement_days,
        Some(365),
        "entitlement_days 应从 attributes 冻结（G4 #4：产品日后下架也不丢已购客户时效）"
    );

    // ── 红线：改价后历史快照不漂移 ──
    // 改 products 表该产品 price=99900 / name=改名了（产品仍 active）。
    app.state
        .db
        .products()
        .update_one(
            doc! { "workspace_id": &ws, "product_id": "annual-vip" },
            doc! { "$set": { "price": 99900_i64, "name": "改名了" } },
            None,
        )
        .await
        .expect("update product price/name");

    // 重读 contact，断言历史成交的 product_ref 仍是原值（快照是订单式冻结、非活引用）。
    let after_change = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": oid }, None)
        .await
        .expect("find contact after price change")
        .expect("contact still exists");
    let frozen = after_change.outcome_events[0]
        .product_ref
        .as_ref()
        .expect("product_ref 仍应为 Some");
    assert_eq!(
        frozen.name, "年度会员",
        "红线：产品改名后历史成交快照 name 不得漂移（订单式冻结）"
    );
    assert_eq!(
        frozen.unit_price,
        Some(19900),
        "红线：产品改价后历史成交快照 unit_price 不得漂移（19900 而非 99900）"
    );
    assert_eq!(frozen.entitlement_days, Some(365), "红线：entitlement_days 快照不漂移");
}

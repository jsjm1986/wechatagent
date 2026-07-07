//! `suspected_deal_e2e` —— F23 疑似成交待核实闭环（方案B 全链）端到端集成测试。
//!
//! 全部 `#[ignore]`（需要 Docker / testcontainers MongoDB，本地无 Docker 时跳过，
//! 由 CI `integration` job 跑 `--ignored`）。
//!
//! 直调 route handler 真函数（不经 axum HTTP 层），与 `tests/ask_human_phase1_e2e.rs`
//! / `tests/domain_profile_e2e.rs` 同惯例。
//!
//! **红线断言**：approve 落正式成交时 `verification == "staff_confirmed"`——AI 永不
//! 直写 outcome，只有运营核实 approve 才落成交。
//!
//! 覆盖：
//! 1. seed pending SuspectedDealSignal → GET list（?status=pending）→ 断言含该条
//!    evidence / confidence。
//! 2. POST approve（body 带 amount/currency）→ 断言 contact.outcome_events 多一条
//!    verification=="staff_confirmed"，signal status==approved。
//! 3. 另一条 reject → status==rejected。

mod common;

use axum::extract::{Extension, Json, Path, Query, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::{json, Value};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{AgentStatus, Contact, SuspectedDealSignal};

/// 构造测试 admin auth context（current_workspace 决定 handler 可见范围）。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "test_admin".to_string(),
        username: "test_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一个 managed 状态的 Contact（approve 时按 contact_id workspace 隔离取回）。
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
        avatar_url: None,
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

/// seed 一条待核实信号（status 可控），返回其 _id 的 hex。
async fn seed_signal(
    state: &wechatagent::routes::AppState,
    ws: &str,
    contact_id: &str,
    status: &str,
) -> String {
    let now = DateTime::now();
    let signal = SuspectedDealSignal {
        id: None,
        workspace_id: ws.to_string(),
        account_id: "default".to_string(),
        contact_id: contact_id.to_string(),
        value: "疑似成交·待核实".to_string(),
        evidence: Some("客户说要下单".to_string()),
        confidence: 75,
        status: status.to_string(),
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

/// 测试 1+2：seed pending → list 含 evidence/confidence → approve（带 amount/currency）
/// → contact 多一条 verification=staff_confirmed 成交，signal status==approved。
#[tokio::test]
#[ignore]
async fn list_then_approve_lands_staff_confirmed_deal() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let contact = make_contact(&ws, "cust_deal_1");
    let contact_id = contact.id.unwrap().to_hex();
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let signal_id = seed_signal(&app.state, &ws, &contact_id, "pending").await;

    // 1. GET list（默认 pending）→ 断言含该条 evidence/confidence。
    let listed = wechatagent::routes::admin_suspected_deals::list_suspected_deals(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Query(serde_json::from_value(json!({})).unwrap()),
    )
    .await
    .expect("list ok");
    let items = listed.0.get("items").and_then(Value::as_array).unwrap();
    assert_eq!(items.len(), 1, "应列出 1 条 pending 信号");
    assert_eq!(items[0]["evidence"], "客户说要下单");
    assert_eq!(items[0]["confidence"], 75);
    assert_eq!(items[0]["status"], "pending");

    // 2. approve（body 带 amount/currency）。
    let _resp = wechatagent::routes::admin_suspected_deals::approve_suspected_deal(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(signal_id.clone()),
        Json(serde_json::from_value(json!({ "amount": 9900, "currency": "CNY" })).unwrap()),
    )
    .await
    .expect("approve ok");

    // 断言：contact.outcome_events 多一条 verification=="staff_confirmed"。
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
    let ev = &after.outcome_events[0];
    assert_eq!(
        ev.verification, "staff_confirmed",
        "红线：approve 落成交 verification 必须是 staff_confirmed"
    );
    assert_eq!(ev.amount, Some(9900));
    assert_eq!(ev.currency.as_deref(), Some("CNY"));

    // 断言：signal status==approved。
    let sid = ObjectId::parse_str(&signal_id).unwrap();
    let updated = app
        .state
        .db
        .collection_suspected_deal_signals()
        .find_one(doc! { "_id": sid }, None)
        .await
        .expect("find signal")
        .expect("signal exists");
    assert_eq!(updated.status, "approved");
    assert!(updated.reviewed_at.is_some());
    assert_eq!(updated.reviewed_by.as_deref(), Some("test_admin"));
}

/// 测试 3：reject 一条 pending → status==rejected + 记 reason。
#[tokio::test]
#[ignore]
async fn reject_marks_rejected() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let contact = make_contact(&ws, "cust_deal_2");
    let contact_id = contact.id.unwrap().to_hex();
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let signal_id = seed_signal(&app.state, &ws, &contact_id, "pending").await;

    let _resp = wechatagent::routes::admin_suspected_deals::reject_suspected_deal(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(signal_id.clone()),
        Json(serde_json::from_value(json!({ "reason": "误判，实际只是咨询" })).unwrap()),
    )
    .await
    .expect("reject ok");

    let sid = ObjectId::parse_str(&signal_id).unwrap();
    let updated = app
        .state
        .db
        .collection_suspected_deal_signals()
        .find_one(doc! { "_id": sid }, None)
        .await
        .expect("find signal")
        .expect("signal exists");
    assert_eq!(updated.status, "rejected");

    // contact 不应有任何成交事件（reject 绝不落成交）。
    let oid = ObjectId::parse_str(&contact_id).unwrap();
    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": oid }, None)
        .await
        .expect("find contact")
        .expect("contact exists");
    assert!(after.outcome_events.is_empty(), "reject 绝不落成交");
}

/// 回归钉（Stage4 孤儿#suspected_deal.2 修复）：unique 锚从全量 (ws,contact) 改为
/// 仅 status="pending" 的部分唯一索引后，同一 contact 在**终态记录已存在**时仍能
/// 因真实二次成交再生成一条新 pending。
///
/// 修复前（全量 unique (ws,contact)）：seed 一条 approved 后，再 insert 一条 pending
/// → 同 (ws,contact) → E11000 DuplicateKey，新 pending 被吞，二次成交永远进不了队列。
/// 修复后（partial unique on status=pending）：approved 不在部分索引内、不占槽，新
/// pending 插入成功；但第二条 pending 仍与第一条 pending 冲突 → 去重意图保留。
///
/// 本测试直接锤 `TestApp::start()` 经 `ensure_indexes` 建出的**真实索引**（非平行
/// 重写逻辑）。把 indexes.rs 的部分唯一索引改回全量 unique → 断言 1（approved 共存
/// 下新 pending 插入成功）翻红。
#[tokio::test]
#[ignore]
async fn terminal_signal_does_not_block_new_pending() {
    use mongodb::error::{ErrorKind, WriteFailure};

    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let contact_id = ObjectId::new().to_hex();

    // 先落一条 approved（模拟上一轮成交已核实闭环）。
    seed_signal(&app.state, &ws, &contact_id, "approved").await;

    // 断言 1：同一 contact 的新 pending 必须能插入（修复前 = E11000 失败）。
    let new_pending = SuspectedDealSignal {
        id: None,
        workspace_id: ws.clone(),
        account_id: "default".to_string(),
        contact_id: contact_id.clone(),
        value: "二次成交·待核实".to_string(),
        evidence: Some("客户说要再下一单".to_string()),
        confidence: 80,
        status: "pending".to_string(),
        occurrences: 1,
        first_seen_at: DateTime::now(),
        last_seen_at: DateTime::now(),
        reviewed_at: None,
        reviewed_by: None,
    };
    app.state
        .db
        .collection_suspected_deal_signals()
        .insert_one(&new_pending, None)
        .await
        .expect("approved 存在时新 pending 必须能插入(修复前会 E11000)");

    // 断言 2：第二条 pending 仍被部分唯一索引拒（去重意图保留）。
    let dup_pending = SuspectedDealSignal {
        id: None,
        ..new_pending.clone()
    };
    let err = app
        .state
        .db
        .collection_suspected_deal_signals()
        .insert_one(&dup_pending, None)
        .await
        .expect_err("第二条 pending 应被部分唯一索引拒(去重)");
    let is_dup_key = matches!(
        err.kind.as_ref(),
        ErrorKind::Write(WriteFailure::WriteError(we)) if we.code == 11000
    );
    assert!(is_dup_key, "第二条 pending 应因 DuplicateKey(11000) 被拒,实际={err:?}");

    // 断言 3：该 contact 下恰好 1 条 pending + 1 条 approved 并存。
    let pending_count = app
        .state
        .db
        .collection_suspected_deal_signals()
        .count_documents(
            doc! { "workspace_id": &ws, "contact_id": &contact_id, "status": "pending" },
            None,
        )
        .await
        .expect("count pending");
    let approved_count = app
        .state
        .db
        .collection_suspected_deal_signals()
        .count_documents(
            doc! { "workspace_id": &ws, "contact_id": &contact_id, "status": "approved" },
            None,
        )
        .await
        .expect("count approved");
    assert_eq!(pending_count, 1, "应恰好 1 条 pending");
    assert_eq!(approved_count, 1, "approved 记录应仍在(未被覆盖)");
}

//! 主动发送台账端到端：ledger CRUD round-trip + 转化字段可空回填 round-trip +
//! workspace_id scope 隔离。需 Docker(testcontainers Mongo)，默认 `#[ignore]`，
//! CI integration job 用 `cargo test --test send_ledger_integration -- --ignored` 跑。
//!
//! 可见性说明（决定本文件能覆盖到哪些层）：
//! - `scan_send_ledger_outcomes` / `build_ledger_entry` / `record_send` 为 `pub(crate)`
//!   且 `send_ledger` 模块未对外 `pub use`，**跨 crate 不可见**（转化判定纯函数已由
//!   `src/agent/send_ledger.rs` 内联单测覆盖，本文件不重复）。
//! - 故集成测试走**公开路径**：直接对 `Database::agent_send_ledger()`(pub accessor)
//!   做集合 CRUD round-trip + workspace scope 验证（IDOR 数据层前提）。
#![cfg(test)]

mod common;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use mongodb::error::{ErrorKind, WriteFailure};
use wechatagent::agent::{recent_sends_for_contact, record_send_ledger, scan_send_ledger_outcomes};
use wechatagent::models::{
    AgentSendLedger, AgentStatus, Contact, ConversationMessage, MessageDirection,
};

/// 构造一条台账 fixture（snake_case 落库，与 models.rs 上 AgentSendLedger 同款）。
fn make_row(workspace: &str, contact: &str, kind: &str, target: &str) -> AgentSendLedger {
    AgentSendLedger {
        id: None,
        outbox_id: Some(ObjectId::new()),
        workspace_id: workspace.into(),
        account_id: "acct1".into(),
        contact_wxid: contact.into(),
        send_kind: kind.into(),
        target_id: target.into(),
        target_title: "fixture".into(),
        run_id: "run1".into(),
        trigger_reason: None,
        customer_stage_at_send: Some("意向".into()),
        sent_at: DateTime::now(),
        responded: None,
        response_window_hours: None,
        stage_advanced: None,
        outcome_evaluated_at: None,
    }
}

fn is_duplicate_key(error: &mongodb::error::Error) -> bool {
    match error.kind.as_ref() {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            matches!(write_error.code, 11000 | 11001)
        }
        ErrorKind::BulkWrite(failure) => failure
            .write_errors
            .as_ref()
            .is_some_and(|errors| errors.iter().any(|item| matches!(item.code, 11000 | 11001))),
        _ => false,
    }
}

// ── Test 1: 插入 → 回填转化字段 → 读回断言 ────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn ledger_roundtrip_and_outcome_update() {
    let app = common::TestApp::start().await;
    let coll = app.state.db.agent_send_ledger();

    // 1. 插入一条未评估的台账（转化字段全空）。
    let res = coll
        .insert_one(make_row("ws1", "wxA", "media", "asset1"), None)
        .await
        .expect("insert ledger row");
    let id = res.inserted_id.as_object_id().expect("ledger oid");

    // 2. 插入即能读回，且转化字段为空（回扫前状态）。
    let before = coll
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find_one before")
        .expect("row exists before backfill");
    assert_eq!(before.workspace_id, "ws1");
    assert_eq!(before.send_kind, "media");
    assert_eq!(before.target_id, "asset1");
    assert_eq!(before.customer_stage_at_send.as_deref(), Some("意向"));
    assert_eq!(before.responded, None);
    assert_eq!(before.stage_advanced, None);
    assert!(before.outcome_evaluated_at.is_none());

    // 3. 回扫回填转化字段。
    coll.update_one(
        doc! { "_id": id },
        doc! { "$set": {
            "responded": true,
            "stage_advanced": false,
            "outcome_evaluated_at": DateTime::now(),
        } },
        None,
    )
    .await
    .expect("update outcome fields");

    // 4. 回填后能读回新值（Option<bool>/Option<DateTime> 正确 round-trip）。
    let back = coll
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find_one after")
        .expect("row exists after backfill");
    assert_eq!(back.responded, Some(true));
    assert_eq!(back.stage_advanced, Some(false));
    assert!(back.outcome_evaluated_at.is_some());

    app.cleanup().await;
}

// ── Test 2: 按 workspace_id 查询是 workspace-scoped（IDOR 数据层前提）────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn ledger_query_is_workspace_scoped() {
    let app = common::TestApp::start().await;
    let coll = app.state.db.agent_send_ledger();

    // 两个不同 workspace 各插一条。
    coll.insert_one(make_row("wsA", "wxA", "media", "a1"), None)
        .await
        .expect("insert wsA row");
    coll.insert_one(make_row("wsB", "wxB", "namecard", "c1"), None)
        .await
        .expect("insert wsB row");

    // 只查 wsA：不能看到 wsB 的条目（IDOR 防护的数据层前提）。
    let mut cursor = coll
        .find(doc! { "workspace_id": "wsA" }, None)
        .await
        .expect("find wsA scope");
    let mut count = 0;
    while let Some(row) = cursor.try_next().await.expect("cursor next") {
        assert_eq!(row.workspace_id, "wsA");
        count += 1;
    }
    assert_eq!(count, 1, "wsA scope 必须只返回自己的 1 条台账");

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn sr050_outbox_anchor_is_idempotent_and_globally_unique() {
    let app = common::TestApp::start().await;
    let outbox_id = ObjectId::new();
    let mut row = make_row("ws1", "shared-wxid", "media", "asset-a");
    row.outbox_id = Some(outbox_id);

    record_send_ledger(&app.state, &row).await;
    record_send_ledger(&app.state, &row).await;
    assert_eq!(
        app.state
            .db
            .agent_send_ledger()
            .count_documents(doc! { "outbox_id": outbox_id }, None)
            .await
            .expect("count idempotent ledger rows"),
        1,
        "replaying any confirmed-delivery path must keep exactly one ledger row"
    );

    let mut conflicting = make_row("ws1", "shared-wxid", "namecard", "card-b");
    conflicting.account_id = "acct2".to_string();
    conflicting.outbox_id = Some(outbox_id);
    let duplicate = app
        .state
        .db
        .agent_send_ledger()
        .insert_one(conflicting, None)
        .await
        .expect_err("one outbox delivery cannot be attributed to a second account");
    assert!(
        is_duplicate_key(&duplicate),
        "expected duplicate key: {duplicate:?}"
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn sr050_recent_history_is_account_scoped_for_shared_wxid() {
    let app = common::TestApp::start().await;
    let mut account_a = make_row("ws1", "shared-wxid", "media", "asset-a");
    account_a.account_id = "acct-a".to_string();
    let mut account_b = make_row("ws1", "shared-wxid", "media", "asset-b");
    account_b.account_id = "acct-b".to_string();
    app.state
        .db
        .agent_send_ledger()
        .insert_many([account_a, account_b], None)
        .await
        .expect("insert account-scoped ledger rows");

    let rows =
        recent_sends_for_contact(&app.state, "ws1", "acct-a", "shared-wxid", "media", 10).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].account_id, "acct-a");
    assert_eq!(rows[0].target_id, "asset-a");

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn sr050_outcome_scan_does_not_attribute_another_accounts_reply_or_stage() {
    let app = common::TestApp::start().await;
    let now = DateTime::now();
    let sent_at = DateTime::from_millis(now.timestamp_millis() - 2 * 3_600_000);
    let mut row = make_row("ws1", "shared-wxid", "media", "asset-a");
    row.account_id = "acct-a".to_string();
    row.sent_at = sent_at;
    row.response_window_hours = Some(1);
    row.customer_stage_at_send = Some("stage-1".to_string());
    let ledger_id = app
        .state
        .db
        .agent_send_ledger()
        .insert_one(row, None)
        .await
        .expect("insert account-a ledger")
        .inserted_id
        .as_object_id()
        .expect("ledger object id");

    let other_contact = Contact {
        id: Some(ObjectId::new()),
        workspace_id: "ws1".to_string(),
        account_id: "acct-b".to_string(),
        wxid: "shared-wxid".to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        tags_version: 0,
        domain_attributes: Some(doc! { "customer_stage": "stage-2" }),
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Default::default(),
        profile_attributes: Default::default(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: now,
        updated_at: now,
    };
    app.state
        .db
        .contacts()
        .insert_one(other_contact, None)
        .await
        .expect("insert account-b contact");
    app.state
        .db
        .messages()
        .insert_one(
            ConversationMessage {
                id: Some(ObjectId::new()),
                workspace_id: "ws1".to_string(),
                account_id: "acct-b".to_string(),
                contact_wxid: "shared-wxid".to_string(),
                message_id: Some("account-b-reply".to_string()),
                dedupe_key: None,
                direction: MessageDirection::Inbound,
                content: "reply for account b only".to_string(),
                msg_type: None,
                media_ref: None,
                raw: None,
                is_synthetic_relay: false,
                created_at: DateTime::from_millis(sent_at.timestamp_millis() + 1_000),
            },
            None,
        )
        .await
        .expect("insert account-b inbound");
    app.state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("operation_domain_configs")
        .insert_one(
            doc! {
                "workspace_id": "ws1",
                "domain": "user_operations",
                "name": "test",
                "goal": "test",
                "methodology": "test",
                "workflow": "test",
                "tool_policy": "test",
                "automation_policy": "test",
                "review_policy": "test",
                "runtime_parameters": {},
                "state_machine": {
                    "states": [
                        { "key": "stage-1" },
                        { "key": "stage-2" },
                    ],
                },
                "status": "active",
                "updated_at": now,
                "version": 1,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("insert stage order");

    assert_eq!(scan_send_ledger_outcomes(&app.state).await.unwrap(), 1);
    let evaluated = app
        .state
        .db
        .agent_send_ledger()
        .find_one(doc! { "_id": ledger_id }, None)
        .await
        .expect("query evaluated ledger")
        .expect("evaluated ledger exists");
    assert_eq!(evaluated.responded, Some(false));
    assert_eq!(evaluated.stage_advanced, Some(false));

    app.cleanup().await;
}

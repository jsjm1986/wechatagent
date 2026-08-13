//! Quiet-hours scheduling integration contract.
//!
//! Quiet hours and normal debounce must share one durable `inbound_reply`
//! obligation. The legacy `deferred_inbound_reply` task must not be produced.

mod common;

use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime, Document};
use wechatagent::models::{
    AgentStatus, Contact, ConversationMessage, MessageDirection, OperationDomainConfig,
};
use wechatagent::webhooks::{ensure_wake_followup_task, reconcile_pending_inbound_handoffs};

const REPLY_KIND: &str = "inbound_reply";

fn managed_contact(wxid: &str) -> Contact {
    managed_contact_in("default", wxid)
}

fn managed_contact_in(workspace_id: &str, wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
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
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
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

#[tokio::test]
#[ignore]
async fn quiet_hours_reuses_single_reply_obligation() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("user_quiet_1");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert seed contact");
    app.state
        .db
        .messages()
        .insert_one(
            ConversationMessage {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                message_id: Some("quiet-inbound-1".to_string()),
                dedupe_key: None,
                direction: MessageDirection::Inbound,
                content: "quiet-hours question".to_string(),
                msg_type: Some("text".to_string()),
                media_ref: None,
                raw: None,
                is_synthetic_relay: false,
                created_at: DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert inbound");

    ensure_wake_followup_task(&app.state, &contact, 8, 8)
        .await
        .expect("first wake schedule");

    let task_filter = doc! { "kind": REPLY_KIND, "contact_wxid": &contact.wxid };
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(task_filter.clone(), None)
            .await
            .expect("count reply obligations"),
        1
    );
    let task = app
        .state
        .db
        .tasks()
        .find_one(task_filter.clone(), None)
        .await
        .expect("query reply obligation")
        .expect("reply obligation should exist");
    assert_eq!(task.status, "pending");
    assert!(task.review_required);
    assert!(task.run_at.timestamp_millis() > DateTime::now().timestamp_millis());
    assert!(task.expires_at.is_none(), "passive replies must not expire");
    assert_eq!(task.gateway_status.as_deref(), Some("quiet_hours_waiting"));
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(
                doc! { "kind": "deferred_inbound_reply", "contact_wxid": &contact.wxid },
                None,
            )
            .await
            .expect("count legacy wake tasks"),
        0
    );

    ensure_wake_followup_task(&app.state, &contact, 8, 8)
        .await
        .expect("second wake schedule");
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(task_filter, None)
            .await
            .expect("count reply obligations after retry"),
        1
    );
}

// ── S5-3：静默时段显式交易意图豁免 ─────────────────────────────────────────

/// 构造一份"此刻必然处于静默时段"的 runtime_parameters：tz 偏移取 0，
/// 静默窗口 = [当前 UTC 小时, +2h)。BSON 类型与
/// `models.rs::typed_round_trip_carries_quiet_hours` 的种子一致（i64 / bool），
/// 保证 `runtime_parameters_typed()` 整体解码成功（解码失败会静默回落默认 22-8）。
fn quiet_now_runtime_parameters() -> Document {
    let now_ms = DateTime::now().timestamp_millis();
    let now_hour = (now_ms / 3_600_000) % 24;
    doc! {
        "quietHoursEnabled": true,
        "quietHoursStart": now_hour,
        "quietHoursEnd": (now_hour + 2) % 24,
        "quietHoursTzOffsetHours": 0_i64,
    }
}

/// 该 workspace 唯一 current 的 user_operations 运营域配置，静默窗口覆盖当前时刻。
fn quiet_domain_config(workspace_id: &str) -> OperationDomainConfig {
    OperationDomainConfig {
        id: None,
        workspace_id: workspace_id.to_string(),
        domain: "user_operations".to_string(),
        name: "quiet-hours bypass 测试域".to_string(),
        goal: "-".to_string(),
        methodology: "-".to_string(),
        workflow: "-".to_string(),
        tool_policy: "-".to_string(),
        automation_policy: "-".to_string(),
        review_policy: "-".to_string(),
        runtime_parameters: quiet_now_runtime_parameters(),
        state_machine: Document::new(),
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("test".to_string()),
        principal_decider: None,
        high_risk_escalation_mode: None,
        ask_human_policy: None,
        assist_mode_enabled: None,
    }
}

/// 以 webhook 崩溃恢复同款形态插入一条待接力（handoff_status=pending）的入站消息，
/// 返回其 `_id`。`reconcile_pending_inbound_handoffs` 会像真实 webhook 路径一样
/// 加载 runtime/active profile 并走静默判定。
async fn insert_pending_inbound(
    state: &wechatagent::routes::AppState,
    contact: &Contact,
    content: &str,
) -> ObjectId {
    let id = ObjectId::new();
    let message = ConversationMessage {
        id: Some(id),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some(format!("quiet-bypass-{id}")),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: content.to_string(),
        msg_type: Some("text".to_string()),
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    };
    let mut raw = to_document(&message).expect("serialize inbound");
    raw.insert("handoff_status", "pending");
    state
        .db
        .messages()
        .clone_with_type::<Document>()
        .insert_one(raw, None)
        .await
        .expect("insert pending inbound handoff");
    id
}

/// 给 workspace 种一个非交易域（transaction_facts_enabled=false）的 active profile。
/// 只写必需字段，其余走 serde 默认（与 `DomainProfile` 模型对齐）。
async fn seed_non_transaction_active_profile(
    state: &wechatagent::routes::AppState,
    workspace_id: &str,
) {
    let now = DateTime::now();
    state
        .db
        .domain_profiles()
        .clone_with_type::<Document>()
        .insert_one(
            doc! {
                "profile_id": "quiet_bypass_nontx_v1",
                "workspace_id": workspace_id,
                "display_name": "非交易测试域",
                "transaction_facts_enabled": false,
                "is_active": true,
                "version": 1,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("seed non-transaction active profile");
}

async fn find_reply_obligation(
    state: &wechatagent::routes::AppState,
    workspace_id: &str,
    wxid: &str,
) -> wechatagent::models::AgentTask {
    state
        .db
        .tasks()
        .find_one(
            doc! { "workspace_id": workspace_id, "kind": REPLY_KIND, "contact_wxid": wxid },
            None,
        )
        .await
        .expect("query reply obligation")
        .expect("reply obligation should exist")
}

async fn count_bypass_events(
    state: &wechatagent::routes::AppState,
    workspace_id: &str,
    message_id: ObjectId,
) -> u64 {
    state
        .db
        .events()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "kind": "quiet_hours_bypassed_buying_intent",
                "details.message_id": message_id,
            },
            None,
        )
        .await
        .expect("count bypass events")
}

/// S5-3：静默时段的显式购买/交易承诺不再等醒来。交易域 profile（默认销售域）下，
/// 确定性词表命中 → 走正常去抖链路（gateway_status=debouncing、run_at≈now）并写
/// `quiet_hours_bypassed_buying_intent` 事件；对照组一（同交易域、寒暄消息）与
/// 对照组二（非交易域、同购买短语）仍旧 defer 到醒来时刻、不写事件。
#[tokio::test]
#[ignore]
async fn explicit_buying_intent_bypasses_quiet_hours_deferral_on_transaction_profile() {
    const W_BUY: &str = "ws_quiet_bypass_buy";
    const W_CASUAL: &str = "ws_quiet_bypass_casual";
    const W_NONTX: &str = "ws_quiet_bypass_nontx";

    let app = common::TestApp::start().await;
    let now_ms = DateTime::now().timestamp_millis();

    for workspace_id in [W_BUY, W_CASUAL, W_NONTX] {
        app.state
            .db
            .operation_domain_configs()
            .insert_one(&quiet_domain_config(workspace_id), None)
            .await
            .expect("seed quiet domain config");
    }
    // W_BUY / W_CASUAL 不种 profile → 回落 DEFAULT 销售域（transaction_facts_enabled=true）。
    seed_non_transaction_active_profile(&app.state, W_NONTX).await;

    let buy_contact = managed_contact_in(W_BUY, "user_quiet_bypass_buy");
    let casual_contact = managed_contact_in(W_CASUAL, "user_quiet_bypass_casual");
    let nontx_contact = managed_contact_in(W_NONTX, "user_quiet_bypass_nontx");
    for contact in [&buy_contact, &casual_contact, &nontx_contact] {
        app.state
            .db
            .contacts()
            .insert_one(contact, None)
            .await
            .expect("insert managed contact");
    }

    let buy_msg = insert_pending_inbound(&app.state, &buy_contact, "我要买，现在付款").await;
    let casual_msg =
        insert_pending_inbound(&app.state, &casual_contact, "今天好累呀，晚点再聊").await;
    let nontx_msg = insert_pending_inbound(&app.state, &nontx_contact, "我要买，现在付款").await;

    assert_eq!(
        reconcile_pending_inbound_handoffs(&app.state)
            .await
            .expect("reconcile pending handoffs"),
        3
    );

    // 交易域 + 显式购买承诺：豁免静默 defer，走正常去抖链路（run_at ≈ 入站 + 去抖窗口）。
    let buy_task = find_reply_obligation(&app.state, W_BUY, &buy_contact.wxid).await;
    assert_eq!(
        buy_task.gateway_status.as_deref(),
        Some("debouncing"),
        "显式交易意图应走正常去抖而非 quiet_hours_waiting: {buy_task:?}"
    );
    assert!(
        buy_task.run_at.timestamp_millis() <= now_ms + 60_000,
        "豁免后的 run_at 应近在眼前（去抖窗口内），实际 {}",
        buy_task.run_at
    );
    assert_eq!(
        count_bypass_events(&app.state, W_BUY, buy_msg).await,
        1,
        "豁免应写一条 quiet_hours_bypassed_buying_intent 事件（含 message_id）"
    );

    // 对照一：同交易域的寒暄消息仍 defer 到醒来（wake ≥ 1h 之后）。
    let casual_task = find_reply_obligation(&app.state, W_CASUAL, &casual_contact.wxid).await;
    assert_eq!(
        casual_task.gateway_status.as_deref(),
        Some("quiet_hours_waiting"),
        "寒暄消息在静默时段应继续 defer: {casual_task:?}"
    );
    assert!(
        casual_task.run_at.timestamp_millis() >= now_ms + 30 * 60_000,
        "defer 的 run_at 应在醒来时刻（≥30min 之后），实际 {}",
        casual_task.run_at
    );
    assert_eq!(
        count_bypass_events(&app.state, W_CASUAL, casual_msg).await,
        0
    );

    // 对照二：非交易域即便命中购买短语也不豁免（词表门以 transaction_facts_enabled 为前置）。
    let nontx_task = find_reply_obligation(&app.state, W_NONTX, &nontx_contact.wxid).await;
    assert_eq!(
        nontx_task.gateway_status.as_deref(),
        Some("quiet_hours_waiting"),
        "非交易域的购买短语不应豁免静默 defer: {nontx_task:?}"
    );
    assert!(nontx_task.run_at.timestamp_millis() >= now_ms + 30 * 60_000);
    assert_eq!(count_bypass_events(&app.state, W_NONTX, nontx_msg).await, 0);

    app.cleanup().await;
}

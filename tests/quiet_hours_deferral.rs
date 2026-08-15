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

// ── 语义判断与作息排程解耦 ────────────────────────────────────────────────

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
        name: "quiet-hours semantic scheduling test domain".to_string(),
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
        message_id: Some(format!("quiet-semantic-{id}")),
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
                "profile_id": "quiet_semantic_nontx_v1",
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

/// 作息排程不读取消息词面：无论是购买表达、寒暄、假设还是非交易域消息，
/// 静默时段都统一排到醒来；醒来后再交给 AI 语义链路判断如何回应。
#[tokio::test]
#[ignore]
async fn all_inbound_messages_defer_during_quiet_hours() {
    const W_BUY: &str = "ws_quiet_semantic_buy";
    const W_CASUAL: &str = "ws_quiet_semantic_casual";
    const W_NONTX: &str = "ws_quiet_semantic_nontx";

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

    let buy_contact = managed_contact_in(W_BUY, "user_quiet_semantic_buy");
    let casual_contact = managed_contact_in(W_CASUAL, "user_quiet_semantic_casual");
    let nontx_contact = managed_contact_in(W_NONTX, "user_quiet_semantic_nontx");
    for contact in [&buy_contact, &casual_contact, &nontx_contact] {
        app.state
            .db
            .contacts()
            .insert_one(contact, None)
            .await
            .expect("insert managed contact");
    }

    insert_pending_inbound(&app.state, &buy_contact, "我要买，现在付款").await;
    insert_pending_inbound(&app.state, &casual_contact, "今天好累呀，晚点再聊").await;
    insert_pending_inbound(&app.state, &nontx_contact, "如果我要买，现在付款有优惠吗").await;

    assert_eq!(
        reconcile_pending_inbound_handoffs(&app.state)
            .await
            .expect("reconcile pending handoffs"),
        3
    );

    for (workspace_id, wxid) in [
        (W_BUY, &buy_contact.wxid),
        (W_CASUAL, &casual_contact.wxid),
        (W_NONTX, &nontx_contact.wxid),
    ] {
        let task = find_reply_obligation(&app.state, workspace_id, wxid).await;
        assert_eq!(
            task.gateway_status.as_deref(),
            Some("quiet_hours_waiting"),
            "消息词面不应改变静默排程: {task:?}"
        );
        assert!(
            task.run_at.timestamp_millis() >= now_ms + 30 * 60_000,
            "静默任务应排到醒来时刻，实际 {}",
            task.run_at
        );
    }
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(doc! { "kind": "quiet_hours_bypassed_buying_intent" }, None)
            .await
            .expect("count removed keyword bypass events"),
        0,
        "不应再产生基于购买关键词的作息豁免事件"
    );

    app.cleanup().await;
}

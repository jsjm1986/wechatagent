//! S5-5 请示预授权底线（standing order）：链尾无人应答超过运营预设时限后，
//! 超时扫描把运营预写的底线口径当作一条 conditional 预授权裁决执行——resolve
//! 台账（resolved_via=standing_order_policy）并复用既有 resolve→relay 链路物化
//! relay task（零新发送路径），同时写 `escalation_standing_order_applied` 审计事件。
//! 未配置 / 时限未到时维持既有链尾安抚行为；resolved 终态天然幂等（重复扫描不重复应用）。
//!
//! 默认 #[ignore]，需 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::models::{
    AgentPrincipalEscalation, AgentStatus, AskHumanPolicy, Contact, DeciderRef,
    PrincipalEscalationProtocol, ESCALATION_CATEGORY_OUT_OF_SCOPE, PRINCIPAL_CARD_DELIVERY_SENT,
    PRINCIPAL_ESCALATION_STATUS_PENDING, PRINCIPAL_ESCALATION_STATUS_RESOLVED,
    PRINCIPAL_RELAY_STATE_ENQUEUED, PRINCIPAL_VERDICT_CONDITIONAL,
};

const CUSTOMER: &str = "cust_standing_order";
const STANDING_ORDER_TEXT: &str = "最多可给 95 折，赠品可送，超出请客户稍等正式确认。";

/// 最小可用 Contact（镜像 escalation_stranded_delivery_timeout.rs 夹具）。
fn minimal_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
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
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

async fn seed_customer_contact(app: &common::TestApp) {
    app.state
        .db
        .contacts()
        .insert_one(minimal_contact(CUSTOMER), None)
        .await
        .expect("insert standing-order customer contact");
}

/// 单人链（boss 即链尾）、timeout 1h 的冻结 policy；standing order 两字段由调用方指定。
fn chain_tail_policy(
    standing_order: Option<&str>,
    standing_order_after_hours: Option<f64>,
) -> AskHumanPolicy {
    AskHumanPolicy {
        decider_chain: vec![DeciderRef {
            wxid: "boss_wxid".to_string(),
            display_name: None,
            account_id: Some("default".to_string()),
        }],
        escalate_safety_guard: true,
        escalate_unverified_product: true,
        escalate_ai_policy_hold: false,
        escalate_stuck: true,
        dedupe_window_hours: None,
        daily_push_cap: None,
        quiet_hours: None,
        timeout_hours: Some(1.0),
        standing_order: standing_order.map(str::to_string),
        standing_order_after_hours,
    }
}

/// 一条已送达（sent + 有推送时刻）的 pending 台账，created_at/last_pushed 均为 `age_ms_ago` 前。
fn aged_pending_escalation(
    short_code: &str,
    policy: AskHumanPolicy,
    age_ms_ago: i64,
) -> AgentPrincipalEscalation {
    let aged = DateTime::from_millis(DateTime::now().timestamp_millis() - age_ms_ago);
    AgentPrincipalEscalation {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: CUSTOMER.to_string(),
        short_code: short_code.to_string(),
        status: PRINCIPAL_ESCALATION_STATUS_PENDING.to_string(),
        category: ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string(),
        reason: "超出标准 9 折权限".to_string(),
        question_for_principal: "是否同意 8 折？".to_string(),
        principal_wxid: "boss_wxid".to_string(),
        protocol: Some(PrincipalEscalationProtocol {
            domain: "user_operations".to_string(),
            policy_version: 1,
            policy,
            principal_account_id: "default".to_string(),
            delivery_generation: 1,
            delivery_state: PRINCIPAL_CARD_DELIVERY_SENT.to_string(),
            delivery_content: "test principal card".to_string(),
            delivery_outbox_id: None,
            failure_cleanup_completed_at: None,
        }),
        decision: None,
        authorization_expires_at: None,
        is_generalizable: false,
        knowledge_proposal_emitted: false,
        last_holding_reply_ms: None,
        last_pushed_at_ms: Some(aged.timestamp_millis()),
        created_at: aged,
        updated_at: aged,
        resolved_at: None,
        resolved_via: None,
        relay_state: None,
        relay_task_id: None,
        relay_enqueued_at: None,
        relay_terminal_at: None,
        relay_terminal_reason: None,
    }
}

async fn insert_escalation(app: &common::TestApp, entry: AgentPrincipalEscalation) -> ObjectId {
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(&entry, None)
        .await
        .expect("insert escalation")
        .inserted_id
        .as_object_id()
        .expect("inserted escalation id")
}

async fn find_escalation(app: &common::TestApp, short_code: &str) -> AgentPrincipalEscalation {
    app.state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": short_code }, None)
        .await
        .expect("query escalation")
        .expect("escalation must exist")
}

async fn customer_outbox_count(app: &common::TestApp) -> u64 {
    app.state
        .db
        .collection_agent_send_outbox()
        .clone_with_type::<Document>()
        .count_documents(
            doc! { "workspace_id": "default", "contact_wxid": CUSTOMER },
            None,
        )
        .await
        .expect("count customer outbox rows")
}

async fn relay_task_count(app: &common::TestApp, short_code: &str) -> u64 {
    app.state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .count_documents(
            doc! {
                "workspace_id": "default",
                "contact_wxid": CUSTOMER,
                "kind": "principal_decision_relay",
                "content": short_code,
            },
            None,
        )
        .await
        .expect("count relay tasks")
}

async fn standing_order_event_count(app: &common::TestApp) -> u64 {
    app.state
        .db
        .events()
        .clone_with_type::<Document>()
        .count_documents(
            doc! {
                "workspace_id": "default",
                "kind": "escalation_standing_order_applied",
            },
            None,
        )
        .await
        .expect("count standing-order events")
}

/// 链尾 + 已配置底线 + 双超时（当前决策人超时且台账年龄超过底线时限）→ 台账 resolved
/// （resolved_via=standing_order_policy），裁决与领导 conditional 裁决同形（substance=底线
/// 文本），并经既有 resolve→relay 链路物化 relay task + 写审计事件；不发链尾安抚。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn standing_order_applies_at_chain_tail_past_deadline() {
    let app = common::TestApp::start().await;
    seed_customer_contact(&app).await;

    let two_hours = 2 * 3600 * 1000;
    let escalation_id = insert_escalation(
        &app,
        aged_pending_escalation(
            "SO01",
            chain_tail_policy(Some(STANDING_ORDER_TEXT), Some(1.0)),
            two_hours,
        ),
    )
    .await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("scan timeouts");

    let resolved = find_escalation(&app, "SO01").await;
    assert_eq!(
        resolved.status, PRINCIPAL_ESCALATION_STATUS_RESOLVED,
        "链尾超过底线时限后台账必须 resolved"
    );
    assert_eq!(
        resolved.resolved_via.as_deref(),
        Some("standing_order_policy"),
        "裁决来源审计必须是 standing_order_policy"
    );
    let decision = resolved.decision.as_ref().expect("standing-order decision");
    assert_eq!(
        decision.verdict, PRINCIPAL_VERDICT_CONDITIONAL,
        "预授权底线与领导 conditional 裁决同形"
    );
    assert_eq!(decision.substance, STANDING_ORDER_TEXT);
    assert!(decision.constraints.is_empty());
    assert!(
        resolved.authorization_expires_at.is_none(),
        "运营常备底线不设授权过期窗"
    );
    assert_eq!(
        resolved.relay_state.as_deref(),
        Some(PRINCIPAL_RELAY_STATE_ENQUEUED),
        "resolve 内核必须已物化 relay intent（复用既有链路）"
    );
    assert_eq!(resolved.relay_task_id, Some(escalation_id));
    assert_eq!(
        relay_task_count(&app, "SO01").await,
        1,
        "relay task 必须经 materialize_relay_task 物化且只有一条"
    );
    assert_eq!(
        standing_order_event_count(&app).await,
        1,
        "必须写 escalation_standing_order_applied 审计事件"
    );
    assert_eq!(
        customer_outbox_count(&app).await,
        0,
        "standing order 分支不得夹带即时发送——客户侧转述由 relay task 走网关"
    );
}

/// 只过一半时限（当前决策人已超时、但台账年龄未达 standing_order_after_hours）→
/// 不应用底线，维持既有链尾安抚行为（客户收安抚、台账保持 pending）。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn standing_order_not_applied_before_after_hours_deadline() {
    let app = common::TestApp::start().await;
    seed_customer_contact(&app).await;

    let two_hours = 2 * 3600 * 1000;
    insert_escalation(
        &app,
        aged_pending_escalation(
            "SO02",
            chain_tail_policy(Some(STANDING_ORDER_TEXT), Some(100.0)),
            two_hours,
        ),
    )
    .await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("scan timeouts");

    let untouched = find_escalation(&app, "SO02").await;
    assert_eq!(
        untouched.status, PRINCIPAL_ESCALATION_STATUS_PENDING,
        "底线时限未到不得 resolve"
    );
    assert!(untouched.decision.is_none());
    assert!(
        untouched.last_holding_reply_ms.is_some(),
        "底线时限未到时链尾安抚行为保持不变"
    );
    assert_eq!(customer_outbox_count(&app).await, 1, "安抚话术照常入队");
    assert_eq!(relay_task_count(&app, "SO02").await, 0);
    assert_eq!(standing_order_event_count(&app).await, 0);
}

/// 未配置底线 → 行为与改动前字节等价：链尾安抚 + 保持 pending。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn no_standing_order_config_keeps_chain_tail_holding() {
    let app = common::TestApp::start().await;
    seed_customer_contact(&app).await;

    let two_hours = 2 * 3600 * 1000;
    insert_escalation(
        &app,
        aged_pending_escalation("SO03", chain_tail_policy(None, None), two_hours),
    )
    .await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("scan timeouts");

    let untouched = find_escalation(&app, "SO03").await;
    assert_eq!(untouched.status, PRINCIPAL_ESCALATION_STATUS_PENDING);
    assert!(untouched.decision.is_none());
    assert!(
        untouched.last_holding_reply_ms.is_some(),
        "未配置底线时既有链尾安抚必须保持"
    );
    assert_eq!(customer_outbox_count(&app).await, 1);
    assert_eq!(relay_task_count(&app, "SO03").await, 0);
    assert_eq!(standing_order_event_count(&app).await, 0);
}

/// 幂等：重复扫描不重复应用（resolved 终态排除在扫描 filter 外），relay task 与
/// 审计事件都只有一条。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn standing_order_applied_once_across_repeated_scans() {
    let app = common::TestApp::start().await;
    seed_customer_contact(&app).await;

    let two_hours = 2 * 3600 * 1000;
    insert_escalation(
        &app,
        aged_pending_escalation(
            "SO04",
            chain_tail_policy(Some(STANDING_ORDER_TEXT), Some(1.0)),
            two_hours,
        ),
    )
    .await;

    for pass in 0..2 {
        wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
            .await
            .unwrap_or_else(|e| panic!("scan pass {pass} failed: {e}"));
    }

    let resolved = find_escalation(&app, "SO04").await;
    assert_eq!(resolved.status, PRINCIPAL_ESCALATION_STATUS_RESOLVED);
    assert_eq!(
        relay_task_count(&app, "SO04").await,
        1,
        "重复扫描不得物化第二条 relay task"
    );
    assert_eq!(
        standing_order_event_count(&app).await,
        1,
        "重复扫描不得重复写审计事件"
    );
}

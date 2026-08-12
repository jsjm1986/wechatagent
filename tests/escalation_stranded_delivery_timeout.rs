//! 23 号终裁回归：投递结果不可核验（delivery_unknown）或形态异常（sent 但缺
//! last_pushed_at_ms）的 pending 请示卡，没有可信推送时刻、永远进不了常规超时
//! 扫描（其 filter 要求 delivery_state=sent 且 last_pushed_at_ms 为数字），此前
//! 会静默滞留。修复后独立收敛分支以 created_at 为时间基准套用同一套超时语义：
//! 超时即改派下一位决策人并重推卡（刷新投递代次）；未超时的滞留卡保持原状。
//!
//! 默认 #[ignore]，需 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::models::{
    AgentPrincipalEscalation, AgentStatus, AskHumanPolicy, Contact, DeciderRef,
    PrincipalEscalationProtocol, ESCALATION_CATEGORY_OUT_OF_SCOPE,
    PRINCIPAL_CARD_DELIVERY_QUEUED, PRINCIPAL_CARD_DELIVERY_SENT,
    PRINCIPAL_CARD_DELIVERY_UNKNOWN, PRINCIPAL_ESCALATION_STATUS_PENDING,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// 最小可用 Contact（镜像 principal_decision_channel.rs 夹具）。
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

/// 种入滞留场景的客户 contact（awaiting 标记激活需要它在场）。
async fn seed_customer_contact(app: &common::TestApp) {
    app.state
        .db
        .contacts()
        .insert_one(minimal_contact("cust_stranded"), None)
        .await
        .expect("insert stranded customer contact");
}

async fn start_mcp_mock_success() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "structuredContent": { "newMsgId": "mock_card_msg", "content": [] } }
        })))
        .mount(&server)
        .await;
    server
}

/// boss → backup 双人链、timeout 1h 的冻结 policy。
fn two_step_policy() -> AskHumanPolicy {
    AskHumanPolicy {
        decider_chain: vec![
            DeciderRef {
                wxid: "boss_wxid".to_string(),
                display_name: None,
                account_id: Some("default".to_string()),
            },
            DeciderRef {
                wxid: "backup_wxid".to_string(),
                display_name: None,
                account_id: Some("default".to_string()),
            },
        ],
        escalate_safety_guard: true,
        escalate_unverified_product: true,
        escalate_ai_policy_hold: false,
        escalate_stuck: true,
        dedupe_window_hours: None,
        daily_push_cap: None,
        quiet_hours: None,
        timeout_hours: Some(1.0),
    }
}

/// 一条滞留形态的 pending 台账：delivery_state 与 last_pushed_at_ms 由调用方指定，
/// created_at 决定滞留时长（收敛分支的时间基准）。
fn stranded_escalation(
    short_code: &str,
    delivery_state: &str,
    last_pushed_at_ms: Option<i64>,
    created_at: DateTime,
) -> AgentPrincipalEscalation {
    AgentPrincipalEscalation {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: "cust_stranded".to_string(),
        short_code: short_code.to_string(),
        status: PRINCIPAL_ESCALATION_STATUS_PENDING.to_string(),
        category: ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string(),
        reason: "超出标准 9 折权限".to_string(),
        question_for_principal: "是否同意 8 折？".to_string(),
        principal_wxid: "boss_wxid".to_string(),
        protocol: Some(PrincipalEscalationProtocol {
            domain: "user_operations".to_string(),
            policy_version: 1,
            policy: two_step_policy(),
            principal_account_id: "default".to_string(),
            delivery_generation: 1,
            delivery_state: delivery_state.to_string(),
            delivery_content: "test principal card".to_string(),
            delivery_outbox_id: None,
            failure_cleanup_completed_at: None,
        }),
        decision: None,
        authorization_expires_at: None,
        is_generalizable: false,
        knowledge_proposal_emitted: false,
        last_holding_reply_ms: None,
        last_pushed_at_ms,
        created_at,
        updated_at: created_at,
        resolved_at: None,
        resolved_via: None,
        relay_state: None,
        relay_task_id: None,
        relay_enqueued_at: None,
        relay_terminal_at: None,
        relay_terminal_reason: None,
    }
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

async fn outbox_count_for(app: &common::TestApp, principal_wxid: &str) -> u64 {
    app.state
        .db
        .collection_agent_send_outbox()
        .clone_with_type::<Document>()
        .count_documents(
            doc! { "workspace_id": "default", "contact_wxid": principal_wxid },
            None,
        )
        .await
        .expect("count principal outbox rows")
}

/// delivery_unknown 滞留卡超过冻结 timeout（以 created_at 计）后，收敛分支改派
/// 下一位决策人：开启下一投递代次并向其重推卡（QUEUED + outbox 入队）。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn stranded_delivery_unknown_card_is_reassigned_on_timeout() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    seed_customer_contact(&app).await;

    let two_hours_ago =
        DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(
            stranded_escalation("SDU1", PRINCIPAL_CARD_DELIVERY_UNKNOWN, None, two_hours_ago),
            None,
        )
        .await
        .expect("insert stranded escalation");

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan timeouts");

    let converged = find_escalation(&app, "SDU1").await;
    assert_eq!(
        converged.principal_wxid, "backup_wxid",
        "滞留卡必须按超时语义改派链上下一位决策人"
    );
    let protocol = converged.protocol.as_ref().expect("frozen protocol");
    assert_eq!(protocol.delivery_generation, 2, "改派须刷新投递代次");
    assert_eq!(
        protocol.delivery_state, PRINCIPAL_CARD_DELIVERY_QUEUED,
        "改派后立即物化下一代次投递（pending_enqueue → queued）"
    );
    assert!(
        converged.last_pushed_at_ms.is_none(),
        "推送时刻只能由 Outbox sent 对账回填"
    );
    assert_eq!(
        outbox_count_for(&app, "backup_wxid").await,
        1,
        "重推卡必须经 durable outbox"
    );
}

/// sent 但缺 last_pushed_at_ms 的异常形态同样进收敛分支（常规扫描的 filter
/// 要求该字段为数字，这类行原本永远不会被扫到）。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn stranded_sent_row_without_push_time_is_reassigned_on_timeout() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    seed_customer_contact(&app).await;

    let two_hours_ago =
        DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(
            stranded_escalation("SDS1", PRINCIPAL_CARD_DELIVERY_SENT, None, two_hours_ago),
            None,
        )
        .await
        .expect("insert anomalous sent escalation");

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan timeouts");

    let converged = find_escalation(&app, "SDS1").await;
    assert_eq!(converged.principal_wxid, "backup_wxid");
    assert_eq!(
        converged.protocol.as_ref().expect("protocol").delivery_generation,
        2
    );
}

/// 未超时的滞留卡保持原状：收敛分支沿用冻结 policy 的 timeout 语义，
/// 不做即时改派。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn stranded_card_before_timeout_stays_put() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    seed_customer_contact(&app).await;

    let now = DateTime::now();
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(
            stranded_escalation("SDF1", PRINCIPAL_CARD_DELIVERY_UNKNOWN, None, now),
            None,
        )
        .await
        .expect("insert fresh stranded escalation");

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan timeouts");

    let untouched = find_escalation(&app, "SDF1").await;
    assert_eq!(untouched.principal_wxid, "boss_wxid", "未超时不得改派");
    let protocol = untouched.protocol.as_ref().expect("protocol");
    assert_eq!(protocol.delivery_generation, 1);
    assert_eq!(protocol.delivery_state, PRINCIPAL_CARD_DELIVERY_UNKNOWN);
    assert_eq!(outbox_count_for(&app, "boss_wxid").await, 0);
    assert_eq!(outbox_count_for(&app, "backup_wxid").await, 0);
}

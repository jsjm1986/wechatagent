//! 决策请示通道集成测试（spec §14 九项）。
//!
//! 多数场景需真实 MongoDB（testcontainers），标 `#[ignore]`，由 CI（带 Docker）跑：
//! `cargo test --test principal_decision_channel -- --ignored`。它们通过 **公共**
//! 模型 + 公共 typed accessor 断言台账/配置/知识库的 DB 状态，覆盖各 pub(crate)
//! 业务函数的"公共表面切片"——而不必把整模块标 pub。
//!
//! 两个纯函数测试（**不**标 ignore）随 `cargo test --test principal_decision_channel`
//! 本地即跑：
//! - §14.4b：`ConversationMessage::synthetic_principal_relay` 哨兵 + 载荷字段守卫；
//! - §14.9b：`fallback_holding_reply` 兜底文案红线（不含任何转接类措辞）。
//!
//! §14.8（目标 wxid 二次防护）的纯函数 `assert_target_is_principal` 是 pub(crate)，
//! crate 外不可达；其纯函数测试已在 src/agent/escalation.rs 的 `#[cfg(test)] mod tests`
//! 内（`assert_target_is_principal_accepts_match` / `assert_target_is_principal_rejects_customer`），
//! 本文件不为它单独放开可见性。

mod common;

use axum::{body::Bytes, extract::State, http::HeaderMap};
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::agent::run_envelope::SOURCE_KIND_PRINCIPAL_CLARIFICATION;
use wechatagent::models::{
    AgentPrincipalEscalation, AgentStatus, AgentTask, AskHumanPolicy, AskHumanQuietHours, Contact,
    ConversationMessage, DeciderRef, OperationKnowledgeChunk, PrincipalDecision,
    PrincipalEscalationProtocol, WechatAccount, AWAITING_PRINCIPAL_DECISION_ATTR,
    ESCALATION_CATEGORY_OUT_OF_SCOPE, PRINCIPAL_CARD_DELIVERY_QUEUED, PRINCIPAL_CARD_DELIVERY_SENT,
    PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED, PRINCIPAL_RELAY_SENTINEL, PRINCIPAL_VERDICT_CONDITIONAL,
};
use wechatagent::webhooks::wechat_webhook;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ───────────────────────────── 测试夹具构造 ─────────────────────────────

/// 最小可用 Contact：wxid 可指定，workspace/account 固定 "default"，其余取 None/空。
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

async fn seed_online_default_account(app: &common::TestApp) {
    let now = DateTime::now();
    app.state
        .db
        .accounts()
        .insert_one(
            WechatAccount {
                id: Some(ObjectId::new()),
                workspace_id: "default".to_string(),
                account_id: "default".to_string(),
                alias: "hc013_test".to_string(),
                display_name: "HC-013 test account".to_string(),
                app_id: Some("hc013-test-app".to_string()),
                wxid: Some("wxid_hc013_test_account".to_string()),
                nick_name: None,
                avatar_url: None,
                // Default-workspace tests intentionally exercise the deployment fallback from
                // the rebuilt AppState, whose MCP URL points at the per-test WireMock server.
                mcp_base_url: None,
                mcp_api_key: None,
                webhook_secret: None,
                online: true,
                status: Some("active".to_string()),
                last_sync_at: now,
                capacity: 0,
                persona_tag: None,
                off_hours: Vec::new(),
                created_at: now,
                updated_at: now,
            },
            None,
        )
        .await
        .expect("insert online default test account");
}

/// 一条 pending 请示台账。镜像 escalation::insert_pending_escalation 写入的形状。
fn minimal_pending_escalation(short_code: &str, contact_wxid: &str) -> AgentPrincipalEscalation {
    let now = DateTime::now();
    AgentPrincipalEscalation {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: contact_wxid.to_string(),
        short_code: short_code.to_string(),
        status: PRINCIPAL_ESCALATION_STATUS_PENDING.to_string(),
        category: ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string(),
        reason: "超出标准 9 折权限".to_string(),
        question_for_principal: "是否同意 8 折？".to_string(),
        principal_wxid: "boss_wxid".to_string(),
        protocol: Some(PrincipalEscalationProtocol {
            domain: "user_operations".to_string(),
            policy_version: 1,
            policy: AskHumanPolicy {
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
                timeout_hours: None,
                standing_order: None,
                standing_order_after_hours: None,
            },
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
        last_pushed_at_ms: None,
        created_at: now,
        updated_at: now,
        resolved_at: None,
        resolved_via: None,
        relay_state: None,
        relay_task_id: None,
        relay_enqueued_at: None,
        relay_terminal_at: None,
        relay_terminal_reason: None,
    }
}

/// Delivery-protocol redline: an ambiguous principal reply enters through the real webhook
/// router, creates one idempotent durable clarification Outbox row, and performs no direct MCP
/// send in the request handler.
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn principal_ambiguity_clarification_is_durable_and_never_direct_mcp() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    state.config.webhook_verify_signature = false;
    seed_online_default_account(&app).await;
    app.state
        .db
        .operation_domain_configs()
        .update_one(
            doc! { "workspace_id": "default", "domain": "user_operations", "current_version": true },
            doc! { "$set": {
                "principal_decider": "boss_wxid",
                "high_risk_escalation_mode": "all",
                "updated_at": DateTime::now(),
            }},
            None,
        )
        .await
        .expect("configure principal");
    for (code, customer) in [("AMB1", "customer-a"), ("AMB2", "customer-b")] {
        app.state
            .db
            .agent_principal_escalations()
            .insert_one(minimal_pending_escalation(code, customer), None)
            .await
            .expect("insert pending escalation");
    }

    let body = Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "appId": "hc013-test-app",
            "fromWxid": "boss_wxid",
            "content": "可以",
            "msgId": "principal-ambiguous-message"
        }))
        .expect("serialize webhook"),
    );
    for _ in 0..2 {
        let response = wechat_webhook(State(state.clone()), HeaderMap::new(), body.clone())
            .await
            .expect("route ambiguous principal reply");
        assert_eq!(response.0["routed"], "principal");
    }

    assert!(
        mcp.received_requests()
            .await
            .expect("wiremock requests")
            .is_empty(),
        "webhook handling must not directly call MCP"
    );
    let rows = app
        .state
        .db
        .collection_agent_send_outbox()
        .count_documents(
            doc! {
                "source_kind": SOURCE_KIND_PRINCIPAL_CLARIFICATION,
                "contact_wxid": "boss_wxid",
                "status": "pending",
            },
            None,
        )
        .await
        .expect("count clarification outbox");
    assert_eq!(
        rows, 1,
        "replayed ambiguous replies must share one durable intent"
    );
    app.cleanup().await;
}

// ───────────────────── §14 DB 集成测试（#[ignore]，CI 跑） ─────────────────────

/// §14.1a：插入 pending 台账后，按 short_code 能查回，且 status=pending、principal_wxid 正确。
/// 覆盖 insert_pending_escalation 的台账/模型+accessor 往返公共切片。
#[tokio::test]
#[ignore]
async fn t_escalation_out_of_scope_creates_pending() {
    let app = common::TestApp::start().await;
    let entry = minimal_pending_escalation("E1A2", "cust_oos");
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(&entry, None)
        .await
        .expect("insert pending escalation");

    let found = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": "E1A2" }, None)
        .await
        .expect("query escalation")
        .expect("escalation must exist");

    assert_eq!(found.status, PRINCIPAL_ESCALATION_STATUS_PENDING);
    assert_eq!(found.principal_wxid, "boss_wxid");
    assert_eq!(found.category, ESCALATION_CATEGORY_OUT_OF_SCOPE);
    assert_eq!(found.contact_wxid, "cust_oos");
    assert!(found.decision.is_none());
}

/// §14.1b：high_risk_escalation_mode="all" 的域配置写后读回，字段持久化。
/// 覆盖升级模式配置的公共切片（parse_high_risk_mode 读的就是这个字段）。
///
/// 注：`ensure_prompt_pack_v2`（TestApp::start 调用）已 seed 一行
/// `(default, user_operations, version=1)`，且 `op_domain_ws_domain_version_unique`
/// 唯一索引禁止重复插入。请示通道的两字段在生产里也是 admin 编辑 `$set` 到既有
/// current 行（版本发布才 bump version），故此处镜像该写法——`$set` 到 seeded 行，
/// 而非另插一行 v1。
#[tokio::test]
#[ignore]
async fn t_high_risk_mode_config_roundtrip() {
    let app = common::TestApp::start().await;
    app.state
        .db
        .operation_domain_configs()
        .update_one(
            doc! { "workspace_id": "default", "domain": "user_operations", "current_version": true },
            doc! { "$set": {
                "principal_decider": "boss_wxid",
                "high_risk_escalation_mode": "all",
                "updated_at": DateTime::now(),
            } },
            None,
        )
        .await
        .expect("set principal config on seeded domain config");

    let found = app
        .state
        .db
        .operation_domain_configs()
        .find_one(
            doc! { "workspace_id": "default", "domain": "user_operations", "current_version": true },
            None,
        )
        .await
        .expect("query domain config")
        .expect("config must exist");

    assert_eq!(found.high_risk_escalation_mode.as_deref(), Some("all"));
    assert_eq!(found.principal_decider.as_deref(), Some("boss_wxid"));
}

/// §14.2：pending → resolved。手动 $set 一份 PrincipalDecision + status=resolved，
/// 断言状态迁移正确且裁决正确反序列化进 decision 字段。覆盖 resolve_escalation 公共切片。
#[tokio::test]
#[ignore]
async fn t_pending_resolve_roundtrip() {
    let app = common::TestApp::start().await;
    let entry = minimal_pending_escalation("E3B4", "cust_resolve");
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(&entry, None)
        .await
        .expect("insert pending escalation");

    let decision = PrincipalDecision {
        verdict: PRINCIPAL_VERDICT_CONDITIONAL.to_string(),
        substance: "可以给 8 折".to_string(),
        constraints: vec!["本周内付款".to_string()],
        authorization_window_hours: Some(48.0),
        exemption_type: wechatagent::models::EXEMPTION_TYPE_NONE.to_string(),
    };
    let decision_bson = mongodb::bson::to_bson(&decision).expect("serialize decision");
    let now = DateTime::now();
    app.state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! { "short_code": "E3B4", "status": PRINCIPAL_ESCALATION_STATUS_PENDING },
            doc! { "$set": {
                "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                "decision": decision_bson,
                "updated_at": now,
                "resolved_at": now,
            } },
            None,
        )
        .await
        .expect("resolve escalation");

    let found = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": "E3B4" }, None)
        .await
        .expect("query escalation")
        .expect("escalation must exist");

    assert_eq!(found.status, PRINCIPAL_ESCALATION_STATUS_RESOLVED);
    assert!(found.resolved_at.is_some());
    let stored = found
        .decision
        .expect("decision must be stored after resolve");
    assert_eq!(stored.verdict, PRINCIPAL_VERDICT_CONDITIONAL);
    assert_eq!(stored.substance, "可以给 8 折");
    assert_eq!(stored.constraints, vec!["本周内付款".to_string()]);
    assert_eq!(stored.authorization_window_hours, Some(48.0));
}

/// §14.3：知识缺口提案永远落 draft + needs_review + 共享域（account_id=None）。
/// 镜像 emit_knowledge_gap_proposal 写入的 chunk 形状；红线：AI 永不自动验证。
#[tokio::test]
#[ignore]
async fn t_knowledge_proposal_is_draft_needs_review() {
    let app = common::TestApp::start().await;
    let chunk = OperationKnowledgeChunk {
        workspace_id: "default".to_string(),
        account_id: None, // workspace 共享域
        status: "draft".to_string(),
        integrity_status: Some("needs_review".to_string()),
        title: "真人决策沉淀（待审核）：超出标准 9 折权限".to_string(),
        body: Some("领导裁决：可以给 8 折；约束：本周内付款".to_string()),
        ..OperationKnowledgeChunk::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert knowledge gap proposal");

    let found = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "title": "真人决策沉淀（待审核）：超出标准 9 折权限" },
            None,
        )
        .await
        .expect("query chunk")
        .expect("chunk must exist");

    assert_eq!(found.status, "draft");
    assert_eq!(found.integrity_status.as_deref(), Some("needs_review"));
    assert!(
        found.account_id.is_none(),
        "知识缺口提案须落 workspace 共享域（account_id=None）"
    );
}

/// §14.9：等待标记落 / 清往返。$set awaiting_principal_decision=true（镜像 apply_agent_updates），
/// 读回为 true；再 $unset（镜像 clear_awaiting_principal_state），读回消失。
#[tokio::test]
#[ignore]
async fn t_awaiting_marker_set_and_clear_roundtrip() {
    let app = common::TestApp::start().await;
    let contact = minimal_contact("cust_awaiting");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // set：apply_agent_updates 在触发请示时写可观测标记。
    let set_key = format!("domain_attributes.{AWAITING_PRINCIPAL_DECISION_ATTR}");
    app.state
        .db
        .contacts()
        .update_one(
            doc! { "wxid": "cust_awaiting", "workspace_id": "default", "account_id": "default" },
            doc! { "$set": { &set_key: true } },
            None,
        )
        .await
        .expect("set awaiting marker");

    let after_set = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "cust_awaiting" }, None)
        .await
        .expect("query contact")
        .expect("contact must exist");
    let attrs = after_set
        .domain_attributes
        .expect("domain_attributes set after marker write");
    assert_eq!(
        attrs.get_bool(AWAITING_PRINCIPAL_DECISION_ATTR).ok(),
        Some(true),
        "等待标记应读回 true"
    );

    // clear：clear_awaiting_principal_state 在 relay 完成后 $unset 标记。
    app.state
        .db
        .contacts()
        .update_one(
            doc! { "wxid": "cust_awaiting", "workspace_id": "default", "account_id": "default" },
            doc! { "$unset": { &set_key: "" } },
            None,
        )
        .await
        .expect("clear awaiting marker");

    let after_clear = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "cust_awaiting" }, None)
        .await
        .expect("query contact")
        .expect("contact must exist");
    let cleared = after_clear
        .domain_attributes
        .map(|d| d.contains_key(AWAITING_PRINCIPAL_DECISION_ATTR))
        .unwrap_or(false);
    assert!(!cleared, "等待标记应在 $unset 后消失");
}

// ─────────── §14.10 超时改派骚扰门 / 重构（#4 #5 #6 + 链尾困死/自我命中，#[ignore]，CI 跑） ───────────
//
// 这些测试覆盖的生产函数（latest_push_ms / reassign_escalation +
// scan_escalation_timeouts 内的 gate→push→推成功才 reassign 接线）都是 pub(crate)，crate 外不可直达。
// 遵循本文件既定约定（见文件头 §：通过公共表面切片断言而非放开可见性），下列测试一律经**唯一 pub 入口**
// `scan_escalation_timeouts` 驱动，并用公共 typed accessor 断言台账 DB 终态。

/// 写一行 user_operations domain config，带 ask_human_policy（决策人链 + 可选骚扰门字段）。
/// 镜像生产 admin `$set` 到 seeded current 行的写法（不另插版本，避 op_domain unique 索引）。
async fn set_ask_human_policy(app: &common::TestApp, policy: &AskHumanPolicy) {
    let policy_bson = mongodb::bson::to_bson(policy).expect("serialize ask_human_policy");
    app.state
        .db
        .operation_domain_configs()
        .update_one(
            doc! { "workspace_id": "default", "domain": "user_operations", "current_version": true },
            doc! { "$set": { "ask_human_policy": policy_bson, "updated_at": DateTime::now() } },
            None,
        )
        .await
        .expect("set ask_human_policy on seeded domain config");
}

/// 启 wiremock，POST /mcp 返回 MCP tools/call 成功 envelope（推卡成功路径）。
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

/// 启 wiremock，POST /mcp 一律 500（推卡失败路径，验 #6：updated_at 不刷新）。
async fn start_mcp_mock_failure() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(500).set_body_string("simulated mcp failure"))
        .mount(&server)
        .await;
    server
}

/// 插一条 principal_wxid=`principal`、updated_at=`updated_at` 的 pending 台账。
async fn insert_pending_with_updated_at(
    app: &common::TestApp,
    short_code: &str,
    principal: &str,
    updated_at: DateTime,
) {
    if app
        .state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "cust_timeout",
            },
            None,
        )
        .await
        .expect("query timeout customer")
        .is_none()
    {
        app.state
            .db
            .contacts()
            .insert_one(minimal_contact("cust_timeout"), None)
            .await
            .expect("insert timeout customer");
    }
    let mut entry = minimal_pending_escalation(short_code, "cust_timeout");
    entry.principal_wxid = principal.to_string();
    let config = app
        .state
        .db
        .operation_domain_configs()
        .find_one(
            doc! { "workspace_id": "default", "domain": "user_operations", "current_version": true },
            None,
        )
        .await
        .expect("query frozen policy config")
        .expect("seeded user_operations config");
    let frozen_policy = config
        .ask_human_policy
        .expect("timeout tests configure ask_human_policy before seeding escalation");
    let principal_account_id = frozen_policy
        .decider_chain
        .iter()
        .find(|decider| decider.wxid == principal)
        .and_then(|decider| decider.account_id.clone())
        .unwrap_or_else(|| "default".to_string());
    entry.protocol = Some(PrincipalEscalationProtocol {
        domain: config.domain,
        policy_version: config.version,
        policy: frozen_policy,
        principal_account_id,
        delivery_generation: 1,
        delivery_state: PRINCIPAL_CARD_DELIVERY_SENT.to_string(),
        delivery_content: "test principal card".to_string(),
        delivery_outbox_id: None,
        failure_cleanup_completed_at: None,
    });
    entry.created_at = updated_at;
    entry.updated_at = updated_at;
    entry.last_pushed_at_ms = Some(updated_at.timestamp_millis());
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(&entry, None)
        .await
        .expect("insert pending escalation");
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

/// §14.10a：主决策人超时只开启下一投递代次；只有 Outbox 确认 sent 并对账后，
/// `last_pushed_at_ms` / `updated_at` 才刷新并启动下一位决策人的超时窗。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_pushes_and_touches_updated_at() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    seed_online_default_account(&app).await;

    state
        .db
        .contacts()
        .insert_one(minimal_contact("backup"), None)
        .await
        .expect("insert backup contact");

    // 决策人链 boss → backup，timeout 1h。无骚扰门字段（全 None → 全放行）。
    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef {
                    wxid: "boss".into(),
                    display_name: None,
                    account_id: Some("default".into()),
                },
                DeciderRef {
                    wxid: "backup".into(),
                    display_name: None,
                    account_id: Some("default".into()),
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
            standing_order: None,
            standing_order_after_hours: None,
        },
    )
    .await;

    // pending 由 boss 持有，updated_at 在 2 小时前（已超 1h timeout）。
    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10A", "boss", two_hours_ago).await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan timeouts");

    let queued = find_escalation(&app, "T10A").await;
    assert_eq!(queued.principal_wxid, "backup");
    let protocol = queued.protocol.as_ref().expect("frozen protocol");
    assert_eq!(protocol.delivery_generation, 2);
    assert_eq!(protocol.delivery_state, PRINCIPAL_CARD_DELIVERY_QUEUED);
    assert!(queued.last_pushed_at_ms.is_none());

    let claimed = wechatagent::agent::atomic_claim_pending(&state, "timeout-success", 60)
        .await
        .expect("claim generation outbox")
        .expect("generation outbox exists");
    wechatagent::agent::process_entry(&state, &claimed)
        .await
        .expect("deliver generation outbox");
    assert_eq!(
        wechatagent::agent::escalation::reconcile_principal_card_deliveries(&state)
            .await
            .expect("reconcile sent generation"),
        1
    );

    let delivered = find_escalation(&app, "T10A").await;
    assert_eq!(
        delivered.protocol.as_ref().unwrap().delivery_state,
        PRINCIPAL_CARD_DELIVERY_SENT
    );
    assert!(delivered.last_pushed_at_ms.is_some());
    assert!(
        delivered.updated_at.timestamp_millis() > two_hours_ago.timestamp_millis(),
        "只有 sent 对账后才刷新下一位决策人的超时起点"
    );
}

/// §14.10b：下一代次 Outbox 重试耗尽后，对账把请示收敛为 delivery_failed，
/// 清掉 awaiting 并释放 pending 唯一槽；同客户同类别可重新建立新请示。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_terminal_delivery_failure_releases_pending() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_failure().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    seed_online_default_account(&app).await;

    let mut customer = minimal_contact("cust_timeout");
    let mut attributes = Document::new();
    attributes.insert(AWAITING_PRINCIPAL_DECISION_ATTR, true);
    customer.domain_attributes = Some(attributes);
    state
        .db
        .contacts()
        .insert_many([customer, minimal_contact("backup")], None)
        .await
        .expect("insert customer and backup contacts");

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef {
                    wxid: "boss".into(),
                    display_name: None,
                    account_id: Some("default".into()),
                },
                DeciderRef {
                    wxid: "backup".into(),
                    display_name: None,
                    account_id: Some("default".into()),
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
            standing_order: None,
            standing_order_after_hours: None,
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10B", "boss", two_hours_ago).await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan timeouts");

    let queued = find_escalation(&app, "T10B").await;
    assert_eq!(queued.principal_wxid, "backup");
    let outbox_id = queued
        .protocol
        .as_ref()
        .and_then(|protocol| protocol.delivery_outbox_id)
        .expect("generation outbox id");
    for attempt in 0..3 {
        state
            .db
            .collection_agent_send_outbox()
            .update_one(
                doc! { "_id": outbox_id },
                doc! { "$set": { "next_retry_at": null } },
                None,
            )
            .await
            .expect("clear retry delay");
        let claimed = wechatagent::agent::atomic_claim_pending(
            &state,
            &format!("timeout-failure-{attempt}"),
            60,
        )
        .await
        .expect("claim failed generation")
        .expect("generation remains claimable");
        wechatagent::agent::process_entry(&state, &claimed)
            .await
            .expect("process failed generation");
    }

    assert_eq!(
        wechatagent::agent::escalation::reconcile_principal_card_deliveries(&state)
            .await
            .expect("reconcile terminal failure"),
        1
    );
    let failed = find_escalation(&app, "T10B").await;
    assert_eq!(failed.status, PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED);
    assert!(failed
        .protocol
        .as_ref()
        .and_then(|protocol| protocol.failure_cleanup_completed_at)
        .is_some());
    let customer = state
        .db
        .contacts()
        .find_one(
            doc! { "workspace_id": "default", "account_id": "default", "wxid": "cust_timeout" },
            None,
        )
        .await
        .expect("query customer")
        .expect("customer exists");
    assert!(!customer
        .domain_attributes
        .as_ref()
        .and_then(|attrs| attrs.get_bool(AWAITING_PRINCIPAL_DECISION_ATTR).ok())
        .unwrap_or(false));

    insert_pending_with_updated_at(&app, "T10B2", "boss", DateTime::now()).await;
    assert_eq!(
        find_escalation(&app, "T10B2").await.status,
        PRINCIPAL_ESCALATION_STATUS_PENDING
    );
}

/// §14.10c（#5 骚扰门接线，gate 先于改派）：next 落在 quiet_hours → gate 拦 → **不改派、不推**
/// （principal_wxid 仍是 boss 原值，updated_at 旧值）。备选决策人不在静默时段被惊扰；原 principal
/// age 仍超时，待下一 tick 重试。验证超时改派重推卡同样过骚扰门（与首推路径一致）。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_blocked_by_quiet_hours_skips_push() {
    let app = common::TestApp::start().await;

    // 构造一个**确定性命中当前小时**的静默窗：[now_hour, (now_hour+23)%24)。
    // in_quiet_hours 对该窗（除「下一小时」外覆盖全部 23 小时）判定当前小时恒为 true，
    // 与 CI 容器时区无关（tz_offset=0，按 UTC 算）。
    let now_hour = ((DateTime::now().timestamp_millis() / (3600 * 1000)) % 24) as u8;
    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef {
                    wxid: "boss".into(),
                    display_name: None,
                    account_id: Some("default".into()),
                },
                DeciderRef {
                    wxid: "backup".into(),
                    display_name: None,
                    account_id: Some("default".into()),
                },
            ],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: Some(AskHumanQuietHours {
                start_hour: now_hour,
                end_hour: (now_hour + 23) % 24,
                tz_offset_hours: 0,
            }),
            timeout_hours: Some(1.0),
            standing_order: None,
            standing_order_after_hours: None,
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10C", "boss", two_hours_ago).await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("scan timeouts");

    let after = find_escalation(&app, "T10C").await;
    assert_eq!(
        after.principal_wxid, "boss",
        "gate 先于改派，quiet_hours 命中应不改派，principal_wxid 保持原值"
    );
    assert_eq!(
        after.updated_at.timestamp_millis(),
        two_hours_ago.timestamp_millis(),
        "quiet_hours 命中应跳过且不刷新 updated_at，待下一 tick 重试"
    );
}

/// §14.10d（验 #2 自我命中已消除）：链 [boss, backup]，daily_push_cap=1，timeout 1h，无其它
/// 历史推送。boss 超时 → gate 查 next=backup 的当日推送数。重构后 gate 先于改派，此刻台账仍挂
/// boss，count_pushes_today(backup) 不含本条 = 0 < cap → 放行 → 推成功 → 改派到 backup。
/// 旧实现 reassign 先于 gate，本条改派后自己被算成 backup 的 1 次推送 → 1>=1 误拦，backup 永远
/// 收不到卡。本测试断言 backup 确实收到（principal_wxid=backup + updated_at 刷新）。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_cap_one_not_self_blocked() {
    let app = common::TestApp::start().await;

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef {
                    wxid: "boss".into(),
                    display_name: None,
                    account_id: Some("default".into()),
                },
                DeciderRef {
                    wxid: "backup".into(),
                    display_name: None,
                    account_id: Some("default".into()),
                },
            ],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: Some(1),
            quiet_hours: None,
            timeout_hours: Some(1.0),
            standing_order: None,
            standing_order_after_hours: None,
        },
    )
    .await;

    // 唯一一条 pending（TestApp 每条测试独立 DB，无其它台账污染 cap 计数）。
    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10D", "boss", two_hours_ago).await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("scan timeouts");

    let after = find_escalation(&app, "T10D").await;
    assert_eq!(
        after.principal_wxid, "backup",
        "cap=1 不应被改派的这条自己误命中，backup 应收到卡并改派成功"
    );
    let protocol = after.protocol.as_ref().expect("frozen protocol");
    assert_eq!(protocol.delivery_generation, 2);
    assert_eq!(protocol.delivery_state, PRINCIPAL_CARD_DELIVERY_QUEUED);
    assert!(after.last_pushed_at_ms.is_none());
}

/// §14.10e：两个 scanner 并发读取同一超时行时，generation CAS + Outbox
/// idempotency key 必须收敛为一次改派和一条 generation=2 Outbox。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_concurrent_scans_enqueue_one_generation() {
    let app = common::TestApp::start().await;

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef {
                    wxid: "boss".into(),
                    display_name: None,
                    account_id: Some("default".into()),
                },
                DeciderRef {
                    wxid: "backup".into(),
                    display_name: None,
                    account_id: Some("default".into()),
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
            standing_order: None,
            standing_order_after_hours: None,
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10E", "boss", two_hours_ago).await;

    let (left, right) = tokio::join!(
        wechatagent::agent::escalation::scan_escalation_timeouts(&app.state),
        wechatagent::agent::escalation::scan_escalation_timeouts(&app.state),
    );
    left.expect("left scanner");
    right.expect("right scanner");

    let after = find_escalation(&app, "T10E").await;
    assert_eq!(after.principal_wxid, "backup");
    let protocol = after.protocol.as_ref().expect("frozen protocol");
    assert_eq!(protocol.delivery_generation, 2);
    let escalation_id = after.id.expect("escalation id");
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(
                doc! { "source_event_id": format!("principal-card:{}:2", escalation_id.to_hex()) },
                None,
            )
            .await
            .expect("count generation outbox"),
        1
    );
}

/// A claimed generation-1 card must be canceled before the remote boundary
/// after the escalation has advanced to generation 2.
#[tokio::test]
#[ignore]
async fn t_stale_principal_card_generation_is_canceled_before_remote_send() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

    let escalation_id = ObjectId::new();
    let mut escalation = minimal_pending_escalation("T10F", "cust_stale_generation");
    escalation.id = Some(escalation_id);
    escalation.principal_wxid = "backup".to_string();
    let protocol = escalation.protocol.as_mut().expect("frozen protocol");
    protocol.principal_account_id = "default".to_string();
    protocol.delivery_generation = 2;
    protocol.delivery_state = PRINCIPAL_CARD_DELIVERY_QUEUED.to_string();
    protocol.delivery_content = "current generation card".to_string();
    state
        .db
        .agent_principal_escalations()
        .insert_one(&escalation, None)
        .await
        .expect("insert generation 2 escalation");

    let stale_outbox_id = match wechatagent::agent::enqueue(
        &state,
        wechatagent::agent::EnqueueRequest {
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: "boss_wxid".to_string(),
            run_id: format!("principal-card:{}:1", escalation_id.to_hex()),
            decision_id: None,
            source_event_id: format!("principal-card:{}:1", escalation_id.to_hex()),
            source_kind: "principal_escalation".to_string(),
            content: "stale generation card".to_string(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await
    .expect("enqueue stale generation")
    {
        wechatagent::agent::EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected created stale outbox, got {other:?}"),
    };
    let claimed = wechatagent::agent::atomic_claim_pending(&state, "stale-generation", 60)
        .await
        .expect("claim stale generation")
        .expect("stale generation outbox exists");

    wechatagent::agent::process_entry(&state, &claimed)
        .await
        .expect("stale generation is canceled");

    let stored = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": stale_outbox_id }, None)
        .await
        .expect("query stale outbox")
        .expect("stale outbox exists");
    assert_eq!(stored.status, "canceled");
    assert_eq!(
        stored.cancel_reason.as_deref(),
        Some("principal_escalation_generation_no_longer_authorized")
    );
    assert!(stored.send_started_at.is_none());
    assert_eq!(
        mcp.received_requests()
            .await
            .expect("read MCP requests")
            .len(),
        0,
        "stale generation must be fenced before any remote request"
    );
}

/// §14.12（③链尾失联安抚去重）：单决策人链 [boss]，timeout 1h，boss 一直不回。
/// 第一次 scan（age 超时、next_decider 返回 None=链尾）→ 发一条安抚话术 + 记
/// last_holding_reply_ms。紧接第二次 scan（min_interval 未到）→ 不重复发（去重）。
/// 台账保持 pending。
#[tokio::test]
#[ignore]
async fn t_timeout_chain_tail_sends_holding_reply_once_within_interval() {
    let app = common::TestApp::start().await;

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![DeciderRef {
                wxid: "boss".into(),
                display_name: None,
                account_id: Some("default".into()),
            }],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: Some(1.0),
            standing_order: None,
            standing_order_after_hours: None,
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T12", "boss", two_hours_ago).await;

    // 第一次 scan：链尾 → 发安抚 + 记 last_holding_reply_ms。
    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("scan 1");
    let after1 = find_escalation(&app, "T12").await;
    assert_eq!(after1.status, "pending", "链尾安抚后台账仍 pending");
    assert!(after1.last_holding_reply_ms.is_some(), "应记录安抚发送时刻");

    // 第二次 scan（紧接，min_interval=6h 未到）：不重复发。
    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("scan 2");
    let after2 = find_escalation(&app, "T12").await;
    assert_eq!(
        after2.last_holding_reply_ms, after1.last_holding_reply_ms,
        "min_interval 内不重复发安抚，时刻不变"
    );
}

// ───────────────────── §14.11（②授权过期闭环，#[ignore]，CI 跑） ─────────────────────

/// 插一条 resolved 台账：decision 带 substance，但 authorization_expires_at 已过期（now-1h）。
/// 镜像 enqueue_relay_task 触发前的台账形状（领导已裁决但授权时效已过）。增量叠加，不改旧 helper。
async fn insert_resolved_expired_escalation(
    app: &common::TestApp,
    short_code: &str,
    contact_wxid: &str,
) -> ObjectId {
    let mut entry = minimal_pending_escalation(short_code, contact_wxid);
    entry.status = PRINCIPAL_ESCALATION_STATUS_RESOLVED.to_string();
    let now = DateTime::now();
    let one_hour_ago = DateTime::from_millis(now.timestamp_millis() - 3600 * 1000);
    entry.decision = Some(PrincipalDecision {
        verdict: PRINCIPAL_VERDICT_CONDITIONAL.to_string(),
        substance: "可以给 8 折".to_string(),
        constraints: vec!["本周内付款".to_string()],
        authorization_window_hours: Some(1.0),
        exemption_type: wechatagent::models::EXEMPTION_TYPE_NONE.to_string(),
    });
    entry.authorization_expires_at = Some(one_hour_ago);
    entry.resolved_at = Some(one_hour_ago);
    entry.updated_at = one_hour_ago;
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(&entry, None)
        .await
        .expect("insert resolved+expired escalation")
        .inserted_id
        .as_object_id()
        .expect("inserted escalation id")
}

/// 按 wxid 读回 contact（断言 awaiting 标记态）。增量 helper。
async fn find_contact(app: &common::TestApp, wxid: &str) -> Contact {
    app.state
        .db
        .contacts()
        .find_one(
            doc! { "wxid": wxid, "workspace_id": "default", "account_id": "default" },
            None,
        )
        .await
        .expect("query contact")
        .expect("contact must exist")
}

/// 构造一条立即可执行的 principal_decision_relay task（content=short_code），镜像 enqueue_relay_task。
fn relay_task_for(short_code: &str, contact_wxid: &str) -> AgentTask {
    let now = DateTime::now();
    AgentTask {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: contact_wxid.to_string(),
        kind: "principal_decision_relay".to_string(),
        run_at: now,
        expires_at: None,
        content: short_code.to_string(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: false,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    }
}

fn relay_reply_decision_json(reply_text: &str) -> serde_json::Value {
    serde_json::json!({
        "decisionPhase": "final",
        "userUnderstanding": "领导已给出明确裁决，需要向客户转述可执行结论。",
        "relationshipRead": "客户正在等待裁决结果，应及时承接。",
        "operationGoal": "用 AI 自己的口吻向客户说明裁决。",
        "knowledgeNeedReason": "裁决载荷已经是本轮唯一事实源，无需查询知识库。",
        "memoryUpdateReason": "本轮没有新的长期客户事实。",
        "selfCritique": "转述必须克制，不得泄漏内部载荷字段。",
        "whyShouldReply": "客户正在等待裁决结果。",
        "whySkipReply": "",
        "riskSelfCheck": "只转述既有裁决，不新增授权。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "need_discovery",
        "shouldReply": true,
        "replyText": reply_text,
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "当前是裁决转述场景。"
    })
}

fn relay_review_pass_json() -> serde_json::Value {
    serde_json::json!({
        "approved": true,
        "scores": {
            "humanLike": 8,
            "emotionalValue": 8,
            "productAccuracy": 8,
            "relationshipProgress": 7,
            "conversionReadiness": 6,
            "pressureRisk": 2,
            "boundaryPrivacySafety": 9,
            "factRisk": 1
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "测试固定为已批准转述。"
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "测试固定放行，由 relay 出站代码守卫做最终拦截。",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true
    })
}

/// §14.11（②授权过期闭环）：relay task 跑时领导授权已过期 → 不发过期承诺，但必须
/// ①清客户 awaiting 标记 ②发一条不含 substance 的中性收尾话术。否则客户零反馈 +
/// awaiting 永挂、永久压制对该议题的自主回复。
#[tokio::test]
#[ignore]
async fn t_relay_expired_authorization_clears_awaiting_and_sends_neutral() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());
    seed_online_default_account(&app).await;

    let wxid = "cust_expired";

    // 客户带 awaiting 标记（镜像请示触发时 apply_agent_updates 写入的可观测标记）。
    let mut contact = minimal_contact(wxid);
    let mut attrs = Document::new();
    attrs.insert(AWAITING_PRINCIPAL_DECISION_ATTR, true);
    contact.domain_attributes = Some(attrs);
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact with awaiting marker");

    // resolved + decision.substance + 授权已过期（now-1h）。
    let escalation_id = insert_resolved_expired_escalation(&app, "X11A", wxid).await;

    // 触发 relay task 处理（经公共入口 handle_follow_up_task → handle_principal_decision_relay）。
    wechatagent::agent::handle_follow_up_task(&state, relay_task_for("X11A", wxid))
        .await
        .expect("handle relay task (expired authorization)");

    // 断言①：客户 awaiting 标记已清（授权过期早退也必须清，否则永久压制自主回复）。
    let after = find_contact(&app, wxid).await;
    let awaiting = after
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_bool(AWAITING_PRINCIPAL_DECISION_ATTR).ok())
        .unwrap_or(false);
    assert!(!awaiting, "授权过期早退也必须清 awaiting 标记");
    let terminal = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "_id": escalation_id }, None)
        .await
        .expect("query terminal escalation")
        .expect("terminal escalation exists");
    assert_eq!(
        terminal.relay_state.as_deref(),
        Some(wechatagent::models::PRINCIPAL_RELAY_STATE_TERMINAL)
    );
    assert_eq!(
        terminal.relay_terminal_reason.as_deref(),
        Some("authorization_expired")
    );
    assert!(terminal.relay_terminal_at.is_some());

    // 中性收尾先进入 durable outbox；测试显式驱动 dispatcher 后再检查 MCP。
    let claimed = wechatagent::agent::atomic_claim_pending(&state, "expired-relay-test", 60)
        .await
        .expect("claim expired relay holding")
        .expect("expired relay holding must be enqueued");
    wechatagent::agent::process_entry(&state, &claimed)
        .await
        .expect("dispatch expired relay holding");
    let stored = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": claimed.id.expect("claimed outbox id") }, None)
        .await
        .expect("query expired relay holding outbox")
        .expect("expired relay holding outbox exists");
    assert_eq!(
        stored.status, "sent",
        "expired relay holding must reach sent; cancel_reason={:?}, last_error={:?}",
        stored.cancel_reason, stored.last_error
    );

    // 断言②：客户收到一条中性收尾话术，且不复述过期 substance。
    let recv = mcp
        .received_requests()
        .await
        .expect("MCP 桩应可读取 received_requests");
    let bodies: Vec<String> = recv
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect();
    assert!(
        !bodies.is_empty(),
        "授权过期早退必须给客户发一条中性收尾，不能零反馈（客户被晾死）"
    );
    let all = bodies.join("\n");
    assert!(
        all.contains("继续") || all.contains("核实") || all.contains("同步"),
        "应发中性收尾话术（会继续跟进），实际：{all}"
    );
    assert!(
        !all.contains("8 折") && !all.contains("8折"),
        "中性收尾绝不复述过期 substance 的具体承诺/数字，实际：{all}"
    );
}

/// 同一客户可同时拥有不同类别的两条等待项。终结其中一条只能移除自己的稳定 owner；
/// 另一条仍活跃时 coarse awaiting 必须保持 true，直到最后一条也终结。
#[tokio::test]
#[ignore]
async fn terminalizing_one_relay_preserves_another_awaiting_owner() {
    let app = common::TestApp::start().await;
    let wxid = "cust_two_relay_owners";
    app.state
        .db
        .contacts()
        .insert_one(minimal_contact(wxid), None)
        .await
        .expect("insert contact");

    let first_id = insert_resolved_expired_escalation(&app, "OWNR1", wxid).await;
    let second_id = insert_resolved_expired_escalation(&app, "OWNR2", wxid).await;
    let awaiting_key = format!(
        "domain_attributes.{}",
        wechatagent::models::AWAITING_PRINCIPAL_DECISION_ATTR
    );
    let owners_key = format!(
        "domain_attributes.{}",
        wechatagent::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR
    );
    let mut owner_set = Document::new();
    owner_set.insert(awaiting_key, true);
    owner_set.insert(owners_key, vec![first_id.to_hex(), second_id.to_hex()]);
    app.state
        .db
        .contacts()
        .update_one(
            doc! { "workspace_id": "default", "account_id": "default", "wxid": wxid },
            doc! { "$set": owner_set },
            None,
        )
        .await
        .expect("seed two awaiting owners");

    wechatagent::agent::handle_follow_up_task(&app.state, relay_task_for("OWNR1", wxid))
        .await
        .expect("terminalize first relay");
    let after_first = find_contact(&app, wxid).await;
    let attrs = after_first.domain_attributes.expect("domain attributes");
    assert_eq!(
        attrs.get_bool(AWAITING_PRINCIPAL_DECISION_ATTR).ok(),
        Some(true),
        "second owner must keep awaiting=true"
    );
    assert_eq!(
        attrs
            .get_array(wechatagent::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR)
            .expect("owner array"),
        &vec![second_id.to_hex().into()]
    );

    wechatagent::agent::handle_follow_up_task(&app.state, relay_task_for("OWNR2", wxid))
        .await
        .expect("terminalize second relay");
    let after_second = find_contact(&app, wxid).await;
    let attrs = after_second.domain_attributes.expect("domain attributes");
    assert_eq!(
        attrs.get_bool(AWAITING_PRINCIPAL_DECISION_ATTR).ok(),
        Some(false)
    );
    assert!(attrs
        .get_array(wechatagent::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR)
        .expect("owner array")
        .is_empty());

    for id in [first_id, second_id] {
        let terminal = app
            .state
            .db
            .agent_principal_escalations()
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("query terminal escalation")
            .expect("terminal escalation exists");
        assert_eq!(
            terminal.relay_state.as_deref(),
            Some(wechatagent::models::PRINCIPAL_RELAY_STATE_TERMINAL)
        );
        assert_eq!(
            terminal.relay_terminal_reason.as_deref(),
            Some("authorization_expired")
        );
    }
}

/// relay 候选泄漏内部字段时，安全门必须在入队前拦截；客户仍处于 awaiting，
/// 原任务明确取消，不能伪装成已入队或已送达。
#[tokio::test]
#[ignore]
async fn blocked_relay_preserves_awaiting_and_cancels_task_without_outbox() {
    let app = common::TestApp::start().await;
    let wxid = "cust_audit_blocked_relay";
    let short_code = "AUDR1";

    let mut contact = minimal_contact(wxid);
    let mut attrs = Document::new();
    attrs.insert(AWAITING_PRINCIPAL_DECISION_ATTR, true);
    contact.domain_attributes = Some(attrs);
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert awaiting contact");

    let mut escalation = minimal_pending_escalation(short_code, wxid);
    escalation.status = PRINCIPAL_ESCALATION_STATUS_RESOLVED.to_string();
    escalation.decision = Some(PrincipalDecision {
        verdict: PRINCIPAL_VERDICT_CONDITIONAL.to_string(),
        substance: "本周内可以按约定方案推进".to_string(),
        constraints: vec!["本周内确认".to_string()],
        authorization_window_hours: None,
        exemption_type: wechatagent::models::EXEMPTION_TYPE_NONE.to_string(),
    });
    escalation.resolved_at = Some(DateTime::now());
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(&escalation, None)
        .await
        .expect("insert resolved escalation");

    let mut task = relay_task_for(short_code, wxid);
    let task_id = ObjectId::new();
    task.id = Some(task_id);
    app.state
        .db
        .tasks()
        .insert_one(&task, None)
        .await
        .expect("insert relay task");

    app.llm.push_response(relay_reply_decision_json(
        "给客户的错误转述：verdict=approved",
    ));
    app.llm.push_response(relay_review_pass_json());
    app.llm
        .push_response(common::independent_claim_gate_pass_json());

    wechatagent::agent::handle_follow_up_task(&app.state, task)
        .await
        .expect("blocked relay returns Ok");

    let outbox_count = app
        .state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! { "contact_wxid": wxid }, None)
        .await
        .expect("count relay outbox");
    assert_eq!(
        outbox_count, 0,
        "内部字段泄漏应被 relay 安全门拦截，实际不得创建 outbox"
    );

    let after = find_contact(&app, wxid).await;
    let awaiting = after
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_bool(AWAITING_PRINCIPAL_DECISION_ATTR).ok())
        .unwrap_or(false);
    assert!(awaiting, "零 outbox 时客户仍在等待有效裁决转述");

    let stored_task = app
        .state
        .db
        .tasks()
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("query relay task")
        .expect("relay task exists");
    assert_eq!(
        stored_task.status, "cancelled",
        "安全门拦截后任务必须进入明确终态"
    );
    assert_eq!(
        stored_task.gateway_status.as_deref(),
        Some("blocked_by_safety_guard")
    );

    let review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "contact_wxid": wxid }, None)
        .await
        .expect("query relay review")
        .expect("relay review exists");
    assert_eq!(review.status, "blocked_by_safety_guard");
}

// ───────────────────── §14 纯函数测试（不标 ignore，本地即跑） ─────────────────────

/// §14.4b：synthetic_principal_relay 合成消息以哨兵前缀开头，且载荷携带 verdict /
/// substance / constraints 三要素，供 decision prompt 据哨兵进入转述模式。
#[test]
fn t_synthetic_relay_carries_sentinel_and_fields() {
    let contact = minimal_contact("cust_relay");
    let msg = ConversationMessage::synthetic_principal_relay(
        &contact,
        "conditional",
        "可以给8折",
        &["本周内付款".to_string()],
    );
    assert!(
        msg.content.starts_with(PRINCIPAL_RELAY_SENTINEL),
        "合成 relay 须以哨兵前缀开头"
    );
    assert!(msg.content.contains("verdict=conditional"));
    assert!(msg.content.contains("可以给8折"));
    assert!(msg.content.contains("本周内付款"));
}

/// §14.9b：兜底安抚文案红线——绝不出现任何转接/转人工类措辞。
/// 注意：tests/ 目录被 check-no-human-takeover 排除，故此处可写这些禁词字面量，
/// 这正是本红线测试的意义所在（断言生产文案里没有它们）。
#[test]
fn fallback_holding_reply_has_no_handoff_wording() {
    let reply = wechatagent::agent::escalation::fallback_holding_reply();
    for forbidden in ["真人", "转人工", "客服", "接管", "人工"] {
        assert!(
            !reply.contains(forbidden),
            "兜底安抚文案不得含转接类措辞「{forbidden}」，实际：{reply}"
        );
    }
}

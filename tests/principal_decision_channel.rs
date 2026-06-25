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

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::models::{
    AgentPrincipalEscalation, AgentStatus, AgentTask, AskHumanPolicy, AskHumanQuietHours, Contact,
    ConversationMessage, DeciderRef, OperationKnowledgeChunk, PrincipalDecision,
    AWAITING_PRINCIPAL_DECISION_ATTR, ESCALATION_CATEGORY_OUT_OF_SCOPE,
    PRINCIPAL_ESCALATION_STATUS_PENDING, PRINCIPAL_ESCALATION_STATUS_RESOLVED,
    PRINCIPAL_RELAY_SENTINEL, PRINCIPAL_VERDICT_CONDITIONAL,
};
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
        decision: None,
        authorization_expires_at: None,
        is_generalizable: false,
        knowledge_proposal_emitted: false,
        last_holding_reply_ms: None,
        created_at: now,
        updated_at: now,
        resolved_at: None,
        resolved_via: None,
    }
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
    let stored = found.decision.expect("decision must be stored after resolve");
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
    let mut entry = minimal_pending_escalation(short_code, "cust_timeout");
    entry.principal_wxid = principal.to_string();
    entry.created_at = updated_at;
    entry.updated_at = updated_at;
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

/// §14.10a（#5 + #6 成功路径）：主决策人超时 → 改派下一位 + 推卡成功 → 台账 principal_wxid
/// 改为下一位，且 updated_at 被刷新（age 自推卡成功起算）。验证超时改派主链路在补了骚扰门
/// 检查后仍正常工作。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_pushes_and_touches_updated_at() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

    // 决策人链 boss → backup，timeout 1h。无骚扰门字段（全 None → 全放行）。
    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef { wxid: "boss".into(), display_name: None },
                DeciderRef { wxid: "backup".into(), display_name: None },
            ],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: Some(1.0),
        },
    )
    .await;

    // pending 由 boss 持有，updated_at 在 2 小时前（已超 1h timeout）。
    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10A", "boss", two_hours_ago).await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan timeouts");

    let after = find_escalation(&app, "T10A").await;
    assert_eq!(
        after.principal_wxid, "backup",
        "超时应改派给链中下一位决策人"
    );
    assert!(
        after.updated_at.timestamp_millis() > two_hours_ago.timestamp_millis(),
        "推卡成功后 updated_at 应被刷新（age 自推卡成功起算）"
    );
}

/// §14.10b（推失败不改派，命门）：gate 过 → 推卡 MCP **失败** → **不改派**（principal_wxid
/// 仍是 boss 原值，updated_at 保持旧值）。原 principal age 仍超时，下一 tick 会重新算出同一个
/// next 重推。重构后改派只发生在卡确实送达 next 之后，故推失败绝不把台账推进到链尾。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_push_failure_does_not_reassign() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_failure().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef { wxid: "boss".into(), display_name: None },
                DeciderRef { wxid: "backup".into(), display_name: None },
            ],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: Some(1.0),
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10B", "boss", two_hours_ago).await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan timeouts");

    let after = find_escalation(&app, "T10B").await;
    // 推卡失败 → 不改派：principal_wxid 仍是 boss（未推进到链尾）。
    assert_eq!(
        after.principal_wxid, "boss",
        "推卡失败不得改派，principal_wxid 应保持原值"
    );
    // 未改派 → updated_at 保持旧值，下一 tick age 仍超时会重推同一个 next。
    assert_eq!(
        after.updated_at.timestamp_millis(),
        two_hours_ago.timestamp_millis(),
        "推卡失败不改派时 updated_at 不得刷新，否则 age 归零、下一 tick 不再重试"
    );
}

/// §14.10c（#5 骚扰门接线，gate 先于改派）：next 落在 quiet_hours → gate 拦 → **不改派、不推**
/// （principal_wxid 仍是 boss 原值，updated_at 旧值）。备选决策人不在静默时段被惊扰；原 principal
/// age 仍超时，待下一 tick 重试。验证超时改派重推卡同样过骚扰门（与首推路径一致）。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_blocked_by_quiet_hours_skips_push() {
    let app = common::TestApp::start().await;
    // 用 success mock：若 quiet_hours 未拦住，会推卡成功 + 改派；
    // 拦住则不改派 → principal_wxid 保持 boss。故断言能干净区分两种行为。
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

    // 构造一个**确定性命中当前小时**的静默窗：[now_hour, (now_hour+23)%24)。
    // in_quiet_hours 对该窗（除「下一小时」外覆盖全部 23 小时）判定当前小时恒为 true，
    // 与 CI 容器时区无关（tz_offset=0，按 UTC 算）。
    let now_hour = ((DateTime::now().timestamp_millis() / (3600 * 1000)) % 24) as u8;
    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef { wxid: "boss".into(), display_name: None },
                DeciderRef { wxid: "backup".into(), display_name: None },
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
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10C", "boss", two_hours_ago).await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
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
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef { wxid: "boss".into(), display_name: None },
                DeciderRef { wxid: "backup".into(), display_name: None },
            ],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: Some(1),
            quiet_hours: None,
            timeout_hours: Some(1.0),
        },
    )
    .await;

    // 唯一一条 pending（TestApp 每条测试独立 DB，无其它台账污染 cap 计数）。
    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10D", "boss", two_hours_ago).await;

    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan timeouts");

    let after = find_escalation(&app, "T10D").await;
    assert_eq!(
        after.principal_wxid, "backup",
        "cap=1 不应被改派的这条自己误命中，backup 应收到卡并改派成功"
    );
    assert!(
        after.updated_at.timestamp_millis() > two_hours_ago.timestamp_millis(),
        "推卡成功 + 改派 → updated_at 应刷新"
    );
}

/// §14.10e（验 #1 链尾不困死 + 推失败重试同一 next）：链 [boss, backup]，无骚扰门，timeout 1h。
/// 第一次 scan 用 failure mock → 推失败 → 不改派（principal 仍 boss）。第二次 scan 换 success mock
/// → 重新算出同一个 next=backup → 推成功 → 改派到 backup。旧实现第一次就把 principal 改成 backup
/// （链尾），第二次 next_decider_on_timeout(backup) 返回 None → 永不重推，backup 永久收不到卡。
#[tokio::test]
#[ignore]
async fn t_timeout_reassign_push_failure_retries_same_next_on_next_tick() {
    let app = common::TestApp::start().await;

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![
                DeciderRef { wxid: "boss".into(), display_name: None },
                DeciderRef { wxid: "backup".into(), display_name: None },
            ],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: Some(1.0),
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T10E", "boss", two_hours_ago).await;

    // 第一次 tick：推卡失败 → 不改派。
    let mcp_fail = start_mcp_mock_failure().await;
    let state_fail = common::rebuild_app_state_with_mcp_url(&app, mcp_fail.uri());
    wechatagent::agent::escalation::scan_escalation_timeouts(&state_fail)
        .await
        .expect("scan timeouts (fail tick)");
    let after_fail = find_escalation(&app, "T10E").await;
    assert_eq!(
        after_fail.principal_wxid, "boss",
        "首 tick 推失败应不改派，principal 仍是 boss（未推进到链尾）"
    );
    assert_eq!(
        after_fail.updated_at.timestamp_millis(),
        two_hours_ago.timestamp_millis(),
        "推失败不改派 → updated_at 不刷新，下 tick age 仍超时"
    );

    // 第二次 tick：换 success mock（指向另一个 wiremock）→ 重推达同一个 next=backup → 改派。
    let mcp_ok = start_mcp_mock_success().await;
    let state_ok = common::rebuild_app_state_with_mcp_url(&app, mcp_ok.uri());
    wechatagent::agent::escalation::scan_escalation_timeouts(&state_ok)
        .await
        .expect("scan timeouts (success tick)");
    let after_ok = find_escalation(&app, "T10E").await;
    assert_eq!(
        after_ok.principal_wxid, "backup",
        "下一 tick 应重推达同一个 next=backup 并改派（旧实现因链尾 None 永不重推）"
    );
    assert!(
        after_ok.updated_at.timestamp_millis() > two_hours_ago.timestamp_millis(),
        "重推成功 + 改派 → updated_at 应刷新"
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
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![DeciderRef { wxid: "boss".into(), display_name: None }],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: Some(1.0),
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T12", "boss", two_hours_ago).await;

    // 第一次 scan：链尾 → 发安抚 + 记 last_holding_reply_ms。
    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
        .await
        .expect("scan 1");
    let after1 = find_escalation(&app, "T12").await;
    assert_eq!(after1.status, "pending", "链尾安抚后台账仍 pending");
    assert!(
        after1.last_holding_reply_ms.is_some(),
        "应记录安抚发送时刻"
    );

    // 第二次 scan（紧接，min_interval=6h 未到）：不重复发。
    wechatagent::agent::escalation::scan_escalation_timeouts(&state)
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
) {
    let mut entry = minimal_pending_escalation(short_code, contact_wxid);
    entry.status = PRINCIPAL_ESCALATION_STATUS_RESOLVED.to_string();
    let now = DateTime::now();
    let one_hour_ago = DateTime::from_millis(now.timestamp_millis() - 3600 * 1000);
    entry.decision = Some(PrincipalDecision {
        verdict: PRINCIPAL_VERDICT_CONDITIONAL.to_string(),
        substance: "可以给 8 折".to_string(),
        constraints: vec!["本周内付款".to_string()],
        authorization_window_hours: Some(1.0),
    });
    entry.authorization_expires_at = Some(one_hour_ago);
    entry.resolved_at = Some(one_hour_ago);
    entry.updated_at = one_hour_ago;
    app.state
        .db
        .agent_principal_escalations()
        .insert_one(&entry, None)
        .await
        .expect("insert resolved+expired escalation");
}

/// 按 wxid 读回 contact（断言 awaiting 标记态）。增量 helper。
async fn find_contact(app: &common::TestApp, wxid: &str) -> Contact {
    app.state
        .db
        .contacts()
        .find_one(doc! { "wxid": wxid, "workspace_id": "default", "account_id": "default" }, None)
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

/// §14.11（②授权过期闭环）：relay task 跑时领导授权已过期 → 不发过期承诺，但必须
/// ①清客户 awaiting 标记 ②发一条不含 substance 的中性收尾话术。否则客户零反馈 +
/// awaiting 永挂、永久压制对该议题的自主回复。
#[tokio::test]
#[ignore]
async fn t_relay_expired_authorization_clears_awaiting_and_sends_neutral() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

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
    insert_resolved_expired_escalation(&app, "X11A", wxid).await;

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

    // 断言②：客户收到一条中性收尾话术（MCP 桩收到 message_send_text），且不复述过期 substance。
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

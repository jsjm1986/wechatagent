//! 深度审查 G14：`operation_state` 派生写入点（gateway.rs `apply_agent_updates`
//! :2735-2820）的**确定性 mock-LLM E2E 覆盖**。
//!
//! ## 背景：补的是哪三段逻辑
//! `tests/c2_state_transition_cross_domain.rs` 已用纯函数确定性覆盖
//! `check_state_transition` 的判定正确性，但**显式放弃**了 gateway 端到端：派生写入点的
//! 三段关键逻辑（synced_state 取值优先级 / fail-soft 非法迁移跳写 / 审计事件）此前
//! **无任何 E2E 断言**。本文件用 **mock-LLM**（不是真模型）驱动 `handle_managed_message`
//! 全链，确定性覆盖：
//!
//! 1. **synced_state 取值优先级**（:2742-2750）：operation_state 优先派生自
//!    `domain_signals.customer_stage`，仅在 customer_stage 缺失时回落
//!    `decision.operation_state`。`normal_transition_*` 用例同时给 customerStage 与
//!    一个**不同的** operationState，断言最终落库值取的是 customerStage 派生值——证明
//!    优先级而非简单二选一。
//! 2. **fail-soft 非法迁移跳写**（:2756-2766）：`check_state_transition` 判非法时
//!    **不写** operation_state（保留旧值），记 rejected。`illegal_transition_*` 用例断言
//!    operation_state 保留旧值、未被非法值覆盖。
//! 3. **审计事件**（:2906-2920）：非法迁移写一条 `agent.operation_state_transition_rejected`
//!    审计事件（details 含 prior_state/attempted_state/reason），且 reply 仍照常下发
//!    （fail-soft，gateway 走 approved + outbox 入队，不是 blocked）。
//!
//! ## 为什么用 mock-LLM 而不是真模型
//! 三段逻辑都是**确定性**的：给定 contact 旧态 + decision 的 customer_stage，状态机
//! （DEFAULT 销售 profile，由 `ensure_prompt_pack_v2` 种入 active）唯一决定合法/非法。
//! 用 mock-LLM 压固定决策 JSON 即可稳定复现，比真模型 E2E 更可靠、不 flaky——正是
//! `c2_state_transition_cross_domain.rs`「命门用确定性测」哲学向全链路的延伸。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB），由 CI integration job 跑。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::handle_managed_message;
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};

/// 构造一个 managed 状态、带指定初始 `operation_state` 的 Contact。
///
/// precheck_send_gateway 仅放行 Managed 路径；`operation_state` 是本测试的「旧态」，
/// C2 派生写入点据它判定迁移合法性。
fn make_managed_contact(wxid: &str, initial_state: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
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
        tags: Vec::new(),
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: Some(initial_state.to_string()),
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

/// 构造一条入站消息，gateway 触发用。
fn make_inbound(contact: &Contact, message_id: &str, content: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some(message_id.to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: content.to_string(),
        raw: None,
        created_at: DateTime::now(),
    }
}

/// Reply Agent 决策 JSON：参数化 `customer_stage` 与 `operation_state` 两个字段，
/// 其余字段全部填充满足 `validate_and_promote`（R1.3/R1.4/R1.6）的合法值。
///
/// 关键：`customerStage`（typed）经 `normalize_domain_signals` 镜像进 `domain_signals`
/// 容器，是 C2 synced_state 的**优先**来源；`operationState` 是缺失时的回落来源。两者
/// 取不同值即可在断言里区分优先级。risk_level=medium + knowledge_need=not_required +
/// consolidation=false → 非「关键变化轮」也非「低风险常规轮」，仅需 R1.3 字段非空。
fn reply_decision_json(customer_stage: &str, operation_state: &str) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户在评估我方方案的适配度，对话推进顺畅，给出了下一步的明确意向。",
        "relationshipRead": "对话氛围积极，客户对我方专业度有基本信任，关系处于稳步推进阶段。",
        "operationGoal": "顺着客户当前关注点把对话推进到下一步，让客户保持掌控感与确定性。",
        "knowledgeNeedReason": "本轮不涉及具体产品能力承诺，无需检索知识库切片确认覆盖范围。",
        "memoryUpdateReason": "本轮客户表达了新的推进意向，需要记录以支持后续节奏匹配。",
        "selfCritique": "上一轮信息密度略高，本轮收敛节奏，先确认客户优先级再给建议。",
        "whyShouldReply": "客户主动延续了对话并表达推进意向，及时回应能巩固信任并降低决策摩擦。",
        "whySkipReply": "",
        "riskSelfCheck": "本轮回复仅承接节奏、不涉及未验证产品承诺，不触发安全门阈值。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": operation_state,
        "customerStage": customer_stage,
        "shouldReply": true,
        "replyText": "明白你的想法，我们可以先按你的优先级把下一步排一下，你看这样行吗？",
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "客户进入方案评估阶段，按顾问模式承接其关注点。",
    })
}

/// Review Agent 通过 JSON（分数全部 ≥ 阈值，approved，不触发 revision）。
fn review_pass_json() -> serde_json::Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": 8,
            "emotionalValue": 8,
            "productAccuracy": 8,
            "relationshipProgress": 7,
            "conversionReadiness": 6,
            "pressureRisk": 2,
            "factRisk": 1,
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "候选回复仅承接节奏，不涉及具体产品能力承诺。",
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "回复语气自然、未越界做产品承诺，可放行。",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true,
    })
}

/// 查 contact 最新 operation_state。
async fn reload_operation_state(app: &common::TestApp, contact: &Contact) -> Option<String> {
    app.state
        .db
        .contacts()
        .find_one(doc! { "_id": contact.id }, None)
        .await
        .expect("query contact")
        .expect("contact present")
        .operation_state
}

/// 用例 1：**合法迁移 + synced_state 优先级**。
///
/// - 旧态 `new_contact`；
/// - 决策 `customerStage = "relationship_building"`（legal：relationship_building.allowedFrom
///   含 new_contact）；
/// - 决策 `operationState = "need_discovery"`（同样 legal，但**不应**被用——customer_stage 优先）；
/// - 断言：最终 operation_state == `relationship_building`（取 customerStage 派生值，
///   不是 need_discovery）→ 同时覆盖「合法迁移写入」与「synced_state 优先级」两段。
#[tokio::test]
#[ignore]
async fn normal_transition_uses_customer_stage_over_operation_state() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_c2_legal", "new_contact");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(&contact, "msg_c2_legal_001", "我大概了解了，可以再多聊聊你们怎么做的吗？");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    // 知识库为空 → route_operation_knowledge 早返回（0 次 LLM）。
    // LLM 调用序列：#1 Reply Agent 决策；#2 Review Agent（approved，无 revision）。
    app.llm
        .push_response(reply_decision_json("relationship_building", "need_discovery"));
    app.llm.push_response(review_pass_json());

    let before = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    assert_eq!(
        app.llm.calls() - before,
        2,
        "空知识库 happy path：Reply ×1 + Review ×1 = 2 次 LLM 调用"
    );

    // [诊断] 打印落库后的 contact 关键字段，定位 customer_stage 是否进了 domain_attributes
    // 以及 operation_state 实际值——区分「容器丢键」vs「C2 派生块本身回落」。
    {
        let reloaded = app
            .state
            .db
            .contacts()
            .find_one(doc! { "_id": contact.id }, None)
            .await
            .expect("query")
            .expect("contact");
        // [诊断2] gateway 用 load_active_domain_profile 读 active profile 决定 declared_dims；
        // 若它返回的不是 DEFAULT（不声明 customer_stage participates），retain 会剔除
        // customer_stage。这里复刻同一读法，打印 profile_id + 声明维度，定位真因。
        let active = wechatagent::agent::load_active_domain_profile(
            &app.state.db,
            &contact.workspace_id,
        )
        .await;
        let declared: Vec<&str> = active
            .profile_dimensions
            .iter()
            .filter(|d| d.participates_in_decision)
            .map(|d| d.kind.as_str())
            .collect();
        eprintln!(
            "[C2诊断] operation_state={:?} domain_attributes={:?} tags={:?}",
            reloaded.operation_state, reloaded.domain_attributes, reloaded.tags
        );
        eprintln!(
            "[C2诊断2] active_profile_id={:?} transaction_facts={} declared_dims={:?}",
            active.profile_id, active.transaction_facts_enabled, declared
        );
    }

    // 合法迁移写入 + customer_stage 优先：取 relationship_building 而非 need_discovery。
    assert_eq!(
        reload_operation_state(&app, &contact).await.as_deref(),
        Some("relationship_building"),
        "operation_state 应派生自 customerStage（优先级高于 operationState=need_discovery）"
    );

    // 合法迁移写一条 transitioned 事件，且**不**写 rejected 事件（互斥）。
    let rejected = app
        .state
        .db
        .events()
        .count_documents(
            doc! {
                "contact_wxid": &contact.wxid,
                "kind": "agent.operation_state_transition_rejected",
            },
            None,
        )
        .await
        .expect("count rejected events");
    assert_eq!(rejected, 0, "合法迁移不应产生 rejected 审计事件");
}

/// 用例 2：**fail-soft 非法迁移跳写 + 审计事件 + reply 照常下发**。
///
/// - 旧态 `new_contact`；
/// - 决策 `customerStage = "customer_success"`（**非法**：customer_success.allowedFrom =
///   ["commitment_followup", "customer_success"]，不含 new_contact，且非 allowFromAny/initial）；
/// - 决策 `operationState = "need_discovery"`（legal，但 customer_stage 已 present → 不回落，
///   被拒后**不**改用它）；
/// - 断言：① operation_state 保留旧值 new_contact（未被 customer_success 覆盖、也未变 need_discovery）；
///   ② 落一条 `agent.operation_state_transition_rejected` 审计事件（details 字段正确）；
///   ③ reply 仍 approved + outbox 入队（fail-soft 不阻断）。
#[tokio::test]
#[ignore]
async fn illegal_transition_keeps_old_state_and_audits_failsoft() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_c2_illegal", "new_contact");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(&contact, "msg_c2_illegal_001", "你好，想先简单了解一下。");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    // 同样 2 次 LLM 调用；customerStage 取一个从 new_contact 非法的终态 customer_success。
    app.llm
        .push_response(reply_decision_json("customer_success", "need_discovery"));
    app.llm.push_response(review_pass_json());

    let before = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok（fail-soft：非法迁移不返回 Err）");
    assert_eq!(app.llm.calls() - before, 2, "Reply ×1 + Review ×1 = 2 次 LLM 调用");

    // ① fail-soft 跳写：保留旧态 new_contact（既没被非法的 customer_success 覆盖，
    //    也没回落到 operationState=need_discovery——customer_stage present 时不回落）。
    assert_eq!(
        reload_operation_state(&app, &contact).await.as_deref(),
        Some("new_contact"),
        "非法迁移应跳写 operation_state、保留旧值 new_contact"
    );

    // ② 审计事件：恰好一条 rejected，details 含 prior/attempted/reason。
    let event = app
        .state
        .db
        .events()
        .find_one(
            doc! {
                "contact_wxid": &contact.wxid,
                "kind": "agent.operation_state_transition_rejected",
            },
            None,
        )
        .await
        .expect("query rejected event")
        .expect("非法迁移必须落一条 agent.operation_state_transition_rejected 审计事件");
    assert_eq!(event.status, "rejected", "审计事件 status 应为 rejected");
    let details = event.details.expect("rejected 事件应带 details");
    assert_eq!(
        details.get_str("prior_state").ok(),
        Some("new_contact"),
        "details.prior_state 应为旧态"
    );
    assert_eq!(
        details.get_str("attempted_state").ok(),
        Some("customer_success"),
        "details.attempted_state 应为被拒的派生值"
    );
    assert!(
        details
            .get_str("reason")
            .map(|r| r.contains("state_transition_invalid"))
            .unwrap_or(false),
        "details.reason 应含 state_transition_invalid，实际 {:?}",
        details.get_str("reason")
    );

    // ③ reply 照常下发（fail-soft 不阻断）：终态 approved，且 outbox 入队一行。
    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("query agent_run_logs")
        .expect("agent_run_logs row exists");
    assert!(
        matches!(
            log.final_review_status.as_str(),
            "approved" | "revision_applied_approved"
        ),
        "非法迁移属 fail-soft：reply 仍应放行（approved 类终态），实际 {:?}",
        log.final_review_status
    );
    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query outbox by run_id")
        .expect("fail-soft 路径 reply 仍应入队 outbox 一行（证明未被阻断）");
    assert_eq!(
        outbox.contact_wxid, contact.wxid,
        "outbox.contact_wxid 不一致：{:?}",
        outbox
    );
}

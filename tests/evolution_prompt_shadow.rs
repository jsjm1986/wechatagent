//! 集成测试：prompt 候选的 shadow replay 端到端（经 `evolution::replay::run_shadow_replay`
//! 接 `agent::prompt_shadow::shadow_replay_prompt_one`）。
//!
//! 覆盖三条路径，全部用 mock LLM 把 decide/review 判定确定化（真模型语义另由
//! nightly real-llm 套件覆盖）：
//!   1. **completed**：种好 prompt proposal（proposed_template_key=user.reply.policy
//!      + 合法 diff_snippet）+ 源 AgentRunLog（含 review.scores + selfCritiqueAddressed
//!      + 顶层 source_event_id = 真实 message.message_id）+ 对应 inbound message +
//!      managed contact；mock 让 decide_reply + review_decision 各返回一条确定结果 →
//!      断言写出的 ShadowReplay status=="completed"，original/new 两侧 5 闸命中向量都被填。
//!   2. **source_message_unavailable**：源 run 的 source_event_id 指向一条不存在的
//!      message → run_shadow_replay 的 retention 探针拦下，status=="failed"、
//!      failure_reason=="source_message_unavailable"。该用例同时回归 replay.rs
//!      retention 探针字段名（必须用 snake_case `message_id`，与 ConversationMessage
//!      落库一致——否则探针对真实消息恒 count==0、把所有 prompt shadow 错杀）。
//!   3. **contact_unavailable**：message 在（探针过）但 run 的 contact_wxid 查不到
//!      contact → shadow_replay_prompt_one 返回 failed("contact_unavailable")。
//!
//! setup 走 mongo 事务无关路径，但与阶段一 redline 集成测试统一用
//! `TestApp::start_repl_set()`（prompt pack v2 + 销售域字典已 seed）。
//! 全部 `#[ignore]`：需 Docker（testcontainers MongoDB），本地不跑、CI `--ignored` 跑。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::models::{
    AgentRunLog, AgentStatus, Contact, ConversationMessage, MessageDirection, Proposal,
};
use wechatagent::routes::AppState;

/// 被 shadow 注入追加片段的目标 prompt key（强约束层，已 seed）。
const TARGET_KEY: &str = "user.reply.policy";

/// 构造一个 managed contact（workspace/account 用测试默认 "default"）。
fn make_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("测试客户".to_string()),
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
        operation_state: Some("need_discovery".to_string()),
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

/// 构造一条 inbound ConversationMessage，message_id 即源 run context 引用的那条。
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
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    }
}

/// 构造源 AgentRunLog：带 `review.scores`（G4 原始基线）+ `selfCritiqueAddressed`
/// + 顶层 `source_event_id`（= R0 envelope 的 message.message_id；shadow 据此反查
/// 真实历史消息）。生产 gateway 从不往 `context` 写 inboundMessageId——shadow /
/// replay retention 探针都读顶层 `source_event_id`，这里还原该真实形状。
/// `contact_wxid` 决定 contact 反查命中与否。字段集与 `src/evolution/replay.rs`
/// 单测 `mk_run_log` 对齐。
fn make_run_log(contact_wxid: &str, inbound_message_id: &str) -> AgentRunLog {
    let scores = doc! {
        "humanLike": 8_i32,
        "emotionalValue": 7_i32,
        "factRisk": 1_i32,
        "pressureRisk": 2_i32,
        "productAccuracy": 9_i32,
    };
    AgentRunLog {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: Some(contact_wxid.to_string()),
        run_id: format!("run_{}", ObjectId::new().to_hex()),
        trigger_kind: "inbound_message".to_string(),
        status: "completed".to_string(),
        planner: Document::new(),
        // 生产形状：gateway 不往 context 写 inboundMessageId。源 inbound id 走
        // 顶层 source_event_id（见下方）。
        context: Document::new(),
        knowledge_route: Document::new(),
        decision: Document::new(),
        review: doc! { "scores": scores, "selfCritiqueAddressed": true },
        gateway_result: Document::new(),
        error: None,
        token_budget: 0,
        tokens_used: 0,
        llm_calls_used: 0,
        degraded_reasons: vec![],
        lifecycle: "completed".to_string(),
        // shadow / replay retention 探针据此反查真实历史消息（snake_case message_id）。
        source_event_id: inbound_message_id.to_string(),
        source_kind: "inbound_message".to_string(),
        error_summary: None,
        abort_reason: None,
        revision_applied: false,
        revision_reason: String::new(),
        pre_revision_summary: None,
        post_revision_summary: None,
        self_critique: None,
        autonomy_mode: "auto".to_string(),
        final_review_status: "approved".to_string(),
        outbox_status: None,
        memory_consolidator_warnings: vec![],
        conversation_mode: String::new(),
        conversation_mode_reason: None,
        created_at: DateTime::now(),
    }
}

/// 构造一条 `proposal_kind="prompt"` 的候选（in-memory；run_shadow_replay 按
/// `&Proposal` 取，不读 DB）。`id` 必须 Some——persist_replay 据它写 shadow_replays。
fn make_prompt_proposal(key: &str, diff_snippet: &str) -> Proposal {
    let now = DateTime::now();
    Proposal {
        id: Some(ObjectId::new()),
        experiment_id: "test-exp".to_string(),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        proposal_kind: "prompt".to_string(),
        status: "pending_eval".to_string(),
        gate_key: None,
        current_value: None,
        proposed_value: None,
        cohort_notes: Document::new(),
        proposed_template_key: Some(key.to_string()),
        proposed_section: Some("policy".to_string()),
        diff_summary: None,
        diff_snippet: Some(diff_snippet.to_string()),
        critic_reasoning: None,
        expected_improvement_on: vec![],
        risk_note: None,
        previous_prompt_version: None,
        eval_metrics: Document::new(),
        eval_replays_completed: 0,
        eval_replays_failed: 0,
        significance_passed: None,
        failure_reason: None,
        released_at: None,
        released_by: None,
        rolled_back_at: None,
        rolled_back_by: None,
        created_at: now,
        updated_at: now,
    }
}

/// Reply Agent 决策 JSON（shouldReply=true，分数全过；mock 让 decide_reply 确定返回）。
/// 取自 happy_path_run.rs 的合法决策骨架——validate_and_promote 严格枚举 / 必填校验全过。
fn reply_decision_json() -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户在评估我方在企业 IM 场景下的方案适配度，并给出落地预算与时间。",
        "relationshipRead": "对话氛围积极，对我方专业度信任，关系处于稳步推进期。",
        "operationGoal": "帮客户厘清下一步排期与成本边界，让客户在不被推销压力下感到掌控感。",
        "knowledgeNeedReason": "客户提及具体场景与预算，需要结合产品能力确认覆盖范围与承诺边界。",
        "memoryUpdateReason": "本轮新增预算与时间锚点信息，需写入长期记忆支持后续节奏匹配。",
        "selfCritique": "上一轮过早提价格，本次收敛信息密度并先确认客户优先级再给建议。",
        "whyShouldReply": "客户主动询问实施周期与预算，回复能确认需求颗粒度并降低决策摩擦。",
        "whySkipReply": "",
        "riskSelfCheck": "本轮回复不涉及未验证的产品能力承诺，仅给节奏与下一步动作建议。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "need_discovery",
        "shouldReply": true,
        "replyText": "理解你们在做横向对比。我们一般 2~4 周可上线，要不要先按你们的优先级排排序？",
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "客户进入方案评估阶段，按顾问模式处理产品与排期问题。",
    })
}

/// Review Agent 通过 JSON（分数全部 ≥ 阈值，无改写）。
fn review_pass_json() -> serde_json::Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": 8,
            "emotionalValue": 8,
            "productAccuracy": 8,
            "boundaryPrivacySafety": 9,
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
        "reviewSummary": "回复人味自然、无越界承诺，可放行。",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true,
    })
}

/// 读出本 proposal 写下的唯一一条 shadow_replays 行。
async fn fetch_replay(state: &AppState, proposal_id: ObjectId) -> wechatagent::models::ShadowReplay {
    state
        .db
        .shadow_replays()
        .find_one(doc! { "proposal_id": proposal_id }, None)
        .await
        .expect("query shadow_replays")
        .expect("shadow_replays row should exist after run_shadow_replay")
}

/// completed：齐全前置 + mock decide/review → ShadowReplay completed，原/新 5 闸都填。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn run_shadow_replay_prompt_completed_fills_both_sides() {
    let app = common::TestApp::start_repl_set().await;

    let contact = make_contact("user_prompt_shadow_ok");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let inbound = make_inbound(
        &contact,
        "msg_prompt_shadow_ok_001",
        "你们的实施周期一般多久？大概预算需要多少？",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    let run = make_run_log(&contact.wxid, "msg_prompt_shadow_ok_001");
    let source_run_id = run.id.expect("run id present");
    app.state
        .db
        .agent_run_logs()
        .insert_one(&run, None)
        .await
        .expect("insert source run log");

    let proposal = make_prompt_proposal(TARGET_KEY, "补充：本行业语气更稳重，先确认优先级再给建议。");
    let proposal_id = proposal.id.expect("proposal id present");

    // shadow 内部：知识库为空 → 路由 0 次 LLM；decide_reply 1 次 + review_decision 1 次。
    app.llm.push_response(reply_decision_json());
    app.llm.push_response(review_pass_json());

    let before_calls = app.llm.calls();
    wechatagent::evolution::replay::run_shadow_replay(&app.state, &proposal, source_run_id)
        .await
        .expect("run_shadow_replay ok");
    let after_calls = app.llm.calls();
    assert_eq!(
        after_calls - before_calls,
        2,
        "completed 路径应恰好调用 2 次 LLM（decide ×1 + review ×1）"
    );

    let replay = fetch_replay(&app.state, proposal_id).await;
    assert_eq!(replay.status, "completed", "shadow_replay 应 completed，实际 {:?}", replay);
    assert!(replay.failure_reason.is_none(), "completed 不应带 failure_reason");
    assert!(
        !replay.original_5gate_hit.is_empty(),
        "G4：源 run review.scores 应推回非空的 original_5gate_hit，实际 {:?}",
        replay.original_5gate_hit
    );
    assert!(
        !replay.new_5gate_hit.is_empty(),
        "新侧 review.scores 应推回非空的 new_5gate_hit，实际 {:?}",
        replay.new_5gate_hit
    );
    assert!(
        replay.original_self_critique_addressed.is_some(),
        "原始侧 selfCritiqueAddressed 应从源 run 读出"
    );
    assert!(
        replay.new_self_critique_addressed.is_some(),
        "新侧 selfCritiqueAddressed 应从 review 读出"
    );
    assert_eq!(replay.source_run_id, source_run_id, "source_run_id 透传一致");
}

/// source_message_unavailable：source_event_id 指向不存在的 message。
/// 同时回归 replay.rs retention 探针字段名（snake_case message_id）：若探针误用
/// camelCase，则**真实**消息也恒 count==0、completed 路径会被错杀——这里反向锁定
/// 「不存在 → failed」「存在 → completed（见上一个用例）」两侧一致。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn run_shadow_replay_prompt_failed_when_message_missing() {
    let app = common::TestApp::start_repl_set().await;

    let contact = make_contact("user_prompt_shadow_nomsg");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 故意不插入 inbound message：source_event_id 引用 msg_ghost_404，messages 集合里没有。
    let run = make_run_log(&contact.wxid, "msg_ghost_404");
    let source_run_id = run.id.expect("run id present");
    app.state
        .db
        .agent_run_logs()
        .insert_one(&run, None)
        .await
        .expect("insert source run log");

    let proposal = make_prompt_proposal(TARGET_KEY, "补充：本行业语气更稳重。");
    let proposal_id = proposal.id.expect("proposal id present");

    let before_calls = app.llm.calls();
    wechatagent::evolution::replay::run_shadow_replay(&app.state, &proposal, source_run_id)
        .await
        .expect("run_shadow_replay ok（failed 是业务结果而非 Err）");
    let after_calls = app.llm.calls();
    assert_eq!(
        after_calls - before_calls,
        0,
        "message 缺失应在 retention 探针即短路，不触达 LLM"
    );

    let replay = fetch_replay(&app.state, proposal_id).await;
    assert_eq!(replay.status, "failed", "message 缺失应 failed，实际 {:?}", replay);
    assert_eq!(
        replay.failure_reason.as_deref(),
        Some("source_message_unavailable"),
        "failure_reason 必须是 source_message_unavailable"
    );
}

/// contact_unavailable：message 在（探针过）但 run 的 contact_wxid 查不到 contact →
/// shadow_replay_prompt_one 返回 failed("contact_unavailable")。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn run_shadow_replay_prompt_failed_when_contact_missing() {
    let app = common::TestApp::start_repl_set().await;

    // message 存在（让 run_shadow_replay 的 retention 探针通过），但其 contact_wxid
    // 指向一个从未插入 contacts 的 wxid。
    let ghost_contact = make_contact("user_prompt_shadow_ghost");
    let inbound = make_inbound(
        &ghost_contact,
        "msg_prompt_shadow_ghost_001",
        "随便一句历史消息。",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    let run = make_run_log("user_prompt_shadow_ghost", "msg_prompt_shadow_ghost_001");
    let source_run_id = run.id.expect("run id present");
    app.state
        .db
        .agent_run_logs()
        .insert_one(&run, None)
        .await
        .expect("insert source run log");

    let proposal = make_prompt_proposal(TARGET_KEY, "补充：本行业语气更稳重。");
    let proposal_id = proposal.id.expect("proposal id present");

    wechatagent::evolution::replay::run_shadow_replay(&app.state, &proposal, source_run_id)
        .await
        .expect("run_shadow_replay ok（failed 是业务结果而非 Err）");

    let replay = fetch_replay(&app.state, proposal_id).await;
    assert_eq!(replay.status, "failed", "contact 缺失应 failed，实际 {:?}", replay);
    assert_eq!(
        replay.failure_reason.as_deref(),
        Some("contact_unavailable"),
        "failure_reason 必须是 contact_unavailable"
    );
}

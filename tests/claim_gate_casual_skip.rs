//! S5-6 寒暄低风险轮跳过独立 ClaimGate：conversation_mode=casual_relationship ∧
//! planner 低风险 ∧ knowledge_need=not_required ∧ 零知识引用 ∧ 零发送物 ∧ 无请示的
//! 轮次，Review 本体照跑、只跳过语义声明抽取器（每轮省一次 LLM 调用），并在
//! review.risks 落 `claim_gate_skipped_casual_low_risk` 审计标记；任一条件不满足
//! （对照：risk=medium）时 ClaimGate 照跑、不落标记。
//!
//! 断言手法：mock LLM 队列不排 ClaimGate 响应——若误未跳过，ClaimGate 调用会因
//! 无匹配 schema 响应而失败并 fail-closed 成 blocked_by_safety_guard，"终态
//! approved + LLM 调用数恰好 2" 即同时证明零调用与链路完好。
//!
//! 默认 #[ignore]，需 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::handle_managed_message;
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};

fn make_managed_contact(wxid: &str) -> Contact {
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

/// 寒暄轮 Reply Agent decision：casual_relationship + 指定 riskLevel +
/// not_required + 零知识引用 / 发送物 / 请示。
fn casual_reply_decision_json(reply_text: &str, risk_level: &str) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户在闲聊周末安排，没有任何业务议题，只是维系日常熟悉度的寒暄互动而已。",
        "relationshipRead": "关系轻松自然，客户主动分享生活近况，说明信任在稳步累积，适合顺着话题自然回应。",
        "operationGoal": "保持自然的朋友式互动节奏，不引入任何业务话题，让客户感到轻松无压力的日常陪伴。",
        "knowledgeNeedReason": "纯寒暄轮不涉及产品能力、价格或方案等业务事实，无需引用任何运营知识库切片支撑。",
        "memoryUpdateReason": "客户提到周末去爬山的生活偏好，值得记入长期画像以便后续找共同话题自然破冰。",
        "selfCritique": "注意不要在寒暄轮硬转业务话题，保持回应简短自然，避免信息密度过高显得刻意。",
        "whyShouldReply": "客户主动分享周末安排，及时自然地回应能维系关系温度，这是低成本的关系维护时机。",
        "whySkipReply": "",
        "riskSelfCheck": "纯寒暄回应不含任何产品声明、价格承诺或业务事实，无安全门风险点，可以自然发送。",
        "riskLevel": risk_level,
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": false,
        "consolidationNeeded": false,
        "operationState": "need_discovery",
        "shouldReply": true,
        "replyText": reply_text,
        "usedKnowledgeIds": [],
        "matchedKnowledgeIds": [],
        "conversationMode": "casual_relationship",
        "conversationModeReason": "客户在闲聊周末安排，按寒暄关系模式轻松回应，不引入业务话题。",
    })
}

fn review_pass_json() -> serde_json::Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": 9,
            "emotionalValue": 8,
            "productAccuracy": 8,
            "relationshipProgress": 7,
            "conversionReadiness": 5,
            "pressureRisk": 1,
            "boundaryPrivacySafety": 9,
            "factRisk": 0,
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "纯寒暄回应，不涉及任何产品能力或业务事实承诺。",
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "寒暄轮回应自然、无业务声明，可以放行。",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true,
    })
}

/// 寒暄 + 低风险 + 零知识引用：独立 ClaimGate 零调用（队列不排 ClaimGate 响应仍
/// 终态 approved），LLM 只调 Reply + Review 两次，review.risks 落跳过审计标记。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn casual_low_risk_turn_skips_claim_gate_llm_call() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_casual_skip");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(&contact, "msg_casual_001", "哈哈周末去爬山啦，你呢？");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    // 只排 Reply + Review 两条响应；ClaimGate 若被误调用会因无匹配 schema 响应
    // 失败并 fail-closed（blocked_by_safety_guard），下方 approved 断言即会失败。
    app.llm.push_response(casual_reply_decision_json(
        "哈哈爬山好啊！这个天气正合适，我周末就宅着追了个剧～你们去的哪座山？",
        "low",
    ));
    app.llm.push_response(review_pass_json());

    let before_calls = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    let after_calls = app.llm.calls();
    assert_eq!(
        after_calls - before_calls,
        2,
        "寒暄低风险轮应只调 Reply + Review 两次 LLM（ClaimGate 被跳过）"
    );

    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("query agent_run_logs")
        .expect("agent_run_logs row exists");
    assert_eq!(
        log.final_review_status, "approved",
        "跳过 ClaimGate 的寒暄轮终态必须 approved（不得误触发 hold_for_claim_gate_failure），实际 {:?}",
        log.final_review_status
    );

    let review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query decision_reviews")
        .expect("decision review row exists");
    assert!(
        review
            .risks
            .iter()
            .any(|risk| risk == "claim_gate_skipped_casual_low_risk"),
        "跳过必须落审计标记 claim_gate_skipped_casual_low_risk，实际 risks={:?}",
        review.risks
    );

    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query outbox by run_id")
        .expect("approved 寒暄轮必须正常入队 outbox");
    assert_eq!(outbox.contact_wxid, contact.wxid);
}

/// 对照：同为寒暄文本但 riskLevel=medium（跳过条件不满足）→ ClaimGate 照跑
/// （消费第三条 mock 响应），不落跳过标记。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn casual_medium_risk_turn_still_runs_claim_gate() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_casual_medium");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(&contact, "msg_casual_002", "哈哈周末去爬山啦，你呢？");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    app.llm.push_response(casual_reply_decision_json(
        "哈哈爬山好啊！这个天气正合适，我周末就宅着追了个剧～你们去的哪座山？",
        "medium",
    ));
    app.llm.push_response(review_pass_json());
    app.llm
        .push_response(common::independent_claim_gate_pass_json());

    let before_calls = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    let after_calls = app.llm.calls();
    assert_eq!(
        after_calls - before_calls,
        3,
        "risk=medium 的寒暄轮 ClaimGate 必须照跑：Reply + Review + ClaimGate = 3 次"
    );

    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("query agent_run_logs")
        .expect("agent_run_logs row exists");
    assert_eq!(log.final_review_status, "approved");

    let review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query decision_reviews")
        .expect("decision review row exists");
    assert!(
        !review
            .risks
            .iter()
            .any(|risk| risk == "claim_gate_skipped_casual_low_risk"),
        "未跳过的轮次不得落跳过标记，实际 risks={:?}",
        review.risks
    );
}

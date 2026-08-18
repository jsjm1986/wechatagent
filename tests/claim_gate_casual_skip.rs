//! 低风险正文可由独立 Light Reviewer 同轮给出完整原子 claim 判断；其它正文仍必须经过
//! 专用 ClaimGate。
//!
//! 回归重点不是匹配某句寒暄，而是锁定两个链路不变量：
//! 1. 完整且确认无 evidence claim 的 Light Reviewer 结果可省去专用 ClaimGate；
//! 2. 合同缺失、业务事实或 Reply 自报 `claims=[] / low risk` 都不能绕过独立核验。
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
        "whyShouldReply": "客户主动分享周末近况，顺着生活话题自然回应并保持轻松交流。",
        "whySkipReply": "",
        "intentAnalysis": {
            "semanticAssessment": {
                "intent": "维持轻松的日常互动",
                "speechAct": "question",
                "subject": "customer",
                "assertionStatus": "interrogative",
                "knowledgeNeed": "not_required",
                "responseDisposition": "reply",
                "semanticRisk": {
                    "content": "low",
                    "pressure": "low",
                    "boundary": "low",
                    "privacy": "low",
                    "confidence": 0.98
                },
                "claims": [],
                "reason": "完整语境表明这是生活寒暄和轻量提问，不代表任何业务事实。"
            }
        },
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

fn review_pass_with_embedded_atomic_json(
    source_quote: &str,
    evidence_need: &str,
    product_claim: bool,
) -> serde_json::Value {
    let mut value = review_pass_json();
    value["claimAnalysis"]["atomicClaimGate"] = json!({
        "claimKinds": ["complete_candidate_unit"],
        "claimsComplete": true,
        "claims": [{
            "sourceQuote": source_quote,
            "claim": "独立审核候选回复的完整原子语义",
            "scope": "open_semantic_scope",
            "subject": if evidence_need == "required" { "business" } else { "general" },
            "actionKind": null,
            "evidenceNeed": evidence_need,
            "negativePolarity": false,
            "productClaim": product_claim,
            "evidenceRefs": [],
            "reason": if evidence_need == "required" {
                "候选代表机构确认了可现实核验的业务事实。"
            } else {
                "候选只完成当前会话中的轻量互动，不结算外部业务事实。"
            }
        }],
        "catalogCoverageComplete": true,
        "catalogClaims": [],
        "reason": "已完整检查候选回复的原子语义。"
    });
    value
}

/// A model may still populate the legacy boolean incorrectly while providing the newer semantic
/// contract correctly. The server must reconcile that contradiction from the typed AI field,
/// without inspecting the candidate text or maintaining a phrase allowlist.
fn contradictory_legacy_claim_gate_json() -> serde_json::Value {
    json!({
        // Legacy aggregate mirrors are deliberately wrong. The parser must ignore them and derive
        // the gate only from claims[].evidenceNeed.
        "requiresEvidence": true,
        "claimKinds": ["conversational_performative"],
        "claimsComplete": true,
        "semanticAssessment": {
            "speechAct": "model_specific_acknowledgement",
            "subject": "none",
            "assertionStatus": "performative_completed_by_reply",
            "knowledgeNeed": "not_required",
            "responseDisposition": "acknowledgement",
            "contentRisk": "low",
            "confidence": 0.96,
            "reason": "当前会话中的确认行为，不是外部业务事实。"
        },
        "responseDisposition": "model_specific_reply_mode",
        "claims": [{
            "sourceQuote": "收到",
            "claim": "确认已收到当前消息",
            "scope": "conversation_acknowledgement",
            "subject": "general",
            "speechAct": "acknowledgement",
            "assertionStatus": "performative_completed_by_reply",
            "evidenceNeed": "not_needed",
            "negativePolarity": false,
            "confidence": 0.96,
            "productClaim": false,
            // Deliberately inconsistent legacy field: evidenceNeed is the semantic authority.
            "requiresEvidence": true,
            "evidenceRefs": [],
            "reason": "会话确认由回复本身完成，不代表外部现实状态。"
        }],
        "hasCatalogClaims": false,
        "catalogCoverageComplete": true,
        "hasNonCatalogEvidenceClaims": true,
        "catalogClaims": [],
        "reason": "没有需要外部证据的现实业务断言。"
    })
}

fn missing_semantic_authority_claim_gate_json() -> serde_json::Value {
    let mut value = contradictory_legacy_claim_gate_json();
    value["claims"][0]
        .as_object_mut()
        .expect("claim object")
        .remove("evidenceNeed");
    value
}

/// 低风险寒暄由独立 Light Reviewer 同轮完成原子 claim 判断，无需再串一个专用
/// ClaimGate；Reply 自报不参与该判定。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn casual_low_risk_turn_uses_embedded_light_claim_gate() {
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

    let reply = "哈哈爬山好啊！这个天气正合适，我周末就宅着追了个剧～你们去的哪座山？";
    app.llm
        .push_response(casual_reply_decision_json(reply, "low"));
    app.llm.push_response(review_pass_with_embedded_atomic_json(
        reply,
        "not_needed",
        false,
    ));

    let before_calls = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    let after_calls = app.llm.calls();
    assert_eq!(
        after_calls - before_calls,
        2,
        "安全 Lean 正文应只执行 Reply + 独立 Light Reviewer 两次 LLM 调用"
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
        "嵌入式原子判断通过后寒暄轮终态应 approved，实际 {:?}",
        log.final_review_status
    );

    let claim_analysis = log
        .review
        .get_document("claimAnalysis")
        .expect("run log persists merged claim analysis");
    assert_eq!(
        claim_analysis.get_str("claimGateMode").unwrap_or_default(),
        "embedded_light"
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

/// The semantic contract must repair a contradictory legacy boolean instead of turning a
/// conversational performative into an unsupported-business safety hold.
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn semantic_claim_contract_overrides_contradictory_legacy_evidence_flag() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_semantic_contract_repair");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(&contact, "msg_semantic_contract_001", "收到");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    app.llm
        .push_response(casual_reply_decision_json("收到", "low"));
    app.llm.push_response(review_pass_with_embedded_atomic_json(
        "可以，我们门店明天下午三点一定有空位，你直接过来就行。",
        "required",
        false,
    ));
    app.llm
        .push_response(contradictory_legacy_claim_gate_json());

    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");

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
        "semantic evidenceNeed=not_needed must prevent a false safety hold"
    );

    let claim_analysis = log
        .review
        .get_document("claimAnalysis")
        .expect("run log persists merged claim analysis");
    assert_eq!(
        claim_analysis
            .get_bool("requiresBusinessEvidence")
            .unwrap_or(true),
        false
    );
    assert_eq!(
        claim_analysis
            .get_i64("unsupportedBusinessClaimCount")
            .unwrap_or(-1),
        0
    );

    app.state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query outbox by run_id")
        .expect("semantically safe acknowledgement must be enqueued");
}

/// Missing semantic authority is a malformed AI contract, not permission to trust a legacy
/// boolean. The gate gets one bounded semantic re-evaluation and then applies the corrected result.
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn missing_evidence_need_retries_once_without_text_fallback() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_semantic_contract_retry");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(&contact, "msg_semantic_contract_retry_001", "明白");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    app.llm
        .push_response(casual_reply_decision_json("收到", "low"));
    app.llm.push_response(review_pass_json());
    app.llm
        .push_response(missing_semantic_authority_claim_gate_json());
    app.llm
        .push_response(contradictory_legacy_claim_gate_json());

    let before_calls = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    assert_eq!(
        app.llm.calls() - before_calls,
        4,
        "Reply + Reviewer + invalid ClaimGate + one semantic contract retry"
    );

    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(
            doc! { "run_id": { "$exists": true }, "contact_wxid": &contact.wxid },
            None,
        )
        .await
        .expect("query agent_run_logs")
        .expect("agent_run_logs row exists");
    assert_eq!(log.final_review_status, "approved");
    let claim_analysis = log
        .review
        .get_document("claimAnalysis")
        .expect("run log persists merged claim analysis");
    assert_eq!(claim_analysis.get_i32("semanticContractVersion"), Ok(2));
    assert_eq!(
        claim_analysis.get_bool("requiresBusinessEvidence"),
        Ok(false)
    );
}

/// Reply Agent 与 Reviewer 可能同时漏判。即便 Reply Agent 把事实性正文伪装成
/// casual + low + claims=[]，缺失独立 ClaimGate 结果也必须 fail-closed，原正文不能获批。
#[tokio::test]
#[ignore = "requires MongoDB"]
async fn self_reported_empty_claims_cannot_bypass_independent_gate() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_false_low_risk_claims");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "msg_false_low_001",
        "那我明天下午直接过去可以吗？",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    app.llm.push_response(casual_reply_decision_json(
        "可以，我们门店明天下午三点一定有空位，你直接过来就行。",
        "low",
    ));
    app.llm.push_response(review_pass_json());
    // 故意不排 ClaimGate 响应。独立闸必须真实调用并因上游结果缺失而 fail-closed；
    // 若代码错误地相信 Reply Agent 自报的 claims=[]，本轮只会调用两次并错误获批。

    let before_calls = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    let after_calls = app.llm.calls();
    assert!(
        after_calls - before_calls >= 3,
        "自报低风险不能跳过独立 ClaimGate；失败后的客户保障回复允许产生额外调用"
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
        log.final_review_status, "blocked_by_safety_guard",
        "独立 ClaimGate 无有效结果时必须 fail-closed"
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
            .any(|risk| risk == "independent_claim_gate_unavailable"),
        "ClaimGate 失败必须留下可审计风险，实际 risks={:?}",
        review.risks
    );
}

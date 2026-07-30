//! SR-072 runtime redline: once a workspace has a current state machine, a
//! missing current state policy must stop every customer-send path before an
//! Outbox row or MCP call can be created.

#![cfg(test)]

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{handle_managed_message, send_contact_message_gateway, ManualContactSend};
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("SR-072 customer".to_string()),
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
        operation_state_confidence: Some(8),
        operation_state_updated_at: Some(now),
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

fn inbound(contact: &Contact, message_id: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some(message_id.to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "我们先梳理一下最优先的场景吧。".to_string(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    }
}

fn reply_decision() -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户希望先明确优先场景。",
        "relationshipRead": "关系稳定，适合继续澄清需求。",
        "operationGoal": "确认优先场景并保持克制。",
        "knowledgeNeedReason": "本轮不需要产品事实。",
        "memoryUpdateReason": "没有新增长期事实。",
        "selfCritique": "只问一个问题，不做承诺。",
        "whyShouldReply": "客户提出了明确问题，应当承接。",
        "whySkipReply": "",
        "riskSelfCheck": "不包含产品、价格或效果声明。",
        "riskLevel": "low",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "need_discovery",
        "shouldReply": true,
        "replyText": "可以，我们先确认最优先的场景，你最想先解决哪一块？",
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "用单一问题澄清需求。"
    })
}

fn review_pass() -> serde_json::Value {
    json!({
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
            "reason": "No product claim."
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "Safe and concise.",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true
    })
}

async fn assert_no_delivery_side_effects(app: &common::TestApp) {
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(Document::new(), None)
            .await
            .expect("count outbox"),
        0,
        "missing policy must stop before Outbox enqueue"
    );
    assert_eq!(
        app.state
            .db
            .raw()
            .collection::<Document>("mcp_call_logs")
            .count_documents(Document::new(), None)
            .await
            .expect("count MCP logs"),
        0,
        "missing policy must stop before MCP"
    );
}

#[test]
#[ignore = "requires MongoDB / testcontainers"]
fn current_machine_missing_policy_blocks_reply_and_management_send() {
    std::thread::Builder::new()
        .name("sr072-policy-redline".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build SR-072 test runtime")
                .block_on(current_machine_missing_policy_blocks_reply_and_management_send_inner())
        })
        .expect("spawn SR-072 test thread")
        .join()
        .expect("SR-072 test thread panicked");
}

async fn current_machine_missing_policy_blocks_reply_and_management_send_inner() {
    eprintln!("SR072_STAGE=test-start");
    let app = common::TestApp::start().await;
    eprintln!("SR072_STAGE=test-app-ready");
    let current_machine = app
        .state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": "default",
                "domain": "user_operations",
                "current_version": true,
            },
            None,
        )
        .await
        .expect("load current machine")
        .expect("TestApp must seed a current machine");
    assert!(current_machine
        .state_machine
        .get_array("states")
        .expect("machine states")
        .iter()
        .filter_map(|value| value.as_document())
        .any(|state| state.get_str("key").ok() == Some("need_discovery")));
    assert!(
        app.state
            .db
            .operation_state_policies()
            .find_one(
                doc! {
                    "workspace_id": "default",
                    "domain": "user_operations",
                    "state_key": "need_discovery",
                    "current_version": true,
                    "status": "active",
                },
                None,
            )
            .await
            .expect("load default policy")
            .is_some(),
        "fresh prompt-pack bootstrap must reconcile state policies"
    );
    eprintln!("SR072_STAGE=bootstrap-policy-proved");

    app.state
        .db
        .operation_state_policies()
        .delete_many(
            doc! {
                "workspace_id": "default",
                "domain": "user_operations",
                "state_key": "need_discovery",
            },
            None,
        )
        .await
        .expect("remove policy to model a reconcile failure");

    let reply_contact = managed_contact("sr072-reply");
    let reply_inbound = inbound(&reply_contact, "sr072-inbound");
    app.state
        .db
        .contacts()
        .insert_one(&reply_contact, None)
        .await
        .expect("insert reply contact");
    app.state
        .db
        .messages()
        .insert_one(&reply_inbound, None)
        .await
        .expect("insert inbound");
    app.llm.push_response(reply_decision());
    app.llm.push_response(review_pass());
    app.llm
        .push_response(common::independent_claim_gate_pass_json());
    let reply_error = handle_managed_message(&app.state, reply_contact, &reply_inbound)
        .await
        .expect_err("Reply path must fail closed when the policy is missing");
    assert!(reply_error
        .to_string()
        .contains("missing_current_operation_state_policy"));
    assert_no_delivery_side_effects(&app).await;
    eprintln!("SR072_STAGE=reply-fail-closed-proved");

    let management_contact = managed_contact("sr072-management");
    app.state
        .db
        .contacts()
        .insert_one(&management_contact, None)
        .await
        .expect("insert management contact");
    app.llm.push_response(review_pass());
    app.llm
        .push_response(common::independent_claim_gate_pass_json());
    let management_error = send_contact_message_gateway(
        &app.state,
        management_contact,
        ManualContactSend {
            content: "可以，我们先确认最优先的场景。".to_string(),
            source: doc! { "test": "sr072" },
            original_content_locked: true,
        },
    )
    .await
    .expect_err("Management send must use the same state-action gate");
    assert!(management_error
        .to_string()
        .contains("missing_current_operation_state_policy"));
    assert_no_delivery_side_effects(&app).await;
    eprintln!("SR072_STAGE=management-fail-closed-proved");

    app.cleanup().await;
    eprintln!("SR072_STAGE=test-complete");
}

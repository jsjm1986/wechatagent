//! Atomic production Harness commit integration tests.
//!
//! These tests exercise the public Gateway APIs against a replica-set MongoDB. They assert
//! durable state and transaction boundaries rather than model wording.

mod common;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime, Document};
use serde_json::{json, Value};
use wechatagent::agent::{
    enqueue, handle_follow_up_task_with_claim, handle_managed_message, EnqueueOutcome,
    EnqueueRequest,
};
use wechatagent::models::{
    AgentStatus, AgentTask, AskHumanPolicy, Contact, ConversationMessage, DeciderRef,
    MessageDirection,
};
use wechatagent::tasks::TaskClaim;

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("事务测试客户".to_string()),
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
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: Some("need_discovery".to_string()),
        operation_state_reason: None,
        operation_state_confidence: Some(8),
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: Some(now),
        last_inbound_at: Some(now),
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

fn inbound(contact: &Contact, message_id: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some(message_id.to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "我想确认下一步怎么安排".to_string(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    }
}

fn reply_json(reply_text: &str, appointment: bool, commitment: bool) -> Value {
    let mut value = json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户希望明确下一步安排，当前诉求可以直接承接。",
        "relationshipRead": "客户愿意继续沟通，适合给出清晰而克制的下一步。",
        "operationGoal": "回应当前问题并准确记录客户主动提出的后续安排。",
        "knowledgeNeedReason": "本轮不涉及需要外部事实支撑的产品断言。",
        "memoryUpdateReason": "客户提出了明确的后续安排，需要保留结构化记录。",
        "selfCritique": "只回应已知信息，不把请求表述成已经确认的结果。",
        "whyShouldReply": "客户提出了明确问题，需要及时回应。",
        "whySkipReply": "",
        "riskSelfCheck": "不承诺未确认的结果，也不添加外部业务事实。",
        "riskLevel": "low",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "need_discovery",
        "operationStateConfidence": 8,
        "shouldReply": true,
        "replyText": reply_text,
        "usedKnowledgeIds": [],
        "matchedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "客户正在确认后续安排，采用顾问式回应。",
        "sufficiency": "enough",
        "missingTier": "",
        "clarificationIntent": ""
    });
    if appointment {
        value["appointmentRequest"] = json!({
            "requested": true,
            "requestText": "客户主动提出到院面诊",
            "preferredStart": "2026-08-20T10:00:00+08:00",
            "preferredEnd": "2026-08-20T11:00:00+08:00",
            "locationPreference": "院区待确认",
            "reason": "记录客户请求，等待人工确认"
        });
    }
    if commitment {
        value["lastCommitment"] = json!("稍后发送面诊准备事项");
        value["commitment"] = json!({
            "text": "稍后发送面诊准备事项",
            "dueAt": "2026-08-20T09:00:00+08:00"
        });
    }
    value
}

fn review_pass_json() -> Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": 9,
            "emotionalValue": 8,
            "productAccuracy": 9,
            "relationshipProgress": 8,
            "conversionReadiness": 7,
            "pressureRisk": 1,
            "boundaryPrivacySafety": 9,
            "factRisk": 0
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "候选回复不包含需要外部证据的业务断言。"
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "回复边界清晰，可以放行。",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true
    })
}

fn appointment_claim_gate_pass_json(request_text: &str) -> Value {
    json!({
        "claimKinds": ["appointment_action"],
        "claimsComplete": true,
        "claims": [{
            "sourceQuote": request_text,
            "claim": "The customer requested an appointment record.",
            "scope": "customer appointment request",
            "subject": "customer",
            "actionKind": "appointment_request",
            "evidenceNeed": "required",
            "negativePolarity": false,
            "productClaim": false,
            "evidenceRefs": ["current_user_message"],
            "reason": "The current customer message directly requests the visit."
        }],
        "catalogCoverageComplete": true,
        "catalogClaims": [],
        "reason": "The durable appointment request is directly supported by the customer."
    })
}

fn queue_authorized_turn(app: &common::TestApp, decision: Value) {
    let should_reply = decision["shouldReply"].as_bool().unwrap_or(false);
    let appointment_request_text = decision
        .get("appointmentRequest")
        .and_then(|value| value.get("requestText"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    app.llm.push_response(decision);
    if should_reply {
        app.llm.push_response(review_pass_json());
    }
    app.llm.push_response(
        appointment_request_text
            .as_deref()
            .map(appointment_claim_gate_pass_json)
            .unwrap_or_else(common::independent_claim_gate_pass_json),
    );
}

async fn insert_contact_and_inbound(
    app: &common::TestApp,
    contact: &Contact,
    inbound: &ConversationMessage,
) {
    app.state
        .db
        .contacts()
        .insert_one(contact, None)
        .await
        .expect("insert contact");
    app.state
        .db
        .messages()
        .insert_one(inbound, None)
        .await
        .expect("insert inbound");
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn authorized_turn_commits_review_request_commitment_run_and_complete_outbox_batch() {
    let app = common::TestApp::start_repl_set().await;
    let contact = managed_contact("production_commit_success");
    let mut inbound = inbound(&contact, "production-commit-success-001");
    inbound.content = "我想预约到院面诊，8月20日上午十点可以".to_string();
    insert_contact_and_inbound(&app, &contact, &inbound).await;
    queue_authorized_turn(&app, reply_json("第一段回应。\n\n第二段回应。", true, true));

    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("authorized production turn");

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query run")
        .expect("run exists");
    assert_eq!(run.status, "outbox_enqueued");
    assert_eq!(run.lifecycle, "completed");

    let review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "run_id": &run.run_id }, None)
        .await
        .expect("query review")
        .expect("review exists");
    assert_eq!(review.status, "outbox_enqueued");

    let appointments = app
        .state
        .db
        .appointments()
        .find(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query appointments")
        .try_collect::<Vec<_>>()
        .await
        .expect("collect appointments");
    assert_eq!(appointments.len(), 1);
    let appointment = &appointments[0];
    assert_eq!(appointment.status, "requested");
    assert!(appointment.confirmed_start.is_none());
    assert!(appointment.confirmed_end.is_none());
    assert!(appointment.confirmation_source_type.is_none());
    assert!(appointment.confirmation_source_id.is_none());

    let persisted_contact = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact.id }, None)
        .await
        .expect("query contact")
        .expect("contact exists");
    assert_eq!(persisted_contact.commitments.len(), 1);
    assert!(!persisted_contact.commitments[0].text().is_empty());
    assert!(persisted_contact.commitments[0].due_at().is_some());

    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find(doc! { "run_id": &run.run_id }, None)
        .await
        .expect("query outbox")
        .try_collect::<Vec<_>>()
        .await
        .expect("collect outbox");
    assert_eq!(outbox.len(), 2);
    assert!(outbox.iter().all(|entry| entry.status == "pending"));
    assert!(outbox.iter().all(|entry| entry.decision_id == review.id));
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn appointment_side_effect_is_enforced_independently_from_allowed_reply() {
    let app = common::TestApp::start_repl_set().await;
    let contact = managed_contact("production_commit_appointment_policy");
    let mut inbound = inbound(&contact, "production-commit-appointment-policy-001");
    inbound.content = "我想预约到院面诊，8月20日上午十点可以".to_string();
    insert_contact_and_inbound(&app, &contact, &inbound).await;

    let policy_update = app
        .state
        .db
        .operation_state_policies()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "state_key": "need_discovery",
                "current_version": true,
            },
            doc! {
                "$set": {
                    "allowed": ["reply", "acknowledgement", "silent", "follow_up"],
                    "forbidden": ["appointment_request"],
                    "status": "active",
                },
            },
            None,
        )
        .await
        .expect("set appointment-specific state policy");
    assert_eq!(policy_update.matched_count, 1);

    queue_authorized_turn(
        &app,
        reply_json(
            "我先记录你的到院请求，具体安排以确认结果为准。",
            true,
            false,
        ),
    );
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("state policy hold is a terminal Harness result");

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query run")
        .expect("run exists");
    assert_eq!(run.status, "held_by_ai_policy");
    assert_eq!(
        app.state
            .db
            .appointments()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count appointments"),
        0
    );
    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find(doc! { "run_id": &run.run_id }, None)
        .await
        .expect("query outbox")
        .try_collect::<Vec<_>>()
        .await
        .expect("collect outbox");
    assert_eq!(
        outbox.len(),
        1,
        "held inbound turns receive one neutral ack"
    );
    assert!(outbox[0].source_event_id.ends_with("#ack-placeholder"));
    assert!(outbox[0].decision_id.is_none());
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn explicit_principal_request_is_materialized_even_when_reply_is_held() {
    let app = common::TestApp::start_repl_set().await;
    let contact = managed_contact("production_commit_explicit_principal_hold");
    let inbound = inbound(&contact, "production-commit-explicit-principal-hold-001");
    insert_contact_and_inbound(&app, &contact, &inbound).await;

    // This is an explicit model-selected handoff, not the generic high-risk fallback. Keep the
    // generic ai-policy escalation switch off so the test proves the structured request is not
    // silently discarded when a later deterministic policy holds the reply.
    let policy = AskHumanPolicy {
        decider_chain: vec![DeciderRef {
            wxid: "principal_for_explicit_hold".to_string(),
            display_name: Some("值班负责人".to_string()),
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
    };
    let policy_bson = mongodb::bson::to_bson(&policy).expect("serialize ask-human policy");
    app.state
        .db
        .operation_domain_configs()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "current_version": true,
            },
            doc! { "$set": { "ask_human_policy": policy_bson } },
            None,
        )
        .await
        .expect("configure ask-human policy");

    let state_policy_update = app
        .state
        .db
        .operation_state_policies()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "state_key": "need_discovery",
                "current_version": true,
            },
            doc! {
                "$set": {
                    "allowed": ["silent"],
                    "forbidden": ["reply"],
                    "status": "active",
                },
            },
            None,
        )
        .await
        .expect("hold replies through operation-state policy");
    assert_eq!(state_policy_update.matched_count, 1);

    let mut decision = reply_json("我先核对一下当前安排，再把结果同步给你。", false, false);
    decision["nextStep"] = json!("ask_principal");
    decision["escalationRequest"] = json!({
        "needed": true,
        "category": "out_of_scope_decision",
        "reason": "当前安排需要有权人员确认",
        "questionForPrincipal": "是否按这条安排继续跟进？",
        "selfServiceablePart": "我先把你的需求记录下来",
        "isGeneralizable": false
    });
    queue_authorized_turn(&app, decision);

    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("held turn should settle without transport error");

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query run")
        .expect("run exists");
    assert_eq!(run.status, "held_by_ai_policy");

    let escalation = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "category": "out_of_scope_decision",
                "status": "pending",
            },
            None,
        )
        .await
        .expect("query explicit escalation")
        .expect("explicit model request must create a pending principal card");
    assert_eq!(escalation.reason, "当前安排需要有权人员确认");
    assert_eq!(
        escalation.question_for_principal,
        "是否按这条安排继续跟进？"
    );

    let customer_outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find(doc! { "run_id": &run.run_id }, None)
        .await
        .expect("query customer outbox")
        .try_collect::<Vec<_>>()
        .await
        .expect("collect customer outbox");
    assert_eq!(customer_outbox.len(), 1);
    assert!(customer_outbox[0]
        .source_event_id
        .ends_with("#ack-placeholder"));

    let principal_outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find(
            doc! {
                "account_id": "default",
                "contact_wxid": "principal_for_explicit_hold",
                "source_kind": "principal_escalation",
            },
            None,
        )
        .await
        .expect("query principal outbox")
        .try_collect::<Vec<_>>()
        .await
        .expect("collect principal outbox");
    assert_eq!(principal_outbox.len(), 1);
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn exhausted_authorization_budget_fails_closed_for_appointment_only_action() {
    let app = common::TestApp::start_repl_set().await;
    let contact = managed_contact("production_commit_appointment_budget");
    let mut inbound = inbound(&contact, "production-commit-appointment-budget-001");
    inbound.content = "我想预约到院面诊，时间还没确定".to_string();
    insert_contact_and_inbound(&app, &contact, &inbound).await;

    let budget_update = app
        .state
        .db
        .operation_domain_configs()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "current_version": true,
            },
            doc! { "$set": { "runtime_parameters.runMaxLlmCalls": 1i32 } },
            None,
        )
        .await
        .expect("limit run to the initial Reply Agent call");
    assert_eq!(budget_update.matched_count, 1);

    let mut decision = reply_json("", true, false);
    decision["shouldReply"] = json!(false);
    decision["whyShouldReply"] = json!("");
    decision["whySkipReply"] = json!("只记录客户请求，不发送消息。");
    app.llm.push_response(decision);

    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("budget exhaustion must settle as a held Harness turn");

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query run")
        .expect("run exists");
    assert_eq!(run.status, "blocked_by_budget");
    assert_eq!(
        app.state
            .db
            .appointments()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count appointments"),
        0
    );
    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find(doc! { "run_id": &run.run_id }, None)
        .await
        .expect("query outbox")
        .try_collect::<Vec<_>>()
        .await
        .expect("collect outbox");
    assert_eq!(
        outbox.len(),
        1,
        "budget-held inbound turns receive one neutral ack"
    );
    assert!(outbox[0].source_event_id.ends_with("#ack-placeholder"));
    assert!(outbox[0].decision_id.is_none());
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn missing_durable_contact_rolls_back_earlier_transaction_writes() {
    let app = common::TestApp::start_repl_set().await;
    let contact = managed_contact("production_commit_rollback");
    let mut inbound = inbound(&contact, "production-commit-rollback-001");
    inbound.content = "我想预约到院面诊，8月20日上午十点可以".to_string();
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound without contact");
    queue_authorized_turn(
        &app,
        reply_json("已记录你的请求，我先整理下一步。", true, true),
    );

    let result = handle_managed_message(&app.state, contact.clone(), &inbound).await;
    assert!(result.is_err(), "missing durable contact must abort commit");

    assert_eq!(
        app.state
            .db
            .decision_reviews()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count reviews"),
        0
    );
    assert_eq!(
        app.state
            .db
            .appointments()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count appointments"),
        0
    );
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count outbox"),
        0
    );

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query failed run")
        .expect("failed run audit remains durable");
    assert_ne!(run.lifecycle, "completed");
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn missing_durable_contact_rolls_back_no_reply_appointment_action() {
    let app = common::TestApp::start_repl_set().await;
    let contact = managed_contact("production_commit_no_reply_rollback");
    let mut inbound = inbound(&contact, "production-commit-no-reply-rollback-001");
    inbound.content = "我想预约到院面诊，8月20日上午十点可以".to_string();
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound without contact");

    let mut decision = reply_json("", true, false);
    decision["shouldReply"] = json!(false);
    decision["whyShouldReply"] = json!("");
    decision["whySkipReply"] = json!("本轮只记录客户主动提出的预约请求，不发送回复。");
    queue_authorized_turn(&app, decision);

    let result = handle_managed_message(&app.state, contact.clone(), &inbound).await;
    assert!(
        result.is_err(),
        "appointment-only commit must require a durable contact"
    );

    assert_eq!(
        app.state
            .db
            .decision_reviews()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count reviews"),
        0
    );
    assert_eq!(
        app.state
            .db
            .appointments()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count appointments"),
        0
    );
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count outbox"),
        0
    );
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn stale_follow_up_claim_commits_held_terminal_without_outbox() {
    let app = common::TestApp::start_repl_set().await;
    let mut contact = managed_contact("production_commit_stale_claim");
    let quiet_hours_update = app
        .state
        .db
        .operation_domain_configs()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "current_version": true,
            },
            doc! { "$set": { "runtime_parameters.quietHoursEnabled": false } },
            None,
        )
        .await
        .expect("disable workspace quiet hours");
    assert_eq!(quiet_hours_update.matched_count, 1);
    contact.last_message_at = None;
    contact.last_inbound_at = None;
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let now = DateTime::now();
    let task_id = ObjectId::new();
    let task = AgentTask {
        id: Some(task_id),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        kind: "follow_up".to_string(),
        run_at: now,
        expires_at: None,
        content: "重新判断是否适合继续沟通".to_string(),
        status: "running".to_string(),
        source_decision_id: None,
        review_required: true,
        attempt_count: 1,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: Some(now),
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    };
    let mut task_document = to_document(&task).expect("serialize task");
    task_document.insert("claim_token", "fresh-owner");
    task_document.insert("claim_generation", 2i64);
    app.state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .insert_one(task_document, None)
        .await
        .expect("insert claimed task");

    queue_authorized_turn(
        &app,
        reply_json("我来接着和你确认下一步安排。", false, false),
    );
    let stale_claim = TaskClaim {
        task_id,
        claim_token: "stale-owner".to_string(),
        claim_generation: 1,
    };

    handle_follow_up_task_with_claim(&app.state, task, Some(&stale_claim))
        .await
        .expect("stale claim is settled as held");

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "source_event_id": task_id.to_hex() }, None)
        .await
        .expect("query run")
        .expect("run exists");
    assert_eq!(run.status, "stale_task_claim");
    assert_eq!(run.lifecycle, "aborted_by_external_signal");
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count outbox"),
        0
    );

    let task_after = app
        .state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("query task")
        .expect("task remains");
    assert_eq!(task_after.get_str("status").unwrap(), "running");
    assert_eq!(task_after.get_str("claim_token").unwrap(), "fresh-owner");
    assert_eq!(task_after.get_i64("claim_generation").unwrap(), 2);
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn partial_existing_outbox_batch_holds_without_adding_rows() {
    let app = common::TestApp::start_repl_set().await;
    let contact = managed_contact("production_commit_partial_batch");
    let message_id = "production-commit-partial-001";
    let inbound = inbound(&contact, message_id);
    insert_contact_and_inbound(&app, &contact, &inbound).await;

    let seeded = enqueue(
        &app.state,
        EnqueueRequest {
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            run_id: "preexisting-partial-run".to_string(),
            decision_id: None,
            source_event_id: format!("{message_id}#seg0"),
            source_kind: "inbound_message".to_string(),
            content: "第一段回应。".to_string(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await
    .expect("seed one outbox segment");
    assert!(matches!(seeded, EnqueueOutcome::Created { .. }));

    queue_authorized_turn(
        &app,
        reply_json("第一段回应。\n\n第二段回应。", false, false),
    );
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("partial batch is held, not raised as a transport error");

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query run")
        .expect("run exists");
    assert_eq!(run.status, "blocked_by_safety_guard");
    assert_eq!(run.lifecycle, "failed_after_decision");

    let review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "run_id": &run.run_id }, None)
        .await
        .expect("query review")
        .expect("review exists");
    assert!(review
        .risks
        .iter()
        .any(|risk| risk == "outbox_partial_batch_conflict"));
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
            .await
            .expect("count outbox"),
        1
    );
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn projection_snapshot_failure_is_fail_soft_after_atomic_commit() {
    let mut app = common::TestApp::start_repl_set().await;
    app.state.config.post_decision_snapshot_max_bytes = 1;
    let contact = managed_contact("production_commit_projection_fail_soft");
    let inbound = inbound(&contact, "production-commit-projection-001");
    insert_contact_and_inbound(&app, &contact, &inbound).await;
    queue_authorized_turn(&app, reply_json("我先把下一步安排整理清楚。", false, false));

    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("projection preparation must not fail the committed reply");

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query run")
        .expect("run exists");
    assert_eq!(run.status, "outbox_enqueued");
    assert_eq!(run.lifecycle, "completed");
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "run_id": &run.run_id }, None)
            .await
            .expect("count outbox"),
        1
    );

    let review = app
        .state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .find_one(doc! { "run_id": &run.run_id }, None)
        .await
        .expect("query raw review")
        .expect("review exists");
    assert_eq!(
        review.get_str("post_decision_status").unwrap(),
        "failed_terminal"
    );
    assert_eq!(
        review.get_str("post_decision_error_kind").unwrap(),
        "snapshot_preparation"
    );
    assert!(!review.contains_key("post_decision_payload"));
}

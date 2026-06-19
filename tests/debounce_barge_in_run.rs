//! 并发多消息去抖 / 抢占重算（barge-in）集成测试。
//!
//! 覆盖 gateway 的协作式中止（should_abort_send）在真实 MongoDB 上的两条路径：
//! 1. 抢占触发：runner 运行期间有更新入站 → guard() 返回 true → 在 apply / outbox
//!    之前放弃这次生成，run log 落 gateway_status=superseded_by_new_inbound，
//!    不入队 outbox，且 last_agent_run_at 不被推进（保证重算 precheck 干净）；
//! 2. 无抢占：guard() 恒 false → 正常走完 decision→review→outbox，approved 终态、
//!    outbox 入队一行。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。调度器本身的去抖 /
//! spawn-vs-bump / generation 抢占信号由 `src/webhooks.rs` 的纯函数单测覆盖；
//! 本文件聚焦"抢占信号穿过网关后的真实落库副作用"。

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::handle_managed_message_aggregated;
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
        raw: None,
        created_at: DateTime::now(),
    }
}

fn reply_agent_decision_json(reply_text: &str, why_should_reply: &str) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户表达明确，正在评估我方在企业 IM 场景下的方案适配度，并给出落地预算与时间。",
        "relationshipRead": "对话氛围积极，对我方专业度信任，但对实施周期与成本有一定顾虑，关系处于稳步推进期。",
        "operationGoal": "聚焦在帮客户厘清下一步排期与成本边界，让客户在不被推销压力下感到掌控感与确定性。",
        "knowledgeNeedReason": "客户提及了具体场景与预算需求，需要结合产品能力切片确认我方覆盖范围与交付承诺边界。",
        "memoryUpdateReason": "本轮新增客户预算与时间锚点信息，需要写入长期记忆以支持后续节奏与产品方案匹配。",
        "selfCritique": "上一轮我方过早提到价格档位，本次需收敛信息密度并先确认客户优先级再给出下一步建议。",
        "whyShouldReply": why_should_reply,
        "whySkipReply": "",
        "riskSelfCheck": "本轮回复不涉及未验证的产品能力承诺，仅给出节奏与下一步动作建议，不触发安全门阈值。",
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
        "conversationModeReason": "客户进入方案/能力评估阶段，按顾问模式明确处理产品与排期问题。",
    })
}

fn review_agent_pass_json(review_summary: &str) -> serde_json::Value {
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
        "reviewSummary": review_summary,
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true,
    })
}

/// 抢占触发：guard() 在网关跑到落盘 / 入队检查点时返回 true，网关 SHALL 在
/// apply_agent_updates / outbox 之前放弃这次生成。断言：
/// - agent_run_logs 落 gateway_status=superseded_by_new_inbound；
/// - 不入队 outbox（0 行）；
/// - last_agent_run_at 未被推进（仍为 None，保证重算 precheck 不会误 rate_limited）。
#[tokio::test]
#[ignore]
async fn barge_in_aborts_before_outbox_and_does_not_advance_last_run() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_barge_in");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "msg_barge_001",
        "你们的实施周期一般多久？大概预算需要多少？",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    // 决策 + 审查都成功——本就会走到落盘 / 入队，正好被 guard 在检查点拦下。
    app.llm.push_response(reply_agent_decision_json(
        "我们一般 2~4 周可上线，预算和场景深度相关，要不要先按你们的优先级排排序？",
        "客户主动询问实施周期与预算，回复能确认需求颗粒度并降低决策摩擦，是关键推进时机。",
    ));
    app.llm.push_response(review_agent_pass_json(
        "回复语气良好、不越界承诺，可放行——但本测试用 guard 模拟期间到达更新入站。",
    ));

    // guard 恒 true：模拟"这次生成期间用户又发了新消息"。
    let guard: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| true);

    handle_managed_message_aggregated(&app.state, contact.clone(), &inbound, Some(guard))
        .await
        .expect("aggregated handler returns Ok even when superseded");

    // run log 落 superseded_by_new_inbound 终态。
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
    assert_eq!(
        log.status, "superseded_by_new_inbound",
        "被更新入站取代时 run log status 必须为 superseded_by_new_inbound，实际 {:?}",
        log.status
    );

    // 不入队 outbox。
    let outbox_count = app
        .state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("count outbox");
    assert_eq!(
        outbox_count, 0,
        "抢占放弃路径绝不入队 outbox，实际 {} 行",
        outbox_count
    );

    // last_agent_run_at 未被推进（apply_agent_updates 在检查点之后，未执行）。
    let reloaded = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": &contact.wxid }, None)
        .await
        .expect("reload contact")
        .expect("contact present");
    assert!(
        reloaded.last_agent_run_at.is_none(),
        "抢占放弃时不得推进 last_agent_run_at，实际 {:?}",
        reloaded.last_agent_run_at
    );
}

/// 无抢占：guard() 恒 false，聚合流水线正常走完，approved 终态 + outbox 入队一行。
/// 这是去抖窗口结束、用户已说完后的常态路径。
#[tokio::test]
#[ignore]
async fn no_barge_in_completes_normally_and_enqueues_outbox() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_no_barge");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "msg_no_barge_001",
        "你们的实施周期一般多久？大概预算需要多少？",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    app.llm.push_response(reply_agent_decision_json(
        "我们一般 2~4 周可上线，预算和场景深度相关，要不要先按你们的优先级排排序？",
        "客户主动询问实施周期与预算，回复能确认需求颗粒度并降低决策摩擦，是关键推进时机。",
    ));
    app.llm.push_response(review_agent_pass_json(
        "回复语气良好、不越界承诺，可放行。",
    ));

    // guard 恒 false：去抖窗口已结束、无更新入站，正常发送。
    let called = Arc::new(AtomicBool::new(false));
    let called_in = called.clone();
    let guard: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
        called_in.store(true, Ordering::SeqCst);
        false
    });

    handle_managed_message_aggregated(&app.state, contact.clone(), &inbound, Some(guard))
        .await
        .expect("aggregated handler ok");

    assert!(
        called.load(Ordering::SeqCst),
        "guard 应在落盘 / 入队检查点被实际调用"
    );

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
    assert_eq!(
        log.final_review_status, "approved",
        "无抢占路径终态必须 approved，实际 {:?}",
        log.final_review_status
    );

    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query outbox by run_id")
        .expect("approved 路径必须入队 outbox 一行");
    assert_eq!(outbox.contact_wxid, contact.wxid);

    // last_agent_run_at 被正常推进。
    let reloaded = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": &contact.wxid }, None)
        .await
        .expect("reload contact")
        .expect("contact present");
    assert!(
        reloaded.last_agent_run_at.is_some(),
        "正常发送路径应推进 last_agent_run_at"
    );
}

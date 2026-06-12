//! 全流程合并套件：把"运营 agent 全流程"与"并发去抖 / 抢占重算"两条线
//! 并到同一份端到端套件里，用真实 MongoDB（testcontainers）+ mock LLM 串起
//! webhook 之后的整条决策 → 审查 → [单轮 revision] → outbox → 发送闸的全细节。
//!
//! 这是**新增**的合并套件，不替换、不删改既有聚焦文件
//! （`tests/happy_path_run.rs` / `tests/debounce_barge_in_run.rs` 仍各自独立
//! 保留）。本套件覆盖更多用例并把两条线放在同一处对照：
//!
//! A. 运营 agent 全流程（`handle_managed_message`，无 barge-in guard）：
//!    A1 直发：Reply→Review(pass) → final_review_status=approved + outbox 一行；
//!    A2 单轮 revision：Reply→Review(needs_revision)→Reply(revised)→Review(pass)
//!       → final_review_status=revision_applied_approved + revision_applied=true；
//!    A3 不回复：shouldReply=false → 1 次 LLM 调用、run log status=no_reply、0 outbox；
//!    A4 知识 tool-loop：list_catalog/open_chunk/answer → toolTrace 三段齐全 + approved。
//!
//! B. 并发去抖 / 抢占重算（`handle_managed_message_aggregated`，带协作式 guard）：
//!    B1 抢占：guard()恒 true → gateway_status=superseded_by_new_inbound、0 outbox、
//!       last_agent_run_at 未推进（保证重算 precheck 不会误判 rate_limited）；
//!    B2 无抢占：guard()恒 false → approved + outbox 一行 + last_agent_run_at 推进；
//!    B3 抢占后重算：第一遍 guard=true 被弃（0 outbox / 不推进），第二遍 guard=false
//!       正常落地一行 outbox —— 模拟"用户连发后说完，最终只发一次"的串行重算语义。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。调度器本身的去抖 /
//! spawn-vs-bump / generation 抢占信号由 `src/webhooks.rs` 纯函数单测覆盖；本套件
//! 聚焦"决策链 + 抢占信号穿过网关后的真实落库副作用"。

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{handle_managed_message, handle_managed_message_aggregated};
use wechatagent::models::{
    AgentStatus, Contact, ConversationMessage, MessageDirection, OperationKnowledgeChunk,
};

// ── 公共 fixture builder ──────────────────────────────────────────────────

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
        agent_status: Default::default(),
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
        deal_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

/// managed 状态 Contact（precheck_send_gateway 仅放行 Managed 路径）。
fn make_managed_contact(wxid: &str) -> Contact {
    let mut contact = make_contact(wxid);
    contact.agent_status = AgentStatus::Managed;
    contact
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

/// seed 一条 active + verified 的 user_operations chunk，返回 hex chunk_id。
/// knowledge_agent 的 list_catalog / open_chunk 只暴露 verified + active 切片，
/// 故 fixture 必须显式置 verified + active，否则 agent 看不到也打不开。
async fn seed_verified_chunk(
    app: &common::TestApp,
    contact: &Contact,
    title: &str,
    body: &str,
) -> String {
    let id = ObjectId::new();
    let now = DateTime::now();
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: contact.workspace_id.clone(),
        account_id: Some(contact.account_id.clone()),
        domain: "user_operations".to_string(),
        knowledge_type: Some("product_capability".to_string()),
        title: title.to_string(),
        summary: Some(body.to_string()),
        body: Some(body.to_string()),
        source_quote: Some(body.to_string()),
        integrity_status: Some("verified".to_string()),
        confidence_score: Some(88),
        status: "active".to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some("methodology".to_string()),
        dynamic_confidence: Some(0.9),
        chunk_type: "product_fact".to_string(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert verified chunk");
    id.to_hex()
}

/// Reply Agent 决策 JSON（shouldReply=true）。`knowledge_need` 默认场景传
/// `not_required`，tool-loop 场景传 `required`。
fn reply_agent_decision_json(
    reply_text: &str,
    why_should_reply: &str,
    knowledge_need: &str,
) -> serde_json::Value {
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
        "knowledgeNeed": knowledge_need,
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

/// Reply Agent 决策 JSON（shouldReply=false）——不回复路径。
/// should_run_review 在 should_reply=false 时返回 false，故本路径只产生 1 次
/// LLM 调用（决策本身），不进 Review，run log status 落 no_reply。
fn reply_agent_no_reply_json(why_skip_reply: &str) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户只是随口寒暄 / 表达情绪，没有提出需要承接的诉求或问题。",
        "relationshipRead": "关系平稳，无新的推进抓手，此刻主动回复反而会显得突兀。",
        "operationGoal": "保持克制，不在没有信息增量时强行触达，维护拟人节奏。",
        "knowledgeNeedReason": "无产品或能力相关诉求，无需调用知识库。",
        "memoryUpdateReason": "本轮无新增可沉淀的长期事实。",
        "selfCritique": "需要克制住那种必须回点什么的冲动，沉默有时是更拟人的选择。",
        "whyShouldReply": "",
        "whySkipReply": why_skip_reply,
        "riskSelfCheck": "不回复，无任何外发内容，无安全门风险。",
        "riskLevel": "low",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": false,
        "consolidationNeeded": false,
        "operationState": "need_discovery",
        "shouldReply": false,
        "replyText": "",
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "无承接抓手，按顾问模式选择此刻不打扰。",
    })
}

/// Review Agent 通过 JSON（分数全部 ≥ 阈值）。`needs_revision` / `revision_direction`
/// 由调用方覆盖以驱动 single-shot revision 路径。
fn review_agent_pass_json(
    needs_revision: bool,
    revision_direction: &str,
    review_summary: &str,
) -> serde_json::Value {
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
        "needsRevision": needs_revision,
        "revisionDirection": revision_direction,
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": !needs_revision,
    })
}

/// 查询某 contact 唯一一行 agent_run_logs（多数场景一个 run）。
async fn fetch_run_log(
    app: &common::TestApp,
    contact: &Contact,
) -> wechatagent::models::AgentRunLog {
    app.state
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
        .expect("agent_run_logs row exists")
}

async fn outbox_count_for(app: &common::TestApp, contact: &Contact) -> u64 {
    app.state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("count outbox")
}

// ── A. 运营 agent 全流程（handle_managed_message） ─────────────────────────

/// A1 直发：Reply→Review(pass)，final_review_status=approved，outbox 入队一行。
#[tokio::test]
#[ignore]
async fn full_flow_a1_direct_approved_enqueues_outbox() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("ff_a1_direct");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "ff_a1_msg_001",
        "你们的方案我们大概了解了，下一步想看看怎么落地试点。",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    app.llm.push_response(reply_agent_decision_json(
        "可以，落地试点我们一般先圈一个核心场景跑通，要不要先按你们最急的场景来定试点范围？",
        "客户主动提出进入试点落地，这是把关系推进到执行阶段的关键时机，回复能直接降低决策摩擦。",
        "not_required",
    ));
    app.llm.push_response(review_agent_pass_json(
        false,
        "",
        "回复承接试点诉求、不越界承诺，语气自然，可直接放行。",
    ));

    let before = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    assert_eq!(
        app.llm.calls() - before,
        2,
        "直发路径：Reply ×1 + Review ×1 = 2 次 LLM 调用"
    );

    let log = fetch_run_log(&app, &contact).await;
    assert_eq!(
        log.final_review_status, "approved",
        "直发路径终态必须 approved，实际 {:?}",
        log.final_review_status
    );
    assert!(
        !log.revision_applied,
        "直发路径不应触发 revision，实际 {:?}",
        log.revision_applied
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
}

/// A2 单轮 revision：Reply→Review(needs_revision)→Reply(revised)→Review(pass)，
/// final_review_status=revision_applied_approved + revision_applied=true。
#[tokio::test]
#[ignore]
async fn full_flow_a2_single_shot_revision() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("ff_a2_revision");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "ff_a2_msg_001",
        "我们最近在评估几家方案，你们的实施周期一般多久？大概预算需要多少？",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    app.llm.push_response(reply_agent_decision_json(
        "理解你们在做横向对比。我们一般 2~4 周可上线，预算区间和场景深度直接相关，要不要先按你们的优先级排排序？",
        "客户主动询问实施周期与预算，回复能确认需求颗粒度并降低决策摩擦，是关键推进时机。",
        "not_required",
    ));
    app.llm.push_response(review_agent_pass_json(
        true,
        "去掉对预算区间的模糊措辞，给出更具体的 2~4 周分阶段交付样例，并明确指出我们不会在此轮强推报价。",
        "首版语气良好，但预算与交付描述偏笼统，需要按 revisionDirection 修正后再放行。",
    ));
    app.llm.push_response(reply_agent_decision_json(
        "理解你们在做横向对比。常见落地节奏是：第 1~2 周梳理流程并接通试点账号，第 3~4 周扩到核心场景，预算我们这轮只做范围确认，等优先级清楚再给报价更稳。",
        "客户主动询问节奏与预算，按修正方向给出更具体的分阶段交付样例并明确不强推报价，能直接降低对方的决策压力。",
        "not_required",
    ));
    app.llm.push_response(review_agent_pass_json(
        false,
        "",
        "二轮回复已按 revisionDirection 收敛信息密度，不再有笼统措辞，可以放行。",
    ));

    let before = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    assert_eq!(
        app.llm.calls() - before,
        4,
        "revision 路径：Reply ×2 + Review ×2 = 4 次 LLM 调用"
    );

    let log = fetch_run_log(&app, &contact).await;
    assert!(
        log.revision_applied,
        "revision_applied 必须为 true，实际 {:?}",
        log
    );
    assert_eq!(
        log.final_review_status, "revision_applied_approved",
        "final_review_status 必须为 revision_applied_approved，实际 {:?}",
        log.final_review_status
    );
    assert!(
        log.pre_revision_summary
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false),
        "pre_revision_summary 必须非空，实际 {:?}",
        log.pre_revision_summary
    );
    assert!(
        log.post_revision_summary
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false),
        "post_revision_summary 必须非空，实际 {:?}",
        log.post_revision_summary
    );

    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query outbox by run_id")
        .expect("revision approved 路径必须入队 outbox 一行");
    assert_eq!(outbox.contact_wxid, contact.wxid);
}

/// A3 不回复：shouldReply=false → 1 次 LLM 调用、run log status=no_reply、0 outbox。
#[tokio::test]
#[ignore]
async fn full_flow_a3_no_reply_skips_review_and_outbox() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("ff_a3_no_reply");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(&contact, "ff_a3_msg_001", "哈哈哈哈好的👌");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    app.llm.push_response(reply_agent_no_reply_json(
        "客户只是确认收到 / 随口寒暄，没有新的承接抓手，此刻沉默更拟人，主动回复会显得突兀。",
    ));

    let before = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    assert_eq!(
        app.llm.calls() - before,
        1,
        "不回复路径：shouldReply=false 不进 Review，应恰好 1 次 LLM 调用"
    );

    let log = fetch_run_log(&app, &contact).await;
    assert_eq!(
        log.status, "no_reply",
        "不回复路径 run log status 必须为 no_reply，实际 {:?}",
        log.status
    );

    let count = outbox_count_for(&app, &contact).await;
    assert_eq!(count, 0, "不回复路径绝不入队 outbox，实际 {} 行", count);
}

/// A4 知识 tool-loop：list_catalog/open_chunk/answer → toolTrace 三段齐全 + approved。
#[tokio::test]
#[ignore]
async fn full_flow_a4_knowledge_tool_loop() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("ff_a4_tool_loop");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "ff_a4_msg_001",
        "你们这套对企业 IM 接入有没有具体能力清单？我想看一下你们具体能覆盖哪些场景。",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    let chunk_id = seed_verified_chunk(
        &app,
        &contact,
        "企业 IM 接入能力清单",
        "覆盖账号纳管、自动应答、手动指令三类核心能力；支持私有化部署与 webhook 回调。",
    )
    .await;

    app.llm.push_response(json!({
        "action": "open_chunk",
        "ids": [chunk_id.clone()],
    }));
    app.llm.push_response(json!({
        "action": "answer",
        "citedChunkIds": [chunk_id.clone()],
        "sourceQuotes": [{
            "chunkId": chunk_id.clone(),
            "quote": "覆盖账号纳管、自动应答、手动指令三类核心能力",
            "sourceAnchorIndex": 0
        }],
        "answer": "我们已形成企业 IM 场景的能力清单，覆盖账号纳管、自动应答、手动指令三类核心能力。",
    }));
    app.llm.push_response(reply_agent_decision_json(
        "我们已经形成了企业 IM 场景下的能力清单，按你们的接入侧重点会优先覆盖账号纳管、自动应答、手动指令三类，要不要我先发一份场景对照？",
        "客户明确询问能力覆盖范围，结合知识库切片给出三类核心能力对照，是把对话推进到具体方案的关键时机。",
        "required",
    ));
    app.llm.push_response(review_agent_pass_json(
        false,
        "",
        "知识路由已扫过 list_catalog/open_chunk/answer，回复未越界做产品承诺，整体可放行。",
    ));

    let before = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    assert_eq!(
        app.llm.calls() - before,
        4,
        "tool-loop 路径：knowledge_agent ×2（open_chunk + answer）+ Reply ×1 + Review ×1 = 4 次"
    );

    let log = fetch_run_log(&app, &contact).await;
    assert_eq!(
        log.final_review_status, "approved",
        "tool-loop 路径终态必须 approved，实际 {:?}",
        log.final_review_status
    );

    let tool_trace = log
        .knowledge_route
        .get_array("toolTrace")
        .expect("knowledge_route.toolTrace array")
        .clone();
    let tool_names: Vec<String> = tool_trace
        .iter()
        .filter_map(|b| b.as_document())
        .filter_map(|d| d.get_str("tool").ok().map(|s| s.to_string()))
        .collect();
    assert!(
        tool_names.iter().any(|n| n == "list_catalog"),
        "tool_trace 必须含 list_catalog，实际 {:?}",
        tool_names
    );
    assert!(
        tool_names.iter().any(|n| n == "open_chunk"),
        "tool_trace 必须含 open_chunk，实际 {:?}",
        tool_names
    );
    assert!(
        tool_names.iter().any(|n| n == "answer"),
        "tool_trace 必须含 answer，实际 {:?}",
        tool_names
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
}

// ── B. 并发去抖 / 抢占重算（handle_managed_message_aggregated） ─────────────

/// B1 抢占：guard()恒 true → gateway_status=superseded_by_new_inbound、0 outbox、
/// last_agent_run_at 未推进。
#[tokio::test]
#[ignore]
async fn full_flow_b1_barge_in_aborts_before_outbox() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("ff_b1_barge_in");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "ff_b1_msg_001",
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
        "not_required",
    ));
    app.llm.push_response(review_agent_pass_json(
        false,
        "",
        "回复语气良好、不越界承诺，可放行——但本测试用 guard 模拟期间到达更新入站。",
    ));

    let guard: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| true);
    handle_managed_message_aggregated(&app.state, contact.clone(), &inbound, Some(guard))
        .await
        .expect("aggregated handler returns Ok even when superseded");

    let log = fetch_run_log(&app, &contact).await;
    assert_eq!(
        log.status, "superseded_by_new_inbound",
        "被更新入站取代时 run log status 必须为 superseded_by_new_inbound，实际 {:?}",
        log.status
    );

    let count = outbox_count_for(&app, &contact).await;
    assert_eq!(count, 0, "抢占放弃路径绝不入队 outbox，实际 {} 行", count);

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

/// B2 无抢占：guard()恒 false → approved + outbox 一行 + last_agent_run_at 推进。
#[tokio::test]
#[ignore]
async fn full_flow_b2_no_barge_in_completes_normally() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("ff_b2_no_barge");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "ff_b2_msg_001",
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
        "not_required",
    ));
    app.llm.push_response(review_agent_pass_json(
        false,
        "",
        "回复语气良好、不越界承诺，可放行。",
    ));

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

    let log = fetch_run_log(&app, &contact).await;
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

/// B3 抢占后重算：第一遍 guard=true 被弃（0 outbox / 不推进 last_agent_run_at），
/// 第二遍 guard=false 正常落地一行 outbox。模拟"用户连发 → 第一次生成被新消息
/// 抢占重算 → 用户说完后最终只发一次"的串行重算语义。
#[tokio::test]
#[ignore]
async fn full_flow_b3_barge_in_then_recompute_sends_once() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("ff_b3_recompute");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");

    // 第一遍：用户发出首条，生成期间被新消息抢占（guard 恒 true）。
    let inbound_1 = make_inbound(&contact, "ff_b3_msg_001", "你们的实施周期一般多久？");
    app.state
        .db
        .messages()
        .insert_one(&inbound_1, None)
        .await
        .expect("insert inbound 1");
    app.llm.push_response(reply_agent_decision_json(
        "我们一般 2~4 周可上线，要不要先按你们的优先级排排序？",
        "客户询问实施周期，回复能降低决策摩擦。",
        "not_required",
    ));
    app.llm.push_response(review_agent_pass_json(
        false,
        "",
        "首版可放行，但本遍用 guard 模拟期间到达更新入站。",
    ));
    let guard_true: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| true);
    handle_managed_message_aggregated(&app.state, contact.clone(), &inbound_1, Some(guard_true))
        .await
        .expect("first pass returns Ok when superseded");

    // 第一遍后：0 outbox、last_agent_run_at 未推进 → 重算 precheck 不会误 rate_limited。
    assert_eq!(
        outbox_count_for(&app, &contact).await,
        0,
        "第一遍被抢占应 0 outbox"
    );
    let after_first = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": &contact.wxid }, None)
        .await
        .expect("reload contact")
        .expect("contact present");
    assert!(
        after_first.last_agent_run_at.is_none(),
        "第一遍被抢占不得推进 last_agent_run_at，实际 {:?}",
        after_first.last_agent_run_at
    );

    // 第二遍（重算）：用户已说完，最新入站进网关，guard=false 正常落地。
    let inbound_2 = make_inbound(
        &contact,
        "ff_b3_msg_002",
        "你们的实施周期一般多久？另外预算大概要多少？我们想尽快定下来。",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound_2, None)
        .await
        .expect("insert inbound 2");
    app.llm.push_response(reply_agent_decision_json(
        "我们一般 2~4 周可上线，预算和场景深度相关；既然想尽快定，要不要先按最急的场景圈个试点范围？",
        "客户合并询问周期 + 预算并表达急迫，聚合后一次性承接能降低决策摩擦、推进到执行。",
        "not_required",
    ));
    app.llm.push_response(review_agent_pass_json(
        false,
        "",
        "重算回复承接了合并后的完整诉求，不越界承诺，可放行。",
    ));
    let guard_false: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| false);
    handle_managed_message_aggregated(&app.state, contact.clone(), &inbound_2, Some(guard_false))
        .await
        .expect("recompute pass ok");

    // 重算落地：最终只 1 行 outbox（第一遍被弃、第二遍入队），last_agent_run_at 推进。
    assert_eq!(
        outbox_count_for(&app, &contact).await,
        1,
        "抢占 + 重算最终应只入队 1 行 outbox"
    );
    let after_second = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": &contact.wxid }, None)
        .await
        .expect("reload contact")
        .expect("contact present");
    assert!(
        after_second.last_agent_run_at.is_some(),
        "重算落地后应推进 last_agent_run_at"
    );
}

//! HP / Task 24：mock LLM + testcontainers MongoDB happy-path 集成测试。
//!
//! 选 `consolidate_contact_memory` 作为代表性 happy path：
//! 1. 启动 TestApp（带 prompt pack v2）；
//! 2. seed 一个 Contact + 一条 pending MemoryCandidate；
//! 3. push 一条合法 memoryCard JSON 给 mock LLM；
//! 4. 调用 `consolidate_contact_memory`；
//! 5. 断言：
//!    - 恰好调用 1 次 LLM；
//!    - operating_memory.memory_card.coreFacts 含 LLM 输出的事实；
//!    - 候选状态被推到非 pending（completed / consumed）。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。

mod common;

use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{consolidate_contact_memory, handle_managed_message};
use wechatagent::models::{
    AgentStatus, Contact, ConversationMessage, MemoryCandidate, MessageDirection,
    OperationKnowledgeChunk,
};

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
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        deal_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

fn make_candidate(contact: &Contact) -> MemoryCandidate {
    let now = DateTime::now();
    MemoryCandidate {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        run_id: Some("run_happy_path".to_string()),
        source: "decision".to_string(),
        candidates: vec![doc! {
            "category": "core",
            "fact": "客户在做企业 IM 选型",
            "importance": 8
        }],
        memory_write_score: 8,
        status: "pending".to_string(),
        reason: Some("init".to_string()),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn consolidate_contact_memory_writes_core_fact_via_mock_llm() {
    let app = common::TestApp::start().await;
    let contact = make_contact("user_happy_path");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let candidate = make_candidate(&contact);
    app.state
        .db
        .memory_candidates()
        .insert_one(&candidate, None)
        .await
        .expect("insert candidate");

    // Mock LLM 输出一个合法的 memoryCard consolidation 结果。
    app.llm.push_response(json!({
        "memoryCard": {
            "coreFacts": ["客户在做企业 IM 选型"],
            "recentFacts": ["最近询问了对接成本"],
            "preferences": [],
            "doNotDo": [],
            "objections": [],
            "openLoops": [],
            "openQuestions": [],
            "deprecatedFacts": [],
            "conflicts": [],
            "confirmedFacts": [],
            "commitments": []
        },
        "summary": "客户在做选型，关注成本",
        "discarded": []
    }));

    let before_calls = app.llm.calls();
    consolidate_contact_memory(&app.state, &contact, None)
        .await
        .expect("consolidate succeeds");
    let after_calls = app.llm.calls();
    assert_eq!(
        after_calls - before_calls,
        1,
        "happy path 应恰好调用 1 次 LLM"
    );

    // operating_memory 应被写入 / 更新。
    let memory = app
        .state
        .db
        .operating_memories()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            None,
        )
        .await
        .unwrap()
        .expect("operating_memory present");

    // task 6.1：`memory_card` 现在是 `MemoryCardTyped`；通过 `as_text()` 拿
    // 每条事实的文本表示（兼容 Plain / Structured 两种 repr）。
    let core_facts: Vec<String> = memory
        .memory_card
        .core_facts
        .iter()
        .map(|f| f.as_text().to_string())
        .collect();
    assert!(
        core_facts
            .iter()
            .any(|s| s == "客户在做企业 IM 选型"),
        "coreFacts 应包含 LLM 输出的事实，实际：{:?}",
        core_facts
    );

    // 候选状态应离开 pending（被 consolidator 标为已消费）。
    let still_pending = app
        .state
        .db
        .memory_candidates()
        .count_documents(
            doc! {
                "contact_wxid": &contact.wxid,
                "status": "pending"
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        still_pending, 0,
        "candidate 应被消费，pending 计数应为 0，实际 {}",
        still_pending
    );
}

// ── W6 / Task 7.4：autonomy 端到端冒烟（revision + tool-loop） ─────────────

/// 构造一个 managed 状态的 Contact（precheck_send_gateway 仅放行 Managed 路径）。
fn make_managed_contact(wxid: &str) -> Contact {
    let mut contact = make_contact(wxid);
    contact.agent_status = AgentStatus::Managed;
    contact
}

/// 构造一条入站消息 ConversationMessage，gateway 触发用。
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

/// 种入一条 active + verified 的 user_operations chunk，返回其 hex chunk_id。
///
/// agent-first knowledge_agent 的 list_catalog / open_chunk 都只暴露
/// `integrity_status="verified"` 的 active chunk（与 router corpus 对齐），故 fixture
/// 必须显式置 verified + active，否则 agent 看不到也打不开。
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


///
/// `reply_text` / `why_should_reply` 由调用方覆盖以表达 revision 前后版本；
/// `knowledge_need` 默认 `not_required`，tool-loop 测试覆盖为 `required`。
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

/// 构造 Review Agent 通过的 JSON（分数全部 ≥ 阈值）。
///
/// `needs_revision` / `revision_direction` 由调用方覆盖以驱动 single-shot revision 路径。
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

#[tokio::test]
#[ignore]
async fn autonomy_full_loop_with_revision() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_revision");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "msg_revision_001",
        "我们最近在评估几家方案，你们的实施周期一般多久？大概预算需要多少？",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    // R2 single-shot revision 完整链路：
    //   #1 Reply Agent → #2 Review Agent (needs_revision=true)
    //   → #3 Reply Agent (revised) → #4 Review Agent (pass)
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
        "客户主动询问节奏与预算，按修正方向给出更具体的 2~4 周分阶段交付样例并明确不强推报价，能直接降低对方的决策压力。",
        "not_required",
    ));
    app.llm.push_response(review_agent_pass_json(
        false,
        "",
        "二轮回复已按 revisionDirection 收敛信息密度，不再有笼统措辞，可以放行。",
    ));

    let before_calls = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    let after_calls = app.llm.calls();
    assert_eq!(
        after_calls - before_calls,
        4,
        "revision happy path: Reply Agent ×2 + Review Agent ×2 = 4 次 LLM 调用"
    );

    // 断言 agent_run_logs 落入 revision_applied_approved 终态。
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
        log.revision_applied,
        "revision_applied 必须为 true，实际 log = {:?}",
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

    // outbox 应当在 run_id 维度入队（idempotency_key 已生成），随后由 dispatcher 推进。
    // 测试无 dispatcher worker，因此只需断言行存在即可（W4 场景本身在 outbox_integration 覆盖）。
    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query outbox by run_id")
        .expect("revision approved 路径必须入队 outbox 一行");
    assert_eq!(
        outbox.contact_wxid, contact.wxid,
        "outbox.contact_wxid 不一致：{:?}",
        outbox
    );
}

#[tokio::test]
#[ignore]
async fn autonomy_tool_loop_happy_path() {
    let app = common::TestApp::start().await;
    let contact = make_managed_contact("user_tool_loop");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "msg_tool_loop_001",
        "你们这套对企业 IM 接入有没有具体能力清单？我想看一下你们具体能覆盖哪些场景。",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    // 种入一条 verified chunk，让 agent-first knowledge_agent 真正进入多轮探索循环。
    // route_operation_knowledge 在知识库非空时把上下文折成 query 喂给 knowledge_agent::answer，
    // 后者先 list_catalog（DB 调用，不耗 LLM），再进入 ≤3 轮 LLM 循环。
    let chunk_id = seed_verified_chunk(
        &app,
        &contact,
        "企业 IM 接入能力清单",
        "覆盖账号纳管、自动应答、手动指令三类核心能力；支持私有化部署与 webhook 回调。",
    )
    .await;

    // LLM 调用序列（agent-first 架构）：
    //   #1 knowledge_agent round1 —— open_chunk 打开上面种入的 chunk；
    //   #2 knowledge_agent round2 —— answer，cite 该 chunk；
    //   #3 Reply Agent —— 单轮决策（知识路由前置，已携带 agent 命中上下文）；
    //   #4 Review Agent —— full review，approved 通过。
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

    let before_calls = app.llm.calls();
    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");
    let after_calls = app.llm.calls();
    assert_eq!(
        after_calls - before_calls,
        4,
        "tool-loop happy path: knowledge_agent ×2（open_chunk + answer）+ Reply ×1 + Review ×1 = 4 次 LLM 调用"
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
        "tool-loop 路径终态必须 approved，实际 {:?}",
        log.final_review_status
    );

    // KnowledgeRouteResult serde_rename_all="camelCase" → BSON 字段为 toolTrace。
    // agent-first knowledge_agent 透传的 trace：第一段恒为 list_catalog（启动时 DB 拉目录），
    // 随后是 LLM 驱动的 open_chunk / answer。
    let tool_trace = log
        .knowledge_route
        .get_array("toolTrace")
        .expect("knowledge_route.toolTrace array")
        .clone();
    assert!(
        tool_trace.len() >= 3,
        "tool_trace 必须至少包含 list_catalog/open_chunk/answer 三段，实际 {:?}",
        tool_trace
    );
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

    // outbox 入队后等待最多 10s（无 dispatcher worker 时只查行存在）。
    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query outbox by run_id")
        .expect("approved 路径必须入队 outbox 一行");
    let _ = (outbox, Duration::from_secs(10));
}

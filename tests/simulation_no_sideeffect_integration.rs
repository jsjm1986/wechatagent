//! Shadow 模拟零副作用红线集成测试:simulate_user_dialogue 跑完后
//! agent_send_outbox 与 conversation outbound 计数**不变**,且不调 MCP 发送。
//! 全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test simulation_no_sideeffect_integration -- --ignored`。
//!
//! ## 红线意义(P0):Shadow 模式复用真实 Reply+Review LLM 链,但发送阶段只输出
//! would_send,**绝不写 outbox / 不写 outbound 消息 / 不调 MCP**(simulation.rs:1-7 契约)。
//! 本测试钉死该边界:无论 LLM 决策 should_reply 与否,跑 simulation 前后 outbox 与
//! outbound 计数恒等。一旦 simulation 误接真实发送链,本测试立刻红。
#![cfg(test)]

mod common;

use std::collections::BTreeMap;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use serde_json::json;

use wechatagent::agent::simulate_user_dialogue;
use wechatagent::llm::ChatUsage;
use wechatagent::models::{AgentStatus, Contact};

use crate::common::TestApp;

fn managed_contact(ws: &str, acc: &str, wxid: &str) -> Contact {
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: wxid.to_string(),
        nickname: None,
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
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

async fn business_snapshot(app: &TestApp) -> BTreeMap<String, Vec<Document>> {
    let mut names = app
        .state
        .db
        .raw()
        .list_collection_names(None)
        .await
        .expect("list collections");
    names.sort();

    let mut snapshot = BTreeMap::new();
    for name in names {
        if matches!(name.as_str(), "llm_call_logs" | "migrations") {
            continue;
        }
        let mut docs: Vec<Document> = app
            .state
            .db
            .raw()
            .collection::<Document>(&name)
            .find(doc! {}, None)
            .await
            .unwrap_or_else(|error| panic!("find collection {name}: {error}"))
            .try_collect()
            .await
            .unwrap_or_else(|error| panic!("collect collection {name}: {error}"));
        docs.sort_by_key(|document| format!("{:?}|{:?}", document.get("_id"), document));
        snapshot.insert(name, docs);
    }
    snapshot
}

fn reply_decision_json() -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户刚开始接触，希望先了解彼此并说明当前最需要解决的问题。",
        "relationshipRead": "这是初次交流，关系尚在建立，适合用一个轻量问题继续了解。",
        "operationGoal": "自然回应并邀请客户说明当前最关心的问题，不做任何产品承诺。",
        "knowledgeNeedReason": "本轮只是建立基本上下文，不需要调用产品或业务知识。",
        "memoryUpdateReason": "当前没有形成需要持久化的新事实，仅保留本轮上下文即可。",
        "selfCritique": "回复应保持简短自然，只问一个问题，避免连续追问或提前推销。",
        "whyShouldReply": "客户主动发来消息，礼貌回应并询问一个轻量问题有助于继续交流。",
        "whySkipReply": "",
        "riskSelfCheck": "回复不包含价格、效果、隐私或未经验证的事实，也不作绝对承诺。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "new_contact",
        "shouldReply": true,
        "replyText": "你好，很高兴认识你。方便先说说你现在最想解决的问题吗？",
        "usedKnowledgeIds": [],
        "conversationMode": "casual_relationship",
        "conversationModeReason": "初次交流以自然建立关系和了解背景为主。"
    })
}

fn review_pass_json() -> serde_json::Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": 9,
            "emotionalValue": 8,
            "productAccuracy": 9,
            "boundaryPrivacySafety": 9,
            "relationshipProgress": 8,
            "conversionReadiness": 5,
            "pressureRisk": 1,
            "factRisk": 1
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "回复只做自然问候和轻量澄清，不包含产品事实。"
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "表达自然、边界安全，可以发送。",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true
    })
}

fn known_usage() -> ChatUsage {
    ChatUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        usage_known: true,
        ..Default::default()
    }
}

fn empty_projection_json() -> serde_json::Value {
    json!({
        "profileUpdate": null,
        "tags": [],
        "tagEvidenceTurns": [],
        "stageEvidenceTurns": [],
        "stageExplicitIntent": false,
        "bayesianObservations": [],
        "customerStage": null,
        "intentLevel": null,
        "domainSignals": {},
        "dimensionDisplayNames": {},
        "followUpPolicy": null,
        "profileAttributes": {},
        "nextBestAction": {},
        "objectionsDetected": [],
        "operatingMemoryUpdate": {},
        "memoryCandidates": [],
        "memoryWriteScore": 0,
        "consolidationNeeded": false,
        "memoryUpdate": "",
        "agentGeneratedSignals": []
    })
}

fn fact_projection_json(content: &str, evidence: &str) -> serde_json::Value {
    let mut projection = empty_projection_json();
    projection["memoryCandidates"] = json!([{
        "type": "fact",
        "content": content,
        "evidence": evidence,
        "importance": 8,
        "confidence": 9
    }]);
    projection["memoryWriteScore"] = json!(8);
    projection
}

fn revision_reply_json(reply_text: &str) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户在轻量交流中希望继续了解当前话题，回复只做自然承接。",
        "relationshipRead": "关系正常推进，当前不需要引入产品事实或交易承诺。",
        "operationGoal": "保持自然交流并只问一个必要问题。",
        "knowledgeNeedReason": "本轮不涉及产品能力或业务事实，无需知识库。",
        "memoryUpdateReason": "本轮没有需要持久化的新事实。",
        "selfCritique": "避免模板化和连续追问，保持口语自然。",
        "whyShouldReply": "客户主动发来消息，及时承接有助于保持对话。",
        "whySkipReply": "",
        "riskSelfCheck": "回复不包含未经核实的现实业务声明。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "new_contact",
        "shouldReply": true,
        "replyText": reply_text,
        "usedKnowledgeIds": [],
        "matchedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "本轮按自然承接模式回复，不主动引入业务话题。"
    })
}

fn revision_review_json(needs_revision: bool) -> serde_json::Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": if needs_revision { 5 } else { 9 },
            "emotionalValue": 8,
            "productAccuracy": 9,
            "boundaryPrivacySafety": 9,
            "pressureRisk": 1,
            "factRisk": 0
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "回复不包含产品能力或现实业务事实。"
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "语义安全，检查口语自然度。",
        "needsRevision": needs_revision,
        "revisionDirection": if needs_revision { "减少模板化表达，改成更自然的微信口吻。" } else { "" },
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": !needs_revision
    })
}

fn semantic_reply_json(
    reply_text: &str,
    speech_act: &str,
    subject: &str,
    assertion_status: &str,
    response_disposition: &str,
    conversation_mode: &str,
) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "根据完整上下文识别本轮言语行为，不按单个词或固定短语升级风险。",
        "relationshipRead": "当前关系正常，本轮只承接客户明确表达，不额外推进或制造压力。",
        "operationGoal": "准确回应当前 speech act，并避免把提问、否定、假设或引用改写成事实。",
        "knowledgeNeedReason": "候选回复不代表产品能力、价格、预约或业务政策已经成立。",
        "memoryUpdateReason": "本轮没有形成需要持久化的稳定客户事实。",
        "selfCritique": "保持自然微信口吻，只处理当前语义，不从词面做过度推断。",
        "whyShouldReply": "客户主动表达了当前意图，简短承接能保持上下文连贯并尊重其边界。",
        "whySkipReply": "",
        "riskSelfCheck": "候选只完成会话行为或提出澄清，不把现实业务事实表述为已确认。",
        "riskLevel": "low",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": false,
        "consolidationNeeded": false,
        "operationState": "new_contact",
        "shouldReply": true,
        "replyText": reply_text,
        "intentAnalysis": {
            "semanticAssessment": {
                "intent": "承接客户当前言语行为",
                "speechAct": speech_act,
                "subject": subject,
                "assertionStatus": assertion_status,
                "knowledgeNeed": "not_required",
                "responseDisposition": response_disposition,
                "semanticRisk": {
                    "content": "low",
                    "pressure": "low",
                    "boundary": "low",
                    "privacy": "low",
                    "confidence": 0.96
                },
                "claims": [],
                "reason": "完整语境表明候选没有把外部业务事实表述为已经确定。"
            }
        },
        "usedKnowledgeIds": [],
        "matchedKnowledgeIds": [],
        "conversationMode": conversation_mode,
        "conversationModeReason": "模式由当前完整语境决定，不按自然语言关键词切换。"
    })
}

fn semantic_claim_gate_pass_json(
    speech_act: &str,
    subject: &str,
    assertion_status: &str,
    response_disposition: &str,
) -> serde_json::Value {
    json!({
        "claimKinds": [],
        "claimsComplete": true,
        "semanticAssessment": {
            "speechAct": speech_act,
            "subject": subject,
            "assertionStatus": assertion_status,
            "knowledgeNeed": "not_required",
            "responseDisposition": response_disposition,
            "contentRisk": "low",
            "confidence": 0.97,
            "reason": "候选只完成当前会话行为，没有确认外部业务事实。"
        },
        "responseDisposition": response_disposition,
        "claims": [],
        "catalogCoverageComplete": true,
        "catalogClaims": [],
        "reason": "No externally verifiable business claim is asserted."
    })
}

/// 红线：完整 Shadow 链结束后，成本日志之外的数据库逐文档不变。
#[tokio::test]
#[ignore]
async fn simulation_has_no_business_side_effects() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = managed_contact(&ws, &acc, "wx_sim");

    // Seed a preference with an old timestamp. The live loader renews
    // last_used_at; Shadow must read the same row without touching it.
    app.state
        .db
        .raw()
        .collection::<Document>("knowledge_operator_memory")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": &acc,
                "operator_id": &acc,
                "kind": "preference",
                "content": "回复保持简洁",
                "created_at": DateTime::from_millis(1_000),
                "last_used_at": DateTime::from_millis(1_000),
                "expires_at": null,
            },
            None,
        )
        .await
        .expect("seed operator memory");

    let before = business_snapshot(&app).await;
    let started_at = DateTime::now();

    app.llm
        .push_response_with_usage(reply_decision_json(), known_usage());
    app.llm
        .push_response_with_usage(review_pass_json(), known_usage());
    app.llm
        .push_response_with_usage(common::independent_claim_gate_pass_json(), known_usage());
    app.llm
        .push_response_with_usage(empty_projection_json(), known_usage());

    let turns = simulate_user_dialogue(&app.state, contact, vec!["你好".to_string()])
        .await
        .expect("shadow simulation must complete");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].status, "would_send");

    let after = business_snapshot(&app).await;
    let changed_collections: Vec<String> = before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|name| before.get(*name) != after.get(*name))
        .map(|name| name.clone())
        .collect();
    if let Some(name) = changed_collections.first() {
        panic!(
            "Shadow changed business collections {changed_collections:?}; first={name}; before={:?}; after={:?}",
            before.get(name),
            after.get(name),
        );
    }

    let logs: Vec<Document> = app
        .state
        .db
        .raw()
        .collection::<Document>("llm_call_logs")
        .find(
            doc! {
                "workspace_id": &ws,
                "contact_wxid": "wx_sim",
                "created_at": { "$gte": started_at },
            },
            None,
        )
        .await
        .expect("query shadow llm logs")
        .try_collect()
        .await
        .expect("collect shadow llm logs");
    assert_eq!(
        logs.len(),
        4,
        "Reply、Review、ClaimGate、Projection 应各留一条成本日志"
    );
    assert!(logs
        .iter()
        .all(|log| log.get_str("run_mode") == Ok("shadow")));

    app.cleanup().await;
}

/// Shadow 必须执行与生产相同的 single-shot revision：首稿只因人味软闸触发改写，
/// 二稿重新走 Reviewer + ClaimGate，最终只能报告 would_send，并标记
/// `revision_applied_approved`，不能把首稿的 `revision_required` 暴露给评测层。
#[tokio::test]
#[ignore]
async fn simulation_runs_single_shot_revision_and_rechecks_candidate() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = managed_contact(&ws, &acc, "wx_sim_revision");

    app.llm
        .push_response(revision_reply_json("这是一版偏模板的回复。"));
    app.llm.push_response(revision_review_json(true));
    app.llm
        .push_response(common::independent_claim_gate_pass_json());
    app.llm.push_response(empty_projection_json());
    app.llm
        .push_response(revision_reply_json("收到，我先顺着你刚才的重点聊。"));
    app.llm.push_response(revision_review_json(false));
    app.llm
        .push_response(common::independent_claim_gate_pass_json());

    let turns = simulate_user_dialogue(&app.state, contact, vec!["你好".to_string()])
        .await
        .expect("shadow revision simulation must complete");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].status, "would_send");
    assert_eq!(
        turns[0]
            .review
            .get_str("finalReviewStatus")
            .unwrap_or_default(),
        "revision_applied_approved"
    );
    assert_eq!(
        app.llm.calls(),
        7,
        "首稿和二稿各应完整执行 Reply + Review + ClaimGate，最终再执行 Projection"
    );

    app.cleanup().await;
}

/// Unsupported open-world business facts must take the targeted rewrite path before finalize;
/// Shadow must then re-run Reviewer + ClaimGate on the rewritten candidate instead of reporting
/// a false hard safety block for a repairable draft.
#[tokio::test]
#[ignore]
async fn simulation_rewrites_unsupported_business_claim_before_finalize() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = managed_contact(&ws, &acc, "wx_sim_targeted_rewrite");
    let unsupported = "明天下午三点一定能安排";

    app.llm
        .push_response(revision_reply_json(&format!("放心，我们{}。", unsupported)));
    app.llm.push_response(revision_review_json(false));
    app.llm
        .push_response(common::independent_claim_gate_unsupported_business_json(
            unsupported,
        ));
    app.llm.push_response(revision_reply_json(
        "具体时间还要先核对清楚，我不先替你保证。",
    ));
    app.llm.push_response(revision_review_json(false));
    app.llm
        .push_response(common::independent_claim_gate_pass_json());
    app.llm.push_response(empty_projection_json());

    let turns = simulate_user_dialogue(&app.state, contact, vec!["明天下午能安排吗？".to_string()])
        .await
        .expect("targeted rewrite simulation must complete");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].status, "would_send");
    assert_eq!(
        app.llm.calls(),
        7,
        "定向改写后必须重新执行 Reply + Review + ClaimGate，最终再执行 Projection"
    );
    assert!(!turns[0].reply_text.contains(unsupported));
    assert_eq!(
        turns[0]
            .review
            .get_i64("unsupportedNonProductBusinessClaimCount")
            .unwrap_or(0),
        0
    );

    app.cleanup().await;
}

/// 多轮语义碰撞矩阵：自然语言里即使出现报价、保证、明天、翻倍、预约等词，服务端也
/// 不得恢复关键词硬门。每个可发送候选都必须保留 Reply Agent 结构化语义，并完整执行
/// Reviewer + 独立 ClaimGate；Shadow 仍不得写任何业务集合。
#[tokio::test]
#[ignore]
async fn simulation_semantic_matrix_runs_full_independent_chain_without_keyword_gates() {
    struct Case {
        inbound: &'static str,
        reply: &'static str,
        speech_act: &'static str,
        subject: &'static str,
        assertion_status: &'static str,
        response_disposition: &'static str,
        conversation_mode: &'static str,
    }

    let cases = [
        Case {
            inbound: "嗨，刚好想到你，就来打声招呼。",
            reply: "嗨，收到你的招呼。最近怎么样？",
            speech_act: "greeting",
            subject: "none",
            assertion_status: "not_applicable",
            response_disposition: "reply",
            conversation_mode: "casual_relationship",
        },
        Case {
            inbound: "这个话题先暂停，我忙完再决定要不要继续。",
            reply: "好，这个话题先停在这里。",
            speech_act: "statement",
            subject: "none",
            assertion_status: "not_applicable",
            response_disposition: "acknowledgement",
            conversation_mode: "boundary_protection",
        },
        Case {
            inbound: "你刚才说‘需要先核对’，依据来自哪里？",
            reply: "你问得对。你想核对的是哪一条结论？",
            speech_act: "question",
            subject: "customer",
            assertion_status: "interrogative",
            response_disposition: "clarify",
            conversation_mode: "value_exchange",
        },
        Case {
            inbound: "我想先梳理业务，不急着买，第一步该看什么？",
            reply: "可以先从目标和约束拆开看。你现在更想先理清哪一块？",
            speech_act: "question",
            subject: "customer",
            assertion_status: "interrogative",
            response_disposition: "clarify",
            conversation_mode: "value_exchange",
        },
        Case {
            inbound: "我不是说预约成功了，也没有要求你保证明天有位置。",
            reply: "明白，我们不把它当成预约确认，也不先做任何保证。",
            speech_act: "negated",
            subject: "business",
            assertion_status: "negated",
            response_disposition: "acknowledgement",
            conversation_mode: "boundary_protection",
        },
        Case {
            inbound: "假如报价减半、效果翻倍，这种前提下会怎么取舍？",
            reply: "这种前提先别当成事实，最好分别看预算约束和效果依据。你想先拆哪一边？",
            speech_act: "uncertain",
            subject: "general",
            assertion_status: "uncertain",
            response_disposition: "clarify",
            conversation_mode: "value_exchange",
        },
        Case {
            inbound: "有人原话是‘这个项目一定能翻倍’，我只是转述，不代表我认同。",
            reply: "单凭这句转述不能判断真假，得看它原本的条件和依据。",
            speech_act: "quoted",
            subject: "general",
            assertion_status: "quoted",
            response_disposition: "reply",
            conversation_mode: "value_exchange",
        },
    ];

    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = managed_contact(&ws, &acc, "wx_semantic_matrix");
    let before = business_snapshot(&app).await;

    for case in &cases {
        app.llm.push_response(semantic_reply_json(
            case.reply,
            case.speech_act,
            case.subject,
            case.assertion_status,
            case.response_disposition,
            case.conversation_mode,
        ));
        app.llm.push_response(review_pass_json());
        app.llm.push_response(semantic_claim_gate_pass_json(
            case.speech_act,
            case.subject,
            case.assertion_status,
            case.response_disposition,
        ));
        app.llm.push_response(empty_projection_json());
    }

    let turns = simulate_user_dialogue(
        &app.state,
        contact,
        cases.iter().map(|case| case.inbound.to_string()).collect(),
    )
    .await
    .expect("semantic matrix simulation must complete");

    assert_eq!(turns.len(), cases.len());
    assert_eq!(
        app.llm.calls(),
        cases.len() * 4,
        "每个可发送候选都必须执行 Reply + Reviewer + ClaimGate + Projection"
    );
    for (turn, case) in turns.iter().zip(&cases) {
        assert_eq!(turn.status, "would_send", "inbound={}", case.inbound);
        let assessment = turn
            .decision
            .get_document("intentAnalysis")
            .and_then(|intent| intent.get_document("semanticAssessment"))
            .expect("valid semantic assessment must survive promotion");
        assert_eq!(assessment.get_str("speechAct"), Ok(case.speech_act));
        assert_eq!(
            assessment.get_str("assertionStatus"),
            Ok(case.assertion_status)
        );
        assert_eq!(
            assessment.get_str("responseDisposition"),
            Ok(case.response_disposition)
        );
        assert_eq!(
            turn.review
                .get_document("claimAnalysis")
                .and_then(|analysis| analysis.get_bool("claimsComplete")),
            Ok(true)
        );
        assert!(!turn.review.get_array("risks").is_ok_and(|risks| {
            risks
                .iter()
                .any(|risk| risk.as_str() == Some("claim_gate_skipped_casual_low_risk"))
        }));
    }

    let after = business_snapshot(&app).await;
    assert_eq!(before, after, "多轮 Shadow 语义矩阵不得产生业务副作用");

    app.cleanup().await;
}

/// Projection is part of the simulated loop rather than a detached display artifact. A fact
/// extracted after one authorized turn must be present in the next turn's context pack, while the
/// durable business collections remain unchanged.
#[tokio::test]
#[ignore]
async fn simulation_carries_authorized_projection_into_the_next_turn() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = managed_contact(&ws, &acc, "wx_sim_projection_loop");
    let before = business_snapshot(&app).await;
    let remembered = "客户明确表示平时只有周末方便沟通";

    for (index, reply) in ["明白，我记下了。", "好，那我们按你的节奏来。"]
        .into_iter()
        .enumerate()
    {
        app.llm.push_response(semantic_reply_json(
            reply,
            "statement",
            "customer",
            "asserted",
            "acknowledgement",
            "casual_relationship",
        ));
        app.llm.push_response(review_pass_json());
        app.llm.push_response(semantic_claim_gate_pass_json(
            "statement",
            "customer",
            "asserted",
            "acknowledgement",
        ));
        app.llm.push_response(if index == 0 {
            fact_projection_json(remembered, "我平时只有周末方便聊")
        } else {
            empty_projection_json()
        });
    }

    let turns = simulate_user_dialogue(
        &app.state,
        contact,
        vec![
            "我平时只有周末方便聊".to_string(),
            "那就先这样，周末再说".to_string(),
        ],
    )
    .await
    .expect("multi-turn projection simulation must complete");

    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].memory_preview.get_str("status"), Ok("applied"));
    assert!(turns[1]
        .context_pack
        .get_array("recentFacts")
        .is_ok_and(|facts| facts.iter().any(|fact| fact.as_str() == Some(remembered))));
    assert_eq!(app.llm.calls(), 8);
    assert_eq!(
        before,
        business_snapshot(&app).await,
        "in-memory projection must not persist business state"
    );

    app.cleanup().await;
}

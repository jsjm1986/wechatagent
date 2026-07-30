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
        if name == "llm_call_logs" {
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
    assert_eq!(logs.len(), 3, "Reply、Review、ClaimGate 应各留一条成本日志");
    assert!(logs
        .iter()
        .all(|log| log.get_str("run_mode") == Ok("shadow")));

    app.cleanup().await;
}

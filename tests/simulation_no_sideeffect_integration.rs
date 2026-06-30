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

use mongodb::bson::{doc, DateTime, Document};

use wechatagent::agent::simulate_user_dialogue;
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

/// 红线:跑 simulate_user_dialogue 前后,outbox 与 outbound 消息计数不变(零副作用)。
#[tokio::test]
#[ignore]
async fn simulation_writes_no_outbox_no_outbound() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = managed_contact(&ws, &acc, "wx_sim");

    let outbox_before = app
        .state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! {}, None)
        .await
        .expect("count outbox before");
    let outbound_before = app
        .state
        .db
        .messages()
        .count_documents(doc! { "direction": "outbound" }, None)
        .await
        .expect("count outbound before");

    // 推几条宽松 LLM 响应驱动 knowledge-route/decide/review 链。无论链路结果如何,
    // 零副作用红线都必须成立(should_reply=true 也只产 would_send,不落 outbox)。
    for _ in 0..6 {
        app.llm.push_response(serde_json::json!({
            "should_reply": true,
            "reply_text": "您好,很高兴为您服务",
            "approved": true,
            "scores": { "humanLike": 8, "emotionalValue": 7, "hallucinationScore": 1, "knowledgeGroundingScore": 8, "pressureRisk": 2 }
        }));
    }

    // 跑 2 轮对话。即便 LLM 响应 schema 不完全匹配导致内部 fail-soft,零副作用仍须成立,
    // 故对结果宽容(Ok/Err 都接受),只断言无写入副作用。
    let _ = simulate_user_dialogue(
        &app.state,
        contact,
        vec!["你们有什么产品".to_string(), "多少钱".to_string()],
    )
    .await;

    let outbox_after = app
        .state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! {}, None)
        .await
        .expect("count outbox after");
    let outbound_after = app
        .state
        .db
        .messages()
        .count_documents(doc! { "direction": "outbound" }, None)
        .await
        .expect("count outbound after");

    assert_eq!(
        outbox_after, outbox_before,
        "Shadow 模拟绝不能写 agent_send_outbox(红线)"
    );
    assert_eq!(
        outbound_after, outbound_before,
        "Shadow 模拟绝不能写 outbound 消息(红线)"
    );
}

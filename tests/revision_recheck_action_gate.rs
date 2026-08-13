//! GATE-1：revision 改写后动作闸复检（缺陷 #8 空壳落实，2026-08-13）。
//!
//! 默认 `#[ignore]`，需 Docker（testcontainers MongoDB）。
//!
//! ## 锁定的不变量（GATE-1，gateway.rs revision 块 second_passed 分支）
//! revision 整条替换 `final_decision` 后，`operation_state` 可能迁入"禁止 reply"
//! 的态。初次动作闸只校验了改写前的 decision；gateway 在二次 finalize 通过后对
//! **改写后的** decision 复检 `apply_state_action_gate`——命中 forbidden 时：
//! - `final_review_status = "held_by_ai_policy"`；
//! - review.risks 追加 `state_action_policy_blocked`；
//! - 写 `agent_events kind="state_action_policy_blocked"`；
//! - **修订稿绝不入队 outbox**（下游统一拦截分支 fail-closed）。
//!
//! ## 驱动方式（确定性 mock LLM，全链真跑）
//! 复用 happy_path_run::autonomy_full_loop_with_revision 的六段编排（TestLlmGenerator
//! 按 schema 定向出队）：Reply#1 → Review#1（needsRevision+方向）→ ClaimGate#1 →
//! Reply#2（修订稿把 operationState 迁入 cooldown）→ Review#2（放行）→ ClaimGate#2。
//! `cooldown` 在默认状态机带 `allowFromAny: true`（prompts.rs 默认机），迁移合法，
//! 因此 `action_policy_state_key` 会采用 proposed 态查 policy；测试把 default
//! workspace 的 cooldown 现行 policy（bootstrap `reconcile_prompt_pack_state_policies`
//! 为每个 state 种入 current 行）改成 `forbidden: ["reply"]` 触发复检拦截。
//!
//! ## 客户回应保障占位的精确口径
//! `held_by_ai_policy` 不在 ACK 占位豁免清单（gateway.rs
//! `ACK_PLACEHOLDER_EXCLUDED_STATUSES`）：inbound 被拦后生产会补一条**确定性安抚
//! 占位**进 outbox（幂等键 `{source_event_id}#ack-placeholder`）。因此"零 outbox"
//! 的精确断言是：修订稿正文零入队；除 `#ack-placeholder` 外零行。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::handle_managed_message;
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};

const REVISED_REPLY: &str =
    "按修正方向收敛后的第二版回复：我们先把你最关心的场景确认下来，再谈节奏。";

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

/// Reply Agent 决策 JSON；`operation_state` 由调用方指定（修订稿迁入 cooldown）。
fn reply_decision_json(reply_text: &str, operation_state: &str) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户正在多方比较，节奏被打乱，需要先稳住优先级再谈时间与预算。",
        "relationshipRead": "对话仍在推进期，客户情绪平稳但决策压力上升，需要低压承接。",
        "operationGoal": "先确认客户最优先的场景，避免在未确认需求前给出节奏承诺。",
        "knowledgeNeedReason": "本轮只做节奏承接与需求澄清，不涉及产品能力，无需知识库。",
        "memoryUpdateReason": "本轮未出现需要持久化的新事实，保持既有画像即可。",
        "selfCritique": "注意不要给出未经核实的周期或价格数字，收敛信息密度。",
        "whyShouldReply": "客户主动追问节奏，正面承接可以降低对方决策焦虑并推进需求澄清。",
        "whySkipReply": "",
        "riskSelfCheck": "回复不含产品声明与数字承诺，压力与边界风险低。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": operation_state,
        "shouldReply": true,
        "replyText": reply_text,
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "客户在评估推进节奏，按顾问模式承接。",
    })
}

fn review_json(needs_revision: bool, revision_direction: &str) -> serde_json::Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": 8,
            "emotionalValue": 8,
            "productAccuracy": 8,
            "pressureRisk": 2,
            "boundaryPrivacySafety": 9,
            "factRisk": 1,
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "候选回复仅做节奏承接，不涉及产品事实。",
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "评审意见",
        "needsRevision": needs_revision,
        "revisionDirection": revision_direction,
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": !needs_revision,
    })
}

/// GATE-1：修订稿迁入 forbidden-reply 态 → held_by_ai_policy + 修订稿零入队。
#[tokio::test]
#[ignore]
async fn revision_into_forbidden_state_is_held() {
    let app = common::TestApp::start().await;
    let ws = "default".to_string();

    // bootstrap 已为默认状态机每个 state 种入唯一 current policy 行
    // （prompts.rs reconcile_prompt_pack_state_policies）；把 cooldown 的现行
    // policy 改成禁止 reply（cooldown 带 allowFromAny，proposed 迁移必然合法，
    // action gate 会按 proposed 态查本行）。
    let policy_update = app
        .state
        .db
        .operation_state_policies()
        .update_one(
            doc! {
                "workspace_id": &ws,
                "domain": "user_operations",
                "state_key": "cooldown",
                "current_version": true,
            },
            doc! { "$set": { "forbidden": ["reply"], "status": "active" } },
            None,
        )
        .await
        .expect("update cooldown policy");
    assert_eq!(
        policy_update.matched_count, 1,
        "default workspace 必须已 seed cooldown 的唯一 current policy（bootstrap 对账不变量）"
    );

    let contact = make_managed_contact("user_gate1_recheck");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(
        &contact,
        "msg_gate1_001",
        "你们后续节奏怎么安排？我这边有点着急。",
    );
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    // 六段编排：Reply#1（合法态）→ Review#1（要求 revision）→ ClaimGate#1
    // → Reply#2（修订稿迁入 cooldown = forbidden-reply 态）→ Review#2（放行）
    // → ClaimGate#2。修订二次 finalize 通过后由 GATE-1 复检拦下。
    app.llm.push_response(reply_decision_json(
        "先别急，我们把最关键的场景确认下来，节奏就清楚了。",
        "need_discovery",
    ));
    app.llm.push_response(review_json(
        true,
        "先承接客户的着急情绪，再确认最优先场景；不要给未经核实的时间承诺。",
    ));
    app.llm
        .push_response(common::independent_claim_gate_pass_json());
    app.llm
        .push_response(reply_decision_json(REVISED_REPLY, "cooldown"));
    app.llm.push_response(review_json(false, ""));
    app.llm
        .push_response(common::independent_claim_gate_pass_json());

    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");

    // 终态：held_by_ai_policy（GATE-1 复检把 revision_applied_approved 改判 held）。
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
        log.final_review_status, "held_by_ai_policy",
        "修订稿迁入 forbidden-reply 态必须被 GATE-1 复检改判 held_by_ai_policy，实际 {:?}",
        log.final_review_status
    );

    // risks 含 state_action_policy_blocked（review 文档随 run log 落库）。
    let risks: Vec<String> = log
        .review
        .get_array("risks")
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        risks.iter().any(|r| r == "state_action_policy_blocked"),
        "review.risks 必须含 state_action_policy_blocked，实际 {risks:?}"
    );

    // 审计事件：state_action_policy_blocked（operation_state=cooldown）。
    let event = app
        .state
        .db
        .events()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "kind": "state_action_policy_blocked",
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("query events")
        .expect("state_action_policy_blocked event exists");
    let details = event.details.expect("event details");
    assert_eq!(
        details.get_str("operation_state").unwrap_or_default(),
        "cooldown",
        "事件应记录命中 forbidden 的 proposed 态"
    );

    // outbox 精确口径：修订稿正文零入队；除客户回应保障占位（#ack-placeholder）
    // 外零行。
    use futures::TryStreamExt;
    let outbox_rows: Vec<wechatagent::models::OutboxEntry> = app
        .state
        .db
        .collection_agent_send_outbox()
        .find(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("query outbox")
        .try_collect()
        .await
        .expect("collect outbox");
    for row in &outbox_rows {
        assert_ne!(
            row.content, REVISED_REPLY,
            "被 GATE-1 拦下的修订稿绝不允许入队 outbox"
        );
        assert!(
            row.source_event_id.ends_with("#ack-placeholder"),
            "held 后唯一允许的入队是客户回应保障占位，实际 source_event_id={:?} content={:?}",
            row.source_event_id,
            row.content
        );
    }

    app.cleanup().await;
}

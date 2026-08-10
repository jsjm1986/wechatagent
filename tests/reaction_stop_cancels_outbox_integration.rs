//! 上线前全量测试第二波 Task 5：用户停止意图 → outbox 取消 串联集成测试。
//!
//! 命门在 `src/agent/reaction.rs:248` 的接线：
//! ```ignore
//! if outbox::outcome_signals_stop(&outcome_for_outbox) {
//!     outbox::cancel_for_contact_on_user_reaction(
//!         state,
//!         &contact.workspace_id,
//!         &contact.account_id,
//!         &contact.wxid,
//!     )
//! }
//! ```
//! 三段各自有测试（映射 `reaction.rs` a6_tests / 规则闸 `outbox.rs` tests /
//! 取消函数孤立 `outbox_integration.rs`），但**没有测试锁定完整串联**——
//! 喂 `record_user_reaction` 一个停止意图，pending outbox 真的被置 canceled。
//!
//! 本测试真调 `record_user_reaction`（而非直调 `cancel_for_contact_on_user_reaction`
//! 跳过停止意图判定），驱动 claim → analyze → outcome 映射 → outcome_signals_stop
//! → cancel 的完整链路，确保这条红线接线不被静默摘除。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）；CI 用
//! `cargo test --test reaction_stop_cancels_outbox_integration -- --ignored` 触发。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{enqueue, record_user_reaction, EnqueueRequest};
use wechatagent::models::{AgentDecisionReview, Contact, ConversationMessage, MessageDirection};

/// 构造一个 managed contact（reaction 路径 claim filter 不看 agent_status，但
/// 保持 Managed 与生产语义一致）。字段严格按 `src/models.rs:132` Contact 结构体
/// 全量填写（对齐 `tests/outbox_integration.rs:33` make_contact 范式）。
fn make_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("停止意图测试客户".to_string()),
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: wechatagent::models::AgentStatus::Managed,
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
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

/// 构造一条已发送的 decision_review，让 `record_user_reaction` 的 claim filter
/// （`reaction.rs:87` `status="sent"` + `outcome_status ∈ {null, "pending"}`）命中，
/// 从而进入 analyze → outcome → cancel 链路。字段严格按 `AgentDecisionReview`
/// 结构体填写（对齐 `tests/reaction_claim_lock.rs:13` pending_review 范式）。
fn sent_review(workspace: &str, account: &str, wxid: &str) -> AgentDecisionReview {
    AgentDecisionReview {
        id: Some(ObjectId::new()),
        workspace_id: workspace.to_string(),
        account_id: account.to_string(),
        contact_wxid: Some(wxid.to_string()),
        run_id: Some("run_stop_cancel_test".to_string()),
        inbound_message_id: None,
        reply_text: Some("上次已回复的内容".to_string()),
        approved: true,
        scores: Document::new(),
        formula_breakdown: Document::new(),
        risks: Vec::new(),
        rewrite_instruction: None,
        review_summary: None,
        playbook_id: None,
        playbook_version: None,
        used_knowledge_ids: Vec::new(),
        prompt_versions: Document::new(),
        operation_state: None,
        next_best_action: Document::new(),
        context_pack_snapshot: Document::new(),
        domain_config_snapshot: Document::new(),
        runtime_parameters_snapshot: Document::new(),
        send_gateway_result: Document::new(),
        // 关键：null/pending 才让 claim filter 命中（否则链路提前 return Ok）。
        outcome_status: Some("pending".to_string()),
        reaction_analysis: Document::new(),
        reaction_claimed_at: None,
        reaction_claim_token: None,
        reaction_claim_generation: 0,
        source_task_id: None,
        source_task_claim_token: None,
        reviewer_misjudge_signal: None,
        expected_text_segments: 0,
        // 关键：claim filter 要求 status="sent"。
        status: "sent".to_string(),
        created_at: DateTime::now(),
    }
}

/// 入队一条 pending outbox（同 account_id + contact_wxid），source_event_id 各异
/// 以规避 idempotency_key 撞键。返回入队结果供调用方断言 Created。
async fn enqueue_pending(
    state: &wechatagent::routes::AppState,
    contact: &Contact,
    source_event_id: &str,
    content: &str,
) -> ObjectId {
    let outcome = enqueue(
        state,
        EnqueueRequest {
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            run_id: "run_stop_cancel_test".to_string(),
            decision_id: None,
            source_event_id: source_event_id.to_string(),
            source_kind: "inbound_message".to_string(),
            content: content.to_string(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await
    .expect("enqueue pending outbox");
    match outcome {
        wechatagent::agent::EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    }
}

#[tokio::test]
#[ignore]
async fn record_user_reaction_stop_cancels_pending_outbox() {
    let app = common::TestApp::start().await;
    let contact = make_contact("user_stop_cancel");

    // seed contact（push_intent_trajectory_entry 会 update_one 该 contact；缺行只是
    // best-effort warn，不影响取消断言，但 seed 让链路更贴近生产）。
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // seed 一条已发送 review，让 claim filter 命中。
    let review = sent_review("default", "default", "user_stop_cancel");
    app.state
        .db
        .decision_reviews()
        .insert_one(&review, None)
        .await
        .expect("insert sent review");

    // seed 2 条 pending outbox（同 account+contact，source_event_id 各异避免撞键）。
    let outbox_a = enqueue_pending(&app.state, &contact, "evt_a", "待发送内容 A").await;
    let outbox_b = enqueue_pending(&app.state, &contact, "evt_b", "待发送内容 B").await;

    // mock LLM：analyze_user_reaction 调 generate_agent_json 恰一次（reaction.rs:335），
    // 返回 stopRequested=true → reaction_outcome_status_with_polarity 映射为
    // user_replied_stop_requested（reaction.rs:369）→ outcome_signals_stop 命中。
    app.llm.push_response(json!({ "stopRequested": true }));

    // 构造一条入站消息（用户明确表达停止），真调 record_user_reaction 驱动完整串联。
    let inbound = ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: "user_stop_cancel".to_string(),
        message_id: Some("inbound_stop_1".to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "最近有点烦，先缓一缓".to_string(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    };

    record_user_reaction(&app.state, &contact, &inbound)
        .await
        .expect("record_user_reaction should succeed");

    // 断言：LLM 恰被调用一次（analyze_user_reaction 单次 generate_agent_json）。
    assert_eq!(
        app.llm.calls(),
        1,
        "analyze_user_reaction 应恰调用一次 generate_agent_json"
    );

    // 断言：两条 pending outbox 都被置 canceled + cancel_reason=user_reaction_stop_requested。
    let collection = app.state.db.collection_agent_send_outbox();
    for (label, id) in [("A", outbox_a), ("B", outbox_b)] {
        let entry = collection
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("query outbox entry")
            .unwrap_or_else(|| panic!("outbox {label} should exist"));
        assert_eq!(
            entry.status, "canceled",
            "outbox {label} 应被用户停止意图取消（status=canceled），实际 {}",
            entry.status
        );
        assert_eq!(
            entry.cancel_reason.as_deref(),
            Some("user_reaction_stop_requested"),
            "outbox {label} cancel_reason 应为 user_reaction_stop_requested，实际 {:?}",
            entry.cancel_reason
        );
    }
}

#[tokio::test]
#[ignore]
async fn deterministic_stop_needs_no_review_or_llm_and_persists_dispatch_barrier() {
    let app = common::TestApp::start().await;
    let contact = make_contact("user_deterministic_stop");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outbox_id = enqueue_pending(
        &app.state,
        &contact,
        "evt_deterministic_stop",
        "这条消息绝不能越过远端边界",
    )
    .await;
    let inbound = ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some("inbound_deterministic_stop".to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "请不要再联系我，停止给我发消息。".to_string(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    };

    record_user_reaction(&app.state, &contact, &inbound)
        .await
        .expect("deterministic stop should succeed without a prior review");
    assert_eq!(
        app.llm.calls(),
        0,
        "explicit stop must never depend on LLM availability"
    );

    let stored_contact = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact.id }, None)
        .await
        .expect("load contact")
        .expect("contact exists");
    assert!(
        stored_contact
            .cooldown_until
            .is_some_and(|until| until.timestamp_millis() > DateTime::now().timestamp_millis()),
        "explicit stop must persist a restart-safe cooldown"
    );
    assert_eq!(
        stored_contact
            .operation_policy
            .get_bool("explicitStopRequested")
            .ok(),
        Some(true)
    );

    let entry = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("load outbox")
        .expect("outbox exists");
    assert_eq!(entry.status, "canceled");
    assert_eq!(
        entry.cancel_reason.as_deref(),
        Some("user_reaction_stop_requested")
    );
}

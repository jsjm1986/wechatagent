//! Reaction claim ownership regressions.
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。
//! 所有 claim / reclaim / finalize 都走 `record_user_reaction` 生产入口；测试只在
//! ABA 用例中回拨 `reaction_claimed_at`，模拟 worker 超时，不复制 Mongo 状态机。

mod common;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::{json, Value};
use tokio::sync::{watch, Notify};
use wechatagent::agent::{enqueue, record_user_reaction, EnqueueOutcome, EnqueueRequest};
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{ChatUsage, LlmJsonResult, LlmProvider};
use wechatagent::models::{
    AgentDecisionReview, AgentStatus, Contact, ConversationMessage, MessageDirection,
};

fn pending_review(workspace: &str, account: &str, wxid: &str) -> AgentDecisionReview {
    AgentDecisionReview {
        id: Some(ObjectId::new()),
        workspace_id: workspace.to_string(),
        account_id: account.to_string(),
        contact_wxid: Some(wxid.to_string()),
        run_id: Some("run_test".to_string()),
        inbound_message_id: None,
        reply_text: Some("hi".to_string()),
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
        outcome_status: Some("pending".to_string()),
        reaction_analysis: Document::new(),
        reaction_claimed_at: None,
        reaction_claim_token: None,
        reaction_claim_generation: 0,
        source_task_id: None,
        source_task_claim_token: None,
        reviewer_misjudge_signal: None,
        expected_text_segments: 0,
        status: "sent".to_string(),
        created_at: DateTime::now(),
    }
}

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("reaction fencing test".to_string()),
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

fn inbound(wxid: &str, message_id: &str, content: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: wxid.to_string(),
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

struct BlockingReactionProvider {
    call_count: AtomicUsize,
    reached: watch::Sender<usize>,
    release_first: Notify,
}

impl BlockingReactionProvider {
    fn new() -> (Arc<Self>, watch::Receiver<usize>) {
        let (reached, receiver) = watch::channel(0usize);
        (
            Arc::new(Self {
                call_count: AtomicUsize::new(0),
                reached,
                release_first: Notify::new(),
            }),
            receiver,
        )
    }

    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    fn release_first(&self) {
        self.release_first.notify_one();
    }

    async fn next_result(&self) -> AppResult<LlmJsonResult> {
        let call = self.call_count.fetch_add(1, Ordering::SeqCst);
        let _ = self.reached.send(call + 1);
        let value = match call {
            0 => {
                self.release_first.notified().await;
                json!({
                    "stopRequested": true,
                    "speechAct": "statement",
                    "assertionStatus": "asserted",
                    "subject": "customer",
                    "confidence": 0.95
                })
            }
            1 => json!({
                "buyingSignal": true,
                "speechAct": "request",
                "assertionStatus": "requested",
                "subject": "business",
                "confidence": 0.95
            }),
            other => {
                return Err(AppError::External(format!(
                    "unexpected reaction LLM call index {other}"
                )))
            }
        };
        Ok(LlmJsonResult {
            value,
            usage: ChatUsage::default(),
            latency_ms: 0,
            model: "blocking-reaction-test".to_string(),
            retry_count: 0,
        })
    }
}

#[async_trait]
impl LlmProvider for BlockingReactionProvider {
    async fn generate_json(&self, _system: &str, _user: &str) -> AppResult<Value> {
        Ok(self.next_result().await?.value)
    }

    async fn generate_json_with_usage(
        &self,
        _system: &str,
        _user: &str,
    ) -> AppResult<LlmJsonResult> {
        self.next_result().await
    }
}

async fn wait_for_provider_calls(receiver: &mut watch::Receiver<usize>, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while *receiver.borrow() < expected {
            receiver
                .changed()
                .await
                .expect("blocking reaction provider still alive");
        }
    })
    .await
    .expect("reaction LLM call did not start in time");
}

async fn seed_contact_and_review(
    state: &wechatagent::routes::AppState,
    wxid: &str,
) -> (Contact, ObjectId) {
    let contact = managed_contact(wxid);
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    let review = pending_review("default", "default", wxid);
    let review_id = review.id.expect("review id");
    state
        .db
        .decision_reviews()
        .insert_one(&review, None)
        .await
        .expect("insert pending review");
    (contact, review_id)
}

#[tokio::test]
#[ignore]
async fn reaction_redline_concurrent_entry_cannot_start_second_analysis() {
    let app = common::TestApp::start().await;
    let (provider, mut reached) = BlockingReactionProvider::new();
    let state = common::rebuild_app_state_with_real_llm(
        &app,
        provider.clone(),
        "http://test-mcp.invalid".to_string(),
    );
    let (contact, review_id) = seed_contact_and_review(&state, "reaction_concurrent").await;

    let first_state = state.clone();
    let first_contact = contact.clone();
    let first = tokio::spawn(async move {
        record_user_reaction(
            &first_state,
            &first_contact,
            &inbound("reaction_concurrent", "inbound-first", "先处理这一条"),
        )
        .await
    });
    wait_for_provider_calls(&mut reached, 1).await;

    record_user_reaction(
        &state,
        &contact,
        &inbound("reaction_concurrent", "inbound-second", "并发到达的第二条"),
    )
    .await
    .expect("second production entry returns without stealing active claim");
    assert_eq!(
        provider.calls(),
        1,
        "active claim must suppress a second LLM call"
    );

    provider.release_first();
    first
        .await
        .expect("first reaction task join")
        .expect("first reaction result");
    let stored = state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": review_id }, None)
        .await
        .expect("query review")
        .expect("review exists");
    assert_eq!(
        stored.outcome_status.as_deref(),
        Some("user_replied_stop_requested")
    );
    assert_eq!(stored.reaction_claim_generation, 1);
    assert!(stored.reaction_claim_token.is_none());

    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn reaction_redline_stale_owner_cannot_overwrite_or_cancel_after_reclaim() {
    let app = common::TestApp::start().await;
    let (provider, mut reached) = BlockingReactionProvider::new();
    let mut state = common::rebuild_app_state_with_real_llm(
        &app,
        provider.clone(),
        "http://test-mcp.invalid".to_string(),
    );
    state.config.reaction_analysis_claim_timeout_seconds = 1;
    let wxid = "reaction_reclaim_aba";
    let (contact, review_id) = seed_contact_and_review(&state, wxid).await;

    let outbox_id = match enqueue(
        &state,
        EnqueueRequest {
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: wxid.to_string(),
            run_id: "reaction-reclaim-outbox".to_string(),
            decision_id: None,
            source_event_id: "reaction-reclaim-source".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "must survive stale stop result".to_string(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await
    .expect("enqueue pending outbox")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected created outbox, got {other:?}"),
    };

    let stale_state = state.clone();
    let stale_contact = contact.clone();
    let stale = tokio::spawn(async move {
        record_user_reaction(
            &stale_state,
            &stale_contact,
            &inbound(wxid, "inbound-stale", "最近消息有点频繁，先缓一缓"),
        )
        .await
    });
    wait_for_provider_calls(&mut reached, 1).await;

    let first_claim = state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": review_id }, None)
        .await
        .expect("query first claim")
        .expect("review exists");
    let first_token = first_claim
        .reaction_claim_token
        .clone()
        .expect("first claim has token");
    assert_eq!(first_claim.outcome_status.as_deref(), Some("analyzing"));
    assert_eq!(first_claim.reaction_claim_generation, 1);

    // Fault injection only: make the current lease old. The second invocation performs the
    // production reclaim and claim transitions itself.
    state
        .db
        .decision_reviews()
        .update_one(
            doc! { "_id": review_id, "reaction_claim_token": &first_token },
            doc! { "$set": { "reaction_claimed_at": DateTime::from_millis(0) } },
            None,
        )
        .await
        .expect("backdate first reaction claim");

    record_user_reaction(
        &state,
        &contact,
        &inbound(wxid, "inbound-current", "我想继续了解"),
    )
    .await
    .expect("new owner completes through production entry");
    wait_for_provider_calls(&mut reached, 2).await;

    let current = state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": review_id }, None)
        .await
        .expect("query current outcome")
        .expect("review exists");
    assert_eq!(
        current.outcome_status.as_deref(),
        Some("user_replied_buying_signal")
    );
    assert_eq!(current.reaction_claim_generation, 2);
    assert!(current.reaction_claim_token.is_none());

    provider.release_first();
    stale
        .await
        .expect("stale reaction task join")
        .expect("stale reaction returns without committing");

    let final_review = state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": review_id }, None)
        .await
        .expect("query final review")
        .expect("review exists");
    assert_eq!(
        final_review.outcome_status.as_deref(),
        Some("user_replied_buying_signal"),
        "stale stop result must not overwrite the current owner"
    );
    assert_eq!(final_review.reaction_claim_generation, 2);
    assert!(final_review.reviewer_misjudge_signal.is_none());

    let stored_contact = state
        .db
        .contacts()
        .find_one(doc! { "_id": contact.id }, None)
        .await
        .expect("query contact")
        .expect("contact exists");
    assert_eq!(
        stored_contact.intent_trajectory.len(),
        1,
        "only the current owner may append trajectory"
    );
    assert_eq!(
        stored_contact.intent_trajectory[0].intent,
        "user_replied_buying_signal"
    );

    let outbox = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query outbox")
        .expect("outbox exists");
    assert_eq!(
        outbox.status, "pending",
        "stale stop result must not cancel pending delivery intents"
    );
    assert_eq!(provider.calls(), 2);

    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn structured_buying_signal_is_claim_scoped_and_model_driven() {
    let app = common::TestApp::start().await;

    // A current AI reaction verdict drives the outcome through the production claim/CAS path;
    // no text keyword or deterministic buying floor is consulted.
    let (buyer, buyer_review_id) =
        seed_contact_and_review(&app.state, "reaction_explicit_buyer").await;
    app.llm.push_response(json!({
        "buyingSignal": true,
        "speechAct": "request",
        "assertionStatus": "requested",
        "subject": "business",
        "confidence": 0.95
    }));
    record_user_reaction(
        &app.state,
        &buyer,
        &inbound(
            "reaction_explicit_buyer",
            "inbound-explicit-buy",
            "可以现在就报名付款吗？我要买",
        ),
    )
    .await
    .expect("structured buying reaction");
    assert_eq!(
        app.llm.calls(),
        1,
        "buying outcome must come from the AI reaction verdict"
    );
    let buyer_review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": buyer_review_id }, None)
        .await
        .expect("read buyer review")
        .expect("buyer review exists");
    assert_eq!(
        buyer_review.outcome_status.as_deref(),
        Some("user_replied_buying_signal")
    );
    assert_eq!(buyer_review.reaction_claim_generation, 1);
    assert!(buyer_review
        .reaction_analysis
        .get("deterministic")
        .is_none());
    assert!(buyer_review.reaction_analysis.get("dealVerified").is_none());
    assert!(buyer_review
        .reaction_analysis
        .get("paymentVerified")
        .is_none());

    // No sent predecessor means there is nothing to classify. The phrase alone must not create a
    // review, trajectory, deal, or additional model call.
    let no_predecessor = managed_contact("reaction_buy_without_predecessor");
    app.state
        .db
        .contacts()
        .insert_one(&no_predecessor, None)
        .await
        .expect("insert no-predecessor contact");
    record_user_reaction(
        &app.state,
        &no_predecessor,
        &inbound(
            "reaction_buy_without_predecessor",
            "inbound-buy-no-review",
            "我要买，现在付款",
        ),
    )
    .await
    .expect("no predecessor is a no-op");
    assert_eq!(app.llm.calls(), 1);
    assert_eq!(
        app.state
            .db
            .decision_reviews()
            .count_documents(
                doc! { "contact_wxid": "reaction_buy_without_predecessor" },
                None,
            )
            .await
            .expect("count no-predecessor reviews"),
        0
    );
    let no_predecessor_after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": no_predecessor.id }, None)
        .await
        .expect("read no-predecessor contact")
        .expect("no-predecessor contact exists");
    assert!(no_predecessor_after.intent_trajectory.is_empty());

    // Negated language stays on the model path instead of being hard-coded as a buying signal.
    let (negated, negated_review_id) =
        seed_contact_and_review(&app.state, "reaction_negated_buyer").await;
    app.llm.push_response(json!({
        "outcomeStatus": "user_replied_continue_exploring",
        "speechAct": "negated",
        "assertionStatus": "negated",
        "subject": "customer",
        "confidence": 0.95
    }));
    record_user_reaction(
        &app.state,
        &negated,
        &inbound(
            "reaction_negated_buyer",
            "inbound-negated-buy",
            "我先不买，也不要帮我下单",
        ),
    )
    .await
    .expect("negated buying reaction uses model");
    assert_eq!(app.llm.calls(), 2);
    let negated_review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": negated_review_id }, None)
        .await
        .expect("read negated review")
        .expect("negated review exists");
    assert_eq!(
        negated_review.outcome_status.as_deref(),
        Some("user_replied_continue_exploring")
    );
    assert_ne!(
        negated_review.reaction_analysis.get_bool("deterministic"),
        Ok(true)
    );

    // The complete emotional template disables transaction facts. The same literal words must
    // remain domain-modelled and consume the queued LLM response rather than the sales floor.
    common::roleplay_fixtures::seed_emotional_companion_profile_in_workspace(&app, "default").await;
    let (companion, companion_review_id) =
        seed_contact_and_review(&app.state, "reaction_companion_buy_words").await;
    app.llm.push_response(json!({
        "outcomeStatus": "user_emotion_opened_up",
        "speechAct": "statement",
        "assertionStatus": "asserted",
        "subject": "customer",
        "confidence": 0.95
    }));
    record_user_reaction(
        &app.state,
        &companion,
        &inbound(
            "reaction_companion_buy_words",
            "inbound-companion-buy-words",
            "我要买，现在付款",
        ),
    )
    .await
    .expect("non-transaction profile uses model semantics");
    assert_eq!(app.llm.calls(), 3);
    let companion_review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": companion_review_id }, None)
        .await
        .expect("read companion review")
        .expect("companion review exists");
    assert_eq!(
        companion_review.outcome_status.as_deref(),
        Some("user_emotion_opened_up")
    );
    assert_ne!(
        companion_review.reaction_analysis.get_bool("deterministic"),
        Ok(true)
    );

    app.cleanup().await;
}

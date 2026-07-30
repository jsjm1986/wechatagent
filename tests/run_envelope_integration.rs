//! agent-autonomy-loop W1 / Task 2.6：Run Envelope 集成测试。
//!
//! 覆盖纯单元测试无法验证、必须真实 MongoDB 才能断言的 4 条不变量
//! （见 requirements.md R0.10）：
//!
//! 1. 入口写信封先于任何 LLM 调用 —— mock LLM 抛异常前 lifecycle 已 = "started"。
//! 2. 同 `run_id` 二次 `insert_one` 触发 DuplicateKey（验证 R0.2 禁止 re-insert）。
//! 3. `update_one` 在不存在 envelope 时走兜底 `insert` + 写
//!    `agent_events kind="run_envelope_recovered_via_insert"`（R0.2 兜底路径）。
//! 4. Reply Agent panic 后 lifecycle 终态 = `failed_before_decision`，
//!    `error_summary` 非空（R0.6 panic-hook + catch_unwind 包装层语义）。
//!
//! 全部默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）；CI 用
//! `cargo test --test run_envelope_integration -- --ignored` 触发。
//!
mod common;

use std::{
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use futures::FutureExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::Value;
use wechatagent::agent::handle_managed_message;
use wechatagent::agent::run_envelope::{
    update_run_envelope_terminal, write_run_envelope_started, AgentRunLogTerminalFields,
    EVENT_RUN_ENVELOPE_RECOVERED_VIA_INSERT, LIFECYCLE_FAILED_AFTER_DECISION,
    LIFECYCLE_FAILED_BEFORE_DECISION, LIFECYCLE_RUNNING, LIFECYCLE_STARTED,
    SOURCE_KIND_INBOUND_MESSAGE,
};
use wechatagent::db::Database;
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{ChatUsage, LlmJsonResult, LlmProvider};
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};

#[derive(Clone, Copy)]
enum ProbeOutcome {
    Error,
    Panic,
}

struct EnvelopeOrderProbeLlm {
    db: Database,
    source_event_id: String,
    saw_started: Arc<AtomicBool>,
    outcome: ProbeOutcome,
}

impl EnvelopeOrderProbeLlm {
    async fn observe_then_stop(&self) -> AppResult<()> {
        let saw_started = self
            .db
            .agent_run_logs()
            .find_one(
                doc! {
                    "source_event_id": &self.source_event_id,
                    "lifecycle": LIFECYCLE_STARTED,
                },
                None,
            )
            .await?
            .is_some();
        self.saw_started.store(saw_started, Ordering::SeqCst);
        match self.outcome {
            ProbeOutcome::Error => Err(AppError::External("probe llm failure".to_string())),
            ProbeOutcome::Panic => panic!("probe llm panic"),
        }
    }
}

#[async_trait]
impl LlmProvider for EnvelopeOrderProbeLlm {
    async fn generate_json(&self, _system: &str, _user: &str) -> AppResult<Value> {
        self.observe_then_stop().await?;
        unreachable!("probe provider always stops")
    }

    async fn generate_json_with_usage(
        &self,
        _system: &str,
        _user: &str,
    ) -> AppResult<LlmJsonResult> {
        self.observe_then_stop().await?;
        unreachable!("probe provider always stops")
    }
}

struct DecisionThenPanicProbeLlm {
    db: Database,
    source_event_id: String,
    call_count: AtomicUsize,
    saw_running: Arc<AtomicBool>,
}

impl DecisionThenPanicProbeLlm {
    async fn next_value(&self) -> AppResult<Value> {
        let call = self.call_count.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            return Ok(serde_json::json!({
                "decisionPhase": "final",
                "userUnderstanding": "客户希望了解方案，当前需求清晰。",
                "relationshipRead": "客户处于初步咨询阶段，沟通氛围正常。",
                "operationGoal": "先回应核心问题并确认下一步需求。",
                "knowledgeNeedReason": "当前回复不需要额外产品事实。",
                "memoryUpdateReason": "本轮没有必须写入长期记忆的新事实。",
                "selfCritique": "保持简洁，不做未经验证的承诺。",
                "whyShouldReply": "客户提出了明确问题，需要及时回应。",
                "whySkipReply": "",
                "riskSelfCheck": "不涉及价格、效果或隐私承诺。",
                "riskLevel": "medium",
                "knowledgeNeed": "not_required",
                "runMode": "fast_chat",
                "autonomyMode": "auto",
                "needsReview": true,
                "consolidationNeeded": false,
                "operationState": "need_discovery",
                "shouldReply": true,
                "replyText": "可以，我先按你的场景梳理一下方案。",
                "usedKnowledgeIds": [],
                "conversationMode": "consultative",
                "conversationModeReason": "客户正在咨询方案，采用顾问式回应。"
            }));
        }

        let saw_running = self
            .db
            .agent_run_logs()
            .find_one(
                doc! {
                    "source_event_id": &self.source_event_id,
                    "lifecycle": LIFECYCLE_RUNNING,
                },
                None,
            )
            .await?
            .is_some();
        self.saw_running.store(saw_running, Ordering::SeqCst);
        panic!("probe review panic after decision")
    }
}

#[async_trait]
impl LlmProvider for DecisionThenPanicProbeLlm {
    async fn generate_json(&self, _system: &str, _user: &str) -> AppResult<Value> {
        self.next_value().await
    }

    async fn generate_json_with_usage(
        &self,
        _system: &str,
        _user: &str,
    ) -> AppResult<LlmJsonResult> {
        Ok(LlmJsonResult {
            value: self.next_value().await?,
            usage: ChatUsage::default(),
            latency_ms: 0,
            model: "decision-then-panic-probe".to_string(),
            retry_count: 0,
        })
    }
}

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("信封测试联系人".to_string()),
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

fn inbound(contact: &Contact, message_id: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some(message_id.to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "请介绍一下方案".to_string(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    }
}

async fn state_with_probe(
    app: &common::TestApp,
    source_event_id: &str,
    outcome: ProbeOutcome,
) -> (wechatagent::routes::AppState, Arc<AtomicBool>) {
    let saw_started = Arc::new(AtomicBool::new(false));
    let probe: Arc<dyn LlmProvider> = Arc::new(EnvelopeOrderProbeLlm {
        db: app.state.db.clone(),
        source_event_id: source_event_id.to_string(),
        saw_started: saw_started.clone(),
        outcome,
    });
    let state =
        common::rebuild_app_state_with_real_llm(app, probe, app.state.config.mcp_base_url.clone());
    (state, saw_started)
}

#[tokio::test]
#[ignore]
async fn envelope_started_written_before_any_llm_call() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("wxid_envelope_order");
    let inbound = inbound(&contact, "evt_inbound_order");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");
    let (state, saw_started) =
        state_with_probe(&app, "evt_inbound_order", ProbeOutcome::Error).await;

    let result = handle_managed_message(&state, contact.clone(), &inbound).await;
    assert!(result.is_err(), "probe LLM failure must propagate");
    assert!(
        saw_started.load(Ordering::SeqCst),
        "LLM invocation must observe the started envelope already persisted"
    );

    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "source_event_id": "evt_inbound_order" }, None)
        .await
        .expect("query agent_run_logs")
        .expect("envelope record present");

    assert_eq!(log.lifecycle, LIFECYCLE_FAILED_BEFORE_DECISION);
    assert_eq!(log.status, "internal_error");
    assert_eq!(log.source_event_id, "evt_inbound_order");
    assert_eq!(log.source_kind, SOURCE_KIND_INBOUND_MESSAGE);
    assert!(log
        .error_summary
        .as_deref()
        .unwrap_or_default()
        .contains("probe llm failure"));
    let count = app
        .state
        .db
        .agent_run_logs()
        .count_documents(doc! { "source_event_id": "evt_inbound_order" }, None)
        .await
        .expect("count run logs");
    assert_eq!(count, 1, "terminal write must update the original envelope");
}

#[tokio::test]
#[ignore]
async fn same_run_id_second_insert_triggers_duplicate_key_error() {
    // R0.2 / R0.10.e：unique index 阻断同 run_id 重复 insert。
    let app = common::TestApp::start().await;

    let run_id = "run_envelope_dup_key_test";
    write_run_envelope_started(
        &app.state.db,
        run_id,
        &app.state.config.default_workspace_id,
        &app.state.config.default_account_id,
        Some("wxid_test"),
        "evt_dup_001",
        SOURCE_KIND_INBOUND_MESSAGE,
        "reply",
    )
    .await
    .expect("first insert SHALL succeed");

    // 第二次 insert SHALL 因 unique(run_id) 触发 DuplicateKey 错误。
    let result = write_run_envelope_started(
        &app.state.db,
        run_id,
        &app.state.config.default_workspace_id,
        &app.state.config.default_account_id,
        Some("wxid_test"),
        "evt_dup_002",
        SOURCE_KIND_INBOUND_MESSAGE,
        "reply",
    )
    .await;

    assert!(
        result.is_err(),
        "同 run_id 二次 insert SHALL 失败（DuplicateKey）"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.to_lowercase().contains("duplicate") || err_msg.to_lowercase().contains("e11000"),
        "错误信息 SHALL 含 duplicate 关键字, err={}",
        err_msg
    );
}

#[tokio::test]
#[ignore]
async fn update_one_falls_back_to_insert_with_recovery_event() {
    // R0.2 兜底路径：update_run_envelope_terminal 命中 matched_count == 0 时，
    // SHALL 走单次 insert 兜底 + 写 agent_events kind="run_envelope_recovered_via_insert"。
    let app = common::TestApp::start().await;

    let run_id = "run_envelope_recovery_test";
    let fields = AgentRunLogTerminalFields {
        lifecycle: Some("completed".to_string()),
        final_review_status: Some("approved".to_string()),
        autonomy_mode: Some("auto".to_string()),
        ..Default::default()
    };

    update_run_envelope_terminal(&app.state.db, run_id, fields)
        .await
        .expect("update_run_envelope_terminal SHALL succeed via insert fallback");

    // 兜底 insert 后应能通过 run_id 找到记录。
    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "run_id": run_id }, None)
        .await
        .expect("query agent_run_logs")
        .expect("envelope recovered via insert");
    assert_eq!(log.lifecycle, "completed");
    assert_eq!(log.final_review_status, "approved");
    assert_eq!(log.autonomy_mode, "auto");

    // agent_events 中 SHALL 留下 recovery 事件。
    let event = app
        .state
        .db
        .events()
        .find_one(
            doc! {
                "kind": EVENT_RUN_ENVELOPE_RECOVERED_VIA_INSERT,
                "details.run_id": run_id,
            },
            None,
        )
        .await
        .expect("query agent_events")
        .expect("recovery event present");
    assert_eq!(event.kind, EVENT_RUN_ENVELOPE_RECOVERED_VIA_INSERT);
    assert_eq!(event.status, "warning");
}

#[tokio::test]
#[ignore]
async fn panic_in_pipeline_marks_lifecycle_failed_before_decision() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("wxid_envelope_panic");
    let inbound = inbound(&contact, "evt_inbound_panic");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");
    let (state, saw_started) =
        state_with_probe(&app, "evt_inbound_panic", ProbeOutcome::Panic).await;

    let result = AssertUnwindSafe(handle_managed_message(&state, contact, &inbound))
        .catch_unwind()
        .await;
    assert!(result.is_err(), "gateway must preserve panic propagation");
    assert!(saw_started.load(Ordering::SeqCst));

    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "source_event_id": "evt_inbound_panic" }, None)
        .await
        .expect("query agent_run_logs")
        .expect("envelope present");
    assert_eq!(log.lifecycle, LIFECYCLE_FAILED_BEFORE_DECISION);
    assert!(
        log.error_summary
            .as_deref()
            .map(|s| s == "unhandled_panic: probe llm panic")
            .unwrap_or(false),
        "error_summary must retain panic payload, actual={:?}",
        log.error_summary
    );
}

#[tokio::test]
#[ignore]
async fn panic_after_reply_decision_marks_lifecycle_failed_after_decision() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("wxid_envelope_after_decision");
    let inbound = inbound(&contact, "evt_inbound_after_decision");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    let saw_running = Arc::new(AtomicBool::new(false));
    let probe: Arc<dyn LlmProvider> = Arc::new(DecisionThenPanicProbeLlm {
        db: app.state.db.clone(),
        source_event_id: "evt_inbound_after_decision".to_string(),
        call_count: AtomicUsize::new(0),
        saw_running: saw_running.clone(),
    });
    let state =
        common::rebuild_app_state_with_real_llm(&app, probe, app.state.config.mcp_base_url.clone());

    let result = AssertUnwindSafe(handle_managed_message(&state, contact, &inbound))
        .catch_unwind()
        .await;
    assert!(result.is_err(), "review panic must propagate");
    assert!(
        saw_running.load(Ordering::SeqCst),
        "review invocation must observe started → running after Reply decision"
    );

    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(
            doc! { "source_event_id": "evt_inbound_after_decision" },
            None,
        )
        .await
        .expect("query agent_run_logs")
        .expect("envelope present");
    assert_eq!(log.lifecycle, LIFECYCLE_FAILED_AFTER_DECISION);
    assert_eq!(log.status, "internal_error");
    assert_eq!(
        log.error_summary.as_deref(),
        Some("unhandled_panic: probe review panic after decision")
    );
    assert!(
        !log.decision.is_empty(),
        "running transition stores decision snapshot"
    );
    let count = app
        .state
        .db
        .agent_run_logs()
        .count_documents(
            doc! { "source_event_id": "evt_inbound_after_decision" },
            None,
        )
        .await
        .expect("count run logs");
    assert_eq!(count, 1, "failure must close the original envelope");
}

//! HC-004 account/workspace scope redlines for SR-080, SR-116, SR-119, SR-124, and SR-152.
//!
//! These tests use the production aggregation, digest, authentication middleware,
//! and Axum router. Each test owns a random database and explicitly cleans it up.

#![cfg(test)]

mod common;

use std::sync::Arc;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::knowledge_digest::{
    analyze_chunk_health_ids_for_redline, generate_all_account_digests_for_redline,
};
use wechatagent::knowledge_task::{run_task, ChatProgressBus};
use wechatagent::knowledge_wiki::gap_signals::refresh_usage_stats_and_confidence;
use wechatagent::models::{
    AgentDecisionReview, AgentRunLog, AgentStatus, Contact, KnowledgeChatTask,
    KnowledgeDailyReport, KnowledgeDigestCard, KnowledgeUsageLog, OperationKnowledgeChunk,
    OutcomeEvent, WechatAccount,
};
use wechatagent::routes::api_router;

use crate::common::TestApp;

const ACCOUNT_A: &str = "hc004-account-a";
const ACCOUNT_B: &str = "hc004-account-b";
const SHARED_WXID: &str = "hc004-shared-wxid";

fn account(workspace_id: &str, account_id: &str) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        alias: account_id.to_string(),
        display_name: account_id.to_string(),
        app_id: None,
        wxid: None,
        nick_name: None,
        avatar_url: None,
        mcp_base_url: None,
        mcp_api_key: None,
        webhook_secret: None,
        online: true,
        status: Some("active".to_string()),
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: vec![],
        created_at: now,
        updated_at: now,
    }
}

fn contact(
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
    outcome_events: Vec<OutcomeEvent>,
) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        wxid: wxid.to_string(),
        nickname: Some(format!("{account_id} contact")),
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
        manual_tags: vec![],
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: vec![],
        bayesian_signals: vec![],
        personality_profile: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: vec![],
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
        intent_trajectory: vec![],
        outcome_events,
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

fn confirmed_deal(at: DateTime) -> OutcomeEvent {
    OutcomeEvent {
        marked_at: at,
        occurred_at: Some(at),
        amount: Some(10_000),
        currency: Some("CNY".to_string()),
        source: "manual".to_string(),
        marked_by: "hc004-admin".to_string(),
        note: Some("scope redline".to_string()),
        verification: "staff_confirmed".to_string(),
        product_ref: None,
        event_kind: "deal".to_string(),
    }
}

fn chunk(workspace_id: &str, account_id: &str, title: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: Some(account_id.to_string()),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        body: Some(title.to_string()),
        integrity_status: Some("verified".to_string()),
        status: "active".to_string(),
        integrity_score: Some(0.5),
        ..OperationKnowledgeChunk::default()
    }
}

fn usage_log(
    workspace_id: &str,
    account_id: &str,
    run_id: &str,
    chunk_id: ObjectId,
    created_at: DateTime,
) -> KnowledgeUsageLog {
    KnowledgeUsageLog {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: Some(SHARED_WXID.to_string()),
        run_id: run_id.to_string(),
        knowledge_ids: vec![chunk_id],
        route_result: Document::new(),
        reply_text: None,
        review_approved: false,
        blocked_reason: None,
        tool_trace: vec![],
        created_at,
    }
}

fn review(workspace_id: &str, account_id: &str, run_id: &str, reply: &str) -> AgentDecisionReview {
    AgentDecisionReview {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: Some(SHARED_WXID.to_string()),
        run_id: Some(run_id.to_string()),
        inbound_message_id: None,
        reply_text: Some(reply.to_string()),
        approved: false,
        scores: Document::new(),
        formula_breakdown: Document::new(),
        risks: vec![],
        rewrite_instruction: None,
        review_summary: None,
        playbook_id: None,
        playbook_version: None,
        used_knowledge_ids: vec![],
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
        status: "blocked".to_string(),
        created_at: DateTime::now(),
    }
}

fn run_log(workspace_id: &str, account_id: &str, run_id: &str, marker: &str) -> AgentRunLog {
    AgentRunLog {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: Some(SHARED_WXID.to_string()),
        run_id: run_id.to_string(),
        trigger_kind: "reply".to_string(),
        status: "blocked".to_string(),
        planner: Document::new(),
        context: Document::new(),
        knowledge_route: Document::new(),
        decision: doc! { "whyShouldReply": marker },
        review: doc! { "holdCategory": marker },
        gateway_result: Document::new(),
        error: None,
        token_budget: 0,
        tokens_used: 0,
        llm_calls_used: 0,
        unknown_usage_calls: 0,
        degraded_reasons: vec![],
        lifecycle: "completed".to_string(),
        source_event_id: format!("event-{run_id}"),
        source_kind: "inbound_message".to_string(),
        error_summary: None,
        abort_reason: None,
        revision_applied: false,
        revision_reason: String::new(),
        pre_revision_summary: None,
        post_revision_summary: None,
        self_critique: None,
        autonomy_mode: "blocked".to_string(),
        conversation_mode: String::new(),
        conversation_mode_reason: None,
        final_review_status: marker.to_string(),
        outbox_status: None,
        memory_consolidator_warnings: vec![],
        created_at: DateTime::now(),
    }
}

fn digest_report(
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
    card_id: ObjectId,
) -> KnowledgeDailyReport {
    let now = DateTime::now();
    KnowledgeDailyReport {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        report_date: report_date.to_string(),
        generated_at: now,
        generated_by: "hc004-redline".to_string(),
        status: "ok".to_string(),
        error_kind: None,
        budget_snapshot: Document::new(),
        cards: vec![KnowledgeDigestCard {
            card_id,
            kind: "chunk_missing_field".to_string(),
            title: "same semantic card".to_string(),
            summary: "same card id in sibling accounts".to_string(),
            target_refs: vec![doc! { "kind": "chunk", "id": "shared-chunk" }],
            suggested_action: "dismiss".to_string(),
            severity: "warn".to_string(),
            metric: None,
        }],
        dismissed_card_ids: vec![],
        prompt_versions: Document::new(),
        attempt_generation: 1,
        current_generation: 1,
        latest_attempt_status: Some("ok".to_string()),
        latest_attempt_error_kind: None,
        latest_attempt_at: Some(now),
        latest_attempt_budget_snapshot: Document::new(),
        last_success_at: Some(now),
    }
}

fn dismiss_task(
    task_id: ObjectId,
    workspace_id: &str,
    account_id: &str,
    report_date: &str,
    card_id: ObjectId,
) -> KnowledgeChatTask {
    let now = DateTime::now();
    KnowledgeChatTask {
        id: Some(task_id),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        session_id: format!("hc004-sr124-{task_id}"),
        owner_admin_id: Some("hc004-admin".to_string()),
        operator_id: Some("hc004-operator".to_string()),
        cards: vec![],
        dispatch_binding: None,
        planned_steps: vec![doc! {
            "stepId": "dismiss-1",
            "cardId": card_id.to_hex(),
            "action": "dismiss",
            "reportDate": report_date,
            "summary": "dismiss the account-scoped card",
        }],
        completed_steps: vec![],
        step_intents: vec![],
        status: "pending".to_string(),
        error_kind: None,
        attempts: 0,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        heartbeat_at: None,
        created_at: now,
        started_at: None,
        finished_at: None,
    }
}

fn admin_user(workspace_id: &str) -> AdminUser {
    AdminUser {
        user_id: "hc004-admin".to_string(),
        username: "hc004-admin".to_string(),
        password_hash: "unused".to_string(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec![workspace_id.to_string()],
        default_workspace: Some(workspace_id.to_string()),
    }
}

async fn start_api(
    app: &TestApp,
    workspace_id: &str,
) -> (String, String, tokio::task::JoinHandle<()>) {
    let user = admin_user(workspace_id);
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&user, None)
        .await
        .expect("seed HC-004 admin");
    let session = create_session(&app.state.db, &user, 1, workspace_id)
        .await
        .expect("create HC-004 session");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind HC-004 API");
    let address = listener.local_addr().expect("HC-004 API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve HC-004 API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn sr116_deal_attribution_keeps_same_wxid_accounts_separate() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let log_at = DateTime::from_millis(DateTime::now().timestamp_millis() - 10_000);
    let deal_at = DateTime::from_millis(log_at.timestamp_millis() + 5_000);
    let chunk_a = chunk(&workspace, ACCOUNT_A, "A knowledge");
    let chunk_b = chunk(&workspace, ACCOUNT_B, "B knowledge");
    let chunk_a_id = chunk_a.id.expect("chunk A id");
    let chunk_b_id = chunk_b.id.expect("chunk B id");

    app.state
        .db
        .contacts()
        .insert_many(
            vec![
                contact(
                    &workspace,
                    ACCOUNT_A,
                    SHARED_WXID,
                    vec![confirmed_deal(deal_at)],
                ),
                contact(&workspace, ACCOUNT_B, SHARED_WXID, vec![]),
            ],
            None,
        )
        .await
        .expect("seed same-wxid contacts");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many(vec![chunk_a, chunk_b], None)
        .await
        .expect("seed scoped chunks");
    app.state
        .db
        .knowledge_usage_logs()
        .insert_many(
            vec![
                usage_log(&workspace, ACCOUNT_A, "sr116-run-a", chunk_a_id, log_at),
                usage_log(&workspace, ACCOUNT_B, "sr116-run-b", chunk_b_id, log_at),
            ],
            None,
        )
        .await
        .expect("seed scoped usage logs");

    let report = refresh_usage_stats_and_confidence(&app.state.db, &workspace, 0, true)
        .await
        .expect("refresh usage stats");
    assert_eq!(report.deal_attributed_hits, 1);

    let a = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_a_id }, None)
        .await
        .expect("read A chunk")
        .expect("A chunk exists");
    let b = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_b_id }, None)
        .await
        .expect("read B chunk")
        .expect("B chunk exists");
    assert_eq!(a.usage_stats.expect("A stats").hit_count_30d, 1);
    assert_eq!(b.usage_stats.expect("B stats").hit_count_30d, 0);

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn sr119_digest_health_includes_shared_chunks_without_crossing_accounts() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();

    let mut shared = chunk(&workspace, ACCOUNT_A, "shared health issue");
    shared.account_id = None;
    shared.integrity_status = Some("missing_evidence".to_string());
    let shared_id = shared.id.expect("shared chunk id");

    let mut own = chunk(&workspace, ACCOUNT_A, "own health issue");
    own.integrity_status = Some("needs_review".to_string());
    let own_id = own.id.expect("own chunk id");

    let mut foreign = chunk(&workspace, ACCOUNT_B, "foreign health issue");
    foreign.integrity_status = Some("missing_evidence".to_string());
    let foreign_id = foreign.id.expect("foreign chunk id");

    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many(vec![shared, own, foreign], None)
        .await
        .expect("seed digest health visibility rows");

    let mut actual = analyze_chunk_health_ids_for_redline(&app.state, &workspace, ACCOUNT_A)
        .await
        .expect("analyze account-visible chunk health");
    actual.sort();
    let mut expected = vec![shared_id.to_hex(), own_id.to_hex()];
    expected.sort();

    assert_eq!(actual, expected);
    assert!(!actual.contains(&foreign_id.to_hex()));

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn sr119_digest_generation_persists_and_audits_each_account_scope() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    app.state
        .db
        .accounts()
        .insert_many(
            vec![
                account(&workspace, ACCOUNT_A),
                account(&workspace, ACCOUNT_B),
            ],
            None,
        )
        .await
        .expect("seed digest accounts");
    app.llm.push_response(json!({ "cards": [] }));
    app.llm.push_response(json!({ "cards": [] }));

    assert_eq!(
        generate_all_account_digests_for_redline(&app.state)
            .await
            .expect("run scheduled account enumeration"),
        2
    );

    for account_id in [ACCOUNT_A, ACCOUNT_B] {
        assert_eq!(
            app.state
                .db
                .knowledge_daily_reports()
                .count_documents(
                    doc! { "workspace_id": &workspace, "account_id": account_id },
                    None,
                )
                .await
                .expect("count scoped reports"),
            1
        );
        assert_eq!(
            app.state
                .db
                .knowledge_usage_logs()
                .count_documents(
                    doc! {
                        "workspace_id": &workspace,
                        "account_id": account_id,
                        "route_result.kind": "digest_compose",
                    },
                    None,
                )
                .await
                .expect("count scoped digest usage logs"),
            1
        );
        assert_eq!(
            app.state
                .db
                .events()
                .count_documents(
                    doc! {
                        "workspace_id": &workspace,
                        "account_id": account_id,
                        "kind": "knowledge_digest_generated",
                    },
                    None,
                )
                .await
                .expect("count scoped digest events"),
            1
        );
        assert_eq!(
            app.state
                .db
                .llm_call_logs()
                .count_documents(
                    doc! {
                        "workspace_id": &workspace,
                        "account_id": account_id,
                        "prompt_key": "knowledge.digest.compose",
                    },
                    None,
                )
                .await
                .expect("count scoped digest LLM logs"),
            1
        );
    }

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn sr124_direct_dismiss_router_never_crosses_same_card_id_accounts() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let card_id = ObjectId::new();
    let report_a = digest_report(&workspace, ACCOUNT_A, &report_date, card_id);
    let report_b = digest_report(&workspace, ACCOUNT_B, &report_date, card_id);
    let report_a_id = report_a.id.expect("report A id");
    let report_b_id = report_b.id.expect("report B id");
    app.state
        .db
        .knowledge_daily_reports()
        .insert_many([report_a, report_b], None)
        .await
        .expect("seed sibling account digest reports");

    let before_a = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_a_id }, None)
        .await
        .expect("read report A before")
        .expect("report A exists");
    let before_b = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_b_id }, None)
        .await
        .expect("read report B before")
        .expect("report B exists");
    let before_a_bson = mongodb::bson::to_document(&before_a).expect("serialize report A");
    let before_b_bson = mongodb::bson::to_document(&before_b).expect("serialize report B");

    let (base_url, cookie, server) = start_api(&app, &workspace).await;
    let client = reqwest::Client::new();
    let path = format!("{base_url}/knowledge/digest/cards/{card_id}/dismiss");

    let missing = client
        .post(&path)
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("missing account dismiss request");
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let unknown = client
        .post(format!("{path}?accountId=hc004-unknown-account"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("unknown account dismiss request");
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let unchanged_a = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_a_id }, None)
        .await
        .expect("read report A after rejects")
        .expect("report A exists");
    let unchanged_b = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_b_id }, None)
        .await
        .expect("read report B after rejects")
        .expect("report B exists");
    assert_eq!(
        mongodb::bson::to_document(&unchanged_a).expect("serialize unchanged A"),
        before_a_bson
    );
    assert_eq!(
        mongodb::bson::to_document(&unchanged_b).expect("serialize unchanged B"),
        before_b_bson
    );

    let accepted = client
        .post(format!("{path}?accountId={ACCOUNT_B}"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("account B dismiss request");
    assert_eq!(accepted.status(), StatusCode::OK);

    let after_a = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_a_id }, None)
        .await
        .expect("read report A after target dismiss")
        .expect("report A exists");
    let after_b = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_b_id }, None)
        .await
        .expect("read report B after target dismiss")
        .expect("report B exists");
    assert_eq!(
        mongodb::bson::to_document(&after_a).expect("serialize final A"),
        before_a_bson,
        "sibling account report must remain byte-for-byte unchanged"
    );
    assert_eq!(after_b.dismissed_card_ids, vec![card_id]);

    server.abort();
    let _ = server.await;
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn sr124_fenced_worker_dismiss_never_crosses_same_card_id_accounts() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let report_date = "2026-07-29";
    let card_id = ObjectId::new();
    let report_a = digest_report(&workspace, ACCOUNT_A, report_date, card_id);
    let report_b = digest_report(&workspace, ACCOUNT_B, report_date, card_id);
    let report_a_id = report_a.id.expect("report A id");
    let report_b_id = report_b.id.expect("report B id");
    app.state
        .db
        .knowledge_daily_reports()
        .insert_many([report_a, report_b], None)
        .await
        .expect("seed sibling account reports for worker");
    let before_a = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_a_id }, None)
        .await
        .expect("read worker sibling before")
        .expect("worker sibling exists");
    let before_a_bson = mongodb::bson::to_document(&before_a).expect("serialize worker sibling");

    let task_id = ObjectId::new();
    let task = dismiss_task(task_id, &workspace, ACCOUNT_B, report_date, card_id);
    app.state
        .db
        .knowledge_chat_session_seqs()
        .insert_one(
            doc! {
                "_id": format!("{}|{}", workspace, task.session_id),
                "workspace_id": &workspace,
                "account_id": ACCOUNT_B,
                "session_id": &task.session_id,
                "owner_admin_id": "hc004-admin",
                "seq": 0_i64,
                "created_at": task.created_at,
                "updated_at": task.created_at,
            },
            None,
        )
        .await
        .expect("seed worker session sequence");
    app.state
        .db
        .knowledge_chat_tasks()
        .insert_one(&task, None)
        .await
        .expect("seed worker dismiss task");

    run_task(&app.state, &Arc::new(ChatProgressBus::new()), task)
        .await
        .expect("run production fenced dismiss task");

    let after_a = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_a_id }, None)
        .await
        .expect("read worker sibling after")
        .expect("worker sibling exists");
    let after_b = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(doc! { "_id": report_b_id }, None)
        .await
        .expect("read worker target after")
        .expect("worker target exists");
    assert_eq!(
        mongodb::bson::to_document(&after_a).expect("serialize worker sibling after"),
        before_a_bson,
        "worker must not mutate the sibling account report"
    );
    assert_eq!(after_b.dismissed_card_ids, vec![card_id]);

    let saved_task = app
        .state
        .db
        .knowledge_chat_tasks()
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("read completed worker task")
        .expect("completed worker task exists");
    assert_eq!(saved_task.status, "completed");
    assert_eq!(saved_task.completed_steps.len(), 1);
    assert_eq!(
        saved_task.completed_steps[0].get_str("status").ok(),
        Some("committed")
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn sr080_enable_agent_uses_the_contact_workspace_account_identity() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let foreign_workspace = "hc004-sr080-foreign-workspace";
    let account_id = "hc004-sr080-shared-account";
    let contact_wxid = "hc004-sr080-target-contact";

    let mut local_account = account(&workspace, account_id);
    local_account.wxid = Some("hc004-sr080-local-self".to_string());
    let mut foreign_account = account(foreign_workspace, account_id);
    foreign_account.wxid = Some(contact_wxid.to_string());
    let foreign_account_id = foreign_account.id.expect("foreign account id");
    app.state
        .db
        .accounts()
        .insert_many([local_account, foreign_account], None)
        .await
        .expect("seed same-account-id workspaces");

    let mut target = contact(&workspace, account_id, contact_wxid, vec![]);
    target.agent_status = AgentStatus::Normal;
    let target_id = target.id.expect("target contact id");
    app.state
        .db
        .contacts()
        .insert_one(target, None)
        .await
        .expect("seed target contact");

    let foreign_before = app
        .state
        .db
        .accounts()
        .find_one(doc! { "_id": foreign_account_id }, None)
        .await
        .expect("read foreign account before")
        .expect("foreign account exists");
    let foreign_before_bson =
        mongodb::bson::to_document(&foreign_before).expect("serialize foreign account before");
    let foreign_events_before = app
        .state
        .db
        .events()
        .count_documents(doc! { "workspace_id": foreign_workspace }, None)
        .await
        .expect("count foreign events before");

    app.llm.push_response(json!({
        "agentProfile": {
            "summary": "Account-scoped enable redline profile",
            "interests": [],
            "communicationStyle": "direct",
            "operationGoal": "Maintain the existing relationship"
        },
        "tags": []
    }));
    let (base_url, cookie, server) = start_api(&app, &workspace).await;
    let response = reqwest::Client::new()
        .post(format!("{base_url}/contacts/{target_id}/enable-agent"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({
            "expectedAccountId": account_id,
            "humanProfileNote": "Known customer in the local workspace"
        }))
        .send()
        .await
        .expect("enable target contact through production router");
    assert_eq!(response.status(), StatusCode::OK);

    let enabled = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": target_id }, None)
        .await
        .expect("read enabled contact")
        .expect("enabled contact exists");
    assert_eq!(enabled.agent_status, AgentStatus::Managed);
    assert_eq!(enabled.workspace_id, workspace);
    assert_eq!(enabled.account_id, account_id);

    let foreign_after = app
        .state
        .db
        .accounts()
        .find_one(doc! { "_id": foreign_account_id }, None)
        .await
        .expect("read foreign account after")
        .expect("foreign account remains");
    assert_eq!(
        mongodb::bson::to_document(&foreign_after).expect("serialize foreign account after"),
        foreign_before_bson,
        "enable must not mutate the same account_id in another workspace"
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(doc! { "workspace_id": foreign_workspace }, None)
            .await
            .expect("count foreign events after"),
        foreign_events_before,
        "enable must not emit an event into the foreign workspace"
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": &workspace,
                    "account_id": account_id,
                    "contact_wxid": contact_wxid,
                    "kind": "contact.enabled_for_ops",
                },
                None,
            )
            .await
            .expect("count local enable event"),
        1
    );

    server.abort();
    let _ = server.await;
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn sr152_review_routes_require_account_and_never_cross_same_wxid_accounts() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    app.state
        .db
        .accounts()
        .insert_many(
            vec![
                account(&workspace, ACCOUNT_A),
                account(&workspace, ACCOUNT_B),
            ],
            None,
        )
        .await
        .expect("seed review accounts");
    let contact_b = contact(&workspace, ACCOUNT_B, SHARED_WXID, vec![]);
    let contact_b_id = contact_b.id.expect("contact B id");
    app.state
        .db
        .contacts()
        .insert_one(contact_b, None)
        .await
        .expect("seed review contact");

    let review_a = review(&workspace, ACCOUNT_A, "sr152-run-a", "A private review");
    let review_b = review(&workspace, ACCOUNT_B, "sr152-run-b", "B expected review");
    let review_b_id = review_b.id.expect("review B id");
    app.state
        .db
        .decision_reviews()
        .insert_many(vec![review_a, review_b], None)
        .await
        .expect("seed scoped reviews");
    app.state
        .db
        .agent_run_logs()
        .insert_many(
            vec![
                run_log(&workspace, ACCOUNT_A, "sr152-run-a", "A-only-marker"),
                run_log(&workspace, ACCOUNT_B, "sr152-run-b", "B-only-marker"),
            ],
            None,
        )
        .await
        .expect("seed scoped run logs");

    let (base_url, cookie, server) = start_api(&app, &workspace).await;
    let client = reqwest::Client::new();

    let missing = client
        .get(format!(
            "{base_url}/decision-reviews?contactId={contact_b_id}&limit=20"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("missing-account request");
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let wrong = client
        .get(format!(
            "{base_url}/decision-reviews?accountId={ACCOUNT_A}&contactId={contact_b_id}&limit=20"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("wrong-account request");
    assert_eq!(wrong.status(), StatusCode::NOT_FOUND);

    let correct = client
        .get(format!(
            "{base_url}/decision-reviews?accountId={ACCOUNT_B}&contactId={contact_b_id}&limit=20"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("correct-account request");
    assert_eq!(correct.status(), StatusCode::OK);
    let body: Value = correct.json().await.expect("correct review JSON");
    let items = body["items"].as_array().expect("review items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["accountId"], ACCOUNT_B);
    assert_eq!(items[0]["replyText"], "B expected review");
    assert_eq!(items[0]["finalReviewStatus"], "B-only-marker");
    assert_eq!(items[0]["holdCategory"], "B-only-marker");
    assert!(!body.to_string().contains("A private review"));
    assert!(!body.to_string().contains("A-only-marker"));

    let wrong_detail = client
        .get(format!(
            "{base_url}/decision-reviews/{review_b_id}?accountId={ACCOUNT_A}"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("wrong-account detail request");
    assert_eq!(wrong_detail.status(), StatusCode::NOT_FOUND);

    let correct_detail = client
        .get(format!(
            "{base_url}/decision-reviews/{review_b_id}?accountId={ACCOUNT_B}"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("correct-account detail request");
    assert_eq!(correct_detail.status(), StatusCode::OK);
    let detail: Value = correct_detail.json().await.expect("review detail JSON");
    assert_eq!(detail["item"]["accountId"], ACCOUNT_B);
    assert_eq!(detail["item"]["finalReviewStatus"], "B-only-marker");

    server.abort();
    let _ = server.await;
    app.cleanup().await;
}

//! W4 / Task 5.8（R13.10）outbox 集成测试。
//!
//! 6 例覆盖：
//! 1. 入队 → atomic_claim → MCP mock 成功 → status=sent
//! 2. MCP mock 失败 3 次 → status=failed_terminal（统一枚举值）
//! 3. record_user_reaction stop_requested → 同 contact 所有 pending outbox canceled
//! 4. 30 分钟陈旧 outbox 自动 canceled（second_safety_gate）
//! 5. 崩溃恢复：worker A 抢占后 lease 过期，worker B reclaim_expired_leases 重新抢占
//! 6. PBT：任意状态序列下唯一 idempotency_key 永远 ≤ 1 次 MCP 实际发送
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）；CI 用
//! `cargo test --test outbox_integration -- --ignored` 触发。

mod common;

use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{
    atomic_claim_pending, cancel_entry, cancel_for_contact_on_user_reaction, enqueue,
    process_entry, reclaim_expired_leases, second_safety_gate, EnqueueOutcome, EnqueueRequest,
    OutboxStatus,
};
use wechatagent::models::Contact;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// 构造一个 managed contact（dispatcher 在 process_entry 时会按
/// (workspace_id, account_id, wxid) 查 contact）。
fn make_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("集成测试客户".to_string()),
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
        created_at: now,
        updated_at: now,
    }
}

/// 启 wiremock，POST /mcp 一律返回 MCP `tools/call` 成功 envelope。
async fn start_mcp_mock_success() -> MockServer {
    let server = MockServer::start().await;
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "structuredContent": {
                "newMsgId": "mock_msg_id_42",
                "content": []
            }
        }
    });
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

/// 启 wiremock，POST /mcp 一律返回 500 失败，便于覆盖 retry-then-terminal 路径。
async fn start_mcp_mock_failure() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(500).set_body_string("simulated mcp failure"))
        .mount(&server)
        .await;
    server
}

fn enqueue_request(run_id: &str, source_event_id: &str, contact_wxid: &str) -> EnqueueRequest {
    EnqueueRequest {
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: contact_wxid.to_string(),
        run_id: run_id.to_string(),
        decision_id: None,
        source_event_id: source_event_id.to_string(),
        source_kind: "inbound_message".to_string(),
        content: "你好，这是集成测试投递的内容。".to_string(),
        max_attempts: 3,
    }
}

// ── Case 1: 入队 → claim → MCP 成功 → sent ──────────────────────────────

#[tokio::test]
#[ignore]
async fn happy_path_enqueue_claim_send_sent() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());

    let contact = make_contact("user_happy");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outcome = enqueue(&state, enqueue_request("run_happy", "evt_1", &contact.wxid))
        .await
        .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    let claimed = atomic_claim_pending(&state, "worker_t1", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    assert_eq!(claimed.id, Some(outbox_id));
    assert_eq!(claimed.status, OutboxStatus::InFlight.as_str());

    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(entry.status, OutboxStatus::Sent.as_str(), "{:?}", entry);
    assert!(entry.sent_at.is_some(), "sent_at must be populated");
    assert!(entry.worker_id.is_none(), "worker_id cleared on sent");
    assert!(entry.locked_until.is_none(), "locked_until cleared on sent");
}

// ── Case 2: MCP 失败 3 次 → failed_terminal ─────────────────────────────

#[tokio::test]
#[ignore]
async fn three_failures_lead_to_failed_terminal() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_failure().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());

    let contact = make_contact("user_fail");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outcome = enqueue(&state, enqueue_request("run_fail", "evt_2", &contact.wxid))
        .await
        .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    // max_attempts=3 → 期望 attempt=1,2 走 retry pending，attempt=3 时 failed_terminal。
    let collection = state.db.collection_agent_send_outbox();
    for i in 0..3 {
        // 清掉 next_retry_at，避免实际等 backoff。
        collection
            .update_one(
                doc! { "_id": outbox_id },
                doc! { "$set": { "next_retry_at": null } },
                None,
            )
            .await
            .expect("clear next_retry_at");
        let claimed = atomic_claim_pending(&state, &format!("worker_t2_{i}"), 60)
            .await
            .expect("claim ok")
            .unwrap_or_else(|| panic!("claim should yield entry on iteration {i}"));
        process_entry(&state, &claimed)
            .await
            .expect("process entry ok");
    }

    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(
        entry.status,
        OutboxStatus::FailedTerminal.as_str(),
        "after 3 failures must be failed_terminal, got {:?}",
        entry
    );
    assert_eq!(entry.attempt, 3, "attempt counter should reach 3");
    assert!(entry.last_error.is_some(), "last_error must be populated");
}

// ── Case 3: user reaction stop → all pending canceled ───────────────────

#[tokio::test]
#[ignore]
async fn user_reaction_stop_cancels_all_pending() {
    let app = common::TestApp::start().await;
    let state = app.state.clone();

    let contact = make_contact("user_stop");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outcome_a = enqueue(
        &state,
        enqueue_request("run_stop_a", "evt_a", &contact.wxid),
    )
    .await
    .expect("enqueue a");
    let outcome_b = enqueue(
        &state,
        EnqueueRequest {
            content: "另一条消息".to_string(),
            ..enqueue_request("run_stop_b", "evt_b", &contact.wxid)
        },
    )
    .await
    .expect("enqueue b");

    let id_a = match outcome_a {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let id_b = match outcome_b {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    let canceled = cancel_for_contact_on_user_reaction(&state, &contact.account_id, &contact.wxid)
        .await
        .expect("cancel ok");
    assert_eq!(canceled, 2, "both pending entries should be canceled");

    let collection = state.db.collection_agent_send_outbox();
    for id in [id_a, id_b] {
        let entry = collection
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("query")
            .expect("entry exists");
        assert_eq!(entry.status, OutboxStatus::Canceled.as_str());
        assert_eq!(
            entry.cancel_reason.as_deref(),
            Some("user_reaction_stop_requested")
        );
    }
}

// ── Case 4: 30-min stale → second_safety_gate cancels ───────────────────

#[tokio::test]
#[ignore]
async fn stale_thirty_minute_entry_is_canceled_by_safety_gate() {
    let app = common::TestApp::start().await;
    let state = app.state.clone();

    let contact = make_contact("user_stale");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outcome = enqueue(
        &state,
        enqueue_request("run_stale", "evt_stale", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    // 把 created_at 倒推 31 分钟，模拟陈旧条目。
    let stale_ms = DateTime::now().timestamp_millis() - 31 * 60 * 1000;
    let stale = DateTime::from_millis(stale_ms);
    let collection = state.db.collection_agent_send_outbox();
    collection
        .update_one(
            doc! { "_id": outbox_id },
            doc! { "$set": { "created_at": stale } },
            None,
        )
        .await
        .expect("backdate created_at");

    let claimed = atomic_claim_pending(&state, "worker_stale", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    let reason = second_safety_gate(&state, &claimed)
        .await
        .expect("safety gate ok")
        .expect("must cancel stale entry");
    assert!(
        reason.contains("stale"),
        "cancel reason should mention stale, got {reason:?}"
    );

    cancel_entry(&state, outbox_id, &claimed, &reason)
        .await
        .expect("cancel entry ok");

    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(entry.status, OutboxStatus::Canceled.as_str());
    assert!(entry
        .cancel_reason
        .as_deref()
        .map(|r| r.contains("stale"))
        .unwrap_or(false));
}

// ── Case 5: crash recovery: lease expires → worker B reclaims ───────────

#[tokio::test]
#[ignore]
async fn crash_recovery_worker_b_reclaims_after_lease_expires() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());

    let contact = make_contact("user_crash");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outcome = enqueue(
        &state,
        enqueue_request("run_crash", "evt_crash", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    // worker A 抢占；不调 process_entry，模拟"worker A 卡住 / 崩溃"。
    let claimed_a = atomic_claim_pending(&state, "worker_A", 60)
        .await
        .expect("claim A ok")
        .expect("worker A claimed entry");
    assert_eq!(claimed_a.status, OutboxStatus::InFlight.as_str());
    assert_eq!(claimed_a.worker_id.as_deref(), Some("worker_A"));

    // 人为把 locked_until 调到过去，模拟 lease 自然过期。
    let expired_ms = DateTime::now().timestamp_millis() - 1_000;
    let expired = DateTime::from_millis(expired_ms);
    let collection = state.db.collection_agent_send_outbox();
    collection
        .update_one(
            doc! { "_id": outbox_id },
            doc! { "$set": { "locked_until": expired } },
            None,
        )
        .await
        .expect("backdate locked_until");

    let reclaimed = reclaim_expired_leases(&state)
        .await
        .expect("reclaim ok");
    assert_eq!(reclaimed, 1, "exactly one entry must be reclaimed");

    let after_reclaim = collection
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query")
        .expect("entry exists");
    assert_eq!(after_reclaim.status, OutboxStatus::Pending.as_str());
    assert!(after_reclaim.worker_id.is_none());
    assert!(after_reclaim.locked_until.is_none());

    // worker B 抢占并完成。
    let claimed_b = atomic_claim_pending(&state, "worker_B", 60)
        .await
        .expect("claim B ok")
        .expect("worker B claimed entry");
    assert_eq!(claimed_b.worker_id.as_deref(), Some("worker_B"));
    process_entry(&state, &claimed_b)
        .await
        .expect("process entry ok");

    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(entry.status, OutboxStatus::Sent.as_str());
}

// ── Case 6: 任意状态序列下 idempotency_key 唯一 → ≤ 1 次实际发送 ────────

#[tokio::test]
#[ignore]
async fn idempotency_key_yields_at_most_one_mcp_send() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());

    let contact = make_contact("user_idem");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 同一 (source_event_id, contact_wxid, content) 重复入队 7 次 + 不同内容 1 次。
    let req = enqueue_request("run_idem", "evt_idem", &contact.wxid);
    let mut created = 0usize;
    let mut skipped = 0usize;
    let mut first_outbox_id: Option<ObjectId> = None;
    for _ in 0..7 {
        match enqueue(&state, req.clone())
            .await
            .expect("enqueue ok")
        {
            EnqueueOutcome::Created { outbox_id, .. } => {
                created += 1;
                first_outbox_id = Some(outbox_id);
            }
            EnqueueOutcome::IdempotentSkip { .. } => {
                skipped += 1;
            }
        }
    }
    assert_eq!(created, 1, "first enqueue creates exactly one row");
    assert_eq!(skipped, 6, "subsequent enqueues hit unique-index dedupe");

    // 不同内容 → 应当再创建一行。
    let other = EnqueueRequest {
        content: "不同的内容，应当独立入队。".to_string(),
        source_event_id: "evt_other".to_string(),
        ..req.clone()
    };
    let other_outcome = enqueue(&state, other).await.expect("enqueue other ok");
    let other_id = match other_outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    // 跑两条都到终态。
    let outbox_id = first_outbox_id.expect("first id captured");
    for _ in 0..2 {
        let claimed = atomic_claim_pending(&state, "worker_idem", 60)
            .await
            .expect("claim ok")
            .expect("must claim entry");
        process_entry(&state, &claimed)
            .await
            .expect("process ok");
    }
    let entry_a =
        common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    let entry_b = common::wait_for_outbox_processed(&state, other_id, Duration::from_secs(5)).await;
    assert_eq!(entry_a.status, OutboxStatus::Sent.as_str());
    assert_eq!(entry_b.status, OutboxStatus::Sent.as_str());

    // 实际 MCP 调用次数（wiremock received_requests）应当 = 2，不是 8。
    let recv = mcp_server
        .received_requests()
        .await
        .expect("wiremock recorded requests");
    let mcp_calls = recv.len();
    assert_eq!(
        mcp_calls, 2,
        "exactly 2 MCP sends for 2 unique idempotency_keys (1 created + 1 created), 6 dupes elided",
    );

    // sanity：DB 里只有 2 行。
    let collection = state.db.collection_agent_send_outbox();
    let total = collection
        .count_documents(doc! { "contact_wxid": &contact.wxid }, None)
        .await
        .expect("count");
    assert_eq!(total, 2, "two outbox rows: one per unique idempotency_key");
}

// ── helpers ──────────────────────────────────────────────────────────────
//
// Visibility regression guard: every dispatcher helper used in this file is
// imported at the top of the module, so any future `pub` → `pub(crate)`
// change in `src/agent/outbox_dispatcher.rs` will fail this test crate's build.

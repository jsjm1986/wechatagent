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
    handle_managed_message, process_entry, reclaim_expired_leases, second_safety_gate,
    EnqueueOutcome, EnqueueRequest, OutboxStatus,
};
use wechatagent::models::{Contact, ConversationMessage, MessageDirection};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// 构造一个 managed contact（dispatcher 在 process_entry 时会按
/// (workspace_id, account_id, wxid) 查 contact，并经 P1-6 发送前状态门
/// `check_contact_status_pure` 拦截非 Managed 的投递——故 fixture 必须显式
/// 置 Managed，否则会被 cancel 成 contact_status_changed_unmanaged）。
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
        agent_status: wechatagent::models::AgentStatus::Managed,
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

/// 每次请求返回唯一 `newMsgId` 的成功 responder。
///
/// gateway::send_outbound_message 会把 `newMsgId` 写进 conversation_messages.message_id，
/// 而该字段有 sparse+unique 索引；若多次发送返回同一 id，第二次插入会撞 E11000、
/// 投递被重新置回 pending。真实 MCP 每条消息都有独立 id，故 mock 必须逐请求递增。
struct UniqueMsgIdResponder {
    counter: std::sync::atomic::AtomicU64,
}

impl wiremock::Respond for UniqueMsgIdResponder {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let seq = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "structuredContent": {
                    "newMsgId": format!("mock_msg_id_{seq}"),
                    "content": []
                }
            }
        });
        ResponseTemplate::new(200).set_body_json(body)
    }
}

/// 启 wiremock，POST /mcp 返回 MCP `tools/call` 成功 envelope（每请求唯一 newMsgId）。
async fn start_mcp_mock_success() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(UniqueMsgIdResponder {
            counter: std::sync::atomic::AtomicU64::new(0),
        })
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
        media_asset_id: None,
        referral_card_id: None,
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

// ── Case 7: ④ 账号当日发送量超软上限 → warning 事件（仅告警不拦截）──────────
//
// 软上限是账号级总量告警（防封号观测先行）：发送主路径 send_outbound_message 在
// MCP 成功后查该账号当日 `agent_send_outbox` status=sent 的总量，达到 cap 即记
// `agent.account_daily_send_soft_cap_exceeded` warning 事件，但**绝不**拦截发送。
//
// 本地无 Docker，待 CI（`cargo test --test outbox_integration -- --ignored`）。
#[tokio::test]
#[ignore]
async fn over_soft_cap_emits_warning_event() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    // cap=1：预置 1 条 sent 后再发一条即达上限，触发 warning。
    state.config.account_daily_send_soft_cap = 1;

    let contact = make_contact("user_softcap_over");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 预置一条「今天已 sent」的 outbox，使当日总量已达 cap。
    // 用 raw 无类型集合插入，便于只填计数关心的字段。
    state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "contact_wxid": &contact.wxid,
                "status": "sent",
                "sent_at": DateTime::now(),
                "content": "已发历史一条",
                "idempotency_key": format!("seed_{}", ObjectId::new()),
            },
            None,
        )
        .await
        .expect("seed sent outbox");

    // 真正发一条（走 enqueue → claim → process_entry → send_outbound_message）。
    let outcome = enqueue(
        &state,
        enqueue_request("run_softcap", "evt_softcap", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker_softcap", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    // 发送未被拦截：本条仍成功 sent。
    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(
        entry.status,
        OutboxStatus::Sent.as_str(),
        "软上限只告警，发送绝不被拦截: {:?}",
        entry
    );

    // warning 事件已记录。
    let warns = state
        .db
        .events()
        .count_documents(
            doc! {
                "account_id": "default",
                "kind": "agent.account_daily_send_soft_cap_exceeded",
                "status": "warning",
            },
            None,
        )
        .await
        .expect("count events");
    assert!(
        warns >= 1,
        "当日发送量达软上限应记 warning 事件，实际 count={warns}"
    );
}

// ── Case 8: ④ 账号当日发送量未达软上限 → 无 warning 事件 ────────────────────
#[tokio::test]
#[ignore]
async fn under_soft_cap_emits_no_warning_event() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    // 默认 cap=500，一条发送远未达上限。

    let contact = make_contact("user_softcap_under");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outcome = enqueue(
        &state,
        enqueue_request("run_softcap_under", "evt_softcap_under", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker_softcap_under", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(entry.status, OutboxStatus::Sent.as_str(), "{:?}", entry);

    let warns = state
        .db
        .events()
        .count_documents(
            doc! {
                "account_id": "default",
                "kind": "agent.account_daily_send_soft_cap_exceeded",
            },
            None,
        )
        .await
        .expect("count events");
    assert_eq!(warns, 0, "未达软上限不应记 warning 事件，实际 count={warns}");
}

// ── Case 9-12: 账号级最小发送间隔闸（防"连珠炮"机器化特征）─────────────────
//
// 闸位于 process_entry 的 reclaim 幂等门之后、send 之前：查该账号
// agent_send_outbox 中 status=sent 的最大 sent_at，距今 < 随机间隔则把本条
// reschedule 回 pending（attempt 不变、不耗重试额度），稍后 atomic_claim_pending
// 在 next_retry_at 到点后照常续发。fail-soft：查询失败 / 无 sent 历史均放行。
//
// 注：common::TestApp 的 test_config 把间隔默认设为 0/0（闸关），故这些测试
// 用 rebuild 后的 `let mut state` 把 min=max 覆盖成固定值消除随机性、便于断言。
// 本地无 Docker，待 CI（`cargo test --test outbox_integration -- --ignored`）。

/// 账号闸：同账号刚发过一条（sent_at=now），紧接的第二条在间隔内 → 被 reschedule
/// 回 pending、attempt 不变、next_retry_at 在未来、写 pacing deferred 事件、MCP 未发。
#[tokio::test]
#[ignore]
async fn account_pacing_gate_reschedules_back_to_back_send() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    // 固定间隔 2s（min=max 消除随机性）。
    state.config.account_send_min_interval_ms = 2000;
    state.config.account_send_max_interval_ms = 2000;

    let contact = make_contact("user_pacing_b2b");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 预置一条「刚刚 sent」(sent_at=now) 的账号历史（account_id="default"）。
    state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "contact_wxid": &contact.wxid,
                "run_id": "seed_pacing_b2b",
                "source_event_id": format!("seed_evt_{}", ObjectId::new()),
                "source_kind": "inbound_message",
                "status": "sent",
                "sent_at": DateTime::now(),
                "content": "刚发的历史一条",
                "content_hash": "seed_hash_b2b",
                "idempotency_key": format!("seed_{}", ObjectId::new()),
                "created_at": DateTime::now(),
                "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("seed sent history");

    // enqueue 第二条同账号 pending 条目并 claim。
    let outcome = enqueue(
        &state,
        enqueue_request("run_pacing_b2b", "evt_pacing_b2b", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker_pacing_b2b", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");

    // process_entry 在 defer 完成后返回 Ok——直接查 DB（pending 非终态，不能用
    // wait_for_outbox_processed，那只等 sent/failed_terminal/canceled 会超时）。
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    let collection = state.db.collection_agent_send_outbox();
    let entry = collection
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query")
        .expect("entry exists");
    assert_eq!(
        entry.status,
        OutboxStatus::Pending.as_str(),
        "间隔内应被 reschedule 回 pending: {:?}",
        entry
    );
    assert_eq!(entry.attempt, 0, "间隔闸不消耗重试额度，attempt 保持 0");
    let next_retry = entry.next_retry_at.expect("next_retry_at must be set");
    assert!(
        next_retry.timestamp_millis() > DateTime::now().timestamp_millis(),
        "next_retry_at 应在未来"
    );
    assert!(entry.worker_id.is_none(), "worker_id 应被清空（放回 pending）");
    assert!(entry.locked_until.is_none(), "locked_until 应被清空");

    // 写了 pacing deferred 事件。
    let deferred = state
        .db
        .events()
        .count_documents(
            doc! { "kind": "agent.send_deferred_account_pacing" },
            None,
        )
        .await
        .expect("count events");
    assert!(deferred >= 1, "应写 pacing deferred 事件，实际 count={deferred}");

    // MCP 未收到第二条的发送调用（被闸拦在发送之前）。
    let recv = mcp_server
        .received_requests()
        .await
        .expect("wiremock recorded requests");
    assert_eq!(recv.len(), 0, "被间隔闸拦下，MCP 不应收到发送调用");
}

/// 账号闸：间隔已过（last sent_at 远在 interval 之前）→ 第二条正常发出。
#[tokio::test]
#[ignore]
async fn account_pacing_gate_allows_after_interval() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    state.config.account_send_min_interval_ms = 2000;
    state.config.account_send_max_interval_ms = 2000;

    let contact = make_contact("user_pacing_after");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 历史 sent 条目 sent_at = now - 10s（远超 2s 间隔）。
    let ten_s_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 10_000);
    state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "contact_wxid": &contact.wxid,
                "run_id": "seed_pacing_after",
                "source_event_id": format!("seed_evt_{}", ObjectId::new()),
                "source_kind": "inbound_message",
                "status": "sent",
                "sent_at": ten_s_ago,
                "content": "10秒前的历史一条",
                "content_hash": "seed_hash_after",
                "idempotency_key": format!("seed_{}", ObjectId::new()),
                "created_at": ten_s_ago,
                "updated_at": ten_s_ago,
            },
            None,
        )
        .await
        .expect("seed old sent history");

    let outcome = enqueue(
        &state,
        enqueue_request("run_pacing_after", "evt_pacing_after", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker_pacing_after", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(
        entry.status,
        OutboxStatus::Sent.as_str(),
        "间隔已过应正常发出: {:?}",
        entry
    );
    let recv = mcp_server
        .received_requests()
        .await
        .expect("wiremock recorded requests");
    assert_eq!(recv.len(), 1, "间隔已过，第二条应正常发往 MCP");
}

/// 账号闸：不同账号互不影响（账号 A 刚发不拦账号 B）。
#[tokio::test]
#[ignore]
async fn account_pacing_gate_isolates_accounts() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    state.config.account_send_min_interval_ms = 2000;
    state.config.account_send_max_interval_ms = 2000;

    // 账号 B 的 contact（account_id="default" 即测试默认账号 = 这里当作 B）。
    let contact_b = make_contact("user_pacing_iso_b");
    state
        .db
        .contacts()
        .insert_one(&contact_b, None)
        .await
        .expect("insert contact B");

    // 账号 A（account_id="account_a"）刚发过一条（sent_at=now）——不应拦 B。
    state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "account_a",
                "contact_wxid": "user_account_a",
                "run_id": "seed_pacing_account_a",
                "source_event_id": format!("seed_evt_{}", ObjectId::new()),
                "source_kind": "inbound_message",
                "status": "sent",
                "sent_at": DateTime::now(),
                "content": "账号A刚发的历史",
                "content_hash": "seed_hash_account_a",
                "idempotency_key": format!("seed_{}", ObjectId::new()),
                "created_at": DateTime::now(),
                "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("seed account A sent history");

    // enqueue 账号 B（=default）的 pending 条目。
    let outcome = enqueue(
        &state,
        enqueue_request("run_pacing_iso", "evt_pacing_iso", &contact_b.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker_pacing_iso", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(
        entry.status,
        OutboxStatus::Sent.as_str(),
        "账号 A 的发送历史不应拦截账号 B: {:?}",
        entry
    );
}

/// 账号闸：该账号无 sent 历史 → 第一条不被拦（fail-soft 放行 None）。
#[tokio::test]
#[ignore]
async fn account_pacing_gate_first_send_not_blocked() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    state.config.account_send_min_interval_ms = 2000;
    state.config.account_send_max_interval_ms = 2000;

    let contact = make_contact("user_pacing_first");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 不预置任何 sent 历史 → account_last_sent_at_ms 返回 None → 放行。
    let outcome = enqueue(
        &state,
        enqueue_request("run_pacing_first", "evt_pacing_first", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker_pacing_first", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    let entry = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(
        entry.status,
        OutboxStatus::Sent.as_str(),
        "无 sent 历史，第一条不应被拦: {:?}",
        entry
    );
}

/// 账号闸 · 全链路端到端（gateway → dispatcher 拼接处）：
/// 同账号背靠背两条消息走**真实链路**，第二条被账号闸拦。
///
/// 与上面 4 个 `account_pacing_gate_*` 隔离直调 `process_entry`（手工 `enqueue`
/// 构造 outbox 条目）不同：本例的第二条 outbox 条目由 gateway
/// `handle_managed_message` 真实决策 → 审查 → 入队产出（account_id / 字段齐全），
/// 验证"gateway 真实链路产物能被账号闸正确处理"这个拼接点。
///
/// 实现路径：seed 一条"刚发过"(sent_at=now) 的同账号历史（account_id="default"，
/// 挂在另一个 contact 上——闸只按 account_id 维度查 last sent，不看 contact），
/// 把账号闸"上一条刚发"的前置条件做实；第二条经真实 gateway 入队后驱动
/// dispatcher，断言被 `defer_account_pacing` 拦回 pending。
///
/// 选这条（"seed 首条历史 + 第二条走真实 gateway 入队"）而非"两条都走真实
/// gateway 发送"，原因：真实发出第一条需要 dispatcher 在 mock MCP 下成功写
/// sent_at，再背靠背跑第二条——多一次 mock LLM 决策 + 一次实发，链路更长、更脆；
/// 而拼接点的核心验证是"第二条 = gateway 真实产出 + 被闸拦"，seed 首条历史既
/// 等价地满足了闸的前置条件，又把不确定性压到最小。
#[tokio::test]
#[ignore]
async fn account_pacing_gate_end_to_end_via_gateway() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    // 固定间隔 2s（min=max 消除随机性）；默认 0/0 是关的，必须显式覆盖。
    state.config.account_send_min_interval_ms = 2000;
    state.config.account_send_max_interval_ms = 2000;

    let contact = make_contact("user_pacing_e2e");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // seed 一条同账号(account_id="default")「刚刚 sent」的历史，挂在另一个
    // contact 上——账号闸 account_last_sent_at_ms 只按 account_id 维度查最近一条
    // status=sent 的 sent_at，做实"上一条刚发过"的前置条件。
    state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "contact_wxid": "user_pacing_e2e_prior",
                "run_id": "seed_pacing_e2e",
                "source_event_id": format!("seed_evt_{}", ObjectId::new()),
                "source_kind": "inbound_message",
                "status": "sent",
                "sent_at": DateTime::now(),
                "content": "同账号刚发的历史一条",
                "content_hash": "seed_hash_e2e",
                "idempotency_key": format!("seed_{}", ObjectId::new()),
                "created_at": DateTime::now(),
                "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("seed sent history");

    // 第二条：经**真实 gateway** 决策 → 审查 → 入队 outbox。
    // 入站消息须先落库（gateway 读取最近上下文）。
    let inbound = ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some("pacing_e2e_msg_001".to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "你们的方案我们大概了解了，下一步想看看怎么落地试点。".to_string(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    };
    state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound message");

    // mock LLM：Reply Agent 决策（直发）+ Review Agent 通过 → approved 入队一行。
    app.llm.push_response(reply_agent_decision_json(
        "可以，落地试点我们一般先圈一个核心场景跑通，要不要先按你们最急的场景来定试点范围？",
        "客户主动提出进入试点落地，这是把关系推进到执行阶段的关键时机，回复能直接降低决策摩擦。",
    ));
    app.llm.push_response(review_agent_pass_json(
        "回复承接试点诉求、不越界承诺，语气自然，可直接放行。",
    ));

    handle_managed_message(&state, contact.clone(), &inbound)
        .await
        .expect("handle_managed_message ok");

    // gateway 真实产出的 outbox 条目：approved → 入队一行 pending。
    let collection = state.db.collection_agent_send_outbox();
    let enqueued = collection
        .find_one(
            doc! { "contact_wxid": &contact.wxid, "status": OutboxStatus::Pending.as_str() },
            None,
        )
        .await
        .expect("query gateway-enqueued outbox")
        .expect("approved 路径必须由 gateway 真实入队一行 outbox");
    let outbox_id = enqueued.id.expect("enqueued outbox has _id");
    // 这条是 gateway 真实产物：account_id 字段齐全（拼接点的核心前提）。
    assert_eq!(
        enqueued.account_id, "default",
        "gateway 真实产物 account_id 应齐全"
    );
    assert_eq!(enqueued.contact_wxid, contact.wxid);

    // 驱动 dispatcher：claim → process_entry，账号闸应在发送前拦下。
    let claimed = atomic_claim_pending(&state, "worker_pacing_e2e", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    assert_eq!(claimed.id, Some(outbox_id), "claim 到的应是 gateway 入队那条");
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    // 被账号闸拦：reschedule 回 pending、attempt 不变、next_retry_at 在未来。
    let entry = collection
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query")
        .expect("entry exists");
    assert_eq!(
        entry.status,
        OutboxStatus::Pending.as_str(),
        "gateway 真实产物在间隔内应被账号闸 reschedule 回 pending: {:?}",
        entry
    );
    assert_eq!(entry.attempt, 0, "账号闸不消耗重试额度，attempt 保持 0");
    let next_retry = entry.next_retry_at.expect("next_retry_at must be set");
    assert!(
        next_retry.timestamp_millis() > DateTime::now().timestamp_millis(),
        "next_retry_at 应在未来"
    );
    assert!(entry.worker_id.is_none(), "worker_id 应被清空（放回 pending）");
    assert!(entry.locked_until.is_none(), "locked_until 应被清空");

    // 写了账号闸 deferred 事件。
    let deferred = state
        .db
        .events()
        .count_documents(doc! { "kind": "agent.send_deferred_account_pacing" }, None)
        .await
        .expect("count events");
    assert!(
        deferred >= 1,
        "应写 agent.send_deferred_account_pacing 事件，实际 count={deferred}"
    );

    // MCP 没有因第二条新增发送调用（被闸拦在发送之前；seed 历史是直接写库未走 MCP）。
    let recv = mcp_server
        .received_requests()
        .await
        .expect("wiremock recorded requests");
    assert_eq!(
        recv.len(),
        0,
        "gateway 真实产物被账号闸拦下，MCP 不应收到发送调用"
    );
}

// ── gateway 决策 / 审查 mock JSON（端到端用，照搬 full_flow_suite 的最小形态）──

/// Reply Agent 决策 JSON（shouldReply=true，knowledge_need=not_required 直发）。
fn reply_agent_decision_json(reply_text: &str, why_should_reply: &str) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户表达明确，正在评估我方方案适配度并提出落地试点诉求。",
        "relationshipRead": "对话氛围积极，关系处于稳步推进期。",
        "operationGoal": "帮客户厘清下一步排期，让客户在不被推销压力下感到掌控感。",
        "knowledgeNeedReason": "本轮只承接节奏，不涉及需核验的具体产品能力。",
        "memoryUpdateReason": "本轮新增客户进入试点阶段的锚点信息。",
        "selfCritique": "需收敛信息密度，先确认客户优先级再给出下一步建议。",
        "whyShouldReply": why_should_reply,
        "whySkipReply": "",
        "riskSelfCheck": "本轮回复不涉及未验证的产品能力承诺，不触发安全门阈值。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "need_discovery",
        "shouldReply": true,
        "replyText": reply_text,
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "客户进入方案评估阶段，按顾问模式承接。",
    })
}

/// Review Agent 通过 JSON（分数全部 ≥ 阈值，不触发 revision）。
fn review_agent_pass_json(review_summary: &str) -> serde_json::Value {
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
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true,
    })
}

// ── helpers ──────────────────────────────────────────────────────────────
//
// Visibility regression guard: every dispatcher helper used in this file is
// imported at the top of the module, so any future `pub` → `pub(crate)`
// change in `src/agent/outbox_dispatcher.rs` will fail this test crate's build.

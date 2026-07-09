//! 业务审查修复波 E 组 ⑪：账号掉线时 AI 不盲发——webhook Offline 落库建状态源 +
//! outbox dispatcher 发送前 defer。
//!
//! 3 例覆盖：
//! 1. webhook `TypeName=Offline` → 对应 account `online=false` 落库（建状态源）；
//!    `TypeName=Online` → 对称落 `online=true`。
//! 2. account.online=false 时 dispatcher `process_entry` 不调 MCP（defer）：entry
//!    回到 pending、attempt 不变（掉线非发送失败，不消耗重试额度）、写
//!    `agent.send_deferred_account_offline` 事件。
//! 3. account.online=true 时正常发送 → status=sent。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）；本地无 Docker 标"待 CI"，
//! CI 用 `cargo test --test account_offline_defer_integration -- --ignored` 触发。

mod common;

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{
    atomic_claim_pending, enqueue, process_entry, EnqueueOutcome, EnqueueRequest, OutboxStatus,
};
use wechatagent::models::{Contact, WechatAccount};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const APP_ID: &str = "wx_app_offline_test";

/// 构造一个 managed contact（process_entry 经 P1-6 发送前状态门拦非 Managed）。
fn make_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("掉线 defer 测试客户".to_string()),
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
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

/// 构造一个 wechat_accounts 行（account_id=default 对齐 enqueue_request）。
fn make_account(online: bool) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        alias: "offline_test".to_string(),
        display_name: "掉线 defer 测试账号".to_string(),
        app_id: Some(APP_ID.to_string()),
        wxid: Some("wxid_account_self".to_string()),
        nick_name: None,
        avatar_url: None,
        mcp_base_url: None,
        mcp_api_key: None,
        online,
        status: None,
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

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

fn enqueue_request(run_id: &str, source_event_id: &str, contact_wxid: &str) -> EnqueueRequest {
    EnqueueRequest {
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: contact_wxid.to_string(),
        run_id: run_id.to_string(),
        decision_id: None,
        source_event_id: source_event_id.to_string(),
        source_kind: "inbound_message".to_string(),
        content: "你好，这是掉线 defer 集成测试的内容。".to_string(),
        media_asset_id: None,
        referral_card_id: None,
        max_attempts: 3,
    }
}

// ── Case 1: webhook Offline/Online 事件落库 online 状态 ─────────────────────

#[tokio::test]
#[ignore]
async fn webhook_offline_event_persists_online_false() {
    let app = common::TestApp::start().await;
    let state = app.state.clone();

    state
        .db
        .accounts()
        .insert_one(&make_account(true), None)
        .await
        .expect("insert account");

    // Offline 事件 → online=false
    let body = Bytes::from(
        serde_json::to_vec(&json!({ "TypeName": "Offline", "Appid": APP_ID }))
            .expect("serialize offline payload"),
    );
    let resp = wechatagent::webhooks::wechat_webhook(State(state.clone()), HeaderMap::new(), body)
        .await
        .expect("offline webhook ok");
    assert_eq!(
        resp.0.get("ignored").and_then(|v| v.as_str()),
        Some("offline_event")
    );

    let acc = state
        .db
        .accounts()
        .find_one(doc! { "app_id": APP_ID }, None)
        .await
        .expect("query account")
        .expect("account exists");
    assert!(!acc.online, "Offline 事件后 online 必须落为 false");

    // Online 事件 → 对称落 online=true
    let body = Bytes::from(
        serde_json::to_vec(&json!({ "TypeName": "Online", "Appid": APP_ID }))
            .expect("serialize online payload"),
    );
    let resp = wechatagent::webhooks::wechat_webhook(State(state.clone()), HeaderMap::new(), body)
        .await
        .expect("online webhook ok");
    assert_eq!(
        resp.0.get("ignored").and_then(|v| v.as_str()),
        Some("online_event")
    );

    let acc = state
        .db
        .accounts()
        .find_one(doc! { "app_id": APP_ID }, None)
        .await
        .expect("query account")
        .expect("account exists");
    assert!(acc.online, "Online 事件后 online 必须落回 true");
}

// ── Case 2: account.online=false → dispatcher defer，不调 MCP、不耗 attempt ──

#[tokio::test]
#[ignore]
async fn offline_account_defers_without_consuming_attempt() {
    let app = common::TestApp::start().await;
    // MCP mock 成功 responder：用来证明 defer 路径根本没有调用它。
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());

    // account 掉线（online=false）。
    state
        .db
        .accounts()
        .insert_one(&make_account(false), None)
        .await
        .expect("insert offline account");

    let contact = make_contact("user_offline_defer");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outcome = enqueue(
        &state,
        enqueue_request("run_offline", "evt_offline_1", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    let claimed = atomic_claim_pending(&state, "worker_offline", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    assert_eq!(claimed.attempt, 0, "首次抢占 attempt=0");

    // 基准：defer 调用前的时刻。defer 会把 next_retry_at 推后
    // ACCOUNT_OFFLINE_DEFER_SECONDS(60s)，故 defer 后的 next_retry_at 必然 > 此刻；
    // 而 enqueue 时 next_retry_at=None——以此坐实"推后"语义（去永真断言）。
    let before_defer = DateTime::now();

    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    // defer：entry 回 pending、attempt 不变、next_retry_at 被推后、未发送。
    let entry = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query outbox")
        .expect("entry exists");
    assert_eq!(
        entry.status,
        OutboxStatus::Pending.as_str(),
        "掉线 defer 后应回到 pending（等恢复重试），got {:?}",
        entry
    );
    assert_eq!(
        entry.attempt, 0,
        "defer 不消耗 attempt（掉线非发送失败），got {}",
        entry.attempt
    );
    assert!(entry.sent_at.is_none(), "defer 不应标记 sent_at");
    let next_retry_at = entry
        .next_retry_at
        .expect("defer 应推后 next_retry_at 等账号恢复");
    assert!(
        next_retry_at > before_defer,
        "defer 必须把 next_retry_at 推后到 defer 时刻之后（推后 60s），got next_retry_at={:?} before_defer={:?}",
        next_retry_at,
        before_defer
    );

    // 写了 defer 审计事件（AI 自治措辞）。
    let evt_count = state
        .db
        .events()
        .count_documents(
            doc! { "kind": "agent.send_deferred_account_offline" },
            None,
        )
        .await
        .expect("count events");
    assert!(
        evt_count >= 1,
        "应写一条 agent.send_deferred_account_offline 事件"
    );

    // 关键：掉线 defer 路径绝不调用 MCP（不盲发）。
    assert_eq!(
        mcp_server.received_requests().await.unwrap().len(),
        0,
        "账号掉线时不得调用 MCP 发送"
    );
}

// ── Case 3: account.online=true → 正常发送 → sent ──────────────────────────

#[tokio::test]
#[ignore]
async fn online_account_sends_normally() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());

    state
        .db
        .accounts()
        .insert_one(&make_account(true), None)
        .await
        .expect("insert online account");

    let contact = make_contact("user_online_send");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outcome = enqueue(
        &state,
        enqueue_request("run_online", "evt_online_1", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    let claimed = atomic_claim_pending(&state, "worker_online", 60)
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
        "account online 时应正常发送 sent，got {:?}",
        entry
    );
    assert!(entry.sent_at.is_some(), "sent_at must be populated");
}

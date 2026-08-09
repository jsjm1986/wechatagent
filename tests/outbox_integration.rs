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

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use axum::{extract::State, routing::post, Json, Router};
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{
    atomic_claim_pending, cancel_entry, cancel_for_contact_on_user_reaction, enqueue,
    handle_managed_message, process_entry, reclaim_expired_leases, run_outbox_dispatcher,
    second_safety_gate, EnqueueOutcome, EnqueueRequest, OutboxStatus,
};
use wechatagent::models::{Contact, ConversationMessage, MessageDirection, ReferralCard};
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

/// Seed the account identity required by the scoped MCP resolver. The mock URL
/// and key intentionally remain deployment-level defaults for the default
/// workspace; this row proves the account exists without weakening the
/// production fail-closed boundary for unknown accounts.
async fn seed_default_mcp_account(state: &wechatagent::routes::AppState) {
    common::ensure_test_account(state, "default", "default").await;
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

/// 按 tool 名分派：`chat_search` 返回**命中** items（供 reclaim 权威核对判 true），
/// 其它 tool（含 `message_send_text`）返回唯一 newMsgId 成功 envelope。
///
/// items 形状严格对齐 `src/mcp.rs::chat_search_hit`（亲验 :772-791）：每条须含
/// `content`（**精确等于** entry.content，非子串）+ `createdAt`（ISO-8601 rfc3339
/// 字符串，其 millis ≥ since=entry.created_at）。`chat_search_outbound`（:819-824）从
/// `call_tool_with_key` 剥壳后的 structuredContent 顶层取 `items`，故 envelope 放在
/// result.structuredContent.items。createdAt 用 respond 时刻（发生在 entry 入队之后，
/// 故必 ≥ since）。
struct ChatSearchHitResponder {
    counter: std::sync::atomic::AtomicU64,
    hit_content: String,
}

impl wiremock::Respond for ChatSearchHitResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let tool = serde_json::from_slice::<serde_json::Value>(&request.body)
            .ok()
            .and_then(|v| {
                v.pointer("/params/name")
                    .and_then(|n| n.as_str())
                    .map(String::from)
            })
            .unwrap_or_default();
        if tool == "chat_search" {
            let created_at = DateTime::now()
                .try_to_rfc3339_string()
                .unwrap_or_else(|_| "2099-01-01T00:00:00Z".to_string());
            let body = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": { "structuredContent": {
                    "items": [ { "content": self.hit_content, "createdAt": created_at } ],
                    "count": 1
                }}
            });
            return ResponseTemplate::new(200).set_body_json(body);
        }
        let seq = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "structuredContent": { "newMsgId": format!("mock_msg_id_{seq}"), "content": [] }}
        });
        ResponseTemplate::new(200).set_body_json(body)
    }
}

/// 按 tool 名分派：`chat_search` 返回 HTTP 500（模拟权威通道不可达 →
/// `verify_already_sent` 走 `Ok(Err(_))` 分支回落本地 mcp_call_logs 核对），其它 tool
/// 返回唯一 newMsgId 成功 envelope。供 `reclaim_gate_precedes_pacing_gate` 复用其
/// step(e) 预置的本地"已发过"证据（chat_search 命中子路径由专门哨兵测覆盖）。
struct ChatSearchErrDispatchResponder {
    counter: std::sync::atomic::AtomicU64,
}

impl wiremock::Respond for ChatSearchErrDispatchResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let tool = serde_json::from_slice::<serde_json::Value>(&request.body)
            .ok()
            .and_then(|v| {
                v.pointer("/params/name")
                    .and_then(|n| n.as_str())
                    .map(String::from)
            })
            .unwrap_or_default();
        if tool == "chat_search" {
            return ResponseTemplate::new(500).set_body_string("simulated chat_search outage");
        }
        let seq = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "structuredContent": { "newMsgId": format!("mock_msg_id_{seq}"), "content": [] }}
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

/// 初始化握手即返回 500：客户投递请求尚未发出，属于可证明的安全重试，
/// 用于覆盖 retry-then-terminal 路径。
async fn start_mcp_mock_failure() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(500).set_body_string("simulated mcp failure"))
        .mount(&server)
        .await;
    server
}

/// 初始化成功，但真正的 message_send_text 返回 HTTP 500；chat_search 也不可用。
/// 这模拟“客户投递请求可能已被远端接收，但本地没有可信回执”的歧义边界。
struct AmbiguousSendResponder;

impl wiremock::Respond for AmbiguousSendResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        if body.get("method").and_then(|v| v.as_str()) == Some("initialize") {
            return ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": { "protocolVersion": "2024-11-05", "capabilities": {} }
            }));
        }
        ResponseTemplate::new(500).set_body_string("ambiguous failure after request boundary")
    }
}

async fn start_mcp_mock_ambiguous_send() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(AmbiguousSendResponder)
        .mount(&server)
        .await;
    server
}

#[derive(Clone)]
struct BlockingMcpState {
    reached_tx: tokio::sync::watch::Sender<usize>,
    release: Arc<tokio::sync::Notify>,
    send_calls: Arc<AtomicUsize>,
}

struct BlockingMcpServer {
    base_url: String,
    reached_rx: tokio::sync::watch::Receiver<usize>,
    release: Arc<tokio::sync::Notify>,
    send_calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

async fn blocking_mcp_handler(
    State(state): State<BlockingMcpState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let id = body.get("id").cloned().unwrap_or_else(|| json!(1));
    if body.get("method").and_then(|value| value.as_str()) == Some("initialize") {
        return Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "protocolVersion": "2024-11-05", "capabilities": {} }
        }));
    }

    let tool_name = body
        .pointer("/params/name")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let sequence = if tool_name.starts_with("message_send_") {
        let sequence = state.send_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = state.reached_tx.send(sequence);
        state.release.notified().await;
        sequence
    } else {
        0
    };
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "structuredContent": {
                "newMsgId": format!("blocking_mcp_msg_{sequence}"),
                "content": []
            }
        }
    }))
}

async fn start_blocking_mcp_server() -> BlockingMcpServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind blocking MCP server");
    let address = listener.local_addr().expect("blocking MCP address");
    let (reached_tx, reached_rx) = tokio::sync::watch::channel(0usize);
    let release = Arc::new(tokio::sync::Notify::new());
    let send_calls = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route("/mcp", post(blocking_mcp_handler))
        .with_state(BlockingMcpState {
            reached_tx,
            release: release.clone(),
            send_calls: send_calls.clone(),
        });
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve blocking MCP server");
    });
    BlockingMcpServer {
        base_url: format!("http://{address}"),
        reached_rx,
        release,
        send_calls,
        task,
    }
}

async fn wait_until_remote_received_send(server: &mut BlockingMcpServer) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while *server.reached_rx.borrow() == 0 {
            server
                .reached_rx
                .changed()
                .await
                .expect("blocking MCP watch sender alive");
        }
    })
    .await
    .expect("remote MCP did not receive send request in time");
}

/// HTTP/JSON-RPC 均成功，但业务信封显式 `ok=false` 且没有 `newMsgId`。
/// 该形态用于验证 dispatcher 是否把“收到响应”误当成“消息已送达”。
async fn start_mcp_mock_negative_receipt() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "structuredContent": {
                    "ok": false,
                    "content": []
                }
            }
        })))
        .mount(&server)
        .await;
    server
}

/// HTTP/JSON-RPC 成功，但业务信封既无显式 `ok` 也无稳定 `newMsgId`。
/// 请求已经越过远端边界，缺少可信回执不能被解释成“明确未送达”。
async fn start_mcp_mock_inconclusive_receipt() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "structuredContent": {
                    "content": []
                }
            }
        })))
        .mount(&server)
        .await;
    server
}

/// 统计 wiremock 收到的真实"发送"调用数（JSON-RPC method==tools/call）。
/// MCP Streamable-HTTP 每个新会话首次调用前先发一次 `initialize` 握手，那是会话
/// 建立、不是发送；用原始 received_requests().len() 当发送数会把握手误算进去。
fn count_tool_calls(requests: &[wiremock::Request]) -> usize {
    requests
        .iter()
        .filter(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body)
                .ok()
                .and_then(|v| {
                    v.get("method")
                        .and_then(|m| m.as_str())
                        .map(|s| s == "tools/call")
                })
                .unwrap_or(false)
        })
        .count()
}

fn count_named_tool_calls(requests: &[wiremock::Request], tool_name: &str) -> usize {
    requests
        .iter()
        .filter(|request| {
            serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| {
                    if body.get("method").and_then(|value| value.as_str()) != Some("tools/call") {
                        return None;
                    }
                    body.pointer("/params/name")
                        .and_then(|value| value.as_str())
                        .map(|name| name == tool_name)
                })
                .unwrap_or(false)
        })
        .count()
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

fn enqueue_request_with_content(
    run_id: &str,
    source_event_id: &str,
    contact_wxid: &str,
    content: &str,
) -> EnqueueRequest {
    let mut request = enqueue_request(run_id, source_event_id, contact_wxid);
    request.content = content.to_string();
    request
}

// ── Case 1: 入队 → claim → MCP 成功 → sent ──────────────────────────────

/// Durable enqueue should wake the real dispatcher loop instead of waiting for its five-second
/// recovery poll. This covers enqueue -> process-local Notify -> Mongo claim -> MCP -> sent.
#[tokio::test]
#[ignore]
async fn durable_enqueue_wakes_dispatcher_without_poll_delay() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    seed_default_mcp_account(&state).await;

    let contact = make_contact("user_notify_fast_path");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let dispatcher_state = state.clone();
    let dispatcher = tokio::spawn(async move {
        run_outbox_dispatcher(dispatcher_state)
            .await
            .expect("dispatcher loop should not exit");
    });
    // Let the initial empty tick finish so the worker is waiting on Notify / five-second fallback.
    tokio::time::sleep(Duration::from_millis(250)).await;

    let wall_started = std::time::Instant::now();
    let outbox_id = match enqueue(
        &state,
        enqueue_request(
            "run_notify_fast_path",
            "evt_notify_fast_path",
            &contact.wxid,
        ),
    )
    .await
    .expect("enqueue")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    let entry = tokio::time::timeout(
        Duration::from_secs(2),
        common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(2)),
    )
    .await
    .expect("notify fast path must beat the five-second poll");
    assert_eq!(entry.status, OutboxStatus::Sent.as_str());
    let send_started_at = entry.send_started_at.expect("send_started_at");
    let claim_delay_ms = send_started_at.timestamp_millis() - entry.created_at.timestamp_millis();
    assert!(
        claim_delay_ms < 1_000,
        "dispatcher should claim within one second after enqueue, actual={claim_delay_ms}ms"
    );
    assert!(
        wall_started.elapsed() < Duration::from_secs(2),
        "end-to-end mock delivery should not wait for the five-second fallback"
    );

    dispatcher.abort();
    let _ = dispatcher.await;
    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn happy_path_enqueue_claim_send_sent() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    seed_default_mcp_account(&state).await;

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

/// HTTP/JSON-RPC 成功但业务回执 `ok=false` 时，必须按发送失败重试，不能记 sent。
#[tokio::test]
#[ignore]
async fn negative_mcp_receipt_is_retried_without_outbound_record() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_negative_receipt().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());

    let contact = make_contact("audit_negative_receipt");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outbox_id = match enqueue(
        &state,
        enqueue_request(
            "audit_run_negative_receipt",
            "audit_evt_negative",
            &contact.wxid,
        ),
    )
    .await
    .expect("enqueue")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "audit-negative-receipt", 60)
        .await
        .expect("claim")
        .expect("entry claimed");
    process_entry(&state, &claimed)
        .await
        .expect("process entry");

    let stored = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query outbox")
        .expect("outbox exists");
    let outbound_count = state
        .db
        .messages()
        .count_documents(
            doc! {
                "contact_wxid": &contact.wxid,
                "direction": "outbound",
                "content": &claimed.content,
            },
            None,
        )
        .await
        .expect("count outbound records");

    assert_eq!(
        stored.status,
        OutboxStatus::Pending.as_str(),
        "否定业务回执必须进入重试"
    );
    assert_eq!(stored.attempt, 1);
    assert!(stored.sent_at.is_none());
    assert_eq!(
        outbound_count, 0,
        "未获成功凭据不得写 outbound conversation record"
    );
}

/// 名片请求已经到达 MCP，但成功 HTTP 信封缺少可信送达字段时，不能自动重放。
/// 名片没有权威 post-hoc 查询，因此必须收敛到 `delivery_unknown` 并等待人工核验。
#[tokio::test]
#[ignore]
async fn delivery_redline_namecard_inconclusive_receipt_is_not_replayed() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_inconclusive_receipt().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    seed_default_mcp_account(&state).await;
    let contact = make_contact("namecard_inconclusive_receipt");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let card_id = ObjectId::new();
    let now = DateTime::now();
    state
        .db
        .referral_cards()
        .insert_one(
            ReferralCard {
                id: Some(card_id),
                workspace_id: contact.workspace_id.clone(),
                account_id: Some(contact.account_id.clone()),
                target_wxid: "wxid_inconclusive_advisor".to_string(),
                display_name: "Inconclusive advisor".to_string(),
                send_trigger_hint: "receipt redline".to_string(),
                target_stages: vec![],
                tags: vec![],
                enabled: true,
                review_status: "approved".to_string(),
                review_note: None,
                created_at: now,
                updated_at: now,
            },
            None,
        )
        .await
        .expect("insert referral card");

    let mut request = enqueue_request(
        "run_namecard_inconclusive_receipt",
        "evt_namecard_inconclusive_receipt",
        &contact.wxid,
    );
    request.content.clear();
    request.referral_card_id = Some(card_id.to_hex());
    let outbox_id = match enqueue(&state, request).await.expect("enqueue namecard") {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker-namecard-inconclusive", 60)
        .await
        .expect("claim namecard")
        .expect("namecard entry claimed");
    process_entry(&state, &claimed)
        .await
        .expect("process namecard");

    let stored = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(stored.status, OutboxStatus::DeliveryUnknown.as_str());
    assert_eq!(stored.attempt, 0, "不确定送达不得排入自动重试");
    assert!(stored
        .last_error
        .as_deref()
        .unwrap_or_default()
        .contains("automatic replay disabled"));
    assert!(
        atomic_claim_pending(&state, "must-not-replay-inconclusive-card", 60)
            .await
            .expect("post-receipt claim")
            .is_none()
    );

    let outbound_count = state
        .db
        .messages()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "direction": "outbound",
                "msg_type": "namecard",
            },
            None,
        )
        .await
        .expect("count outbound namecard records");
    assert_eq!(outbound_count, 0, "不可信回执不得伪记已送达名片");

    let requests = mcp_server
        .received_requests()
        .await
        .expect("received MCP requests");
    assert_eq!(
        count_named_tool_calls(&requests, "message_send_namecard"),
        1,
        "名片物理发送请求只能发生一次"
    );
}

/// 客户投递请求发出后收到 HTTP 500 时，不能把“无成功日志”当成“确认未送达”。
/// 条目必须进入 delivery_unknown，且后续 claim 不得造成第二次客户投递。
#[tokio::test]
#[ignore]
async fn delivery_redline_ambiguous_http_failure_is_not_automatically_replayed() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_ambiguous_send().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    seed_default_mcp_account(&state).await;
    let contact = make_contact("audit_ambiguous_http");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outbox_id = match enqueue(
        &state,
        enqueue_request("audit_run_ambiguous", "audit_evt_ambiguous", &contact.wxid),
    )
    .await
    .expect("enqueue")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "audit-ambiguous", 60)
        .await
        .expect("claim")
        .expect("entry claimed");
    process_entry(&state, &claimed)
        .await
        .expect("process entry");

    let stored = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(stored.status, OutboxStatus::DeliveryUnknown.as_str());
    assert_eq!(stored.attempt, 0, "歧义结果不得消耗后自动重放");
    assert!(atomic_claim_pending(&state, "must-not-replay", 60)
        .await
        .expect("second claim")
        .is_none());

    let requests = mcp_server
        .received_requests()
        .await
        .expect("received requests");
    assert_eq!(
        count_named_tool_calls(&requests, "message_send_text"),
        1,
        "客户投递请求只能发生一次；chat_search 核验调用不计为发送"
    );
}

/// 对同一 run 制造一条 sent + 一条 canceled，并返回 run log 最终 outbox_status。
/// `cancel_first=true` 时 sent 最后发生；false 时 canceled 最后发生。
async fn mixed_run_status_after_ordered_transitions(
    state: &wechatagent::routes::AppState,
    contact: &Contact,
    run_id: &str,
    cancel_first: bool,
) -> String {
    state
        .db
        .raw()
        .collection::<Document>("agent_run_logs")
        .insert_one(
            doc! {
                "run_id": run_id,
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "outbox_status": null,
                "created_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("seed run log");

    let cancel_one = async {
        let cancel_id = match enqueue(
            state,
            enqueue_request_with_content(
                run_id,
                &format!("{run_id}-cancel"),
                &contact.wxid,
                "本条用于取消状态复现",
            ),
        )
        .await
        .expect("enqueue cancel entry")
        {
            EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
            other => panic!("expected Created, got {other:?}"),
        };
        let claimed = atomic_claim_pending(state, &format!("{run_id}-cancel-worker"), 60)
            .await
            .expect("claim cancel entry")
            .expect("cancel entry claimed");
        assert_eq!(claimed.id, Some(cancel_id));
        cancel_entry(state, cancel_id, &claimed, "audit_mixed_run_cancel")
            .await
            .expect("cancel entry");
    };

    let send_one = async {
        let send_id = match enqueue(
            state,
            enqueue_request_with_content(
                run_id,
                &format!("{run_id}-send"),
                &contact.wxid,
                "本条用于送达状态复现",
            ),
        )
        .await
        .expect("enqueue send entry")
        {
            EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
            other => panic!("expected Created, got {other:?}"),
        };
        let claimed = atomic_claim_pending(state, &format!("{run_id}-send-worker"), 60)
            .await
            .expect("claim send entry")
            .expect("send entry claimed");
        assert_eq!(claimed.id, Some(send_id));
        process_entry(state, &claimed).await.expect("send entry");
    };

    if cancel_first {
        cancel_one.await;
        send_one.await;
    } else {
        send_one.await;
        cancel_one.await;
    }

    let sent_count = state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! { "run_id": run_id, "status": "sent" }, None)
        .await
        .expect("count sent entries");
    let canceled_count = state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! { "run_id": run_id, "status": "canceled" }, None)
        .await
        .expect("count canceled entries");
    assert_eq!((sent_count, canceled_count), (1, 1));

    state
        .db
        .raw()
        .collection::<Document>("agent_run_logs")
        .find_one(doc! { "run_id": run_id }, None)
        .await
        .expect("query run log")
        .expect("run log exists")
        .get_str("outbox_status")
        .expect("outbox_status string")
        .to_string()
}

/// 相同的 run 级事实集合（1 sent + 1 canceled）必须与处理顺序无关。
#[tokio::test]
#[ignore]
async fn mixed_run_status_is_order_independent() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    seed_default_mcp_account(&state).await;

    let contact = make_contact("audit_mixed_run_status");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let sent_last =
        mixed_run_status_after_ordered_transitions(&state, &contact, "audit_mixed_sent_last", true)
            .await;
    let canceled_last = mixed_run_status_after_ordered_transitions(
        &state,
        &contact,
        "audit_mixed_canceled_last",
        false,
    )
    .await;

    assert_eq!(sent_last, "partially_sent");
    assert_eq!(canceled_last, "partially_sent");
    assert_eq!(sent_last, canceled_last);
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

    let canceled = cancel_for_contact_on_user_reaction(
        &state,
        &contact.workspace_id,
        &contact.account_id,
        &contact.wxid,
    )
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

/// worker 已 claim、但尚未越过 MCP 边界时，用户停止请求必须赢得最后一次 CAS。
/// process_entry 使用旧 claim 快照继续运行也不得调用 message_send_text。
#[tokio::test]
#[ignore]
async fn delivery_redline_in_flight_stop_request_fences_remote_send() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    seed_default_mcp_account(&state).await;
    let contact = make_contact("user_stop_after_claim");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outbox_id = match enqueue(
        &state,
        enqueue_request(
            "run_stop_after_claim",
            "evt_stop_after_claim",
            &contact.wxid,
        ),
    )
    .await
    .expect("enqueue")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker-before-stop", 60)
        .await
        .expect("claim")
        .expect("entry claimed");
    assert_eq!(claimed.id, Some(outbox_id));

    let accepted = cancel_for_contact_on_user_reaction(
        &state,
        &contact.workspace_id,
        &contact.account_id,
        &contact.wxid,
    )
    .await
    .expect("persist stop request");
    assert_eq!(accepted, 1);
    process_entry(&state, &claimed)
        .await
        .expect("stale owner must stop safely");

    let stored = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(stored.status, OutboxStatus::Canceled.as_str());
    assert_eq!(
        stored.cancel_reason.as_deref(),
        Some("user_reaction_stop_requested")
    );
    let requests = mcp_server
        .received_requests()
        .await
        .expect("received requests");
    assert_eq!(count_named_tool_calls(&requests, "message_send_text"), 0);
}

/// The remote endpoint has received the customer send request, but has not replied yet.
/// A cancellation at this point is best-effort only: a later success receipt must settle to
/// `sent`, preserve the cancellation audit marker, and never trigger a second physical send.
#[tokio::test]
#[ignore]
async fn delivery_redline_late_cancel_after_remote_acceptance_settles_sent_once() {
    let app = common::TestApp::start().await;
    let mut mcp_server = start_blocking_mcp_server().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.base_url.clone());
    seed_default_mcp_account(&state).await;
    let contact = make_contact("user_stop_after_remote_acceptance");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let outbox_id = match enqueue(
        &state,
        enqueue_request(
            "run_stop_after_remote_acceptance",
            "evt_stop_after_remote_acceptance",
            &contact.wxid,
        ),
    )
    .await
    .expect("enqueue")
    {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker-late-cancel", 60)
        .await
        .expect("claim")
        .expect("entry claimed");
    let worker_state = state.clone();
    let worker = tokio::spawn(async move { process_entry(&worker_state, &claimed).await });

    wait_until_remote_received_send(&mut mcp_server).await;
    let crossed_boundary = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query crossed-boundary entry")
        .expect("entry exists");
    assert_eq!(crossed_boundary.status, OutboxStatus::InFlight.as_str());
    assert!(
        crossed_boundary.send_started_at.is_some(),
        "remote request was observed, so the durable boundary marker must exist"
    );

    let accepted = cancel_for_contact_on_user_reaction(
        &state,
        &contact.workspace_id,
        &contact.account_id,
        &contact.wxid,
    )
    .await
    .expect("persist late cancellation request");
    assert_eq!(accepted, 1);
    let cancel_pending = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query cancellation request")
        .expect("entry exists");
    assert_eq!(cancel_pending.status, OutboxStatus::InFlight.as_str());
    assert!(cancel_pending.cancel_requested);

    mcp_server.release.notify_one();
    worker
        .await
        .expect("dispatcher task join")
        .expect("dispatcher result");
    let stored = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(stored.status, OutboxStatus::Sent.as_str());
    assert!(
        stored.cancel_requested,
        "late cancellation remains auditable"
    );
    assert_eq!(
        stored.cancel_reason.as_deref(),
        Some("user_reaction_stop_requested")
    );
    assert_eq!(mcp_server.send_calls.load(Ordering::SeqCst), 1);
    assert!(
        atomic_claim_pending(&state, "must-not-replay-late-cancel", 60)
            .await
            .expect("post-send claim")
            .is_none(),
        "a late cancellation plus success receipt must not create a replay"
    );
    let outbound_count = state
        .db
        .messages()
        .count_documents(
            doc! {
                "contact_wxid": &contact.wxid,
                "direction": "outbound",
            },
            None,
        )
        .await
        .expect("count outbound records");
    assert_eq!(
        outbound_count, 1,
        "delivery side effects must finalize once"
    );
    mcp_server.task.abort();
}

/// A namecard request reached the remote endpoint and the worker then crashed before a receipt.
/// Since namecards have no authoritative post-hoc lookup, lease recovery must stop in
/// `delivery_unknown`; returning to pending would permit a duplicate physical card send.
#[tokio::test]
#[ignore]
async fn delivery_redline_namecard_crash_after_remote_boundary_is_not_replayed() {
    let app = common::TestApp::start().await;
    let mut mcp_server = start_blocking_mcp_server().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.base_url.clone());
    seed_default_mcp_account(&state).await;
    let contact = make_contact("namecard_crash_after_remote_boundary");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    let card_id = ObjectId::new();
    let now = DateTime::now();
    state
        .db
        .referral_cards()
        .insert_one(
            ReferralCard {
                id: Some(card_id),
                workspace_id: contact.workspace_id.clone(),
                account_id: Some(contact.account_id.clone()),
                target_wxid: "wxid_test_advisor".to_string(),
                display_name: "Test advisor".to_string(),
                send_trigger_hint: "integration redline".to_string(),
                target_stages: vec![],
                tags: vec![],
                enabled: true,
                review_status: "approved".to_string(),
                review_note: None,
                created_at: now,
                updated_at: now,
            },
            None,
        )
        .await
        .expect("insert approved referral card");

    let mut request = enqueue_request(
        "run_namecard_crash_boundary",
        "evt_namecard_crash_boundary",
        &contact.wxid,
    );
    request.content.clear();
    request.referral_card_id = Some(card_id.to_hex());
    let outbox_id = match enqueue(&state, request).await.expect("enqueue namecard") {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let claimed = atomic_claim_pending(&state, "worker-namecard-crash", 60)
        .await
        .expect("claim namecard")
        .expect("namecard entry claimed");
    let worker_state = state.clone();
    let worker = tokio::spawn(async move { process_entry(&worker_state, &claimed).await });

    wait_until_remote_received_send(&mut mcp_server).await;
    let crossed_boundary = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query namecard boundary")
        .expect("entry exists");
    assert!(crossed_boundary.send_started_at.is_some());
    assert_eq!(mcp_server.send_calls.load(Ordering::SeqCst), 1);

    worker.abort();
    let _ = worker.await;
    mcp_server.release.notify_one();
    state
        .db
        .collection_agent_send_outbox()
        .update_one(
            doc! { "_id": outbox_id },
            doc! { "$set": { "locked_until": DateTime::from_millis(0) } },
            None,
        )
        .await
        .expect("expire crashed worker lease");
    let reclaimed = reclaim_expired_leases(&state)
        .await
        .expect("reclaim crashed namecard worker");
    assert_eq!(reclaimed, 1);

    let stored = common::wait_for_outbox_processed(&state, outbox_id, Duration::from_secs(5)).await;
    assert_eq!(stored.status, OutboxStatus::DeliveryUnknown.as_str());
    assert!(stored
        .last_error
        .as_deref()
        .unwrap_or_default()
        .contains("manual verification"));
    assert!(atomic_claim_pending(&state, "must-not-replay-namecard", 60)
        .await
        .expect("post-crash claim")
        .is_none());
    assert_eq!(
        mcp_server.send_calls.load(Ordering::SeqCst),
        1,
        "namecard physical send must not be replayed after an uncertain crash"
    );
    mcp_server.task.abort();
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
    seed_default_mcp_account(&state).await;

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

    let reclaimed = reclaim_expired_leases(&state).await.expect("reclaim ok");
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
    seed_default_mcp_account(&state).await;

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
        match enqueue(&state, req.clone()).await.expect("enqueue ok") {
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
        process_entry(&state, &claimed).await.expect("process ok");
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
    let mcp_calls = count_tool_calls(&recv);
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
    seed_default_mcp_account(&state).await;
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
    seed_default_mcp_account(&state).await;
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
    assert_eq!(
        warns, 0,
        "未达软上限不应记 warning 事件，实际 count={warns}"
    );
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
    assert!(
        entry.worker_id.is_none(),
        "worker_id 应被清空（放回 pending）"
    );
    assert!(entry.locked_until.is_none(), "locked_until 应被清空");

    // 写了 pacing deferred 事件。
    let deferred = state
        .db
        .events()
        .count_documents(doc! { "kind": "agent.send_deferred_account_pacing" }, None)
        .await
        .expect("count events");
    assert!(
        deferred >= 1,
        "应写 pacing deferred 事件，实际 count={deferred}"
    );

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
    seed_default_mcp_account(&state).await;
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
    assert_eq!(count_tool_calls(&recv), 1, "间隔已过，第二条应正常发往 MCP");
}

/// 账号闸：不同账号互不影响（账号 A 刚发不拦账号 B）。
#[tokio::test]
#[ignore]
async fn account_pacing_gate_isolates_accounts() {
    let app = common::TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    seed_default_mcp_account(&state).await;
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
    seed_default_mcp_account(&state).await;
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
    assert_eq!(
        claimed.id,
        Some(outbox_id),
        "claim 到的应是 gateway 入队那条"
    );
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
    assert!(
        entry.worker_id.is_none(),
        "worker_id 应被清空（放回 pending）"
    );
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

// ── Case 13: reclaim 幂等门(2B) vs 账号级发送间隔闸 相对执行顺序 ──────────────
//
// process_entry 里 reclaim 幂等门 post-hoc 核对(:645)在账号 pacing 节流闸(:719)
// **之前**（源码注释 :721 明写此序）。上面 4 个 account_pacing_gate_* 只验闸
// 本身（拦/放行/隔离/fail-soft），没有锁定这两道门的先后。
//
// 命门：一条 reclaimed_in_flight=true 且 MCP 已经发过、又恰好落在 pacing 间隔内
// 的 entry，必须走 2B post-hoc 标 sent（因为消息其实已送达客户），而**不能**被
// pacing 先拦成 pending——否则这条"已发过"的消息会永远卡在 pending 里成僵尸条目
// （每轮 claim 又被 pacing 拦，永远发不出也不 sent）。若把 pacing 闸误挪到 2B 之前，
// 本例 status 会变成 pending → 测试红，从而把"reclaim 门在 pacing 之前"这条不变量
// 锁死在测试里。

/// reclaim 门 vs pacing 闸相对序：reclaimed_in_flight + MCP 已发过 + pacing 命中
/// → 走 2B post-hoc 标 sent（非被 pacing reschedule 成僵尸 pending）。
#[tokio::test]
#[ignore]
async fn reclaim_gate_precedes_pacing_gate() {
    let app = common::TestApp::start().await;
    // F-01：reclaim text 路先查权威 chat_search。本测验的是"reclaim 门先于 pacing 闸"，
    // 复用 step(e) 预置的**本地** mcp_call_logs 证据，故让 chat_search 返回 500 →
    // verify_already_sent 走 Ok(Err(_)) 分支回落本地核对命中（chat_search 命中子路径由
    // reclaim_text_verifies_via_chat_search_before_local 专门覆盖）。其它 tool 仍返回唯一
    // newMsgId，若误真发可被计数。
    let mcp_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ChatSearchErrDispatchResponder {
            counter: std::sync::atomic::AtomicU64::new(0),
        })
        .mount(&mcp_server)
        .await;
    let mut state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    // 开 pacing 闸（固定 2s，min=max 消除随机性）：默认 0/0 是关的，必须显式覆盖。
    // 这样若 2B 门不先命中、执行落到 pacing，就会被拦成 pending，暴露顺序错误。
    state.config.account_send_min_interval_ms = 2000;
    state.config.account_send_max_interval_ms = 2000;

    let contact = make_contact("user_reclaim_before_pacing");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // (a) 预置一条同账号「刚刚 sent」(sent_at=now) 的历史 → account_last_sent_at_ms
    // 返回 now，把 pacing 闸的命中前置条件做实（若执行到 pacing 必被拦）。
    state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox")
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "contact_wxid": &contact.wxid,
                "run_id": "seed_reclaim_pacing",
                "source_event_id": format!("seed_evt_{}", ObjectId::new()),
                "source_kind": "inbound_message",
                "status": "sent",
                "sent_at": DateTime::now(),
                "content": "刚发过的历史一条（arms pacing）",
                "content_hash": "seed_hash_reclaim_pacing",
                "idempotency_key": format!("seed_{}", ObjectId::new()),
                "created_at": DateTime::now(),
                "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("seed sent history to arm pacing");

    // (b) enqueue 目标 pending 条目（content 来自 enqueue_request，text 版无
    // media_asset_id / referral_card_id → 走 mcp_already_succeeded 文本核对分支）。
    let outcome = enqueue(
        &state,
        enqueue_request("run_reclaim_pacing", "evt_reclaim_pacing", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    // (c) 置 reclaimed_in_flight=true，模拟"上一个 worker 抢占后在写 sent 前崩溃、
    // 被 reclaim_expired_leases 放回 pending"的状态（reclaim 门只对这条跑 post-hoc）。
    let collection = state.db.collection_agent_send_outbox();
    collection
        .update_one(
            doc! { "_id": outbox_id },
            doc! { "$set": { "reclaimed_in_flight": true } },
            None,
        )
        .await
        .expect("set reclaimed_in_flight=true");

    // (d) claim（status pending → in_flight，reclaimed_in_flight 原样保留、After 返回）。
    let claimed = atomic_claim_pending(&state, "worker_reclaim_pacing", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    assert_eq!(claimed.id, Some(outbox_id));
    assert!(
        claimed.reclaimed_in_flight,
        "claim 应保留 reclaimed_in_flight=true（后续走 2B 门的前提）"
    );

    // (e) seed mcp_call_logs 一条成功记录（tool_name=message_send_text、同 recipient +
    // 同 content、error=null、created_at=now），让 mcp_already_succeeded(:521) 命中
    // → "这条其实已发过"。用 claimed.content 精确对齐 request.content，避免与
    // enqueue_request 字面量漂移导致命不中而假绿。
    state
        .db
        .raw()
        .collection::<Document>("mcp_call_logs")
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "tool_name": "message_send_text",
                "request": {
                    "recipient": &contact.wxid,
                    "content": &claimed.content,
                },
                "response": { "ok": true },
                "error": null,
                "created_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("seed successful mcp_call_logs record");

    // (f) 驱动真实 process_entry——由它自己走完整链路决定顺序（不在测试内自判顺序）。
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    // (g) 断言：走了 2B post-hoc 标 sent，而非被 pacing 拦成 pending。
    let entry = collection
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query")
        .expect("entry exists");
    assert_eq!(
        entry.status,
        OutboxStatus::Sent.as_str(),
        "reclaim 门必须在 pacing 闸之前命中：MCP 已发过的条目应走 2B post-hoc 标 sent，\
         而非被 pacing reschedule 成僵尸 pending。status={:?}",
        entry.status
    );
    assert_ne!(
        entry.status,
        OutboxStatus::Pending.as_str(),
        "若 status=pending 说明 pacing 闸被误挪到 reclaim 门之前，已发过的消息被拦成僵尸条目"
    );
    // last_error 是 reclaim 分支专属 marker——确认确实走的是 2B post-hoc，
    // 而非碰巧走真实发送（后者 last_error 为 None、且会有 MCP 调用）。
    let last_error = entry.last_error.clone().unwrap_or_default();
    assert!(
        last_error.contains("delivery was confirmed post-hoc"),
        "应走 reclaim 2B post-hoc 分支（last_error 带专属 marker），实际 last_error={last_error:?}"
    );
    assert!(entry.worker_id.is_none(), "标 sent 时 worker_id 应清空");
    assert!(
        entry.locked_until.is_none(),
        "标 sent 时 locked_until 应清空"
    );

    // (h) 2B 门标 sent 不重发 → MCP 未收到任何真实 message_send_text 发送调用。
    // F-01 修复后 reclaim text 路会先发一次 chat_search 的 tools/call（本 mock 返回 500 →
    // verify_already_sent 回落本地 mcp_already_succeeded，step(e) 已 seed 命中 → 仍标 sent）。
    // 故不再断言"零请求"（chat_search 与 initialize 握手会到达 wiremock），而是断言
    // "零 message_send_text 真实重发"——这正是 2B 门要守护的不变量。
    let recv = mcp_server
        .received_requests()
        .await
        .expect("wiremock recorded requests");
    let send_calls = recv
        .iter()
        .filter(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body)
                .ok()
                .and_then(|v| {
                    v.pointer("/params/name")
                        .and_then(|n| n.as_str())
                        .map(|s| s == "message_send_text")
                })
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        send_calls, 0,
        "2B post-hoc 确认已发过，不应再真实重发 message_send_text；send_calls={send_calls}"
    );
}

/// F-01 守门：reclaim text 路必须**先查权威 chat_search**——本地 mcp_call_logs 查不到时，
/// 只要 chat_search 命中就标 sent 不重发。若回退到"reclaim 直接查本地"（Task 1 修复前的
/// 行为）→ 本地查不到 → 真实重发 message_send_text → send_calls≥1 → 本测变红。
///
/// 构造：reclaim 分支 text entry（reclaimed_in_flight=true）+ chat_search 命中 +
/// **故意不 seed 本地 mcp_call_logs** → 断言 status=Sent 且零 message_send_text 重发
/// 且收到过 ≥1 次 chat_search 调用。enqueue/置位/claim 步骤照
/// `reclaim_gate_precedes_pacing_gate` 的 (b)(c)(d) 段复用。
#[tokio::test]
#[ignore]
async fn reclaim_text_verifies_via_chat_search_before_local() {
    let app = common::TestApp::start().await;
    seed_default_mcp_account(&app.state).await;
    let contact = make_contact("user_reclaim_chat_search");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // (b) enqueue 目标 pending 条目（text 版：无 media_asset_id / referral_card_id）。
    let outcome = enqueue(
        &app.state,
        enqueue_request("run_reclaim_search", "evt_reclaim_search", &contact.wxid),
    )
    .await
    .expect("enqueue ok");
    let outbox_id = match outcome {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("expected Created, got {other:?}"),
    };

    // (c) 置 reclaimed_in_flight=true，模拟崩溃恢复放回 pending（reclaim 门只对这条跑 post-hoc）。
    let collection = app.state.db.collection_agent_send_outbox();
    collection
        .update_one(
            doc! { "_id": outbox_id },
            doc! { "$set": { "reclaimed_in_flight": true } },
            None,
        )
        .await
        .expect("set reclaimed_in_flight=true");

    // (d) claim（status pending → in_flight，reclaimed_in_flight 原样保留）。
    let claimed = atomic_claim_pending(&app.state, "worker_reclaim_search", 60)
        .await
        .expect("claim ok")
        .expect("claimed entry");
    assert_eq!(claimed.id, Some(outbox_id));
    assert!(
        claimed.reclaimed_in_flight,
        "claim 应保留 reclaimed_in_flight=true（走 reclaim post-hoc 门的前提）"
    );

    // chat_search 命中（hit_content=claimed.content，精确对齐避免字面漂移）；其它 tool
    // 返回唯一 newMsgId。**故意不 seed mcp_call_logs**——若 reclaim 直接查本地必查不到。
    let mcp_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ChatSearchHitResponder {
            counter: std::sync::atomic::AtomicU64::new(0),
            hit_content: claimed.content.clone(),
        })
        .mount(&mcp_server)
        .await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());

    // (f) 驱动真实 process_entry。
    process_entry(&state, &claimed)
        .await
        .expect("process entry ok");

    // (g) chat_search 命中即标 sent（不重发）。
    let entry = collection
        .find_one(doc! { "_id": outbox_id }, None)
        .await
        .expect("query")
        .expect("entry exists");
    assert_eq!(
        entry.status,
        OutboxStatus::Sent.as_str(),
        "chat_search 命中即应标 sent（本地无 mcp_call_logs 也不重发），status={:?}",
        entry.status
    );

    let recv = mcp_server
        .received_requests()
        .await
        .expect("wiremock recorded requests");
    let send_calls = recv
        .iter()
        .filter(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body)
                .ok()
                .and_then(|v| {
                    v.pointer("/params/name")
                        .and_then(|n| n.as_str())
                        .map(|s| s == "message_send_text")
                })
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        send_calls, 0,
        "chat_search 命中不应真实重发 message_send_text；send_calls={send_calls}"
    );
    // 反向坐实走了 chat_search（而非直接查本地/直接重发）：收到过 ≥1 次 chat_search tools/call。
    let search_calls = recv
        .iter()
        .filter(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body)
                .ok()
                .and_then(|v| {
                    v.pointer("/params/name")
                        .and_then(|n| n.as_str())
                        .map(|s| s == "chat_search")
                })
                .unwrap_or(false)
        })
        .count();
    assert!(
        search_calls >= 1,
        "reclaim text 路必须先查权威 chat_search；search_calls={search_calls}"
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
            "boundaryPrivacySafety": 9,
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

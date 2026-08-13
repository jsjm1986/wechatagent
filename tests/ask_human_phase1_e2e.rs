//! `ask_human_phase1_e2e` —— Ask-Human Phase 1 决策请示通道端到端集成测试。
//!
//! 全部 `#[ignore]`（需要 Docker / testcontainers MongoDB，本地无 Docker 时跳过，
//! 由 CI `integration` job 跑 `--ignored`）。
//!
//! 这些测试**直调** route handler 真函数（不经 axum HTTP 层），与
//! `tests/domain_profile_e2e.rs` 同惯例：handler 是普通 async fn，参数是 axum
//! extractor（`State`/`Extension`/`Path`/`Query`/`Json`），构造好就 `.await`，
//! 经 `.0` 取出 `Json<Value>` 的内层 `Value`。
//!
//! 覆盖：
//! 1. `put_ask_human_policy_persists_and_reads_back`：PUT ask_human_policy →
//!    回读 config 行，字段一致、version 未 bump、current_version 仍 true。
//! 2. `admin_resolve_enqueues_relay_and_marks_resolved`：seed pending → admin
//!    resolve → 台账 resolved + resolved_via="admin" + 一条 relay task 入队。
//! 3. `admin_resolve_is_idempotent`：对已 resolved 的再 resolve → alreadyResolved，
//!    不重复入队。
//! 4. `reassign_rejects_wxid_not_in_chain`：decider_chain=[a]，reassign 到 b → 400。
//! 5. `inbox_aggregates_and_degrades`：seed pending escalation + needs_review chunk
//!    → inbox 返回 ≥2 items，errors 为空。
//! 6. `summary_counts_pending`：summary → principalEscalation 计数正确。
//! 7. `resolve_foreign_workspace_escalation_is_noop`：跨 workspace resolve →
//!    幂等 200 alreadyResolved，台账仍 pending（IDOR 守卫，Task-7 保证）。
//!
//! ## 关键 harness 纪律
//! `TestApp::start()` 已跑 migrations + ensure_indexes + `ensure_prompt_pack_v2`，
//! 后者已 seed `(default, user_operations, version:1, current_version:true)` 底座
//! config 行。所以 ask_human_policy 测试**不再 insert** config 行——它已存在，直接
//! PUT 即 `$set` 到现有 current 行。若测试需预置 config 前置条件，用 `update_one`
//! `$set` 到既有 current 行，**绝不 `insert_one`**（会撞 (workspace,domain,version)
//! 唯一索引 E11000）。见 [[project_config_seed_in_prompts_not_migrations]]。

mod common;

use axum::extract::{Extension, Json, Path, Query, State};
use mongodb::{
    bson::{doc, DateTime, Document},
    options::UpdateOptions,
};
use serde_json::Value;

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{
    AgentPrincipalEscalation, AskHumanPolicy, DeciderRef, OperationKnowledgeChunk,
    PrincipalEscalationProtocol, ESCALATION_CATEGORY_HIGH_RISK_GATED,
    PRINCIPAL_CARD_DELIVERY_QUEUED, PRINCIPAL_CARD_DELIVERY_SENT,
    PRINCIPAL_ESCALATION_STATUS_PENDING,
};
use wechatagent::routes::AppState;

/// 构造测试 admin auth context（current_workspace 决定 handler 可见范围）。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "test_admin".to_string(),
        username: "test_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 保存 ask-human 策略前建立后端权威校验所需的真实账号—联系人归属。
async fn ensure_decider_identity(state: &AppState, ws: &str, account_id: &str, wxid: &str) {
    let now = DateTime::now();
    state
        .db
        .raw()
        .collection::<Document>("wechat_accounts")
        .update_one(
            doc! { "workspace_id": ws, "account_id": account_id },
            doc! { "$setOnInsert": {
                "workspace_id": ws,
                "account_id": account_id,
                "alias": account_id,
                "display_name": account_id,
                "online": true,
                "last_sync_at": now,
                "capacity": 0i32,
                "off_hours": [],
                "created_at": now,
                "updated_at": now,
            } },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await
        .expect("seed decider account identity");
    state
        .db
        .raw()
        .collection::<Document>("contacts")
        .update_one(
            doc! { "workspace_id": ws, "account_id": account_id, "wxid": wxid },
            doc! { "$setOnInsert": {
                "workspace_id": ws,
                "account_id": account_id,
                "wxid": wxid,
                "agent_status": "normal",
                "created_at": now,
                "updated_at": now,
            } },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await
        .expect("seed decider contact identity");
}

/// seed 一条 pending 请示台账行（`insert_pending_escalation` 是 pub(crate) 不可达，
/// 故在测试侧直接构造结构体插入）。short_code 用 UUID 前缀保证全局唯一（台账唯一键）。
async fn seed_pending_escalation(
    state: &AppState,
    ws: &str,
    contact: &str,
    principal: &str,
) -> AgentPrincipalEscalation {
    let now = DateTime::now();
    let config = current_user_ops_config(state, ws).await;
    let frozen_policy = config
        .ask_human_policy
        .clone()
        .unwrap_or_else(|| AskHumanPolicy {
            decider_chain: vec![DeciderRef {
                wxid: principal.to_string(),
                display_name: None,
                account_id: Some("acc_test".to_string()),
            }],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: None,
            standing_order: None,
            standing_order_after_hours: None,
        });
    let principal_account_id = frozen_policy
        .decider_chain
        .iter()
        .find(|decider| decider.wxid == principal)
        .and_then(|decider| decider.account_id.clone())
        .unwrap_or_else(|| "acc_test".to_string());
    let entry = AgentPrincipalEscalation {
        id: None,
        workspace_id: ws.to_string(),
        account_id: "acc_test".to_string(),
        contact_wxid: contact.to_string(),
        short_code: format!("T{}", &uuid::Uuid::new_v4().simple().to_string()[..5]).to_uppercase(),
        status: PRINCIPAL_ESCALATION_STATUS_PENDING.to_string(),
        category: ESCALATION_CATEGORY_HIGH_RISK_GATED.to_string(),
        reason: "测试卡点".to_string(),
        question_for_principal: "请领导定夺".to_string(),
        principal_wxid: principal.to_string(),
        protocol: Some(PrincipalEscalationProtocol {
            domain: config.domain,
            policy_version: config.version,
            policy: frozen_policy,
            principal_account_id,
            delivery_generation: 1,
            delivery_state: PRINCIPAL_CARD_DELIVERY_SENT.to_string(),
            delivery_content: "test principal card".to_string(),
            delivery_outbox_id: None,
            failure_cleanup_completed_at: None,
        }),
        decision: None,
        authorization_expires_at: None,
        is_generalizable: false,
        knowledge_proposal_emitted: false,
        created_at: now,
        updated_at: now,
        resolved_at: None,
        resolved_via: None,
        relay_state: None,
        relay_task_id: None,
        relay_enqueued_at: None,
        relay_terminal_at: None,
        relay_terminal_reason: None,
        last_holding_reply_ms: None,
        last_pushed_at_ms: Some(now.timestamp_millis()),
    };
    state
        .db
        .agent_principal_escalations()
        .insert_one(&entry, None)
        .await
        .expect("seed escalation");
    entry
}

/// seed 一条 needs_review 知识切片（inbox knowledge_review source 用）。
async fn seed_needs_review_chunk(state: &AppState, ws: &str) {
    let chunk = OperationKnowledgeChunk {
        workspace_id: ws.to_string(),
        domain: "user_operations".to_string(),
        title: "待核验切片".to_string(),
        body: Some("AI 抽取的产品说明，待人审核验".to_string()),
        integrity_status: Some("needs_review".to_string()),
        status: "draft".to_string(),
        ..Default::default()
    };
    state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("seed needs_review chunk");
}

/// 回读 `(workspace, user_operations)` 下 current_version=true 的 config 行。
async fn current_user_ops_config(
    state: &AppState,
    ws: &str,
) -> wechatagent::models::OperationDomainConfig {
    state
        .db
        .operation_domain_configs()
        .find_one(
            doc! { "workspace_id": ws, "domain": "user_operations", "current_version": true },
            None,
        )
        .await
        .expect("find current config")
        .expect("current user_operations config exists (ensure_prompt_pack_v2 已 seed)")
}

// ── 测试 1：PUT ask_human_policy 持久化 + 回读 + 不 bump 版本 ──────────────────

/// PUT ask_human_policy → 回读 config 行：字段一致、version 未变（无 bump）、
/// current_version 仍 true（折叠确认项 A：config no-version-bump round-trip）。
#[tokio::test]
#[ignore]
async fn put_ask_human_policy_persists_and_reads_back() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // 捕获 PUT 前的版本号（证 PUT 后不变）。
    let before = current_user_ops_config(&app.state, &ws).await;
    let version_before = before.version;
    assert!(
        before.current_version,
        "前置：底座 config current_version=true"
    );
    ensure_decider_identity(&app.state, &ws, "acc_test", "boss_wx").await;

    // 构造 AskHumanPolicy（camelCase wire 格式经 serde 反序列化）。
    let policy: wechatagent::models::AskHumanPolicy = serde_json::from_value(serde_json::json!({
        "deciderChain": [{ "wxid": "boss_wx", "displayName": "王总", "accountId": "acc_test" }],
        "escalateSafetyGuard": true,
        "escalateUnverifiedProduct": true,
        "escalateAiPolicyHold": true,
        "escalateStuck": false,
        "dedupeWindowHours": 6.0,
        "dailyPushCap": 10_u32,
        "timeoutHours": 24.0,
    }))
    .expect("deserialize AskHumanPolicy");

    let resp = wechatagent::routes::domains::put_ask_human_policy(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path("user_operations".to_string()),
        Json(policy.clone()),
    )
    .await
    .expect("put_ask_human_policy ok");
    assert_eq!(resp.0.get("ok").and_then(|v| v.as_bool()), Some(true));

    // 回读：ask_human_policy 落库且与发送的一致。
    let after = current_user_ops_config(&app.state, &ws).await;
    let stored = after
        .ask_human_policy
        .as_ref()
        .expect("ask_human_policy 应已落库");
    assert_eq!(
        stored, &policy,
        "回读的 ask_human_policy 应与 PUT 的逐字段一致"
    );

    // 确认项 A：version 未 bump、current_version 仍 true（$set 贴生产 admin 编辑语义）。
    assert_eq!(
        after.version, version_before,
        "PUT ask_human_policy 不应 bump version"
    );
    assert!(after.current_version, "PUT 后 current_version 仍为 true");
}

// ── 测试 2：admin resolve 起 relay + 标 resolved ────────────────────────────

/// seed pending → admin 结构化裁决 → 台账 status=resolved + resolved_via="admin"
/// + 起一条 principal_decision_relay task（content=short_code）。
#[tokio::test]
#[ignore]
async fn admin_resolve_enqueues_relay_and_marks_resolved() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    ensure_decider_identity(&app.state, &ws, "acc_test", "a").await;

    let entry = seed_pending_escalation(&app.state, &ws, "cust1", "boss").await;

    let resp = wechatagent::routes::principal_escalations::resolve_principal_escalation(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(entry.short_code.clone()),
        Json(
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "substance": "可以给 8 折",
            }))
            .expect("deserialize ResolveBody"),
        ),
    )
    .await
    .expect("resolve ok");
    assert_eq!(resp.0.get("ok").and_then(|v| v.as_bool()), Some(true));

    // 台账：resolved + resolved_via=admin。
    let updated = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None)
        .await
        .expect("query escalation")
        .expect("escalation exists");
    assert_eq!(updated.status, "resolved", "裁决后台账应转 resolved");
    assert_eq!(
        updated.resolved_via.as_deref(),
        Some("admin"),
        "resolved_via=admin"
    );
    assert_eq!(updated.relay_state.as_deref(), Some("enqueued"));
    assert_eq!(updated.relay_task_id, updated.id);
    assert!(updated.relay_enqueued_at.is_some());

    // relay task 入队（kind=principal_decision_relay，content=short_code）。
    let task_count = app
        .state
        .db
        .tasks()
        .count_documents(
            doc! { "kind": "principal_decision_relay", "content": &entry.short_code },
            None,
        )
        .await
        .expect("count relay tasks");
    assert_eq!(task_count, 1, "应恰好起一条 relay task");
}

/// A committed resolution whose task materialization was interrupted remains
/// recoverable. Repeated worker reconciliation must converge to one task.
#[tokio::test]
#[ignore]
async fn resolved_relay_intent_recovers_exactly_one_task() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let entry = seed_pending_escalation(&app.state, &ws, "cust-recovery", "boss").await;

    let _ = wechatagent::routes::principal_escalations::resolve_principal_escalation(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(entry.short_code.clone()),
        Json(
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "substance": "恢复测试裁决",
            }))
            .expect("deserialize ResolveBody"),
        ),
    )
    .await
    .expect("resolve before interruption");

    let resolved = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None)
        .await
        .expect("query resolved escalation")
        .expect("resolved escalation exists");
    let task_id = resolved.relay_task_id.expect("durable relay task id");

    // Simulate an interruption after the resolution CAS but before task
    // materialization/acknowledgement became durable.
    app.state
        .db
        .tasks()
        .delete_one(doc! { "_id": task_id }, None)
        .await
        .expect("remove materialized task for crash simulation");
    app.state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! { "_id": resolved.id.expect("escalation id") },
            doc! {
                "$set": { "relay_state": "pending" },
                "$unset": { "relay_enqueued_at": "" },
            },
            None,
        )
        .await
        .expect("restore interrupted durable intent");

    assert_eq!(
        wechatagent::agent::escalation::reconcile_pending_relay_intents(&app.state)
            .await
            .expect("first reconciliation"),
        1
    );
    assert_eq!(
        wechatagent::agent::escalation::reconcile_pending_relay_intents(&app.state)
            .await
            .expect("idempotent reconciliation"),
        0
    );
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(
                doc! {
                    "_id": task_id,
                    "kind": "principal_decision_relay",
                    "content": &entry.short_code,
                },
                None,
            )
            .await
            .expect("count recovered relay task"),
        1
    );
    let recovered = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "_id": resolved.id.expect("escalation id") }, None)
        .await
        .expect("query recovered intent")
        .expect("recovered intent exists");
    assert_eq!(recovered.relay_state.as_deref(), Some("enqueued"));
    assert!(recovered.relay_enqueued_at.is_some());
}

// ── 测试 3：admin resolve 幂等 ──────────────────────────────────────────────

/// 对已 resolved 的请示再 resolve → 返回 alreadyResolved，不重复起 relay task。
#[tokio::test]
#[ignore]
async fn admin_resolve_is_idempotent() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let entry = seed_pending_escalation(&app.state, &ws, "cust2", "boss").await;

    // 第一次 resolve：成功。
    let first = wechatagent::routes::principal_escalations::resolve_principal_escalation(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(entry.short_code.clone()),
        Json(
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "substance": "同意",
            }))
            .expect("deserialize ResolveBody"),
        ),
    )
    .await
    .expect("first resolve ok");
    assert_eq!(first.0.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert!(
        first.0.get("alreadyResolved").is_none(),
        "首次 resolve 不应带 alreadyResolved"
    );

    // 第二次 resolve 同一条：幂等 alreadyResolved=true。
    let second = wechatagent::routes::principal_escalations::resolve_principal_escalation(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(entry.short_code.clone()),
        Json(
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "substance": "同意",
            }))
            .expect("deserialize ResolveBody"),
        ),
    )
    .await
    .expect("second resolve ok");
    assert_eq!(
        second.0.get("alreadyResolved").and_then(|v| v.as_bool()),
        Some(true),
        "重复 resolve 应返回 alreadyResolved=true"
    );

    // relay task 仍只有一条（未因重复 resolve 多入队）。
    let task_count = app
        .state
        .db
        .tasks()
        .count_documents(
            doc! { "kind": "principal_decision_relay", "content": &entry.short_code },
            None,
        )
        .await
        .expect("count relay tasks");
    assert_eq!(task_count, 1, "幂等 resolve 不应重复起 relay task");
}

// ── 测试 4：reassign 拒绝不在链内的 wxid ────────────────────────────────────

/// 配置 decider_chain=[a]，reassign 到 b（不在链内）→ handler 返回 Err(BadRequest)。
/// 直调 handler 时 400 表现为 `Err(AppError::BadRequest)`，断言 `.is_err()`。
#[tokio::test]
#[ignore]
async fn reassign_rejects_wxid_not_in_chain() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    ensure_decider_identity(&app.state, &ws, "acc_test", "a").await;

    // 先把 decider_chain=[{wxid:"a"}] PUT 到 current config（含 a 不含 b）。
    let policy: wechatagent::models::AskHumanPolicy = serde_json::from_value(serde_json::json!({
        "deciderChain": [{ "wxid": "a", "accountId": "acc_test" }],
    }))
    .expect("deserialize policy");
    let _ = wechatagent::routes::domains::put_ask_human_policy(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path("user_operations".to_string()),
        Json(policy),
    )
    .await
    .expect("put policy ok");

    let entry = seed_pending_escalation(&app.state, &ws, "cust3", "a").await;

    // reassign 到 b（不在链内）→ 必须 Err。
    let result = wechatagent::routes::principal_escalations::reassign_principal_escalation(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(entry.short_code.clone()),
        Json(
            serde_json::from_value(serde_json::json!({ "toWxid": "b" }))
                .expect("deserialize ReassignBody"),
        ),
    )
    .await;
    assert!(result.is_err(), "reassign 到链外 wxid 应返回 Err（400）");
    assert!(
        matches!(result, Err(wechatagent::error::AppError::BadRequest(_))),
        "应是 BadRequest 变体"
    );
}

// ── 测试 5：inbox 聚合 + per-source 降级 ────────────────────────────────────

/// seed 一条 pending escalation + 一条 needs_review chunk → inbox 返回 ≥2 items，
/// errors 为空（两 source 都成功聚合，无降级触发）。
#[tokio::test]
#[ignore]
async fn inbox_aggregates_and_degrades() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    seed_pending_escalation(&app.state, &ws, "cust4", "boss").await;
    seed_needs_review_chunk(&app.state, &ws).await;

    let resp = wechatagent::routes::ask_human_inbox::ask_human_inbox(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Query(serde_json::from_value(serde_json::json!({})).expect("deserialize InboxQuery")),
    )
    .await
    .expect("inbox ok");
    let body: Value = resp.0;

    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .expect("items array");
    assert!(
        items.len() >= 2,
        "inbox 应至少聚合到请示 + 知识核验两条 item，实际={}",
        items.len()
    );
    let sources: Vec<&str> = items
        .iter()
        .filter_map(|i| i.get("source").and_then(|s| s.as_str()))
        .collect();
    assert!(
        sources.contains(&"principal_escalation"),
        "应含 principal_escalation source: {sources:?}"
    );
    assert!(
        sources.contains(&"knowledge_review"),
        "应含 knowledge_review source: {sources:?}"
    );

    let errors = body
        .get("errors")
        .and_then(|v| v.as_array())
        .expect("errors array");
    assert!(errors.is_empty(), "正常聚合 errors 应为空，实际={errors:?}");
}

// ── 测试 6：summary 计数 pending ────────────────────────────────────────────

/// seed 两条 pending escalation → summary.principalEscalation == 2。
#[tokio::test]
#[ignore]
async fn summary_counts_pending() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    seed_pending_escalation(&app.state, &ws, "cust5", "boss").await;
    seed_pending_escalation(&app.state, &ws, "cust6", "boss").await;

    let resp = wechatagent::routes::ask_human_inbox::ask_human_summary(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Query(wechatagent::routes::ask_human_inbox::InboxQuery::default()),
    )
    .await
    .expect("summary ok");
    let body: Value = resp.0;

    assert_eq!(
        body.get("principalEscalation").and_then(|v| v.as_i64()),
        Some(2),
        "summary.principalEscalation 应为 2"
    );
}

// ── 测试 7：跨 workspace resolve 幂等 noop（Task-7 IDOR 守卫）────────────────

/// seed pending 于 workspace "default" → 用 **另一 workspace** 的 admin resolve →
/// 返回 alreadyResolved=true（幂等成功，不泄漏存在性），且台账仍 pending（**未真正
/// 被裁决**，IDOR 守卫生效）。
#[tokio::test]
#[ignore]
async fn resolve_foreign_workspace_escalation_is_noop() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let entry = seed_pending_escalation(&app.state, &ws, "cust7", "boss").await;

    // 用 "other_ws"（≠ "default"）的 admin 尝试 resolve 同一短码。
    let resp = wechatagent::routes::principal_escalations::resolve_principal_escalation(
        State(app.state.clone()),
        Extension(test_admin("other_ws")),
        Path(entry.short_code.clone()),
        Json(
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "substance": "越权裁决尝试",
            }))
            .expect("deserialize ResolveBody"),
        ),
    )
    .await
    .expect("foreign resolve returns ok (幂等，不报错泄漏存在性)");
    assert_eq!(
        resp.0.get("alreadyResolved").and_then(|v| v.as_bool()),
        Some(true),
        "跨 workspace resolve 应幂等返回 alreadyResolved=true"
    );

    // 关键：台账仍 pending —— 越权方未能真正裁决。
    let still = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None)
        .await
        .expect("query escalation")
        .expect("escalation exists");
    assert_eq!(
        still.status, "pending",
        "跨 workspace resolve 不应真正裁决，台账须仍 pending（IDOR 守卫）"
    );
    assert!(
        still.resolved_via.is_none(),
        "未被裁决 → resolved_via 仍为空"
    );
}

// ── 测试 8（终审修 #1）：admin deferred 暂缓保持 pending，不 resolve 不 relay ──

/// admin 给出 "deferred" 裁决 → 短路返回 deferred=true；台账仍 pending、resolved_via
/// 仍空；零 relay task 入队。（修前：deferred 会 resolve 台账却不转述 → 静默关闭，
/// scan_escalation_timeouts 只扫 pending 永不再 surface。）
#[tokio::test]
#[ignore]
async fn admin_deferred_keeps_escalation_pending() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let entry = seed_pending_escalation(&app.state, &ws, "cust_def", "boss").await;

    let resp = wechatagent::routes::principal_escalations::resolve_principal_escalation(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(entry.short_code.clone()),
        Json(
            serde_json::from_value(serde_json::json!({
                "verdict": "deferred",
                "substance": "再等等",
            }))
            .expect("deserialize ResolveBody"),
        ),
    )
    .await
    .expect("deferred resolve ok");
    assert_eq!(
        resp.0.get("deferred").and_then(|v| v.as_bool()),
        Some(true),
        "deferred 应短路返回 deferred=true"
    );

    // 台账仍 pending、未被裁决。
    let still = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None)
        .await
        .expect("query escalation")
        .expect("escalation exists");
    assert_eq!(
        still.status, "pending",
        "deferred 不应 resolve，台账须仍 pending（与 wechat 路径一致）"
    );
    assert!(
        still.resolved_via.is_none(),
        "deferred 未裁决 → resolved_via 仍空"
    );

    // 零 relay task 入队（deferred 不转述）。
    let task_count = app
        .state
        .db
        .tasks()
        .count_documents(
            doc! { "kind": "principal_decision_relay", "content": &entry.short_code },
            None,
        )
        .await
        .expect("count relay tasks");
    assert_eq!(task_count, 0, "deferred 不应起任何 relay task");
}

// ── 测试 9（终审修 #2）：超时改派后 age 自 updated_at 起算，每位决策人拿到完整窗 ──

/// decider_chain=[a,b,c]、timeout=24h。a 的已确认送达时刻拨回 25h 前后，scanner
/// 只原子开启 b 的 generation 并写入 Outbox；b 仍为 queued 时重复扫描不得级联到 c。
/// 只有 Outbox 进入 sent 且 reconciler 写回 last_pushed_at_ms 后，b 的完整 24h 窗才开始。
#[tokio::test]
#[ignore]
async fn timeout_reassign_gives_each_decider_full_window() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    for decider in ["a", "b", "c"] {
        ensure_decider_identity(&app.state, &ws, "acc_test", decider).await;
    }
    let now = DateTime::now();
    app.state
        .db
        .raw()
        .collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc_test",
                "wxid": "cust_chain",
                "agent_status": "managed",
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("seed timeout customer identity");

    // PUT 决策人链 [a,b,c] + timeout=24h 到 current config。
    let policy: wechatagent::models::AskHumanPolicy = serde_json::from_value(serde_json::json!({
        "deciderChain": [
            { "wxid": "a", "accountId": "acc_test" },
            { "wxid": "b", "accountId": "acc_test" },
            { "wxid": "c", "accountId": "acc_test" }
        ],
        "timeoutHours": 24.0,
    }))
    .expect("deserialize policy");
    let _ = wechatagent::routes::domains::put_ask_human_policy(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path("user_operations".to_string()),
        Json(policy),
    )
    .await
    .expect("put policy ok");

    let entry = seed_pending_escalation(&app.state, &ws, "cust_chain", "a").await;

    // 模拟 a 的卡已在 25h 前确认送达。
    let twenty_five_h_ago =
        DateTime::from_millis(DateTime::now().timestamp_millis() - 25 * 3600 * 1000);
    app.state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! { "short_code": &entry.short_code },
            doc! { "$set": {
                "created_at": twenty_five_h_ago,
                "updated_at": twenty_five_h_ago,
                "last_pushed_at_ms": twenty_five_h_ago.timestamp_millis(),
            } },
            None,
        )
        .await
        .expect("backdate clock");

    // 第一次 scan 只开启 b 的 generation=2 并物化唯一 Outbox，不跨远端边界。
    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("first scan ok");
    let after_first = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None)
        .await
        .expect("query")
        .expect("exists");
    assert_eq!(after_first.principal_wxid, "b", "第一次超时扫描应 a→b");
    let first_protocol = after_first.protocol.as_ref().expect("frozen protocol");
    assert_eq!(first_protocol.delivery_generation, 2);
    assert_eq!(
        first_protocol.delivery_state,
        PRINCIPAL_CARD_DELIVERY_QUEUED
    );
    let outbox_id = first_protocol
        .delivery_outbox_id
        .expect("generation 2 outbox");
    assert!(
        after_first.last_pushed_at_ms.is_none(),
        "queued 尚未确认送达，不得启动 timeout"
    );

    // queued 不属于 timeout eligible；重复扫描不得产生 generation=3 或第二条 Outbox。
    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("second scan ok");
    let after_second = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None)
        .await
        .expect("query")
        .expect("exists");
    assert_eq!(after_second.principal_wxid, "b", "queued 阶段不得级联到 c");
    assert_eq!(
        after_second.protocol.as_ref().unwrap().delivery_generation,
        2
    );
    assert_eq!(
        app.state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "_id": outbox_id }, None)
            .await
            .expect("count generation outbox"),
        1
    );

    // 模拟 dispatcher 的权威 sent 终态，再用生产 reconciler 写回计时起点。
    let delivered_at = DateTime::now();
    app.state
        .db
        .collection_agent_send_outbox()
        .update_one(
            doc! { "_id": outbox_id },
            doc! { "$set": {
                "status": "sent",
                "sent_at": delivered_at,
                "updated_at": delivered_at,
            } },
            None,
        )
        .await
        .expect("mark generation outbox sent");
    assert_eq!(
        wechatagent::agent::escalation::reconcile_principal_card_deliveries(&app.state)
            .await
            .expect("reconcile sent generation"),
        1
    );
    let delivered = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None)
        .await
        .expect("query delivered generation")
        .expect("escalation exists");
    assert_eq!(
        delivered.protocol.as_ref().unwrap().delivery_state,
        PRINCIPAL_CARD_DELIVERY_SENT
    );
    assert_eq!(
        delivered.last_pushed_at_ms,
        Some(delivered_at.timestamp_millis())
    );

    wechatagent::agent::escalation::scan_escalation_timeouts(&app.state)
        .await
        .expect("scan inside b full window");
    let still_b = app
        .state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": &entry.short_code }, None)
        .await
        .expect("query b window")
        .expect("escalation exists");
    assert_eq!(
        still_b.principal_wxid, "b",
        "b 从 sent 对账时刻起获得完整 24h 窗"
    );
}

/// P2 Task 1 + 终审修复: lessons_learned 收件项必须带 richParams.lessonId（前端
/// LessonPromoteCard 深链前提）。lessonId 必须是文档的 `lesson_id` 字段
/// （`{workspace}::{pattern_kind}`，由 aggregate_lessons_for_workspace 写入），
/// 而非 `_id` hex——list/promote 端点按 `lesson_id` 寻址，用 _id 会 NotFound。
#[tokio::test]
#[ignore]
async fn inbox_lessons_item_carries_lesson_id() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    // seed 一条 pending_review lessons_learned（裸 Document，无 typed accessor）。
    // 带真实 lesson_id 字段（生产由 aggregate_lessons_for_workspace 写 {ws}::{kind}）。
    let lesson_id = format!("{ws}::objection_handling");
    let coll = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("lessons_learned");
    coll.insert_one(
        doc! {
            "workspace_id": &ws,
            "lesson_id": &lesson_id,
            "review_status": "pending_review",
            "pattern_kind": "objection_handling",
            "created_at": DateTime::now(),
        },
        None,
    )
    .await
    .unwrap();
    // 调 inbox handler，过滤 lessons_learned 源
    let resp = wechatagent::routes::ask_human_inbox::ask_human_inbox(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Query(
            serde_json::from_value(serde_json::json!({ "source": "lessons_learned" }))
                .expect("deserialize InboxQuery"),
        ),
    )
    .await
    .expect("inbox ok");
    let body: Value = resp.0;
    let items = body["items"].as_array().expect("items array");
    let lesson = items
        .iter()
        .find(|i| i["source"] == "lessons_learned")
        .expect("应含 lessons_learned item");
    assert_eq!(
        lesson["richParams"]["lessonId"],
        serde_json::json!(lesson_id),
        "lessons 收件项应带 richParams.lessonId={lesson_id}（lesson_id 字段，非 _id hex）"
    );
}

#[tokio::test]
#[ignore]
async fn get_single_chunk_by_id_scoped_to_workspace() {
    let app = common::TestApp::start().await;
    let ws = &app.state.config.default_workspace_id;
    let chunk = wechatagent::models::OperationKnowledgeChunk {
        id: None,
        workspace_id: ws.to_string(),
        title: "测试切片".into(),
        body: Some("正文".into()),
        ..Default::default()
    };
    let inserted = app
        .state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .unwrap();
    let hex = inserted.inserted_id.as_object_id().unwrap().to_hex();
    let resp = wechatagent::routes::knowledge::crud::get_operation_knowledge_chunk(
        axum::extract::State(app.state.clone()),
        axum::Extension(test_admin(ws)),
        axum::extract::Path(hex.clone()),
    )
    .await
    .unwrap();
    let body: serde_json::Value = resp.0;
    assert_eq!(body["item"]["title"], serde_json::json!("测试切片"));

    // 反向断言（IDOR 隔离命门）：另一个 workspace 的 admin 读 default ws 的 chunk
    // → 守卫（crud.rs find_one 带 workspace_id 条件）拒绝 → NotFound。
    // 没有这条，测试名声称的 *_scoped_to_workspace 隔离属性从未被真正验证（假绿）。
    let foreign = wechatagent::routes::knowledge::crud::get_operation_knowledge_chunk(
        axum::extract::State(app.state.clone()),
        axum::Extension(test_admin("other_ws")),
        axum::extract::Path(hex.clone()),
    )
    .await;
    assert!(
        matches!(foreign, Err(wechatagent::error::AppError::NotFound(_))),
        "跨 workspace 读 chunk 必须 NotFound（IDOR 守卫），实际: {foreign:?}"
    );
}

#[tokio::test]
#[ignore]
async fn operation_domain_json_includes_ask_human_policy() {
    let app = common::TestApp::start().await;
    let ws = &app.state.config.default_workspace_id;
    // 给 user_operations 当前版本写一条 ask_human_policy。
    let policy = wechatagent::models::AskHumanPolicy {
        decider_chain: vec![wechatagent::models::DeciderRef {
            wxid: "wxid_boss".into(),
            display_name: Some("老板".into()),
            account_id: Some("acc_test".into()),
        }],
        escalate_safety_guard: true,
        escalate_unverified_product: true,
        escalate_ai_policy_hold: false,
        escalate_stuck: true,
        dedupe_window_hours: Some(6.0),
        daily_push_cap: Some(3),
        quiet_hours: None,
        timeout_hours: Some(24.0),
        standing_order: None,
        standing_order_after_hours: None,
    };
    let policy_bson = mongodb::bson::to_bson(&policy).unwrap();
    app.state
        .db
        .operation_domain_configs()
        .update_one(
            mongodb::bson::doc! { "workspace_id": ws, "domain": "user_operations", "current_version": true },
            mongodb::bson::doc! { "$set": { "ask_human_policy": policy_bson } },
            None,
        )
        .await
        .unwrap();
    let resp = wechatagent::routes::domains::get_operation_domain(
        axum::extract::State(app.state.clone()),
        axum::Extension(test_admin(ws)),
        axum::extract::Path("user_operations".to_string()),
    )
    .await
    .unwrap();
    let body: serde_json::Value = resp.0;
    assert_eq!(
        body["item"]["askHumanPolicy"]["deciderChain"][0]["wxid"],
        serde_json::json!("wxid_boss")
    );
    assert_eq!(
        body["item"]["askHumanPolicy"]["timeoutHours"],
        serde_json::json!(24.0)
    );
    assert_eq!(
        body["item"]["askHumanPolicy"]["dailyPushCap"],
        serde_json::json!(3)
    );
}

// ── 测试 8：关系类型建议富投影（E10 审核反盲批）────────────────────────────────
//
// seed 一条 pending RelationshipTypeSuggestion（evidence/confidence/occurrences 全持久化）
// → 经真投影函数 collect_relationship_suggestions 聚合到 inbox → 断言对应 item 携带
// evidence / confidence==80 / occurrences==3。证据全程经真投影函数流出（非手搓 InboxItem）。
#[tokio::test]
#[ignore]
async fn inbox_relationship_suggestion_carries_evidence() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let now = DateTime::now();
    let suggestion = wechatagent::models::RelationshipTypeSuggestion {
        id: None,
        workspace_id: ws.clone(),
        account_id: "acc_test".to_string(),
        contact_id: "contact_e10".to_string(),
        suggested_value: "peer".to_string(),
        evidence: Some("多次自称同行".to_string()),
        confidence: 80,
        status: "pending".to_string(),
        occurrences: 3,
        first_seen_at: now,
        last_seen_at: now,
        reviewed_at: None,
        reviewed_by: None,
    };
    app.state
        .db
        .collection_relationship_type_suggestions()
        .insert_one(&suggestion, None)
        .await
        .expect("seed relationship suggestion");

    let resp = wechatagent::routes::ask_human_inbox::ask_human_inbox(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Query(
            serde_json::from_value(serde_json::json!({ "source": "relationship_suggestion" }))
                .expect("deserialize InboxQuery"),
        ),
    )
    .await
    .expect("inbox ok");
    let body: Value = resp.0;

    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .expect("items array");
    let item = items
        .iter()
        .find(|i| i.get("source").and_then(|s| s.as_str()) == Some("relationship_suggestion"))
        .expect("应含 relationship_suggestion item");

    assert_eq!(
        item["evidence"],
        serde_json::json!("多次自称同行"),
        "应投影 evidence"
    );
    assert_eq!(
        item["confidence"],
        serde_json::json!(80),
        "应投影 confidence==80"
    );
    assert_eq!(
        item["occurrences"],
        serde_json::json!(3),
        "应投影 occurrences==3"
    );
    assert_eq!(
        item["contactWxid"],
        serde_json::json!("contact_e10"),
        "应投影 contact_id 入 contactWxid"
    );
}

/// SR-067：疑似成交必须进入统一待审箱和汇总计数；其它 workspace 及非 pending
/// 历史不得泄漏或占用当前待办数。
#[tokio::test]
#[ignore]
async fn inbox_and_summary_include_only_workspace_pending_suspected_deals() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let now = DateTime::now();

    for (workspace_id, status, contact_id) in [
        (ws.as_str(), "pending", "contact_pending"),
        (ws.as_str(), "approved", "contact_approved"),
        ("other_workspace", "pending", "contact_foreign"),
    ] {
        app.state
            .db
            .collection_suspected_deal_signals()
            .insert_one(
                wechatagent::models::SuspectedDealSignal {
                    id: None,
                    workspace_id: workspace_id.to_string(),
                    account_id: "acc_test".to_string(),
                    contact_id: contact_id.to_string(),
                    value: "疑似成交·待核实".to_string(),
                    evidence: Some("客户明确表达下单意向".to_string()),
                    confidence: 88,
                    status: status.to_string(),
                    occurrences: 2,
                    first_seen_at: now,
                    last_seen_at: now,
                    reviewed_at: None,
                    reviewed_by: None,
                },
                None,
            )
            .await
            .expect("seed suspected deal signal");
    }

    let inbox = wechatagent::routes::ask_human_inbox::ask_human_inbox(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Query(
            serde_json::from_value(serde_json::json!({ "source": "suspected_deal" }))
                .expect("deserialize InboxQuery"),
        ),
    )
    .await
    .expect("suspected deal inbox");
    let items = inbox.0["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "只应投影当前 workspace 的 pending 信号");
    assert_eq!(items[0]["source"], "suspected_deal");
    assert_eq!(items[0]["contactWxid"], "contact_pending");
    assert_eq!(items[0]["richComponent"], "suspectedDealReview");
    assert_eq!(items[0]["confidence"], 88);
    assert_eq!(items[0]["occurrences"], 2);

    let summary = wechatagent::routes::ask_human_inbox::ask_human_summary(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Query(wechatagent::routes::ask_human_inbox::InboxQuery::default()),
    )
    .await
    .expect("suspected deal summary");
    assert_eq!(summary.0["counts"]["suspectedDeal"], 1);
    assert_eq!(summary.0["suspectedDeal"], 1, "兼容顶层计数也必须一致");

    app.cleanup().await;
}

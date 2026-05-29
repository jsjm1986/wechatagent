//! agent-autonomy-loop W6 / Task 7.3：`GET /api/outcomes/autonomy` 端到端测试。
//!
//! 直接调用 `routes::get_autonomy_outcomes` 处理函数，绕过 axum HTTP harness：
//! testcontainers MongoDB 已构造完整 `AppState`，处理函数本身只是
//! `(State, Query) -> Json<Value>`，HTTP 层只是 wire format。
//!
//! 覆盖 4 个核心场景（与 `.kiro/specs/agent-autonomy-loop/tasks.md:388-393` 对齐）：
//! - 接口在 `total_runs == 0` 时所有比率返回 `null`；
//! - 5 条 run（其中 2 条触发 revision）后 `revision_trigger_rate == 0.4`；
//! - 3 条 hold（每个 holdCategory 各 1 条）后 `ai_hold_breakdown` 三类各 1/total_runs；
//! - `held_for_human` 历史脏值不被统计在任何分类内。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。

mod common;

use axum::extract::{Query, State};
use axum::Extension;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::Value;
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::routes::{get_autonomy_outcomes, AutonomyMetricsQuery};

/// 直接插入一条 `agent_run_logs` 原始 BSON 文档（不走 typed `AgentRunLog`），
/// 避免重复声明所有 30+ 字段；缺字段反序列化时取 `#[serde(default)]`。
async fn insert_run_log(app: &common::TestApp, mut fields: Document) {
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    fields
        .entry("_id".to_string())
        .or_insert(ObjectId::new().into());
    fields.insert("workspace_id", &workspace_id);
    fields.insert("account_id", &account_id);
    fields
        .entry("run_id".to_string())
        .or_insert(format!("run_{}", ObjectId::new().to_hex()).into());
    fields
        .entry("trigger_kind".to_string())
        .or_insert("inbound_message".into());
    fields
        .entry("status".to_string())
        .or_insert("completed".into());
    fields
        .entry("created_at".to_string())
        .or_insert(DateTime::now().into());
    let raw = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_run_logs");
    raw.insert_one(fields, None).await.expect("insert run log");
}

async fn call_metrics(app: &common::TestApp) -> Value {
    // 不带任何 query 参数 → 所有字段为 None；处理函数走默认 horizon=24h、
    // account_id=config.default_account_id。serde-urlencoded 走不通时这里
    // 直接 deserialize 一个空字符串就能命中"全 None"。
    let q: AutonomyMetricsQuery = serde_json::from_value(serde_json::json!({})).unwrap();
    let admin = AuthenticatedAdmin {
        user_id: "test_admin".into(),
        username: "test_admin".into(),
        current_workspace: app.state.config.default_workspace_id.clone(),
    };
    let resp = get_autonomy_outcomes(State(app.state.clone()), Extension(admin), Query(q))
        .await
        .expect("get_autonomy_outcomes ok");
    resp.0
}

#[tokio::test]
#[ignore]
async fn outcomes_autonomy_returns_null_ratios_when_no_runs() {
    let app = common::TestApp::start().await;
    let body = call_metrics(&app).await;

    assert_eq!(body["totalRuns"].as_u64(), Some(0));
    let m = &body["metrics"];
    assert!(m["revisionTriggerRate"].is_null(), "{m:?}");
    assert!(m["revisionPassRate"].is_null());
    assert!(m["aiHoldBreakdown"]["heldByAiPolicy"].is_null());
    assert!(m["aiHoldBreakdown"]["blockedBySafetyGuard"].is_null());
    assert!(m["aiHoldBreakdown"]["aiWaitingForMoreContext"].is_null());
    assert!(m["taxonomyCandidateRate"].is_null());
    assert!(m["unverifiedClaimBlockRate"].is_null());
    assert!(m["selfCritiqueAddressedRate"].is_null());
    assert!(m["autonomyModeDistribution"]["auto"].is_null());
    assert!(m["autonomyModeDistribution"]["assisted"].is_null());
    assert!(m["autonomyModeDistribution"]["blocked"].is_null());
}

#[tokio::test]
#[ignore]
async fn outcomes_autonomy_revision_trigger_rate_two_of_five_is_0_4() {
    let app = common::TestApp::start().await;

    // 5 条 upgraded run；其中 2 条 revision_applied=true（一条 approved、一条
    // revision_applied_approved）。剩余 3 条为普通 approved。
    for _ in 0..3 {
        insert_run_log(
            &app,
            doc! {
                "final_review_status": "approved",
                "revision_applied": false,
                "autonomy_mode": "auto",
                "review": doc! {},
            },
        )
        .await;
    }
    insert_run_log(
        &app,
        doc! {
            "final_review_status": "revision_applied_approved",
            "revision_applied": true,
            "autonomy_mode": "auto",
            "review": doc! { "selfCritiqueAddressed": true },
            "pre_revision_summary": "v1",
            "post_revision_summary": "v2",
        },
    )
    .await;
    insert_run_log(
        &app,
        doc! {
            "final_review_status": "revision_failed",
            "revision_applied": true,
            "autonomy_mode": "auto",
            "review": doc! { "selfCritiqueAddressed": false },
            "pre_revision_summary": "v1",
            "post_revision_summary": "v2",
        },
    )
    .await;

    let body = call_metrics(&app).await;
    assert_eq!(body["totalRuns"].as_u64(), Some(5));
    assert_eq!(
        body["metrics"]["revisionTriggerRate"].as_f64(),
        Some(0.4),
        "2/5 = 0.4，实际 {:?}",
        body["metrics"]["revisionTriggerRate"]
    );
    assert_eq!(
        body["metrics"]["revisionPassRate"].as_f64(),
        Some(0.5),
        "1/2 = 0.5（仅一条 revision 通过），实际 {:?}",
        body["metrics"]["revisionPassRate"]
    );
}

#[tokio::test]
#[ignore]
async fn outcomes_autonomy_ai_hold_breakdown_each_one_third_with_three_holds() {
    let app = common::TestApp::start().await;

    for status in ["held_by_ai_policy", "blocked_by_safety_guard", "ai_waiting_for_more_context"] {
        insert_run_log(
            &app,
            doc! {
                "final_review_status": status,
                "revision_applied": false,
                "autonomy_mode": "blocked",
                "review": doc! { "holdCategory": status },
            },
        )
        .await;
    }

    let body = call_metrics(&app).await;
    assert_eq!(body["totalRuns"].as_u64(), Some(3));
    let breakdown = &body["metrics"]["aiHoldBreakdown"];
    let one_third = 1.0_f64 / 3.0_f64;
    let close = |v: &Value, target: f64| -> bool {
        v.as_f64()
            .map(|x| (x - target).abs() < 1e-9)
            .unwrap_or(false)
    };
    assert!(
        close(&breakdown["heldByAiPolicy"], one_third),
        "heldByAiPolicy = {:?}, 期望 1/3",
        breakdown["heldByAiPolicy"]
    );
    assert!(
        close(&breakdown["blockedBySafetyGuard"], one_third),
        "blockedBySafetyGuard = {:?}, 期望 1/3",
        breakdown["blockedBySafetyGuard"]
    );
    assert!(
        close(&breakdown["aiWaitingForMoreContext"], one_third),
        "aiWaitingForMoreContext = {:?}, 期望 1/3",
        breakdown["aiWaitingForMoreContext"]
    );
}

#[tokio::test]
#[ignore]
async fn outcomes_autonomy_legacy_held_for_human_is_not_counted() {
    let app = common::TestApp::start().await;

    // 1 条干净 approved（升级后），1 条非法 held_for_human（脏数据 / 历史值）。
    // R10：脏数据 SHALL 不进入任何分类（aiHoldBreakdown 三类都为 0/total，且
    // total 仅算干净那一条）。
    insert_run_log(
        &app,
        doc! {
            "final_review_status": "approved",
            "revision_applied": false,
            "autonomy_mode": "auto",
            "review": doc! {},
        },
    )
    .await;
    insert_run_log(
        &app,
        doc! {
            "final_review_status": "held_for_human",
            "revision_applied": false,
            "autonomy_mode": "auto",
            "review": doc! {},
        },
    )
    .await;

    let body = call_metrics(&app).await;
    assert_eq!(
        body["totalRuns"].as_u64(),
        Some(1),
        "held_for_human SHALL 被剔除出 totalRuns，实际 {:?}",
        body["totalRuns"]
    );
    let bd = &body["metrics"]["aiHoldBreakdown"];
    assert_eq!(bd["heldByAiPolicy"].as_f64(), Some(0.0));
    assert_eq!(bd["blockedBySafetyGuard"].as_f64(), Some(0.0));
    assert_eq!(bd["aiWaitingForMoreContext"].as_f64(), Some(0.0));
}

// ── W6 / Task 7.1：补 unverified-claim / taxonomy-candidate / outbox ─────────

#[tokio::test]
#[ignore]
async fn outcomes_autonomy_unverified_claim_block_rate_counts_only_blocked_status() {
    let app = common::TestApp::start().await;

    // 4 条 upgraded run：1 条 blocked_unverified_product_claim、3 条 approved。
    // unverifiedClaimBlockRate = 1 / 4 = 0.25。
    insert_run_log(
        &app,
        doc! {
            "final_review_status": "blocked_unverified_product_claim",
            "revision_applied": false,
            "autonomy_mode": "blocked",
            "review": doc! {},
        },
    )
    .await;
    for _ in 0..3 {
        insert_run_log(
            &app,
            doc! {
                "final_review_status": "approved",
                "revision_applied": false,
                "autonomy_mode": "auto",
                "review": doc! {},
            },
        )
        .await;
    }

    let body = call_metrics(&app).await;
    assert_eq!(body["totalRuns"].as_u64(), Some(4));
    assert_eq!(
        body["metrics"]["unverifiedClaimBlockRate"].as_f64(),
        Some(0.25),
        "1/4 unverified product claim block，实际 {:?}",
        body["metrics"]["unverifiedClaimBlockRate"]
    );
    assert_eq!(
        body["rawCounts"]["unverifiedClaimBlock"].as_u64(),
        Some(1)
    );
}

#[tokio::test]
#[ignore]
async fn outcomes_autonomy_taxonomy_candidate_rate_matches_review_risk_prefix() {
    let app = common::TestApp::start().await;

    // 3 条 run：2 条 review.risks 含 taxonomy_candidate:* 前缀（一条多前缀混合，
    // 一条单 taxonomy_candidate），1 条只含其它 risk → 不计。
    insert_run_log(
        &app,
        doc! {
            "final_review_status": "approved",
            "revision_applied": false,
            "autonomy_mode": "auto",
            "review": doc! {
                "risks": ["taxonomy_candidate:domain_signal=超预算"],
            },
        },
    )
    .await;
    insert_run_log(
        &app,
        doc! {
            "final_review_status": "approved",
            "revision_applied": false,
            "autonomy_mode": "auto",
            "review": doc! {
                "risks": [
                    "hallucination:product_unverified",
                    "taxonomy_candidate:domain_signal=urgent",
                ],
            },
        },
    )
    .await;
    insert_run_log(
        &app,
        doc! {
            "final_review_status": "approved",
            "revision_applied": false,
            "autonomy_mode": "auto",
            "review": doc! {
                "risks": ["knowledge_grounding:missing_citation"],
            },
        },
    )
    .await;

    let body = call_metrics(&app).await;
    assert_eq!(body["totalRuns"].as_u64(), Some(3));
    let close = |v: &serde_json::Value, target: f64| -> bool {
        v.as_f64().map(|x| (x - target).abs() < 1e-9).unwrap_or(false)
    };
    assert!(
        close(&body["metrics"]["taxonomyCandidateRate"], 2.0 / 3.0),
        "2/3 taxonomy candidate, 实际 {:?}",
        body["metrics"]["taxonomyCandidateRate"]
    );
    assert_eq!(
        body["rawCounts"]["taxonomyCandidate"].as_u64(),
        Some(2)
    );
}

#[tokio::test]
#[ignore]
async fn outcomes_autonomy_outbox_link_breaks_down_by_status() {
    let app = common::TestApp::start().await;

    // 4 条 outbox 记录：2 sent、1 canceled、1 failed_terminal。
    // sendSuccessRate = 0.5、canceledRate = 0.25、failedTerminalRate = 0.25。
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let outbox = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_send_outbox");

    let now = DateTime::now();
    let mut docs = Vec::new();
    let mut push = |status: &str| {
        docs.push(doc! {
            "_id": ObjectId::new(),
            "workspace_id": &workspace_id,
            "account_id": &account_id,
            "contact_wxid": "user_outbox",
            "status": status,
            "created_at": now,
        });
    };
    push("sent");
    push("sent");
    push("canceled");
    push("failed_terminal");
    outbox
        .insert_many(docs, None)
        .await
        .expect("insert outbox docs");

    let body = call_metrics(&app).await;
    let outbox = &body["outboxLink"];
    assert_eq!(outbox["totalEnqueued"].as_u64(), Some(4));
    assert_eq!(outbox["sent"].as_u64(), Some(2));
    assert_eq!(outbox["canceled"].as_u64(), Some(1));
    assert_eq!(outbox["failedTerminal"].as_u64(), Some(1));
    assert_eq!(outbox["sendSuccessRate"].as_f64(), Some(0.5));
    assert_eq!(outbox["canceledRate"].as_f64(), Some(0.25));
    assert_eq!(outbox["failedTerminalRate"].as_f64(), Some(0.25));
}

// ── M3 / Task 70：`planner` 子段聚合 ───────────────────────────────────────
//
// 验证 `/api/outcomes/autonomy` 在响应 body 末尾追加的 `planner` 子段会按 kind
// 聚合 `agent_events` 中的 `strategic_planner_*` 事件，且 silent_tick 的
// `details.scanned / emitted` 也被正确累加。
//
// 场景：插入 4 条 strategic_planner_* 事件（不同 kind），断言：
// - silent.emitted == 2（两条 `strategic_planner_emit`）
// - commitment.overdueEmits == 1（一条 `strategic_planner_commitment_overdue`）
// - stagnation.backoff == 1（一条 `strategic_planner_stage_stagnation_backoff`）
// - 其它字段（tick / capped 等）落地为 0，不影响判定
#[tokio::test]
#[ignore]
async fn outcomes_autonomy_planner_section_aggregates_strategic_events() {
    let app = common::TestApp::start().await;

    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let events = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_events");

    let now = DateTime::now();
    let push = |kind: &str, contact_wxid: &str| -> Document {
        doc! {
            "_id": ObjectId::new(),
            "workspace_id": &workspace_id,
            "account_id": &account_id,
            "contact_wxid": contact_wxid,
            "kind": kind,
            "status": "ok",
            "summary": "M3 planner section integration test",
            "details": doc! {},
            "created_at": now,
        }
    };
    let docs = vec![
        push("strategic_planner_emit", "user_a"),
        push("strategic_planner_emit", "user_b"),
        push("strategic_planner_commitment_overdue", "user_c"),
        push("strategic_planner_stage_stagnation_backoff", "user_d"),
    ];
    events
        .insert_many(docs, None)
        .await
        .expect("insert planner events");

    let body = call_metrics(&app).await;
    let planner = &body["planner"];
    assert!(planner.is_object(), "planner 子段缺失：{body:?}");

    assert_eq!(
        planner["silent"]["emitted"].as_i64(),
        Some(2),
        "silent.emitted 应聚合两条 strategic_planner_emit, 实际 {:?}",
        planner["silent"]["emitted"]
    );
    assert_eq!(
        planner["commitment"]["overdueEmits"].as_i64(),
        Some(1),
        "commitment.overdueEmits 应聚合一条 strategic_planner_commitment_overdue, 实际 {:?}",
        planner["commitment"]["overdueEmits"]
    );
    assert_eq!(
        planner["stagnation"]["backoff"].as_i64(),
        Some(1),
        "stagnation.backoff 应聚合一条 strategic_planner_stage_stagnation_backoff, 实际 {:?}",
        planner["stagnation"]["backoff"]
    );

    // 没有 tick / capped / 其它 kind → 留 0，不应出现 null。
    assert_eq!(planner["silent"]["tick"].as_i64(), Some(0));
    assert_eq!(planner["silent"]["capped"].as_i64(), Some(0));
    assert_eq!(planner["silent"]["backoff"].as_i64(), Some(0));
    assert_eq!(planner["commitment"]["imminentEmits"].as_i64(), Some(0));
    assert_eq!(planner["stagnation"]["emitted"].as_i64(), Some(0));
}

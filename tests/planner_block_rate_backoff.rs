//! M3 Strategic Planner —— 反馈环 (block-rate backoff) 集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）；CI 通过
//! `cargo test --test planner_block_rate_backoff -- --ignored` 触发。
//!
//! 覆盖：
//! - 一个 managed + 静默 contact，过去 24h 命中 4 条 blocked_by_safety_guard +
//!   1 条 approved 的 run logs（block-rate=0.8，超 0.6 阈值，min_runs=3 满足）；
//! - 同 tick 应当**不**写 follow_up 任务，**也不**写 strategic_planner_emit；
//! - 应当写 1 条 `strategic_planner_silent_backoff` 事件，`details.blockRate >= threshold`；
//! - daily cap 不被消费——`EMIT_EVENT_KINDS` 不含 backoff kind。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::models::{AgentStatus, Contact};
use wechatagent::planner;

fn template(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
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
        custom_agent_instructions: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        deal_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

fn silent_managed(wxid: &str) -> Contact {
    let long_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 200 * 60 * 60 * 1000);
    Contact {
        last_inbound_at: Some(long_ago),
        ..template(wxid)
    }
}

/// 直接插入一条原始 BSON `agent_run_logs`，避免重复声明全部字段。
async fn insert_run_log(app: &common::TestApp, wxid: &str, status: &str) {
    let raw = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_run_logs");
    raw.insert_one(
        doc! {
            "_id": ObjectId::new(),
            "workspace_id": "default",
            "account_id": "default",
            "contact_wxid": wxid,
            "run_id": format!("run_{}", ObjectId::new().to_hex()),
            "trigger_kind": "inbound_message",
            "status": "completed",
            "final_review_status": status,
            "revision_applied": false,
            "autonomy_mode": "blocked",
            "review": doc! {},
            "created_at": DateTime::now(),
        },
        None,
    )
    .await
    .expect("insert run log");
}

#[tokio::test]
#[ignore]
async fn planner_silent_segment_skips_when_block_rate_above_threshold() {
    let app = common::TestApp::start().await;

    // 一个 managed + 静默 contact。
    app.state
        .db
        .contacts()
        .insert_one(silent_managed("user_blocky"), None)
        .await
        .expect("insert contact");

    // 过去 24h 内：4 条 blocked-like + 1 条 ok-like → block-rate = 0.8 > 0.6。
    for _ in 0..4 {
        insert_run_log(&app, "user_blocky", "blocked_by_safety_guard").await;
    }
    insert_run_log(&app, "user_blocky", "approved").await;

    planner::tick(&app.state).await.expect("planner tick");

    // 0 条 follow_up；0 条 emit；1 条 backoff。
    let task_count = app
        .state
        .db
        .tasks()
        .count_documents(
            doc! { "kind": "follow_up", "contact_wxid": "user_blocky" },
            None,
        )
        .await
        .expect("count follow_up");
    assert_eq!(task_count, 0, "block-rate 超阈值时不应 emit follow_up");

    let emit_events = app
        .state
        .db
        .events()
        .count_documents(
            doc! {
                "kind": "strategic_planner_emit",
                "contact_wxid": "user_blocky",
            },
            None,
        )
        .await
        .expect("count emit events");
    assert_eq!(emit_events, 0);

    let backoff_events = app
        .state
        .db
        .events()
        .count_documents(
            doc! {
                "kind": "strategic_planner_silent_backoff",
                "contact_wxid": "user_blocky",
            },
            None,
        )
        .await
        .expect("count backoff events");
    assert_eq!(
        backoff_events, 1,
        "应当写一条 strategic_planner_silent_backoff 事件"
    );

    // 验证 backoff 事件 details 字段：block-rate 与 threshold 都被记下。
    use futures::TryStreamExt;
    let details_cursor = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_events")
        .find(
            doc! {
                "kind": "strategic_planner_silent_backoff",
                "contact_wxid": "user_blocky",
            },
            None,
        )
        .await
        .expect("find backoff events");
    let docs: Vec<Document> = details_cursor
        .try_collect()
        .await
        .expect("collect backoff events");
    assert_eq!(docs.len(), 1);
    let detail = docs[0]
        .get_document("details")
        .expect("backoff details present");
    let rate = detail
        .get_f64("blockRate")
        .expect("blockRate field present");
    assert!(
        rate >= 0.6,
        "blockRate {} 应当 >= 0.6 阈值",
        rate
    );
    let blocked = detail.get_i64("blockedCount").expect("blockedCount");
    let ok = detail.get_i64("okCount").expect("okCount");
    assert_eq!(blocked, 4);
    assert_eq!(ok, 1);
}

#[tokio::test]
#[ignore]
async fn planner_silent_segment_passes_when_under_min_runs() {
    let app = common::TestApp::start().await;

    app.state
        .db
        .contacts()
        .insert_one(silent_managed("user_coldstart"), None)
        .await
        .expect("insert contact");

    // 仅 2 条 blocked-like，分母 < min_runs(=3) → 不参与判定，应放行。
    for _ in 0..2 {
        insert_run_log(&app, "user_coldstart", "blocked_by_safety_guard").await;
    }

    planner::tick(&app.state).await.expect("planner tick");

    let task_count = app
        .state
        .db
        .tasks()
        .count_documents(
            doc! { "kind": "follow_up", "contact_wxid": "user_coldstart" },
            None,
        )
        .await
        .expect("count follow_up");
    assert_eq!(task_count, 1, "min_runs 未达时反馈环不参与判定，应正常 emit");

    let backoff_events = app
        .state
        .db
        .events()
        .count_documents(
            doc! {
                "kind": "strategic_planner_silent_backoff",
                "contact_wxid": "user_coldstart",
            },
            None,
        )
        .await
        .expect("count backoff events");
    assert_eq!(backoff_events, 0, "冷启动 contact 不应写 backoff 事件");
}

//! M2 Strategic Planner —— stage_stagnation 扫描器集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）；CI 通过
//! `cargo test --test planner_stage_stagnation -- --ignored` 触发。
//!
//! 覆盖（5 个 contact）：
//! - 1×stagnant managed: customer_stage_updated_at=15d ago, last_inbound_at=15d ago → emit
//! - 1×terminal: customer_stage="closed_won" → 不 emit（mongo filter 排除）
//! - 1×recent inbound: last_inbound_at=1h 前 → 不 emit（mongo filter 排除）
//! - 1×non-managed: agent_status=normal → 不 emit
//! - 1×stagnant + cooldown: cooldown_until 在未来 → 不 emit
//!
//! 同时验证：
//! - `strategic_planner_stage_stagnation_tick` 每 tick 写一条；
//! - emit 事件 detail 含 `stage / idleDays / stageUpdatedAt`；
//! - 把 stagnant contact 的 customer_stage_updated_at 刷成 now → 再 tick 不再 emit。

mod common;

use mongodb::bson::{doc, DateTime, Document};
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
        customer_stage: None,
        customer_stage_updated_at: None,
        intent_level: None,
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
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn planner_emits_stage_stagnation_for_long_idle_managed_only() {
    let app = common::TestApp::start().await;
    let now_ms = DateTime::now().timestamp_millis();
    let fifteen_days_ms = 15 * 24 * 60 * 60 * 1000;

    // 1) 真正命中：managed + qualified + 15d 未变 + 15d 未 inbound
    let mut stagnant = template("user_stagnant");
    stagnant.customer_stage = Some("qualified".to_string());
    stagnant.customer_stage_updated_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));
    stagnant.last_inbound_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));

    // 2) 终状态：closed_won → 不 emit
    let mut terminal = template("user_terminal");
    terminal.customer_stage = Some("closed_won".to_string());
    terminal.customer_stage_updated_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));
    terminal.last_inbound_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));

    // 3) 近期 inbound：last_inbound_at 在 1h 前 → 不 emit（< 24h 阈值）
    let mut recent = template("user_recent");
    recent.customer_stage = Some("qualified".to_string());
    recent.customer_stage_updated_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));
    recent.last_inbound_at = Some(DateTime::from_millis(now_ms - 60 * 60 * 1000));

    // 4) 非 managed：normal status → 不 emit
    let mut non_managed = template("user_normal");
    non_managed.agent_status = AgentStatus::Normal;
    non_managed.customer_stage = Some("qualified".to_string());
    non_managed.customer_stage_updated_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));
    non_managed.last_inbound_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));

    // 5) cooldown：与 stagnant 同款，但 cooldown_until 在未来
    let mut cooled = template("user_cooldown");
    cooled.customer_stage = Some("qualified".to_string());
    cooled.customer_stage_updated_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));
    cooled.last_inbound_at = Some(DateTime::from_millis(now_ms - fifteen_days_ms));
    cooled.cooldown_until = Some(DateTime::from_millis(now_ms + 60 * 60 * 1000));

    app.state
        .db
        .contacts()
        .insert_many(
            &[stagnant, terminal, recent, non_managed, cooled],
            None,
        )
        .await
        .expect("insert seed contacts");

    planner::tick(&app.state).await.expect("first planner tick");

    // ── 断言 1：恰好 emit 1 条 follow_up，contact_wxid=user_stagnant ────────
    let follow_up_filter = doc! {
        "kind": "follow_up",
        "status": "pending",
    };
    use futures::TryStreamExt;
    let tasks: Vec<_> = app
        .state
        .db
        .tasks()
        .find(follow_up_filter.clone(), None)
        .await
        .expect("query follow-up tasks")
        .try_collect()
        .await
        .expect("collect follow-up tasks");
    assert_eq!(
        tasks.len(),
        1,
        "应当只对 stagnant managed contact emit 一条 follow_up, 实际: {tasks:?}"
    );
    let task = &tasks[0];
    assert_eq!(task.contact_wxid, "user_stagnant");
    assert!(
        task.content.starts_with("Planner: stage_stagnation"),
        "stage_stagnation content 必须以 Planner: stage_stagnation 起头, 实际: {}",
        task.content
    );
    assert!(
        task.content.contains("stage=qualified"),
        "stage_stagnation content 必须含 stage=qualified, 实际: {}",
        task.content
    );
    assert!(task.content.contains("idle="));
    assert!(
        task.review_required,
        "Planner emit 的 follow_up 必须保留 review_required"
    );

    // ── 断言 2：emit 事件 + tick 事件 ────────────────────────────────────────
    let emit_count = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_stage_stagnation" }, None)
        .await
        .expect("count stage_stagnation emit events");
    assert_eq!(emit_count, 1, "stage_stagnation emit 事件应当为 1 条");

    let tick_count = app
        .state
        .db
        .events()
        .count_documents(
            doc! { "kind": "strategic_planner_stage_stagnation_tick" },
            None,
        )
        .await
        .expect("count stage_stagnation tick events");
    assert_eq!(tick_count, 1, "每 tick 应记录一条 stage_stagnation_tick 事件");

    // 抽查 emit detail：含 stage / idleDays / stageUpdatedAt
    let emit_event = app
        .state
        .db
        .events()
        .find_one(doc! { "kind": "strategic_planner_stage_stagnation" }, None)
        .await
        .expect("query stage_stagnation emit event")
        .expect("emit event exists");
    let details = emit_event
        .details
        .as_ref()
        .expect("stage_stagnation event has details");
    assert_eq!(
        details.get_str("stage").expect("stage"),
        "qualified"
    );
    assert!(details.get("idleDays").is_some());
    assert!(details.get("stageUpdatedAt").is_some());

    // ── 断言 3：刷新 customer_stage_updated_at = now → 不再 emit ────────────
    let now = DateTime::now();
    app.state
        .db
        .contacts()
        .update_one(
            doc! { "wxid": "user_stagnant" },
            doc! { "$set": { "customer_stage_updated_at": now } },
            None,
        )
        .await
        .expect("refresh stage updated_at");

    // 把现有 follow_up 标记为已完成，让 has_pending_follow_up 不再阻挡。
    app.state
        .db
        .tasks()
        .update_many(
            doc! { "contact_wxid": "user_stagnant", "kind": "follow_up" },
            doc! { "$set": { "status": "completed" } },
            None,
        )
        .await
        .expect("complete prior follow-up");

    planner::tick(&app.state)
        .await
        .expect("second planner tick");

    let emit_count_after = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_stage_stagnation" }, None)
        .await
        .expect("count stage_stagnation emit events after second tick");
    assert_eq!(
        emit_count_after, 1,
        "刷新 stage_updated_at 后应不再 emit, 实际: {emit_count_after}"
    );

    let tick_count_after = app
        .state
        .db
        .events()
        .count_documents(
            doc! { "kind": "strategic_planner_stage_stagnation_tick" },
            None,
        )
        .await
        .expect("count stage_stagnation tick events after second tick");
    assert_eq!(
        tick_count_after, 2,
        "tick 事件应当 +1 (无论是否 emit, 每 tick 都写)"
    );
}

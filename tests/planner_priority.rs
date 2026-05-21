//! M3 Strategic Planner —— 跨 contact 优先级排序集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）；CI 通过
//! `cargo test --test planner_priority -- --ignored` 触发。
//!
//! 覆盖 commitment 段优先级——daily cap 撞顶时谁先被 emit：
//!
//! 1. `daily_cap=1`，3 个 contact 都满足 commitment 候选条件：
//!    - `user_high`：customer_stage=negotiation（权重 80）+ overdue
//!    - `user_mid`：customer_stage=qualified（权重 60）+ overdue
//!    - `user_low`：customer_stage=awareness（权重 40）+ overdue
//!    期望：emit 顺序按价值权重，`user_high` 拿到唯一一个 emit slot。
//!
//! 2. overdue 优先于 imminent：高价值 imminent 也输给低价值 overdue
//!    （reason 紧迫度排在 stage_priority 之前，按 plan §2.1）。
//!
//! 3. `STRATEGIC_PLANNER_PRIORITY_ENABLED=false` → 退化为 cursor 自然顺序。
//!    本集成测试通过直接 mutate `app.state.config.strategic_planner_priority_enabled`
//!    检验"关闭 flag → 拿到的不一定是最高 stage"。

mod common;

use std::time::Duration;

use mongodb::bson::{doc, DateTime, Document};
use wechatagent::models::{AgentStatus, CommitmentEntry, CommitmentRepr, Contact};
use wechatagent::planner;
use wechatagent::routes::AppState;

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

fn structured(id: &str, text: &str, due_at: Option<DateTime>) -> CommitmentRepr {
    CommitmentRepr::Structured(CommitmentEntry {
        id: id.to_string(),
        text: text.to_string(),
        due_at,
        created_at: DateTime::now(),
        extra: Document::new(),
    })
}

fn with_stage_overdue(wxid: &str, stage: &str) -> Contact {
    let now_ms = DateTime::now().timestamp_millis();
    let mut c = template(wxid);
    c.customer_stage = Some(stage.to_string());
    c.commitments = vec![structured(
        &format!("cmt-{wxid}"),
        "周五前给方案",
        Some(DateTime::from_millis(now_ms - 5 * 60 * 60 * 1000)),
    )];
    c
}

fn with_stage_imminent(wxid: &str, stage: &str) -> Contact {
    let now_ms = DateTime::now().timestamp_millis();
    let mut c = template(wxid);
    c.customer_stage = Some(stage.to_string());
    c.commitments = vec![structured(
        &format!("cmt-{wxid}"),
        "今晚给报价",
        Some(DateTime::from_millis(now_ms + 4 * 60 * 60 * 1000)),
    )];
    c
}

fn override_state_with_cap(app: &common::TestApp, daily_cap: i64, priority: bool) -> AppState {
    let mut config = app.state.config.clone();
    config.strategic_planner_daily_emit_cap = daily_cap;
    config.strategic_planner_priority_enabled = priority;
    AppState {
        db: app.state.db.clone(),
        mcp: app.state.mcp.clone(),
        llm: app.state.llm.clone(),
        config,
        prompt_pack_version: app.state.prompt_pack_version.clone(),
    }
}

#[tokio::test]
#[ignore]
async fn commitment_priority_picks_higher_stage_first_when_cap_is_tight() {
    let app = common::TestApp::start().await;

    // 三个 overdue contact，stage 权重各异：negotiation(80) / qualified(60) / awareness(40)。
    // 故意按"低 → 高" 顺序插入，证明排序依靠 priority key 而非 cursor 顺序。
    app.state
        .db
        .contacts()
        .insert_one(with_stage_overdue("user_low", "awareness"), None)
        .await
        .expect("insert low");
    tokio::time::sleep(Duration::from_millis(10)).await;
    app.state
        .db
        .contacts()
        .insert_one(with_stage_overdue("user_mid", "qualified"), None)
        .await
        .expect("insert mid");
    tokio::time::sleep(Duration::from_millis(10)).await;
    app.state
        .db
        .contacts()
        .insert_one(with_stage_overdue("user_high", "negotiation"), None)
        .await
        .expect("insert high");

    // daily cap=1：三个候选只能 emit 一个，必须是 user_high。
    let state = override_state_with_cap(&app, 1, true);
    planner::tick(&state).await.expect("planner tick");

    use futures::TryStreamExt;
    let tasks: Vec<_> = app
        .state
        .db
        .tasks()
        .find(doc! { "kind": "follow_up", "status": "pending" }, None)
        .await
        .expect("query follow_up tasks")
        .try_collect()
        .await
        .expect("collect tasks");
    assert_eq!(tasks.len(), 1, "daily cap=1 应只 emit 一条");
    assert_eq!(
        tasks[0].contact_wxid, "user_high",
        "应选 negotiation 阶段(权重 80)的高价值 contact"
    );

    // 撞 cap → 写一条 commitment capped 事件。
    let capped = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_capped" }, None)
        .await
        .expect("count capped events");
    assert_eq!(capped, 1, "cap 撞顶时应写一条 strategic_planner_capped 事件");
}

#[tokio::test]
#[ignore]
async fn commitment_priority_overdue_beats_imminent_even_with_lower_stage() {
    let app = common::TestApp::start().await;

    // user_a：awareness(40) + overdue
    // user_b：negotiation(80) + imminent
    // 按计划 §2.1 排序键：reason 紧迫度优先 → overdue (Reason 排在 Imminent 前)
    // 即便 imminent 那一边 stage 更高，daily cap=1 时也应输给 overdue。
    app.state
        .db
        .contacts()
        .insert_one(with_stage_imminent("user_b", "negotiation"), None)
        .await
        .expect("insert imminent");
    app.state
        .db
        .contacts()
        .insert_one(with_stage_overdue("user_a", "awareness"), None)
        .await
        .expect("insert overdue");

    let state = override_state_with_cap(&app, 1, true);
    planner::tick(&state).await.expect("planner tick");

    use futures::TryStreamExt;
    let tasks: Vec<_> = app
        .state
        .db
        .tasks()
        .find(doc! { "kind": "follow_up", "status": "pending" }, None)
        .await
        .expect("query follow_up tasks")
        .try_collect()
        .await
        .expect("collect tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].contact_wxid, "user_a",
        "overdue 紧迫度优先于 imminent，即便后者 stage 更高"
    );
}

#[tokio::test]
#[ignore]
async fn commitment_priority_disabled_falls_back_to_natural_order() {
    let app = common::TestApp::start().await;

    // 三个 overdue contact，priority_enabled=false → 按 mongo cursor 自然顺序消费 cap。
    // 我们插入顺序是 low → mid → high，cursor 通常按 _id 升序（也即插入顺序）。
    // 关键断言：被 emit 的不必是最高 stage（即与 priority=true 测试结果不同）。
    app.state
        .db
        .contacts()
        .insert_one(with_stage_overdue("user_low", "awareness"), None)
        .await
        .expect("insert low");
    tokio::time::sleep(Duration::from_millis(10)).await;
    app.state
        .db
        .contacts()
        .insert_one(with_stage_overdue("user_mid", "qualified"), None)
        .await
        .expect("insert mid");
    tokio::time::sleep(Duration::from_millis(10)).await;
    app.state
        .db
        .contacts()
        .insert_one(with_stage_overdue("user_high", "negotiation"), None)
        .await
        .expect("insert high");

    let state = override_state_with_cap(&app, 1, false);
    planner::tick(&state).await.expect("planner tick");

    use futures::TryStreamExt;
    let tasks: Vec<_> = app
        .state
        .db
        .tasks()
        .find(doc! { "kind": "follow_up", "status": "pending" }, None)
        .await
        .expect("query follow_up tasks")
        .try_collect()
        .await
        .expect("collect tasks");
    assert_eq!(tasks.len(), 1);
    // 自然顺序通常拿到第一个插入的 user_low；不强约束 cursor 顺序，但**绝不可**
    // 是 user_high（如果是 user_high 说明排序仍生效）。
    assert_ne!(
        tasks[0].contact_wxid, "user_high",
        "priority_enabled=false 时不应当再是 stage 权重最高那个"
    );
}

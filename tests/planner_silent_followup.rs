//! M1 Strategic Planner —— 静默跟进扫描器集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）；CI 通过
//! `cargo test --test planner_silent_followup -- --ignored` 触发。
//!
//! 覆盖：
//! - 5 个 managed + 静默 contact 应被 emit follow_up；
//! - 3 个 normal contact 不应被 emit；
//! - 1 个 cooldown_until=未来 的 managed contact 不应被 emit；
//! - 同一 tick 写入 1 条 strategic_planner_tick + 5 条 strategic_planner_emit；
//! - 再 tick 一次保持幂等（不重复 emit）。

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
        custom_agent_instructions: None,
        created_at: now,
        updated_at: now,
    }
}

/// 构造 `last_inbound_at` 远早于阈值（即"已静默"）的 managed 联系人。
fn silent_managed(wxid: &str) -> Contact {
    let long_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 200 * 60 * 60 * 1000);
    Contact {
        last_inbound_at: Some(long_ago),
        ..template(wxid)
    }
}

#[tokio::test]
#[ignore]
async fn planner_emits_follow_up_for_silent_managed_contacts_only() {
    let app = common::TestApp::start().await;
    // 默认 silent_threshold = 72h；这里把 contact.last_inbound_at 设到 200h 前。
    let mut contacts = Vec::new();
    for idx in 0..5 {
        contacts.push(silent_managed(&format!("user_managed_{idx}")));
    }
    for idx in 0..3 {
        // normal 状态联系人即便静默也不应被 emit。
        let mut c = silent_managed(&format!("user_normal_{idx}"));
        c.agent_status = AgentStatus::Normal;
        contacts.push(c);
    }
    // cooldown_until 在未来——应被 mongo filter 排除。
    let mut cooled = silent_managed("user_cooldown");
    cooled.cooldown_until = Some(DateTime::from_millis(
        DateTime::now().timestamp_millis() + 60 * 60 * 1000,
    ));
    contacts.push(cooled);

    app.state
        .db
        .contacts()
        .insert_many(&contacts, None)
        .await
        .expect("insert seed contacts");

    planner::tick(&app.state).await.expect("first planner tick");

    let follow_up_filter = doc! {
        "kind": "follow_up",
        "status": "pending",
    };
    let task_count = app
        .state
        .db
        .tasks()
        .count_documents(follow_up_filter.clone(), None)
        .await
        .expect("count follow-up tasks");
    assert_eq!(task_count, 5, "应当为 5 个 managed+静默 contact 各 emit 一条 follow_up");

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
    for task in &tasks {
        assert!(
            task.content.starts_with("Planner: silent_follow_up"),
            "follow-up content 必须以 Planner: silent_follow_up 起头, 实际: {}",
            task.content
        );
        assert!(
            task.contact_wxid.starts_with("user_managed_"),
            "只有 managed 静默联系人应被 emit, 实际: {}",
            task.contact_wxid
        );
        assert!(task.review_required, "Planner emit 的 follow_up 必须保留 review_required");
    }

    let tick_events = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_tick" }, None)
        .await
        .expect("count tick events");
    assert_eq!(tick_events, 1, "每个 tick 应记录 1 条 strategic_planner_tick 事件");

    let emit_events = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_emit" }, None)
        .await
        .expect("count emit events");
    assert_eq!(emit_events, 5, "每个被 emit 的 follow_up 都应有 strategic_planner_emit 事件");

    // 再跑一次 tick：candidate 还是同样这 5 个，但已存在 pending follow_up，
    // 应被幂等跳过；任务总数仍为 5，emit 事件总数仍为 5，tick 事件 +1。
    planner::tick(&app.state).await.expect("second planner tick");

    let task_count_after = app
        .state
        .db
        .tasks()
        .count_documents(follow_up_filter, None)
        .await
        .expect("count follow-up tasks after second tick");
    assert_eq!(
        task_count_after, 5,
        "存在 pending follow_up 时应幂等跳过, 不重复 emit"
    );

    let emit_events_after = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_emit" }, None)
        .await
        .expect("count emit events after second tick");
    assert_eq!(
        emit_events_after, 5,
        "幂等 tick 不应再写入 strategic_planner_emit"
    );

    let tick_events_after = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_tick" }, None)
        .await
        .expect("count tick events after second tick");
    assert_eq!(
        tick_events_after, 2,
        "每个 tick 都写一条 strategic_planner_tick 事件, 即便 emit=0"
    );
}

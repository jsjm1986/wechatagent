//! 作息门控（quiet hours，#69）入站延迟排程集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）；CI 通过
//! `cargo test --test quiet_hours_deferral -- --ignored` 触发。
//!
//! 时区判定本身是纯函数（`quiet_hours::hour_in_offset` / `next_wake_utc_ms`，由
//! `src/agent/quiet_hours.rs` 的单测覆盖跨偏移 / 跨午夜 / 负偏移），`Utc::now()` 不可
//! 注入，故集成测试不验"现在是否静默"，只直接驱动 [`ensure_wake_followup_task`]
//! 验证 DB 侧契约：
//! - 排出 1 条 `deferred_inbound_reply` 跟进任务（pending、review_required、run_at 在未来）；
//! - 写 1 条 `quiet_hours_deferred_inbound` 观测事件（status=deferred）；
//! - 静默时段连发（重复调用）幂等：任务仍 1 条、事件仍 1 条。

mod common;

use mongodb::bson::{doc, DateTime, Document};
use wechatagent::models::{AgentStatus, Contact};
use wechatagent::webhooks::ensure_wake_followup_task;

const DEFERRED_KIND: &str = "deferred_inbound_reply";

fn managed_contact(wxid: &str) -> Contact {
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
        operation_mode_override: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        outcome_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn quiet_hours_defers_inbound_and_is_idempotent() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("user_quiet_1");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert seed contact");

    // 醒来小时 8、时区偏移 +8（中国）——run_at 落在未来某个本地 08:00。
    ensure_wake_followup_task(&app.state, &contact, 8, 8)
        .await
        .expect("first ensure_wake_followup_task");

    let task_filter = doc! { "kind": DEFERRED_KIND, "contact_wxid": &contact.wxid };
    let task_count = app
        .state
        .db
        .tasks()
        .count_documents(task_filter.clone(), None)
        .await
        .expect("count deferred tasks");
    assert_eq!(task_count, 1, "应排出 1 条 deferred_inbound_reply 任务");

    let task = app
        .state
        .db
        .tasks()
        .find_one(task_filter.clone(), None)
        .await
        .expect("query deferred task")
        .expect("deferred task should exist");
    assert_eq!(task.status, "pending", "新排任务应为 pending");
    assert!(task.review_required, "延迟回复任务必须保留 review_required");
    assert!(
        task.run_at.timestamp_millis() > DateTime::now().timestamp_millis(),
        "run_at 必须落在未来（下一次醒来时刻）"
    );
    assert!(
        task.expires_at.is_some(),
        "应设置 expires_at，过期未跑则作废"
    );

    let event_filter = doc! {
        "kind": "quiet_hours_deferred_inbound",
        "contact_wxid": &contact.wxid,
    };
    let event_count = app
        .state
        .db
        .events()
        .count_documents(event_filter.clone(), None)
        .await
        .expect("count deferral events");
    assert_eq!(event_count, 1, "应写 1 条 quiet_hours_deferred_inbound 观测事件");

    let event = app
        .state
        .db
        .events()
        .find_one(event_filter.clone(), None)
        .await
        .expect("query deferral event")
        .expect("deferral event should exist");
    assert_eq!(event.status, "deferred", "事件 status 应为 deferred");

    // 静默时段连发：再次调用应命中去重，task / event 都不增加。
    ensure_wake_followup_task(&app.state, &contact, 8, 8)
        .await
        .expect("second ensure_wake_followup_task (idempotent)");

    let task_count_after = app
        .state
        .db
        .tasks()
        .count_documents(task_filter, None)
        .await
        .expect("count deferred tasks after second call");
    assert_eq!(
        task_count_after, 1,
        "已存在 pending wake 任务时应幂等跳过，不重复排"
    );

    let event_count_after = app
        .state
        .db
        .events()
        .count_documents(event_filter, None)
        .await
        .expect("count deferral events after second call");
    assert_eq!(
        event_count_after, 1,
        "去重命中（未真正新建任务）不应再写观测事件，避免连发刷屏"
    );
}

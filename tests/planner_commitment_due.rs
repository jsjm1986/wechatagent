//! M2 Strategic Planner —— commitment 到期扫描器集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）；CI 通过
//! `cargo test --test planner_commitment_due -- --ignored` 触发。
//!
//! 覆盖（6 个 contact）：
//! - 1×overdue managed   → emit `commitment_overdue`
//! - 1×imminent managed  → emit `commitment_imminent`
//! - 1×plain（旧字符串） → 不 emit（`Plain` 无 due_at）
//! - 1×future managed    → 不 emit（超出 imminent_window）
//! - 1×normal-status     → 不 emit（mongo filter 排除）
//! - 1×cooldown          → 不 emit（mongo filter 排除）
//!
//! 同时验证：
//! - 第二次 tick 对同一 commitment 不再重复 emit（24h dedup 生效）；
//! - `strategic_planner_commitment_tick` 每 tick 写一条；
//! - emit 事件 detail 含 `commitmentId / dueAt / reason`。

mod common;

use mongodb::bson::{doc, DateTime, Document};
use wechatagent::models::{AgentStatus, CommitmentEntry, CommitmentRepr, Contact};
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
        operation_mode_override: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        outcome_events: Vec::new(),
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

#[tokio::test]
#[ignore]
async fn planner_emits_commitment_overdue_and_imminent_only() {
    let app = common::TestApp::start().await;
    let now_ms = DateTime::now().timestamp_millis();

    // 1) overdue: due_at 在 5 小时前
    let mut overdue = template("user_overdue");
    overdue.commitments = vec![structured(
        "cmt-overdue-1",
        "周五前给方案",
        Some(DateTime::from_millis(now_ms - 5 * 60 * 60 * 1000)),
    )];

    // 2) imminent: due_at 在 4 小时后（默认 imminent window=8h）
    let mut imminent = template("user_imminent");
    imminent.commitments = vec![structured(
        "cmt-imminent-1",
        "今晚给报价",
        Some(DateTime::from_millis(now_ms + 4 * 60 * 60 * 1000)),
    )];

    // 3) plain：旧字符串形态，无 due_at → 不 emit
    let mut plain = template("user_plain");
    plain.commitments = vec![CommitmentRepr::Plain("某天给个反馈".to_string())];

    // 4) future managed: due_at 超过 8h imminent window → 不 emit
    let mut future = template("user_future");
    future.commitments = vec![structured(
        "cmt-future-1",
        "下周三发样品",
        Some(DateTime::from_millis(now_ms + 5 * 24 * 60 * 60 * 1000)),
    )];

    // 5) normal status：与 overdue 同款 commitment，但 status=normal → mongo filter 排除
    let mut normal = template("user_normal");
    normal.agent_status = AgentStatus::Normal;
    normal.commitments = vec![structured(
        "cmt-normal-1",
        "也过期了",
        Some(DateTime::from_millis(now_ms - 10 * 60 * 60 * 1000)),
    )];

    // 6) cooldown：与 overdue 同款 commitment，但 cooldown_until 在未来 → mongo filter 排除
    let mut cooled = template("user_cooldown");
    cooled.commitments = vec![structured(
        "cmt-cool-1",
        "也过期了",
        Some(DateTime::from_millis(now_ms - 10 * 60 * 60 * 1000)),
    )];
    cooled.cooldown_until = Some(DateTime::from_millis(now_ms + 60 * 60 * 1000));

    app.state
        .db
        .contacts()
        .insert_many(
            &[overdue, imminent, plain, future, normal, cooled],
            None,
        )
        .await
        .expect("insert seed contacts");

    planner::tick(&app.state).await.expect("first planner tick");

    // ── 断言 1：恰好 emit 2 条 follow_up（overdue + imminent） ───────────────
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
    assert_eq!(
        task_count, 2,
        "应当只对 overdue 和 imminent 各 emit 一条 follow_up"
    );

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

    let mut overdue_tasks = 0;
    let mut imminent_tasks = 0;
    for task in &tasks {
        assert!(
            task.review_required,
            "Planner emit 的 follow_up 必须保留 review_required"
        );
        assert!(
            task.contact_wxid == "user_overdue" || task.contact_wxid == "user_imminent",
            "只有 overdue/imminent contact 应被 emit, 实际: {}",
            task.contact_wxid
        );
        if task.content.starts_with("Planner: commitment_overdue") {
            assert_eq!(task.contact_wxid, "user_overdue");
            assert!(task.content.contains("id=cmt-overdue-1"));
            assert!(task.content.contains("text="));
            overdue_tasks += 1;
        } else if task.content.starts_with("Planner: commitment_imminent") {
            assert_eq!(task.contact_wxid, "user_imminent");
            assert!(task.content.contains("id=cmt-imminent-1"));
            imminent_tasks += 1;
        } else {
            panic!("unexpected follow_up content: {}", task.content);
        }
    }
    assert_eq!(overdue_tasks, 1);
    assert_eq!(imminent_tasks, 1);

    // ── 断言 2：commitment_tick 写一条；emit 事件 overdue/imminent 各 1 条 ──
    let commitment_tick = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_commitment_tick" }, None)
        .await
        .expect("count commitment tick events");
    assert_eq!(commitment_tick, 1, "每 tick 应记录一条 commitment_tick 事件");

    let overdue_emit = app
        .state
        .db
        .events()
        .count_documents(
            doc! { "kind": "strategic_planner_commitment_overdue" },
            None,
        )
        .await
        .expect("count overdue emit events");
    assert_eq!(overdue_emit, 1);

    let imminent_emit = app
        .state
        .db
        .events()
        .count_documents(
            doc! { "kind": "strategic_planner_commitment_imminent" },
            None,
        )
        .await
        .expect("count imminent emit events");
    assert_eq!(imminent_emit, 1);

    // 抽查：overdue 事件 detail 含 commitmentId / reason / dueAt
    let overdue_event = app
        .state
        .db
        .events()
        .find_one(
            doc! { "kind": "strategic_planner_commitment_overdue" },
            None,
        )
        .await
        .expect("query overdue event")
        .expect("overdue event exists");
    let details = overdue_event
        .details
        .as_ref()
        .expect("overdue event has details");
    assert_eq!(
        details.get_str("commitmentId").expect("commitmentId"),
        "cmt-overdue-1"
    );
    assert_eq!(
        details.get_str("reason").expect("reason"),
        "commitment_overdue"
    );
    assert!(
        details.get("dueAt").is_some(),
        "overdue event detail 必须含 dueAt"
    );

    // ── 断言 3：再 tick 一次 → 已存在 pending follow_up，幂等跳过 ──────────
    planner::tick(&app.state)
        .await
        .expect("second planner tick");

    let task_count_after = app
        .state
        .db
        .tasks()
        .count_documents(follow_up_filter, None)
        .await
        .expect("count follow-up tasks after second tick");
    assert_eq!(
        task_count_after, 2,
        "存在 pending follow_up 时不应重复 emit"
    );

    // emit 事件总数仍 = 2（dedup + has_pending_follow_up 双保险）。
    let overdue_emit_after = app
        .state
        .db
        .events()
        .count_documents(
            doc! { "kind": "strategic_planner_commitment_overdue" },
            None,
        )
        .await
        .expect("count overdue emit events after second tick");
    assert_eq!(overdue_emit_after, 1);

    let imminent_emit_after = app
        .state
        .db
        .events()
        .count_documents(
            doc! { "kind": "strategic_planner_commitment_imminent" },
            None,
        )
        .await
        .expect("count imminent emit events after second tick");
    assert_eq!(imminent_emit_after, 1);

    // commitment_tick 事件 +1（无论是否 emit，每次 tick 都写）。
    let commitment_tick_after = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_commitment_tick" }, None)
        .await
        .expect("count commitment tick events after second tick");
    assert_eq!(commitment_tick_after, 2);
}

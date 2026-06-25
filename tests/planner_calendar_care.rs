//! §3.7 Strategic Planner —— 主动情绪关怀（scan_calendar）集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）；CI 通过
//! `cargo test --test planner_calendar_care -- --ignored` 触发。
//!
//! 覆盖：
//! - 情感陪伴 active profile（calendar.enabled=true + anniversaries date_dimension）下，
//!   一个 managed contact 的 memory_card 含「今日生日」结构化纪念日 → emit calendar_care；
//! - DEFAULT 销售 active profile（calendar 关、无 date_dimension）下，同样的 contact +
//!   纪念日 → **零 emit**（销售域零扰动护栏）；
//! - 再 tick 一次保持幂等（已有 pending follow_up 不重复 emit）。

mod common;

use mongodb::bson::{doc, DateTime, Document};
use serial_test::serial;
use wechatagent::agent::{
    default_domain_profile, example_emotional_companion_profile,
    invalidate_global_domain_profile_cache,
};
use wechatagent::models::{AgentStatus, Contact, DomainProfile, MemoryCardTyped, OperatingMemory};
use wechatagent::planner;

const WORKSPACE: &str = "default";
const ACCOUNT: &str = "default";

fn contact_template(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: None,
        workspace_id: WORKSPACE.to_string(),
        account_id: ACCOUNT.to_string(),
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

/// 今日（运营方 +8 时区）的 "MM-DD"，让种子纪念日无论何时跑都恰好命中"今日"。
fn today_month_day_plus8() -> String {
    use chrono::Datelike;
    let shifted = chrono::Utc::now() + chrono::Duration::hours(8);
    let d = shifted.date_naive();
    format!("{:02}-{:02}", d.month(), d.day())
}

/// 构造一份 memory_card.extra.anniversaries 含「今日生日」结构化条目的 operating_memory。
fn memory_with_today_anniversary(wxid: &str) -> OperatingMemory {
    let now = DateTime::now();
    let mut extra = Document::new();
    extra.insert(
        "anniversaries",
        mongodb::bson::bson!([
            { "label": "她生日", "date": today_month_day_plus8(), "recurring": true }
        ]),
    );
    OperatingMemory {
        id: None,
        workspace_id: WORKSPACE.to_string(),
        account_id: ACCOUNT.to_string(),
        contact_wxid: wxid.to_string(),
        user_understanding: Document::new(),
        relationship_state: Document::new(),
        product_fit: Document::new(),
        next_action: Document::new(),
        context_pack: Document::new(),
        context_pack_version: 0,
        context_pack_updated_at: None,
        memory_card: MemoryCardTyped { extra, ..Default::default() },
        memory_card_version: 1,
        memory_card_updated_at: Some(now),
        created_at: now,
        updated_at: now,
    }
}

/// 把一份 profile 以 is_active=true 落库，并失效进程级缓存（让 scan_calendar 立即见到）。
async fn seed_active_profile(db: &wechatagent::db::Database, mut profile: DomainProfile) {
    profile.is_active = true;
    profile.current_version = true;
    db.domain_profiles()
        .insert_one(&profile, None)
        .await
        .expect("insert active profile");
    invalidate_global_domain_profile_cache();
}

#[tokio::test]
#[ignore]
#[serial]
async fn calendar_care_emits_for_emotional_profile_today_anniversary() {
    let app = common::TestApp::start().await;

    // 情感陪伴 profile：calendar 开 + anniversaries date_dimension。
    seed_active_profile(&app.state.db, example_emotional_companion_profile(WORKSPACE)).await;

    let contact = contact_template("user_companion");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    app.state
        .db
        .operating_memories()
        .insert_one(&memory_with_today_anniversary("user_companion"), None)
        .await
        .expect("insert memory");

    planner::tick(&app.state).await.expect("first planner tick");

    let care_filter = doc! { "kind": "follow_up", "status": "pending" };
    let tasks: Vec<_> = {
        use futures::TryStreamExt;
        app.state
            .db
            .tasks()
            .find(care_filter.clone(), None)
            .await
            .expect("query tasks")
            .try_collect()
            .await
            .expect("collect tasks")
    };
    assert_eq!(tasks.len(), 1, "今日纪念日应 emit 一条 calendar_care follow_up");
    assert!(
        tasks[0].content.starts_with("Planner: calendar_care"),
        "content 应以 Planner: calendar_care 起头，实际: {}",
        tasks[0].content
    );
    assert!(tasks[0].content.contains("她生日"), "content 应含纪念日标签");
    assert!(tasks[0].review_required, "calendar_care follow_up 必须保留 review_required");

    let emit_events = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_calendar_care" }, None)
        .await
        .expect("count care events");
    assert_eq!(emit_events, 1, "应写一条 strategic_planner_calendar_care 事件");

    // 幂等：再 tick，已有 pending follow_up → 不重复 emit。
    planner::tick(&app.state).await.expect("second planner tick");
    let after = app
        .state
        .db
        .tasks()
        .count_documents(care_filter, None)
        .await
        .expect("count after");
    assert_eq!(after, 1, "存在 pending follow_up 时应幂等跳过");
}

#[tokio::test]
#[ignore]
#[serial]
async fn calendar_care_no_emit_for_default_sales_profile() {
    let app = common::TestApp::start().await;

    // DEFAULT 销售 profile：calendar 关、无 date_dimension → scan_calendar 整段 no-op。
    seed_active_profile(&app.state.db, default_domain_profile(WORKSPACE)).await;

    let contact = contact_template("user_sales");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    // 即便 memory 里塞了今日纪念日，销售域也不该触发（零扰动护栏）。
    app.state
        .db
        .operating_memories()
        .insert_one(&memory_with_today_anniversary("user_sales"), None)
        .await
        .expect("insert memory");

    planner::tick(&app.state).await.expect("planner tick");

    let care_events = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_calendar_care" }, None)
        .await
        .expect("count care events");
    assert_eq!(care_events, 0, "DEFAULT 销售域不应触发 calendar_care（零扰动）");

    // calendar tick 事件也不应写（无 date_dimension → 提前 return，连 tick 事件都不写）。
    let tick_events = app
        .state
        .db
        .events()
        .count_documents(doc! { "kind": "strategic_planner_calendar_tick" }, None)
        .await
        .expect("count calendar tick events");
    assert_eq!(tick_events, 0, "无 date_dimension 维度时 scan_calendar 应提前短路、不写 tick");
}

//! campaign 营销活动派发红线集成测试:命中0人拒绝 / 跨workspace NotFound / dispatch建task /
//! 二次dispatch去重(dispatchedCount=0)。全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test campaign_dispatch_integration -- --ignored`。
//!
//! ## 形态:直调 dispatch_campaign handler 真函数,验真实 DB 副作用(建 follow_up task / 去重)。
//! 空 SegmentFilter 命中 workspace+account 内所有 contact(粗筛只按 workspace_id+account_id)。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{AgentStatus, Campaign, Contact, SegmentFilter};
use wechatagent::routes::campaigns::dispatch_campaign;

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "camp_admin".to_string(),
        username: "camp_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// managed contact,wxid 唯一,workspace/account 对齐 campaign。
fn make_contact(ws: &str, acc: &str, wxid: &str) -> Contact {
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
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
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

/// 空 filter campaign(命中 workspace+account 内全部 contact)。返回其 ObjectId。
fn make_campaign(ws: &str, acc: &str) -> Campaign {
    let now = DateTime::now();
    Campaign {
        id: Some(ObjectId::new()),
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        title: "促活".to_string(),
        intent_text: "回访问候".to_string(),
        segment_filter: SegmentFilter::default(),
        status: "draft".to_string(),
        target_count: None,
        dispatched_count: 0,
        created_by: "camp_admin".to_string(),
        created_at: now,
        updated_at: now,
    }
}

/// 红线:命中 0 人(无匹配 contact)→ BadRequest,不静默"派发成功 0 人"。
#[tokio::test]
#[ignore]
async fn dispatch_zero_hits_rejected() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    // 不 seed 任何 contact → 命中 0 人

    let result = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(result.is_err(), "命中 0 人必须 BadRequest,不静默成功");
}

/// 红线:跨 workspace dispatch → NotFound(handler 注入 current_workspace 到 filter)。
#[tokio::test]
#[ignore]
async fn dispatch_cross_workspace_not_found() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");

    // 用 other_workspace 视角 dispatch → filter {_id, workspaceId:other} 不匹配 → NotFound
    let result = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin("other_workspace")),
        Path(cid.to_hex()),
    )
    .await;
    assert!(result.is_err(), "跨 workspace dispatch 必须 NotFound");
}

/// 设计意图:命中 N 人 → 建 N 条 follow_up task(走 gateway 的证据);
/// 二次 dispatch 同集 → campaign_sends 唯一索引去重 → dispatchedCount=0。
#[tokio::test]
#[ignore]
async fn dispatch_builds_tasks_and_dedups_on_repeat() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    // seed 2 个 managed contact → 命中 2 人
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_a"), None).await.expect("seed wx_a");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_b"), None).await.expect("seed wx_b");

    let tasks_before = app
        .state
        .db
        .tasks()
        .count_documents(doc! {}, None)
        .await
        .expect("count tasks before");

    // 首次 dispatch
    let resp1 = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("首次 dispatch 应成功");
    assert_eq!(
        resp1.0["dispatchedCount"].as_i64(),
        Some(2),
        "命中 2 人首次 dispatch 应派 2 条,实际 {}",
        resp1.0["dispatchedCount"]
    );
    let tasks_after_1 = app.state.db.tasks().count_documents(doc! {}, None).await.expect("count after 1");
    assert_eq!(
        tasks_after_1 - tasks_before,
        2,
        "命中 2 人应建 2 条 follow_up task(走 gateway 证据)"
    );

    // 二次 dispatch 同集 → 唯一索引去重 → dispatchedCount=0、不再新增 task
    let resp2 = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("二次 dispatch 应成功(去重不报错)");
    assert_eq!(
        resp2.0["dispatchedCount"].as_i64(),
        Some(0),
        "二次 dispatch 同集应全去重,dispatchedCount=0,实际 {}",
        resp2.0["dispatchedCount"]
    );
    let tasks_after_2 = app.state.db.tasks().count_documents(doc! {}, None).await.expect("count after 2");
    assert_eq!(tasks_after_2, tasks_after_1, "二次 dispatch 去重后不应新增 task");
}

/// KC-01/03 补偿回滚：dispatch 循环中 agent_tasks insert 被 validator 拒 → task insert 失败
/// → 补偿删掉刚占位的 send → campaign_sends 无孤儿（该 contact 无残留 send 记录）、dispatch 返 Err。
#[tokio::test]
#[ignore]
async fn dispatch_task_insert_failure_rolls_back_send() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_rollback"), None).await.expect("seed contact");

    // 装 validator：让 agent_tasks 的 insert 确定性失败（拒绝所有 kind=follow_up 的插入）。
    let _ = app.state.db.raw().create_collection("agent_tasks", None).await;
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "agent_tasks",
                "validator": { "kind": { "$ne": "follow_up" } },
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install agent_tasks validator");

    let result = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(result.is_err(), "task insert 失败应中断并返 Err");

    // 核心：补偿回滚后无孤儿 send（该 campaign 下 0 条 campaign_sends）。
    let orphan_sends = app
        .state
        .db
        .campaign_sends()
        .count_documents(doc! { "campaignId": cid }, None)
        .await
        .expect("count sends");
    assert_eq!(orphan_sends, 0, "task insert 失败须补偿删除 send,不留孤儿(KC-01)");
}

/// KC-02 status 门：completed 活动 dispatch → BadRequest（防重复推送）。
#[tokio::test]
#[ignore]
async fn dispatch_completed_campaign_rejected() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let mut campaign = make_campaign(&ws, &acc);
    campaign.status = "completed".to_string();
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_done"), None).await.expect("seed contact");

    let result = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(result.is_err(), "completed 活动不可再 dispatch(防重推)");
}

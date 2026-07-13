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
use wechatagent::models::{AgentStatus, Campaign, CampaignSend, Contact, SegmentFilter};
use wechatagent::routes::campaigns::dispatch_campaign;
use wechatagent::routes::campaigns::preview_campaign;

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
        last_dispatch_target_count: None,
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
/// 首次 dispatch 成功后 campaign 置 completed → 二次 dispatch 被 KC-02 status 前置门
/// 以 BadRequest 拒(completed 不可再派发,防已完成活动对后来新命中者继续扩张受众),
/// 且门在圈人前拦截→不新增任何 task。
/// (KC-02 前旧契约是"completed 可反复 dispatch 靠 unique 索引幂等去重返0",
///  该旧行为语义错误,已被 status 门有意取代,见 findings KC-02。)
#[tokio::test]
#[ignore]
async fn dispatch_builds_tasks_then_rejects_repeat_after_completed() {
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

    // 首次成功后 campaign 已置 completed → 二次 dispatch 被 KC-02 status 门以 BadRequest 拒。
    // (KC-02 前旧契约靠 unique 索引幂等返 dispatchedCount=0,现由门直接拒绝取代。)
    let result2 = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(
        matches!(result2, Err(wechatagent::error::AppError::BadRequest(_))),
        "首次成功后 campaign=completed,二次 dispatch 应被 status 门以 BadRequest 拒,实际 {:?}",
        result2.map(|r| r.0.clone())
    );
    // 门在圈人前拦截 → 二次不新增任何 task(比旧的 unique 去重更早、更强的防重推)。
    let tasks_after_2 = app.state.db.tasks().count_documents(doc! {}, None).await.expect("count after 2");
    assert_eq!(tasks_after_2, tasks_after_1, "二次 dispatch 被门拒后不应新增 task");
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
    // 精确断言 status 门的 BadRequest（非圈人/NotFound 等其它早退）——门在圈人前(:314)，
    // 已 seed contact 对齐 ws，故唯一可达 Err 即 status 门；类型断言抵御未来 handler 早退分支变动。
    assert!(
        matches!(result, Err(wechatagent::error::AppError::BadRequest(_))),
        "completed 活动应被 status 门以 BadRequest 拒(防重推)，实际 {:?}",
        result.err()
    );
}

/// KC-04/07：受众粗筛候选超过 campaign_max_audience → preview 返 BadRequest（不静默截断受众）。
#[tokio::test]
#[ignore]
async fn preview_rejects_when_coarse_audience_exceeds_max() {
    let mut app = TestApp::start().await;
    app.state.config.campaign_max_audience = 3; // 小上限便于确定性触发
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    // seed 4 个 managed contact（> 上限 3）→ 粗筛候选超限
    for wx in ["wx_1", "wx_2", "wx_3", "wx_4"] {
        app.state.db.contacts().insert_one(make_contact(&ws, &acc, wx), None).await.expect("seed contact");
    }
    let result = preview_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(
        matches!(result, Err(wechatagent::error::AppError::BadRequest(_))),
        "粗筛候选超过上限须 BadRequest（回退守卫即绿变红），实际 {:?}",
        result.map(|r| r.0.clone())
    );
}

/// KC-04/07：粗筛候选正好等于上限 → preview 成功、targetCount == 上限（探测法边界）。
#[tokio::test]
#[ignore]
async fn preview_succeeds_at_exactly_max() {
    let mut app = TestApp::start().await;
    app.state.config.campaign_max_audience = 3;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    // seed 正好 3 个 → 不超限（空 filter：粗筛=精筛，全部命中）
    for wx in ["wx_1", "wx_2", "wx_3"] {
        app.state.db.contacts().insert_one(make_contact(&ws, &acc, wx), None).await.expect("seed contact");
    }
    let resp = preview_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("正好等于上限应成功");
    assert_eq!(resp.0["targetCount"].as_i64(), Some(3), "targetCount 应为 3");
}

/// KC-06：dispatch 成功后回刷 lastDispatchTargetCount == 本次命中人数，与 dispatchedCount
/// （去重后新入队数）区分，消 targetCount 三义误导。
#[tokio::test]
#[ignore]
async fn dispatch_backfills_last_dispatch_target_count() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_x"), None).await.expect("seed wx_x");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_y"), None).await.expect("seed wx_y");

    let _ = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("dispatch 应成功");

    // 类型化读回 Campaign，断言回刷字段。
    let reloaded = app
        .state
        .db
        .campaigns()
        .find_one(doc! { "_id": cid }, None)
        .await
        .expect("query campaign")
        .expect("campaign exists");
    assert_eq!(
        reloaded.last_dispatch_target_count,
        Some(2),
        "命中 2 人应回刷 lastDispatchTargetCount=2"
    );
    assert_eq!(reloaded.dispatched_count, 2, "首次全新命中 dispatchedCount=2");
}

/// KC-06 哨兵加强：**去重时 lastDispatchTargetCount 与 dispatchedCount 分叉**——
/// 锁死 `lastDispatchTargetCount = hits.len()`（本次粗筛命中总数）语义，而非
/// `= dispatched`（去重后新入队数）。上面的 `dispatch_backfills_last_dispatch_target_count`
/// 只覆盖"无去重"（两值巧合都 = 2），无法区分正确实现与写成 `= dispatched` 的错误实现。
///
/// 场景 = 重入恢复（campaign 状态 dispatching，KC-02 放行）：wx_p 上一轮已建 send 台账
/// （预置一条既存 campaign_send 占住去重位），wx_q 尚未推。本轮 dispatch：
/// - wx_p：insert campaign_send 撞 (campaignId, contactWxid) 唯一索引 DuplicateKey → 跳过，
///   dispatched 不自增（campaigns.rs:439）。
/// - wx_q：全新 → dispatched += 1（campaigns.rs:437）。
/// 于是 `dispatchedCount == 1`（去重后新入队）**但** `lastDispatchTargetCount == 2`
/// （粗筛仍命中 2 人，campaigns.rs:452 写 `hits.len()`）——二者分叉。若有人把回刷改写成
/// `lastDispatchTargetCount = dispatched`，本测试立刻绿变红（真哨兵）。
#[tokio::test]
#[ignore]
async fn dispatch_last_dispatch_target_count_diverges_on_dedup() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    // 重入恢复：dispatching 态放行（KC-02 dispatch_allowed_from_status，campaigns.rs:323）。
    let mut campaign = make_campaign(&ws, &acc);
    campaign.status = "dispatching".to_string();
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    // 命中 2 人（空 filter：粗筛=精筛，workspace+account 内全部 managed contact）。
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_p"), None).await.expect("seed wx_p");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_q"), None).await.expect("seed wx_q");

    // 预置一条既存 campaign_send（campaignId=本活动, contactWxid=wx_p）占住去重位——
    // 模拟 wx_p 上一轮已推过。CampaignSend serde rename_all=camelCase（models.rs:622），
    // campaign_id→campaignId / contact_wxid→contactWxid，与唯一索引键
    // (campaignId, contactWxid)（indexes.rs:784）一致，故本轮对 wx_p 的 insert 必撞 DuplicateKey。
    let existing_send = CampaignSend {
        id: None,
        workspace_id: ws.clone(),
        account_id: acc.clone(),
        campaign_id: cid,
        contact_wxid: "wx_p".to_string(),
        task_id: None,
        status: "enqueued".to_string(),
        created_at: DateTime::now(),
    };
    app.state
        .db
        .campaign_sends()
        .insert_one(&existing_send, None)
        .await
        .expect("seed 既存 campaign_send（占 wx_p 去重位）");

    let resp = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("dispatch 应成功");
    // 响应层：wx_q 新入队、wx_p 撞去重跳过 → dispatchedCount=1。
    assert_eq!(
        resp.0["dispatchedCount"].as_i64(),
        Some(1),
        "wx_p 撞去重跳过、仅 wx_q 新入队 → dispatchedCount=1,实际 {}",
        resp.0["dispatchedCount"]
    );

    // 类型化读回 Campaign，断言两字段分叉（2 ≠ 1）——锁死 hits.len() 语义。
    let reloaded = app
        .state
        .db
        .campaigns()
        .find_one(doc! { "_id": cid }, None)
        .await
        .expect("query campaign")
        .expect("campaign exists");
    assert_eq!(
        reloaded.dispatched_count, 1,
        "去重后新入队数 dispatchedCount=1（wx_p 跳过）"
    );
    assert_eq!(
        reloaded.last_dispatch_target_count,
        Some(2),
        "粗筛命中总数 lastDispatchTargetCount=2（含被去重的 wx_p）——若写成 =dispatched(1) 即绿变红"
    );
}

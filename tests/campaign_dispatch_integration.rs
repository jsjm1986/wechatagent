//! campaign 营销活动派发红线集成测试:命中0人拒绝 / 跨workspace NotFound / dispatch建task /
//! 二次dispatch去重(dispatchedCount=0)。全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test campaign_dispatch_integration -- --ignored`。
//!
//! ## 形态:直调 dispatch_campaign handler 真函数,验真实 DB 副作用(建 follow_up task / 去重)。
//! 空 SegmentFilter 命中 workspace+account 内所有 contact(粗筛只按 workspace_id+account_id)。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use axum::Json;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{AgentStatus, Campaign, Contact, SegmentFilter, WechatAccount};
use wechatagent::routes::campaigns::{
    campaign_spec_hash_for_view, create_campaign, dispatch_campaign, preview_campaign,
    reconcile_campaign_dispatches, update_campaign_draft, CreateCampaignRequest,
    DispatchCampaignRequest, UpdateCampaignDraftRequest,
};

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "camp_admin".to_string(),
        username: "camp_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

fn make_account(workspace_id: &str, account_id: &str) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        alias: account_id.to_string(),
        display_name: account_id.to_string(),
        app_id: None,
        wxid: None,
        nick_name: None,
        avatar_url: None,
        mcp_base_url: None,
        mcp_api_key: None,
        webhook_secret: None,
        online: true,
        status: None,
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: Vec::new(),
        created_at: now,
        updated_at: now,
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
        spec_version: 1,
        spec_hash: None,
        status: "draft".to_string(),
        target_count: None,
        dispatched_count: 0,
        last_dispatch_target_count: None,
        dispatch_generation: 0,
        dispatch_spec_hash: None,
        dispatch_audience: Vec::new(),
        dispatch_intent_text: None,
        dispatch_started_at: None,
        dispatch_completed_at: None,
        created_by: "camp_admin".to_string(),
        created_at: now,
        updated_at: now,
    }
}

/// Build the dispatch request from the production Campaign hash implementation.
/// Integration tests deliberately do not duplicate the canonicalization/hash algorithm.
async fn dispatch_current_spec(
    app: &TestApp,
    admin_workspace: &str,
    campaign_id: ObjectId,
) -> wechatagent::error::AppResult<Json<serde_json::Value>> {
    let campaign = app
        .state
        .db
        .campaigns()
        .find_one(doc! { "_id": campaign_id }, None)
        .await?
        .expect("campaign exists before dispatch");
    let spec_hash = campaign_spec_hash_for_view(&campaign)?;
    dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(admin_workspace)),
        Path(campaign_id.to_hex()),
        Json(DispatchCampaignRequest {
            spec_hash,
            spec_version: campaign.spec_version,
        }),
    )
    .await
}

/// Campaign identity is `(workspace_id, account_id)`. An account with the same
/// public id in another workspace must not authorize a draft in this workspace.
#[tokio::test]
#[ignore]
async fn create_rejects_account_owned_by_another_workspace_without_writing() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = "shared_campaign_account";
    app.state
        .db
        .accounts()
        .insert_one(make_account("other_workspace", account_id), None)
        .await
        .expect("seed same account id in another workspace");
    let before = app
        .state
        .db
        .campaigns()
        .count_documents(doc! {}, None)
        .await
        .expect("count campaigns before");

    let result = create_campaign(
        State(app.state.clone()),
        Extension(test_admin(&workspace_id)),
        Json(CreateCampaignRequest {
            title: "tenant scope guard".to_string(),
            intent_text: "must not persist".to_string(),
            segment_filter: SegmentFilter::default(),
            account_id: Some(account_id.to_string()),
        }),
    )
    .await;

    let rejected_wrong_workspace = matches!(result, Err(wechatagent::error::AppError::NotFound(_)));
    let after = app
        .state
        .db
        .campaigns()
        .count_documents(doc! {}, None)
        .await
        .expect("count campaigns after");
    let zero_writes = after == before;
    app.cleanup().await;

    assert!(
        rejected_wrong_workspace,
        "an account owned only by another workspace must be rejected"
    );
    assert!(
        zero_writes,
        "invalid tenant account must produce zero writes: before={before}, after={after}"
    );
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
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .expect("seed campaign");
    // 不 seed 任何 contact → 命中 0 人

    let result = dispatch_current_spec(&app, &ws, cid).await;
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
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .expect("seed campaign");

    // 用 other_workspace 视角 dispatch → filter {_id, workspaceId:other} 不匹配 → NotFound
    let result = dispatch_current_spec(&app, "other_workspace", cid).await;
    assert!(result.is_err(), "跨 workspace dispatch 必须 NotFound");
}

/// SR-075: preview rejects terminal campaigns and leaves the campaign document byte-for-byte
/// unchanged. A later contact therefore cannot reopen or expand a completed campaign.
#[tokio::test]
#[ignore]
async fn preview_completed_campaign_is_zero_write() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let mut campaign = make_campaign(&ws, &acc);
    campaign.status = "completed".to_string();
    let cid = campaign.id.unwrap();
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .unwrap();
    app.state
        .db
        .contacts()
        .insert_one(make_contact(&ws, &acc, "wx_late"), None)
        .await
        .unwrap();
    let raw = app.state.db.raw().collection::<Document>("campaigns");
    let before = raw
        .find_one(doc! { "_id": cid }, None)
        .await
        .unwrap()
        .unwrap();

    let result = preview_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(matches!(
        result,
        Err(wechatagent::error::AppError::Conflict(_))
    ));
    let after = raw
        .find_one(doc! { "_id": cid }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after, before,
        "terminal preview must have zero campaign writes"
    );
}

/// SR-077: saving a changed draft spec with CAS must alter the persisted spec and the next
/// pure preview must evaluate that new spec, not the first-created one.
#[tokio::test]
#[ignore]
async fn draft_patch_is_consumed_by_next_preview() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .unwrap();

    let mut won = make_contact(&ws, &acc, "wx_won");
    won.domain_attributes = Some(doc! { "customer_stage": "won" });
    let mut lead = make_contact(&ws, &acc, "wx_lead");
    lead.domain_attributes = Some(doc! { "customer_stage": "lead" });
    app.state.db.contacts().insert_one(won, None).await.unwrap();
    app.state
        .db
        .contacts()
        .insert_one(lead, None)
        .await
        .unwrap();

    let updated = update_campaign_draft(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
        Json(UpdateCampaignDraftRequest {
            title: "new title".into(),
            intent_text: "new intent".into(),
            segment_filter: SegmentFilter {
                customer_stage: Some("won".into()),
                ..Default::default()
            },
            expected_spec_version: 1,
        }),
    )
    .await
    .expect("draft patch succeeds");
    assert_eq!(updated.0["specVersion"].as_i64(), Some(2));

    let preview = preview_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("preview updated spec");
    assert_eq!(preview.0["targetCount"].as_i64(), Some(1));
    assert_eq!(preview.0["intentText"].as_str(), Some("new intent"));
    assert_eq!(preview.0["specVersion"].as_i64(), Some(2));
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
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .expect("seed campaign");
    // seed 2 个 managed contact → 命中 2 人
    app.state
        .db
        .contacts()
        .insert_one(make_contact(&ws, &acc, "wx_a"), None)
        .await
        .expect("seed wx_a");
    app.state
        .db
        .contacts()
        .insert_one(make_contact(&ws, &acc, "wx_b"), None)
        .await
        .expect("seed wx_b");

    let tasks_before = app
        .state
        .db
        .tasks()
        .count_documents(doc! {}, None)
        .await
        .expect("count tasks before");

    // 首次 dispatch
    let resp1 = dispatch_current_spec(&app, &ws, cid)
        .await
        .expect("首次 dispatch 应成功");
    assert_eq!(
        resp1.0["dispatchedCount"].as_i64(),
        Some(2),
        "命中 2 人首次 dispatch 应派 2 条,实际 {}",
        resp1.0["dispatchedCount"]
    );
    let tasks_after_1 = app
        .state
        .db
        .tasks()
        .count_documents(doc! {}, None)
        .await
        .expect("count after 1");
    assert_eq!(
        tasks_after_1 - tasks_before,
        2,
        "命中 2 人应建 2 条 follow_up task(走 gateway 证据)"
    );

    // 首次成功后 campaign 已置 completed → 二次 dispatch 被 KC-02 status 门以 BadRequest 拒。
    // (KC-02 前旧契约靠 unique 索引幂等返 dispatchedCount=0,现由门直接拒绝取代。)
    let result2 = dispatch_current_spec(&app, &ws, cid).await;
    assert!(
        matches!(result2, Err(wechatagent::error::AppError::BadRequest(_))),
        "首次成功后 campaign=completed,二次 dispatch 应被 status 门以 BadRequest 拒,实际 {:?}",
        result2.map(|r| r.0.clone())
    );
    // 门在圈人前拦截 → 二次不新增任何 task(比旧的 unique 去重更早、更强的防重推)。
    let tasks_after_2 = app
        .state
        .db
        .tasks()
        .count_documents(doc! {}, None)
        .await
        .expect("count after 2");
    assert_eq!(
        tasks_after_2, tasks_after_1,
        "二次 dispatch 被门拒后不应新增 task"
    );
}

/// HC-021 durable fanout：task insert 被 validator 拒后保留 prepared send intent；
/// 解除故障后 reconciler 用同一冻结身份恢复确定性 task，并提交 campaign completed。
#[tokio::test]
#[ignore]
async fn dispatch_task_insert_failure_is_reconciled_from_prepared_intent() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .expect("seed campaign");
    app.state
        .db
        .contacts()
        .insert_one(make_contact(&ws, &acc, "wx_rollback"), None)
        .await
        .expect("seed contact");

    // 装 validator：让 agent_tasks 的 insert 确定性失败（拒绝所有 kind=follow_up 的插入）。
    let _ = app
        .state
        .db
        .raw()
        .create_collection("agent_tasks", None)
        .await;
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

    let result = dispatch_current_spec(&app, &ws, cid).await;
    assert!(result.is_err(), "task insert 失败应中断并返 Err");

    let prepared = app
        .state
        .db
        .campaign_sends()
        .find_one(doc! { "campaignId": cid }, None)
        .await
        .expect("read prepared send")
        .expect("prepared send must remain durable");
    assert_eq!(prepared.status, "prepared");
    assert!(prepared.task_id.is_some());
    let frozen = app
        .state
        .db
        .campaigns()
        .find_one(doc! { "_id": cid }, None)
        .await
        .expect("read frozen campaign")
        .expect("frozen campaign");
    assert_eq!(frozen.status, "dispatching");
    assert_eq!(frozen.dispatch_audience, vec!["wx_rollback"]);

    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "agent_tasks",
                "validator": {},
                "validationLevel": "off",
            },
            None,
        )
        .await
        .expect("remove agent_tasks validator");
    assert_eq!(reconcile_campaign_dispatches(&app.state).await.unwrap(), 1);

    let recovered = app
        .state
        .db
        .campaigns()
        .find_one(doc! { "_id": cid }, None)
        .await
        .expect("read recovered campaign")
        .expect("recovered campaign");
    assert_eq!(recovered.status, "completed");
    assert_eq!(recovered.dispatched_count, 1);
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(doc! { "_id": prepared.task_id.unwrap() }, None)
            .await
            .unwrap(),
        1
    );
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
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .expect("seed campaign");
    app.state
        .db
        .contacts()
        .insert_one(make_contact(&ws, &acc, "wx_done"), None)
        .await
        .expect("seed contact");

    let result = dispatch_current_spec(&app, &ws, cid).await;
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
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .expect("seed campaign");
    // seed 4 个 managed contact（> 上限 3）→ 粗筛候选超限
    for wx in ["wx_1", "wx_2", "wx_3", "wx_4"] {
        app.state
            .db
            .contacts()
            .insert_one(make_contact(&ws, &acc, wx), None)
            .await
            .expect("seed contact");
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
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .expect("seed campaign");
    // seed 正好 3 个 → 不超限（空 filter：粗筛=精筛，全部命中）
    for wx in ["wx_1", "wx_2", "wx_3"] {
        app.state
            .db
            .contacts()
            .insert_one(make_contact(&ws, &acc, wx), None)
            .await
            .expect("seed contact");
    }
    let resp = preview_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("正好等于上限应成功");
    assert_eq!(
        resp.0["targetCount"].as_i64(),
        Some(3),
        "targetCount 应为 3"
    );
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
    app.state
        .db
        .campaigns()
        .insert_one(&campaign, None)
        .await
        .expect("seed campaign");
    app.state
        .db
        .contacts()
        .insert_one(make_contact(&ws, &acc, "wx_x"), None)
        .await
        .expect("seed wx_x");
    app.state
        .db
        .contacts()
        .insert_one(make_contact(&ws, &acc, "wx_y"), None)
        .await
        .expect("seed wx_y");

    let _ = dispatch_current_spec(&app, &ws, cid)
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
    assert_eq!(
        reloaded.dispatched_count, 2,
        "首次全新命中 dispatchedCount=2"
    );
}

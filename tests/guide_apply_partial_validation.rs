//! guide apply 部分应用红线集成测试:LLM 产的越界枚举字段被跳过+记 skipped,
//! 合法字段照落;手动表单/审批路径的 AdminWrite 硬拒不在本文件范围。
//! 全部 `#[ignore]` 需 Docker。CI:`cargo test --test guide_apply_partial_validation -- --ignored`。
//!
//! ## 红线意义:apply_contact_changes 不能因单个 LLM 越界字段(operationState="active")
//! 整请求 400 把合法字段(humanProfileNote/customerStage/...)全陪葬。
#![cfg(test)]

mod common;

use mongodb::bson::{doc, DateTime, Document};
use wechatagent::models::{AgentStatus, Contact};
use wechatagent::routes::apply_contact_changes;

use crate::common::TestApp;

/// 构造一个 managed contact,operation_state 初始为 DEFAULT 初始态 new_contact。
fn seed_contact(ws: &str, acc: &str, wxid: &str) -> Contact {
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
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
        operation_state: Some("new_contact".to_string()),
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

/// seed + 取回带 _id 的 contact(apply_contact_changes 需要 contact.id)。
async fn insert_and_load(app: &TestApp, c: Contact, wxid: &str) -> Contact {
    app.state.db.contacts().insert_one(c, None).await.expect("seed contact");
    app.state
        .db
        .contacts()
        .find_one(doc! { "wxid": wxid }, None)
        .await
        .expect("query")
        .expect("contact exists")
}

/// 越界字段跳过,合法字段照落。
#[tokio::test]
#[ignore]
async fn apply_skips_invalid_keeps_valid() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = insert_and_load(&app, seed_contact(&ws, &acc, "wx_skip1"), "wx_skip1").await;

    // humanProfileNote 合法;operationState="active" 状态机无此态;customerStage="瞎填" 字典越界。
    let changes = doc! {
        "humanProfileNote": "关注价格",
        "operationState": "active",
        "customerStage": "瞎填一个不存在的阶段",
    };
    let skipped = apply_contact_changes(&app.state, &contact, &changes)
        .await
        .expect("apply 不应整体失败");

    // 合法字段真落库。
    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_skip1" }, None)
        .await
        .expect("query")
        .expect("exists");
    assert_eq!(
        after.human_profile_note.as_deref(),
        Some("关注价格"),
        "合法字段 humanProfileNote 必须落库"
    );
    // operationState 越界被跳过 → 保持初始 new_contact 不变。
    assert_eq!(
        after.operation_state.as_deref(),
        Some("new_contact"),
        "越界 operationState 不应写入,保持原态"
    );
    // 两个越界字段都进 skipped。
    let fields: Vec<&str> = skipped.iter().map(|s| s.field.as_str()).collect();
    assert!(fields.contains(&"operationState"), "operationState 应在 skipped: {fields:?}");
    assert!(fields.contains(&"customerStage"), "customerStage 应在 skipped: {fields:?}");
    assert_eq!(skipped.len(), 2, "恰两个越界字段被跳过");
}

/// 三枚举字段全越界、无其它合法字段 → set_doc 空判生效,contact 完全不变。
#[tokio::test]
#[ignore]
async fn apply_all_invalid_no_empty_write() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = insert_and_load(&app, seed_contact(&ws, &acc, "wx_allbad"), "wx_allbad").await;
    let before_updated = contact.updated_at;

    let changes = doc! {
        "operationState": "active",
        "customerStage": "瞎填阶段",
        "intentLevel": "瞎填意向",
    };
    let skipped = apply_contact_changes(&app.state, &contact, &changes)
        .await
        .expect("apply 不应整体失败");

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_allbad" }, None)
        .await
        .expect("query")
        .expect("exists");
    // set_doc 空判:无合法字段 → 不写库 → updated_at 不变。
    assert_eq!(
        after.updated_at, before_updated,
        "全越界应触发 set_doc 空判,不产生只刷 updated_at 的空写"
    );
    assert_eq!(skipped.len(), 3, "三个越界字段全部记入 skipped");
}

/// customerStage 越界 + intentLevel 合法 → intent 落库,stage 进 skipped。
#[tokio::test]
#[ignore]
async fn apply_intent_valid_stage_skipped() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = insert_and_load(&app, seed_contact(&ws, &acc, "wx_mix"), "wx_mix").await;

    // customerStage 越界(进 skipped),intentLevel="high" 是合法 canonical。
    let changes = doc! {
        "customerStage": "瞎填阶段",
        "intentLevel": "high",
    };
    let skipped = apply_contact_changes(&app.state, &contact, &changes)
        .await
        .expect("apply 不应整体失败");

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_mix" }, None)
        .await
        .expect("query")
        .expect("exists");
    // intent_level 存在 domain_attributes.intent_level。
    let intent = after
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str("intent_level").ok());
    assert_eq!(intent, Some("high"), "合法 intentLevel 必须落库");
    let fields: Vec<&str> = skipped.iter().map(|s| s.field.as_str()).collect();
    assert_eq!(fields, vec!["customerStage"], "仅 customerStage 被跳过");
}

/// 正向回归:三字段全合法 → 全部落库,skipped 为空。
#[tokio::test]
#[ignore]
async fn apply_legal_values_all_persist() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = insert_and_load(&app, seed_contact(&ws, &acc, "wx_ok"), "wx_ok").await;

    // operationState="cooldown" allowFromAny:true → 从任意态合法;stage/intent 用 canonical。
    let changes = doc! {
        "customerStage": "need_discovery",
        "intentLevel": "high",
        "operationState": "cooldown",
    };
    let skipped = apply_contact_changes(&app.state, &contact, &changes)
        .await
        .expect("apply 成功");

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_ok" }, None)
        .await
        .expect("query")
        .expect("exists");
    assert_eq!(after.operation_state.as_deref(), Some("cooldown"), "合法迁移落库");
    let stage = after.domain_attributes.as_ref().and_then(|d| d.get_str("customer_stage").ok());
    let intent = after.domain_attributes.as_ref().and_then(|d| d.get_str("intent_level").ok());
    assert_eq!(stage, Some("need_discovery"), "合法 stage 落库");
    assert_eq!(intent, Some("high"), "合法 intent 落库");
    assert!(skipped.is_empty(), "全合法 → skipped 空(证明不影响 happy path)");
}

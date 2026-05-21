//! agent-self-evolution M4 W4 Task 5.9：release → rollback 闭环集成测试。
//!
//! Requirements 6.6：admin rollback 后 `resolve_thresholds` 必须立即读回 baseline，
//! 当下还在跑的 run 不受影响（`resolve_thresholds` 在每次 run 入口读一次）。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）。
//!
//! 路径：
//! 1. 预 seed 一条 `eligible_for_release` 的 threshold proposal（gate=fact_risk_block,
//!    current=6, proposed=7）；
//! 2. 调 `evolution::release::release_threshold` → 校验 `resolve_thresholds` 读到 7；
//! 3. 调 `evolution::release::rollback_threshold` → 校验 `resolve_thresholds`
//!    读回 baseline（6，来自 contact 维度 runtime / AppConfig）。
//!
//! prompt 路径同步覆盖：seed 一条 `current_version=true` 的 prompt template，
//! 一条 `eligible_for_release` 的 prompt proposal，release 后 `prompt_pack_version`
//! +1 + `current_version=true` 切到新 version；rollback 后切回 old version
//! 并再 +1。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use std::sync::atomic::Ordering;
use wechatagent::agent::resolve_thresholds;
use wechatagent::evolution::release::{
    release_prompt, release_threshold, rollback_prompt, rollback_threshold,
};
use wechatagent::models::{AgentStatus, Contact, Proposal};

fn make_contact(wxid: &str) -> Contact {
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

fn make_eligible_threshold_proposal(gate_key: &str, current: f64, proposed: f64) -> Proposal {
    Proposal {
        id: Some(ObjectId::new()),
        experiment_id: "exp_rollback_1".to_string(),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        proposal_kind: "threshold".to_string(),
        status: "eligible_for_release".to_string(),
        gate_key: Some(gate_key.to_string()),
        current_value: Some(current),
        proposed_value: Some(proposed),
        cohort_notes: Document::new(),
        proposed_template_key: None,
        proposed_section: None,
        diff_summary: None,
        diff_snippet: None,
        critic_reasoning: None,
        expected_improvement_on: Vec::new(),
        risk_note: None,
        previous_prompt_version: None,
        eval_metrics: Document::new(),
        eval_replays_completed: 30,
        eval_replays_failed: 0,
        significance_passed: Some(true),
        failure_reason: None,
        released_at: None,
        released_by: None,
        rolled_back_at: None,
        rolled_back_by: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

fn make_eligible_prompt_proposal(template_key: &str, new_content: &str) -> Proposal {
    Proposal {
        id: Some(ObjectId::new()),
        experiment_id: "exp_rollback_prompt_1".to_string(),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        proposal_kind: "prompt".to_string(),
        status: "eligible_for_release".to_string(),
        gate_key: None,
        current_value: None,
        proposed_value: None,
        cohort_notes: Document::new(),
        proposed_template_key: Some(template_key.to_string()),
        proposed_section: Some("fact_risk_block".to_string()),
        diff_summary: Some("test rewrite".to_string()),
        diff_snippet: Some(new_content.to_string()),
        critic_reasoning: Some("make claims more verifiable".to_string()),
        expected_improvement_on: vec!["fact_risk_block".to_string()],
        risk_note: None,
        previous_prompt_version: None,
        eval_metrics: Document::new(),
        eval_replays_completed: 30,
        eval_replays_failed: 0,
        significance_passed: Some(true),
        failure_reason: None,
        released_at: None,
        released_by: None,
        rolled_back_at: None,
        rolled_back_by: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

#[tokio::test]
#[ignore]
async fn threshold_release_then_rollback_round_trips_resolve_thresholds() {
    let app = common::TestApp::start().await;
    let contact = make_contact("user_rollback_1");

    // baseline = AppConfig 默认值（fact_risk_block_at 默认 6）。
    let before = resolve_thresholds(&app.state, &contact)
        .await
        .expect("resolve baseline");
    let baseline_value = before.fact_risk_block;

    // 预置 eligible_for_release 的 threshold proposal。
    let proposal = make_eligible_threshold_proposal(
        "fact_risk_block",
        baseline_value as f64,
        baseline_value as f64 + 1.0,
    );
    let proposal_id = proposal.id.expect("proposal id");
    app.state
        .db
        .proposals()
        .insert_one(&proposal, None)
        .await
        .expect("seed proposal");

    // release → fact_risk_block 应当变为 baseline + 1。
    release_threshold(&app.state, proposal_id, "admin_test")
        .await
        .expect("release_threshold");
    let after_release = resolve_thresholds(&app.state, &contact)
        .await
        .expect("resolve after release");
    assert_eq!(
        after_release.fact_risk_block,
        baseline_value + 1,
        "after release, fact_risk_block must read overridden value"
    );

    // rollback → 读回 baseline。
    rollback_threshold(&app.state, proposal_id, "admin_test")
        .await
        .expect("rollback_threshold");
    let after_rollback = resolve_thresholds(&app.state, &contact)
        .await
        .expect("resolve after rollback");
    assert_eq!(
        after_rollback.fact_risk_block, baseline_value,
        "after rollback, fact_risk_block must read baseline again"
    );

    // proposal.status 终态应当是 rolled_back。
    let final_proposal = app
        .state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .expect("reload proposal")
        .expect("proposal still exists");
    assert_eq!(final_proposal.status, "rolled_back");
}

#[tokio::test]
#[ignore]
async fn prompt_release_then_rollback_round_trips_current_version_and_pack_version() {
    let app = common::TestApp::start().await;

    // ensure_prompt_pack_v2 已在 TestApp::start 内 seed，挑选其中一条 key 做演化。
    let prompt_key = "user.reply.system";
    let baseline = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": "default",
                "prompt_key": prompt_key,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("find baseline prompt template")
        .expect("baseline prompt template must exist after seed");
    let baseline_version = baseline.version;
    let baseline_content = baseline.content.clone();

    let pack_version_before = app.state.prompt_pack_version.load(Ordering::SeqCst);

    // 预置 eligible_for_release 的 prompt proposal。
    let proposal = make_eligible_prompt_proposal(prompt_key, "PROPOSED-NEW-PROMPT-BODY-FROM-TEST");
    let proposal_id = proposal.id.expect("proposal id");
    app.state
        .db
        .proposals()
        .insert_one(&proposal, None)
        .await
        .expect("seed prompt proposal");

    // release → 新 version 落库 + current_version 切换 + pack_version +1。
    release_prompt(&app.state, proposal_id, "admin_test")
        .await
        .expect("release_prompt");
    let pack_version_after_release = app.state.prompt_pack_version.load(Ordering::SeqCst);
    assert_eq!(
        pack_version_after_release,
        pack_version_before + 1,
        "release_prompt must bump prompt_pack_version exactly once"
    );

    let after_release = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": "default",
                "prompt_key": prompt_key,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("find current after release")
        .expect("current prompt template after release");
    assert_eq!(after_release.version, baseline_version + 1);
    assert_eq!(after_release.content, "PROPOSED-NEW-PROMPT-BODY-FROM-TEST");

    // rollback → current_version 切回旧 version + pack_version 再 +1。
    rollback_prompt(&app.state, proposal_id, "admin_test")
        .await
        .expect("rollback_prompt");
    let pack_version_after_rollback = app.state.prompt_pack_version.load(Ordering::SeqCst);
    assert_eq!(
        pack_version_after_rollback,
        pack_version_before + 2,
        "rollback_prompt must also bump prompt_pack_version"
    );

    let after_rollback = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": "default",
                "prompt_key": prompt_key,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("find current after rollback")
        .expect("current prompt template after rollback");
    assert_eq!(after_rollback.version, baseline_version);
    assert_eq!(after_rollback.content, baseline_content);

    // proposal 终态 = rolled_back。
    let final_proposal = app
        .state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .expect("reload proposal")
        .expect("proposal still exists");
    assert_eq!(final_proposal.status, "rolled_back");
}

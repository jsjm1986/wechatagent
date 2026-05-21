//! agent-self-evolution M4 W4 Task 5.9：threshold E2E 集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）。
//!
//! 路径（与 spec tasks.md 5.9 第一条对齐 —— "mock 5 闸命中率分布 → tick 产
//! ≥1 条 threshold proposal → shadow eval → admin release → resolve_thresholds
//! 读到新值"）：
//!
//! 1. 预 seed 30+ 条 `agent_run_logs`（lifecycle=completed，分散在多个
//!    contact 以满足 `evolution_cohort_per_contact_cap=3` 去重），其中
//!    `held_by_ai_policy` 命中率 > `fact_risk_block` 上限 0.15
//!    （阈值过松信号，按 `THRESHOLD_REASONABLE_BANDS` 应当生成 +step
//!    候选）。
//! 2. 调 `select_cohorts` → 校验 threshold cohort 至少 30 条。
//! 3. 调 `threshold::generate` → 校验 ≥1 条 fact_risk_block 候选 +
//!    `proposed_value > current_value`。
//! 4. 直接把候选 status 改成 `eligible_for_release`（W3 shadow eval 已被
//!    `evolution_isolation` 覆盖，本测试只验 release → resolve_thresholds
//!    闭环）。
//! 5. 调 `release_threshold` → 校验 `resolve_thresholds` 读到新阈值。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::agent::resolve_thresholds;
use wechatagent::evolution::{cohort::select_cohorts, release::release_threshold, threshold};
use wechatagent::models::{AgentStatus, Contact};

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

async fn insert_run_log(app: &common::TestApp, contact_wxid: &str, final_review_status: &str) {
    let raw = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_run_logs");
    raw.insert_one(
        doc! {
            "_id": ObjectId::new(),
            "workspace_id": "default",
            "account_id": "default",
            "contact_wxid": contact_wxid,
            "run_id": format!("run_{}", ObjectId::new().to_hex()),
            "trigger_kind": "inbound_message",
            "status": "completed",
            "lifecycle": "completed",
            "final_review_status": final_review_status,
            "revision_applied": false,
            "review": doc! {},
            "decision": doc! {},
            "knowledge_route": doc! {},
            "planner": doc! {},
            "context": doc! {},
            "created_at": DateTime::now(),
        },
        None,
    )
    .await
    .expect("seed run log");
}

#[tokio::test]
#[ignore]
async fn cohort_then_threshold_generate_then_release_round_trips() {
    let app = common::TestApp::start().await;
    let contact = make_contact("user_threshold_e2e");

    let baseline = resolve_thresholds(&app.state, &contact)
        .await
        .expect("resolve baseline");
    let baseline_fact_risk = baseline.fact_risk_block;

    // Seed 36 条 run logs，分散到 12 个 contact（per-contact cap=3）。
    // 命中分布：18 条 held_by_ai_policy（fact_risk_block 命中），18 条 approved。
    // → fact_risk_block hit_rate = 18/36 = 0.5，>0.15 上限 → 候选 +step。
    for i in 0..12 {
        let wxid = format!("user_cohort_{i:02}");
        for j in 0..3 {
            let status = if (i * 3 + j) % 2 == 0 {
                "held_by_ai_policy"
            } else {
                "approved"
            };
            insert_run_log(&app, &wxid, status).await;
        }
    }

    let cohorts = select_cohorts(&app.state, "default", "default")
        .await
        .expect("select cohorts");
    assert!(
        cohorts.threshold.len() >= 30,
        "expected threshold cohort >= 30 but got {}",
        cohorts.threshold.len()
    );

    // 直接驱动 threshold::generate（与 run_one_tick 走同一路径，但不进 W3 shadow eval
    // —— shadow eval 已被 evolution_isolation 覆盖到 100 次零副作用）。
    let exp_id = "exp_threshold_e2e_1";
    let proposals = threshold::generate(&app.state, exp_id, &cohorts.threshold)
        .await
        .expect("threshold::generate");

    let fact_risk_proposal = proposals
        .iter()
        .find(|p| p.gate_key.as_deref() == Some("fact_risk_block") && p.status == "pending_eval")
        .expect("expected at least one pending_eval fact_risk_block candidate");
    assert!(
        fact_risk_proposal.proposed_value.unwrap() > fact_risk_proposal.current_value.unwrap(),
        "hit-rate over upper band must propose +step (proposed > current)"
    );

    // 把 proposal 持久化到 mongo（run_one_tick 内 insert_proposals 的等价路径），
    // 再手工切到 eligible_for_release（W3 shadow eval 在本测试外覆盖）。
    let mut proposal_to_release = fact_risk_proposal.clone();
    proposal_to_release.id = Some(ObjectId::new());
    proposal_to_release.status = "eligible_for_release".to_string();
    proposal_to_release.significance_passed = Some(true);
    let pid = proposal_to_release.id.expect("proposal id");
    app.state
        .db
        .proposals()
        .insert_one(&proposal_to_release, None)
        .await
        .expect("seed proposal");

    // release → resolve_thresholds 读到新值（≥ baseline + 0.5，向下取整后至少 baseline + 0）。
    // 步长 0.5 → as i32 截断仍可能等于 baseline；用 baseline + 1 不一定成立，
    // 改为对 raw f64 → i32 之后的实际值断言。
    let proposed_int = proposal_to_release.proposed_value.unwrap() as i32;
    release_threshold(&app.state, pid, "admin_test")
        .await
        .expect("release_threshold");

    let after = resolve_thresholds(&app.state, &contact)
        .await
        .expect("resolve after release");
    assert_eq!(
        after.fact_risk_block, proposed_int,
        "after release, fact_risk_block must read proposed_value as i32 (proposed_raw={})",
        proposal_to_release.proposed_value.unwrap()
    );
    assert!(
        after.fact_risk_block >= baseline_fact_risk,
        "release must not lower fact_risk_block below baseline in this scenario"
    );
}

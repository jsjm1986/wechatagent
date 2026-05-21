//! agent-self-evolution M4 W4 Task 5.9：prompt E2E 集成测试。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）。
//!
//! 路径（与 spec tasks.md 5.9 第二条对齐 —— "failure cohort + mock LLM 返回
//! fixture diff → release → prompt_pack_version bump → 下次 generate_agent_json
//! 从 Mongo 重读"）：
//!
//! 1. 预 seed 30+ 条 failure 类 `agent_run_logs`（lifecycle=completed，
//!    final_review_status ∈ blocked_unverified_product_claim / held_by_ai_policy 等），
//!    分散在 ≥ 12 contact 满足 `evolution_cohort_per_contact_cap=3` 去重。
//! 2. 调 `select_cohorts` → 校验 prompt cohort ≥ 30。
//! 3. 直接预置一条 `eligible_for_release` 的 prompt proposal，diff_snippet =
//!    新的完整 content body（W4 release path 把整段 diff_snippet 当 new content）。
//!    本测试不驱动 prompt_critic LLM 路径（W3 W2 已有 unit test 覆盖；W4 端到端
//!    重点是 release → cache 失效 → 下次读取的闭环）。
//! 4. 调 `release_prompt` →
//!    - `prompt_pack_version` +1（让 LRU 立即失效）；
//!    - `(workspace, prompt_key, current_version=true)` 切到 new content；
//!    - proposal status="released"。
//! 5. 直接 `prompt_templates.find_one(current_version=true)` 模拟下次
//!    `generate_agent_json` 从 Mongo 重读：内容必须是 fixture 提供的新 body。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use std::sync::atomic::Ordering;
use wechatagent::evolution::{cohort::select_cohorts, release::release_prompt};
use wechatagent::models::Proposal;

const FAILURE_STATUSES: &[&str] = &[
    "blocked_unverified_product_claim",
    "held_by_ai_policy",
    "blocked_by_safety_guard",
    "ai_waiting_for_more_context",
    "budget_exceeded",
    "revision_failed",
];

async fn insert_failure_run_log(
    app: &common::TestApp,
    contact_wxid: &str,
    final_review_status: &str,
) {
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
    .expect("seed failure run log");
}

fn make_eligible_prompt_proposal(template_key: &str, new_content: &str) -> Proposal {
    Proposal {
        id: Some(ObjectId::new()),
        experiment_id: "exp_prompt_e2e_1".to_string(),
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
        diff_summary: Some("test rewrite for fact-claim verifiability".to_string()),
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
async fn failure_cohort_then_prompt_release_bumps_pack_version_and_swaps_current() {
    let app = common::TestApp::start().await;

    // 1. seed 36 条失败类 run logs，分散到 12 contact（cap=3），覆盖 6 种 failure status。
    for i in 0..12 {
        let wxid = format!("user_prompt_cohort_{i:02}");
        for j in 0..3 {
            let status = FAILURE_STATUSES[(i * 3 + j) % FAILURE_STATUSES.len()];
            insert_failure_run_log(&app, &wxid, status).await;
        }
    }

    // 2. 校验 prompt cohort 命中 ≥ 30（min_replays=30 是发起 LLM 的最低门槛）。
    let cohorts = select_cohorts(&app.state, "default", "default")
        .await
        .expect("select_cohorts");
    assert!(
        cohorts.prompt.len() >= 30,
        "expected prompt cohort >= 30 but got {}",
        cohorts.prompt.len()
    );

    // 3. 取一条 ensure_prompt_pack_v2 已 seed 的 prompt key 当作 release target。
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
        .expect("find baseline current prompt template")
        .expect("baseline current prompt template must exist after seed");
    let baseline_version = baseline.version;
    let baseline_content = baseline.content.clone();
    assert!(
        !baseline_content.is_empty(),
        "ensure_prompt_pack_v2 must seed a non-empty content for {prompt_key}"
    );

    let pack_version_before = app.state.prompt_pack_version.load(Ordering::SeqCst);

    // 4. 预置 eligible_for_release prompt proposal。
    let new_body = "PROPOSED-CONTENT-FROM-CRITIC-FIXTURE-V1";
    let proposal = make_eligible_prompt_proposal(prompt_key, new_body);
    let proposal_id = proposal.id.expect("proposal id");
    app.state
        .db
        .proposals()
        .insert_one(&proposal, None)
        .await
        .expect("seed eligible prompt proposal");

    // 5. release_prompt → pack_version +1, current_version 切换。
    release_prompt(&app.state, proposal_id, "admin_test")
        .await
        .expect("release_prompt");
    let pack_version_after = app.state.prompt_pack_version.load(Ordering::SeqCst);
    assert_eq!(
        pack_version_after,
        pack_version_before + 1,
        "release_prompt must bump prompt_pack_version exactly once for cache invalidation"
    );

    // 6. 模拟"下次 generate_agent_json 从 Mongo 重读 current_version=true 那条"。
    let after = app
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
        .expect("re-read current template")
        .expect("current template still exists after release");
    assert_eq!(
        after.version,
        baseline_version + 1,
        "release_prompt must bump version by 1"
    );
    assert_eq!(
        after.content, new_body,
        "release_prompt must persist diff_snippet as the new content body"
    );
    assert_eq!(
        after.seeded_by.as_deref(),
        Some("evolution_release"),
        "new template must record seeded_by=evolution_release for audit"
    );

    // 7. 同 (workspace, prompt_key) 下应当只剩一条 current_version=true。
    let current_count = app
        .state
        .db
        .prompt_templates()
        .count_documents(
            doc! {
                "workspace_id": "default",
                "prompt_key": prompt_key,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("count current_version=true rows");
    assert_eq!(
        current_count, 1,
        "release_prompt must keep exactly one current_version=true per (workspace, prompt_key)"
    );

    // 8. proposal.status 终态 = released。
    let final_proposal = app
        .state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .expect("reload proposal")
        .expect("proposal still exists");
    assert_eq!(final_proposal.status, "released");
    assert_eq!(
        final_proposal.previous_prompt_version.as_deref(),
        Some(baseline_version.to_string().as_str())
    );
}

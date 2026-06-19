//! roleplay-fuzz P0 helper 自验证（P0 退出条件 2/3/4）。
//!
//! 关联：`docs/superpowers/specs/2026-06-15-roleplay-fuzz-testing-design.md` §12 P0。
//!
//! 多数 `#[ignore]`，需 Docker（testcontainers MongoDB）。这些测试**不调用任何
//! real-LLM**，只验证夹具本身：
//! - seed_active_domain_profile 能经 load_active_domain_profile 读回（退出条件 2）。
//! - override_review_prompt 能在 ensure_prompt_pack_v2 之后覆写并被 load_prompt
//!   读回（退出条件 3，验证时序坑已规避）。
//! - RoleplayLedger 能输出含 suspected_layer 的 JSON 行（退出条件 4，无需 Docker）。

mod common;

use common::roleplay_fixtures::{
    override_review_prompt, seed_emotional_companion_profile, seed_verified_chunk, RoleplayLedger,
    EMOTIONAL_COMPANION_WORKSPACE,
};
use common::TestApp;
use wechatagent::agent::load_active_domain_profile;
use wechatagent::prompts;

/// 退出条件 2：seed active DomainProfile → load_active_domain_profile 读回。
#[tokio::test]
#[ignore]
async fn p0_seed_active_profile_round_trips() {
    let app = TestApp::start().await;
    seed_emotional_companion_profile(&app).await;

    let profile = load_active_domain_profile(&app.state.db, EMOTIONAL_COMPANION_WORKSPACE).await;
    // 命中的是 seed 的情感陪伴 profile，不是 DEFAULT 回落。
    assert_eq!(profile.profile_id, "emotional_companion_minimal");
    assert!(profile.is_active);
    assert!(
        profile.conversation_modes.iter().any(|m| m == "intimate_companion"),
        "conversation_modes 应含 intimate_companion，实际：{:?}",
        profile.conversation_modes
    );
    assert!(
        profile.grounding_gate_bypass_without_claim,
        "情感陪伴应旁路 grounding 软闸"
    );
    assert!(
        !profile.operation_mode.funnel.enabled,
        "情感陪伴应关闭漏斗推进"
    );
}

/// 退出条件 2 补充：默认 workspace 仍回落 DEFAULT（隔离性——seed 不污染别的 ws）。
#[tokio::test]
#[ignore]
async fn p0_default_workspace_still_falls_back() {
    let app = TestApp::start().await;
    seed_emotional_companion_profile(&app).await;

    let default_ws = app.state.config.default_workspace_id.clone();
    let profile = load_active_domain_profile(&app.state.db, &default_ws).await;
    // 默认 ws 没 seed 情感 profile → 回落 DEFAULT（profile_id 不是情感的）。
    assert_ne!(profile.profile_id, "emotional_companion_minimal");
}

/// 退出条件 3：override_review_prompt 在 ensure_prompt_pack_v2 之后覆写 → load_prompt 读回。
#[tokio::test]
#[ignore]
async fn p0_override_review_prompt_wins() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    const MARKER: &str = "ROLEPLAY_EMOTIONAL_RUBRIC_MARKER_v1";
    let body = format!("你是情感陪伴场景的评审员。主动关心不是 pressure。{MARKER}");
    // 必须在 TestApp::start() 之后（ensure_prompt_pack_v2 已跑完）。
    override_review_prompt(&app, &ws, "user.review.system", &body).await;

    let loaded = prompts::load_prompt(&app.state.db, &ws, "user.review.system")
        .await
        .expect("load user.review.system");
    assert!(
        loaded.contains(MARKER),
        "覆写后的 review prompt 未被 load_prompt 读回（时序坑?），实际开头：{}",
        loaded.chars().take(80).collect::<String>()
    );
}

/// 退出条件 4 之知识链路：verified chunk seed 能写入。
#[tokio::test]
#[ignore]
async fn p0_seed_verified_chunk_writes() {
    let app = TestApp::start().await;
    let id = seed_verified_chunk(
        &app,
        EMOTIONAL_COMPANION_WORKSPACE,
        "陪伴边界",
        "AI 不提供医疗/法律诊断",
        "出现自伤风险时，建议联系现实可信赖的人或当地紧急资源；AI 保持第一人称承接。",
    )
    .await;
    assert!(!id.is_empty());
}

/// 退出条件 4：RoleplayLedger 输出含 suspected_layer 的 JSON 行（无需 Docker）。
#[test]
fn p0_ledger_writes_suspected_layer() {
    let dir = std::env::temp_dir().join(format!("roleplay_ledger_test_{}", std::process::id()));
    std::env::set_var("REAL_LLM_LEDGER", &dir);
    let ledger = RoleplayLedger::for_fixture("emotional_companion_minimal");
    ledger.append_issue(
        "scene_night_low_mood",
        "reviewer",
        serde_json::json!({ "pressureRisk": 8, "reason": "主动关心被误判高压" }),
    );

    let path = dir.join("roleplay_emotional_companion_minimal.jsonl");
    let content = std::fs::read_to_string(&path).expect("ledger file written");
    let line = content.lines().next().expect("at least one line");
    let parsed: serde_json::Value = serde_json::from_str(line).expect("valid json line");
    assert_eq!(parsed["suspected_layer"], "reviewer");
    assert_eq!(parsed["scene_id"], "scene_night_low_mood");
    assert_eq!(parsed["kind"], "issue");

    std::fs::remove_dir_all(&dir).ok();
}

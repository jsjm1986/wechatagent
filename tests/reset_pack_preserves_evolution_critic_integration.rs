//! M12 回归：reset_prompt_pack_v2 不得孤儿化演化器 Critic prompt。
//!
//! reset 无条件 delete_many 本 workspace 全部 prompt_templates（含 evolution_critic_v1），
//! 业务 pack 由 prompt_specs() 重种,但 evolution_critic_v1 属独立 evolution pack、平时只在
//! 启动时种（main.rs）。修复前 reset 后该 key 消失、load_prompt→default_prompt_content 也
//! 不含它 → 演化 Critic 循环持续报错到进程重启。修复=reset 末尾补调
//! ensure_evolution_prompt_pack_v1 重种回来。
//!
//! `#[ignore]` 需 Docker;CI:`cargo test --test reset_pack_preserves_evolution_critic_integration -- --ignored`。
#![cfg(test)]

mod common;

use mongodb::bson::doc;
use wechatagent::prompts;

use crate::common::TestApp;

const CRITIC_KEY: &str = "evolution_critic_v1";

async fn critic_current(app: &TestApp, workspace: &str) -> Option<wechatagent::models::PromptTemplate> {
    app.state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": workspace,
                "prompt_key": CRITIC_KEY,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("query critic prompt")
}

/// M12 核心红线:reset_prompt_pack_v2 之后 evolution_critic_v1 仍存在
/// （current_version=true、status=active）——不再被 reset 孤儿化。
#[tokio::test]
#[ignore]
async fn reset_pack_preserves_evolution_critic_prompt() {
    let app = TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    // 前置:确保 evolution critic pack 已种（TestApp::start 走启动路径已种,幂等再确认一次）。
    prompts::ensure_evolution_prompt_pack_v1(&app.state.db, &workspace)
        .await
        .expect("seed evolution pack");
    let before = critic_current(&app, &workspace).await;
    assert!(
        before.is_some(),
        "前置:reset 前 evolution_critic_v1 应存在"
    );

    // act:显式销毁性 reseed。
    prompts::reset_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("reset_prompt_pack_v2");

    // assert:critic prompt 仍在（修复前会被 delete_many 删且不重种→None）。
    let after = critic_current(&app, &workspace).await.expect(
        "reset 后 evolution_critic_v1 必须仍存在(修复补种);缺失=M12 回归",
    );
    assert_eq!(after.prompt_key, CRITIC_KEY);
    assert_eq!(after.status, "active", "critic prompt 应为 active");
    assert!(after.current_version, "critic prompt 应为 current_version");

    // 消费方口径:load_prompt 能取到 critic system(不再 NotFound→演化循环不再断)。
    let loaded = prompts::load_prompt(&app.state.db, &workspace, CRITIC_KEY)
        .await
        .expect("reset 后 load_prompt(evolution_critic_v1) 不应 NotFound");
    assert!(
        !loaded.trim().is_empty(),
        "critic prompt 内容不应为空"
    );
}

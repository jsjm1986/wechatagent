//! 知识 worker execute_step 红线集成测试:占位桩的**行为契约**——
//! 合法 action 返 Ok + chunk_id 透传 targetChunkId;unsupported action 返 Err。
//! 全部 `#[ignore]`,需 Docker testcontainers(execute_step 取 &AppState,虽不读 DB)。
//! CI:`cargo test --test knowledge_worker_behavior_integration -- --ignored`。
//!
//! ## 红线意义(P0):execute_step 是 Phase 4 派工编排占位桩——6 个 action 全部只返回
//! StepOutcome,**绝不写 chunk verified / 不发 outbox**(mod.rs:430 契约 + `_state` 未使用)。
//! 真实 fix/add 仍走运营在 chat 内的 chat_apply(强制 draft+needs_review)。本测试钉死
//! "派工不直接落 verified" 的边界:一旦未来 execute_step 被改为真写 chunk 状态,
//! 其签名/行为会变,本契约测试是回归哨兵。
#![cfg(test)]

mod common;

use mongodb::bson::doc;

use wechatagent::knowledge_task::execute_step;

use crate::common::TestApp;

/// fix_chunk / retag 透传 targetChunkId;返回 Ok(不报错、不写 verified)。
#[tokio::test]
#[ignore]
async fn execute_step_fix_and_retag_passthrough_chunk_id() {
    let app = TestApp::start().await;
    let step = doc! { "targetChunkId": "chunk_abc" };

    let fix = execute_step(&app.state, "ws", "acc", "fix_chunk", &step)
        .await
        .expect("fix_chunk 应 Ok");
    assert_eq!(
        fix.chunk_id.as_deref(),
        Some("chunk_abc"),
        "fix_chunk 应透传 targetChunkId"
    );

    let retag = execute_step(&app.state, "ws", "acc", "retag", &step)
        .await
        .expect("retag 应 Ok");
    assert_eq!(
        retag.chunk_id.as_deref(),
        Some("chunk_abc"),
        "retag 应透传 targetChunkId"
    );
}

/// add_chunk / review_evolution / analyze_logs / dismiss 返 Ok 且 chunk_id=None(纯编排)。
#[tokio::test]
#[ignore]
async fn execute_step_orchestration_actions_ok_no_chunk() {
    let app = TestApp::start().await;
    let empty = doc! {};
    for action in ["add_chunk", "review_evolution", "analyze_logs", "dismiss"] {
        let out = execute_step(&app.state, "ws", "acc", action, &empty)
            .await
            .unwrap_or_else(|e| panic!("{action} 应 Ok,实际 {e:?}"));
        assert!(out.chunk_id.is_none(), "{action} 不应关联具体 chunk");
    }
}

/// 红线:未知 action 必须返 Err(不静默吞掉派工指令)。
#[tokio::test]
#[ignore]
async fn execute_step_unsupported_action_errors() {
    let app = TestApp::start().await;
    let result = execute_step(&app.state, "ws", "acc", "drop_table", &doc! {}).await;
    assert!(result.is_err(), "未知 action 必须 Err");
}

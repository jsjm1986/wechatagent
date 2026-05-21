//! agent-self-evolution M4：演化器模块。
//!
//! **隔离红线**：本模块严禁引用 `crate::agent::gateway / outbox`、`crate::mcp::*`、
//! `agent_send_outbox` 写入路径，或 `run_user_operation_gateway / handle_managed_message
//! / handle_follow_up_task` 等生产链路入口。`scripts/check-evolution-isolation.sh`
//! 在 CI 内静态扫描该目录强制此约束（M4 W0 Task 1.4）。
//!
//! 主循环 [`run_evolutionary_worker`] 由 `main.rs` 在 `EVOLUTION_ENABLED=true`
//! 时 spawn；关闭时 worker 直接 return，不影响主进程。波次落地节奏：
//! - W1（本波）：worker 主循环 + EvolutionBudget + experiment 信封 + cohort 选择
//! - W2：threshold 候选 + Critic LLM prompt 候选
//! - W3：Shadow eval + 显著性
//! - W4：Release + 前端 + 回滚 + post-release review
//!
//! FORBIDDEN dependencies: gateway / outbox / mcp / tasks / webhooks。

pub mod budget;
pub mod cohort;
pub mod envelope;
pub mod error;

use std::time::Duration;

use mongodb::bson::{doc, DateTime};
use tokio::time::interval;

use crate::routes::AppState;

pub use self::budget::EvolutionBudget;
pub use self::cohort::{select_cohorts, Cohorts};
pub use self::envelope::{insert_experiment_envelope, update_experiment_status};
pub use self::error::EvolutionError;

/// 演化器主循环。`EVOLUTION_ENABLED=false` 时立即 return，等价于功能未启用。
///
/// 每 `evolution_tick_seconds` 秒触发一次 [`run_one_tick`]，单次 tick 失败
/// 不影响下次（异常被捕获后写 `agent_events kind="evolution_tick_failed"`）。
pub async fn run_evolutionary_worker(state: AppState) {
    if !state.config.evolution_enabled {
        tracing::info!("evolution worker disabled (EVOLUTION_ENABLED=false); skip spawn");
        return;
    }
    let tick_seconds = state.config.evolution_tick_seconds.max(60);
    tracing::info!(
        tick_seconds,
        "evolution worker starting (M4 W1 skeleton — empty tick by design)"
    );
    let mut ticker = interval(Duration::from_secs(tick_seconds));
    loop {
        ticker.tick().await;
        if let Err(err) = run_one_tick(&state).await {
            // 单 tick 失败不再传播；最大化保留 worker 存活时间。
            tracing::warn!(?err, "evolution tick failed; continuing");
            let _ = write_tick_failed_event(&state, &err.to_string()).await;
        }
    }
}

/// 单次 tick 主流程（W1 骨架版）：
/// 1. 写 `experiments` 信封；
/// 2. 选 cohort（threshold + prompt）；
/// 3. **W1 暂不调** W2/W3 候选生成 / shadow eval / release，直接把 status 推到
///    `awaiting_admin` 并写一条 `evolution_tick_completed` 事件；
/// 4. 后续 W2/W3/W4 在此扩展即可（不破坏 envelope shape）。
pub async fn run_one_tick(state: &AppState) -> Result<(), EvolutionError> {
    let exp_id = format!(
        "exp_{}_{}",
        state.config.default_account_id,
        DateTime::now().timestamp_millis()
    );
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();

    // 1. 信封
    insert_experiment_envelope(
        state,
        &exp_id,
        &workspace_id,
        &account_id,
        state.config.evolution_eval_window_hours as i32,
    )
    .await?;

    // 2. cohort
    let cohorts = select_cohorts(state, &workspace_id, &account_id).await?;
    let threshold_count = cohorts.threshold.len();
    let prompt_count = cohorts.prompt.len();

    // 3. 推进 cohort 字段（即使为空也写，后台能看到 tick 跑过）
    state
        .db
        .experiments()
        .update_one(
            doc! { "experiment_id": &exp_id },
            doc! {
                "$set": {
                    "cohort_threshold_run_ids": cohorts.threshold.clone(),
                    "cohort_prompt_run_ids": cohorts.prompt.clone(),
                    "updated_at": DateTime::now(),
                }
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;

    // 4. 占位推进到 awaiting_admin（W2/W3 接入后改为 evaluating）。
    update_experiment_status(state, &exp_id, "awaiting_admin").await?;

    write_tick_completed_event(state, &exp_id, threshold_count, prompt_count).await?;
    Ok(())
}

async fn write_tick_completed_event(
    state: &AppState,
    exp_id: &str,
    threshold_count: usize,
    prompt_count: usize,
) -> Result<(), EvolutionError> {
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: state.config.default_account_id.clone(),
        contact_wxid: None,
        kind: "evolution_tick_completed".to_string(),
        status: "ok".to_string(),
        summary: format!(
            "evolution tick completed (threshold_cohort={threshold_count}, prompt_cohort={prompt_count})"
        ),
        details: Some(doc! {
            "experiment_id": exp_id,
            "threshold_cohort_size": threshold_count as i32,
            "prompt_cohort_size": prompt_count as i32,
            "budget_used_tokens": 0_i64,
            "proposals_count": 0_i32,
        }),
        created_at: DateTime::now(),
    };
    state
        .db
        .events()
        .insert_one(event, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

async fn write_tick_failed_event(
    state: &AppState,
    error_summary: &str,
) -> Result<(), EvolutionError> {
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: state.config.default_account_id.clone(),
        contact_wxid: None,
        kind: "evolution_tick_failed".to_string(),
        status: "error".to_string(),
        summary: format!("evolution tick failed: {}", truncate(error_summary, 1024)),
        details: Some(doc! { "error": truncate(error_summary, 1024) }),
        created_at: DateTime::now(),
    };
    state
        .db
        .events()
        .insert_one(event, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

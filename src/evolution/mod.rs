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
pub mod lint;
pub mod post_release;
pub mod prompt_critic;
pub mod release;
pub mod replay;
pub mod significance;
pub mod threshold;
pub mod auto_release;

use std::time::Duration;

use mongodb::bson::{doc, DateTime};
use tokio::time::interval;

use crate::routes::AppState;

pub use self::budget::EvolutionBudget;
pub use self::cohort::{select_cohorts, select_cohorts_filtered, Cohorts};
pub use self::envelope::{insert_experiment_envelope, update_experiment_status};
pub use self::error::EvolutionError;

pub mod runtime_flag;
pub use self::runtime_flag::{
    bucket_for_contact, is_evolution_enabled_for, load_runtime_flag, rollout_bucket_index,
};

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

/// 单次 tick 主流程：
/// 1. 写 `experiments` 信封；
/// 2. 选 cohort（threshold + prompt）；
/// 3. M4 W2：threshold 候选（纯统计，不消 EvolutionBudget）；
/// 4. M4 W2：prompt critic 候选（消 EvolutionBudget；耗尽时 silent skip）；
/// 5. 把 status 推到 `evaluating`（W3 引入 shadow eval 后由 eval 路径
///    切换到 `awaiting_admin`）；
/// 6. 写 `evolution_tick_completed` 事件。
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

    // Phase C / C3：mongo runtime flag 决定灰度桶。`enabled=false` 或文档不存在
    // → 全员排除（worker 仍跑空 tick，保留可观察性 + 写 envelope）；`enabled=true`
    // 时按 `hash(contact_id) % 100 < rollout_percent` 分桶。
    //
    // 读失败按 None 处理，避免 mongo 抖动让灰度门误开。
    let runtime_flag = match self::runtime_flag::load_runtime_flag(state, &workspace_id).await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(?err, "evolution runtime_flag load failed; treating as disabled this tick");
            None
        }
    };

    // 2. cohort（灰度过滤）
    let cohorts =
        select_cohorts_filtered(state, &workspace_id, &account_id, runtime_flag.as_ref())
            .await?;
    let threshold_count = cohorts.threshold.len();
    let prompt_count = cohorts.prompt.len();

    // 推进 cohort 字段
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

    // 3. threshold 候选（纯统计，不消 EvolutionBudget）。
    let threshold_proposals = threshold::generate(state, &exp_id, &cohorts.threshold).await?;
    insert_proposals(state, &threshold_proposals).await?;

    // 4. prompt critic 候选（消 EvolutionBudget；BudgetExceeded 不向上传播）。
    let mut budget = EvolutionBudget::from_config(&state.config);
    let prompt_proposals = match prompt_critic::generate(state, &exp_id, &cohorts, &mut budget).await
    {
        Ok(v) => v,
        Err(EvolutionError::BudgetExceeded {
            tokens_used,
            calls_used,
        }) => {
            write_budget_exceeded_event(state, &exp_id, tokens_used, calls_used).await?;
            Vec::new()
        }
        Err(e) => return Err(e),
    };
    insert_proposals(state, &prompt_proposals).await?;

    // 5. 写预算用量到 envelope。
    state
        .db
        .experiments()
        .update_one(
            doc! { "experiment_id": &exp_id },
            doc! {
                "$set": {
                    "budget_used_tokens": budget.token_used,
                    "budget_used_calls": budget.call_used as i32,
                    "updated_at": DateTime::now(),
                }
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;

    // 6. M4 W3：shadow replay + 显著性聚合。
    //    pending_eval 候选驱动；budget 在 prompt critic 阶段已记录消耗，replay
    //    现阶段 threshold 不调 LLM、prompt 走 placeholder failed，所以这里不会
    //    再触发 BudgetExceeded。
    let pending_count = threshold_proposals
        .iter()
        .chain(prompt_proposals.iter())
        .filter(|p| p.status == "pending_eval")
        .count();
    let (eligible_count, rejected_after_eval) = if pending_count > 0 {
        match replay::eval_all(state, &exp_id, &mut budget).await {
            Ok(()) => {}
            Err(EvolutionError::BudgetExceeded {
                tokens_used,
                calls_used,
            }) => {
                write_budget_exceeded_event(state, &exp_id, tokens_used, calls_used).await?;
            }
            Err(e) => return Err(e),
        }
        significance::aggregate_and_grade(state, &exp_id).await?
    } else {
        (0, 0)
    };

    // 7. 推进状态：W3 后无论候选是否存在，都直接走 awaiting_admin
    //    （eligible_for_release 由 admin 二次确认，rejected 也已落字段）。
    update_experiment_status(state, &exp_id, "awaiting_admin").await?;

    // envelope 上同步写聚合计数，便于前端 EvolutionCenterTab 拉取。
    state
        .db
        .experiments()
        .update_one(
            doc! { "experiment_id": &exp_id },
            doc! {
                "$set": {
                    "proposals_count": (threshold_proposals.len() + prompt_proposals.len()) as i32,
                    "proposals_eligible_count": eligible_count as i32,
                    "updated_at": DateTime::now(),
                }
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;

    // 8. M4 W4 Task 5.6：扫一次到期的 post_release_reviews（+24h 对比窗口）。
    //    单条失败不影响 tick；已 release 的 proposal 仍受 admin 控制是否回滚。
    let post_release_completed = post_release::run_due_reviews(state).await.unwrap_or_else(|e| {
        tracing::warn!(?e, "post_release run_due_reviews failed; will retry next tick");
        0
    });

    // 9. Phase C / C5：threshold proposal 自动 release 闭环。
    //    `evolution_auto_release_enabled=false` 时立即 return 0；
    //    `released_count > 0` 表示本 tick 触发了 hold_rate close-loop 自动放量。
    //    rollback 永远人工——Requirements 9.7 不允许自动回滚。
    let auto_released = auto_release::auto_release_eligible_thresholds(state)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(?e, "auto_release_eligible_thresholds failed; will retry next tick");
            0
        });

    write_tick_completed_event(
        state,
        &exp_id,
        threshold_count,
        prompt_count,
        threshold_proposals.len(),
        prompt_proposals.len(),
        budget.token_used,
        eligible_count,
        rejected_after_eval,
        post_release_completed,
        auto_released,
    )
    .await?;
    Ok(())
}

async fn insert_proposals(
    state: &AppState,
    proposals: &[crate::models::Proposal],
) -> Result<(), EvolutionError> {
    if proposals.is_empty() {
        return Ok(());
    }
    state
        .db
        .proposals()
        .insert_many(proposals.to_vec(), None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

async fn write_budget_exceeded_event(
    state: &AppState,
    exp_id: &str,
    tokens_used: i64,
    calls_used: i32,
) -> Result<(), EvolutionError> {
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: state.config.default_account_id.clone(),
        contact_wxid: None,
        kind: "evolution_budget_exceeded".to_string(),
        status: "warning".to_string(),
        summary: format!(
            "evolution budget exceeded (tokens_used={tokens_used}, calls_used={calls_used})"
        ),
        details: Some(doc! {
            "experiment_id": exp_id,
            "tokens_used": tokens_used,
            "calls_used": calls_used as i32,
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

async fn write_tick_completed_event(
    state: &AppState,
    exp_id: &str,
    threshold_count: usize,
    prompt_count: usize,
    threshold_proposals: usize,
    prompt_proposals: usize,
    budget_used_tokens: i64,
    proposals_eligible_count: usize,
    proposals_rejected_count: usize,
    post_release_reviews_completed: usize,
    auto_released_count: usize,
) -> Result<(), EvolutionError> {
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: state.config.default_account_id.clone(),
        contact_wxid: None,
        kind: "evolution_tick_completed".to_string(),
        status: "ok".to_string(),
        summary: format!(
            "evolution tick completed (threshold_cohort={threshold_count}, prompt_cohort={prompt_count}, threshold_proposals={threshold_proposals}, prompt_proposals={prompt_proposals}, eligible={proposals_eligible_count}, rejected={proposals_rejected_count}, post_release_reviews_completed={post_release_reviews_completed}, auto_released={auto_released_count})"
        ),
        details: Some(doc! {
            "experiment_id": exp_id,
            "threshold_cohort_size": threshold_count as i32,
            "prompt_cohort_size": prompt_count as i32,
            "threshold_proposals_count": threshold_proposals as i32,
            "prompt_proposals_count": prompt_proposals as i32,
            "budget_used_tokens": budget_used_tokens,
            "proposals_eligible_count": proposals_eligible_count as i32,
            "proposals_rejected_count": proposals_rejected_count as i32,
            "post_release_reviews_completed": post_release_reviews_completed as i32,
            "auto_released_count": auto_released_count as i32,
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

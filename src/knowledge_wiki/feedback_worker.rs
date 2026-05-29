//! `feedback_worker` —— knowledge-wiki 反馈闭环主循环。
//!
//! 一轮职责（参见 design.md §6）：
//! 1. 30d 滑窗回写 `usage_stats` + 重算 `dynamic_confidence`；
//! 2. 跑 structural lint 生成/合并 `knowledge_gap_signals`；
//! 3. stage 1 sweep（规则消解过期信号）。
//!
//! 关停态：`KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS=0` → worker 不进入循环；
//! 默认 600s（10 分钟）跟 strategic planner 同档。本轮 stage 2（LLM）
//! 暂未串入热路径，预留 [`crate::knowledge_wiki::gap_signals::sweep_stale_signals`]
//! 接口。

use std::time::Duration;

use tokio::time::sleep;

use crate::knowledge_wiki::{gap_signals, lessons_learned, reviewer_stats};
use crate::routes::AppState;

/// 反馈 worker 主循环。`interval_secs == 0` 直接 return。
pub async fn feedback_worker_loop(state: AppState, interval_secs: u64) {
    if interval_secs == 0 {
        tracing::info!("knowledge_wiki feedback_worker disabled (interval=0)");
        return;
    }
    tracing::info!(
        "knowledge_wiki feedback_worker started (interval={}s)",
        interval_secs
    );
    loop {
        if let Err(err) = run_one_round(&state).await {
            tracing::warn!(?err, "feedback_worker round failed");
        }
        sleep(Duration::from_secs(interval_secs)).await;
    }
}

/// 单轮：扫所有 workspace（取 `system_taxonomies` distinct，简版用 chunks
/// distinct workspace_id 即可），逐个跑 3 步。
async fn run_one_round(state: &AppState) -> anyhow::Result<()> {
    let workspaces = list_workspaces(state).await?;
    for ws in workspaces {
        if let Err(err) = gap_signals::refresh_usage_stats_and_confidence(
            &state.db,
            &ws,
            state.config.dynamic_confidence_min_samples,
            state.config.dynamic_confidence_real_outcome_enabled,
        )
        .await
        {
            tracing::warn!(workspace_id = %ws, ?err, "refresh_usage_stats failed");
        }
        match gap_signals::run_structural_lint(&state.db, &ws).await {
            Ok(report) => {
                tracing::info!(
                    workspace_id = %ws,
                    new = report.new_signals,
                    existing = report.existing_pending,
                    auto_resolved = report.stage1_auto_resolved,
                    "structural_lint done"
                );
            }
            Err(err) => {
                tracing::warn!(workspace_id = %ws, ?err, "structural_lint failed");
            }
        }
        match gap_signals::sweep_stale_signals(&state.db, &ws).await {
            Ok(report) => {
                if report.stage1_auto_resolved > 0 {
                    tracing::info!(
                        workspace_id = %ws,
                        stage1 = report.stage1_auto_resolved,
                        "sweep_stale_signals done"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(workspace_id = %ws, ?err, "sweep_stale_signals failed");
            }
        }
        // Phase D / D5：跨用户 lessons_learned 聚合（14d 滑窗）。
        // 默认每轮都跑——纯 mongo 聚合 + upsert，不发 LLM 调用，廉价。
        match lessons_learned::aggregate_lessons_for_workspace(state, &ws, 14).await {
            Ok(report) => {
                if report.success_lessons + report.failure_lessons + report.blocked_lessons > 0 {
                    tracing::info!(
                        workspace_id = %ws,
                        success = report.success_lessons,
                        failure = report.failure_lessons,
                        blocked = report.blocked_lessons,
                        "lessons_learned aggregate done"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(workspace_id = %ws, ?err, "lessons_learned aggregate failed");
            }
        }
        // Phase C / C1：reviewer 度量层聚合（14d 滑窗）。同样纯 mongo count +
        // upsert，廉价；汇总 reviewer 通过率 / 误判率，供 admin 面板与 C2
        // negative_example 候选挑选参考。
        match reviewer_stats::aggregate_reviewer_stats_for_workspace(state, &ws, 14).await {
            Ok(report) => {
                if report.considered > 0 {
                    tracing::info!(
                        workspace_id = %ws,
                        considered = report.considered,
                        approved = report.approved,
                        misjudge = report.approved_but_user_negative,
                        "reviewer_stats aggregate done"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(workspace_id = %ws, ?err, "reviewer_stats aggregate failed");
            }
        }
    }
    Ok(())
}

/// 列出所有有 chunk 的 workspace_id。distinct 数量假设 < 100，全量拉回内存。
async fn list_workspaces(state: &AppState) -> anyhow::Result<Vec<String>> {
    let cursor = state
        .db
        .operation_knowledge_chunks()
        .distinct("workspace_id", None, None)
        .await?;
    let workspaces: Vec<String> = cursor
        .into_iter()
        .filter_map(|b| b.as_str().map(String::from))
        .collect();
    if workspaces.is_empty() {
        // fallback：用 default workspace，避免空 distinct 导致 worker 无所事事
        Ok(vec![state.config.default_workspace_id.clone()])
    } else {
        Ok(workspaces)
    }
}

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

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use mongodb::{
    bson::{doc, DateTime, Document},
    options::{FindOneAndUpdateOptions, ReturnDocument},
};
use tokio::time::sleep;

const FEEDBACK_LEASE_SECONDS: i64 = 300;
const FEEDBACK_LEASE_HEARTBEAT_SECONDS: u64 = 60;

#[derive(Clone)]
struct FeedbackLease {
    workspace_id: String,
    token: String,
}

impl FeedbackLease {
    fn id(&self) -> String {
        format!("knowledge_feedback::{}", self.workspace_id)
    }

    fn owner_filter(&self) -> Document {
        doc! { "_id": self.id(), "token": &self.token }
    }
}

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

/// 单轮：扫所有 workspace；每个 workspace 必须先取得 Mongo lease。不同副本可并行
/// 处理不同 workspace，但同一 workspace 始终只有一个持有 fencing token 的 owner。
async fn run_one_round(state: &AppState) -> anyhow::Result<()> {
    let workspaces = list_workspaces(state).await?;
    for ws in workspaces {
        let Some(lease) = try_acquire_feedback_lease(state, &ws).await? else {
            tracing::debug!(workspace_id = %ws, "feedback workspace lease held by another replica");
            continue;
        };
        run_workspace_with_lease(state, &ws, lease).await;
    }
    Ok(())
}

async fn try_acquire_feedback_lease(
    state: &AppState,
    workspace_id: &str,
) -> anyhow::Result<Option<FeedbackLease>> {
    let now = DateTime::now();
    let token = uuid::Uuid::new_v4().to_string();
    let lease = FeedbackLease {
        workspace_id: workspace_id.to_string(),
        token: token.clone(),
    };
    let locked_until = DateTime::from_millis(
        now.timestamp_millis() + FEEDBACK_LEASE_SECONDS.saturating_mul(1_000),
    );
    let options = FindOneAndUpdateOptions::builder()
        .upsert(true)
        .return_document(ReturnDocument::After)
        .build();
    let collection = state.db.background_worker_leases();
    let result = collection
        .find_one_and_update(
            doc! {
                "_id": lease.id(),
                "$or": [
                    { "locked_until": { "$lte": now } },
                    { "locked_until": null },
                    { "locked_until": { "$exists": false } },
                ],
            },
            doc! {
                "$set": {
                    "workspace_id": workspace_id,
                    "worker_kind": "knowledge_feedback",
                    "token": &token,
                    "locked_until": locked_until,
                    "updated_at": now,
                },
                "$setOnInsert": { "created_at": now },
            },
            options,
        )
        .await;
    match result {
        Ok(Some(row)) if row.get_str("token").ok() == Some(token.as_str()) => Ok(Some(lease)),
        Ok(_) => Ok(None),
        Err(error) if is_duplicate_key_error(&error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match &*error.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            matches!(write_error.code, 11000 | 11001)
        }
        ErrorKind::BulkWrite(bulk) => bulk.write_errors.as_ref().is_some_and(|errors| {
            errors
                .iter()
                .any(|error| matches!(error.code, 11000 | 11001))
        }),
        _ => false,
    }
}

fn spawn_feedback_lease_heartbeat(
    state: AppState,
    lease: FeedbackLease,
    cancelled: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let collection = state.db.background_worker_leases();
        let mut ticker =
            tokio::time::interval(Duration::from_secs(FEEDBACK_LEASE_HEARTBEAT_SECONDS));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = DateTime::now();
            let locked_until = DateTime::from_millis(
                now.timestamp_millis() + FEEDBACK_LEASE_SECONDS.saturating_mul(1_000),
            );
            match collection
                .update_one(
                    lease.owner_filter(),
                    doc! { "$set": { "locked_until": locked_until, "updated_at": now } },
                    None,
                )
                .await
            {
                Ok(result) if result.matched_count == 1 => {}
                Ok(_) | Err(_) => {
                    cancelled.store(true, Ordering::SeqCst);
                    return;
                }
            }
        }
    })
}

async fn run_workspace_with_lease(state: &AppState, ws: &str, lease: FeedbackLease) {
    let cancelled = Arc::new(AtomicBool::new(false));
    let heartbeat = spawn_feedback_lease_heartbeat(state.clone(), lease.clone(), cancelled.clone());

    match gap_signals::refresh_usage_stats_and_confidence_controlled(
        &state.db,
        ws,
        state.config.dynamic_confidence_min_samples,
        state.config.dynamic_confidence_real_outcome_enabled,
        Some(cancelled.as_ref()),
    )
    .await
    {
        Ok(report) => {
            if report.deal_attributed_hits > 0 {
                tracing::info!(
                    workspace_id = %ws,
                    deal_attributed_hits = report.deal_attributed_hits,
                    "H11-linkage: 成交追认强化了召回置信度"
                );
            }
            if let Err(err) =
                upsert_deal_attribution_stats(state, ws, report.deal_attributed_hits).await
            {
                tracing::warn!(workspace_id = %ws, ?err, "upsert deal_attribution_stats failed");
            }
        }
        Err(err) => {
            tracing::warn!(workspace_id = %ws, ?err, "refresh_usage_stats failed");
        }
    }

    if !cancelled.load(Ordering::SeqCst) {
        match gap_signals::run_structural_lint(&state.db, ws).await {
            Ok(report) => tracing::info!(
                workspace_id = %ws,
                new = report.new_signals,
                existing = report.existing_pending,
                auto_resolved = report.stage1_auto_resolved,
                "structural_lint done"
            ),
            Err(err) => tracing::warn!(workspace_id = %ws, ?err, "structural_lint failed"),
        }
    }
    if !cancelled.load(Ordering::SeqCst) {
        if let Err(err) = gap_signals::sweep_stale_signals(&state.db, ws).await {
            tracing::warn!(workspace_id = %ws, ?err, "sweep_stale_signals failed");
        }
    }
    if !cancelled.load(Ordering::SeqCst) {
        if let Err(err) = lessons_learned::aggregate_lessons_for_workspace(state, ws, 14).await {
            tracing::warn!(workspace_id = %ws, ?err, "lessons_learned aggregate failed");
        }
    }
    if !cancelled.load(Ordering::SeqCst) {
        if let Err(err) =
            reviewer_stats::aggregate_reviewer_stats_for_workspace(state, ws, 14).await
        {
            tracing::warn!(workspace_id = %ws, ?err, "reviewer_stats aggregate failed");
        }
    }

    heartbeat.abort();
    let now = DateTime::now();
    let _ = state
        .db
        .background_worker_leases()
        .update_one(
            lease.owner_filter(),
            doc! { "$set": { "locked_until": now, "updated_at": now } },
            None,
        )
        .await;
}

/// D（可观测）：把本轮 30d 窗口成交追认强化的命中数 upsert 到滚动统计 doc。
/// 仿 reviewer_stats：每 workspace 一行，stat_id = `<ws>::deal_attribution`，`$set`
/// 覆盖（瞬时值非累加），为 0 也写锚点。phase_rollup 读出展示 H11-linkage 效果。
async fn upsert_deal_attribution_stats(
    state: &AppState,
    workspace_id: &str,
    deal_attributed_hits: u64,
) -> anyhow::Result<()> {
    use mongodb::bson::{doc, DateTime};
    let now = DateTime::now();
    let stat_id = format!("{workspace_id}::deal_attribution");
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("deal_attribution_stats")
        .update_one(
            doc! { "stat_id": &stat_id, "workspace_id": workspace_id },
            doc! {
                "$set": {
                    "workspace_id": workspace_id,
                    "deal_attributed_hits": deal_attributed_hits as i64,
                    "updated_at": now,
                },
                "$setOnInsert": {
                    "stat_id": &stat_id,
                    "created_at": now,
                },
            },
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_lease_identity_is_workspace_scoped_and_token_fenced() {
        let a = FeedbackLease {
            workspace_id: "ws-a".into(),
            token: "token-a".into(),
        };
        let b = FeedbackLease {
            workspace_id: "ws-b".into(),
            token: "token-a".into(),
        };
        assert_ne!(a.id(), b.id());
        assert_eq!(a.owner_filter().get_str("token").unwrap(), "token-a");
        assert_eq!(
            a.owner_filter().get_str("_id").unwrap(),
            "knowledge_feedback::ws-a"
        );
    }

    #[test]
    fn feedback_lease_duration_exceeds_heartbeat_interval() {
        assert!(FEEDBACK_LEASE_SECONDS > FEEDBACK_LEASE_HEARTBEAT_SECONDS as i64);
    }
}

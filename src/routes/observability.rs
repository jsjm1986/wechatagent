//! Phase 0-D 自治信号 admin 聚合：把已经在 DB 里、admin UI 还看不到的关键信号
//! 用一次 RTT 拉齐：
//!
//! - lifecycle 终态分布（`agent_run_logs.lifecycle` 近 24h）
//! - revision_reason top（`agent_run_logs.revision_reason` 非空近 24h）
//! - reviewer_misjudge_signal 分类（`decision_reviews.reviewer_misjudge_signal` 近 24h）
//! - negative_example 候选数（`operation_knowledge_chunks` 即时计数）
//!
//! 设计取舍：
//! - 全只读，零写路径，红线零引入。
//! - workspace_id 强制 default_workspace_id，与 ops 三表 admin 路由同源。
//! - 空集合返回空数组 / 0；不抛 5xx 给前端。
//! - lifecycle 闭集与 [`crate::agent::run_envelope`] 同源，DB 偶发出现非闭集
//!   值时（理论上 R9.10.e 已拦截）原样透出，不静默吞掉。
//!
//! 不做：
//! - cold_contact_worker / account_scheduler 计数：等 staging 出现真实事件
//!   再加，避免 over-build。
//! - lessons_learned pattern × status 矩阵：已在 [`super::lessons_learned`]
//!   面板单独出现，不在本接口重复。

use axum::{extract::State, Json};
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use serde_json::{json, Value};

use crate::{
    agent::run_envelope::{
        LIFECYCLE_ABORTED_BY_BUDGET, LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL, LIFECYCLE_COMPLETED,
        LIFECYCLE_FAILED_AFTER_DECISION, LIFECYCLE_FAILED_BEFORE_DECISION, LIFECYCLE_RUNNING,
        LIFECYCLE_STARTED,
    },
    error::AppResult,
};

use super::AppState;

/// 24h 滑窗（毫秒）。固定值；admin 只读面板，没必要做参数化。
const WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// revision_reason top N：避免 admin 面板被低频 reason 噪声淹没。
const REVISION_REASON_TOP_N: i64 = 10;

pub(super) async fn phase_rollup(
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let workspace = state.config.default_workspace_id.clone();

    let lifecycle = aggregate_lifecycle(&state, &workspace).await?;
    let revision_reasons = aggregate_revision_reasons(&state, &workspace).await?;
    let reviewer_misjudge = aggregate_reviewer_misjudge(&state, &workspace).await?;
    let negative_example_pending = count_negative_example_pending(&state, &workspace).await?;

    Ok(Json(json!({
        "windowHours": 24,
        "lifecycle": lifecycle,
        "revisionReasons": revision_reasons,
        "reviewerMisjudge": reviewer_misjudge,
        "negativeExamplePending": negative_example_pending,
    })))
}

async fn aggregate_lifecycle(
    state: &AppState,
    workspace: &str,
) -> AppResult<Value> {
    let since = mongodb::bson::DateTime::from_millis(now_ms() - WINDOW_MS);
    let coll = state.db.raw().collection::<Document>("agent_run_logs");
    let pipeline = vec![
        doc! { "$match": { "workspace_id": workspace, "created_at": { "$gte": since } } },
        doc! { "$group": { "_id": "$lifecycle", "count": { "$sum": 1 } } },
    ];
    let mut cursor = coll.aggregate(pipeline, None).await?;
    let mut buckets: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    while let Some(d) = cursor.try_next().await? {
        let key = d
            .get("_id")
            .and_then(|b| b.as_str())
            .map(str::to_string)
            .unwrap_or_default();
        let count = d.get_i64("count").unwrap_or(0);
        if !key.is_empty() {
            buckets.insert(key, count);
        }
    }
    // 把闭集 7 个值作为稳定 key 输出（无样本时 0），便于前端不抖动。
    let known: [&str; 7] = [
        LIFECYCLE_STARTED,
        LIFECYCLE_RUNNING,
        LIFECYCLE_COMPLETED,
        LIFECYCLE_FAILED_BEFORE_DECISION,
        LIFECYCLE_FAILED_AFTER_DECISION,
        LIFECYCLE_ABORTED_BY_BUDGET,
        LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL,
    ];
    let mut items: Vec<Value> = known
        .iter()
        .map(|k| {
            json!({
                "lifecycle": k,
                "count": buckets.remove(*k).unwrap_or(0),
            })
        })
        .collect();
    // 闭集外的（理论上 R9.10.e 不会落库）原样透出，不吞。
    let mut leftovers: Vec<(String, i64)> = buckets.into_iter().collect();
    leftovers.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, v) in leftovers {
        items.push(json!({ "lifecycle": k, "count": v, "outOfClosedSet": true }));
    }
    Ok(Value::Array(items))
}

async fn aggregate_revision_reasons(
    state: &AppState,
    workspace: &str,
) -> AppResult<Value> {
    let since = mongodb::bson::DateTime::from_millis(now_ms() - WINDOW_MS);
    let coll = state.db.raw().collection::<Document>("agent_run_logs");
    let pipeline = vec![
        doc! { "$match": {
            "workspace_id": workspace,
            "created_at": { "$gte": since },
            "revision_reason": { "$exists": true, "$nin": [null, ""] },
        } },
        doc! { "$group": { "_id": "$revision_reason", "count": { "$sum": 1 } } },
        doc! { "$sort": { "count": -1 } },
        doc! { "$limit": REVISION_REASON_TOP_N },
    ];
    let mut cursor = coll.aggregate(pipeline, None).await?;
    let mut items = Vec::new();
    while let Some(d) = cursor.try_next().await? {
        let reason = d
            .get("_id")
            .and_then(|b| b.as_str())
            .map(str::to_string)
            .unwrap_or_default();
        let count = d.get_i64("count").unwrap_or(0);
        if reason.is_empty() {
            continue;
        }
        items.push(json!({ "reason": reason, "count": count }));
    }
    Ok(Value::Array(items))
}

async fn aggregate_reviewer_misjudge(
    state: &AppState,
    workspace: &str,
) -> AppResult<Value> {
    let since = mongodb::bson::DateTime::from_millis(now_ms() - WINDOW_MS);
    let coll = state.db.raw().collection::<Document>("decision_reviews");
    let pipeline = vec![
        doc! { "$match": {
            "workspace_id": workspace,
            "created_at": { "$gte": since },
            "reviewer_misjudge_signal": { "$exists": true, "$ne": null },
        } },
        doc! { "$group": { "_id": "$reviewer_misjudge_signal", "count": { "$sum": 1 } } },
    ];
    let mut cursor = coll.aggregate(pipeline, None).await?;
    let mut items = Vec::new();
    while let Some(d) = cursor.try_next().await? {
        let kind = d
            .get("_id")
            .and_then(|b| b.as_str())
            .map(str::to_string)
            .unwrap_or_default();
        let count = d.get_i64("count").unwrap_or(0);
        if kind.is_empty() {
            continue;
        }
        items.push(json!({ "kind": kind, "count": count }));
    }
    items.sort_by(|a, b| {
        b.get("count")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .cmp(&a.get("count").and_then(Value::as_i64).unwrap_or(0))
    });
    Ok(Value::Array(items))
}

async fn count_negative_example_pending(
    state: &AppState,
    workspace: &str,
) -> AppResult<i64> {
    let coll = state.db.raw().collection::<Document>("operation_knowledge_chunks");
    let n = coll
        .count_documents(
            doc! {
                "workspace_id": workspace,
                "chunk_type": "negative_example",
                "integrity_status": "needs_review",
            },
            None,
        )
        .await?;
    Ok(n as i64)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_ms_is_exactly_24_hours() {
        assert_eq!(WINDOW_MS, 86_400_000);
    }

    #[test]
    fn lifecycle_closed_set_size_matches_run_envelope() {
        // 与 [`crate::agent::run_envelope`] 闭集同步。改了那边必须改这里。
        let known: [&str; 7] = [
            LIFECYCLE_STARTED,
            LIFECYCLE_RUNNING,
            LIFECYCLE_COMPLETED,
            LIFECYCLE_FAILED_BEFORE_DECISION,
            LIFECYCLE_FAILED_AFTER_DECISION,
            LIFECYCLE_ABORTED_BY_BUDGET,
            LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL,
        ];
        assert_eq!(known.len(), 7);
        // 全互不相同
        let mut sorted = known.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 7);
    }

    #[test]
    fn revision_reason_top_n_is_bounded() {
        // top 10 既能覆盖常见 reason（双闸 / fact_risk / pressure_risk / dual_reviewer
        // disagreement 等）又不让面板过长。
        assert!((1..=20).contains(&REVISION_REASON_TOP_N));
    }
}

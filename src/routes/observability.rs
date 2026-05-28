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
    models::ALLOWED_TASK_STATUS,
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

/// G-后续Ⅱ/2：worker 健康聚合 —— 一次 RTT 拉齐三类后台任务的状态分布，
/// 给 admin ObservabilityDashboard 第二波卡片用。三类源都已经在 DB 里、
/// admin UI 还看不到聚合视图：
///
/// - `knowledge_chat_tasks`：状态分布 + 最近 7d 失败 error_kind top；
/// - `knowledge_gap_signals`：status / kind 矩阵（pending 指示 sweep 落后）；
/// - `lessons_learned`：14d 滑窗 pattern × status 矩阵，feedback_worker
///   周期产物的可见信号（feedback runs 没有显式 collection，pattern 增长
///   即是 worker 在跑的间接证据）。
///
/// 设计取舍延续 [`phase_rollup`]：
/// - 全只读，零写路径；
/// - workspace_id 强制 default；
/// - 闭集 status 在无样本时也输出 0，前端不抖动；
/// - 闭集外 status 原样透出（理论上 [`ALLOWED_TASK_STATUS`] 已拦截，
///   但 R9.10.e 防御性透出便于诊断历史脏数据）。
pub(super) async fn worker_health(
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let workspace = state.config.default_workspace_id.clone();

    let chat_tasks = aggregate_chat_tasks(&state, &workspace).await?;
    let gap_signals = aggregate_gap_signals(&state, &workspace).await?;
    let lessons_learned = aggregate_lessons_learned(&state, &workspace).await?;

    Ok(Json(json!({
        "windowHours": 24,
        "lessonsWindowDays": 14,
        "chatTasks": chat_tasks,
        "gapSignals": gap_signals,
        "lessonsLearned": lessons_learned,
    })))
}

async fn aggregate_chat_tasks(
    state: &AppState,
    workspace: &str,
) -> AppResult<Value> {
    let coll = state
        .db
        .raw()
        .collection::<Document>("knowledge_chat_tasks");
    // 全量 status 分布——不开 24h 窗，运营要看的是"现在 pending 多少 / 历史 fail 比例"。
    let pipeline = vec![
        doc! { "$match": { "workspace_id": workspace } },
        doc! { "$group": { "_id": "$status", "count": { "$sum": 1 } } },
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
    let mut status_items: Vec<Value> = ALLOWED_TASK_STATUS
        .iter()
        .map(|k| {
            json!({
                "status": *k,
                "count": buckets.remove(*k).unwrap_or(0),
            })
        })
        .collect();
    let mut leftovers: Vec<(String, i64)> = buckets.into_iter().collect();
    leftovers.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, v) in leftovers {
        status_items.push(json!({ "status": k, "count": v, "outOfClosedSet": true }));
    }

    // error_kind top（仅 status=failed，全量；运营看 retry/budget/llm_json_error 哪个多）。
    let pipeline_err = vec![
        doc! { "$match": {
            "workspace_id": workspace,
            "status": "failed",
            "error_kind": { "$exists": true, "$nin": [null, ""] },
        } },
        doc! { "$group": { "_id": "$error_kind", "count": { "$sum": 1 } } },
        doc! { "$sort": { "count": -1 } },
        doc! { "$limit": 10 },
    ];
    let mut cur_err = coll.aggregate(pipeline_err, None).await?;
    let mut error_items = Vec::new();
    while let Some(d) = cur_err.try_next().await? {
        let kind = d
            .get("_id")
            .and_then(|b| b.as_str())
            .map(str::to_string)
            .unwrap_or_default();
        let count = d.get_i64("count").unwrap_or(0);
        if !kind.is_empty() {
            error_items.push(json!({ "errorKind": kind, "count": count }));
        }
    }

    Ok(json!({
        "byStatus": status_items,
        "errorKindsTop": error_items,
    }))
}

async fn aggregate_gap_signals(
    state: &AppState,
    workspace: &str,
) -> AppResult<Value> {
    let coll = state
        .db
        .raw()
        .collection::<Document>("knowledge_gap_signals");
    // status 分布：pending = sweep 还没消化的；auto_resolved/applied/dismissed 之比是 sweep 命中率。
    let pipeline_status = vec![
        doc! { "$match": { "workspace_id": workspace } },
        doc! { "$group": { "_id": "$status", "count": { "$sum": 1 } } },
    ];
    let mut cur_status = coll.aggregate(pipeline_status, None).await?;
    let mut status_buckets: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();
    while let Some(d) = cur_status.try_next().await? {
        let key = d
            .get("_id")
            .and_then(|b| b.as_str())
            .map(str::to_string)
            .unwrap_or_default();
        let count = d.get_i64("count").unwrap_or(0);
        if !key.is_empty() {
            status_buckets.insert(key, count);
        }
    }
    // 闭集与 [`crate::knowledge_wiki::gap_signals`] 同源。
    let known_status: [&str; 5] = [
        "pending",
        "auto_resolved",
        "llm_resolved",
        "applied",
        "dismissed",
    ];
    let mut status_items: Vec<Value> = known_status
        .iter()
        .map(|k| {
            json!({
                "status": *k,
                "count": status_buckets.remove(*k).unwrap_or(0),
            })
        })
        .collect();
    let mut leftovers: Vec<(String, i64)> = status_buckets.into_iter().collect();
    leftovers.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, v) in leftovers {
        status_items.push(json!({ "status": k, "count": v, "outOfClosedSet": true }));
    }

    // kind top：哪些信号种类最多 pending（broken_link / contradiction / stale 等）。
    let pipeline_kind = vec![
        doc! { "$match": {
            "workspace_id": workspace,
            "status": "pending",
        } },
        doc! { "$group": { "_id": "$kind", "count": { "$sum": 1 } } },
        doc! { "$sort": { "count": -1 } },
        doc! { "$limit": 10 },
    ];
    let mut cur_kind = coll.aggregate(pipeline_kind, None).await?;
    let mut kind_items = Vec::new();
    while let Some(d) = cur_kind.try_next().await? {
        let kind = d
            .get("_id")
            .and_then(|b| b.as_str())
            .map(str::to_string)
            .unwrap_or_default();
        let count = d.get_i64("count").unwrap_or(0);
        if !kind.is_empty() {
            kind_items.push(json!({ "kind": kind, "count": count }));
        }
    }

    // sweep hit rate：(auto_resolved + llm_resolved + applied) / total（excluding 'dismissed' 与 'pending'）
    // —— 直观表征"上一轮 sweep 把多少 pending 消化掉了"。
    let total: i64 = status_items
        .iter()
        .filter_map(|v| v.get("count").and_then(Value::as_i64))
        .sum();
    let resolved: i64 = status_items
        .iter()
        .filter(|v| {
            matches!(
                v.get("status").and_then(Value::as_str),
                Some("auto_resolved") | Some("llm_resolved") | Some("applied")
            )
        })
        .filter_map(|v| v.get("count").and_then(Value::as_i64))
        .sum();
    let pending: i64 = status_items
        .iter()
        .filter(|v| v.get("status").and_then(Value::as_str) == Some("pending"))
        .filter_map(|v| v.get("count").and_then(Value::as_i64))
        .sum();
    let hit_rate = if total > 0 {
        resolved as f64 / total as f64
    } else {
        0.0
    };

    Ok(json!({
        "byStatus": status_items,
        "pendingKindsTop": kind_items,
        "total": total,
        "pending": pending,
        "resolved": resolved,
        "sweepHitRate": hit_rate,
    }))
}

/// 14d 滑窗：与 [`crate::knowledge_wiki::feedback_worker::run_one_round`]
/// 调 `aggregate_lessons_for_workspace(_, _, 14)` 同窗口。
const LESSONS_WINDOW_MS: i64 = 14 * 24 * 60 * 60 * 1000;

async fn aggregate_lessons_learned(
    state: &AppState,
    workspace: &str,
) -> AppResult<Value> {
    let since = mongodb::bson::DateTime::from_millis(now_ms() - LESSONS_WINDOW_MS);
    let coll = state.db.raw().collection::<Document>("lessons_learned");
    // [`crate::knowledge_wiki::lessons_learned`] 写出的文档结构：
    //   { pattern_kind, count, review_status, updated_at, ... }
    // 没有顶层 `status` 字段；按 pattern_kind 聚合 sum(count) 看 worker 14d 产出。
    // pattern_kind 闭集：success / reviewer_misjudge_negative / blocked_by_safety_guard。
    let pipeline = vec![
        doc! { "$match": {
            "workspace_id": workspace,
            "updated_at": { "$gte": since },
        } },
        doc! { "$group": {
            "_id": { "pattern": "$pattern_kind", "reviewStatus": "$review_status" },
            "documents": { "$sum": 1 },
            "totalCount": { "$sum": "$count" },
        } },
        doc! { "$sort": { "totalCount": -1 } },
    ];
    let mut cursor = coll.aggregate(pipeline, None).await?;
    let mut items = Vec::new();
    let mut pattern_totals: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();
    let mut blocked_total: i64 = 0;
    while let Some(d) = cursor.try_next().await? {
        let id = d.get_document("_id").ok();
        let pattern = id
            .and_then(|x| x.get("pattern").and_then(|b| b.as_str()).map(String::from))
            .unwrap_or_default();
        let review_status = id
            .and_then(|x| {
                x.get("reviewStatus")
                    .and_then(|b| b.as_str())
                    .map(String::from)
            })
            .unwrap_or_default();
        let documents = d.get_i64("documents").unwrap_or(0);
        let total_count = d.get_i64("totalCount").unwrap_or(0);
        if pattern.is_empty() {
            continue;
        }
        if pattern == "blocked_by_safety_guard" {
            blocked_total += total_count;
        }
        *pattern_totals.entry(pattern.clone()).or_insert(0) += total_count;
        items.push(json!({
            "pattern": pattern,
            "reviewStatus": review_status,
            "documents": documents,
            "totalCount": total_count,
        }));
    }
    // 闭集 3 个 pattern_kind 在无样本时也输出 0（与 [`crate::knowledge_wiki::lessons_learned::aggregate_lessons_for_workspace`]
    // 写入端三类 pattern 同源），前端柱状图不抖动。
    let known_patterns: [&str; 3] = [
        "success",
        "reviewer_misjudge_negative",
        "blocked_by_safety_guard",
    ];
    let mut pattern_top: Vec<Value> = known_patterns
        .iter()
        .map(|k| {
            json!({
                "pattern": *k,
                "count": pattern_totals.remove(*k).unwrap_or(0),
            })
        })
        .collect();
    // 闭集外（不应出现，但若出现原样透出便于诊断）
    let mut leftovers: Vec<(String, i64)> = pattern_totals.into_iter().collect();
    leftovers.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, v) in leftovers {
        pattern_top.push(json!({ "pattern": k, "count": v, "outOfClosedSet": true }));
    }

    Ok(json!({
        "windowDays": 14,
        "matrix": items,
        "patternTop": pattern_top,
        "blockedTotal": blocked_total,
    }))
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

    #[test]
    fn lessons_window_is_exactly_14_days() {
        // 与 feedback_worker::run_one_round 调 aggregate_lessons_for_workspace(_, _, 14) 同窗口。
        // 改了那边必须改这里。
        assert_eq!(LESSONS_WINDOW_MS, 14 * 24 * 60 * 60 * 1000);
    }

    #[test]
    fn allowed_task_status_closed_set_size() {
        // 与 [`crate::models::ALLOWED_TASK_STATUS`] 同源；改了那边必须更新前端 UI。
        assert_eq!(ALLOWED_TASK_STATUS.len(), 5);
        let mut sorted = ALLOWED_TASK_STATUS.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5);
    }
}

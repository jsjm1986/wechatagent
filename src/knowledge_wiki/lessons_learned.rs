//! Phase D / D5：跨用户 lessons_learned 聚合。
//!
//! 设计目标：把每条 `agent_run_logs` 的胜/败模式压缩成"可被下一轮决策检索"的
//! 知识颗粒。与 `feedback_worker` 已有的"按 reviewer 维度聚合统计"互补：
//! - reviewer_stats：度量层（reviewer 通过率、误判信号）；
//! - **lessons_learned**：模式层（"在 X 条件下用 Y 措辞 → 用户 Z 反应"）。
//!
//! 与 `chunk_type=peer_case` 的关系：lessons_learned 是 peer_case chunk 的
//! 上游候选池。本模块只负责发现 + 存储 lessons；是否最终晋升为 chunk 由
//! admin review 决定（不绕开 review queue）。
//!
//! 输入：近 N 天 agent_run_logs.final_review_status ∈ {approved-类, blocked-类}
//! + reaction.user_polarity 信号；
//! 输出：去重后的 `lessons_learned` 文档，字段含 pattern_kind / sample /
//! source_run_ids / count。

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};

use crate::routes::AppState;

/// 单 workspace 单轮聚合。N 天窗口由 caller 透传，默认 14d。
pub async fn aggregate_lessons_for_workspace(
    state: &AppState,
    workspace_id: &str,
    window_days: i64,
) -> anyhow::Result<LessonsAggregateReport> {
    let now_ms = DateTime::now().timestamp_millis();
    let since = DateTime::from_millis(now_ms - window_days.max(1) * 24 * 60 * 60 * 1000);

    let mut report = LessonsAggregateReport::default();

    // 1) success 模式：approved + 用户后续正反应（reaction.user_polarity ∈
    //    {"positive","constructive"}）。
    report.success_lessons = upsert_pattern(
        state,
        workspace_id,
        "success",
        doc! {
            "workspace_id": workspace_id,
            "lifecycle": "completed",
            "final_review_status": "approved",
            "created_at": { "$gte": since },
            "review.reaction_analysis.user_polarity": {
                "$in": ["positive", "constructive"],
            },
        },
    )
    .await?;

    // 2) failure 模式：approved 但用户负反应（reviewer 通过却挨骂）。
    report.failure_lessons = upsert_pattern(
        state,
        workspace_id,
        "reviewer_misjudge_negative",
        doc! {
            "workspace_id": workspace_id,
            "lifecycle": "completed",
            "final_review_status": "approved",
            "created_at": { "$gte": since },
            "review.reaction_analysis.user_polarity": "negative",
        },
    )
    .await?;

    // 3) blocked 模式：safety_guard 拦截了 → 教训"哪些类型的草稿不该再出"。
    report.blocked_lessons = upsert_pattern(
        state,
        workspace_id,
        "blocked_by_safety_guard",
        doc! {
            "workspace_id": workspace_id,
            "lifecycle": "completed",
            "final_review_status": "blocked_by_safety_guard",
            "created_at": { "$gte": since },
        },
    )
    .await?;

    Ok(report)
}

#[derive(Debug, Default)]
pub struct LessonsAggregateReport {
    pub success_lessons: usize,
    pub failure_lessons: usize,
    pub blocked_lessons: usize,
}

async fn upsert_pattern(
    state: &AppState,
    workspace_id: &str,
    pattern_kind: &str,
    filter: Document,
) -> anyhow::Result<usize> {
    let count = state
        .db
        .raw()
        .collection::<Document>("agent_run_logs")
        .count_documents(filter.clone(), None)
        .await?;
    if count == 0 {
        return Ok(0);
    }

    // 抽样最近 5 条作为代表样本，给 chunk reviewer 做参考。
    let opts = mongodb::options::FindOptions::builder()
        .sort(doc! { "created_at": -1 })
        .limit(5)
        .build();
    let mut cursor = state
        .db
        .raw()
        .collection::<Document>("agent_run_logs")
        .find(filter, opts)
        .await?;
    let mut sample_run_ids: Vec<String> = Vec::new();
    while let Some(doc) = cursor.try_next().await? {
        if let Ok(run_id) = doc.get_str("run_id") {
            sample_run_ids.push(run_id.to_string());
        }
    }

    let now = DateTime::now();
    let lesson_id = format!("{workspace_id}::{pattern_kind}");
    let update = doc! {
        "$set": {
            "workspace_id": workspace_id,
            "pattern_kind": pattern_kind,
            "count": count as i64,
            "sample_run_ids": sample_run_ids,
            "updated_at": now,
        },
        "$setOnInsert": {
            "lesson_id": &lesson_id,
            "created_at": now,
            "promoted_chunk_id": null,
            "review_status": "pending_review",
        },
    };
    state
        .db
        .raw()
        .collection::<Document>("lessons_learned")
        .update_one(
            doc! { "lesson_id": &lesson_id },
            update,
            mongodb::options::UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_default_is_zero() {
        let r = LessonsAggregateReport::default();
        assert_eq!(r.success_lessons, 0);
        assert_eq!(r.failure_lessons, 0);
        assert_eq!(r.blocked_lessons, 0);
    }

    #[test]
    fn lesson_id_format_is_workspace_scoped() {
        // 文档 lesson_id 必须 workspace 前缀，避免跨 workspace 串扰。
        let id = format!("{}::{}", "ws_a", "success");
        assert!(id.starts_with("ws_a::"));
        assert!(id.ends_with("::success"));
    }
}

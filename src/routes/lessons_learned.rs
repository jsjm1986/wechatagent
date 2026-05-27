//! Phase D / D5：`lessons_learned` admin 只读列表。
//!
//! 上游写入路径在 [`crate::knowledge_wiki::lessons_learned::aggregate_lessons_for_workspace`]
//! （feedback_worker 周期触发）。本模块仅暴露：
//!
//! - `GET /api/admin/lessons-learned?patternKind=&limit=`
//!
//! 设计取舍：
//! - 只读。"晋升为 peer_case chunk" 走的是 `operation_knowledge_chunks` review queue
//!   的写路径，跨集合 + 跨幂等键，不在本最小闭环里开口子；admin 看到归纳结果就完
//!   成"召回闭环"的可观测。
//! - 三类 pattern：`success / reviewer_misjudge_negative / blocked_by_safety_guard`，
//!   与 [`crate::knowledge_wiki::lessons_learned`] 写入端枚举对齐；未识别的 pattern
//!   保留在 JSON 输出，不在 server 侧白名单（让上游 schema 自由演进）。
//! - workspace 隔离：filter 强制 `workspace_id == default_workspace_id`，与 ops 三表
//!   admin 路由同源。

use axum::{
    extract::{Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{error::AppResult, models::dt_to_string};

use super::AppState;

const DEFAULT_LIST_LIMIT: i64 = 50;
const MAX_LIST_LIMIT: i64 = 200;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListLessonsLearnedQuery {
    /// 不传 = 全部三类；传 `success` / `reviewer_misjudge_negative` /
    /// `blocked_by_safety_guard` 任一 → 精确匹配。
    pattern_kind: Option<String>,
    /// 默认 50；clamp 到 [1, 200]。
    limit: Option<i64>,
}

pub(super) async fn list_lessons_learned(
    State(state): State<AppState>,
    Query(query): Query<ListLessonsLearnedQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &state.config.default_workspace_id };
    if let Some(pk) = query.pattern_kind.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("pattern_kind", pk.trim());
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);

    let mut cursor = state
        .db
        .raw()
        .collection::<Document>("lessons_learned")
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(doc) = cursor.try_next().await? {
        items.push(lesson_doc_to_json(&doc));
    }
    Ok(Json(json!({ "items": items })))
}

fn lesson_doc_to_json(doc: &Document) -> Value {
    let lesson_id = doc
        .get_str("lesson_id")
        .ok()
        .map(str::to_string)
        .unwrap_or_default();
    let workspace_id = doc
        .get_str("workspace_id")
        .ok()
        .map(str::to_string)
        .unwrap_or_default();
    let pattern_kind = doc
        .get_str("pattern_kind")
        .ok()
        .map(str::to_string)
        .unwrap_or_default();
    let count = doc.get_i64("count").unwrap_or(0);
    let sample_run_ids: Vec<String> = doc
        .get_array("sample_run_ids")
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let updated_at = doc
        .get_datetime("updated_at")
        .ok()
        .copied()
        .map(dt_to_string)
        .unwrap_or_default();
    let created_at = doc
        .get_datetime("created_at")
        .ok()
        .copied()
        .map(dt_to_string)
        .unwrap_or_default();
    let review_status = doc
        .get_str("review_status")
        .ok()
        .map(str::to_string)
        .unwrap_or_else(|| "pending_review".to_string());
    // promoted_chunk_id 可能是 null / ObjectId / String；只把 String 形态透出
    // （DB 里写的是 null，未来若以字符串 promote 也兼容）。
    let promoted_chunk_id = doc
        .get("promoted_chunk_id")
        .and_then(|b| b.as_str().map(str::to_string));

    json!({
        "lessonId": lesson_id,
        "workspaceId": workspace_id,
        "patternKind": pattern_kind,
        "count": count,
        "sampleRunIds": sample_run_ids,
        "updatedAt": updated_at,
        "createdAt": created_at,
        "reviewStatus": review_status,
        "promotedChunkId": promoted_chunk_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::DateTime;

    #[test]
    fn list_query_defaults_are_empty_and_limit_none() {
        let q: ListLessonsLearnedQuery = serde_json::from_value(json!({})).unwrap();
        assert!(q.pattern_kind.is_none());
        assert!(q.limit.is_none());
    }

    #[test]
    fn list_query_camel_case_pattern_kind_decodes() {
        let q: ListLessonsLearnedQuery = serde_json::from_value(json!({
            "patternKind": "success",
            "limit": 10,
        }))
        .unwrap();
        assert_eq!(q.pattern_kind.as_deref(), Some("success"));
        assert_eq!(q.limit, Some(10));
    }

    #[test]
    fn limit_clamp_keeps_range() {
        let big = 99999i64.clamp(1, MAX_LIST_LIMIT);
        assert_eq!(big, MAX_LIST_LIMIT);
        let zero = 0i64.clamp(1, MAX_LIST_LIMIT);
        assert_eq!(zero, 1);
        let neg = (-5i64).clamp(1, MAX_LIST_LIMIT);
        assert_eq!(neg, 1);
    }

    #[test]
    fn lesson_doc_to_json_handles_all_three_pattern_kinds() {
        let now = DateTime::now();
        for kind in [
            "success",
            "reviewer_misjudge_negative",
            "blocked_by_safety_guard",
        ] {
            let mut doc = Document::new();
            doc.insert("lesson_id", format!("default::{}", kind));
            doc.insert("workspace_id", "default");
            doc.insert("pattern_kind", kind);
            doc.insert("count", 7i64);
            doc.insert("sample_run_ids", vec!["r1", "r2"]);
            doc.insert("updated_at", now);
            doc.insert("created_at", now);
            doc.insert("review_status", "pending_review");
            let v = lesson_doc_to_json(&doc);
            assert_eq!(v["patternKind"], kind);
            assert_eq!(v["count"], 7);
            assert_eq!(v["sampleRunIds"], json!(["r1", "r2"]));
            assert_eq!(v["reviewStatus"], "pending_review");
            assert!(v["promotedChunkId"].is_null());
        }
    }

    #[test]
    fn lesson_doc_to_json_tolerates_missing_fields() {
        // 老库 / 异常文档：缺字段时给安全默认值，不应 panic。
        let v = lesson_doc_to_json(&Document::new());
        assert_eq!(v["patternKind"], "");
        assert_eq!(v["count"], 0);
        assert_eq!(v["sampleRunIds"], json!([]));
        assert_eq!(v["reviewStatus"], "pending_review");
        assert!(v["promotedChunkId"].is_null());
    }
}

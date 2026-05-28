//! Phase D / D5：`lessons_learned` admin 只读列表 + Phase-D 收尾的 peer_case 晋升。
//!
//! 上游写入路径在 [`crate::knowledge_wiki::lessons_learned::aggregate_lessons_for_workspace`]
//! （feedback_worker 周期触发）。本模块暴露：
//!
//! - `GET  /api/admin/lessons-learned?patternKind=&limit=`
//! - `POST /api/admin/lessons-learned/:lesson_id/promote-to-peer-case`
//!
//! 设计取舍：
//! - 列表只读。"晋升为 peer_case chunk" 走 `operation_knowledge_chunks` 写路径，
//!   chunk 默认 `integrity_status=needs_review` + `status=draft`，由现有 chunk
//!   review queue 二次审核才落地，**不绕开 review queue**（与 negative_example
//!   入库保持同源 admin gate）。
//! - 三类 pattern：`success / reviewer_misjudge_negative / blocked_by_safety_guard`，
//!   与 [`crate::knowledge_wiki::lessons_learned`] 写入端枚举对齐；未识别的 pattern
//!   保留在 JSON 输出，不在 server 侧白名单（让上游 schema 自由演进）。
//! - workspace 隔离：filter 强制 `workspace_id == default_workspace_id`，与 ops 三表
//!   admin 路由同源。
//! - 幂等：lesson `review_status="promoted"` 且 `promoted_chunk_id` 已存在时，再次
//!   POST promote 直接返回已有 chunk_id，不重复写库。

use axum::{
    extract::{Path, Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    models::{dt_to_string, OperationKnowledgeChunk},
};

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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PromoteLessonRequest {
    /// peer_case chunk 的 title；admin 抽象后填入。≤ 200 字。
    title: String,
    /// peer_case chunk 的 body（≈ 案例正文）。≤ 4000 字。
    body: String,
    /// 可选 summary（一句话）。
    #[serde(default)]
    summary: Option<String>,
}

/// 纯校验：把 [`PromoteLessonRequest`] 字符串字段 trim + 长度门跑一遍。
///
/// 抽出来便于在 lib 用例里 PBT 边界（空字符串 / 全空白 / 超 200 / 超 4000），
/// 不必启动 axum / mongo。
pub(crate) fn validate_promote_request(
    payload: &PromoteLessonRequest,
) -> Result<(String, String, Option<String>), AppError> {
    let title = payload.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::BadRequest("title is required".to_string()));
    }
    if title.chars().count() > 200 {
        return Err(AppError::BadRequest(
            "title must be ≤ 200 chars".to_string(),
        ));
    }
    let body = payload.body.trim().to_string();
    if body.is_empty() {
        return Err(AppError::BadRequest("body is required".to_string()));
    }
    if body.chars().count() > 4000 {
        return Err(AppError::BadRequest(
            "body must be ≤ 4000 chars".to_string(),
        ));
    }
    let summary_opt = payload
        .summary
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok((title, body, summary_opt))
}

/// `POST /api/admin/lessons-learned/:lesson_id/promote-to-peer-case`
///
/// admin 把一条 lesson 抽象为 `chunk_type=peer_case` 的候选 chunk：
/// - chunk `integrity_status=needs_review` + `status=draft`：必须经 chunk review
///   queue 才能流向 cold_contact_worker / knowledge_router 召回，红线复用现有
///   verify 路径，不在本接口开旁路。
/// - lesson `review_status` flip 为 `promoted`，`promoted_chunk_id` 写回字符串
///   形态的新 chunk hex id；幂等：再次 POST 直接返回已有 id，不重复 insert。
pub(super) async fn promote_lesson_to_peer_case(
    State(state): State<AppState>,
    Path(lesson_id): Path<String>,
    Json(payload): Json<PromoteLessonRequest>,
) -> AppResult<Json<Value>> {
    let lesson_id_trim = lesson_id.trim();
    if lesson_id_trim.is_empty() {
        return Err(AppError::BadRequest("lessonId is required".to_string()));
    }
    let (title, body, summary_opt) = validate_promote_request(&payload)?;

    let lessons_coll = state
        .db
        .raw()
        .collection::<Document>("lessons_learned");
    let lesson = lessons_coll
        .find_one(
            doc! {
                "lesson_id": lesson_id_trim,
                "workspace_id": &state.config.default_workspace_id,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("lesson_id={} not found", lesson_id_trim)))?;

    // 幂等：已 promoted 直接返回。
    let existing_chunk_id = lesson
        .get("promoted_chunk_id")
        .and_then(|b| b.as_str().map(str::to_string));
    let existing_status = lesson
        .get_str("review_status")
        .ok()
        .map(str::to_string)
        .unwrap_or_default();
    if existing_status == "promoted" {
        if let Some(chunk_id) = existing_chunk_id {
            return Ok(Json(json!({
                "lessonId": lesson_id_trim,
                "promotedChunkId": chunk_id,
                "alreadyPromoted": true,
            })));
        }
    }

    let pattern_kind = lesson
        .get_str("pattern_kind")
        .ok()
        .map(str::to_string)
        .unwrap_or_default();

    let now = mongodb::bson::DateTime::now();
    let new_id = ObjectId::new();
    let chunk = OperationKnowledgeChunk {
        id: Some(new_id),
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: None,
        document_id: None,
        item_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("peer_case".to_string()),
        business_context: Some(format!("lessons_learned::{pattern_kind}")),
        title,
        summary: summary_opt,
        body: Some(body),
        applicable_scenes: Vec::new(),
        not_applicable_scenes: Vec::new(),
        product_tags: Vec::new(),
        business_topics: Vec::new(),
        source_quote: None,
        source_anchors: Vec::new(),
        // 红线：peer_case 也走 review queue，admin 须在 chunk review 二次确认才能 verify。
        integrity_status: Some("needs_review".to_string()),
        confidence_score: None,
        status: "draft".to_string(),
        priority: 0,
        created_at: now,
        updated_at: now,
        wiki_type: Some("finding".to_string()),
        domain_attributes: None,
        provenance: None,
        valid_from: Some(now),
        valid_to: None,
        superseded_by: None,
        previous_version_id: None,
        related_chunks: None,
        usage_stats: None,
        dynamic_confidence: None,
        integrity_score: None,
        locked_fields: None,
        chunk_type: "peer_case".to_string(),
    };
    state
        .db
        .operation_knowledge_chunks()
        .insert_one(chunk, None)
        .await?;

    // chunk_revisions 留痕同 admin 编辑路径同源；这里走最小化记录：
    // 直接在 lesson 写回 promoted_chunk_id + review_status，chunk 自身的 review
    // queue 会被 admin 在 verify 时再 patch。
    lessons_coll
        .update_one(
            doc! { "lesson_id": lesson_id_trim },
            doc! {
                "$set": {
                    "promoted_chunk_id": new_id.to_hex(),
                    "review_status": "promoted",
                    "updated_at": now,
                },
            },
            None,
        )
        .await?;

    // 写一条 agent_events 审计；与 lessons_learned 是 admin 显式动作的红线对齐。
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: state.config.default_account_id.clone(),
        contact_wxid: None,
        kind: "lesson_promoted_to_peer_case".to_string(),
        status: "ok".to_string(),
        summary: format!(
            "lesson_id={} promoted to peer_case chunk={} (pending chunk review)",
            lesson_id_trim,
            new_id.to_hex()
        ),
        details: Some(doc! {
            "lesson_id": lesson_id_trim,
            "pattern_kind": &pattern_kind,
            "chunk_id": new_id.to_hex(),
            "chunk_type": "peer_case",
            "integrity_status": "needs_review",
        }),
        created_at: now,
    };
    let _ = state.db.events().insert_one(event, None).await;

    Ok(Json(json!({
        "lessonId": lesson_id_trim,
        "promotedChunkId": new_id.to_hex(),
        "alreadyPromoted": false,
    })))
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

    // ── promote-to-peer-case 输入校验 ─────────────────────────────────────

    fn promote_req(title: &str, body: &str, summary: Option<&str>) -> PromoteLessonRequest {
        PromoteLessonRequest {
            title: title.to_string(),
            body: body.to_string(),
            summary: summary.map(str::to_string),
        }
    }

    #[test]
    fn promote_validate_happy_path_trims_and_returns_summary_some() {
        let req = promote_req("  hello  ", "  world  ", Some("  brief  "));
        let (t, b, s) = validate_promote_request(&req).unwrap();
        assert_eq!(t, "hello");
        assert_eq!(b, "world");
        assert_eq!(s.as_deref(), Some("brief"));
    }

    #[test]
    fn promote_validate_empty_summary_collapses_to_none() {
        let req = promote_req("t", "b", Some("   "));
        let (_, _, s) = validate_promote_request(&req).unwrap();
        assert!(s.is_none());
    }

    #[test]
    fn promote_validate_rejects_blank_title() {
        let req = promote_req("   ", "body", None);
        let err = validate_promote_request(&req).unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(msg.contains("title")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn promote_validate_rejects_blank_body() {
        let req = promote_req("title", "", None);
        let err = validate_promote_request(&req).unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(msg.contains("body")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn promote_validate_rejects_title_over_200_chars() {
        let long = "a".repeat(201);
        let req = promote_req(&long, "body", None);
        let err = validate_promote_request(&req).unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(msg.contains("≤ 200")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn promote_validate_rejects_body_over_4000_chars() {
        let long = "b".repeat(4001);
        let req = promote_req("t", &long, None);
        let err = validate_promote_request(&req).unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(msg.contains("≤ 4000")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn promote_validate_chinese_chars_count_by_codepoints() {
        // 200 个汉字 = 200 chars，不应被字节长度误判超限。
        let title: String = "测".repeat(200);
        let req = promote_req(&title, "body", None);
        validate_promote_request(&req).expect("200 chars should be ≤ 200");

        let title_201: String = "测".repeat(201);
        let req = promote_req(&title_201, "body", None);
        assert!(validate_promote_request(&req).is_err());
    }
}

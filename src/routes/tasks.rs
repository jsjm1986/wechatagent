//! Agent 任务路由：跟进任务、Run 日志、LLM 用量等运行时观测。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Bson, DateTime, Document},
    options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentRunQuery {
    account_id: Option<String>,
    contact_wxid: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LlmUsageQuery {
    account_id: Option<String>,
    prompt_key: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskActionRequest {
    pub(crate) expected_account_id: String,
}

impl TaskActionRequest {
    pub(crate) fn for_account(account_id: &str) -> Self {
        Self {
            expected_account_id: account_id.to_string(),
        }
    }

    fn account_id(&self) -> AppResult<&str> {
        let account_id = self.expected_account_id.trim();
        if account_id.is_empty() {
            return Err(AppError::BadRequest(
                "expectedAccountId is required".to_string(),
            ));
        }
        Ok(account_id)
    }
}

pub(super) async fn list_tasks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    // F-003：只展示客户触达类任务（运营视角）；隐藏纯内部后台作业
    // （outcome_aggregation 统计 / memory_consolidation 记忆整理 / initial_profile 画像生成）。
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "kind": { "$in": ["follow_up", "inbound_reply", "principal_decision_relay"] }
            },
            FindOptions::builder()
                .sort(doc! { "run_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(task) = cursor.try_next().await? {
        items.push(json!({
            "id": task.id.map(|id| id.to_hex()).unwrap_or_default(),
            "accountId": task.account_id,
            "contactWxid": task.contact_wxid,
            "kind": task.kind,
            "runAt": crate::models::dt_to_string(task.run_at),
            "expiresAt": task.expires_at.and_then(crate::models::dt_to_string),
            "content": task.content,
            "status": task.status,
            "sourceDecisionId": task.source_decision_id.map(|id| id.to_hex()),
            "reviewRequired": task.review_required,
            "attemptCount": task.attempt_count,
            "maxAttempts": task.max_attempts,
            "nextRetryAt": task.next_retry_at.and_then(crate::models::dt_to_string),
            "gatewayStatus": task.gateway_status,
            "cancelReason": task.cancel_reason,
            "error": task.error
        }));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn list_agent_runs(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AgentRunQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let mut filter = doc! {
        "workspace_id": &admin.current_workspace,
        "account_id": &account_id
    };
    if let Some(contact_wxid) = query.contact_wxid {
        filter.insert("contact_wxid", contact_wxid);
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let mut cursor = state
        .db
        .agent_run_logs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(agent_run_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn list_llm_usage(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<LlmUsageQuery>,
) -> AppResult<Json<Value>> {
    let as_of = DateTime::now();
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    if let Some(account_id) = query.account_id {
        filter.insert("account_id", account_id);
    }
    if let Some(prompt_key) = query.prompt_key {
        filter.insert("prompt_key", prompt_key);
    }
    filter.insert("created_at", doc! { "$lte": as_of });
    let items_limit = query.limit.unwrap_or(100).clamp(1, 300);

    let summary_pipeline = vec![
        doc! { "$match": filter.clone() },
        doc! {
            "$group": {
                "_id": Bson::Null,
                "totalCalls": { "$sum": 1i64 },
                "totalTokens": { "$sum": "$total_tokens" },
                "promptCacheHitTokens": { "$sum": "$prompt_cache_hit_tokens" },
                "promptCacheMissTokens": { "$sum": "$prompt_cache_miss_tokens" },
                "knownUsageCalls": {
                    "$sum": {
                        "$cond": [
                            {
                                "$or": [
                                    { "$eq": ["$usage_known", true] },
                                    { "$eq": ["$status", "cache_hit"] },
                                    { "$ne": [{ "$ifNull": ["$prompt_tokens", 0i64] }, 0i64] },
                                    { "$ne": [{ "$ifNull": ["$completion_tokens", 0i64] }, 0i64] },
                                    { "$ne": [{ "$ifNull": ["$total_tokens", 0i64] }, 0i64] },
                                    { "$ne": [{ "$ifNull": ["$prompt_cache_hit_tokens", 0i64] }, 0i64] },
                                    { "$ne": [{ "$ifNull": ["$prompt_cache_miss_tokens", 0i64] }, 0i64] },
                                ]
                            },
                            1i64,
                            0i64
                        ]
                    }
                },
                "retainedFrom": { "$min": "$created_at" },
            }
        },
    ];
    let mut summary_cursor = state
        .db
        .llm_call_logs()
        .aggregate(summary_pipeline, None)
        .await?;
    let summary_doc = summary_cursor.try_next().await?.unwrap_or_default();
    let total_calls = numeric_i64(&summary_doc, "totalCalls");
    let total_tokens = numeric_i64(&summary_doc, "totalTokens");
    let hit_tokens = numeric_i64(&summary_doc, "promptCacheHitTokens");
    let miss_tokens = numeric_i64(&summary_doc, "promptCacheMissTokens");
    let known_usage_calls = numeric_i64(&summary_doc, "knownUsageCalls");
    let unknown_usage_calls = total_calls.saturating_sub(known_usage_calls);
    let retained_from = summary_doc
        .get_datetime("retainedFrom")
        .ok()
        .copied()
        .and_then(crate::models::dt_to_string);

    let mut cursor = state
        .db
        .llm_call_logs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(items_limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(llm_call_log_json(item));
    }
    let cache_total = hit_tokens + miss_tokens;
    let items_returned = items.len() as i64;
    Ok(Json(json!({
        "asOf": crate::models::dt_to_string(as_of),
        "window": {
            "kind": "retained_logs",
            "start": retained_from,
            "end": crate::models::dt_to_string(as_of),
        },
        "summary": {
            "totalCalls": total_calls,
            "totalTokens": total_tokens,
            "promptCacheHitTokens": hit_tokens,
            "promptCacheMissTokens": miss_tokens,
            "promptCacheHitRate": if cache_total > 0 { hit_tokens as f64 / cache_total as f64 } else { 0.0 },
            "knownUsageCalls": known_usage_calls,
            "unknownUsageCalls": unknown_usage_calls,
            "usageComplete": unknown_usage_calls == 0,
        },
        "itemsReturned": items_returned,
        "itemsLimit": items_limit,
        "itemsTruncated": total_calls > items_returned,
        "items": items
    })))
}

fn numeric_i64(document: &Document, key: &str) -> i64 {
    match document.get(key) {
        Some(Bson::Int32(value)) => i64::from(*value),
        Some(Bson::Int64(value)) => *value,
        Some(Bson::Double(value)) if value.is_finite() => *value as i64,
        _ => 0,
    }
}

#[cfg(test)]
mod llm_usage_tests {
    use super::*;

    #[test]
    fn numeric_i64_accepts_mongo_numeric_shapes() {
        assert_eq!(numeric_i64(&doc! { "n": 7i32 }, "n"), 7);
        assert_eq!(numeric_i64(&doc! { "n": 9i64 }, "n"), 9);
        assert_eq!(numeric_i64(&doc! { "n": 3.0f64 }, "n"), 3);
        assert_eq!(numeric_i64(&doc! { "n": "3" }, "n"), 0);
        assert_eq!(numeric_i64(&Document::new(), "n"), 0);
    }
}

pub async fn review_task_now(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<TaskActionRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let expected_account_id = payload.account_id()?;
    crate::models::assert_agent_task_status_valid("running");
    let Some((task, claim)) = crate::tasks::claim_task_by_id_for_account(
        &state,
        object_id,
        &admin.current_workspace,
        expected_account_id,
    )
    .await?
    else {
        return Err(AppError::Conflict(
            "task_account_mismatch_or_not_claimable".to_string(),
        ));
    };
    crate::tasks::execute_claimed_task(&state, task, &claim).await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn cancel_agent_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<TaskActionRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let expected_account_id = payload.account_id()?;
    crate::models::assert_agent_task_status_valid("cancelled");
    // ReturnDocument::Before gives us the exact decision binding whose task state this CAS
    // invalidated. The task transition happens first, so a concurrent Gateway can no longer
    // commit authorization; then the durable Outbox cancellation closes already-enqueued rows.
    let previous = state
        .db
        .tasks()
        .clone_with_type::<mongodb::bson::Document>()
        .find_one_and_update(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "account_id": expected_account_id,
                "status": { "$in": [
                    "pending", "retry", "failed", "running", "outbox_enqueued"
                ] },
            },
            doc! {
                "$set": {
                    "status": "cancelled",
                    "gateway_status": "admin_cancelled",
                    "cancel_reason": "admin 取消",
                    "updated_at": DateTime::now()
                },
                "$unset": {
                    "claimed_at": "",
                    "claim_token": "",
                    "active_task_key": "",
                    "rerun_requested": "",
                }
            },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::Before)
                .build(),
        )
        .await?;
    let Some(previous) = previous else {
        let exists = state
            .db
            .tasks()
            .count_documents(
                doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
                None,
            )
            .await?
            == 1;
        return Err(if exists {
            AppError::Conflict("task_account_mismatch_or_not_cancelable".to_string())
        } else {
            AppError::NotFound("task not found".to_string())
        });
    };
    let decision_id = previous.get_object_id("outbox_decision_id").ok();
    if previous.get_str("kind").ok() == Some("initial_profile") {
        if let (Ok(account_id), Ok(contact_wxid), Ok(enrollment_token)) = (
            previous.get_str("account_id"),
            previous.get_str("contact_wxid"),
            previous.get_str("enrollment_token"),
        ) {
            let replacement = uuid::Uuid::new_v4().to_string();
            state
                .db
                .contacts()
                .clone_with_type::<mongodb::bson::Document>()
                .update_one(
                    doc! {
                        "workspace_id": &admin.current_workspace,
                        "account_id": account_id,
                        "wxid": contact_wxid,
                        "enrollment_token": enrollment_token,
                    },
                    doc! { "$set": {
                        "enrollment_token": replacement,
                        "updated_at": DateTime::now(),
                    } },
                    None,
                )
                .await?;
        }
    }
    let canceled_outbox = if let Some(decision_id) = decision_id {
        agent::cancel_for_decision(
            &state,
            &admin.current_workspace,
            decision_id,
            "admin_task_cancelled",
        )
        .await?
    } else {
        0
    };
    Ok(Json(
        json!({ "ok": true, "canceledOutbox": canceled_outbox }),
    ))
}

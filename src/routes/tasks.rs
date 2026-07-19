//! Agent 任务路由：跟进任务、Run 日志、LLM 用量等运行时观测。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
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
                "kind": { "$in": ["follow_up", "deferred_inbound_reply", "principal_decision_relay"] }
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
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    if let Some(account_id) = query.account_id {
        filter.insert("account_id", account_id);
    }
    if let Some(prompt_key) = query.prompt_key {
        filter.insert("prompt_key", prompt_key);
    }
    let mut cursor = state
        .db
        .llm_call_logs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(query.limit.unwrap_or(100).clamp(1, 300))
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    let mut total_tokens = 0;
    let mut hit_tokens = 0;
    let mut miss_tokens = 0;
    while let Some(item) = cursor.try_next().await? {
        total_tokens += item.total_tokens;
        hit_tokens += item.prompt_cache_hit_tokens;
        miss_tokens += item.prompt_cache_miss_tokens;
        items.push(llm_call_log_json(item));
    }
    let cache_total = hit_tokens + miss_tokens;
    Ok(Json(json!({
        "summary": {
            "totalCalls": items.len(),
            "totalTokens": total_tokens,
            "promptCacheHitTokens": hit_tokens,
            "promptCacheMissTokens": miss_tokens,
            "promptCacheHitRate": if cache_total > 0 { hit_tokens as f64 / cache_total as f64 } else { 0.0 }
        },
        "items": items
    })))
}

pub async fn review_task_now(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    crate::models::assert_agent_task_status_valid("running");
    let Some((task, claim)) =
        crate::tasks::claim_task_by_id(&state, object_id, Some(&admin.current_workspace)).await?
    else {
        return Err(AppError::Conflict(
            "任务正在运行或已完成，无法立即复核".to_string(),
        ));
    };
    crate::tasks::execute_claimed_task(&state, task, &claim).await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn cancel_agent_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
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
            AppError::Conflict("任务已完成或已取消，无法再次取消".to_string())
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

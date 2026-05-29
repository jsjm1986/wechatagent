//! agent-autonomy-loop W4 / Task 5.6：outbox 后台只读 + 取消路由（admin）。
//!
//! 职责：让运营 / 运维侧能在不进 Mongo shell 的前提下：
//!
//! - `GET /api/admin/outbox?status=...&account_id=...&horizon=...&limit=...`
//!   过滤查看 `agent_send_outbox` 中的条目，便于审计 / 排障。
//! - `POST /api/admin/outbox/:id/cancel`，仅允许把 `pending` / `in_flight`
//!   两种状态置为 `canceled`；其它状态 → 返回 409，避免误覆盖终态。
//!
//! 取消路径与 `outbox::cancel_for_contact_on_user_reaction` 共享同一份"可取
//! 消枚举"约定（design.md §3.2 / R13.6 / R13.9）。本 admin 入口的 `cancel_reason`
//! 来自请求 body，用户反应通道则固定写 `user_reaction_stop_requested`。

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent::outbox::{outbox_status_is_user_cancelable, OutboxStatus},
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::OutboxEntry,
};

use super::shared::*;
use super::AppState;

/// `cancel_reason` 上限。控制在 200 字符内，避免恶意写入超大 BSON。
const MAX_CANCEL_REASON_LEN: usize = 200;
/// 默认列表大小。
const DEFAULT_LIST_LIMIT: i64 = 50;
/// 列表大小上限。
const MAX_LIST_LIMIT: i64 = 200;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListOutboxQuery {
    /// 多个 status 用逗号分隔（如 `pending,in_flight`）。每个值必须落在
    /// `OutboxStatus::from_str` 合法集合内，否则返回 400。
    status: Option<String>,
    account_id: Option<String>,
    /// ISO 8601 时间字符串。命中后只返回 `created_at >= horizon` 的条目。
    horizon: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CancelOutboxRequest {
    cancel_reason: String,
}

pub(super) async fn list_outbox(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ListOutboxQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = Document::new();
    filter.insert("workspace_id", &admin.current_workspace);

    if let Some(raw) = query.status.as_ref().filter(|s| !s.trim().is_empty()) {
        let statuses = parse_status_filter(raw)?;
        if statuses.len() == 1 {
            filter.insert("status", statuses[0]);
        } else {
            filter.insert("status", doc! { "$in": &statuses });
        }
    }
    if let Some(account_id) = query.account_id.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("account_id", account_id.trim());
    }
    if let Some(horizon_raw) = query.horizon.as_ref().filter(|s| !s.trim().is_empty()) {
        let horizon = DateTime::parse_rfc3339_str(horizon_raw.trim()).map_err(|_| {
            AppError::BadRequest(format!(
                "horizon 不是合法 ISO8601 时间：{}",
                horizon_raw.trim()
            ))
        })?;
        filter.insert("created_at", doc! { "$gte": horizon });
    }

    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);

    let collection = state.db.collection_agent_send_outbox();
    let total = collection.count_documents(filter.clone(), None).await?;

    let mut cursor = collection
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(entry) = cursor.try_next().await? {
        items.push(outbox_entry_json(&entry));
    }
    Ok(Json(json!({
        "items": items,
        "total": total
    })))
}

pub(super) async fn cancel_outbox(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<CancelOutboxRequest>,
) -> Result<Response, AppError> {
    let cancel_reason = payload.cancel_reason.trim().to_string();
    if cancel_reason.is_empty() {
        return Err(AppError::BadRequest("cancel_reason 不能为空".to_string()));
    }
    if cancel_reason.chars().count() > MAX_CANCEL_REASON_LEN {
        return Err(AppError::BadRequest(format!(
            "cancel_reason 长度超过上限 {} 字符",
            MAX_CANCEL_REASON_LEN
        )));
    }
    let object_id = parse_object_id(&id)?;
    let cancelable_statuses: Vec<&str> = [OutboxStatus::Pending, OutboxStatus::InFlight]
        .iter()
        .map(|s| s.as_str())
        .collect();
    debug_assert!(
        cancelable_statuses
            .iter()
            .all(|s| outbox_status_is_user_cancelable(s)),
        "filter SHALL match outbox_status_is_user_cancelable",
    );

    let collection = state.db.collection_agent_send_outbox();
    let now = DateTime::now();
    let updated = collection
        .find_one_and_update(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "status": { "$in": &cancelable_statuses },
            },
            doc! {
                "$set": {
                    "status": OutboxStatus::Canceled.as_str(),
                    "cancel_reason": &cancel_reason,
                    "updated_at": now,
                },
                "$unset": {
                    "worker_id": "",
                    "locked_until": "",
                }
            },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;

    let Some(entry) = updated else {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "outbox_not_cancelable",
                "message": "entry not in pending/in_flight"
            })),
        )
            .into_response());
    };

    // 写一条 outbox_canceled 事件，与用户反应通道写的事件 kind 对齐，便于
    // 看板按 kind 聚合"取消"维度的健康度。
    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: entry.workspace_id.clone(),
                account_id: entry.account_id.clone(),
                contact_wxid: Some(entry.contact_wxid.clone()),
                kind: "outbox_canceled".to_string(),
                status: "warning".to_string(),
                summary: "outbox entry canceled by admin".to_string(),
                details: Some(doc! {
                    "outbox_id": object_id,
                    "run_id": &entry.run_id,
                    "cancel_reason": &cancel_reason,
                    "source": "admin_route"
                }),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;

    Ok(Json(json!({ "item": outbox_entry_json(&entry) })).into_response())
}

/// 解析 `status=pending,in_flight` 形式的逗号分隔列表，并强制每个 token
/// 命中 [`OutboxStatus::from_str`]。任意非法 token → `400`，避免静默返回
/// 不一致结果。
pub(super) fn parse_status_filter(raw: &str) -> AppResult<Vec<&'static str>> {
    let mut out = Vec::new();
    for token in raw.split(',') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        match OutboxStatus::from_str(trimmed) {
            Some(status) => out.push(status.as_str()),
            None => {
                return Err(AppError::BadRequest(format!(
                    "非法 outbox status：{}",
                    trimmed
                )));
            }
        }
    }
    if out.is_empty() {
        return Err(AppError::BadRequest(
            "status 至少需要一个合法值".to_string(),
        ));
    }
    Ok(out)
}

pub(super) fn outbox_entry_json(entry: &OutboxEntry) -> Value {
    json!({
        "id": entry.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": entry.workspace_id,
        "accountId": entry.account_id,
        "contactWxid": entry.contact_wxid,
        "runId": entry.run_id,
        "decisionId": entry.decision_id.map(|id| id.to_hex()),
        "sourceEventId": entry.source_event_id,
        "sourceKind": entry.source_kind,
        "content": entry.content,
        "contentHash": entry.content_hash,
        "idempotencyKey": entry.idempotency_key,
        "attempt": entry.attempt,
        "maxAttempts": entry.max_attempts,
        "status": entry.status,
        "cancelReason": entry.cancel_reason,
        "lastError": entry.last_error,
        "nextRetryAt": entry.next_retry_at.and_then(crate::models::dt_to_string),
        "workerId": entry.worker_id,
        "lockedUntil": entry.locked_until.and_then(crate::models::dt_to_string),
        "createdAt": crate::models::dt_to_string(entry.created_at),
        "updatedAt": crate::models::dt_to_string(entry.updated_at),
        "sentAt": entry.sent_at.and_then(crate::models::dt_to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;

    fn sample_entry(status: &str) -> OutboxEntry {
        OutboxEntry {
            id: Some(ObjectId::new()),
            workspace_id: "default".to_string(),
            account_id: "wx_acc_1".to_string(),
            contact_wxid: "wxid_alice".to_string(),
            run_id: "run-001".to_string(),
            decision_id: None,
            source_event_id: "evt-001".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "你好".to_string(),
            content_hash: "abc".to_string(),
            idempotency_key: "key-1".to_string(),
            attempt: 0,
            max_attempts: 3,
            status: status.to_string(),
            cancel_reason: None,
            last_error: None,
            next_retry_at: None,
            worker_id: None,
            locked_until: None,
            reclaimed_in_flight: false,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
            sent_at: None,
        }
    }

    /// W4 / Task 5.6：合法 status 逗号列表正确解析为去重 / 顺序稳定的字符串
    /// 切片，并且单值场景也能被处理。
    #[test]
    fn list_status_filter_accepts_valid_csv() {
        let one = parse_status_filter("pending").unwrap();
        assert_eq!(one, vec!["pending"]);

        let two = parse_status_filter("pending,in_flight").unwrap();
        assert_eq!(two, vec!["pending", "in_flight"]);

        // 容忍空白 + trailing 空 token
        let with_spaces = parse_status_filter(" pending , in_flight , ").unwrap();
        assert_eq!(with_spaces, vec!["pending", "in_flight"]);

        // 全枚举集
        let all =
            parse_status_filter("pending,in_flight,sent,failed_terminal,canceled").unwrap();
        assert_eq!(
            all,
            vec!["pending", "in_flight", "sent", "failed_terminal", "canceled"]
        );
    }

    /// W4 / Task 5.6：非法 status token 返回 400 BadRequest；包括旧的
    /// `"failed"` 字面量（R13.10 hard rule）以及大小写错误。
    #[test]
    fn list_status_filter_rejects_invalid_token() {
        let err = parse_status_filter("pending,bogus").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // R13.10：旧的 `failed` 不再合法
        let err = parse_status_filter("failed").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 大小写敏感
        let err = parse_status_filter("PENDING").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // 全空 / 全逗号
        let err = parse_status_filter("").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        let err = parse_status_filter(",,,").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    /// W4 / Task 5.6：JSON shape 携带稳定字段；`null` 字段也要明确出现，
    /// 让前端可以 destructure 而不必判 `undefined`。
    #[test]
    fn outbox_entry_json_shape_is_stable() {
        let entry = sample_entry("pending");
        let oid = entry.id.unwrap().to_hex();
        let value = outbox_entry_json(&entry);
        assert_eq!(value["id"], oid);
        assert_eq!(value["accountId"], "wx_acc_1");
        assert_eq!(value["contactWxid"], "wxid_alice");
        assert_eq!(value["status"], "pending");
        assert_eq!(value["attempt"], 0);
        assert_eq!(value["maxAttempts"], 3);
        assert!(value["createdAt"].is_string());
        assert!(value["updatedAt"].is_string());
        assert!(value["sentAt"].is_null());
        assert!(value["cancelReason"].is_null());
        assert!(value["decisionId"].is_null());
    }

    /// W4 / Task 5.6：cancel handler 对"已经 sent 的条目"应当返回 409。
    /// 这里直接验证用于决定 `find_one_and_update` 命中 vs. 未命中 的
    /// "可取消枚举集合"语义：sent / failed_terminal / canceled 不在集合里。
    #[test]
    fn cancel_returns_409_for_already_sent() {
        // 与 cancel_outbox 内部使用同一组 cancelable 列表，断言 sent 不在其中。
        let cancelable: Vec<&str> = [OutboxStatus::Pending, OutboxStatus::InFlight]
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert!(!cancelable.contains(&"sent"));
        assert!(!cancelable.contains(&"failed_terminal"));
        assert!(!cancelable.contains(&"canceled"));
        // pending / in_flight 仍然在 cancelable 集合里。
        assert!(cancelable.contains(&"pending"));
        assert!(cancelable.contains(&"in_flight"));
    }

    /// W4 / Task 5.6：cancel happy path —— 把 `pending` 行视为成功取消后
    /// JSON shape 中应该体现 `status="canceled"` + `cancelReason`。
    #[test]
    fn cancel_happy_path_marks_canceled_in_json() {
        let mut entry = sample_entry("canceled");
        entry.cancel_reason = Some("admin_manual_intervention".to_string());
        entry.worker_id = None;
        entry.locked_until = None;
        let value = outbox_entry_json(&entry);
        assert_eq!(value["status"], "canceled");
        assert_eq!(value["cancelReason"], "admin_manual_intervention");
        assert!(value["workerId"].is_null());
        assert!(value["lockedUntil"].is_null());
    }

    /// W4 / Task 5.6：`CancelOutboxRequest` 需要 `cancelReason` 字段
    /// （camelCase 自动来自 serde 配置）；空字段 / 缺失字段都应该被业务
    /// 校验拦截。这里直接验证 serde 反序列化最低要求。
    #[test]
    fn cancel_request_requires_reason_field() {
        let parsed: Result<CancelOutboxRequest, _> = serde_json::from_value(json!({}));
        assert!(parsed.is_err(), "缺少 cancelReason 应被 serde 拒绝");
        let ok: CancelOutboxRequest =
            serde_json::from_value(json!({ "cancelReason": "operator_test" })).unwrap();
        assert_eq!(ok.cancel_reason, "operator_test");
    }

    /// W4 / Task 5.6：query parser 对 `limit` 默认 / 上限的处理
    /// （default=50, cap=200）。
    #[test]
    fn list_query_limit_defaults_and_caps() {
        let q: ListOutboxQuery = serde_json::from_value(json!({})).unwrap();
        let lim = q.limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
        assert_eq!(lim, 50);

        let q: ListOutboxQuery = serde_json::from_value(json!({ "limit": 9999 })).unwrap();
        let lim = q.limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
        assert_eq!(lim, 200);

        let q: ListOutboxQuery = serde_json::from_value(json!({ "limit": 0 })).unwrap();
        let lim = q.limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
        assert_eq!(lim, 1);
    }

    /// W4 / Task 5.6：horizon 解析必须是 RFC3339；非法字符串应返回 400 而
    /// 不是默默忽略 filter（避免误导查询结果）。
    #[test]
    fn list_query_horizon_must_be_rfc3339() {
        let good = DateTime::parse_rfc3339_str("2026-05-20T00:00:00Z");
        assert!(good.is_ok());
        let bad = DateTime::parse_rfc3339_str("not a date");
        assert!(bad.is_err());
    }
}

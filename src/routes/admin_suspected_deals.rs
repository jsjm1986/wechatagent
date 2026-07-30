//! F23：疑似成交待核实闭环（方案B 全链）的运营核实路由（admin）。
//!
//! 职责：列表 / approve / reject `suspected_deal_signals` 记录——LLM 决策时若判断
//! 客户疑似成交，会在 `agent_generated_signals` 输出 `kind="suspected_deal"` 弱信号，
//! gateway 把它 upsert 进待核实专表（`status=pending`，见 gateway.rs）。
//!
//! **红线：AI 永不直写 `outcome_events`**。疑似成交信号只进待核实队列，**只有运营
//! approve 才在事务内提交已校验的正式成交**，且 `verification` 强制
//! `Some("staff_confirmed")`——AI 侧的 `conversation_inferred` 疑似线索绝不直登成交。
//!
//! - `GET /api/admin/suspected-deals?status=pending`
//!     列待核实信号；按 status 过滤（默认 pending），**强制 workspace 隔离**。
//! - `POST /api/admin/suspected-deals/:id/approve`
//!     行为（**validate-first + transaction**）：
//!       1. workspace 隔离读取 signal/contact，校验金额、币种、产品并冻结成交快照；
//!          任一错误均保持 signal=pending，可修正后重试。
//!       2. Mongo transaction 内以 `status:"pending"` CAS 成 approved，同时 append
//!          contact.outcome_events 与 agent_events 审计；任一步失败全部回滚。
//! - `POST /api/admin/suspected-deals/:id/reject`
//!     body: `{ reason }` —— 写 `rejection_reason` 并 `status="rejected"`。
//!
//! 该路径要求 MongoDB replica set（项目其它发布/激活事务同一部署前提）。重复 approve
//! 在事务 CAS 处冲突，不会重复 append；提交结果不确定时按 Mongo 推荐规则重试 commit。

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};
use mongodb::options::{FindOptions, TransactionOptions};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::SuspectedDealSignal,
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSuspectedDealsQuery {
    /// 默认只看 `pending`；前端可显式传 `approved` / `rejected` / `all` 看历史。
    status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveSuspectedDealRequest {
    /// 可选：成交金额（最小币种单位整数，如分）。
    #[serde(default)]
    amount: Option<i64>,
    /// 可选：ISO-4217 三位大写币种码（如 CNY）。
    #[serde(default)]
    currency: Option<String>,
    /// 可选：成交关联的 product_id。
    #[serde(default)]
    product_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectSuspectedDealRequest {
    reason: String,
}

pub async fn list_suspected_deals(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ListSuspectedDealsQuery>,
) -> AppResult<Json<Value>> {
    // workspace 隔离：信号是 contact 级、带 workspace_id，必须按当前登录态 workspace
    // 过滤，绝不跨 workspace 暴露他人待核实信号。
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("pending");
    if status != "all" {
        filter.insert("status", status);
    }

    let mut cursor = state
        .db
        .collection_suspected_deal_signals()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "last_seen_at": -1 })
                .limit(500)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(suspected_deal_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub async fn approve_suspected_deal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ApproveSuspectedDealRequest>,
) -> Result<Response, AppError> {
    let value = approve_suspected_deal_inner(&state, &admin, &id, payload).await?;
    Ok(Json(value).into_response())
}

/// approve_suspected_deal 的内部核心：先完成所有可预检条件，再在 transaction 中
/// 原子提交 pending CAS + 正式成交 + 审计。
/// 跨 workspace / 不存在 / 已非 pending 的 _id 返 NotFound（不泄漏存在性）。
async fn approve_suspected_deal_inner(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    id: &str,
    payload: ApproveSuspectedDealRequest,
) -> AppResult<Value> {
    let workspace_id = &admin.current_workspace;
    let object_id = parse_object_id(id)?;
    let signals = state.db.collection_suspected_deal_signals();
    // 事务前读取仅用于完成全部业务校验；最终 pending 状态以下方事务 CAS 为准。
    let mut signal = signals
        .find_one(
            doc! { "_id": object_id, "workspace_id": workspace_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("suspected deal signal not found".to_string()))?;

    let reviewer = ReviewActor::from_admin(admin)?;

    // validate-first：联系人、金额、币种、产品归属及产品快照全部在任何状态写入前完成。
    let contact = find_contact_by_id(state, workspace_id, &signal.contact_id).await?;
    let prepared = prepare_outcome_event(
        state,
        &contact,
        OutcomeEventInput {
            source: "manual".to_string(),
            marked_by: reviewer.as_str().to_string(),
            audit_summary: "疑似成交运营核实确认".to_string(),
            amount: payload.amount,
            currency: payload.currency,
            verification: Some("staff_confirmed".to_string()),
            event_kind: Some("deal".to_string()),
            product_id: payload.product_id,
            quantity: None,
            note: None,
            occurred_at_ms: None,
        },
    )
    .await?;

    let now = DateTime::now();
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let transaction_result: AppResult<()> = async {
        let cas = signals
            .update_one_with_session(
                doc! {
                    "_id": object_id,
                    "workspace_id": workspace_id,
                    "status": "pending",
                },
                doc! {
                    "$set": {
                        "status": "approved",
                        "reviewed_at": now,
                        "reviewed_by": reviewer.as_str(),
                    }
                },
                None,
                &mut session,
            )
            .await?;
        if cas.modified_count != 1 {
            return Err(AppError::Conflict("suspected_deal_not_pending".to_string()));
        }
        persist_prepared_outcome_event_with_session(state, &prepared, &mut session).await?;
        Ok(())
    }
    .await;
    if let Err(error) = transaction_result {
        let _ = session.abort_transaction().await;
        return Err(match error {
            AppError::Db(db_error) => {
                tracing::warn!(error = %db_error, "suspected deal approval transaction conflicted");
                AppError::Conflict("suspected_deal_approval_conflict".to_string())
            }
            other => other,
        });
    }
    loop {
        match session.commit_transaction().await {
            Ok(()) => break,
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => {
                tracing::warn!(error = %error, "suspected deal approval commit failed");
                return Err(AppError::Conflict(
                    "suspected_deal_approval_conflict".to_string(),
                ));
            }
        }
    }

    signal.status = "approved".to_string();
    signal.reviewed_at = Some(now);
    signal.reviewed_by = Some(reviewer.as_str().to_string());
    Ok(json!({ "item": suspected_deal_json(signal) }))
}

pub async fn reject_suspected_deal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<RejectSuspectedDealRequest>,
) -> AppResult<Json<Value>> {
    if payload.reason.trim().is_empty() {
        return Err(AppError::BadRequest("reason 不能为空".to_string()));
    }
    let object_id = parse_object_id(&id)?;
    let signals = state.db.collection_suspected_deal_signals();
    let reviewer = ReviewActor::from_admin(&admin)?;
    let now = DateTime::now();
    let result = signals
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "status": "pending"
            },
            doc! {
                "$set": {
                    "status": "rejected",
                    "reviewed_at": now,
                    "reviewed_by": reviewer.as_str(),
                    "rejection_reason": payload.reason.trim()
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "suspected deal signal not found or not pending".to_string(),
        ));
    }
    let updated = signals
        .find_one(doc! { "_id": object_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("suspected deal signal not found".to_string()))?;
    Ok(Json(json!({ "item": suspected_deal_json(updated) })))
}

pub fn suspected_deal_json(item: SuspectedDealSignal) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "contactId": item.contact_id,
        "value": item.value,
        "evidence": item.evidence,
        "confidence": item.confidence,
        "occurrences": item.occurrences,
        "status": item.status,
        "firstSeenAt": crate::models::dt_to_string(item.first_seen_at),
        "lastSeenAt": crate::models::dt_to_string(item.last_seen_at),
        "reviewedAt": item.reviewed_at.and_then(crate::models::dt_to_string),
        "reviewedBy": item.reviewed_by
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;

    fn sample_signal(status: &str) -> SuspectedDealSignal {
        SuspectedDealSignal {
            id: Some(ObjectId::new()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_id: "507f1f77bcf86cd799439011".to_string(),
            value: "疑似成交·待核实".to_string(),
            evidence: Some("客户说要下单".to_string()),
            confidence: 75,
            status: status.to_string(),
            occurrences: 2,
            first_seen_at: DateTime::now(),
            last_seen_at: DateTime::now(),
            reviewed_at: None,
            reviewed_by: None,
        }
    }

    /// F23：信号 JSON 形状稳定（camelCase 输出）。
    #[test]
    fn suspected_deal_json_shape_is_stable() {
        let s = sample_signal("pending");
        let id_hex = s.id.unwrap().to_hex();
        let value = suspected_deal_json(s);
        assert_eq!(value["id"], id_hex);
        assert_eq!(value["workspaceId"], "ws-1");
        assert_eq!(value["accountId"], "acc-1");
        assert_eq!(value["contactId"], "507f1f77bcf86cd799439011");
        assert_eq!(value["value"], "疑似成交·待核实");
        assert_eq!(value["evidence"], "客户说要下单");
        assert_eq!(value["confidence"], 75);
        assert_eq!(value["occurrences"], 2);
        assert_eq!(value["status"], "pending");
        assert!(value["firstSeenAt"].is_string());
        assert!(value["lastSeenAt"].is_string());
        assert!(value["reviewedAt"].is_null());
    }

    /// F23：默认 list query 不传 status 时 handler 内部解析为 "pending"。
    #[test]
    fn list_query_defaults_to_pending() {
        let q: ListSuspectedDealsQuery = serde_json::from_value(json!({})).unwrap();
        let resolved = q
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("pending");
        assert_eq!(resolved, "pending");
    }

    /// F23：approve 请求所有字段可缺省（serde default）。
    #[test]
    fn approve_request_fields_optional() {
        let req: ApproveSuspectedDealRequest = serde_json::from_value(json!({})).unwrap();
        assert!(req.amount.is_none());
        assert!(req.currency.is_none());
        assert!(req.product_id.is_none());
        let req2: ApproveSuspectedDealRequest = serde_json::from_value(
            json!({ "reviewedBy": "alice@corp", "amount": 9900, "currency": "CNY", "productId": "p-1" }),
        )
        .unwrap();
        assert_eq!(req2.amount, Some(9900));
        assert_eq!(req2.currency.as_deref(), Some("CNY"));
        assert_eq!(req2.product_id.as_deref(), Some("p-1"));
    }

    /// F23：reject 请求要求 `reason` 字段（serde 默认 missing 报错）。
    #[test]
    fn reject_request_requires_reason() {
        let parsed: Result<RejectSuspectedDealRequest, _> = serde_json::from_value(json!({}));
        assert!(parsed.is_err(), "缺少 reason 应该被 serde 拒绝");
        let ok: RejectSuspectedDealRequest =
            serde_json::from_value(json!({ "reason": "误判，实际只是咨询" })).unwrap();
        assert_eq!(ok.reason, "误判，实际只是咨询");
    }

    /// 契约快照:suspected_deal_json。SuspectedDealSignal 13 字段全量构造(evidence/reviewed_at/
    /// reviewed_by 给 Some,reviewedAt 非 null);id→Option.map(to_hex).unwrap_or_default();
    /// contact_id 是 String 直发;first_seen_at/last_seen_at→dt_to_string,reviewed_at→and_then。
    /// 投影 1:1 下发 13 顶层键。
    #[test]
    fn suspected_deal_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let item = SuspectedDealSignal {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_id: "wxid_alice".to_string(),
            value: "疑似成交·待核实".to_string(),
            evidence: Some("客户已确认付款意向".to_string()),
            confidence: 8,
            status: "pending".to_string(),
            occurrences: 3,
            first_seen_at: DateTime::from_millis(1_700_000_000_000),
            last_seen_at: DateTime::from_millis(1_700_000_100_000),
            reviewed_at: Some(DateTime::from_millis(1_700_000_200_000)),
            reviewed_by: Some("admin-1".to_string()),
        };
        let value = suspected_deal_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("suspected_deal", value);
    }
}

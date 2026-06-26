//! F23：疑似成交待核实闭环（方案B 全链）的运营核实路由（admin）。
//!
//! 职责：列表 / approve / reject `suspected_deal_signals` 记录——LLM 决策时若判断
//! 客户疑似成交，会在 `agent_generated_signals` 输出 `kind="suspected_deal"` 弱信号，
//! gateway 把它 upsert 进待核实专表（`status=pending`，见 gateway.rs）。
//!
//! **红线：AI 永不直写 `outcome_events`**。疑似成交信号只进待核实队列，**只有运营
//! approve 才调 [`add_outcome_event_inner`] 落正式成交**，且 `verification` 强制
//! `Some("staff_confirmed")`——AI 侧的 `conversation_inferred` 疑似线索绝不直登成交。
//!
//! - `GET /api/admin/suspected-deals?status=pending`
//!     列待核实信号；按 status 过滤（默认 pending），**强制 workspace 隔离**。
//! - `POST /api/admin/suspected-deals/:id/approve`
//!     行为（**CAS-first**，财务安全顺序）：
//!       1. 读 signal（属当前 workspace），仅为拿 contact_id。
//!       2. **原子 CAS 占位**：`update_one(filter:{_id, workspace_id, status:"pending"},
//!          $set:{status:"approved", reviewed_at, reviewed_by})`；`matched_count==0`
//!          （并发/已审/跨 workspace）→ NotFound，**绝不落成交**。
//!       3. CAS 成功后才 `find_contact_by_id`（workspace 隔离）+ **落正式成交**：
//!          `add_outcome_event_inner(verification=staff_confirmed, source="manual",
//!          event_kind="deal", marked_by=<admin>)`。
//! - `POST /api/admin/suspected-deals/:id/reject`
//!     body: `{ reason }` —— 写 `rejection_reason` 并 `status="rejected"`。
//!
//! 注意：approve 涉及「改信号状态 + 落成交」两步，MongoDB 单机部署不支持事务。
//! `outcome_events` 是 append-only 无 dedup，故这里采用 **CAS-first**：先原子地把信号
//! 从 pending 占位改 approved，再落成交。重复 approve 在 CAS 步（status 已非 pending →
//! matched==0）即被挡，根治财务双计；代价是「CAS 成功但落成交失败 → 已 approved 但未落
//! 成交」的漏登假阴——可由运营走 add_deal_event 手动补登，对 append-only 财务远比双计可接受。

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};
use mongodb::options::FindOptions;
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
    /// 可选：操作人标识（一般是 admin email / id），落入 `reviewed_by`。
    #[serde(default)]
    reviewed_by: Option<String>,
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
    #[serde(default)]
    reviewed_by: Option<String>,
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

/// approve_suspected_deal 的内部核心（**CAS-first**）：workspace 隔离读 signal（仅为
/// 拿 contact_id）→ **原子 CAS 把信号从 pending 占位改 approved** → CAS 成功后才落正式
/// 成交（verification=staff_confirmed）→ 返回 `{"item": <signal json>}`。
/// 跨 workspace / 不存在 / 已非 pending 的 _id 返 NotFound（不泄漏存在性）。
///
/// **为什么先 CAS 再落成交**：`outcome_events` 是 append-only 无 dedup。若反过来「先落成交
/// 再改状态」，步间崩溃会留下 pending 信号 → 重试 approve 再 append → 财务双计。CAS-first
/// 让重复 approve 在第 1 步 CAS（status 已非 pending → matched==0）就被挡，根本到不了落成交，
/// 把「双计假阳」换成「CAS 成功但落成交失败 → 已 approved 但未落成交」的「漏登假阴」——漏登
/// 可由运营走 add_deal_event 手动补登，对 append-only 财务数据远比双计可接受。
async fn approve_suspected_deal_inner(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    id: &str,
    payload: ApproveSuspectedDealRequest,
) -> AppResult<Value> {
    let workspace_id = &admin.current_workspace;
    let object_id = parse_object_id(id)?;
    let signals = state.db.collection_suspected_deal_signals();
    // 读 signal 仅为拿 contact_id（落成交需要）；pending 校验以下方 CAS 的 matched_count
    // 为准（防 TOCTOU）。查询带 workspace 过滤：跨 workspace 的 _id 返回 NotFound。
    let signal = signals
        .find_one(doc! { "_id": object_id, "workspace_id": workspace_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("suspected deal signal not found".to_string()))?;

    let reviewer = payload
        .reviewed_by
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| admin.username.clone());

    // 第 1 步（CAS 占位，先于落成交）：原子地把信号从 pending 改 approved。filter 带
    // status:"pending" + workspace_id —— matched==0 说明并发/已审/跨 workspace，此时
    // **绝不落成交**直接返回，重复 approve 在此被挡，根治 append-only 财务双计。
    let now = DateTime::now();
    let cas = signals
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": workspace_id,
                "status": "pending"
            },
            doc! {
                "$set": {
                    "status": "approved",
                    "reviewed_at": now,
                    "reviewed_by": &reviewer
                }
            },
            None,
        )
        .await?;
    if cas.matched_count == 0 {
        return Err(AppError::NotFound(
            "suspected deal signal not found or not pending".to_string(),
        ));
    }

    // 第 2 步（CAS 成功后才执行）：workspace 隔离取 contact + 落正式成交。**红线**：
    // verification 强制 staff_confirmed——疑似线索经人审核实后才落成交，AI 永不直写 outcome。
    // 若此步失败 → 信号已 approved 但未落成交（漏登假阴，运营可手动补登），不会双计。
    let contact = find_contact_by_id(state, workspace_id, &signal.contact_id).await?;
    add_outcome_event_inner(
        state,
        &contact,
        OutcomeEventInput {
            source: "manual".to_string(),
            marked_by: reviewer.clone(),
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

    let updated = signals
        .find_one(doc! { "_id": object_id, "workspace_id": workspace_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("suspected deal signal not found".to_string()))?;
    Ok(json!({ "item": suspected_deal_json(updated) }))
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
                    "reviewed_by": payload.reviewed_by.as_deref().unwrap_or("admin"),
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
        assert!(req.reviewed_by.is_none());
        assert!(req.amount.is_none());
        assert!(req.currency.is_none());
        assert!(req.product_id.is_none());
        let req2: ApproveSuspectedDealRequest = serde_json::from_value(
            json!({ "reviewedBy": "alice@corp", "amount": 9900, "currency": "CNY", "productId": "p-1" }),
        )
        .unwrap();
        assert_eq!(req2.reviewed_by.as_deref(), Some("alice@corp"));
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
}

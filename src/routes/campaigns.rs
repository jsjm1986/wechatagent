//! 活动定向推送引擎：segment 圈人（两阶段）+ 活动生命周期。
//!
//! 生命周期路由（挂在 `/api` 下）：
//! - `POST /campaigns`              新建活动（status=draft）
//! - `POST /campaigns/:id/preview`  圈人预览（回 targetCount + 抽样，status→previewed）
//! - `POST /campaigns/:id/dispatch` 确认推送（重新圈人 + 扇出 follow_up 任务，status→completed）
//!
//! 扇出只批量建 `kind="follow_up"` 的 [`AgentTask`]，发送链路（task worker→gateway→
//! outbox→MCP）完全复用。活动级去重靠 `campaign_sends` 唯一索引 (campaignId, contactWxid)。
//! IDOR 红线：所有 campaigns filter 含 `workspaceId = admin.current_workspace`。
use crate::agent::entitlements;
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::models::{
    assert_agent_task_status_valid, assert_campaign_status_valid, AgentRunLog, AgentTask, Campaign,
    CampaignSend, Contact, Product, SegmentFilter,
};
use axum::extract::{Path, State};
use axum::{Extension, Json};
use futures::TryStreamExt;
use mongodb::bson::oid::ObjectId;
use mongodb::bson::{doc, DateTime, Document};
use serde::Deserialize;
use serde_json::{json, Value};

use super::AppState;
use crate::agent::run_envelope::SOURCE_KIND_FOLLOW_UP_TASK;

/// 阶段1：Mongo 粗筛 filter。命中 outcome_events.productRef.productId 索引。
/// product_ids 非空时用 $elemMatch 同元素匹配「买过指定产品 + 高可信 + 正向成交」。
pub(super) fn build_segment_coarse_filter(
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
) -> Document {
    let mut d = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "agent_status": "managed",
    };
    // 客户阶段裸字段在粗筛层 filter（domain_attributes.customer_stage 真实路径，
    // 已对 contacts.rs:786 `domain_attributes.get_str("customer_stage")` 核实一致）。
    if let Some(stage) = &filter.customer_stage {
        d.insert("domain_attributes.customer_stage", stage);
    }
    // 产品反查：$elemMatch 同一成交事件内匹配「指定产品 + 高可信 + 非退款」。
    // KC-05：verification/eventKind 的 serde 默认(staff_confirmed/deal)只在反序列化补、
    // Mongo 查询不补，缺这两字段的老成交(§4.5 上线前登记)会被精确匹配漏掉→product 定向
    // 静默漏老客户。故把"缺字段=默认值"显式写进查询：
    // - verification：白名单命中 或 字段缺失(老文档=staff_confirmed)。$elemMatch 内多键是
    //   隐式 AND，字段级"或缺失"须用 $and 包裹(顶层 $or 不能做字段级)。
    // - eventKind：$ne:"reversal" 一箭双雕——缺字段(missing ≠ reversal)与显式"deal"都命中，
    //   只排退款；同时与精筛口径对齐(精筛不按 kind 排除、只对 reversal 抵消件数)。
    if !filter.product_ids.is_empty() {
        d.insert(
            "outcome_events",
            doc! { "$elemMatch": {
                "productRef.productId": { "$in": &filter.product_ids },
                "$and": [
                    { "$or": [
                        { "verification": { "$in": ["staff_confirmed", "payment_verified"] } },
                        { "verification": { "$exists": false } },
                    ]},
                    { "eventKind": { "$ne": "reversal" } },
                ],
            }},
        );
    }
    d
}

/// 阶段2：内存精筛。复用 G4 纯函数判净持有/售后/价值分层。
pub(super) fn contact_matches_segment(
    contact: &Contact,
    active_products: &[Product],
    filter: &SegmentFilter,
    now: DateTime,
    mid_threshold: i64,
    high_threshold: i64,
) -> bool {
    // 复用 G4 投影：净持有（退款抵消、净件数>0）。
    let (entitlements, _) = entitlements::project_entitlements(
        &contact.outcome_events,
        active_products,
        now,
        usize::MAX,
    );
    // 产品维度：要求净持有指定产品之一。
    if !filter.product_ids.is_empty() {
        let holds = entitlements
            .iter()
            .any(|e| filter.product_ids.contains(&e.product_id));
        if !holds {
            return false;
        }
    }
    // 售后维度。
    if let Some(aftercare) = filter.aftercare.as_deref() {
        match aftercare {
            "in_aftercare" => {
                if !entitlements.iter().any(|e| e.in_aftercare == Some(true)) {
                    return false;
                }
            }
            "expired" => {
                if !entitlements.iter().any(|e| e.in_aftercare == Some(false)) {
                    return false;
                }
            }
            _ => {} // "any" 或未知：不约束
        }
    }
    // 价值分层维度。
    if let Some(tier) = filter.value_tier.as_deref() {
        let value = entitlements::compute_customer_value_cents(&contact.outcome_events);
        let actual = entitlements::classify_value_tier(value, mid_threshold, high_threshold);
        if actual != tier {
            return false;
        }
    }
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCampaignRequest {
    pub title: String,
    pub intent_text: String,
    #[serde(default)]
    pub segment_filter: SegmentFilter,
    /// 目标账号；缺省回落 default_account_id（与 list_contacts / list_tasks 同模式，
    /// workspace_id + account_id 组合过滤即隔离，不额外校验账号归属）。
    #[serde(default)]
    pub account_id: Option<String>,
}

/// 自建活动 follow_up 任务（不调 planner 私有 emit_planner_follow_up；
/// 形态对齐 management.rs:1461 create_follow_up_task）。
pub(super) fn build_campaign_follow_up_task(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    intent_text: &str,
    now: DateTime,
) -> AgentTask {
    let expires_at = DateTime::from_millis(now.timestamp_millis() + 48 * 60 * 60 * 1000);
    AgentTask {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: contact_wxid.to_string(),
        kind: "follow_up".to_string(),
        run_at: now,
        expires_at: Some(expires_at),
        content: intent_text.to_string(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: true,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    }
}

/// value_tier 阈值来源：复用 G6 classify_value_tier 调用方（gateway.rs:3756-3759）的
/// 同一对 config 阈值，不新造配置字段。
pub(super) fn value_tier_thresholds(config: &crate::config::AppConfig) -> (i64, i64) {
    (
        config.value_tier_mid_threshold_cents,
        config.value_tier_high_threshold_cents,
    )
}

/// DuplicateKey 判定（仿 products.rs:329 is_duplicate_key）。
fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    matches!(
        *err.kind,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(ref e)) if e.code == 11000
    )
}

/// 跑两阶段圈人，返回命中的 contacts。粗筛 Mongo + 内存精筛复用 G4。
///
/// KC-04/07：受众规模硬上限（粗筛扫描量）。cursor `.limit(max_audience+1)` 在 Mongo
/// 层截断扫描量（防全量 contacts 驻内存）；循环内 `coarse_count > max_audience` 报错
/// （防 limit 静默截断受众——运营会误以为圈到的就是全部）。上限加在粗筛层而非精筛后，
/// 治的正是"全量驻内存 + dispatch 串行千次 DB 写超时"的根因。preview/dispatch 共用。
async fn resolve_segment_contacts(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
    max_audience: i64,
) -> AppResult<Vec<Contact>> {
    let coarse = build_segment_coarse_filter(workspace_id, account_id, filter);
    let opts = mongodb::options::FindOptions::builder()
        .limit(max_audience + 1)
        .build();
    let mut cursor = state.db.contacts().find(coarse, opts).await?;
    let active_products: Vec<Product> =
        entitlements::load_active_products(&state.db, workspace_id).await;
    let now = DateTime::now();
    let (mid, high) = value_tier_thresholds(&state.config);
    let mut coarse_count = 0i64;
    let mut hits = Vec::new();
    while let Some(c) = cursor.try_next().await? {
        coarse_count += 1;
        if coarse_count > max_audience {
            return Err(AppError::BadRequest(format!(
                "受众粗筛候选超过 {max_audience} 人，请细化圈选条件（产品/阶段/价值分层）后重试"
            )));
        }
        if contact_matches_segment(&c, &active_products, filter, now, mid, high) {
            hits.push(c);
        }
    }
    Ok(hits)
}

pub async fn create_campaign(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<CreateCampaignRequest>,
) -> AppResult<Json<Value>> {
    if body.title.trim().is_empty() || body.intent_text.trim().is_empty() {
        return Err(AppError::BadRequest(
            "title 与 intentText 不能为空".to_string(),
        ));
    }
    let now = DateTime::now();
    assert_campaign_status_valid("draft");
    let account_id = body
        .account_id
        .filter(|a| !a.trim().is_empty())
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let campaign = Campaign {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id,
        title: body.title.trim().to_string(),
        intent_text: body.intent_text.trim().to_string(),
        segment_filter: body.segment_filter,
        status: "draft".to_string(),
        target_count: None,
        dispatched_count: 0,
        created_by: admin.username.clone(),
        created_at: now,
        updated_at: now,
    };
    let res = state.db.campaigns().insert_one(&campaign, None).await?;
    Ok(Json(json!({
        "id": res.inserted_id.as_object_id().map(|i| i.to_hex()),
        "status": "draft"
    })))
}

pub async fn preview_campaign(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("非法 campaign id".to_string()))?;
    let campaign = state
        .db
        .campaigns()
        .find_one(
            doc! { "_id": oid, "workspaceId": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("campaign not found".to_string()))?;
    let hits = resolve_segment_contacts(
        &state,
        &campaign.workspace_id,
        &campaign.account_id,
        &campaign.segment_filter,
        state.config.campaign_max_audience,
    )
    .await?;
    let target = hits.len() as i64;
    // 抽样最多 5 个示例（名/wxid）。
    let samples: Vec<Value> = hits
        .iter()
        .take(5)
        .map(|c| {
            json!({
                "wxid": c.wxid,
                "name": c.remark.clone().or(c.nickname.clone()).unwrap_or_default(),
            })
        })
        .collect();
    assert_campaign_status_valid("previewed");
    state
        .db
        .campaigns()
        .update_one(
            doc! { "_id": oid, "workspaceId": &admin.current_workspace },
            doc! { "$set": { "status": "previewed", "targetCount": target, "updatedAt": DateTime::now() } },
            None,
        )
        .await?;
    Ok(Json(json!({
        "campaignId": id,
        "intentText": campaign.intent_text,
        "targetCount": target,
        "samples": samples,
    })))
}

/// KC-02：仅这些 status 允许 dispatch。dispatching = 允许重入恢复（配合补偿回滚，
/// 已完成的 send 撞去重跳过、失败/剩余 contact 重建）；completed = 拒绝（防重复推送）；
/// 未知态 = 拒绝（fail-safe）。
pub(super) fn dispatch_allowed_from_status(status: &str) -> bool {
    matches!(status, "draft" | "previewed" | "dispatching")
}

pub async fn dispatch_campaign(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("非法 campaign id".to_string()))?;
    let campaign = state
        .db
        .campaigns()
        .find_one(
            doc! { "_id": oid, "workspaceId": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("campaign not found".to_string()))?;
    // KC-02：置 dispatching 前先校验当前 status——completed/未知态拒绝重推，
    // draft/previewed/dispatching 放行（dispatching 支持后续补偿回滚的重入恢复）。
    if !dispatch_allowed_from_status(&campaign.status) {
        return Err(AppError::BadRequest(format!(
            "当前活动状态 {} 不可派发（仅 draft/previewed/dispatching 可派发；completed 需另建活动）",
            campaign.status
        )));
    }
    // 重新跑圈人（防预览后数据漂移）。
    let hits = resolve_segment_contacts(
        &state,
        &campaign.workspace_id,
        &campaign.account_id,
        &campaign.segment_filter,
        state.config.campaign_max_audience,
    )
    .await?;
    if hits.is_empty() {
        return Err(AppError::BadRequest("命中 0 人，无可推送对象".to_string()));
    }
    assert_campaign_status_valid("dispatching");
    state
        .db
        .campaigns()
        .update_one(
            doc! { "_id": oid, "workspaceId": &admin.current_workspace },
            doc! { "$set": { "status": "dispatching", "updatedAt": DateTime::now() } },
            None,
        )
        .await?;
    let now = DateTime::now();
    let mut dispatched = 0i64;
    for c in &hits {
        // 活动级去重：先尝试插 campaign_sends（唯一索引 (campaignId, contactWxid)）。
        // DuplicateKey → 已推过，跳过。
        let send = CampaignSend {
            id: None,
            workspace_id: campaign.workspace_id.clone(),
            account_id: campaign.account_id.clone(),
            campaign_id: oid,
            contact_wxid: c.wxid.clone(),
            task_id: None,
            status: "enqueued".to_string(),
            created_at: now,
        };
        match state.db.campaign_sends().insert_one(&send, None).await {
            Ok(send_res) => {
                let send_id = send_res.inserted_id.as_object_id();
                let task = build_campaign_follow_up_task(
                    &campaign.workspace_id,
                    &campaign.account_id,
                    &c.wxid,
                    &campaign.intent_text,
                    now,
                );
                assert_agent_task_status_valid(&task.status);
                // KC-01：建 task 失败 → 补偿删掉刚占位的 send,避免留下 task_id=None 的孤儿 send
                // (重入撞去重跳过→客户永久漏推)。补偿删除 best-effort(let _)。
                let task_res = match state.db.tasks().insert_one(&task, None).await {
                    Ok(r) => r,
                    Err(e) => {
                        if let Some(sid) = send_id {
                            let _ = state
                                .db
                                .campaign_sends()
                                .delete_one(doc! { "_id": sid }, None)
                                .await;
                        }
                        return Err(e.into());
                    }
                };
                // KC-03：回填 taskId 失败 → 补偿删 task + send,保持 all-or-nothing
                // (否则 task 会真发但 report 显 pending 成效虚低)。
                if let (Some(sid), Some(tid)) =
                    (send_id, task_res.inserted_id.as_object_id())
                {
                    if let Err(e) = state
                        .db
                        .campaign_sends()
                        .update_one(
                            doc! { "_id": sid },
                            doc! { "$set": { "taskId": tid } },
                            None,
                        )
                        .await
                    {
                        let _ = state.db.tasks().delete_one(doc! { "_id": tid }, None).await;
                        let _ = state
                            .db
                            .campaign_sends()
                            .delete_one(doc! { "_id": sid }, None)
                            .await;
                        return Err(e.into());
                    }
                }
                dispatched += 1;
            }
            Err(e) if is_duplicate_key(&e) => { /* 去重命中，跳过 */ }
            Err(e) => return Err(e.into()),
        }
    }
    assert_campaign_status_valid("completed");
    state
        .db
        .campaigns()
        .update_one(
            doc! { "_id": oid, "workspaceId": &admin.current_workspace },
            doc! { "$set": { "status": "completed", "dispatchedCount": dispatched, "updatedAt": DateTime::now() } },
            None,
        )
        .await?;
    Ok(Json(
        json!({ "campaignId": id, "dispatchedCount": dispatched, "status": "completed" }),
    ))
}

/// 把一条 campaign_send 的真实推送结果归桶。输入 = 台账 status + 关联到的最新
/// agent_run_log（None 表示 task 还没被 worker 跑到 / 无关联 run log）。
/// 桶：sent/pending/blocked/canceled/escalated/skipped/unknown。优先级自上而下命中即停：
/// outbox_status=sent（真送达）先于 status 判定。run_log.status 取值是
/// GATEWAY_STATUS_VALUES 闭集（run_envelope.rs:86-135），逐值明确归桶。
/// escalated = 走请示通道交幕后领导裁决、待补料后 AI 会继续触达（非失败漏推），
/// 与 blocked（纯频控/硬约束、无后续）区分。详见设计 spec §5.3/§10。
pub(super) fn classify_send_outcome(
    send_status: &str,
    run_log: Option<&Document>,
) -> (&'static str, Option<String>) {
    // ① 去重命中：dispatch 当初就没建 task。
    if send_status == "skipped_duplicate" {
        return ("skipped", None);
    }
    // ② 有 taskId 但查不到 run log：task 还没被 worker 跑到。
    let Some(log) = run_log else {
        return ("pending", Some("not_yet_run".to_string()));
    };
    let outbox_status = log.get_str("outbox_status").ok();
    let run_status = log.get_str("status").ok();
    // ③ 真送达（最高优先级，先于一切 status 判定）。
    if outbox_status == Some("sent") {
        return ("sent", None);
    }
    // ④ outbox 终态失败/取消。
    if matches!(outbox_status, Some("failed_terminal") | Some("canceled")) {
        return ("canceled", outbox_status.map(str::to_string));
    }
    // ⑤ 进了发送队列、还没发出/发送中。
    if matches!(outbox_status, Some("pending") | Some("in_flight")) {
        return ("pending", None);
    }
    // ⑥ 按 run_log.status 归桶（GATEWAY_STATUS_VALUES 闭集逐值明确）。
    match run_status {
        // a. 放行/已入队/作息重排：会继续，视作在途。
        Some("allowed" | "outbox_enqueued" | "quiet_hours_deferred") => ("pending", None),
        // b. 频控/硬约束/改写失败——没发出且无后续。gateway_blocked = 二次 precheck
        //    在 LLM 决策后命中（罕见：频控/状态在决策窗口内翻转），顶层 status 是泛标签
        //    （真实原因在 gateway_result 子文档），语义上确定是一次"被拦下没发出"。
        Some(s @ ("daily_limit" | "cooldown" | "rate_limited"
            | "policy_cooldown" | "policy_wait_user_reply" | "policy_consecutive_limit"
            | "blocked_by_required_field" | "blocked_by_budget"
            | "review_blocked" | "revision_failed" | "revision_skipped_invalid_direction"
            | "revision_skipped_budget_exceeded" | "revision_llm_failure"
            | "tool_loop_timeout" | "gateway_blocked")) => ("blocked", Some(s.to_string())),
        // c. 已转交幕后领导请示，待裁决后 AI 会继续触达（非失败漏推）。
        Some(s @ ("blocked_unverified_product_claim" | "blocked_by_safety_guard"
            | "held_by_ai_policy" | "ai_waiting_for_more_context")) => {
            ("escalated", Some(s.to_string()))
        }
        // d. 取消（无后续）。
        Some(s @ ("context_changed" | "expired" | "not_managed"
            | "no_reply" | "admin_cancelled" | "superseded_by_new_inbound")) => {
            ("canceled", Some(s.to_string()))
        }
        // e. 灰度/口径态 / 不认识的值：诚实标 unknown，绝不强划进 sent。
        Some(other) => ("unknown", Some(other.to_string())),
        None => ("unknown", None),
    }
}

/// 把每人明细 items 聚合成 summary。sent/pending/skipped/unknown 标量计数，
/// blocked/canceled/escalated 按 reason 二级 map 计数。targetCount = items 总数。
pub(super) fn build_sends_summary(items: &[Value]) -> Value {
    use serde_json::Map;
    let (mut sent, mut pending, mut skipped, mut unknown) = (0i64, 0i64, 0i64, 0i64);
    let mut blocked: Map<String, Value> = Map::new();
    let mut canceled: Map<String, Value> = Map::new();
    let mut escalated: Map<String, Value> = Map::new();
    for it in items {
        let status = it.get("status").and_then(Value::as_str).unwrap_or("unknown");
        let reason = it.get("reason").and_then(Value::as_str);
        match status {
            "sent" => sent += 1,
            "pending" => pending += 1,
            "skipped" => skipped += 1,
            "blocked" => bump(&mut blocked, reason.unwrap_or("unknown")),
            "canceled" => bump(&mut canceled, reason.unwrap_or("unknown")),
            "escalated" => bump(&mut escalated, reason.unwrap_or("unknown")),
            _ => unknown += 1,
        }
    }
    json!({
        "targetCount": items.len() as i64,
        "sent": sent,
        "pending": pending,
        "skipped": skipped,
        "unknown": unknown,
        "blocked": Value::Object(blocked),
        "canceled": Value::Object(canceled),
        "escalated": Value::Object(escalated),
    })
}

/// reason 二级计数自增。
fn bump(map: &mut serde_json::Map<String, Value>, reason: &str) {
    let n = map.get(reason).and_then(Value::as_i64).unwrap_or(0);
    map.insert(reason.to_string(), json!(n + 1));
}

/// GET /campaigns/:id/sends —— 活动推送结果聚合（只读）。
/// 把 campaign_sends 台账与 agent_run_logs（关联键 source_event_id=taskId.hex）
/// 聚合成 7 桶分布 + 每人明细。零写入。IDOR：filter 含 workspaceId。
pub async fn campaign_sends_report(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("非法 campaign id".to_string()))?;
    // IDOR：先核实活动归属本 workspace。
    let campaign = state
        .db
        .campaigns()
        .find_one(doc! { "_id": oid, "workspaceId": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("campaign not found".to_string()))?;

    // 1) 台账（已有唯一索引 (campaignId, contactWxid)）。
    let sends: Vec<CampaignSend> = state
        .db
        .campaign_sends()
        .find(
            doc! { "campaignId": oid, "workspaceId": &admin.current_workspace },
            None,
        )
        .await?
        .try_collect()
        .await?;

    // 2) 批量拉 run log：taskId.hex 集合 → 一次 $in，内存按 source_event_id 取最新（max _id）。
    let task_hexes: Vec<String> = sends
        .iter()
        .filter_map(|s| s.task_id.map(|t| t.to_hex()))
        .collect();
    let mut latest_run: std::collections::HashMap<String, AgentRunLog> = std::collections::HashMap::new();
    if !task_hexes.is_empty() {
        let logs: Vec<AgentRunLog> = state
            .db
            .agent_run_logs()
            .find(
                doc! {
                    "source_event_id": { "$in": &task_hexes },
                    "source_kind": SOURCE_KIND_FOLLOW_UP_TASK,
                },
                None,
            )
            .await?
            .try_collect()
            .await?;
        for log in logs {
            let key = log.source_event_id.clone();
            // 同一 task 多条（retry）取 _id 最大那条 = 最新一次 run。
            match latest_run.get(&key) {
                Some(prev) if prev.id >= log.id => {}
                _ => {
                    latest_run.insert(key, log);
                }
            }
        }
    }

    // 3) 批量补客户名。
    let wxids: Vec<&String> = sends.iter().map(|s| &s.contact_wxid).collect();
    let mut name_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if !wxids.is_empty() {
        let contacts: Vec<Contact> = state
            .db
            .contacts()
            .find(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &campaign.account_id,
                    "wxid": { "$in": &wxids },
                },
                None,
            )
            .await?
            .try_collect()
            .await?;
        for c in contacts {
            let name = c.remark.clone().or(c.nickname.clone()).unwrap_or_default();
            name_of.insert(c.wxid, name);
        }
    }

    // 4) 逐人分类 → items。
    let mut items: Vec<Value> = Vec::with_capacity(sends.len());
    for s in &sends {
        let run_doc = s
            .task_id
            .map(|t| t.to_hex())
            .and_then(|hex| latest_run.get(&hex))
            .and_then(|log| mongodb::bson::to_document(log).ok());
        let (bucket, reason) = classify_send_outcome(&s.status, run_doc.as_ref());
        let name = name_of.get(&s.contact_wxid).cloned().unwrap_or_default();
        let mut item = json!({
            "contactWxid": s.contact_wxid,
            "name": name,
            "status": bucket,
        });
        if let Some(r) = reason {
            item["reason"] = json!(r);
        }
        items.push(item);
    }

    let summary = build_sends_summary(&items);
    Ok(Json(json!({
        "campaignId": id,
        "title": campaign.title,
        "status": campaign.status,
        "summary": summary,
        "items": items,
    })))
}

/// `GET /api/campaigns` 列表项投影（不裸序列化 Campaign，避免泄漏
/// workspace_id/segment_filter/intent_text，且 created_at 转 RFC3339 string
/// 而非 {$date}——照 products.rs:85 ProductView 范式）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CampaignListItem {
    campaign_id: String,
    title: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_count: Option<i64>,
    dispatched_count: i64,
    created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
}

impl From<&Campaign> for CampaignListItem {
    fn from(c: &Campaign) -> Self {
        Self {
            campaign_id: c.id.map(|i| i.to_hex()).unwrap_or_default(),
            title: c.title.clone(),
            status: c.status.clone(),
            target_count: c.target_count,
            dispatched_count: c.dispatched_count,
            created_by: c.created_by.clone(),
            created_at: crate::models::dt_to_string(c.created_at),
        }
    }
}

/// GET /api/campaigns —— 列出本 workspace 全部活动（只读，createdAt 倒序）。
/// 无分页（活动数量本身有限）。IDOR：filter 含 workspace_id。
pub async fn list_campaigns(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .campaigns()
        .find(
            doc! { "workspaceId": &admin.current_workspace },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "createdAt": -1 })
                .build(),
        )
        .await?;
    let mut items: Vec<CampaignListItem> = Vec::new();
    while let Some(c) = cursor.try_next().await? {
        items.push(CampaignListItem::from(&c));
    }
    Ok(Json(json!({ "items": items })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{OutcomeEvent, OutcomeProductRef};

    #[test]
    fn dispatch_allowed_only_from_draft_previewed_dispatching() {
        // KC-02：completed 活动不可再派发（防重复推送）；dispatching 允许重入恢复；未知态 fail-safe 拒。
        assert!(dispatch_allowed_from_status("draft"));
        assert!(dispatch_allowed_from_status("previewed"));
        assert!(dispatch_allowed_from_status("dispatching"), "dispatching 须允许重入恢复");
        assert!(!dispatch_allowed_from_status("completed"), "completed 不可重推");
        assert!(!dispatch_allowed_from_status("canceled"));
        assert!(!dispatch_allowed_from_status("赫赫"), "未知态 fail-safe 拒");
    }

    fn ev(verification: &str, pid: &str, qty: u32, kind: &str, amount: i64) -> OutcomeEvent {
        OutcomeEvent {
            marked_at: DateTime::from_millis(0),
            occurred_at: Some(DateTime::from_millis(0)),
            amount: Some(amount),
            currency: Some("CNY".to_string()),
            source: "manual".to_string(),
            marked_by: "admin".to_string(),
            note: None,
            verification: verification.to_string(),
            product_ref: Some(OutcomeProductRef {
                product_id: pid.to_string(),
                name: "P".to_string(),
                unit_price: Some(amount),
                sku: None,
                quantity: qty,
                entitlement_days: None,
            }),
            event_kind: kind.to_string(),
        }
    }

    fn contact_with(events: Vec<OutcomeEvent>) -> Contact {
        let mut c = base_contact();
        c.outcome_events = events;
        c
    }

    // 照 models.rs 的 Contact 真实字段构造一个最小 base（managed 状态）。
    pub(super) fn base_contact() -> Contact {
        Contact {
            id: None,
            workspace_id: "ws".to_string(),
            account_id: "acc".to_string(),
            wxid: "wx1".to_string(),
            nickname: None,
            remark: None,
            alias: None,
            avatar_url: None,
            sex: None,
            agent_status: crate::models::AgentStatus::Managed,
            human_profile_note: None,
            custom_agent_instructions: None,
            operation_mode_override: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            manual_tags: Vec::new(),
            manual_tags_updated_at: None,
            manual_tags_by: None,
            confirmed_tags: Vec::new(),
            bayesian_signals: Vec::new(),
            personality_profile: None,
            tags_version: 0,
            domain_attributes: None,
            domain_attributes_updated_at: None,
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: Document::new(),
            profile_attributes: Document::new(),
            profile_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            last_outbound_style: None,
            intent_trajectory: Vec::new(),
            outcome_events: Vec::new(),
            locale: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    pub(super) fn base_campaign() -> Campaign {
        let now = DateTime::from_millis(1_700_000_000_000);
        Campaign {
            id: Some(ObjectId::new()),
            workspace_id: "ws".to_string(),
            account_id: "acc".to_string(),
            title: "t".to_string(),
            intent_text: "i".to_string(),
            segment_filter: SegmentFilter::default(),
            status: "draft".to_string(),
            target_count: None,
            dispatched_count: 0,
            created_by: "admin".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn coarse_filter_with_products_uses_elemmatch_real_keys() {
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        let d = build_segment_coarse_filter("ws", "acc", &f);
        let em = d.get_document("outcome_events").unwrap();
        let elem = em.get_document("$elemMatch").unwrap();
        // productRef.productId（camelCase 内嵌）仍在
        assert!(elem.contains_key("productRef.productId"));
        // KC-05：verification / eventKind 从精确匹配改成"缺字段=默认值"显式表达，
        // 用 $elemMatch 内的 $and 数组承载（字段级"或缺失"不能用顶层 $or）。
        let and = elem.get_array("$and").unwrap();
        assert_eq!(and.len(), 2, "$and 恰两个子条件：verification 或缺失 + eventKind != reversal");
        // 子条件 1：verification $in 白名单 OR $exists:false
        let ver = and[0].as_document().unwrap();
        let ver_or = ver.get_array("$or").unwrap();
        assert_eq!(ver_or.len(), 2, "verification: $in 白名单 或 $exists:false");
        // 子条件 2：eventKind $ne reversal（缺字段天然命中，与精筛口径对齐）
        let kind = and[1].as_document().unwrap();
        let kind_ne = kind.get_document("eventKind").unwrap();
        assert_eq!(kind_ne.get_str("$ne").unwrap(), "reversal");
        // 始终带租户隔离
        assert_eq!(d.get_str("workspace_id").unwrap(), "ws");
        assert_eq!(d.get_str("account_id").unwrap(), "acc");
        assert_eq!(d.get_str("agent_status").unwrap(), "managed");
    }

    #[test]
    fn coarse_filter_empty_products_skips_outcome_condition() {
        let f = SegmentFilter::default();
        let d = build_segment_coarse_filter("ws", "acc", &f);
        // 空 product_ids：不加 outcome_events 条件，退化为按其他维度圈纳管客户
        assert!(d.get("outcome_events").is_none());
        assert_eq!(d.get_str("agent_status").unwrap(), "managed");
    }

    #[test]
    fn precise_filter_net_holding_excludes_fully_refunded() {
        // 买1件后全额退款 → 净持有0 → 不命中「买过 vip」
        let events = vec![
            ev("staff_confirmed", "vip", 1, "deal", 19900),
            ev("staff_confirmed", "vip", 1, "reversal", 19900),
        ];
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        assert!(!contact_matches_segment(
            &contact_with(events), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }

    #[test]
    fn precise_filter_conversation_inferred_never_matches() {
        // conversation_inferred 不进 G4 投影 → 不算持有
        let events = vec![ev("conversation_inferred", "vip", 1, "deal", 19900)];
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        assert!(!contact_matches_segment(
            &contact_with(events), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }

    #[test]
    fn precise_filter_value_tier_high_only() {
        // 累计 35 万分 = high 档（high_threshold=30万）；要求 high → 命中
        let events = vec![ev("staff_confirmed", "vip", 1, "deal", 350000)];
        let f = SegmentFilter {
            product_ids: vec!["vip".into()],
            value_tier: Some("high".into()),
            ..Default::default()
        };
        assert!(contact_matches_segment(
            &contact_with(events.clone()), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
        // 要求 high 但只值 1.99 元(19900分=low) → 不命中
        let cheap = vec![ev("staff_confirmed", "vip", 1, "deal", 19900)];
        assert!(!contact_matches_segment(
            &contact_with(cheap), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }

    #[test]
    fn build_follow_up_task_carries_intent_and_review() {
        // 自建 follow_up task（不调 planner 私有函数）：content=活动意图，
        // review_required=true，kind=follow_up，48h expiry，status=pending。
        let now = DateTime::from_millis(1_000_000);
        let task = build_campaign_follow_up_task("ws", "acc", "wx1", "双11老客7折", now);
        assert_eq!(task.kind, "follow_up");
        assert_eq!(task.content, "双11老客7折");
        assert!(task.review_required);
        assert_eq!(task.status, "pending");
        assert_eq!(task.contact_wxid, "wx1");
        // 48h expiry
        assert_eq!(
            task.expires_at.unwrap().timestamp_millis(),
            now.timestamp_millis() + 48 * 60 * 60 * 1000
        );
    }

    fn run_log(status: &str, outbox_status: Option<&str>) -> Document {
        let mut d = doc! { "status": status };
        if let Some(o) = outbox_status {
            d.insert("outbox_status", o);
        }
        d
    }

    #[test]
    fn classify_covers_all_buckets_and_priority() {
        // ① 去重
        assert_eq!(classify_send_outcome("skipped_duplicate", None), ("skipped", None));
        // ② 有 task 无 run log
        assert_eq!(
            classify_send_outcome("enqueued", None),
            ("pending", Some("not_yet_run".to_string()))
        );
        // ③ 真送达
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", Some("sent")))),
            ("sent", None)
        );
        // ③优先级：outbox=sent 时即便 status 非 allowed 也归 sent（命中即停）
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("daily_limit", Some("sent")))),
            ("sent", None)
        );
        // ③优先级关键：outbox=sent 压过 escalated 类 status（已送达优先于请示）
        assert_eq!(
            classify_send_outcome(
                "enqueued",
                Some(&run_log("blocked_unverified_product_claim", Some("sent")))
            ),
            ("sent", None)
        );
        // ④ outbox 终态失败
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", Some("failed_terminal")))),
            ("canceled", Some("failed_terminal".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", Some("canceled")))),
            ("canceled", Some("canceled".to_string()))
        );
        // ⑤ 在途（outbox pending / in_flight）
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", Some("pending")))),
            ("pending", None)
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("daily_limit", Some("in_flight")))),
            ("pending", None)
        );
        // ⑥a 放行/已入队/作息重排 → pending（会继续）
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", None))),
            ("pending", None)
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("outbox_enqueued", None))),
            ("pending", None)
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("quiet_hours_deferred", None))),
            ("pending", None)
        );
        // ⑥b 频控/硬约束/改写失败 → blocked，原因保留
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("daily_limit", None))),
            ("blocked", Some("daily_limit".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("policy_wait_user_reply", None))),
            ("blocked", Some("policy_wait_user_reply".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("blocked_by_required_field", None))),
            ("blocked", Some("blocked_by_required_field".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("revision_failed", None))),
            ("blocked", Some("revision_failed".to_string()))
        );
        // gateway_blocked = 二次 precheck 命中（顶层泛标签），语义=被拦下没发出 → blocked
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("gateway_blocked", None))),
            ("blocked", Some("gateway_blocked".to_string()))
        );
        // ⑥c 请示通道（escalated）：产品红线/安全门/AI策略/等上下文，原因保留
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("blocked_unverified_product_claim", None))),
            ("escalated", Some("blocked_unverified_product_claim".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("blocked_by_safety_guard", None))),
            ("escalated", Some("blocked_by_safety_guard".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("held_by_ai_policy", None))),
            ("escalated", Some("held_by_ai_policy".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("ai_waiting_for_more_context", None))),
            ("escalated", Some("ai_waiting_for_more_context".to_string()))
        );
        // ⑥d 取消（run status）
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("context_changed", None))),
            ("canceled", Some("context_changed".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("no_reply", None))),
            ("canceled", Some("no_reply".to_string()))
        );
        // ⑥e 灰度/口径态 / 不认识的 status → unknown
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("weird_new_status", None))),
            ("unknown", Some("weird_new_status".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("precheck_blocked", None))),
            ("unknown", Some("precheck_blocked".to_string()))
        );
        // ⑦ run log 有但 status 字段缺失
        assert_eq!(
            classify_send_outcome("enqueued", Some(&Document::new())),
            ("unknown", None)
        );
    }

    #[test]
    fn summary_counts_scalars_and_reason_submaps() {
        let items = vec![
            json!({ "contactWxid": "a", "name": "甲", "status": "sent" }),
            json!({ "contactWxid": "b", "name": "乙", "status": "sent" }),
            json!({ "contactWxid": "c", "name": "丙", "status": "pending" }),
            json!({ "contactWxid": "d", "name": "丁", "status": "skipped" }),
            json!({ "contactWxid": "e", "name": "戊", "status": "blocked", "reason": "daily_limit" }),
            json!({ "contactWxid": "f", "name": "己", "status": "blocked", "reason": "daily_limit" }),
            json!({ "contactWxid": "g", "name": "庚", "status": "blocked", "reason": "cooldown" }),
            json!({ "contactWxid": "h", "name": "辛", "status": "canceled", "reason": "context_changed" }),
            json!({ "contactWxid": "j", "name": "癸", "status": "escalated", "reason": "blocked_unverified_product_claim" }),
            json!({ "contactWxid": "k", "name": "子", "status": "escalated", "reason": "blocked_unverified_product_claim" }),
            json!({ "contactWxid": "l", "name": "丑", "status": "escalated", "reason": "held_by_ai_policy" }),
            json!({ "contactWxid": "i", "name": "壬", "status": "unknown" }),
        ];
        let s = build_sends_summary(&items);
        assert_eq!(s["targetCount"], json!(12));
        assert_eq!(s["sent"], json!(2));
        assert_eq!(s["pending"], json!(1));
        assert_eq!(s["skipped"], json!(1));
        assert_eq!(s["unknown"], json!(1));
        assert_eq!(s["blocked"]["daily_limit"], json!(2));
        assert_eq!(s["blocked"]["cooldown"], json!(1));
        assert_eq!(s["canceled"]["context_changed"], json!(1));
        assert_eq!(s["escalated"]["blocked_unverified_product_claim"], json!(2));
        assert_eq!(s["escalated"]["held_by_ai_policy"], json!(1));
    }

    #[test]
    fn summary_empty_items_all_zero() {
        let s = build_sends_summary(&[]);
        assert_eq!(s["targetCount"], json!(0));
        assert_eq!(s["sent"], json!(0));
        assert_eq!(s["pending"], json!(0));
        assert_eq!(s["skipped"], json!(0));
        assert_eq!(s["unknown"], json!(0));
        assert_eq!(s["blocked"], json!({}));
        assert_eq!(s["canceled"], json!({}));
        assert_eq!(s["escalated"], json!({}));
    }

    #[test]
    fn campaign_list_item_projection_shape_and_no_leak() {
        use serde_json::to_value;
        let now = DateTime::from_millis(1_700_000_000_000);
        let c = Campaign {
            id: Some(ObjectId::parse_str("64a1f0c2e4b0a1b2c3d4e5f6").unwrap()),
            workspace_id: "ws_secret".to_string(),
            account_id: "acc".to_string(),
            title: "双11老客7折".to_string(),
            intent_text: "内部意图不该泄漏".to_string(),
            segment_filter: SegmentFilter::default(),
            status: "completed".to_string(),
            target_count: Some(500),
            dispatched_count: 470,
            created_by: "admin".to_string(),
            created_at: now,
            updated_at: now,
        };
        let v = to_value(CampaignListItem::from(&c)).unwrap();
        // 投影字段齐全且 camelCase
        assert_eq!(v.get("campaignId").unwrap(), "64a1f0c2e4b0a1b2c3d4e5f6");
        assert_eq!(v.get("title").unwrap(), "双11老客7折");
        assert_eq!(v.get("status").unwrap(), "completed");
        assert_eq!(v.get("targetCount").unwrap(), 500);
        assert_eq!(v.get("dispatchedCount").unwrap(), 470);
        assert_eq!(v.get("createdBy").unwrap(), "admin");
        // createdAt 是 RFC3339 字符串（非 {$date} 对象）
        assert!(v.get("createdAt").unwrap().is_string());
        assert!(v.get("createdAt").unwrap().as_str().unwrap().contains("2023"));
        // 不泄漏内部字段
        assert!(v.get("workspaceId").is_none());
        assert!(v.get("workspace_id").is_none());
        assert!(v.get("segmentFilter").is_none());
        assert!(v.get("intentText").is_none());
        assert!(v.get("accountId").is_none());
    }

    #[test]
    fn campaign_list_item_omits_target_count_when_none() {
        use serde_json::to_value;
        let mut c = base_campaign();
        c.target_count = None;
        let v = to_value(CampaignListItem::from(&c)).unwrap();
        // draft 没预览过 → targetCount 字段整个缺失（skip_serializing_if）
        assert!(v.get("targetCount").is_none());
        assert_eq!(v.get("dispatchedCount").unwrap(), 0);
    }
}

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
    assert_agent_task_status_valid, assert_campaign_status_valid, AgentTask, Campaign,
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
    // 产品反查：$elemMatch 同一成交事件内匹配「指定产品 + 高可信 + 正向」。
    if !filter.product_ids.is_empty() {
        d.insert(
            "outcome_events",
            doc! { "$elemMatch": {
                "productRef.productId": { "$in": &filter.product_ids },
                "verification": { "$in": ["staff_confirmed", "payment_verified"] },
                "eventKind": "deal",
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
async fn resolve_segment_contacts(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
) -> AppResult<Vec<Contact>> {
    let coarse = build_segment_coarse_filter(workspace_id, account_id, filter);
    let mut cursor = state.db.contacts().find(coarse, None).await?;
    let active_products: Vec<Product> =
        entitlements::load_active_products(&state.db, workspace_id).await;
    let now = DateTime::now();
    let (mid, high) = value_tier_thresholds(&state.config);
    let mut hits = Vec::new();
    while let Some(c) = cursor.try_next().await? {
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
    let campaign = Campaign {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: state.config.default_account_id.clone(),
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
    // 重新跑圈人（防预览后数据漂移）。
    let hits = resolve_segment_contacts(
        &state,
        &campaign.workspace_id,
        &campaign.account_id,
        &campaign.segment_filter,
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
                let task = build_campaign_follow_up_task(
                    &campaign.workspace_id,
                    &campaign.account_id,
                    &c.wxid,
                    &campaign.intent_text,
                    now,
                );
                assert_agent_task_status_valid(&task.status);
                let task_res = state.db.tasks().insert_one(&task, None).await?;
                // 回填 taskId。
                if let (Some(send_id), Some(task_id)) = (
                    send_res.inserted_id.as_object_id(),
                    task_res.inserted_id.as_object_id(),
                ) {
                    state
                        .db
                        .campaign_sends()
                        .update_one(
                            doc! { "_id": send_id },
                            doc! { "$set": { "taskId": task_id } },
                            None,
                        )
                        .await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{OutcomeEvent, OutcomeProductRef};

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

    #[test]
    fn coarse_filter_with_products_uses_elemmatch_real_keys() {
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        let d = build_segment_coarse_filter("ws", "acc", &f);
        // 真实混合大小写路径
        let em = d.get_document("outcome_events").unwrap();
        let elem = em.get_document("$elemMatch").unwrap();
        // productRef.productId（camelCase 内嵌）
        assert!(elem.get_document("productRef").is_ok()
            || elem.contains_key("productRef.productId"));
        // verification 高可信 $in
        assert!(elem.contains_key("verification"));
        // eventKind 正向
        assert_eq!(elem.get_str("eventKind").ok(), Some("deal"));
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
}

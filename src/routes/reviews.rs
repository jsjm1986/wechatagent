//! 决策复盘路由：列出 / 查询 Agent 决策审阅记录。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DecisionReviewQuery {
    account_id: Option<String>,
    contact_id: Option<String>,
    contact_wxid: Option<String>,
    limit: Option<i64>,
}

pub(super) async fn list_decision_reviews(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<DecisionReviewQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let mut filter = doc! {
        "workspace_id": &admin.current_workspace,
        "account_id": &account_id
    };
    if let Some(contact_id) = query.contact_id {
        let contact = find_contact_by_id(&state, &admin.current_workspace, &contact_id).await?;
        filter.insert("contact_wxid", contact.wxid);
    } else if let Some(contact_wxid) = query.contact_wxid {
        if !contact_wxid.is_empty() {
            filter.insert("contact_wxid", contact_wxid);
        }
    }
    let mut cursor = state
        .db
        .decision_reviews()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(query.limit.unwrap_or(100).clamp(1, 300))
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(review) = cursor.try_next().await? {
        let status = fetch_run_status(&state, review.run_id.as_deref()).await;
        items.push(decision_review_json(
            review,
            status.final_review_status,
            status.hold_category,
            status.autonomy_protocol,
        ));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn get_decision_review(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let review = state
        .db
        .decision_reviews()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("decision review not found".to_string()))?;
    let status = fetch_run_status(&state, review.run_id.as_deref()).await;
    Ok(Json(json!({ "item": decision_review_json(
        review,
        status.final_review_status,
        status.hold_category,
        status.autonomy_protocol,
    ) })))
}

/// agent_run_logs.decision（camelCase Document）里的 9 个 R1.1 自治协议字段。
/// 9 个全空（缺失或空串）→ None（优雅降级，前端不渲染「AI 内心独白」区，覆盖历史
/// 旧数据 + 管理发送路径两类无完整 decision 的复盘）；否则 Some(全 9 键对象，空的填 "")。
fn autonomy_protocol_from_decision(decision: &Document) -> Option<Value> {
    const KEYS: [&str; 9] = [
        "userUnderstanding",
        "relationshipRead",
        "operationGoal",
        "knowledgeNeedReason",
        "memoryUpdateReason",
        "riskSelfCheck",
        "selfCritique",
        "whyShouldReply",
        "whySkipReply",
    ];
    let vals: Vec<&str> = KEYS
        .iter()
        .map(|k| decision.get_str(*k).unwrap_or(""))
        .collect();
    if vals.iter().all(|v| v.trim().is_empty()) {
        return None;
    }
    let mut obj = serde_json::Map::new();
    for (k, v) in KEYS.iter().zip(vals.iter()) {
        obj.insert((*k).to_string(), Value::from(*v));
    }
    Some(Value::Object(obj))
}

struct RunStatusView {
    final_review_status: Option<String>,
    hold_category: Option<String>,
    autonomy_protocol: Option<Value>,
}

/// 关联同 run_id 的 AgentRunLog，取 final_review_status（顶层 snake 字段）、
/// review doc 内的 holdCategory（camelCase），以及 decision doc 内的 9 个自治协议字段。
/// 纯读投影，缺失则回 None。
async fn fetch_run_status(state: &AppState, run_id: Option<&str>) -> RunStatusView {
    let Some(run_id) = run_id.filter(|s| !s.is_empty()) else {
        return RunStatusView {
            final_review_status: None,
            hold_category: None,
            autonomy_protocol: None,
        };
    };
    match state
        .db
        .agent_run_logs()
        .find_one(doc! { "run_id": run_id }, None)
        .await
    {
        Ok(Some(log)) => {
            let frs = if log.final_review_status.is_empty() {
                None
            } else {
                Some(log.final_review_status.clone())
            };
            let hc = log
                .review
                .get_str("holdCategory")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let ap = autonomy_protocol_from_decision(&log.decision);
            RunStatusView {
                final_review_status: frs,
                hold_category: hc,
                autonomy_protocol: ap,
            }
        }
        _ => RunStatusView {
            final_review_status: None,
            hold_category: None,
            autonomy_protocol: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn autonomy_protocol_all_empty_returns_none() {
        // decision 无任何自治字段（或全空串）→ None（优雅降级）
        let decision = doc! { "replyText": "hi", "userUnderstanding": "" };
        assert!(autonomy_protocol_from_decision(&decision).is_none());
    }

    #[test]
    fn autonomy_protocol_partial_returns_full_nine_keys() {
        // 任一非空 → Some，含全 9 键，缺失/空的填 ""
        let decision = doc! { "whyShouldReply": "用户主动询问，及时回应推进决策" };
        let v = autonomy_protocol_from_decision(&decision).expect("some");
        let obj = v.as_object().expect("object");
        assert_eq!(obj.len(), 9);
        assert_eq!(obj.get("whyShouldReply").and_then(|x| x.as_str()), Some("用户主动询问，及时回应推进决策"));
        assert_eq!(obj.get("userUnderstanding").and_then(|x| x.as_str()), Some(""));
        assert_eq!(obj.get("riskSelfCheck").and_then(|x| x.as_str()), Some(""));
    }
}

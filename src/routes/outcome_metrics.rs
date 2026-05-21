//! Agent 成效指标路由：聚合性指标暴露。

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

use crate::{error::AppResult, models::AgentOutcomeMetric};

use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OutcomeMetricsQuery {
    account_id: Option<String>,
    horizon: Option<String>,
    from_date: Option<String>,
    to_date: Option<String>,
    limit: Option<i64>,
}

pub(super) async fn list_agent_outcome_metrics(
    State(state): State<AppState>,
    Query(query): Query<OutcomeMetricsQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let mut filter = doc! {
        "workspace_id": &state.config.default_workspace_id,
        "account_id": &account_id
    };
    if let Some(horizon) = query.horizon {
        filter.insert("horizon", horizon);
    }
    let mut date_filter = Document::new();
    if let Some(from) = query.from_date {
        date_filter.insert("$gte", from);
    }
    if let Some(to) = query.to_date {
        date_filter.insert("$lte", to);
    }
    if !date_filter.is_empty() {
        filter.insert("date", date_filter);
    }
    let limit = query.limit.unwrap_or(60).clamp(1, 365);
    let mut cursor = state
        .db
        .outcome_metrics()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "date": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(metric) = cursor.try_next().await? {
        items.push(outcome_metric_json(metric));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) fn outcome_metric_json(item: AgentOutcomeMetric) -> Value {
    // 波 A2：reply_rate / conversation_depth / human_handoff_success_rate /
    // agent_block_rate 都是 Option<f64>，在 JSON 中序列化为 number 或 null。
    // 前端按 null 显示"暂无数据"。
    json!({
        "id": item.id,
        "accountId": item.account_id,
        "horizon": item.horizon,
        "date": item.date,
        "replyRate": item.reply_rate,
        "conversationDepth": item.conversation_depth,
        "humanHandoffSuccessRate": item.human_handoff_success_rate,
        "agentBlockRate": item.agent_block_rate,
        "dailyRunCount": item.daily_run_count,
        "dailyRunTokenTotal": item.daily_run_token_total,
        "createdAt": crate::models::dt_to_string(item.created_at)
    })
}

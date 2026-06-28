//! P3 采集健康度路由：`behavior_signal_metrics` 三态计数只读暴露。
//!
//! 镜像 `outcome_metrics.rs` 的查询形状（按 workspace + 日期倒序拉近期），
//! 暴露采集管道的新鲜度 / 量 / 失败率三指标，供 admin 观测自学习采集是否断流。

use axum::{
    extract::{Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::AuthenticatedAdmin, error::AppResult, models::BehaviorSignalMetric};

use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BehaviorSignalMetricsQuery {
    from_date: Option<String>,
    to_date: Option<String>,
    limit: Option<i64>,
}

pub(super) async fn list_behavior_signal_metrics(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<BehaviorSignalMetricsQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
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
        .behavior_signal_metrics()
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
        items.push(behavior_signal_metric_json(metric));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) fn behavior_signal_metric_json(item: BehaviorSignalMetric) -> Value {
    json!({
        "id": item.id,
        "workspaceId": item.workspace_id,
        "date": item.date,
        "persisted": item.persisted,
        "dedupeSkipped": item.dedupe_skipped,
        "errors": item.errors,
        "lastSuccessAt": item.last_success_at.and_then(crate::models::dt_to_string),
        "updatedAt": crate::models::dt_to_string(item.updated_at),
    })
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    use mongodb::bson::DateTime;

    /// 契约快照：behavior_signal_metric_json 线上形状钉死到 fixture。
    /// 全量赋值（每个 Option 给 Some、DateTime 用固定值），调投影 → assert_contract_fixture。
    #[test]
    fn behavior_signal_metric_json_matches_contract_fixture() {
        let item = BehaviorSignalMetric {
            id: "ws-1:2026-06-29".to_string(),
            workspace_id: "ws-1".to_string(),
            date: "2026-06-29".to_string(),
            persisted: 42,
            dedupe_skipped: 3,
            errors: 1,
            last_success_at: Some(DateTime::from_millis(1_700_000_000_000)),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let projected = behavior_signal_metric_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "behavior_signal_metric",
            projected,
        );
    }
}

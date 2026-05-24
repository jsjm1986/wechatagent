//! Agent 事件流路由：审计与运营追踪用。

use axum::{
    extract::{Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{bson::doc, options::FindOptions};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppResult;

use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EventsQuery {
    pub(super) account_id: Option<String>,
    /// 精确事件类型过滤。审计场景下当 strategic_planner_*tick 高频写入时，
    /// 不带 kind 过滤的列表会把 knowledge_repair_applied 等低频事件挤出窗口。
    pub(super) kind: Option<String>,
    /// 1..=500，默认 100。配合 kind 用，避免事件被同期高频事件淹没。
    pub(super) limit: Option<i64>,
}

pub(super) async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let mut filter = doc! {
        "workspace_id": &state.config.default_workspace_id,
        "account_id": &account_id
    };
    if let Some(kind) = query.kind.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("kind", kind.trim());
    }
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let mut cursor = state
        .db
        .events()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(event) = cursor.try_next().await? {
        items.push(json!({
            "id": event.id.map(|id| id.to_hex()).unwrap_or_default(),
            "contactWxid": event.contact_wxid,
            "kind": event.kind,
            "status": event.status,
            "summary": event.summary,
            "createdAt": crate::models::dt_to_string(event.created_at)
        }));
    }
    Ok(Json(json!({ "items": items })))
}

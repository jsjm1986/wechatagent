//! Agent 事件流路由：审计与运营追踪用。

use axum::{
    extract::{Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{bson::doc, options::FindOptions};
use serde_json::{json, Value};

use crate::error::AppResult;

use super::shared::*;
use super::AppState;

pub(super) async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let mut cursor = state
        .db
        .events()
        .find(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "account_id": &account_id
            },
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(100)
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

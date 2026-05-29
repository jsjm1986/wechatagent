//! 决策复盘路由：列出 / 查询 Agent 决策审阅记录。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{bson::doc, options::FindOptions};
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
        items.push(decision_review_json(review));
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
    Ok(Json(json!({ "item": decision_review_json(review) })))
}

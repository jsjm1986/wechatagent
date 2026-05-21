//! 内容资产路由：私域素材库的列表与新增。

use axum::{
    extract::{Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    models::ContentAsset,
};

use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentAssetQuery {
    account_id: Option<String>,
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentAssetRequest {
    account_id: Option<String>,
    kind: String,
    title: String,
    body: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    url: Option<String>,
    media_id: Option<String>,
    usage_scene: Option<String>,
}

pub(super) async fn list_content_assets(
    State(state): State<AppState>,
    Query(query): Query<ContentAssetQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &state.config.default_workspace_id };
    if let Some(account_id) = query.account_id {
        filter.insert(
            "$or",
            vec![
                doc! { "account_id": null },
                doc! { "account_id": account_id },
            ],
        );
    }
    if let Some(kind) = query.kind {
        if !kind.is_empty() {
            filter.insert("kind", kind);
        }
    }
    let mut cursor = state
        .db
        .content_assets()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(200)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(asset) = cursor.try_next().await? {
        items.push(json!({
            "id": asset.id.map(|id| id.to_hex()).unwrap_or_default(),
            "workspaceId": asset.workspace_id,
            "accountId": asset.account_id,
            "kind": asset.kind,
            "title": asset.title,
            "body": asset.body,
            "tags": asset.tags,
            "url": asset.url,
            "mediaId": asset.media_id,
            "usageScene": asset.usage_scene,
            "updatedAt": crate::models::dt_to_string(asset.updated_at)
        }));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_content_asset(
    State(state): State<AppState>,
    Json(payload): Json<ContentAssetRequest>,
) -> AppResult<Json<Value>> {
    if payload.kind.trim().is_empty() || payload.title.trim().is_empty() {
        return Err(AppError::BadRequest(
            "kind and title are required".to_string(),
        ));
    }
    let asset = ContentAsset {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: payload.account_id,
        kind: payload.kind,
        title: payload.title,
        body: payload.body,
        tags: payload.tags,
        url: payload.url,
        media_id: payload.media_id,
        usage_scene: payload.usage_scene,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state.db.content_assets().insert_one(asset, None).await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

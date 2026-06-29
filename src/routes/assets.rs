//! 内容资产路由：私域素材库的列表与新增。

use axum::{
    extract::{Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::ContentAsset,
};

use super::AppState;

/// 归一化前端传入的 min_inject_tier：闭集 {lean,relational,full} 内保留原值，
/// 否则（None/空/非法）落 "full"（保守，等价改造前仅 Full 注入）。
fn normalize_min_inject_tier(raw: Option<&str>) -> String {
    match raw.map(str::trim) {
        Some("lean") => "lean".to_string(),
        Some("relational") => "relational".to_string(),
        _ => "full".to_string(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentAssetQuery {
    account_id: Option<String>,
    kind: Option<String>,
    tag: Option<String>,
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
    usage_scene: Option<String>,
    min_inject_tier: Option<String>,
}

pub async fn list_content_assets(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ContentAssetQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
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
    // 按 tag 检索：MongoDB 数组字段等值匹配命中 tags 数组含该元素的文档（workspace scope 保留）。
    if let Some(tag) = query.tag {
        if !tag.is_empty() {
            filter.insert("tags", tag);
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
            // 销售素材文件字段（前端预览/审核用）
            "mediaType": asset.media_type,
            "fileName": asset.file_name,
            "fileSize": asset.file_size,
            "mimeType": asset.mime_type,
            "sendTriggerHint": asset.send_trigger_hint,
            "targetStages": asset.target_stages,
            "expressionPref": asset.expression_pref,
            "requiresPrincipalApproval": asset.requires_principal_approval,
            "reviewStatus": asset.review_status,
            "sendable": asset.sendable,
            "reviewNote": asset.review_note,
            "minInjectTier": asset.min_inject_tier,
            "updatedAt": crate::models::dt_to_string(asset.updated_at)
        }));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_content_asset(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<ContentAssetRequest>,
) -> AppResult<Json<Value>> {
    if payload.kind.trim().is_empty() || payload.title.trim().is_empty() {
        return Err(AppError::BadRequest(
            "kind and title are required".to_string(),
        ));
    }
    let asset = ContentAsset {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: payload.account_id,
        kind: payload.kind,
        title: payload.title,
        body: payload.body,
        tags: payload.tags,
        url: None,
        media_id: None,
        usage_scene: payload.usage_scene,
        media_type: None,
        file_path: None,
        file_name: None,
        file_size: None,
        mime_type: None,
        file_sha256: None,
        sendable: None,
        send_trigger_hint: None,
        target_stages: None,
        expression_pref: None,
        requires_principal_approval: None,
        review_status: None,
        review_note: None,
        min_inject_tier: Some(normalize_min_inject_tier(payload.min_inject_tier.as_deref())),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state.db.content_assets().insert_one(asset, None).await?;
    Ok(Json(
        json!({ "id": result.inserted_id.as_object_id().map(|id| id.to_hex()) }),
    ))
}

#[cfg(test)]
mod tests {
    use super::normalize_min_inject_tier;

    #[test]
    fn normalize_keeps_valid_lowercases_defaults_full() {
        assert_eq!(normalize_min_inject_tier(Some("lean")), "lean");
        assert_eq!(normalize_min_inject_tier(Some("relational")), "relational");
        assert_eq!(normalize_min_inject_tier(Some("full")), "full");
        assert_eq!(normalize_min_inject_tier(None), "full");
        assert_eq!(normalize_min_inject_tier(Some("garbage")), "full");
        assert_eq!(normalize_min_inject_tier(Some("")), "full");
    }
}

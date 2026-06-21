//! 销售素材库：文件上传（multipart）+ 审核状态流转。
//!
//! 安全红线：
//! - 落盘路径**只能**经 [`media_storage::safe_relative_path`] 产出（workspace/sha 分片），
//!   绝不把用户传入的 `file_name` 拼进磁盘路径——原始文件名只存 DB 字段供展示。
//! - 文件大小受 `media_max_file_size_mb` 限制；扩展名/mime 走 `sanitize_ext` 白名单。
//! - 上传素材默认 `review_status="draft"`：AI 不自我核验红线，必须人类 approve 才可发。
use axum::{
    extract::{Multipart, Path, State},
    Extension, Json,
};
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde::Deserialize;
use serde_json::{json, Value};

use super::AppState;
use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    media_storage,
    models::ContentAsset,
};

/// media_type 合法性判断（纯函数，便于单测）。
fn is_valid_media_type(media_type: &str) -> bool {
    matches!(media_type, "image" | "file" | "video")
}

/// 审核目标状态合法性判断（纯函数，便于单测）。
fn is_valid_review_status(status: &str) -> bool {
    matches!(status, "approved" | "draft")
}

pub(super) async fn upload_media_asset(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    mut multipart: Multipart,
) -> AppResult<Json<Value>> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name = String::new();
    let mut mime = String::new();
    let mut title = String::new();
    let mut media_type = String::new();
    let mut send_trigger_hint: Option<String> = None;
    let mut expression_pref: Option<String> = None;
    let mut target_stages: Vec<String> = vec![];
    let mut requires_principal_approval = false;
    let mut account_id: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        match field.name().unwrap_or_default() {
            "file" => {
                file_name = field.file_name().unwrap_or_default().to_string();
                mime = field.content_type().unwrap_or_default().to_string();
                file_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("read file failed: {e}")))?
                        .to_vec(),
                );
            }
            "title" => title = field.text().await.unwrap_or_default(),
            "mediaType" => media_type = field.text().await.unwrap_or_default(),
            "sendTriggerHint" => {
                let v = field.text().await.unwrap_or_default();
                send_trigger_hint = (!v.is_empty()).then_some(v);
            }
            "expressionPref" => {
                let v = field.text().await.unwrap_or_default();
                expression_pref = (!v.is_empty()).then_some(v);
            }
            "targetStages" => {
                // 逗号分隔
                target_stages = field
                    .text()
                    .await
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
            "requiresPrincipalApproval" => {
                requires_principal_approval = field.text().await.unwrap_or_default() == "true";
            }
            "accountId" => {
                let v = field.text().await.unwrap_or_default();
                account_id = (!v.is_empty()).then_some(v);
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let bytes = file_bytes.ok_or_else(|| AppError::BadRequest("file field required".into()))?;
    if title.trim().is_empty() {
        return Err(AppError::BadRequest("title required".into()));
    }
    // 大小上限
    let max = state.config.media_max_file_size_mb * 1024 * 1024;
    if bytes.len() as u64 > max {
        return Err(AppError::BadRequest(format!(
            "file exceeds {} MB",
            state.config.media_max_file_size_mb
        )));
    }
    // media_type 合法性
    if !is_valid_media_type(&media_type) {
        return Err(AppError::BadRequest(
            "mediaType must be image|file|video".into(),
        ));
    }
    // 扩展名/mime 白名单（拒绝即 400）
    let ext = media_storage::sanitize_ext(&file_name, &mime)
        .ok_or_else(|| AppError::BadRequest("file type not allowed".into()))?;

    // 缺口 6：归一 target_stages 到 canonical（与 contact.customer_stage 同空间），
    // 越界即 400。account_id 缺失 → 空串走 global scope。
    // 前移到 store_bytes 之前：越界 stage 时不落盘、不入库，避免留下孤儿文件。
    let scope = account_id.as_deref().unwrap_or("");
    let target_stages = crate::agent::dimension_registry::normalize_target_stages(
        &state.db,
        scope,
        &target_stages,
    )
    .await
    .map_err(|reason| AppError::BadRequest(format!("target_stages 校验未通过：{reason}")))?;

    // 落盘：路径只由 workspace+sha+ext 产出，user 的 file_name 不进磁盘路径。
    let sha = media_storage::sha256_hex(&bytes);
    let rel = media_storage::safe_relative_path(&admin.current_workspace, &sha, &ext)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let root = std::path::Path::new(&state.config.media_storage_dir);
    media_storage::store_bytes(root, &rel, &bytes)
        .await
        .map_err(|e| AppError::External(format!("store file failed: {e}")))?;

    let asset = ContentAsset {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id,
        kind: "media".into(),
        title,
        body: None,
        tags: vec![],
        url: None,
        media_id: None,
        usage_scene: None,
        media_type: Some(media_type),
        file_path: Some(rel),
        file_name: Some(file_name),
        file_size: Some(bytes.len() as i64),
        mime_type: Some(mime),
        file_sha256: Some(sha),
        sendable: Some(true),
        send_trigger_hint,
        target_stages: (!target_stages.is_empty()).then_some(target_stages),
        expression_pref,
        requires_principal_approval: Some(requires_principal_approval),
        // AI 不自我核验红线：默认草稿，待人类 approve 才可发。
        review_status: Some("draft".into()),
        review_note: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let res = state.db.content_assets().insert_one(asset, None).await?;
    Ok(Json(
        json!({ "id": res.inserted_id.as_object_id().map(|i| i.to_hex()) }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    status: String,
    note: Option<String>,
}

pub async fn review_media_asset(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ReviewRequest>,
) -> AppResult<Json<Value>> {
    if !is_valid_review_status(&payload.status) {
        return Err(AppError::BadRequest("status must be approved|draft".into()));
    }
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    let res = state
        .db
        .content_assets()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": {
                "review_status": &payload.status,
                "review_note": payload.note.clone(),
                "updated_at": DateTime::now(),
            }},
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound("asset not found".into()));
    }
    // 缺口 3：审计审核动作（谁把哪份素材改成什么状态）。回查拿 account_id/title。
    // fail-soft：审计写失败只 warn，不回滚 review（review 已生效=既成事实）。
    if let Ok(Some(asset)) = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await
    {
        let account_id = asset.account_id.clone().unwrap_or_default();
        let details = doc! {
            "asset_id": oid.to_hex(),
            "review_note": payload.note.clone().unwrap_or_default(),
            "reviewed_by": admin.username.clone(),
        };
        if let Err(e) = crate::agent::write_event_for_account(
            &state,
            &account_id,
            None,
            "media_asset.reviewed",
            &payload.status,
            &format!("管理员审核素材：{} → {}", asset.title, payload.status),
            Some(details),
        )
        .await
        {
            tracing::warn!("media_asset.reviewed 审计写入失败（不影响审核）: {e}");
        }
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetaRequest {
    title: Option<String>,
    body: Option<String>,
    tags: Option<Vec<String>>,
    url: Option<String>,
    usage_scene: Option<String>,
    send_trigger_hint: Option<String>,
    expression_pref: Option<String>,
    target_stages: Option<Vec<String>>,
    requires_principal_approval: Option<bool>,
}

/// PUT /content-assets/:id —— 改元数据（JSON，部分更新）。
/// 只 $set 客户端提供的字段；不动 file_*/media_id/review_status/sendable。
/// target_stages 复用簇 B normalize_target_stages 归一，越界 400。
pub async fn update_content_asset_meta(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateMetaRequest>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    // 回查 asset（workspace 隔离）拿 account_id 做归一 scope。
    let asset = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;

    // 部分更新：字段 Some → $set；None（JSON 缺失或 null）→ 不动。
    // serde 不区分缺失与 null，故不支持显式清成 null；清空走传 ""/[]。
    let mut set = Document::new();
    if let Some(v) = payload.title {
        set.insert("title", v);
    }
    if let Some(v) = payload.body {
        set.insert("body", v);
    }
    if let Some(v) = payload.tags {
        set.insert("tags", v);
    }
    if let Some(v) = payload.url {
        set.insert("url", v);
    }
    if let Some(v) = payload.usage_scene {
        set.insert("usage_scene", v);
    }
    if let Some(v) = payload.send_trigger_hint {
        set.insert("send_trigger_hint", v);
    }
    if let Some(v) = payload.expression_pref {
        set.insert("expression_pref", v);
    }
    if let Some(v) = payload.requires_principal_approval {
        set.insert("requires_principal_approval", v);
    }
    if let Some(stages) = payload.target_stages {
        // 复用簇 B 归一；scope 取被编辑 asset 自身 account_id，缺失走空串。
        let scope = asset.account_id.as_deref().unwrap_or("");
        let normalized =
            crate::agent::dimension_registry::normalize_target_stages(&state.db, scope, &stages)
                .await
                .map_err(|reason| {
                    AppError::BadRequest(format!("target_stages 校验未通过：{reason}"))
                })?;
        set.insert("target_stages", normalized);
    }
    set.insert("updated_at", DateTime::now());

    state
        .db
        .content_assets()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": set },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// POST /content-assets/:id/file —— 换文件（multipart）。
/// 落新文件 → $set file_* + media_id=None（清缓存防发旧文件）+ review_status="draft"（强制重审）。
/// 旧文件无兄弟引用则物理删（fail-soft）。
pub(super) async fn replace_content_asset_file(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    // 回查 asset（workspace 隔离）拿旧 file_path。
    let asset = state
        .db
        .content_assets()
        .find_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;
    let old_file_path = asset.file_path.clone();

    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name = String::new();
    let mut mime = String::new();
    let mut media_type = asset.media_type.clone().unwrap_or_default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        match field.name().unwrap_or_default() {
            "file" => {
                file_name = field.file_name().unwrap_or_default().to_string();
                mime = field.content_type().unwrap_or_default().to_string();
                file_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("read file failed: {e}")))?
                        .to_vec(),
                );
            }
            "mediaType" => media_type = field.text().await.unwrap_or_default(),
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let bytes = file_bytes.ok_or_else(|| AppError::BadRequest("file field required".into()))?;
    let max = state.config.media_max_file_size_mb * 1024 * 1024;
    if bytes.len() as u64 > max {
        return Err(AppError::BadRequest(format!(
            "file exceeds {} MB",
            state.config.media_max_file_size_mb
        )));
    }
    if !is_valid_media_type(&media_type) {
        return Err(AppError::BadRequest(
            "mediaType must be image|file|video".into(),
        ));
    }
    let ext = media_storage::sanitize_ext(&file_name, &mime)
        .ok_or_else(|| AppError::BadRequest("file type not allowed".into()))?;
    let sha = media_storage::sha256_hex(&bytes);
    let rel = media_storage::safe_relative_path(&admin.current_workspace, &sha, &ext)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let root = std::path::Path::new(&state.config.media_storage_dir);
    media_storage::store_bytes(root, &rel, &bytes)
        .await
        .map_err(|e| AppError::External(format!("store file failed: {e}")))?;

    // 换文件副作用：清 media_id（防 TTL 内发旧文件）+ 退 draft（强制重审）。
    state
        .db
        .content_assets()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": {
                "file_path": &rel,
                "file_name": &file_name,
                "file_size": bytes.len() as i64,
                "mime_type": &mime,
                "file_sha256": &sha,
                "media_type": &media_type,
                "media_id": null,
                "review_status": "draft",
                "updated_at": DateTime::now(),
            }},
            None,
        )
        .await?;

    // 旧文件清理：仅当旧路径与新路径不同（确实换了文件）且无兄弟引用时物理删。fail-soft。
    if let Some(old) = old_file_path {
        if old != rel {
            let refs = state
                .db
                .content_assets()
                .count_documents(
                    doc! { "workspace_id": &admin.current_workspace, "file_path": &old },
                    None,
                )
                .await
                .unwrap_or(1); // 查询失败 → 视为有引用，保守不删
            if media_storage::should_delete_physical_file(refs) {
                if let Err(e) = media_storage::delete_bytes(root, &old).await {
                    tracing::warn!("换文件后旧素材文件删除失败（不影响换文件）: {e}");
                }
            }
        }
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleSendableRequest {
    sendable: bool,
}

/// POST /content-assets/:id/toggle —— 启停（写 sendable）。
/// 与 review_status 正交：停用不动审核态，重启不必重审。workspace 隔离。
pub async fn toggle_content_asset_sendable(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ToggleSendableRequest>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    let res = state
        .db
        .content_assets()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": { "sendable": payload.sendable, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound("asset not found".into()));
    }
    Ok(Json(json!({ "ok": true, "sendable": payload.sendable })))
}

/// DELETE /content-assets/:id —— 删除。
/// 先删 DB 记录,再查同 file_path 剩余引用,无引用才物理删文件(防误删兄弟共享文件)。
/// 物理删 fail-soft(DB 已删=既成事实,残留文件无害)。workspace 隔离。
pub async fn delete_content_asset(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    // 回查拿 file_path（workspace 隔离）。
    let asset = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;

    let res = state
        .db
        .content_assets()
        .delete_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?;
    if res.deleted_count == 0 {
        return Err(AppError::NotFound("asset not found".into()));
    }

    // 引用计数清理：本记录已删,count 同 file_path 剩余引用,为 0 才物理删。
    if let Some(rel) = asset.file_path {
        let refs = state
            .db
            .content_assets()
            .count_documents(doc! { "workspace_id": &admin.current_workspace, "file_path": &rel }, None)
            .await
            .unwrap_or(1); // 查询失败 → 视为有引用,保守不删
        if media_storage::should_delete_physical_file(refs) {
            let root = std::path::Path::new(&state.config.media_storage_dir);
            if let Err(e) = media_storage::delete_bytes(root, &rel).await {
                tracing::warn!("删除素材后物理文件删除失败（不影响删除）: {e}");
            }
        }
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_whitelist() {
        assert!(is_valid_media_type("image"));
        assert!(is_valid_media_type("file"));
        assert!(is_valid_media_type("video"));
        assert!(!is_valid_media_type("audio"));
        assert!(!is_valid_media_type(""));
        assert!(!is_valid_media_type("File"));
    }

    #[test]
    fn review_status_whitelist() {
        assert!(is_valid_review_status("approved"));
        assert!(is_valid_review_status("draft"));
        assert!(!is_valid_review_status("rejected"));
        assert!(!is_valid_review_status(""));
        assert!(!is_valid_review_status("Approved"));
    }
}

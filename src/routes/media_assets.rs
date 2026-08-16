//! 销售素材库：文件上传（multipart）+ 审核状态流转。
//!
//! 安全红线：
//! - 落盘路径**只能**经 [`media_storage::safe_relative_path`] 产出（workspace/sha 分片），
//!   绝不把用户传入的 `file_name` 拼进磁盘路径——原始文件名只存 DB 字段供展示。
//! - 文件大小受 `media_max_file_size_mb` 限制；扩展名/mime 走 `sanitize_ext` 白名单。
//! - 上传素材默认 `review_status="draft"`：AI 不自我核验红线，必须人类 approve 才可发。
use axum::{
    extract::{Multipart, Path, Query, State},
    Extension, Json,
};
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
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

/// An asset write must freeze the scope carried by the rendered entity.
/// Account-private assets match their exact account; workspace assets match
/// only `account_id: null` and are never inferred from the current account.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetScopeRequest {
    expected_scope: String,
    expected_account_id: Option<String>,
}

fn content_asset_scope_filter(
    oid: ObjectId,
    workspace_id: &str,
    scope: &AssetScopeRequest,
) -> AppResult<Document> {
    let mut filter = doc! { "_id": oid, "workspace_id": workspace_id };
    match scope.expected_scope.trim() {
        "account" => {
            let account_id = scope
                .expected_account_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "expectedAccountId is required for account scope".to_string(),
                    )
                })?;
            filter.insert("account_id", account_id);
        }
        "workspace" => {
            if scope
                .expected_account_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err(AppError::BadRequest(
                    "expectedAccountId must be empty for workspace scope".to_string(),
                ));
            }
            filter.insert("account_id", Bson::Null);
        }
        _ => {
            return Err(AppError::BadRequest(
                "expectedScope must be account|workspace".to_string(),
            ));
        }
    }
    Ok(filter)
}

async fn find_content_asset_for_scope(
    state: &AppState,
    workspace_id: &str,
    oid: ObjectId,
    scope: &AssetScopeRequest,
) -> AppResult<ContentAsset> {
    let asset = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": workspace_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;
    let matches = match scope.expected_scope.trim() {
        "account" => scope
            .expected_account_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(|expected| asset.account_id.as_deref() == Some(expected)),
        "workspace" => {
            scope
                .expected_account_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
                && asset.account_id.is_none()
        }
        _ => false,
    };
    // Also run protocol validation so malformed requests remain 400 rather
    // than being collapsed into an identity conflict.
    let _ = content_asset_scope_filter(oid, workspace_id, scope)?;
    if !matches {
        return Err(AppError::Conflict("content_asset_scope_conflict".into()));
    }
    Ok(asset)
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
    let mut tags: Vec<String> = vec![];
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
            "tags" => {
                // 逗号分隔（与 targetStages 同款解析），供候选清单注入 + list 按 tag 检索。
                tags = field
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
        &admin.current_workspace,
        scope,
        &target_stages,
    )
    .await
    .map_err(|reason| AppError::BadRequest(format!("target_stages 校验未通过：{reason}")))?;

    // 文件协议：同 SHA 路径加进程锁，先写同目录 pending，再提交 Mongo，最后
    // 原子 rename 发布。DB/rename 任一步失败均补偿；崩溃窗口由 reconciler 修复。
    let sha = media_storage::sha256_hex(&bytes);
    let rel = media_storage::safe_relative_path(&admin.current_workspace, &sha, &ext)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let root = std::path::Path::new(&state.config.media_storage_dir);
    let _path_guards = media_storage::lock_paths(root, [rel.clone()])
        .await
        .map_err(|e| AppError::External(format!("lock media path failed: {e}")))?;
    let staged = media_storage::stage_bytes(root, &rel, &bytes)
        .await
        .map_err(|e| AppError::External(format!("stage file failed: {e}")))?;

    let asset = ContentAsset {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id,
        kind: "media".into(),
        title,
        body: None,
        tags,
        url: None,
        media_id: None,
        usage_scene: None,
        media_type: Some(media_type),
        file_path: Some(rel.clone()),
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
        min_inject_tier: None,
        enabled: None,
        allowed_insertion_levels: None,
        usage_guidance: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let res = match state.db.content_assets().insert_one(asset, None).await {
        Ok(result) => result,
        Err(error) => {
            if staged {
                if let Err(settle_error) =
                    media_storage::settle_staged_after_db_failure(&state.db, root, &rel).await
                {
                    tracing::warn!(?settle_error, path = %rel, "failed to settle staged upload");
                }
            }
            return Err(error.into());
        }
    };
    if staged {
        if let Err(error) = media_storage::publish_staged(root, &rel).await {
            let rollback = state
                .db
                .content_assets()
                .delete_one(doc! { "_id": &res.inserted_id }, None)
                .await;
            match rollback {
                Ok(result) if result.deleted_count == 1 => {
                    if let Err(settle_error) =
                        media_storage::settle_staged_after_db_failure(&state.db, root, &rel).await
                    {
                        tracing::warn!(?settle_error, path = %rel, "failed to settle upload after rollback");
                    }
                }
                Ok(_) | Err(_) => {
                    // Keep pending when rollback is uncertain. The reconciler
                    // will publish it if the DB reference survived.
                    tracing::error!(path = %rel, ?rollback, "upload publish failed and DB rollback was not confirmed");
                }
            }
            return Err(AppError::External(format!("publish file failed: {error}")));
        }
    }
    Ok(Json(
        json!({ "id": res.inserted_id.as_object_id().map(|i| i.to_hex()) }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    #[serde(flatten)]
    scope: AssetScopeRequest,
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
    let asset =
        find_content_asset_for_scope(&state, &admin.current_workspace, oid, &payload.scope).await?;
    let filter = content_asset_scope_filter(oid, &admin.current_workspace, &payload.scope)?;
    let res = state
        .db
        .content_assets()
        .update_one(
            filter,
            doc! { "$set": {
                "review_status": &payload.status,
                "review_note": payload.note.clone(),
                "updated_at": DateTime::now(),
            }},
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::Conflict("content_asset_scope_conflict".into()));
    }
    // 缺口 3：审计审核动作（谁把哪份素材改成什么状态）。回查拿 account_id/title。
    // fail-soft：审计写失败只 warn，不回滚 review（review 已生效=既成事实）。
    {
        let account_id = asset.account_id.clone().unwrap_or_default();
        let details = doc! {
            "asset_id": oid.to_hex(),
            "review_note": payload.note.clone().unwrap_or_default(),
            "reviewed_by": admin.username.clone(),
        };
        if let Err(e) = crate::agent::write_event_for_account(
            &state,
            &admin.current_workspace,
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
    #[serde(flatten)]
    scope: AssetScopeRequest,
    title: Option<String>,
    body: Option<String>,
    tags: Option<Vec<String>>,
    url: Option<String>,
    usage_scene: Option<String>,
    send_trigger_hint: Option<String>,
    expression_pref: Option<String>,
    target_stages: Option<Vec<String>>,
    requires_principal_approval: Option<bool>,
    min_inject_tier: Option<String>,
    enabled: Option<bool>,
    allowed_insertion_levels: Option<Vec<String>>,
    usage_guidance: Option<String>,
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
    let asset =
        find_content_asset_for_scope(&state, &admin.current_workspace, oid, &payload.scope).await?;

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
        let normalized = crate::agent::dimension_registry::normalize_target_stages(
            &state.db,
            &admin.current_workspace,
            scope,
            &stages,
        )
        .await
        .map_err(|reason| AppError::BadRequest(format!("target_stages 校验未通过：{reason}")))?;
        set.insert("target_stages", normalized);
    }
    if let Some(v) = payload.min_inject_tier {
        // 复用 create 路径同一归一化：闭集校验，非法/空落 "full"（防脏值进注入查询）。
        set.insert(
            "min_inject_tier",
            crate::routes::assets::normalize_min_inject_tier(Some(&v)),
        );
    }
    let text_governed = asset.kind != "media" && asset.kind != "forbidden_expression";
    if !text_governed
        && (payload.enabled.is_some()
            || payload.allowed_insertion_levels.is_some()
            || payload.usage_guidance.is_some())
    {
        return Err(AppError::BadRequest(
            "text governance fields are unavailable for this asset kind".into(),
        ));
    }
    if let Some(v) = payload.enabled {
        set.insert("enabled", v);
    }
    if let Some(levels) = payload.allowed_insertion_levels {
        set.insert(
            "allowed_insertion_levels",
            crate::routes::assets::normalize_insertion_levels(levels),
        );
    }
    if let Some(guidance) = payload.usage_guidance {
        set.insert("usage_guidance", guidance.trim().to_string());
    }
    set.insert("updated_at", DateTime::now());

    let result = state
        .db
        .content_assets()
        .update_one(
            content_asset_scope_filter(oid, &admin.current_workspace, &payload.scope)?,
            doc! { "$set": set },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict("content_asset_scope_conflict".into()));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 换文件的副作用语义（簇C 缺口4 红线）：换文件 = 发送物变了，必须
/// 退回草稿强制人类重审（AI 不自我核验红线）+ 清空 media_id 缓存（防
/// ensure_media_uploaded 在 TTL 内复用旧 media_id 导致 AI 发旧文件）。
/// 抽成纯函数让 handler 调用 + lib 测试钉死语义，防未来误改 $set 字面量。
pub(super) struct FileReplaceEffects {
    pub review_status: &'static str,
    pub clear_media_id: bool,
}

pub(super) fn file_replace_effects() -> FileReplaceEffects {
    FileReplaceEffects {
        review_status: "draft",
        clear_media_id: true,
    }
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
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name = String::new();
    let mut mime = String::new();
    let mut media_type = String::new();
    let mut expected_scope = String::new();
    let mut expected_account_id: Option<String> = None;
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
            "expectedScope" => expected_scope = field.text().await.unwrap_or_default(),
            "expectedAccountId" => {
                let value = field.text().await.unwrap_or_default();
                expected_account_id = (!value.trim().is_empty()).then_some(value);
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let scope = AssetScopeRequest {
        expected_scope,
        expected_account_id,
    };
    // Scope is verified before staging or publishing any bytes.
    let asset = find_content_asset_for_scope(&state, &admin.current_workspace, oid, &scope).await?;
    let old_file_path = asset.file_path.clone();
    if media_type.trim().is_empty() {
        media_type = asset.media_type.clone().unwrap_or_default();
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
    let mut lock_paths = vec![rel.clone()];
    if let Some(old) = old_file_path.as_ref() {
        lock_paths.push(old.clone());
    }
    let _path_guards = media_storage::lock_paths(root, lock_paths)
        .await
        .map_err(|e| AppError::External(format!("lock media paths failed: {e}")))?;

    // Re-read under the path locks. A preceding replace may have changed the
    // old path after our initial metadata lookup.
    let asset = find_content_asset_for_scope(&state, &admin.current_workspace, oid, &scope).await?;
    if asset.file_path != old_file_path {
        return Err(AppError::Conflict("asset file changed concurrently".into()));
    }
    let staged = media_storage::stage_bytes(root, &rel, &bytes)
        .await
        .map_err(|e| AppError::External(format!("stage file failed: {e}")))?;

    // 换文件副作用：清 media_id（防 TTL 内发旧文件）+ 退 draft（强制重审）。
    // 语义由 file_replace_effects() 纯函数钉死（lib 测试覆盖），勿改回字面量。
    let effects = file_replace_effects();
    let replacement_updated_at = DateTime::now();
    let mut set_doc = doc! {
        "file_path": &rel,
        "file_name": &file_name,
        "file_size": bytes.len() as i64,
        "mime_type": &mime,
        "file_sha256": &sha,
        "media_type": &media_type,
        "review_status": effects.review_status,
        "updated_at": replacement_updated_at,
    };
    // clear_media_id 语义驱动：清缓存的 media_id（置 null），防 ensure_media_uploaded
    // 在 TTL 内复用旧 media_id 导致 AI 发旧文件。
    if effects.clear_media_id {
        set_doc.insert("media_id", mongodb::bson::Bson::Null);
    }
    let mut replace_filter = content_asset_scope_filter(oid, &admin.current_workspace, &scope)?;
    replace_filter.insert(
        "file_path",
        old_file_path
            .clone()
            .map(Bson::String)
            .unwrap_or(Bson::Null),
    );
    replace_filter.insert("updated_at", asset.updated_at);
    let update = state
        .db
        .content_assets()
        .update_one(replace_filter, doc! { "$set": set_doc }, None)
        .await;
    let update = match update {
        Ok(result) if result.matched_count == 1 => result,
        Ok(_) => {
            if staged {
                let _ = media_storage::settle_staged_after_db_failure(&state.db, root, &rel).await;
            }
            return Err(AppError::Conflict("asset file changed concurrently".into()));
        }
        Err(error) => {
            if staged {
                let _ = media_storage::settle_staged_after_db_failure(&state.db, root, &rel).await;
            }
            return Err(error.into());
        }
    };
    let _ = update;

    if staged {
        if let Err(error) = media_storage::publish_staged(root, &rel).await {
            let mut rollback_set = doc! {
                "updated_at": asset.updated_at,
            };
            let mut rollback_unset = Document::new();
            macro_rules! restore_optional {
                ($field:literal, $value:expr) => {
                    if let Some(value) = $value.clone() {
                        rollback_set.insert($field, value);
                    } else {
                        rollback_unset.insert($field, "");
                    }
                };
            }
            restore_optional!("file_path", asset.file_path);
            restore_optional!("file_name", asset.file_name);
            restore_optional!("file_size", asset.file_size);
            restore_optional!("mime_type", asset.mime_type);
            restore_optional!("file_sha256", asset.file_sha256);
            restore_optional!("media_type", asset.media_type);
            restore_optional!("media_id", asset.media_id);
            restore_optional!("review_status", asset.review_status);
            let mut rollback_update = doc! { "$set": rollback_set };
            if !rollback_unset.is_empty() {
                rollback_update.insert("$unset", rollback_unset);
            }
            let mut rollback_filter =
                content_asset_scope_filter(oid, &admin.current_workspace, &scope)?;
            rollback_filter.insert("file_path", &rel);
            rollback_filter.insert("updated_at", replacement_updated_at);
            let rollback = state
                .db
                .content_assets()
                .update_one(rollback_filter, rollback_update, None)
                .await;
            match rollback {
                Ok(result) if result.matched_count == 1 => {
                    let _ =
                        media_storage::settle_staged_after_db_failure(&state.db, root, &rel).await;
                }
                Ok(_) | Err(_) => {
                    tracing::error!(path = %rel, ?rollback, "replace publish failed and DB rollback was not confirmed");
                }
            }
            return Err(AppError::External(format!("publish file failed: {error}")));
        }
    }

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
    #[serde(flatten)]
    scope: AssetScopeRequest,
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
    let _asset =
        find_content_asset_for_scope(&state, &admin.current_workspace, oid, &payload.scope).await?;
    let res = state
        .db
        .content_assets()
        .update_one(
            content_asset_scope_filter(oid, &admin.current_workspace, &payload.scope)?,
            doc! { "$set": { "sendable": payload.sendable, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::Conflict("content_asset_scope_conflict".into()));
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
    Query(scope): Query<AssetScopeRequest>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    // 回查拿 file_path（workspace 隔离）。
    let asset = find_content_asset_for_scope(&state, &admin.current_workspace, oid, &scope).await?;
    let old_file_path = asset.file_path.clone();

    let root = std::path::Path::new(&state.config.media_storage_dir);
    let _path_guards = match asset.file_path.as_ref() {
        Some(rel) => media_storage::lock_paths(root, [rel.clone()])
            .await
            .map_err(|e| AppError::External(format!("lock media path failed: {e}")))?,
        None => Vec::new(),
    };
    // Re-read under the path lock so delete and replace cannot release a stale
    // object path.
    let asset = find_content_asset_for_scope(&state, &admin.current_workspace, oid, &scope).await?;
    if asset.file_path != old_file_path {
        return Err(AppError::Conflict("asset file changed concurrently".into()));
    }

    let res = state
        .db
        .content_assets()
        .delete_one(
            content_asset_scope_filter(oid, &admin.current_workspace, &scope)?,
            None,
        )
        .await?;
    if res.deleted_count == 0 {
        return Err(AppError::Conflict("content_asset_scope_conflict".into()));
    }

    // 引用计数清理：本记录已删,count 同 file_path 剩余引用,为 0 才物理删。
    if let Some(rel) = asset.file_path {
        let refs = state
            .db
            .content_assets()
            .count_documents(
                doc! { "workspace_id": &admin.current_workspace, "file_path": &rel },
                None,
            )
            .await
            .unwrap_or(1); // 查询失败 → 视为有引用,保守不删
        if media_storage::should_delete_physical_file(refs) {
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

    #[test]
    fn file_replace_resets_to_draft_and_clears_media_id() {
        let e = file_replace_effects();
        assert_eq!(e.review_status, "draft", "换文件必须退回草稿强制重审");
        assert!(e.clear_media_id, "换文件必须清 media_id 防发旧文件");
    }

    #[test]
    fn update_meta_deserializes_min_inject_tier_camel_case() {
        // PUT 编辑路径必须能接收 minInjectTier（camelCase）→ 落 min_inject_tier。
        // 这把"前端编辑注入档被后端静默丢弃"的回归钉死。
        let payload: UpdateMetaRequest = serde_json::from_str(
            r#"{"expectedScope":"workspace","title":"话术A","minInjectTier":"lean"}"#,
        )
        .unwrap();
        assert_eq!(payload.min_inject_tier.as_deref(), Some("lean"));
        assert_eq!(payload.title.as_deref(), Some("话术A"));
        // 缺省时为 None（部分更新语义：不传则不动该字段）。
        let bare: UpdateMetaRequest =
            serde_json::from_str(r#"{"expectedScope":"workspace","title":"x"}"#).unwrap();
        assert_eq!(bare.min_inject_tier, None);
    }
}

//! LLM 服务商配置 admin 路由。
//!
//! 职责：把 `.env` 里的 `OPENAI_BASE_URL / OPENAI_API_KEY / OPENAI_MODEL`
//! 抬升为可在前端 UI 编辑、保存、测试连接、热切换的 DB 数据，并支持
//! `format=openai|anthropic` 两种协议形态。
//!
//! 路由（全部挂在 `/api/admin/llm-providers` 下）：
//!
//! - `GET    /admin/llm-providers`                列表（api_key 一律 mask）
//! - `POST   /admin/llm-providers`                新建
//! - `PUT    /admin/llm-providers/:id`            更新（id 为 provider_id slug）
//! - `DELETE /admin/llm-providers/:id`            删除（不允许删 active 那条）
//! - `POST   /admin/llm-providers/:id/activate`   切换 active；并热替换 LlmRegistry
//! - `POST   /admin/llm-providers/test`           测试连接（按 id 或裸 form）
//!
//! 安全：列表 / 详情接口对 `api_key` 一律 mask 成 `sk-****<last4>`；客户端写
//! 入若提交 `apiKey` 的 mask 形态（含 `****`），视为不更新该字段，沿用旧值。
//! test 接口接收的明文 key 只在内存中构造一次性 LlmClient，不入库。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    llm::{LlmClient, LlmFormat, LlmProvider, LlmProviderMeta},
    models::LlmProviderConfig,
    secret::mask_secret,
};

use super::AppState;

/// api_key mask：复用 [`crate::secret::mask_secret`]（保留前 3 + 后 4，
/// 中间 `****`）。本路由保留 `mask_api_key` 名称是为兼容已有调用站点；
/// 实现委托给共享 helper，与 Debug / tracing 输出口径统一。
fn mask_api_key(key: &str) -> String {
    mask_secret(key)
}

fn is_masked_value(value: &str) -> bool {
    value.contains("****")
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LlmProviderView {
    provider_id: String,
    name: String,
    format: String,
    base_url: String,
    api_key_masked: String,
    model: String,
    is_active: bool,
    timeout_seconds: Option<u64>,
    max_retries: Option<u32>,
    retry_base_ms: Option<u64>,
    supports_vision: bool,
    is_vision_active: bool,
    created_at: i64,
    updated_at: i64,
}

impl From<&LlmProviderConfig> for LlmProviderView {
    fn from(cfg: &LlmProviderConfig) -> Self {
        Self {
            provider_id: cfg.provider_id.clone(),
            name: cfg.name.clone(),
            format: LlmFormat::parse(&cfg.format)
                .map(|f| f.as_protocol().to_string())
                .unwrap_or_else(|_| cfg.format.clone()),
            base_url: cfg.base_url.clone(),
            api_key_masked: mask_api_key(&cfg.api_key),
            model: cfg.model.clone(),
            is_active: cfg.is_active,
            timeout_seconds: cfg.timeout_seconds,
            max_retries: cfg.max_retries,
            retry_base_ms: cfg.retry_base_ms,
            supports_vision: cfg.supports_vision,
            is_vision_active: cfg.is_vision_active,
            created_at: cfg.created_at.timestamp_millis(),
            updated_at: cfg.updated_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListQuery {
    workspace_id: Option<String>,
}

pub(super) async fn list_providers(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = params
        .workspace_id
        .unwrap_or_else(|| admin.current_workspace.clone());
    let mut cursor = state
        .db
        .llm_provider_configs()
        .find(doc! { "workspaceId": &workspace_id }, None)
        .await?;
    let mut items = Vec::new();
    while let Some(cfg) = cursor.try_next().await? {
        items.push(LlmProviderView::from(&cfg));
    }
    let active_meta = match &state.llm_registry {
        Some(reg) => Some(reg.current_meta().await),
        None => None,
    };
    Ok(Json(json!({
        "items": items,
        "active": active_meta.map(|m| json!({
            "providerId": m.provider_id,
            "format": m.format.as_protocol(),
            "model": m.model,
            "baseUrl": m.base_url,
        })),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpsertRequest {
    pub workspace_id: Option<String>,
    pub provider_id: String,
    pub name: String,
    pub format: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_seconds: Option<u64>,
    pub max_retries: Option<u32>,
    pub retry_base_ms: Option<u64>,
    #[serde(default)]
    pub supports_vision: Option<bool>,
}

pub(super) async fn create_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<UpsertRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id = body
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
    LlmFormat::parse(&body.format)?;
    if body.provider_id.trim().is_empty() {
        return Err(AppError::BadRequest("providerId 不能为空".to_string()));
    }
    if body.base_url.trim().is_empty()
        || body.api_key.trim().is_empty()
        || body.model.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "baseUrl / apiKey / model 不能为空".to_string(),
        ));
    }
    if is_masked_value(&body.api_key) {
        return Err(AppError::BadRequest(
            "apiKey 不能是已 mask 的占位串".to_string(),
        ));
    }
    let now = DateTime::now();
    let cfg = LlmProviderConfig {
        id: None,
        workspace_id: workspace_id.clone(),
        provider_id: body.provider_id.clone(),
        name: body.name.clone(),
        format: body.format.clone(),
        base_url: body.base_url.trim_end_matches('/').to_string(),
        api_key: body.api_key.clone(),
        model: body.model.clone(),
        is_active: false,
        timeout_seconds: body.timeout_seconds,
        max_retries: body.max_retries,
        retry_base_ms: body.retry_base_ms,
        supports_vision: body.supports_vision.unwrap_or(false),
        is_vision_active: false,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .llm_provider_configs()
        .insert_one(&cfg, None)
        .await
        .map_err(|err| {
            AppError::BadRequest(format!(
                "创建失败（可能 providerId 重复）: {err}"
            ))
        })?;
    Ok(Json(json!({ "item": LlmProviderView::from(&cfg) })))
}

pub(super) async fn update_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(provider_id): Path<String>,
    Json(body): Json<UpsertRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id = body
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
    LlmFormat::parse(&body.format)?;
    let existing = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider {provider_id} not found")))?;
    let api_key = if is_masked_value(&body.api_key) {
        existing.api_key.clone()
    } else {
        body.api_key.clone()
    };
    let now = DateTime::now();
    let mut update = doc! {
        "name": &body.name,
        "format": &body.format,
        "baseUrl": body.base_url.trim_end_matches('/').to_string(),
        "apiKey": &api_key,
        "model": &body.model,
        "updatedAt": now,
    };
    if let Some(v) = body.timeout_seconds {
        update.insert("timeoutSeconds", v as i64);
    }
    if let Some(v) = body.max_retries {
        update.insert("maxRetries", v as i64);
    }
    if let Some(v) = body.retry_base_ms {
        update.insert("retryBaseMs", v as i64);
    }
    if let Some(v) = body.supports_vision {
        update.insert("supportsVision", v);
    }
    state
        .db
        .llm_provider_configs()
        .update_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            doc! { "$set": update },
            None,
        )
        .await?;
    let refreshed = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("provider 更新后未找到".to_string()))?;
    // 若被更新的就是 active 那条，热切换一次。
    if refreshed.is_active {
        if let Some(reg) = &state.llm_registry {
            swap_registry(reg, &refreshed).await?;
        }
    }
    Ok(Json(json!({ "item": LlmProviderView::from(&refreshed) })))
}

pub(super) async fn delete_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(provider_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = params
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
    let existing = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider {provider_id} not found")))?;
    if existing.is_active {
        return Err(AppError::BadRequest(
            "请先启用其它 provider 再删除当前激活的配置".to_string(),
        ));
    }
    state
        .db
        .llm_provider_configs()
        .delete_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn activate_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(provider_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = params
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
    let target = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider {provider_id} not found")))?;
    let now = DateTime::now();
    state
        .db
        .llm_provider_configs()
        .update_many(
            doc! { "workspaceId": &workspace_id, "isActive": true, "providerId": { "$ne": &provider_id } },
            doc! { "$set": { "isActive": false, "updatedAt": now } },
            None,
        )
        .await?;
    state
        .db
        .llm_provider_configs()
        .update_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            doc! { "$set": { "isActive": true, "updatedAt": now } },
            None,
        )
        .await?;
    if let Some(reg) = &state.llm_registry {
        swap_registry(reg, &target).await?;
    }
    Ok(Json(
        json!({ "ok": true, "item": LlmProviderView::from(&target) }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VisionActivateRequest {
    pub workspace_id: Option<String>,
    /// `true` 指派为视觉模型；`false` 取消本 workspace 的视觉模型指派。
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_true() -> bool {
    true
}

/// #574：把某条 provider 指派为本 workspace 的专职视觉模型（或取消指派）。
///
/// - `active=true`：要求该 provider `supports_vision=true`，否则 400；先清掉同
///   workspace 其它 `is_vision_active`，再把本条置 true（保证至多一条）。
/// - `active=false`：仅把本条置 false。
///
/// 与 [`activate_provider`]（文字主模型）正交：不触碰 `is_active`，也不热切换
/// `LlmRegistry`——视觉模型按需在 `/import-apply-image` 里临时构造 client。
pub(super) async fn set_vision_active(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(provider_id): Path<String>,
    Json(body): Json<VisionActivateRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id = body
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
    let target = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider {provider_id} not found")))?;
    let now = DateTime::now();
    if body.active {
        if !target.supports_vision {
            return Err(AppError::BadRequest(
                "该 provider 未开启 supportsVision，不能指派为视觉模型".to_string(),
            ));
        }
        // 清掉同 workspace 其它视觉指派，保证至多一条。
        state
            .db
            .llm_provider_configs()
            .update_many(
                doc! { "workspaceId": &workspace_id, "isVisionActive": true, "providerId": { "$ne": &provider_id } },
                doc! { "$set": { "isVisionActive": false, "updatedAt": now } },
                None,
            )
            .await?;
    }
    state
        .db
        .llm_provider_configs()
        .update_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            doc! { "$set": { "isVisionActive": body.active, "updatedAt": now } },
            None,
        )
        .await?;
    let refreshed = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("provider 更新后未找到".to_string()))?;
    Ok(Json(
        json!({ "ok": true, "item": LlmProviderView::from(&refreshed) }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TestRequest {
    pub workspace_id: Option<String>,
    /// 优先级：若提供 providerId，按 DB 中该条配置测；否则取 inline 字段直接构造一次性 client。
    pub provider_id: Option<String>,
    pub format: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub timeout_seconds: Option<u64>,
}

pub(super) async fn test_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<TestRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id = body
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
    let (format, base_url, api_key, model, timeout) = if let Some(pid) = body
        .provider_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        let cfg = state
            .db
            .llm_provider_configs()
            .find_one(
                doc! { "workspaceId": &workspace_id, "providerId": pid },
                None,
            )
            .await?
            .ok_or_else(|| AppError::NotFound(format!("provider {pid} not found")))?;
        // 若客户端额外提供 inline 覆盖（编辑表单未保存即测试），按 inline 优先；
        // 但若 apiKey 是 mask 形态则继续用 DB 中的真值。
        let api_key = match body
            .api_key
            .as_ref()
            .filter(|k| !k.trim().is_empty() && !is_masked_value(k))
        {
            Some(k) => k.clone(),
            None => cfg.api_key.clone(),
        };
        let format = body.format.clone().unwrap_or(cfg.format.clone());
        let base_url = body.base_url.clone().unwrap_or(cfg.base_url.clone());
        let model = body.model.clone().unwrap_or(cfg.model.clone());
        let timeout = body
            .timeout_seconds
            .or(cfg.timeout_seconds)
            .unwrap_or(state.config.llm_timeout_seconds);
        (format, base_url, api_key, model, timeout)
    } else {
        let format = body
            .format
            .clone()
            .ok_or_else(|| AppError::BadRequest("format 必填".to_string()))?;
        let base_url = body
            .base_url
            .clone()
            .ok_or_else(|| AppError::BadRequest("baseUrl 必填".to_string()))?;
        let api_key = body
            .api_key
            .clone()
            .filter(|k| !k.trim().is_empty() && !is_masked_value(k))
            .ok_or_else(|| AppError::BadRequest("apiKey 必填且不能是 mask 占位".to_string()))?;
        let model = body
            .model
            .clone()
            .ok_or_else(|| AppError::BadRequest("model 必填".to_string()))?;
        let timeout = body
            .timeout_seconds
            .unwrap_or(state.config.llm_timeout_seconds);
        (format, base_url, api_key, model, timeout)
    };
    let fmt = LlmFormat::parse(&format)?;
    let client = LlmClient::with_format(
        base_url,
        api_key,
        model.clone(),
        fmt,
        timeout,
        // test 路径不重试：失败立刻返回，让前端看到真实错误而不是被退避吞掉时间。
        1,
        500,
    )
    .map_err(|e| AppError::External(format!("构造测试 client 失败: {e}")))?;
    let started = std::time::Instant::now();
    let user = "请回复一个 JSON：{\"ok\": true}";
    let result = client
        .generate_json("你是一个连通性测试助手。只输出严格 JSON。", user)
        .await;
    let elapsed_ms = started.elapsed().as_millis() as i64;
    match result {
        Ok(value) => Ok(Json(json!({
            "ok": true,
            "latencyMs": elapsed_ms,
            "preview": value,
        }))),
        Err(err) => match err {
            AppError::LlmUnavailable {
                kind,
                detail,
                hint,
                retry_count,
            } => Ok(Json(json!({
                "ok": false,
                "latencyMs": elapsed_ms,
                "error": {
                    "kind": kind,
                    "retryCount": retry_count,
                    "detail": detail,
                    "hint": hint,
                }
            }))),
            other => Ok(Json(json!({
                "ok": false,
                "latencyMs": elapsed_ms,
                "error": {
                    "kind": "other",
                    "detail": other.to_string(),
                }
            }))),
        },
    }
}

async fn swap_registry(
    reg: &std::sync::Arc<crate::llm::LlmRegistry>,
    cfg: &LlmProviderConfig,
) -> AppResult<()> {
    let fmt = LlmFormat::parse(&cfg.format)?;
    let client = LlmClient::with_format(
        cfg.base_url.clone(),
        cfg.api_key.clone(),
        cfg.model.clone(),
        fmt,
        cfg.timeout_seconds.unwrap_or(45),
        cfg.max_retries.unwrap_or(3),
        cfg.retry_base_ms.unwrap_or(1500),
    )
    .map_err(|e| AppError::External(format!("构造 LLM client 失败: {e}")))?;
    reg.swap(
        client,
        LlmProviderMeta {
            provider_id: cfg.provider_id.clone(),
            format: fmt,
            model: cfg.model.clone(),
            base_url: cfg.base_url.clone(),
        },
    )
    .await;
    Ok(())
}

#[allow(dead_code)]
fn _ensure_llm_provider_object_safe(_g: &dyn LlmProvider) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_api_key_redacts_middle_keeps_head_and_tail() {
        // sk-1234567890abcdef → "sk-****cdef"
        let masked = mask_api_key("sk-1234567890abcdef");
        assert!(masked.contains("****"));
        assert!(masked.starts_with("sk-"));
        assert!(masked.ends_with("cdef"));
        assert!(!masked.contains("1234567890ab"));
    }

    #[test]
    fn mask_api_key_short_key_fully_masked() {
        assert_eq!(mask_api_key("short"), "****");
        assert_eq!(mask_api_key("12345678"), "****");
    }

    #[test]
    fn is_masked_value_detects_placeholder() {
        assert!(is_masked_value("sk-****cdef"));
        assert!(is_masked_value("****"));
        assert!(!is_masked_value("sk-real-key-1234"));
        assert!(!is_masked_value(""));
    }

    /// 边界 1：客户端回传 mask 占位时，update_provider 必须沿用旧值，绝不把
    /// "sk-****cdef" 写回 DB 顶替真 key。
    #[test]
    fn update_keeps_existing_api_key_when_payload_is_masked() {
        let existing = "sk-real-secret-abc123";
        let payload = mask_api_key(existing);
        let resolved = if is_masked_value(&payload) {
            existing.to_string()
        } else {
            payload.clone()
        };
        assert_eq!(resolved, existing, "mask 占位必须不覆盖真 key");
    }

    /// 边界 2：客户端回传明文新 key 时，update_provider 必须采用新值。
    #[test]
    fn update_replaces_api_key_when_payload_is_real() {
        let existing = "sk-old-key";
        let payload = "sk-brand-new-key-xyz";
        let resolved = if is_masked_value(payload) {
            existing.to_string()
        } else {
            payload.to_string()
        };
        assert_eq!(resolved, payload);
    }
}

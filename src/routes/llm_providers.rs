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
use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::TransactionOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::LazyLock};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    llm::{LlmClient, LlmFormat, LlmProvider, LlmProviderMeta},
    models::LlmProviderConfig,
    secret::mask_secret,
};

use super::shared::resolve_authorized_workspace;
use super::AppState;

// Provider changes are rare admin operations. Serialize them in the current
// single-process deployment so DB active flags and workspace registry slots
// cannot interleave. A future multi-replica deployment must replace this with
// a distributed lease or transaction-backed compare-and-swap.
static LLM_PROVIDER_MUTATION_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

const ACTIVE_UPDATE_APPROVAL_TTL_MS: i64 = 10 * 60 * 1000;

#[derive(Clone)]
struct ActiveUpdateApproval {
    workspace_id: String,
    provider_id: String,
    admin_user_id: String,
    expected_updated_at: i64,
    draft_fingerprint: String,
    expires_at: i64,
}

static ACTIVE_UPDATE_APPROVALS: LazyLock<
    tokio::sync::Mutex<HashMap<String, ActiveUpdateApproval>>,
> = LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderDraftFingerprint<'a> {
    workspace_id: &'a str,
    provider_id: &'a str,
    name: &'a str,
    format: &'a str,
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    timeout_seconds: Option<u64>,
    max_retries: Option<u32>,
    retry_base_ms: Option<u64>,
    supports_vision: bool,
}

fn provider_draft_fingerprint(cfg: &LlmProviderConfig) -> AppResult<String> {
    let normalized_format = LlmFormat::parse(&cfg.format)?.as_protocol();
    let canonical = serde_json::to_vec(&ProviderDraftFingerprint {
        workspace_id: &cfg.workspace_id,
        provider_id: &cfg.provider_id,
        name: &cfg.name,
        format: normalized_format,
        base_url: cfg.base_url.trim_end_matches('/'),
        api_key: &cfg.api_key,
        model: &cfg.model,
        timeout_seconds: cfg.timeout_seconds,
        max_retries: cfg.max_retries,
        retry_base_ms: cfg.retry_base_ms,
        supports_vision: cfg.supports_vision,
    })?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

async fn issue_active_update_approval(
    workspace_id: &str,
    provider_id: &str,
    admin_user_id: &str,
    expected_updated_at: i64,
    draft_fingerprint: String,
) -> (String, i64) {
    let now = DateTime::now().timestamp_millis();
    let expires_at = now + ACTIVE_UPDATE_APPROVAL_TTL_MS;
    let token = uuid::Uuid::new_v4().to_string();
    let mut approvals = ACTIVE_UPDATE_APPROVALS.lock().await;
    approvals.retain(|_, approval| approval.expires_at > now);
    approvals.insert(
        token.clone(),
        ActiveUpdateApproval {
            workspace_id: workspace_id.to_string(),
            provider_id: provider_id.to_string(),
            admin_user_id: admin_user_id.to_string(),
            expected_updated_at,
            draft_fingerprint,
            expires_at,
        },
    );
    (token, expires_at)
}

async fn consume_active_update_approval(
    token: &str,
    workspace_id: &str,
    provider_id: &str,
    admin_user_id: &str,
    expected_updated_at: i64,
    draft_fingerprint: &str,
) -> AppResult<()> {
    let now = DateTime::now().timestamp_millis();
    let mut approvals = ACTIVE_UPDATE_APPROVALS.lock().await;
    approvals.retain(|_, approval| approval.expires_at > now);
    let approval = approvals.remove(token).ok_or_else(|| {
        AppError::Conflict("active_provider_test_approval_missing_or_expired".to_string())
    })?;
    if approval.workspace_id != workspace_id
        || approval.provider_id != provider_id
        || approval.admin_user_id != admin_user_id
        || approval.expected_updated_at != expected_updated_at
        || approval.draft_fingerprint != draft_fingerprint
    {
        return Err(AppError::Conflict(
            "active_provider_test_approval_mismatch".to_string(),
        ));
    }
    Ok(())
}

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
    effective_timeout_seconds: u64,
    timeout_seconds_source: &'static str,
    max_retries: Option<u32>,
    effective_max_retries: u32,
    max_retries_source: &'static str,
    retry_base_ms: Option<u64>,
    effective_retry_base_ms: u64,
    retry_base_ms_source: &'static str,
    supports_vision: bool,
    is_vision_active: bool,
    created_at: i64,
    updated_at: i64,
}

impl LlmProviderView {
    fn from_config(cfg: &LlmProviderConfig, defaults: &crate::config::AppConfig) -> Self {
        Self::from_defaults(
            cfg,
            defaults.llm_timeout_seconds,
            defaults.llm_max_retries,
            defaults.llm_retry_base_ms,
        )
    }

    fn from_defaults(
        cfg: &LlmProviderConfig,
        default_timeout_seconds: u64,
        default_max_retries: u32,
        default_retry_base_ms: u64,
    ) -> Self {
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
            effective_timeout_seconds: cfg.timeout_seconds.unwrap_or(default_timeout_seconds),
            timeout_seconds_source: if cfg.timeout_seconds.is_some() {
                "provider"
            } else {
                "global_default"
            },
            max_retries: cfg.max_retries,
            effective_max_retries: cfg.max_retries.unwrap_or(default_max_retries),
            max_retries_source: if cfg.max_retries.is_some() {
                "provider"
            } else {
                "global_default"
            },
            retry_base_ms: cfg.retry_base_ms,
            effective_retry_base_ms: cfg.retry_base_ms.unwrap_or(default_retry_base_ms),
            retry_base_ms_source: if cfg.retry_base_ms.is_some() {
                "provider"
            } else {
                "global_default"
            },
            supports_vision: cfg.supports_vision,
            is_vision_active: cfg.is_vision_active,
            created_at: cfg.created_at.timestamp_millis(),
            updated_at: cfg.updated_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum NullablePatch<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<'de, T> Deserialize<'de> for NullablePatch<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

impl<T: Copy> NullablePatch<T> {
    fn apply(self, current: Option<T>) -> Option<T> {
        match self {
            Self::Missing => current,
            Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }

    fn create_value(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Missing | Self::Null => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    workspace_id: Option<String>,
}

// pub（非默认私有）：h3_cross_tenant_idor.rs 集成测试需从 tests/ 直调
// list_providers 验证跨租户读泄漏被拒,仿 activate_provider 先例。
pub async fn list_providers(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id = resolve_authorized_workspace(&state, &admin, params.workspace_id).await?;
    let mut cursor = state
        .db
        .llm_provider_configs()
        .find(doc! { "workspaceId": &workspace_id }, None)
        .await?;
    let mut items = Vec::new();
    while let Some(cfg) = cursor.try_next().await? {
        items.push(LlmProviderView::from_config(&cfg, &state.config));
    }
    let active_meta = match &state.llm_registry {
        Some(reg) => reg.current_meta(&workspace_id).await,
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
    #[serde(default)]
    timeout_seconds: NullablePatch<u64>,
    #[serde(default)]
    max_retries: NullablePatch<u32>,
    #[serde(default)]
    retry_base_ms: NullablePatch<u64>,
    #[serde(default)]
    pub supports_vision: Option<bool>,
    #[serde(default)]
    pub expected_updated_at: Option<i64>,
    #[serde(default)]
    pub active_update_confirmed: bool,
    #[serde(default)]
    pub active_update_test_token: Option<String>,
}

fn refreshed_provider_from_upsert(
    existing: &LlmProviderConfig,
    body: &UpsertRequest,
) -> AppResult<LlmProviderConfig> {
    if body.provider_id.trim() != existing.provider_id {
        return Err(AppError::Conflict("provider_identity_conflict".to_string()));
    }
    LlmFormat::parse(&body.format)?;
    let api_key = if is_masked_value(&body.api_key) {
        existing.api_key.clone()
    } else {
        body.api_key.clone()
    };
    let mut refreshed = existing.clone();
    refreshed.name = body.name.clone();
    refreshed.format = body.format.clone();
    refreshed.base_url = body.base_url.trim_end_matches('/').to_string();
    refreshed.api_key = api_key;
    refreshed.model = body.model.clone();
    refreshed.timeout_seconds = body.timeout_seconds.apply(existing.timeout_seconds);
    refreshed.max_retries = body.max_retries.apply(existing.max_retries);
    refreshed.retry_base_ms = body.retry_base_ms.apply(existing.retry_base_ms);
    if let Some(v) = body.supports_vision {
        if existing.is_vision_active && !v {
            return Err(AppError::Conflict(
                "vision_provider_must_be_unassigned_before_disabling".to_string(),
            ));
        }
        refreshed.supports_vision = v;
    }
    Ok(refreshed)
}

pub(super) async fn create_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<UpsertRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id =
        resolve_authorized_workspace(&state, &admin, body.workspace_id.clone()).await?;
    let fmt = LlmFormat::parse(&body.format)?;
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
        timeout_seconds: body.timeout_seconds.create_value(),
        max_retries: body.max_retries.create_value(),
        retry_base_ms: body.retry_base_ms.create_value(),
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
        .map_err(|err| AppError::BadRequest(format!("创建失败（可能 providerId 重复）: {err}")))?;
    // KD-09：openai 形态 base_url 缺 /v1 时软提示（不阻断保存）。cfg.base_url 已 trim :178，
    // fmt 复用上方 :154 已解析的合法值，不重复 parse。
    let warning = base_url_v1_warning(fmt, &cfg.base_url);
    if let Some(w) = &warning {
        tracing::warn!("provider {} base_url 软校验: {w}", cfg.provider_id);
    }
    let mut resp = json!({ "item": LlmProviderView::from_config(&cfg, &state.config) });
    if let Some(w) = warning {
        resp["warning"] = json!(w);
    }
    Ok(Json(resp))
}

pub(super) async fn update_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(provider_id): Path<String>,
    Json(body): Json<UpsertRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id =
        resolve_authorized_workspace(&state, &admin, body.workspace_id.clone()).await?;
    let _mutation_guard = LLM_PROVIDER_MUTATION_LOCK.lock().await;
    let existing = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider {provider_id} not found")))?;
    let mut refreshed = refreshed_provider_from_upsert(&existing, &body)?;
    if existing.is_active {
        let expected_updated_at = body.expected_updated_at.ok_or_else(|| {
            AppError::Conflict("active_provider_expected_updated_at_required".to_string())
        })?;
        if expected_updated_at != existing.updated_at.timestamp_millis() {
            return Err(AppError::Conflict(
                "active_provider_revision_changed".to_string(),
            ));
        }
        if !body.active_update_confirmed {
            return Err(AppError::Conflict(
                "active_provider_explicit_confirmation_required".to_string(),
            ));
        }
        let token = body
            .active_update_test_token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AppError::Conflict("active_provider_test_approval_required".to_string())
            })?;
        let fingerprint = provider_draft_fingerprint(&refreshed)?;
        consume_active_update_approval(
            token,
            &workspace_id,
            &provider_id,
            &admin.user_id,
            expected_updated_at,
            &fingerprint,
        )
        .await?;
    }
    let now = DateTime::from_millis(
        DateTime::now()
            .timestamp_millis()
            .max(existing.updated_at.timestamp_millis().saturating_add(1)),
    );
    refreshed.updated_at = now;
    let runtime_entry = if refreshed.is_active && state.llm_registry.is_some() {
        Some(build_registry_entry(&state, &refreshed)?)
    } else {
        None
    };
    let mut update = doc! {
        "name": &body.name,
        "format": &body.format,
        "baseUrl": body.base_url.trim_end_matches('/').to_string(),
        "apiKey": &refreshed.api_key,
        "model": &body.model,
        "updatedAt": now,
    };
    let mut unset = Document::new();
    match body.timeout_seconds {
        NullablePatch::Missing => {}
        NullablePatch::Null => {
            unset.insert("timeoutSeconds", "");
        }
        NullablePatch::Value(v) => {
            update.insert("timeoutSeconds", v as i64);
        }
    }
    match body.max_retries {
        NullablePatch::Missing => {}
        NullablePatch::Null => {
            unset.insert("maxRetries", "");
        }
        NullablePatch::Value(v) => {
            update.insert("maxRetries", v as i64);
        }
    }
    match body.retry_base_ms {
        NullablePatch::Missing => {}
        NullablePatch::Null => {
            unset.insert("retryBaseMs", "");
        }
        NullablePatch::Value(v) => {
            update.insert("retryBaseMs", v as i64);
        }
    }
    if let Some(v) = body.supports_vision {
        update.insert("supportsVision", v);
    }
    let mut update_filter = doc! {
        "workspaceId": &workspace_id,
        "providerId": &provider_id,
    };
    if existing.is_active {
        update_filter.insert("isActive", true);
        update_filter.insert("updatedAt", existing.updated_at);
        update_filter.insert("name", &existing.name);
        update_filter.insert("format", &existing.format);
        update_filter.insert("baseUrl", &existing.base_url);
        update_filter.insert("apiKey", &existing.api_key);
        update_filter.insert("model", &existing.model);
        update_filter.insert(
            "timeoutSeconds",
            existing.timeout_seconds.map(|value| value as i64),
        );
        update_filter.insert("maxRetries", existing.max_retries.map(|value| value as i64));
        update_filter.insert(
            "retryBaseMs",
            existing.retry_base_ms.map(|value| value as i64),
        );
        update_filter.insert("supportsVision", existing.supports_vision);
        update_filter.insert("isVisionActive", existing.is_vision_active);
    }
    if body.supports_vision == Some(false) {
        // A different replica may assign this provider for vision after our
        // pre-read. Never let that race persist isVisionActive=true together
        // with supportsVision=false.
        update_filter.insert("isVisionActive", false);
    }
    let mut update_document = doc! { "$set": update };
    if !unset.is_empty() {
        update_document.insert("$unset", unset);
    }
    let update_result = state
        .db
        .llm_provider_configs()
        .update_one(update_filter, update_document, None)
        .await?;
    if update_result.matched_count != 1 && existing.is_active {
        return Err(AppError::Conflict(
            "active_provider_revision_changed".to_string(),
        ));
    }
    if update_result.matched_count != 1 && body.supports_vision == Some(false) {
        return Err(AppError::Conflict(
            "vision_provider_assignment_changed".to_string(),
        ));
    }
    if update_result.matched_count != 1 {
        return Err(AppError::NotFound(format!(
            "provider {provider_id} disappeared during update"
        )));
    }
    // DB write succeeded; replacing an existing workspace slot is in-memory
    // and infallible because the client was constructed before the write.
    if let (Some(reg), Some((client, meta))) = (&state.llm_registry, runtime_entry) {
        reg.swap(&workspace_id, client, meta).await;
    }
    // KD-09：用 refreshed.format/base_url（实际存库值）软校验；refreshed.format 由上方 :210
    // 已验的 body.format 写入，同样合法，parse 不会失败。
    let warning = base_url_v1_warning(LlmFormat::parse(&refreshed.format)?, &refreshed.base_url);
    if let Some(w) = &warning {
        tracing::warn!("provider {} base_url 软校验: {w}", refreshed.provider_id);
    }
    let mut resp = json!({ "item": LlmProviderView::from_config(&refreshed, &state.config) });
    if let Some(w) = warning {
        resp["warning"] = json!(w);
    }
    Ok(Json(resp))
}

pub(super) async fn delete_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(provider_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id =
        resolve_authorized_workspace(&state, &admin, params.workspace_id.clone()).await?;
    let _mutation_guard = LLM_PROVIDER_MUTATION_LOCK.lock().await;
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
    if existing.is_vision_active {
        return Err(AppError::Conflict(
            "vision_provider_must_be_unassigned_before_delete".to_string(),
        ));
    }
    let delete_result = state
        .db
        .llm_provider_configs()
        .delete_one(
            doc! {
                "workspaceId": &workspace_id,
                "providerId": &provider_id,
                "isActive": false,
                "isVisionActive": false,
            },
            None,
        )
        .await?;
    if delete_result.deleted_count != 1 {
        return Err(AppError::Conflict(
            "provider_assignment_changed_during_delete".to_string(),
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn activate_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(provider_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let workspace_id =
        resolve_authorized_workspace(&state, &admin, params.workspace_id.clone()).await?;
    let _mutation_guard = LLM_PROVIDER_MUTATION_LOCK.lock().await;
    let target = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider {provider_id} not found")))?;
    let runtime_entry = if state.llm_registry.is_some() {
        Some(build_registry_entry(&state, &target)?)
    } else {
        None
    };
    let mut previous_active_ids = Vec::new();
    let mut active_cursor = state
        .db
        .llm_provider_configs()
        .find(
            doc! { "workspaceId": &workspace_id, "isActive": true },
            None,
        )
        .await?;
    while let Some(active) = active_cursor.try_next().await? {
        previous_active_ids.push(active.provider_id);
    }
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
    let activate_result = state
        .db
        .llm_provider_configs()
        .update_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            doc! { "$set": { "isActive": true, "updatedAt": now } },
            None,
        )
        .await;
    let activated = match activate_result {
        Ok(result) if result.matched_count == 1 => true,
        Ok(_) => false,
        Err(error) => {
            restore_active_providers(&state, &workspace_id, &previous_active_ids).await;
            return Err(error.into());
        }
    };
    if !activated {
        restore_active_providers(&state, &workspace_id, &previous_active_ids).await;
        return Err(AppError::NotFound(format!(
            "provider {provider_id} disappeared during activation"
        )));
    }
    if let (Some(reg), Some((client, meta))) = (&state.llm_registry, runtime_entry) {
        reg.swap(&workspace_id, client, meta).await;
    }
    let mut activated_target = target;
    activated_target.is_active = true;
    activated_target.updated_at = now;
    Ok(Json(
        json!({ "ok": true, "item": LlmProviderView::from_config(&activated_target, &state.config) }),
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
/// - `active=true`：要求该 provider `supports_vision=true`；事务内清掉同
///   workspace 旧指派并把本条置 true。partial unique 索引阻止多副本并发双指派。
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
    let workspace_id =
        resolve_authorized_workspace(&state, &admin, body.workspace_id.clone()).await?;
    let _mutation_guard = LLM_PROVIDER_MUTATION_LOCK.lock().await;
    let target = state
        .db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("provider {provider_id} not found")))?;
    if !body.active {
        let now = DateTime::now();
        let result = state
            .db
            .llm_provider_configs()
            .update_one(
                doc! {
                    "workspaceId": &workspace_id,
                    "providerId": &provider_id,
                    "isVisionActive": true,
                },
                doc! { "$set": { "isVisionActive": false, "updatedAt": now } },
                None,
            )
            .await?;
        let mut refreshed = target;
        if result.matched_count == 1 {
            refreshed.is_vision_active = false;
            refreshed.updated_at = now;
        }
        return Ok(Json(
            json!({ "ok": true, "item": LlmProviderView::from_config(&refreshed, &state.config) }),
        ));
    }
    if !target.supports_vision {
        return Err(AppError::BadRequest(
            "该 provider 未开启 supportsVision，不能指派为视觉模型".to_string(),
        ));
    }

    let coll = state.db.llm_provider_configs();
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let transaction_result: AppResult<LlmProviderConfig> = async {
        let mut refreshed = coll
            .find_one_with_session(
                doc! {
                    "workspaceId": &workspace_id,
                    "providerId": &provider_id,
                    "supportsVision": true,
                },
                None,
                &mut session,
            )
            .await?
            .ok_or_else(|| AppError::Conflict("vision_provider_capability_changed".to_string()))?;
        if refreshed.is_vision_active {
            return Ok(refreshed);
        }
        let now = DateTime::now();
        coll.update_many_with_session(
            doc! {
                "workspaceId": &workspace_id,
                "isVisionActive": true,
                "providerId": { "$ne": &provider_id },
            },
            doc! { "$set": { "isVisionActive": false, "updatedAt": now } },
            None,
            &mut session,
        )
        .await?;
        let promoted = coll
            .update_one_with_session(
                doc! {
                    "workspaceId": &workspace_id,
                    "providerId": &provider_id,
                    "supportsVision": true,
                    "isVisionActive": { "$ne": true },
                },
                doc! { "$set": { "isVisionActive": true, "updatedAt": now } },
                None,
                &mut session,
            )
            .await?;
        if promoted.modified_count != 1 {
            return Err(AppError::Conflict(
                "vision_provider_assignment_changed".to_string(),
            ));
        }
        refreshed.is_vision_active = true;
        refreshed.updated_at = now;
        Ok(refreshed)
    }
    .await;
    let refreshed = match transaction_result {
        Ok(refreshed) => refreshed,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(match error {
                AppError::Db(db_error) => {
                    tracing::warn!(error = %db_error, "vision provider reassignment conflicted");
                    AppError::Conflict("vision_provider_assignment_conflict".to_string())
                }
                other => other,
            });
        }
    };
    loop {
        match session.commit_transaction().await {
            Ok(()) => break,
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => {
                let _ = session.abort_transaction().await;
                tracing::warn!(error = %error, "vision provider reassignment commit failed");
                return Err(AppError::Conflict(
                    "vision_provider_assignment_conflict".to_string(),
                ));
            }
        }
    }
    Ok(Json(
        json!({ "ok": true, "item": LlmProviderView::from_config(&refreshed, &state.config) }),
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
    #[serde(default)]
    timeout_seconds: NullablePatch<u64>,
    pub name: Option<String>,
    #[serde(default)]
    max_retries: NullablePatch<u32>,
    #[serde(default)]
    retry_base_ms: NullablePatch<u64>,
    pub supports_vision: Option<bool>,
}

pub(super) async fn test_provider(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<TestRequest>,
) -> AppResult<Json<Value>> {
    let workspace_id =
        resolve_authorized_workspace(&state, &admin, body.workspace_id.clone()).await?;
    let mut tested_candidate: Option<LlmProviderConfig> = None;
    let (format, base_url, api_key, model, timeout) =
        if let Some(pid) = body.provider_id.as_ref().filter(|s| !s.trim().is_empty()) {
            let mut cfg = state
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
            cfg.timeout_seconds = body.timeout_seconds.apply(cfg.timeout_seconds);
            cfg.max_retries = body.max_retries.apply(cfg.max_retries);
            cfg.retry_base_ms = body.retry_base_ms.apply(cfg.retry_base_ms);
            let timeout = cfg
                .timeout_seconds
                .unwrap_or(state.config.llm_timeout_seconds);
            cfg.name = body.name.clone().unwrap_or(cfg.name);
            cfg.format = format.clone();
            cfg.base_url = base_url.trim_end_matches('/').to_string();
            cfg.api_key = api_key.clone();
            cfg.model = model.clone();
            if let Some(value) = body.supports_vision {
                cfg.supports_vision = value;
            }
            tested_candidate = Some(cfg);
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
                .create_value()
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
    let admission = state
        .llm_concurrency
        .acquire(crate::llm_concurrency::LlmPriority::Foreground)
        .await;
    let result = client
        .generate_json("你是一个连通性测试助手。只输出严格 JSON。", user)
        .await;
    drop(admission);
    let elapsed_ms = started.elapsed().as_millis() as i64;
    match result {
        Ok(value) => {
            let mut response = json!({
                "ok": true,
                "latencyMs": elapsed_ms,
                "preview": value,
            });
            if let Some(candidate) = tested_candidate {
                let expected_updated_at = candidate.updated_at.timestamp_millis();
                let fingerprint = provider_draft_fingerprint(&candidate)?;
                let (token, expires_at) = issue_active_update_approval(
                    &workspace_id,
                    &candidate.provider_id,
                    &admin.user_id,
                    expected_updated_at,
                    fingerprint,
                )
                .await;
                response["activeUpdateApproval"] = json!({
                    "token": token,
                    "expectedUpdatedAt": expected_updated_at,
                    "expiresAt": expires_at,
                });
            }
            Ok(Json(response))
        }
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

fn build_registry_entry(
    state: &AppState,
    cfg: &LlmProviderConfig,
) -> AppResult<(LlmClient, LlmProviderMeta)> {
    let fmt = LlmFormat::parse(&cfg.format)?;
    let client = LlmClient::with_format(
        cfg.base_url.clone(),
        cfg.api_key.clone(),
        cfg.model.clone(),
        fmt,
        cfg.timeout_seconds
            .unwrap_or(state.config.llm_timeout_seconds),
        cfg.max_retries.unwrap_or(state.config.llm_max_retries),
        cfg.retry_base_ms.unwrap_or(state.config.llm_retry_base_ms),
    )
    .map_err(|e| AppError::External(format!("构造 LLM client 失败: {e}")))?;
    Ok((
        client,
        LlmProviderMeta {
            provider_id: cfg.provider_id.clone(),
            format: fmt,
            model: cfg.model.clone(),
            base_url: cfg.base_url.clone(),
        },
    ))
}

async fn restore_active_providers(state: &AppState, workspace_id: &str, provider_ids: &[String]) {
    if provider_ids.is_empty() {
        return;
    }
    if let Err(error) = state
        .db
        .llm_provider_configs()
        .update_many(
            doc! {
                "workspaceId": workspace_id,
                "providerId": { "$in": provider_ids.to_vec() },
            },
            doc! { "$set": { "isActive": true, "updatedAt": DateTime::now() } },
            None,
        )
        .await
    {
        tracing::error!(%workspace_id, ?error, "failed to restore active LLM provider flags");
    }
}

/// openai 形态 base_url 软校验（KD-09）：不以 /v1 结尾时返回 warning 文案（None=无警告）。
/// 非 openai 形态（messages 形态）请求路径 {base_url}/v1/messages 自带 /v1，不校验（返 None）。
/// 软提示不阻断保存——各家兼容端点路径不一（Azure/代理网关可能非 /v1），hard block 会误伤合法配置。
fn base_url_v1_warning(fmt: LlmFormat, base_url: &str) -> Option<String> {
    if fmt != LlmFormat::Openai {
        return None;
    }
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        return None;
    }
    Some(format!(
        "baseUrl \"{trimmed}\" 不以 /v1 结尾：OpenAI 形态请求路径为 {{baseUrl}}/chat/completions，\
         多数服务商需 baseUrl 含 /v1（如 https://api.deepseek.com/v1）。若你的服务商路径确不含 /v1 可忽略此提示。"
    ))
}

#[allow(dead_code)]
fn _ensure_llm_provider_object_safe(_g: &dyn LlmProvider) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval_test_provider() -> LlmProviderConfig {
        let now = DateTime::from_millis(1_700_000_000_000);
        LlmProviderConfig {
            id: None,
            workspace_id: "ws-approval".to_string(),
            provider_id: "provider-a".to_string(),
            name: "Provider A".to_string(),
            format: "chat".to_string(),
            base_url: "https://llm.example/v1".to_string(),
            api_key: "secret-key".to_string(),
            model: "model-a".to_string(),
            is_active: true,
            timeout_seconds: Some(30),
            max_retries: Some(2),
            retry_base_ms: Some(500),
            supports_vision: false,
            is_vision_active: false,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn active_update_approval_is_exact_and_single_use() {
        let candidate = approval_test_provider();
        let expected_updated_at = candidate.updated_at.timestamp_millis();
        let fingerprint = provider_draft_fingerprint(&candidate).unwrap();
        let (token, _) = issue_active_update_approval(
            &candidate.workspace_id,
            &candidate.provider_id,
            "admin-a",
            expected_updated_at,
            fingerprint.clone(),
        )
        .await;

        consume_active_update_approval(
            &token,
            &candidate.workspace_id,
            &candidate.provider_id,
            "admin-a",
            expected_updated_at,
            &fingerprint,
        )
        .await
        .expect("matching approval should be consumed");
        assert!(
            consume_active_update_approval(
                &token,
                &candidate.workspace_id,
                &candidate.provider_id,
                "admin-a",
                expected_updated_at,
                &fingerprint,
            )
            .await
            .is_err(),
            "an approval token must not be replayable"
        );
    }

    #[tokio::test]
    async fn active_update_approval_mismatch_consumes_token() {
        let candidate = approval_test_provider();
        let expected_updated_at = candidate.updated_at.timestamp_millis();
        let fingerprint = provider_draft_fingerprint(&candidate).unwrap();
        let (token, _) = issue_active_update_approval(
            &candidate.workspace_id,
            &candidate.provider_id,
            "admin-a",
            expected_updated_at,
            fingerprint.clone(),
        )
        .await;
        let mut changed = candidate.clone();
        changed.model = "model-b".to_string();
        let changed_fingerprint = provider_draft_fingerprint(&changed).unwrap();

        assert!(
            consume_active_update_approval(
                &token,
                &candidate.workspace_id,
                &candidate.provider_id,
                "admin-a",
                expected_updated_at,
                &changed_fingerprint,
            )
            .await
            .is_err(),
            "a changed draft must not match the tested capability"
        );
        assert!(
            consume_active_update_approval(
                &token,
                &candidate.workspace_id,
                &candidate.provider_id,
                "admin-a",
                expected_updated_at,
                &fingerprint,
            )
            .await
            .is_err(),
            "a mismatch attempt must burn the one-time capability"
        );
    }

    #[tokio::test]
    async fn active_update_approval_binds_admin_workspace_provider_and_revision() {
        let candidate = approval_test_provider();
        let expected_updated_at = candidate.updated_at.timestamp_millis();
        let fingerprint = provider_draft_fingerprint(&candidate).unwrap();
        for (workspace, provider, admin, revision) in [
            ("other-ws", "provider-a", "admin-a", expected_updated_at),
            ("ws-approval", "provider-b", "admin-a", expected_updated_at),
            ("ws-approval", "provider-a", "admin-b", expected_updated_at),
            (
                "ws-approval",
                "provider-a",
                "admin-a",
                expected_updated_at + 1,
            ),
        ] {
            let (token, _) = issue_active_update_approval(
                &candidate.workspace_id,
                &candidate.provider_id,
                "admin-a",
                expected_updated_at,
                fingerprint.clone(),
            )
            .await;
            assert!(consume_active_update_approval(
                &token,
                workspace,
                provider,
                admin,
                revision,
                &fingerprint,
            )
            .await
            .is_err());
        }
    }

    #[test]
    fn nullable_patch_distinguishes_missing_null_and_value() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PatchFixture {
            #[serde(default)]
            timeout_seconds: NullablePatch<u64>,
        }

        let missing: PatchFixture = serde_json::from_value(json!({})).unwrap();
        let null: PatchFixture = serde_json::from_value(json!({ "timeoutSeconds": null })).unwrap();
        let value: PatchFixture = serde_json::from_value(json!({ "timeoutSeconds": 17 })).unwrap();

        assert_eq!(missing.timeout_seconds, NullablePatch::Missing);
        assert_eq!(null.timeout_seconds, NullablePatch::Null);
        assert_eq!(value.timeout_seconds, NullablePatch::Value(17));
        assert_eq!(missing.timeout_seconds.apply(Some(30)), Some(30));
        assert_eq!(null.timeout_seconds.apply(Some(30)), None);
        assert_eq!(value.timeout_seconds.apply(Some(30)), Some(17));
    }

    #[test]
    fn provider_view_reports_effective_values_and_sources() {
        let mut provider = approval_test_provider();
        provider.timeout_seconds = None;
        provider.retry_base_ms = None;

        let view = LlmProviderView::from_defaults(&provider, 45, 5, 1_500);
        assert_eq!(view.effective_timeout_seconds, 45);
        assert_eq!(view.timeout_seconds_source, "global_default");
        assert_eq!(view.effective_max_retries, 2);
        assert_eq!(view.max_retries_source, "provider");
        assert_eq!(view.effective_retry_base_ms, 1_500);
        assert_eq!(view.retry_base_ms_source, "global_default");
    }

    #[test]
    fn mask_api_key_redacts_middle_keeps_head_and_tail() {
        // Synthetic input preserves the public key shape without resembling a live secret.
        let masked = mask_api_key("sk-synthetic-1234567890abcdef");
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
        let payload = "sk-synthetic-brand-new-key-xyz";
        let resolved = if is_masked_value(payload) {
            existing.to_string()
        } else {
            payload.to_string()
        };
        assert_eq!(resolved, payload);
    }

    #[test]
    fn base_url_v1_warning_openai_missing_v1_warns() {
        let w = base_url_v1_warning(LlmFormat::Openai, "https://api.deepseek.com");
        assert!(w.is_some(), "openai 缺 /v1 应 warning");
        assert!(w.unwrap().contains("/v1"));
    }

    #[test]
    fn base_url_v1_warning_openai_with_v1_ok() {
        assert!(base_url_v1_warning(LlmFormat::Openai, "https://api.deepseek.com/v1").is_none());
        // 尾斜杠 trim 后仍含 /v1
        assert!(base_url_v1_warning(LlmFormat::Openai, "https://api.deepseek.com/v1/").is_none());
    }

    #[test]
    fn base_url_v1_warning_messages_format_never_warns() {
        // 非 openai 形态（messages 形态）拼 /v1/messages 自带 /v1，base_url 不含 /v1 也不警告。
        // 用 parse("messages") 构造非 openai 形态，避免硬编码具体模型/品牌字面量（no-model-hint lint）。
        let fmt = LlmFormat::parse("messages").expect("messages 形态可解析");
        assert!(base_url_v1_warning(fmt, "https://api.example.com").is_none());
    }
}

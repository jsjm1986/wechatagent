//! P0 鉴权 admin REST：login / logout / me / switch_workspace。
//!
//! - `POST /api/auth/login` 接 username + password JSON，验密码 → 写
//!   `admin_sessions` → Set-Cookie `wa_session=<session_id>; HttpOnly; SameSite=Strict
//!   [; Secure]; Path=/; Max-Age=...`。
//! - `POST /api/auth/logout` 删 session 行 + 清 cookie（设 Max-Age=0）。
//! - `GET /api/auth/me` 经过 middleware 后必有 [`AuthenticatedAdmin`]，返回
//!   username/userId + 该 admin 可访问的 workspaces 列表 + 当前 currentWorkspace。
//! - `POST /api/auth/workspace` body `{workspaceId}` 切换 session 的当前 workspace；
//!   仅允许切到 admin.workspaces 列表内的 ws。
//!
//! 路由 mount 在 [`super::api_router`]；`/login` 走 middleware 白名单。

use axum::{extract::State, Extension, Json};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use cookie::time::Duration as CookieDuration;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{
    jwt::issue_jwt,
    session::{
        authenticate, create_session, delete_session, get_admin_user, update_session_workspace,
        AuthError,
    },
    AuthenticatedAdmin, SESSION_COOKIE_NAME,
};
use crate::error::{AppError, AppResult};

use super::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub(super) struct LoginResponse {
    pub username: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
    #[serde(rename = "currentWorkspace")]
    pub current_workspace: String,
}

pub(super) async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> AppResult<(CookieJar, Json<LoginResponse>)> {
    let username = req.username.trim();
    let password = req.password.as_str();
    if username.is_empty() || password.is_empty() {
        return Err(AppError::BadRequest(
            "username and password are required".into(),
        ));
    }
    let user = authenticate(&state.db, username, password)
        .await
        .map_err(map_auth_error)?;
    let ttl_hours = state.config.session_ttl_hours.max(1);
    let session = create_session(
        &state.db,
        &user,
        ttl_hours,
        &state.config.default_workspace_id,
    )
    .await
    .map_err(map_auth_error)?;
    let cookie = build_session_cookie(
        session.session_id.clone(),
        ttl_hours,
        state.config.session_cookie_secure,
    );
    let jar = jar.add(cookie);
    let body = LoginResponse {
        username: session.username.clone(),
        expires_at: session.expires_at.to_rfc3339(),
        current_workspace: session
            .current_workspace
            .unwrap_or_else(|| state.config.default_workspace_id.clone()),
    };
    Ok((jar, Json(body)))
}

pub(super) async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> AppResult<(CookieJar, Json<Value>)> {
    if let Some(c) = jar.get(SESSION_COOKIE_NAME) {
        if let Err(e) = delete_session(&state.db, c.value()).await {
            tracing::warn!("logout: delete_session failed: {}", e);
        }
    }
    let jar = jar.remove(Cookie::from(SESSION_COOKIE_NAME));
    Ok((jar, Json(json!({ "ok": true }))))
}

pub(super) async fn me(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let user = get_admin_user(&state.db, &admin.user_id)
        .await
        .map_err(map_auth_error)?;
    let workspaces = user
        .as_ref()
        .map(|u| u.workspaces.clone())
        .unwrap_or_default();
    let workspaces = if workspaces.is_empty() {
        vec![state.config.default_workspace_id.clone()]
    } else {
        workspaces
    };
    Ok(Json(json!({
        "username": admin.username,
        "userId": admin.user_id,
        "workspaces": workspaces,
        "currentWorkspace": admin.current_workspace,
    })))
}

#[derive(Debug, Deserialize)]
pub(super) struct SwitchWorkspaceRequest {
    #[serde(rename = "workspaceId")]
    pub workspace_id: String,
}

pub(super) async fn switch_workspace(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    jar: CookieJar,
    Json(req): Json<SwitchWorkspaceRequest>,
) -> AppResult<Json<Value>> {
    let target = req.workspace_id.trim();
    if target.is_empty() {
        return Err(AppError::BadRequest("workspaceId is required".into()));
    }
    let user = get_admin_user(&state.db, &admin.user_id)
        .await
        .map_err(map_auth_error)?
        .ok_or_else(|| AppError::Unauthorized("admin_user_not_found".into()))?;
    let allowed = if user.workspaces.is_empty() {
        target == state.config.default_workspace_id
    } else {
        user.workspaces.iter().any(|w| w == target)
    };
    if !allowed {
        return Err(AppError::BadRequest("workspace_not_in_user_acl".into()));
    }
    let session_id = jar
        .get(SESSION_COOKIE_NAME)
        .map(|c| c.value().to_string())
        .ok_or_else(|| AppError::Unauthorized("missing_session".into()))?;
    update_session_workspace(&state.db, &session_id, target)
        .await
        .map_err(map_auth_error)?;
    Ok(Json(json!({
        "ok": true,
        "currentWorkspace": target,
    })))
}

fn build_session_cookie(value: String, ttl_hours: i64, secure: bool) -> Cookie<'static> {
    let mut c = Cookie::new(SESSION_COOKIE_NAME, value);
    c.set_http_only(true);
    c.set_same_site(SameSite::Strict);
    c.set_secure(secure);
    c.set_path("/");
    c.set_max_age(CookieDuration::hours(ttl_hours));
    c
}

fn map_auth_error(e: AuthError) -> AppError {
    match e {
        AuthError::InvalidCredentials => AppError::Unauthorized("invalid_credentials".into()),
        AuthError::SessionExpired => AppError::Unauthorized("session_expired".into()),
        AuthError::SessionNotFound => AppError::Unauthorized("session_not_found".into()),
        AuthError::Password(_) => AppError::External(format!("password hashing: {}", e)),
        AuthError::Mongo(err) => AppError::Db(err),
    }
}

// ── P1-7：JWT RS256 公网 Bearer token ──────────────────────────────────
//
// `POST /api/auth/token` body `{username, password}` → 返回 RS256 JWT。
// 默认 ttl 60min（`JWT_TTL_MINUTES` 可调）。`JWT_ENABLED=false` 时整个路由
// 走 [`AppError::Unauthorized("jwt_disabled")`] 拒绝，避免误以为开了实际没开。

#[derive(Debug, Deserialize)]
pub(super) struct IssueTokenRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub(super) struct IssueTokenResponse {
    pub token: String,
    #[serde(rename = "tokenType")]
    pub token_type: &'static str,
    #[serde(rename = "expiresInMinutes")]
    pub expires_in_minutes: i64,
    #[serde(rename = "currentWorkspace")]
    pub current_workspace: String,
}

pub(super) async fn issue_token(
    State(state): State<AppState>,
    Json(req): Json<IssueTokenRequest>,
) -> AppResult<Json<IssueTokenResponse>> {
    if !state.config.jwt_enabled {
        return Err(AppError::Unauthorized("jwt_disabled".into()));
    }
    let keys = state
        .jwt_keys
        .as_ref()
        .ok_or_else(|| AppError::External("jwt_keys_missing".into()))?;
    let username = req.username.trim();
    let password = req.password.as_str();
    if username.is_empty() || password.is_empty() {
        return Err(AppError::BadRequest(
            "username and password are required".into(),
        ));
    }
    let user = authenticate(&state.db, username, password)
        .await
        .map_err(map_auth_error)?;
    let ws = user
        .default_workspace
        .clone()
        .or_else(|| user.workspaces.first().cloned())
        .unwrap_or_else(|| state.config.default_workspace_id.clone());
    let token = issue_jwt(keys, &user.user_id, &user.username, &ws)?;
    Ok(Json(IssueTokenResponse {
        token,
        token_type: "Bearer",
        expires_in_minutes: keys.ttl_minutes,
        current_workspace: ws,
    }))
}

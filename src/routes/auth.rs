//! P0 鉴权 admin REST：login / logout / me。
//!
//! - `POST /api/auth/login` 接 username + password JSON，验密码 → 写
//!   `admin_sessions` → Set-Cookie `wa_session=<session_id>; HttpOnly; SameSite=Strict
//!   [; Secure]; Path=/; Max-Age=...`。
//! - `POST /api/auth/logout` 删 session 行 + 清 cookie（设 Max-Age=0）。
//! - `GET /api/auth/me` 经过 middleware 后必有 [`AuthenticatedAdmin`]，原样返回。
//!
//! 路由 mount 在 [`super::api_router`]；`/login` 走 middleware 白名单。

use axum::{extract::State, Extension, Json};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use cookie::time::Duration as CookieDuration;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{
    session::{authenticate, create_session, delete_session, AuthError},
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
    let session = create_session(&state.db, &user, ttl_hours)
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
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> Json<Value> {
    Json(json!({
        "username": admin.username,
        "userId": admin.user_id,
    }))
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

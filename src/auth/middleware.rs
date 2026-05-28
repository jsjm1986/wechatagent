//! `/api/*` session 校验 middleware。
//!
//! 行为：
//! - 读 `wa_session` cookie → 查 `admin_sessions` → 校验 `expires_at > now` →
//!   注入 [`AuthenticatedAdmin`] 到 request extension，路由 handler 通过
//!   `Extension<AuthenticatedAdmin>` 拿到。
//! - 白名单（无需登录）：`/health`、`/auth/login`。注意 layer 挂在 `api_router`
//!   内部，进来的 `req.uri().path()` 已剥掉 `/api` 前缀。
//! - cookie 缺失 / session 不存在 / 已过期 → 直接返 401，前端拿到后统一跳
//!   `LoginScreen`（`AppError::Unauthorized` 在 `error.rs` 已映射 401）。
//!
//! TTL：admin_sessions 的 `expires_at` 同时是 mongo TTL index 字段，
//! 服务端兜底过期清理；本 middleware 自己也再校验一次时间窗，
//! 避免 TTL daemon 滞后窗口里的 session 还能用。

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use axum_extra::extract::cookie::CookieJar;

use crate::auth::session::{lookup_session, AuthError};
use crate::auth::{AuthenticatedAdmin, SESSION_COOKIE_NAME};
use crate::routes::AppState;

/// 不需要登录就能访问的路径（已剥 `/api` 前缀）。
fn is_public_path(path: &str) -> bool {
    matches!(path, "/health" | "/auth/login")
}

pub async fn require_session(
    State(state): State<AppState>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = req.uri().path().to_string();
    if is_public_path(&path) {
        return Ok(next.run(req).await);
    }

    let cookie = match jar.get(SESSION_COOKIE_NAME) {
        Some(c) => c,
        None => return Err(StatusCode::UNAUTHORIZED),
    };
    let session_id = cookie.value();

    let session = match lookup_session(&state.db, session_id).await {
        Ok(s) => s,
        Err(AuthError::SessionExpired) | Err(AuthError::SessionNotFound) => {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Err(e) => {
            tracing::warn!("session lookup failed: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    req.extensions_mut().insert(AuthenticatedAdmin {
        user_id: session.admin_user_id,
        username: session.username,
    });

    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_only_login_and_health() {
        assert!(is_public_path("/health"));
        assert!(is_public_path("/auth/login"));
        assert!(!is_public_path("/auth/logout"));
        assert!(!is_public_path("/auth/me"));
        assert!(!is_public_path("/contacts"));
        assert!(!is_public_path("/health/foo"));
        assert!(!is_public_path(""));
    }
}

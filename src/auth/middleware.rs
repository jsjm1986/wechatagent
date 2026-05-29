//! `/api/*` session 校验 middleware。
//!
//! 行为：
//! - 读 `wa_session` cookie → 查 `admin_sessions` → 校验 `expires_at > now` →
//!   注入 [`AuthenticatedAdmin`] 到 request extension，路由 handler 通过
//!   `Extension<AuthenticatedAdmin>` 拿到。
//! - cookie 路径失败且 `JWT_ENABLED=true` 时尝试 `Authorization: Bearer <jwt>`，
//!   RS256 验签 + exp 校验通过即注入 [`AuthenticatedAdmin`]，与 cookie 路径等效。
//! - 白名单（无需登录）：`/health`、`/auth/login`、`/auth/token`（JWT 自身签发）。
//!   注意 layer 挂在 `api_router` 内部，进来的 `req.uri().path()` 已剥掉 `/api` 前缀。
//! - cookie 缺失 / session 不存在 / 已过期 → 直接返 401，前端拿到后统一跳
//!   `LoginScreen`（`AppError::Unauthorized` 在 `error.rs` 已映射 401）。
//!
//! TTL：admin_sessions 的 `expires_at` 同时是 mongo TTL index 字段，
//! 服务端兜底过期清理；本 middleware 自己也再校验一次时间窗，
//! 避免 TTL daemon 滞后窗口里的 session 还能用。

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::cookie::CookieJar;

use crate::auth::jwt::verify_jwt;
use crate::auth::session::{lookup_session, AuthError};
use crate::auth::{AuthenticatedAdmin, SESSION_COOKIE_NAME};
use crate::routes::AppState;

/// 不需要登录就能访问的路径（已剥 `/api` 前缀）。
fn is_public_path(path: &str) -> bool {
    matches!(path, "/health" | "/auth/login" | "/auth/token")
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

    // ── 路径 1：cookie session ──
    if let Some(cookie) = jar.get(SESSION_COOKIE_NAME) {
        match lookup_session(&state.db, cookie.value()).await {
            Ok(session) => {
                req.extensions_mut().insert(AuthenticatedAdmin {
                    user_id: session.admin_user_id,
                    username: session.username,
                    current_workspace: session
                        .current_workspace
                        .unwrap_or_else(|| state.config.default_workspace_id.clone()),
                });
                return Ok(next.run(req).await);
            }
            Err(AuthError::SessionExpired) | Err(AuthError::SessionNotFound) => {
                // 落到 Bearer 路径再试一次。
            }
            Err(e) => {
                tracing::warn!("session lookup failed: {}", e);
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }

    // ── 路径 2：Authorization: Bearer <jwt> ──
    if state.config.jwt_enabled {
        if let Some(keys) = state.jwt_keys.as_ref() {
            if let Some(bearer) = extract_bearer(&req) {
                match verify_jwt(keys, bearer) {
                    Ok(claims) => {
                        req.extensions_mut().insert(AuthenticatedAdmin {
                            user_id: claims.sub,
                            username: claims.username,
                            current_workspace: claims.current_workspace,
                        });
                        return Ok(next.run(req).await);
                    }
                    Err(_) => return Err(StatusCode::UNAUTHORIZED),
                }
            }
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

fn extract_bearer(req: &Request) -> Option<&str> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_only_login_health_and_token() {
        assert!(is_public_path("/health"));
        assert!(is_public_path("/auth/login"));
        assert!(is_public_path("/auth/token"));
        assert!(!is_public_path("/auth/logout"));
        assert!(!is_public_path("/auth/me"));
        assert!(!is_public_path("/contacts"));
        assert!(!is_public_path("/health/foo"));
        assert!(!is_public_path(""));
    }
}

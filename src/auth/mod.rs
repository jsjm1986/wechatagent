//! P0 鉴权 / Session 模块：admin 用户、Argon2 密码哈希、cookie session。
//!
//! 边界：
//! - admin SPA 同 origin 走 HttpOnly cookie session，未登录禁止 `/api/*`（除白名单）。
//! - webhook `/webhooks/wechat` 不在 `/api` 下，单独走 HMAC 签名校验，不受 admin auth 影响。
//! - 不引入第三方 IdP，user/password 单一登录形态；多 admin 由首个 admin 在 admin UI 添加（后续）。
//!
//! 设计取舍：
//! - 密码用 Argon2id（OWASP 2024 推荐），盐自动 per-user，不做全局 pepper。
//! - session 走 mongo 一张表 `admin_sessions`，TTL index 自动过期；不放内存（重启不掉线 + 多实例就绪）。
//! - 不返 access/refresh 双 token；session 只有一个 cookie + DB 行，登出删行 + 清 cookie 即彻底失效。

pub mod middleware;
pub mod password;
pub mod session;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// admin 用户：username 唯一，password_hash 是 Argon2id PHC 字符串
/// （`$argon2id$v=19$m=...$...$...`，自带盐，可直接落库）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUser {
    pub user_id: String,
    pub username: String,
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

/// admin session：cookie 里只存 session_id；admin_user_id 反向关联。
/// `expires_at` 同时是 mongo TTL index 的字段（过期自动清理）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSession {
    pub session_id: String,
    pub admin_user_id: String,
    pub username: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// 通过 middleware 注入到 request extension 的已认证 admin 上下文。
/// 路由处理函数通过 `Extension<AuthenticatedAdmin>` 拿到。
#[derive(Debug, Clone)]
pub struct AuthenticatedAdmin {
    pub user_id: String,
    pub username: String,
}

/// session cookie 名。固定字面量，前后端约定一致。
pub const SESSION_COOKIE_NAME: &str = "wa_session";

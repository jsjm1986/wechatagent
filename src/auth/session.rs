//! admin_users + admin_sessions 的 mongo CRUD + 启动 bootstrap。
//!
//! 集合：
//! - `admin_users`：username 唯一索引
//! - `admin_sessions`：SHA-256 session token 摘要唯一 + expires_at TTL（mongo 自动过期清理）
//!
//! bootstrap：每次启动检查 env `BOOTSTRAP_ADMIN_USERNAME` + `BOOTSTRAP_ADMIN_PASSWORD`；
//! admin_users 集合空时创建第一个 admin。env 留着也幂等（admin 已存在就跳过）。

use chrono::{Duration, Utc};
use mongodb::bson::doc;
use mongodb::Collection;
use sha2::{Digest, Sha256};

use super::{password, AdminSession, AdminUser};
use crate::db::Database;

const ADMIN_USERS: &str = "admin_users";
const ADMIN_SESSIONS: &str = "admin_sessions";
const SESSION_DIGEST_PREFIX: &str = "sha256-v1:";

fn session_token_digest(session_token: &str) -> String {
    let digest = Sha256::digest(session_token.as_bytes());
    format!("{SESSION_DIGEST_PREFIX}{}", hex::encode(digest))
}

fn session_lookup_filter(session_token: &str) -> mongodb::bson::Document {
    let digest = session_token_digest(session_token);
    doc! { "session_id": { "$in": [digest, session_token] } }
}

async fn find_session_by_token(
    collection: &Collection<AdminSession>,
    session_token: &str,
) -> Result<Option<(AdminSession, bool)>, mongodb::error::Error> {
    let digest = session_token_digest(session_token);
    if let Some(session) = collection
        .find_one(doc! { "session_id": &digest }, None)
        .await?
    {
        return Ok(Some((session, false)));
    }
    Ok(collection
        .find_one(doc! { "session_id": session_token }, None)
        .await?
        .map(|session| (session, true)))
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("session expired")]
    SessionExpired,
    #[error("session not found")]
    SessionNotFound,
    #[error("admin has no authorized workspace")]
    NoAuthorizedWorkspace,
    #[error("password hashing failed: {0}")]
    Password(#[from] password::PasswordError),
    #[error("mongo: {0}")]
    Mongo(#[from] mongodb::error::Error),
}

fn admin_users(db: &Database) -> Collection<AdminUser> {
    db.raw().collection(ADMIN_USERS)
}

fn admin_sessions(db: &Database) -> Collection<AdminSession> {
    db.raw().collection(ADMIN_SESSIONS)
}

/// 启动时调用：当 admin_users 为空且 env 提供了 username+password 时创建第一个 admin。
/// 已存在 admin 则跳过（幂等）。env 缺一就跳过（不报错，便于本地开发）。
///
/// `default_workspace` 用 `config.default_workspace_id` 兜底；admin 可后续在
/// 治理面新增/编辑 workspace 列表。
pub async fn bootstrap_admin_if_needed(
    db: &Database,
    username: Option<&str>,
    password_plain: Option<&str>,
    default_workspace: Option<&str>,
) -> Result<bool, AuthError> {
    let (Some(username), Some(password_plain)) = (username, password_plain) else {
        return Ok(false);
    };
    let coll = admin_users(db);
    let existing = coll.estimated_document_count(None).await?;
    if existing > 0 {
        return Ok(false);
    }
    let workspaces = default_workspace
        .map(|w| vec![w.to_string()])
        .unwrap_or_default();
    let user = AdminUser {
        user_id: uuid::Uuid::new_v4().to_string(),
        username: username.to_string(),
        password_hash: password::hash_password(password_plain)?,
        created_at: Utc::now(),
        last_login_at: None,
        workspaces,
        default_workspace: default_workspace.map(|w| w.to_string()),
    };
    coll.insert_one(&user, None).await?;
    Ok(true)
}

/// 校验 username + password；成功则更新 last_login_at 并返回 AdminUser。
pub async fn authenticate(
    db: &Database,
    username: &str,
    password_plain: &str,
) -> Result<AdminUser, AuthError> {
    let coll = admin_users(db);
    let Some(user) = coll.find_one(doc! { "username": username }, None).await? else {
        // 用户名不存在时也跑一次 verify（对进程级假哈希），支付与"用户存在"等价的
        // Argon2 耗时，抹平枚举时序侧信道；恒判凭据无效。
        let _ = password::verify_against_dummy(password_plain);
        return Err(AuthError::InvalidCredentials);
    };
    let ok = password::verify_password(password_plain, &user.password_hash)?;
    if !ok {
        return Err(AuthError::InvalidCredentials);
    }
    let now = Utc::now();
    coll.update_one(
        doc! { "user_id": &user.user_id },
        // last_login_at 与 created_at 一样是 chrono DateTime<Utc>（serde 序列化为 RFC3339
        // 字符串）。这里必须写 RFC3339 字符串，不能写 bson::DateTime（会变成 BSON Date /
        // map，导致下次反序列化 AdminUser 时 "invalid type: map, expected RFC3339 string"）。
        doc! { "$set": { "last_login_at": now.to_rfc3339() } },
        None,
    )
    .await?;
    Ok(user)
}

/// 创建一条 session：返回给 cookie 的 token 使用 UUIDv4，Mongo 仅写其 SHA-256 摘要。
/// `current_workspace` 在登录时初始为 user.default_workspace（或 fallback 到
/// `config.default_workspace_id`）；后续可由 [`update_session_workspace`] 切换。
pub async fn create_session(
    db: &Database,
    user: &AdminUser,
    ttl_hours: i64,
    fallback_workspace: &str,
) -> Result<AdminSession, AuthError> {
    let now = Utc::now();
    let initial_ws = initial_authorized_workspace(user, fallback_workspace)
        .ok_or(AuthError::NoAuthorizedWorkspace)?;
    let session_token = uuid::Uuid::new_v4().to_string();
    let session = AdminSession {
        session_id: session_token.clone(),
        admin_user_id: user.user_id.clone(),
        username: user.username.clone(),
        created_at: now,
        expires_at: now + Duration::hours(ttl_hours.max(1)),
        current_workspace: Some(initial_ws),
    };
    let mut stored = session.clone();
    stored.session_id = session_token_digest(&session_token);
    admin_sessions(db).insert_one(&stored, None).await?;
    Ok(session)
}

/// Select the login workspace from the current ACL. A stale `default_workspace`
/// must never grant access after it has been removed from `workspaces`.
pub fn initial_authorized_workspace(user: &AdminUser, _fallback_workspace: &str) -> Option<String> {
    if user.workspaces.is_empty() {
        return None;
    }
    user.default_workspace
        .as_ref()
        .filter(|workspace| user.workspaces.iter().any(|allowed| allowed == *workspace))
        .cloned()
        .or_else(|| user.workspaces.first().cloned())
}

/// 拿 session_id 查 session；未找到 / 已过期都返错。不更新 expires_at（不滚动续期，
/// 若需要可以在后续加 sliding window；当前 7 天 TTL 够用）。
pub async fn lookup_session(db: &Database, session_token: &str) -> Result<AdminSession, AuthError> {
    let collection = admin_sessions(db);
    let (mut session, legacy_plaintext) = find_session_by_token(&collection, session_token)
        .await?
        .ok_or(AuthError::SessionNotFound)?;
    if session.expires_at <= Utc::now() {
        return Err(AuthError::SessionExpired);
    }
    if legacy_plaintext {
        let digest = session_token_digest(session_token);
        // Transparently migrate a pre-hash session after a successful lookup. Digest lookup
        // happens first, so an upgrade transition cannot ambiguously select a plaintext row.
        collection
            .update_one(
                doc! { "session_id": session_token },
                doc! { "$set": { "session_id": &digest } },
                None,
            )
            .await?;
    }
    // Never expose the stored digest as a bearer token to callers.
    session.session_id = session_token.to_string();
    Ok(session)
}

/// 删 session（登出）。session 不存在不报错——登出幂等。
pub async fn delete_session(db: &Database, session_token: &str) -> Result<(), AuthError> {
    admin_sessions(db)
        .delete_many(session_lookup_filter(session_token), None)
        .await?;
    Ok(())
}

/// 切换当前 session 的 workspace。caller 必须先校验目标 workspace 在
/// `admin_user.workspaces` 列表内（中间层做权限校验，本函数只写 DB）。
pub async fn update_session_workspace(
    db: &Database,
    session_token: &str,
    new_workspace: &str,
) -> Result<(), AuthError> {
    let collection = admin_sessions(db);
    let digest = session_token_digest(session_token);
    let update = doc! { "$set": { "current_workspace": new_workspace } };
    let result = collection
        .update_one(doc! { "session_id": &digest }, update.clone(), None)
        .await?;
    if result.matched_count == 0 {
        collection
            .update_one(doc! { "session_id": session_token }, update, None)
            .await?;
    }
    Ok(())
}

/// 按 user_id 查 admin user，用于切换 workspace 时校验权限。
pub async fn get_admin_user(db: &Database, user_id: &str) -> Result<Option<AdminUser>, AuthError> {
    let user = admin_users(db)
        .find_one(doc! { "user_id": user_id }, None)
        .await?;
    Ok(user)
}

#[cfg(test)]
mod session_token_tests {
    use super::*;

    #[test]
    fn session_digest_is_stable_prefixed_and_does_not_contain_token() {
        let token = "550e8400-e29b-41d4-a716-446655440000";
        let digest = session_token_digest(token);
        assert!(digest.starts_with(SESSION_DIGEST_PREFIX));
        assert_eq!(digest, session_token_digest(token));
        assert!(!digest.contains(token));
    }

    #[test]
    fn lookup_filter_accepts_digest_and_legacy_plaintext() {
        let token = "legacy-token";
        let filter = session_lookup_filter(token);
        let values = filter
            .get_document("session_id")
            .unwrap()
            .get_array("$in")
            .unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[1].as_str(), Some(token));
        assert_ne!(values[0].as_str(), Some(token));
    }
}

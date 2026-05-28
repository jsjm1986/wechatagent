//! admin_users + admin_sessions 的 mongo CRUD + 启动 bootstrap。
//!
//! 集合：
//! - `admin_users`：username 唯一索引
//! - `admin_sessions`：session_id 唯一 + expires_at TTL（mongo 自动过期清理）
//!
//! bootstrap：每次启动检查 env `BOOTSTRAP_ADMIN_USERNAME` + `BOOTSTRAP_ADMIN_PASSWORD`；
//! admin_users 集合空时创建第一个 admin。env 留着也幂等（admin 已存在就跳过）。

use chrono::{Duration, Utc};
use mongodb::bson::doc;
use mongodb::Collection;

use super::{password, AdminSession, AdminUser};
use crate::db::Database;

const ADMIN_USERS: &str = "admin_users";
const ADMIN_SESSIONS: &str = "admin_sessions";

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("session expired")]
    SessionExpired,
    #[error("session not found")]
    SessionNotFound,
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
pub async fn bootstrap_admin_if_needed(
    db: &Database,
    username: Option<&str>,
    password_plain: Option<&str>,
) -> Result<bool, AuthError> {
    let (Some(username), Some(password_plain)) = (username, password_plain) else {
        return Ok(false);
    };
    let coll = admin_users(db);
    let existing = coll.estimated_document_count(None).await?;
    if existing > 0 {
        return Ok(false);
    }
    let user = AdminUser {
        user_id: uuid::Uuid::new_v4().to_string(),
        username: username.to_string(),
        password_hash: password::hash_password(password_plain)?,
        created_at: Utc::now(),
        last_login_at: None,
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
    let user = coll
        .find_one(doc! { "username": username }, None)
        .await?
        .ok_or(AuthError::InvalidCredentials)?;
    let ok = password::verify_password(password_plain, &user.password_hash)?;
    if !ok {
        return Err(AuthError::InvalidCredentials);
    }
    let now = Utc::now();
    coll.update_one(
        doc! { "user_id": &user.user_id },
        doc! { "$set": { "last_login_at": mongodb::bson::DateTime::from_millis(now.timestamp_millis()) } },
        None,
    )
    .await?;
    Ok(user)
}

/// 创建一条 session（写 mongo + 返结构）。session_id 用 uuid v4。
pub async fn create_session(
    db: &Database,
    user: &AdminUser,
    ttl_hours: i64,
) -> Result<AdminSession, AuthError> {
    let now = Utc::now();
    let session = AdminSession {
        session_id: uuid::Uuid::new_v4().to_string(),
        admin_user_id: user.user_id.clone(),
        username: user.username.clone(),
        created_at: now,
        expires_at: now + Duration::hours(ttl_hours.max(1)),
    };
    admin_sessions(db).insert_one(&session, None).await?;
    Ok(session)
}

/// 拿 session_id 查 session；未找到 / 已过期都返错。不更新 expires_at（不滚动续期，
/// 若需要可以在后续加 sliding window；当前 7 天 TTL 够用）。
pub async fn lookup_session(
    db: &Database,
    session_id: &str,
) -> Result<AdminSession, AuthError> {
    let session = admin_sessions(db)
        .find_one(doc! { "session_id": session_id }, None)
        .await?
        .ok_or(AuthError::SessionNotFound)?;
    if session.expires_at <= Utc::now() {
        return Err(AuthError::SessionExpired);
    }
    Ok(session)
}

/// 删 session（登出）。session 不存在不报错——登出幂等。
pub async fn delete_session(db: &Database, session_id: &str) -> Result<(), AuthError> {
    admin_sessions(db)
        .delete_one(doc! { "session_id": session_id }, None)
        .await?;
    Ok(())
}

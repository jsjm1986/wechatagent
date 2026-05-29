//! P1-7：RS256 JWT 签发 / 校验。
//!
//! 公网 / 第三方调用走 `Authorization: Bearer <jwt>`；admin SPA 同 origin 仍用
//! cookie session。两条路径互不干扰，由 [`crate::auth::middleware::require_session`]
//! 先 cookie 后 Bearer 的顺序兼容。
//!
//! 设计取舍：
//! - **算法 RS256**：非对称签名，公钥可分发到下游服务校验，避免 HS256 共享密钥。
//! - **TTL 短**：默认 60min；refresh token 留 P2，首版只签 access token。
//! - **claims 只放 user_id / username / current_workspace**：与 cookie session
//!   注入的 [`crate::auth::AuthenticatedAdmin`] 等价，不复制 ACL；workspace 切换
//!   走重新签发。
//! - **PEM 双密钥来自 `JWT_PRIVATE_KEY_PEM` / `JWT_PUBLIC_KEY_PEM` env**；
//!   `jwt_enabled=true` 时 [`JwtKeys::from_config`] 必须返回 Ok，否则 main.rs 启动
//!   panic（"以为开了实际没开"防御）。
//!
//! [`verify_jwt`] 的失败模式（[`AppError::Unauthorized`] 错误码字面量）：
//! | 场景 | 错误码 |
//! |---|---|
//! | token 过期 | `token_expired` |
//! | 签名 / payload 篡改 | `token_invalid` |
//!
//! "Bearer 与 cookie 都缺" 不经本模块——[`crate::auth::middleware::require_session`]
//! 直接返裸 `401 StatusCode`（无 body），前端据此跳 `LoginScreen`。

use chrono::{Duration, Utc};
use jsonwebtoken::{
    decode, encode, errors::ErrorKind, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};

/// JWT 载荷。`exp` 由 [`issue_jwt`] 写入，单位是 unix 秒。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub username: String,
    pub current_workspace: String,
    pub exp: i64,
    pub iat: i64,
}

/// 启动期解码好的 RS256 公私钥对。`AppState` 持有一个 `Option<JwtKeys>`，
/// `jwt_enabled=false` 时为 None；为 Some 即代表密钥已加载、可签可验。
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    pub ttl_minutes: i64,
}

impl std::fmt::Debug for JwtKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtKeys")
            .field("ttl_minutes", &self.ttl_minutes)
            .finish_non_exhaustive()
    }
}

impl JwtKeys {
    /// 从 [`AppConfig`] 读 PEM。`jwt_enabled=true` 但任一密钥缺/格式错都返
    /// 错；main.rs 应把这个错向上传成启动 panic。
    pub fn from_config(config: &AppConfig) -> anyhow::Result<Self> {
        let private_pem = config
            .jwt_private_key_pem
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("JWT_ENABLED=true 但 JWT_PRIVATE_KEY_PEM 未配置"))?;
        let public_pem = config
            .jwt_public_key_pem
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("JWT_ENABLED=true 但 JWT_PUBLIC_KEY_PEM 未配置"))?;
        let encoding = EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .map_err(|e| anyhow::anyhow!("JWT_PRIVATE_KEY_PEM 解析失败: {e}"))?;
        let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .map_err(|e| anyhow::anyhow!("JWT_PUBLIC_KEY_PEM 解析失败: {e}"))?;
        Ok(Self {
            encoding,
            decoding,
            ttl_minutes: config.jwt_ttl_minutes.max(1),
        })
    }
}

/// 签发一个 RS256 JWT。`exp = now + ttl_minutes`。
pub fn issue_jwt(
    keys: &JwtKeys,
    user_id: &str,
    username: &str,
    current_workspace: &str,
) -> AppResult<String> {
    let now = Utc::now();
    let exp = now + Duration::minutes(keys.ttl_minutes);
    let claims = JwtClaims {
        sub: user_id.to_string(),
        username: username.to_string(),
        current_workspace: current_workspace.to_string(),
        iat: now.timestamp(),
        exp: exp.timestamp(),
    };
    encode(&Header::new(Algorithm::RS256), &claims, &keys.encoding)
        .map_err(|e| AppError::External(format!("jwt encode failed: {e}")))
}

/// 校验 JWT。过期 → `token_expired`；签名/载荷异常 → `token_invalid`。
pub fn verify_jwt(keys: &JwtKeys, token: &str) -> AppResult<JwtClaims> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = 0;
    match decode::<JwtClaims>(token, &keys.decoding, &validation) {
        Ok(data) => Ok(data.claims),
        Err(err) => match err.kind() {
            ErrorKind::ExpiredSignature => Err(AppError::Unauthorized("token_expired".into())),
            _ => Err(AppError::Unauthorized("token_invalid".into())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自签 RSA 2048 测试 keypair（仅用于单测；生产用 `openssl genrsa` 生成）。
    /// 使用 jsonwebtoken 自带的 `pem` 工具友好的 PKCS#8 / SPKI 格式。
    fn test_keys() -> JwtKeys {
        // 临时 2048-bit RSA test keypair（PKCS#8 私钥 + SPKI 公钥）。
        // 不参与生产逻辑，仅供单测验证 issue → verify → tampered 三条路径。
        const PRIVATE_PEM: &str = include_str!("../../tests/fixtures/jwt_test_private.pem");
        const PUBLIC_PEM: &str = include_str!("../../tests/fixtures/jwt_test_public.pem");
        let encoding = EncodingKey::from_rsa_pem(PRIVATE_PEM.as_bytes()).expect("private pem");
        let decoding = DecodingKey::from_rsa_pem(PUBLIC_PEM.as_bytes()).expect("public pem");
        JwtKeys {
            encoding,
            decoding,
            ttl_minutes: 5,
        }
    }

    #[test]
    fn issue_then_verify_round_trips_claims() {
        let keys = test_keys();
        let token = issue_jwt(&keys, "u1", "alice", "ws_default").expect("issue");
        let claims = verify_jwt(&keys, &token).expect("verify");
        assert_eq!(claims.sub, "u1");
        assert_eq!(claims.username, "alice");
        assert_eq!(claims.current_workspace, "ws_default");
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn tampered_token_rejected() {
        let keys = test_keys();
        let token = issue_jwt(&keys, "u1", "alice", "ws_default").expect("issue");
        // 翻转最后一个字节制造签名 mismatch
        let mut bad = token.clone();
        bad.pop();
        bad.push('X');
        let err = verify_jwt(&keys, &bad).unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(ref m) if m == "token_invalid"));
    }

    #[test]
    fn expired_token_returns_token_expired() {
        // ttl=0 即签出即过期；leeway=0 让 verify 立即识别为过期。
        let mut keys = test_keys();
        keys.ttl_minutes = 0;
        let claims = JwtClaims {
            sub: "u1".into(),
            username: "alice".into(),
            current_workspace: "ws".into(),
            iat: Utc::now().timestamp() - 600,
            exp: Utc::now().timestamp() - 60,
        };
        let token = encode(&Header::new(Algorithm::RS256), &claims, &keys.encoding)
            .expect("encode expired");
        let err = verify_jwt(&keys, &token).unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(ref m) if m == "token_expired"));
    }
}

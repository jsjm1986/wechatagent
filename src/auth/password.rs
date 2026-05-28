//! Argon2id 密码哈希包装。
//!
//! 直接走 [`argon2`] crate 的默认参数（OWASP 2024 推荐 m=19MiB / t=2 / p=1）。
//! PHC 字符串自带盐与参数，不需要单独存盐字段。

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password hashing failed: {0}")]
    Hash(String),
    #[error("password verification failed: {0}")]
    Verify(String),
}

/// 把明文密码哈希成 PHC 字符串（含算法 / 参数 / 盐 / 摘要）。
pub fn hash_password(plaintext: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(plaintext.as_bytes(), &salt)
        .map_err(|e| PasswordError::Hash(e.to_string()))?;
    Ok(hash.to_string())
}

/// 验证明文密码与 PHC 字符串。常数时间比较由 [`argon2`] 内部保证。
pub fn verify_password(plaintext: &str, phc: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(phc).map_err(|e| PasswordError::Verify(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_verifies_correct_password() {
        let phc = hash_password("hunter2-very-long").unwrap();
        assert!(verify_password("hunter2-very-long", &phc).unwrap());
    }

    #[test]
    fn rejects_wrong_password() {
        let phc = hash_password("hunter2-very-long").unwrap();
        assert!(!verify_password("wrong", &phc).unwrap());
    }

    #[test]
    fn salt_makes_each_hash_unique() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "Argon2 PHC 必须每次盐不同");
    }

    #[test]
    fn rejects_malformed_phc() {
        let res = verify_password("any", "not-a-phc-string");
        assert!(res.is_err());
    }
}

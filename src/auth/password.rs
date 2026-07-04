//! Argon2id 密码哈希包装。
//!
//! 直接走 [`argon2`] crate 的默认参数（OWASP 2024 推荐 m=19MiB / t=2 / p=1）。
//! PHC 字符串自带盐与参数，不需要单独存盐字段。

use std::sync::LazyLock;

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

/// 进程级预计算的假 PHC 哈希。用户不存在时对它跑一次 verify，抹平"用户存在 vs
/// 不存在"的 Argon2 耗时差——否则不存在的用户会因跳过 verify 而秒回，泄漏用户名
/// 是否存在（枚举时序侧信道）。必须是合法 PHC，否则 verify 会早退成 Err（快路径）
/// 反而重新制造时序差；`dummy_hash_is_valid_phc` 测试锁住这一不变量。
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    hash_password("constant-time-dummy-never-a-real-password").expect("dummy hash must build")
});

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

/// 用户名不存在时跑一次 verify（对进程级假哈希），支付与"用户存在"路径等价的
/// Argon2 耗时，抹平枚举时序侧信道。恒返回 false，调用方按"凭据无效"处理。
pub fn verify_against_dummy(plaintext: &str) -> bool {
    verify_password(plaintext, &DUMMY_HASH).unwrap_or(false)
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

    #[test]
    fn dummy_hash_is_valid_phc() {
        // 假哈希必须是合法 PHC，否则 verify_against_dummy 会走 PHC 解析失败的
        // 快路径（不跑 Argon2），重新制造用户存在/不存在的时序差。
        assert!(PasswordHash::new(&DUMMY_HASH).is_ok());
    }

    #[test]
    fn verify_against_dummy_always_false() {
        assert!(!verify_against_dummy("anything"));
        assert!(!verify_against_dummy(""));
    }
}

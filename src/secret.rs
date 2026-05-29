//! 秘密值脱敏工具：避免 `Debug` / `tracing` / API 响应回显完整 api_key。
//!
//! [`mask_secret`] 保留前 3 + 后 4 字符，中间统一替换为 `****`；长度 ≤ 8 的
//! 值整体掩码，避免短 key 反推。`""` 直接返回 `""`，便于"未配置"语义透传。
//!
//! 使用约定：
//! - 任何 `api_key` / `password` / `secret` / `token` 字段在 `tracing::*!` /
//!   `format!` / API 响应里出现时都过此函数；
//! - `Debug` 派生若结构体含上述字段，必须改成手写 `Debug` 并对该字段调
//!   `mask_secret`；
//! - 仅 admin SPA 的 mask 形态展示走 [`super::routes::llm_providers`] 内部
//!   `mask_api_key`（保留原 wire 兼容），底层共享逻辑应迁到本函数。

/// 把 secret 掩码为 `prefix(3) + "****" + suffix(4)`，长度 ≤ 8 整体掩码。
///
/// 空串原样返回，便于在日志里区分"未配置"与"已配置但被遮罩"。
pub fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if value.chars().count() <= 8 {
        return "****".to_string();
    }
    let head: String = value.chars().take(3).collect();
    let tail: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{head}****{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stays_empty() {
        assert_eq!(mask_secret(""), "");
    }

    #[test]
    fn short_value_fully_masked() {
        assert_eq!(mask_secret("short"), "****");
        assert_eq!(mask_secret("12345678"), "****");
    }

    #[test]
    fn long_value_keeps_head_and_tail() {
        let masked = mask_secret("sk-1234567890abcdef");
        assert!(masked.starts_with("sk-"));
        assert!(masked.ends_with("cdef"));
        assert!(masked.contains("****"));
        assert!(!masked.contains("1234567890ab"));
    }

    #[test]
    fn does_not_panic_on_multibyte() {
        let masked = mask_secret("私钥sk-12345678abcdef");
        assert!(masked.contains("****"));
    }
}

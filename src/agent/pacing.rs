//! 发送节奏拟人化纯函数。
//!
//! 账号级最小发送间隔闸用它把随机抖动映射成毫秒间隔。随机由调用点用
//! `fastrand::f64()` 注入（对称 `outbox::backoff_with_jitter_seeded` 的纯函数模式），
//! 故本函数确定性可测。

/// 把 `jitter01 ∈ [0,1]` 线性映射到 `[min_ms, max_ms]` 毫秒区间。
///
/// - `jitter01 = 0.0` → `min_ms`
/// - `jitter01 = 1.0` → `max_ms`
/// - `jitter01 = 0.5` → 中点
///
/// 越界的 `jitter01` 会被 clamp 到 `[0,1]`。`max_ms < min_ms` 时返回 `min_ms`
/// （调用方应保证 `min_ms <= max_ms`）。
pub(crate) fn account_send_interval_ms(jitter01: f64, min_ms: i64, max_ms: i64) -> i64 {
    let j = jitter01.clamp(0.0, 1.0);
    let span = (max_ms - min_ms).max(0);
    min_ms + (span as f64 * j).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_zero_to_min() {
        assert_eq!(account_send_interval_ms(0.0, 1000, 4000), 1000);
    }

    #[test]
    fn maps_one_to_max() {
        assert_eq!(account_send_interval_ms(1.0, 1000, 4000), 4000);
    }

    #[test]
    fn maps_half_to_midpoint() {
        assert_eq!(account_send_interval_ms(0.5, 1000, 4000), 2500);
    }

    #[test]
    fn clamps_out_of_range_jitter() {
        assert_eq!(account_send_interval_ms(-1.0, 1000, 4000), 1000);
        assert_eq!(account_send_interval_ms(2.0, 1000, 4000), 4000);
    }

    #[test]
    fn degenerate_range_returns_min() {
        // max < min：span clamp 到 0，恒返 min。
        assert_eq!(account_send_interval_ms(0.7, 4000, 1000), 4000);
    }
}

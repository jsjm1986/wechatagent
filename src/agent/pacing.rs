//! 发送节奏拟人化纯函数。
//!
//! 账号级最小发送间隔闸用它把随机抖动映射成毫秒间隔。随机由调用点用
//! `fastrand::f64()` 注入（对称 `outbox::backoff_with_jitter_seeded` 的纯函数模式），
//! 故本函数确定性可测。

/// S5-4：每字符打字时间加权（毫秒/字符）。
///
/// 拟人依据是**方向性**的：真人中文手机输入约 25-45 字/分钟量级，逐字复刻会让
/// 120 字长段等上数分钟、拖垮时效；取 35ms/字符做保守加权（120 字 ≈ +4.2s），
/// 只求"长段比短句慢几拍、不再秒发穿帮"，不求逐字节拟真。剩余基础间隔仍由
/// `[min_ms, max_ms]` 随机映射承担。
const PER_CHAR_TYPING_MS: i64 = 35;

/// S5-4：打字加权后的总间隔封顶余量（毫秒）——总间隔不超过 `max_ms + 6000`，
/// 超长段（粘贴文案/知识引用）不至于把队列拖出十几秒。
const TYPING_CAP_EXTRA_MS: i64 = 6000;

/// 把 `jitter01 ∈ [0,1]` 线性映射到 `[min_ms, max_ms]` 毫秒基础区间，再按本段
/// 内容长度加权打字时间：`total = base + content_chars × 35ms`，封顶
/// `max_ms + 6000ms`（见 [`PER_CHAR_TYPING_MS`] / [`TYPING_CAP_EXTRA_MS`]）。
///
/// - `jitter01 = 0.0` → `min_ms`；`1.0` → `max_ms`；`0.5` → 中点（越界 clamp）。
/// - `content_chars = 0` 与旧三参行为逐值等价（含 `max_ms < min_ms` 退化恒返
///   `min_ms`；调用方应保证 `min_ms <= max_ms`）。
/// - `min_ms = max_ms = 0`（tests/common 约定的"闸关"）恒返 0：打字加权不得让
///   关闭的闸复活。
/// - 对 `content_chars` 单调不减；封顶不会把结果压到 base 之下。
pub(crate) fn account_send_interval_ms(
    jitter01: f64,
    min_ms: i64,
    max_ms: i64,
    content_chars: usize,
) -> i64 {
    let j = jitter01.clamp(0.0, 1.0);
    let span = (max_ms - min_ms).max(0);
    let base = min_ms + (span as f64 * j).round() as i64;
    // 闸关（0/0 或非正区间）：恒返 base（= 旧行为），不加打字权重。
    if min_ms <= 0 && max_ms <= 0 {
        return base;
    }
    let typing = (content_chars as i64).saturating_mul(PER_CHAR_TYPING_MS);
    base.saturating_add(typing)
        .min(max_ms.saturating_add(TYPING_CAP_EXTRA_MS))
        .max(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 0 字符 = 与旧三参线性映射逐值等价（S5-4 回归锚）──

    #[test]
    fn maps_zero_to_min() {
        assert_eq!(account_send_interval_ms(0.0, 1000, 4000, 0), 1000);
    }

    #[test]
    fn maps_one_to_max() {
        assert_eq!(account_send_interval_ms(1.0, 1000, 4000, 0), 4000);
    }

    #[test]
    fn maps_half_to_midpoint() {
        assert_eq!(account_send_interval_ms(0.5, 1000, 4000, 0), 2500);
    }

    #[test]
    fn clamps_out_of_range_jitter() {
        assert_eq!(account_send_interval_ms(-1.0, 1000, 4000, 0), 1000);
        assert_eq!(account_send_interval_ms(2.0, 1000, 4000, 0), 4000);
    }

    #[test]
    fn degenerate_range_returns_min() {
        // max < min：span clamp 到 0，恒返 min（0 字符下与旧行为逐值等价）。
        assert_eq!(account_send_interval_ms(0.7, 4000, 1000, 0), 4000);
    }

    // ── S5-4：按内容长度加权（打字时间拟人）──

    #[test]
    fn typing_weight_adds_per_char_time() {
        // 短消息（"好的"×2 字）：+70ms，与现状几乎一致。
        assert_eq!(account_send_interval_ms(0.0, 1000, 4000, 2), 1070);
        // 120 字长段：base + 4200ms，像真人打完一段的节奏。
        assert_eq!(account_send_interval_ms(0.0, 1000, 4000, 120), 5200);
    }

    #[test]
    fn long_content_caps_at_max_plus_extra() {
        // 1000 字（base=4000 + 35000）封顶在 max_ms + 6000 = 10000。
        assert_eq!(account_send_interval_ms(1.0, 1000, 4000, 1000), 10_000);
        // 恰好触顶边界：typing 把 total 顶到 10000 整。
        assert_eq!(
            account_send_interval_ms(0.0, 1000, 4000, 9000 / 35 + 1),
            10_000
        );
    }

    #[test]
    fn interval_is_monotonic_in_content_length() {
        // 字符越多间隔不减（含跨越封顶点）。
        let mut prev = 0;
        for chars in [0usize, 1, 2, 10, 50, 120, 200, 400, 1000] {
            let interval = account_send_interval_ms(0.5, 1000, 4000, chars);
            assert!(
                interval >= prev,
                "chars={chars} 的间隔 {interval} 不应小于前值 {prev}"
            );
            prev = interval;
        }
    }

    #[test]
    fn disabled_gate_stays_zero_regardless_of_length() {
        // tests/common 约定：min=max=0 即"闸关"（恒返 0 → 永不 defer）。
        // 打字加权不得让关闭的闸复活，否则全部集成测试的背靠背发送会被注入延迟。
        for chars in [0usize, 2, 120, 5000] {
            assert_eq!(account_send_interval_ms(0.7, 0, 0, chars), 0);
        }
    }
}

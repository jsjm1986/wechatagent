//! 主动发送台账：转化判定纯函数（responded 窗口 / stage_advanced 推进）、
//! 聚合率计算。写入 / 回扫的 DB 逻辑在 gateway/tasks 调用侧，这里只放可单测的纯逻辑。

/// 任一入站时间戳落在 (sent_at, sent_at + window_hours] 内 → 已响应。
/// 早于/等于发送时刻的入站（历史消息）不算。
pub(crate) fn responded_within_window(sent_at_ms: i64, window_hours: i32, inbound_ms: &[i64]) -> bool {
    let window_end = sent_at_ms + (window_hours.max(0) as i64) * 3_600_000;
    inbound_ms
        .iter()
        .any(|&ms| ms > sent_at_ms && ms <= window_end)
}

/// 当前阶段在 ordered_stages 里严格靠后于发送时阶段 → 推进。
/// 任一阶段缺失或不在有序表 → 保守判 false（不算推进）。
pub(crate) fn stage_advanced(
    stage_at_send: Option<&str>,
    current_stage: Option<&str>,
    ordered_stages: &[String],
) -> bool {
    let (Some(from), Some(to)) = (stage_at_send, current_stage) else {
        return false;
    };
    let idx = |s: &str| ordered_stages.iter().position(|x| x == s);
    match (idx(from), idx(to)) {
        (Some(i), Some(j)) => j > i,
        _ => false,
    }
}

/// 响应率：total=0 返 0.0，否则 responded/total 保留 4 位小数。
pub(crate) fn response_rate(total: u64, responded: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let raw = responded as f64 / total as f64;
    (raw * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 3_600_000;

    #[test]
    fn responded_true_when_inbound_in_window() {
        let sent = 1_000_000_000_000;
        // 窗口 24h，入站在 sent 后 2h → 命中
        assert!(responded_within_window(sent, 24, &[sent + 2 * HOUR_MS]));
    }

    #[test]
    fn responded_false_when_inbound_after_window() {
        let sent = 1_000_000_000_000;
        // 入站在 sent 后 25h，窗口 24h → 不命中
        assert!(!responded_within_window(sent, 24, &[sent + 25 * HOUR_MS]));
    }

    #[test]
    fn responded_false_when_inbound_before_send() {
        let sent = 1_000_000_000_000;
        // 入站早于发送（历史消息）→ 不算响应
        assert!(!responded_within_window(sent, 24, &[sent - HOUR_MS]));
    }

    #[test]
    fn responded_false_when_no_inbound() {
        assert!(!responded_within_window(1_000_000_000_000, 24, &[]));
    }

    #[test]
    fn stage_advanced_true_when_moves_forward() {
        let order = vec!["new_contact".to_string(), "意向".to_string(), "待成交".to_string()];
        assert!(stage_advanced(Some("意向"), Some("待成交"), &order));
    }

    #[test]
    fn stage_advanced_false_when_same_or_backward() {
        let order = vec!["new_contact".to_string(), "意向".to_string(), "待成交".to_string()];
        assert!(!stage_advanced(Some("意向"), Some("意向"), &order)); // 持平
        assert!(!stage_advanced(Some("待成交"), Some("意向"), &order)); // 回退
    }

    #[test]
    fn stage_advanced_false_when_unknown_or_missing() {
        let order = vec!["new_contact".to_string(), "意向".to_string()];
        // 任一阶段不在有序表 → 保守判 false（不算推进）
        assert!(!stage_advanced(Some("意向"), Some("不存在"), &order));
        assert!(!stage_advanced(None, Some("意向"), &order));
    }

    #[test]
    fn response_rate_zero_total_is_zero() {
        assert_eq!(response_rate(0, 0), 0.0);
    }

    #[test]
    fn response_rate_basic() {
        assert_eq!(response_rate(4, 1), 0.25);
    }
}

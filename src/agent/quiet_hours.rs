//! 作息门控（quiet hours，#69）：运营方时区内的"静默时段"判定与"醒来时刻"计算。
//!
//! 产品语义：客户在运营方休息时段（默认 22:00–08:00）发来的消息**不立即回**，
//! 而是排一条 `deferred_inbound_reply` 跟进任务，等运营方醒来时段一次性基于累积
//! 消息回复——最像真人（睡觉时不回、醒来看完所有消息再答）。主动发送（planner
//! 催进 / 承诺跟进）若在静默时段到点，则**重排**到醒来时刻而非取消（避免丢承诺）。
//!
//! 时区：用**运营参数固定偏移** `quiet_hours_tz_offset_hours`（小时，如中国 +8），
//! 不依赖部署宿主时区（`chrono::Local` 取的是进程时区，容器多默认 UTC，会让
//! "22:00 静默"实际在 UTC 22:00 触发、偏 8 小时）。判定全部用 epoch 毫秒 + 偏移的
//! 纯整数运算——既消除宿主依赖，又规避本地时刻歧义（夏令时 / 不存在的本地时刻）。
//!
//! 全部判定逻辑做成**纯函数**（UTC 毫秒 + 偏移 / 小时数），完全本地可测；只有两个
//! 取真实时钟的薄包装（[`is_quiet_now`] / [`next_wake_at`]）用 `Utc::now()`。

use chrono::Utc;

/// 静默时段入站延迟回复的跟进任务 kind。区别于 planner 主动催进的 `follow_up`，
/// 用于在 precheck 中豁免 `context_changed`（它存在就是为了回 task 创建后累积的
/// 客户消息），并标记"这是延迟的被动应答、不是主动打扰"。
pub(crate) const DEFERRED_INBOUND_REPLY_KIND: &str = "deferred_inbound_reply";

/// 判定 `now_hour`（0..=23）是否落在静默时段 `[start, end)` 内。
///
/// 边界语义：start 含、end 不含（hour==start 静默，hour==end 已醒来）。
/// - `start < end`（如 1..6）：当日区间，`start <= hour < end`。
/// - `start > end`（如 22..8，跨午夜）：`hour >= start || hour < end`。
/// - `start == end`：退化为**永不静默**（防误配把 agent 全天禁言）。
pub(crate) fn in_quiet_hours(now_hour: u32, start: u32, end: u32) -> bool {
    let h = now_hour % 24;
    let s = start % 24;
    let e = end % 24;
    if s == e {
        return false;
    }
    if s < e {
        h >= s && h < e
    } else {
        h >= s || h < e
    }
}

/// 给定 UTC 毫秒与运营方时区偏移（小时），返回运营方本地"当前小时"(0..=23)。
///
/// 用 `div_euclid` / `rem_euclid` 保证负偏移 / 负毫秒（理论上不会出现，但防御）
/// 也落在 0..=23，不会出现 Rust `%` 对负数取负的坑。
pub(crate) fn hour_in_offset(now_utc_ms: i64, tz_offset_hours: i32) -> u32 {
    let shifted = now_utc_ms + (tz_offset_hours as i64) * 3_600_000;
    shifted.div_euclid(3_600_000).rem_euclid(24) as u32
}

/// 给定 UTC 毫秒、醒来小时 `end`、时区偏移，返回下一次"运营方本地 `end`:00"对应的
/// UTC 毫秒。严格在 `now` 之后（恰好命中 `end`:00 也取次日，保证 wake 落在未来，
/// 与旧 `next_wake_instant` 的"严格大于"语义一致）。
pub(crate) fn next_wake_utc_ms(now_utc_ms: i64, end: u32, tz_offset_hours: i32) -> i64 {
    let off = (tz_offset_hours as i64) * 3_600_000;
    let local_ms = now_utc_ms + off;
    let day = local_ms.div_euclid(86_400_000); // 本地"第几天"
    let end_ms_today = day * 86_400_000 + (end.min(23) as i64) * 3_600_000;
    let local_target = if end_ms_today > local_ms {
        end_ms_today
    } else {
        end_ms_today + 86_400_000
    };
    local_target - off // 回到 UTC
}

/// 薄包装：当前真实时刻（按运营方偏移换算）是否在静默时段。生产判定入口。
pub(crate) fn is_quiet_now(start: u32, end: u32, tz_offset_hours: i32) -> bool {
    in_quiet_hours(
        hour_in_offset(Utc::now().timestamp_millis(), tz_offset_hours),
        start,
        end,
    )
}

/// 薄包装：从现在算下一次醒来时刻（UTC），转成 BSON `DateTime` 供 task `run_at` 用。
pub(crate) fn next_wake_at(end: u32, tz_offset_hours: i32) -> mongodb::bson::DateTime {
    mongodb::bson::DateTime::from_millis(next_wake_utc_ms(
        Utc::now().timestamp_millis(),
        end,
        tz_offset_hours,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把 "YYYY-MM-DDThh:mm:ssZ" 解析成 UTC 毫秒，便于断言时区换算。
    fn utc_ms(rfc3339: &str) -> i64 {
        mongodb::bson::DateTime::parse_rfc3339_str(rfc3339)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn cross_midnight_window_22_to_8() {
        // 22..8 跨午夜：22/23/0/7 静默，8/9/12/21 不静默。
        for h in [22, 23, 0, 3, 7] {
            assert!(in_quiet_hours(h, 22, 8), "hour {h} 应静默");
        }
        for h in [8, 9, 12, 21] {
            assert!(!in_quiet_hours(h, 22, 8), "hour {h} 不应静默");
        }
    }

    #[test]
    fn same_day_window_1_to_6() {
        for h in [1, 3, 5] {
            assert!(in_quiet_hours(h, 1, 6), "hour {h} 应静默");
        }
        for h in [0, 6, 7, 23] {
            assert!(!in_quiet_hours(h, 1, 6), "hour {h} 不应静默");
        }
    }

    #[test]
    fn start_inclusive_end_exclusive() {
        // hour==start 静默；hour==end 已醒来。
        assert!(in_quiet_hours(22, 22, 8), "start 含");
        assert!(!in_quiet_hours(8, 22, 8), "end 不含");
    }

    #[test]
    fn degenerate_equal_start_end_never_quiet() {
        for h in 0..24 {
            assert!(!in_quiet_hours(h, 9, 9), "start==end 应永不静默, hour {h}");
        }
    }

    #[test]
    fn hour_in_offset_china_plus8() {
        // UTC 14:00 + 8 = 北京 22:00（静默起点）。
        assert_eq!(hour_in_offset(utc_ms("2026-06-09T14:00:00Z"), 8), 22);
        // UTC 00:00 + 8 = 北京 08:00（醒来）。
        assert_eq!(hour_in_offset(utc_ms("2026-06-09T00:00:00Z"), 8), 8);
        // UTC 18:30 + 8 = 北京次日 02:30 → 小时 2。
        assert_eq!(hour_in_offset(utc_ms("2026-06-09T18:30:00Z"), 8), 2);
    }

    #[test]
    fn hour_in_offset_negative_offset_wraps_correctly() {
        // 西五区 -5：UTC 02:00 - 5 = 前一日 21:00 → 小时 21（rem_euclid 不出负）。
        assert_eq!(hour_in_offset(utc_ms("2026-06-09T02:00:00Z"), -5), 21);
        // UTC 00:00 - 12 = 前一日 12:00 → 小时 12。
        assert_eq!(hour_in_offset(utc_ms("2026-06-09T00:00:00Z"), -12), 12);
        // 任意偏移结果都落在 0..=23。
        for off in [-12, -5, 0, 8, 14] {
            let h = hour_in_offset(utc_ms("2026-06-09T03:17:00Z"), off);
            assert!(h < 24, "offset {off} 算出非法小时 {h}");
        }
    }

    #[test]
    fn wake_same_day_when_end_still_ahead() {
        // 北京 02:30（= UTC 前一日 18:30），end=8 → 北京当天 08:00（= UTC 00:00）。
        let now = utc_ms("2026-06-08T18:30:00Z");
        let wake = next_wake_utc_ms(now, 8, 8);
        assert_eq!(wake, utc_ms("2026-06-09T00:00:00Z"));
    }

    #[test]
    fn wake_next_day_when_end_already_passed() {
        // 北京 23:00（= UTC 15:00），end=8 → 北京次日 08:00（= 次日 UTC 00:00）。
        let now = utc_ms("2026-06-09T15:00:00Z");
        let wake = next_wake_utc_ms(now, 8, 8);
        assert_eq!(wake, utc_ms("2026-06-10T00:00:00Z"));
    }

    #[test]
    fn wake_strictly_after_now_at_exact_hour() {
        // 恰好北京 08:00（= UTC 00:00）命中 end → 不取当天，取次日，保证 wake 严格在未来。
        let now = utc_ms("2026-06-09T00:00:00Z");
        let wake = next_wake_utc_ms(now, 8, 8);
        assert_eq!(wake, utc_ms("2026-06-10T00:00:00Z"));
        assert!(wake > now, "wake 必须严格在 now 之后");
    }

    #[test]
    fn wake_respects_negative_offset() {
        // 西五区 -5：UTC 12:00 = 当地 07:00，end=8 → 当地当天 08:00 = UTC 13:00。
        let now = utc_ms("2026-06-09T12:00:00Z");
        let wake = next_wake_utc_ms(now, 8, -5);
        assert_eq!(wake, utc_ms("2026-06-09T13:00:00Z"));
    }

    #[test]
    fn deferred_kind_constant_stable() {
        assert_eq!(DEFERRED_INBOUND_REPLY_KIND, "deferred_inbound_reply");
    }
}

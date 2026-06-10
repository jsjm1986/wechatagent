//! 作息门控（quiet hours，#69）：运营方时区内的"静默时段"判定与"醒来时刻"计算。
//!
//! 产品语义：客户在运营方休息时段（默认 22:00–08:00）发来的消息**不立即回**，
//! 而是排一条 `deferred_inbound_reply` 跟进任务，等运营方醒来时段一次性基于累积
//! 消息回复——最像真人（睡觉时不回、醒来看完所有消息再答）。主动发送（planner
//! 催进 / 承诺跟进）若在静默时段到点，则**重排**到醒来时刻而非取消（避免丢承诺）。
//!
//! 时区：用进程本地 [`chrono::Local`]（运营方部署时区），不引 `chrono-tz`、不在
//! `Contact` 上加 timezone 字段——与 `knowledge_digest::duration_until_next_run`
//! 同款约定。
//!
//! 全部判定逻辑做成**纯函数**（小时数 / 可注入 `now`），最大化本地可测；只有两个
//! 取真实时钟的薄包装（[`is_quiet_now`] / [`next_wake_at`]）不单测。

use chrono::{DateTime, Local, NaiveTime, TimeZone, Timelike};

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

/// 计算从 `now` 起、下一次"小时 == `hour`、分秒 == 0"的本地时刻。
///
/// 今天该小时还在未来则取今天，否则取次日。仿
/// `knowledge_digest::duration_until_next_run` 的范式，但返回时刻而非 Duration，
/// 且把 `now` 抽成参数以便注入测试。
pub(crate) fn next_occurrence_of_hour(now: DateTime<Local>, hour: u32) -> DateTime<Local> {
    let h = hour.min(23);
    let target_today = Local
        .from_local_datetime(
            &now.date_naive()
                .and_time(NaiveTime::from_hms_opt(h, 0, 0).unwrap_or_default()),
        )
        .single();
    match target_today {
        Some(t) if t > now => t,
        _ => {
            let next_day = now.date_naive().succ_opt().unwrap_or(now.date_naive());
            Local
                .from_local_datetime(
                    &next_day.and_time(NaiveTime::from_hms_opt(h, 0, 0).unwrap_or_default()),
                )
                .single()
                .unwrap_or(now + chrono::Duration::hours(24))
        }
    }
}

/// 静默时段的"醒来时刻" = 下一次 `end` 整点（醒来即静默区间的右开端点）。
pub(crate) fn next_wake_instant(now: DateTime<Local>, end: u32) -> DateTime<Local> {
    next_occurrence_of_hour(now, end)
}

/// 薄包装：当前真实本地时刻是否在静默时段。生产判定入口。
pub(crate) fn is_quiet_now(start: u32, end: u32) -> bool {
    in_quiet_hours(Local::now().hour(), start, end)
}

/// 薄包装：从现在算下一次醒来时刻，转成 BSON `DateTime` 供 task `run_at` 用。
pub(crate) fn next_wake_at(end: u32) -> mongodb::bson::DateTime {
    let wake = next_wake_instant(Local::now(), end);
    mongodb::bson::DateTime::from_millis(wake.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn local_at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap()
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
    fn wake_same_day_when_end_still_ahead() {
        // 凌晨 2:30，end=8 → 当天 08:00。
        let now = local_at(2026, 6, 9, 2, 30);
        let wake = next_wake_instant(now, 8);
        assert_eq!(wake, local_at(2026, 6, 9, 8, 0));
    }

    #[test]
    fn wake_next_day_when_end_already_passed() {
        // 夜里 23:00，end=8 → 次日 08:00。
        let now = local_at(2026, 6, 9, 23, 0);
        let wake = next_wake_instant(now, 8);
        assert_eq!(wake, local_at(2026, 6, 10, 8, 0));
    }

    #[test]
    fn wake_strictly_after_now_at_exact_hour() {
        // 恰好 08:00 命中 end → 不取当天（已到点），取次日，保证 wake 严格在未来。
        let now = local_at(2026, 6, 9, 8, 0);
        let wake = next_wake_instant(now, 8);
        assert_eq!(wake, local_at(2026, 6, 10, 8, 0));
    }

    #[test]
    fn deferred_kind_constant_stable() {
        assert_eq!(DEFERRED_INBOUND_REPLY_KIND, "deferred_inbound_reply");
    }
}

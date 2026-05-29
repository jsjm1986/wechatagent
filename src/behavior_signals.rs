//! 自学习采集管道（第一阶段）：T1 行为信号采集底座。
//!
//! 本模块只负责把"系统客观观察到的量"以 append-only、幂等的方式落进
//! `behavior_signals` collection，**不解释、不评分、不喂任何学习公式**。
//! 解释层（情绪 / 极性）继续留在已有的 `reaction_analysis`，两层物理隔离
//! （Iron Law ③：观察与解释分层）。
//!
//! 采集的信号类型（按认知层 T1）：
//! - `reply_latency`：上一条 outbound → 本条 inbound 的间隔毫秒；
//! - `reply_length`：本条 inbound 的字符数（`chars().count()`，多字节安全）；
//! - `reactivation`：久未联系（间隔超阈值）后重新入站；
//! - `silence`：一条 outbound 至今无回（由 silence worker 探测，**恒
//!   `censored=true`**，Iron Law ②：沉默 = 删失，绝不当负例）。
//!
//! 每条信号带元数据 `source="system_observed"` / `confidence=1.0` /
//! `observed_at` / `dedupe_key`（Iron Law ④），写入走 `dedupe_key` +
//! partial unique 索引保证幂等（Iron Law ⑤）。

use mongodb::bson::DateTime;

use crate::models::BehaviorSignal;
use crate::routes::AppState;

/// 系统观察信号的来源标记；所有 T1 信号恒为此值（与 admin 手动 / LLM 解释区分）。
pub const SOURCE_SYSTEM_OBSERVED: &str = "system_observed";

/// reactivation 判定阈值（毫秒）：上一条 inbound 距本条 inbound 超过此间隔，
/// 视为"久未联系后重新激活"。默认 7 天，与 cold_contact 阈值同量级但独立。
pub const REACTIVATION_THRESHOLD_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// reply_latency 的 dedupe key：同一条 inbound 只产一次延迟信号。
pub fn reply_latency_dedupe_key(wxid: &str, inbound_message_id: &str) -> String {
    format!("reply_latency:{wxid}:{inbound_message_id}")
}

/// reply_length 的 dedupe key：同一条 inbound 只产一次长度信号。
pub fn reply_length_dedupe_key(wxid: &str, inbound_message_id: &str) -> String {
    format!("reply_length:{wxid}:{inbound_message_id}")
}

/// reactivation 的 dedupe key：同一条触发 reactivation 的 inbound 只产一次。
pub fn reactivation_dedupe_key(wxid: &str, inbound_message_id: &str) -> String {
    format!("reactivation:{wxid}:{inbound_message_id}")
}

/// silence 的 dedupe key：同一条 outbound（按其毫秒时间戳）只产一次沉默信号，
/// 与 worker tick 节奏解耦——重复 tick 不会对同一条 outbound 重复落删失。
pub fn silence_dedupe_key(wxid: &str, last_outbound_at_ms: i64) -> String {
    format!("silence:{wxid}:{last_outbound_at_ms}")
}

/// 构造一条 `reply_latency` 观察。
///
/// `last_outbound_ms` 缺失（contact 从未出站过）时 `latency_ms=None`——
/// 没有可比基准，不臆造 0。`latency_ms` 允许为 0（同毫秒内的极快回复），
/// 但不允许为负：inbound 早于记录的 outbound 属时钟/乱序，落 None 而非负值。
pub fn build_reply_latency(
    workspace_id: &str,
    wxid: &str,
    inbound_message_id: &str,
    inbound_at: DateTime,
    last_outbound_ms: Option<i64>,
) -> BehaviorSignal {
    let latency_ms = last_outbound_ms.and_then(|out_ms| {
        let delta = inbound_at.timestamp_millis() - out_ms;
        if delta >= 0 {
            Some(delta)
        } else {
            None
        }
    });
    BehaviorSignal {
        id: None,
        workspace_id: workspace_id.to_string(),
        contact_wxid: wxid.to_string(),
        signal_type: "reply_latency".to_string(),
        observed_at: inbound_at,
        source: SOURCE_SYSTEM_OBSERVED.to_string(),
        confidence: 1.0,
        censored: false,
        dedupe_key: reply_latency_dedupe_key(wxid, inbound_message_id),
        latency_ms,
        char_len: None,
        silence_since: None,
        silence_ms: None,
        unanswered: None,
        reactivated_at: None,
    }
}

/// 构造一条 `reply_length` 观察。字符数用 `chars().count()`，对中文 / emoji
/// 等多字节安全（不是字节数）。
pub fn build_reply_length(
    workspace_id: &str,
    wxid: &str,
    inbound_message_id: &str,
    inbound_at: DateTime,
    content: &str,
) -> BehaviorSignal {
    BehaviorSignal {
        id: None,
        workspace_id: workspace_id.to_string(),
        contact_wxid: wxid.to_string(),
        signal_type: "reply_length".to_string(),
        observed_at: inbound_at,
        source: SOURCE_SYSTEM_OBSERVED.to_string(),
        confidence: 1.0,
        censored: false,
        dedupe_key: reply_length_dedupe_key(wxid, inbound_message_id),
        latency_ms: None,
        char_len: Some(content.chars().count() as i64),
        silence_since: None,
        silence_ms: None,
        unanswered: None,
        reactivated_at: None,
    }
}

/// 判定本条 inbound 是否构成 reactivation：上一条 inbound 距本条超过阈值。
/// `last_inbound_ms` 缺失（首条 inbound）不算 reactivation——属"新建首次触达"。
pub fn is_reactivation(last_inbound_ms: Option<i64>, inbound_at: DateTime, threshold_ms: i64) -> bool {
    match last_inbound_ms {
        Some(prev_ms) => inbound_at.timestamp_millis() - prev_ms >= threshold_ms,
        None => false,
    }
}

/// 构造一条 `reactivation` 观察（仅当 [`is_reactivation`] 为真时由调用方构造）。
pub fn build_reactivation(
    workspace_id: &str,
    wxid: &str,
    inbound_message_id: &str,
    inbound_at: DateTime,
) -> BehaviorSignal {
    BehaviorSignal {
        id: None,
        workspace_id: workspace_id.to_string(),
        contact_wxid: wxid.to_string(),
        signal_type: "reactivation".to_string(),
        observed_at: inbound_at,
        source: SOURCE_SYSTEM_OBSERVED.to_string(),
        confidence: 1.0,
        censored: false,
        dedupe_key: reactivation_dedupe_key(wxid, inbound_message_id),
        latency_ms: None,
        char_len: None,
        silence_since: None,
        silence_ms: None,
        unanswered: None,
        reactivated_at: Some(inbound_at),
    }
}

/// 构造一条 `silence` 删失观察。**恒 `censored=true`**——沉默是删失，不是负例。
/// `silence_since` = 起算的那条 outbound 时间；`silence_ms` = 至 `observed_at`
/// 的已沉默时长。
pub fn build_silence(
    workspace_id: &str,
    wxid: &str,
    last_outbound_at: DateTime,
    observed_at: DateTime,
) -> BehaviorSignal {
    let silence_ms = (observed_at.timestamp_millis() - last_outbound_at.timestamp_millis()).max(0);
    BehaviorSignal {
        id: None,
        workspace_id: workspace_id.to_string(),
        contact_wxid: wxid.to_string(),
        signal_type: "silence".to_string(),
        observed_at,
        source: SOURCE_SYSTEM_OBSERVED.to_string(),
        confidence: 1.0,
        censored: true,
        dedupe_key: silence_dedupe_key(wxid, last_outbound_at.timestamp_millis()),
        latency_ms: None,
        char_len: None,
        silence_since: Some(last_outbound_at),
        silence_ms: Some(silence_ms),
        unanswered: Some(true),
        reactivated_at: None,
    }
}

/// 幂等落库一条行为信号。返回 `Ok(true)` 表示真正写入，`Ok(false)` 表示
/// dedupe_key 撞 partial unique 索引（已存在，幂等跳过）。其它错误透传。
///
/// 采集是 best-effort 旁路：调用方应 `if let Err(e) = persist_signal(...)`
/// 仅 `tracing::warn!`，**绝不影响主应答链路**。
pub async fn persist_signal(state: &AppState, signal: BehaviorSignal) -> anyhow::Result<bool> {
    match state.db.behavior_signals().insert_one(&signal, None).await {
        Ok(_) => Ok(true),
        Err(err) if is_duplicate_key_error(&err) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

fn is_duplicate_key_error(err: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match &*err.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            write_error.code == 11000 || write_error.code == 11001
        }
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .as_ref()
            .map(|errs| errs.iter().any(|e| e.code == 11000 || e.code == 11001))
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_none_when_no_prior_outbound() {
        // 从未出站过 → 没有基准，latency_ms 必须 None（不臆造 0）。
        let sig = build_reply_latency(
            "ws",
            "wxid_a",
            "msg1",
            DateTime::from_millis(10_000),
            None,
        );
        assert_eq!(sig.latency_ms, None);
        assert_eq!(sig.source, SOURCE_SYSTEM_OBSERVED);
        assert_eq!(sig.confidence, 1.0);
        assert!(!sig.censored);
    }

    #[test]
    fn latency_computed_from_last_outbound() {
        let sig = build_reply_latency(
            "ws",
            "wxid_a",
            "msg2",
            DateTime::from_millis(10_000),
            Some(7_000),
        );
        assert_eq!(sig.latency_ms, Some(3_000));
    }

    #[test]
    fn latency_zero_is_allowed() {
        // 同毫秒回复 → 0 是合法的极快延迟，不该被丢成 None。
        let sig = build_reply_latency("ws", "w", "m", DateTime::from_millis(5_000), Some(5_000));
        assert_eq!(sig.latency_ms, Some(0));
    }

    #[test]
    fn latency_negative_falls_to_none() {
        // inbound 早于记录 outbound → 时钟/乱序，落 None 而非负值。
        let sig = build_reply_latency("ws", "w", "m", DateTime::from_millis(4_000), Some(5_000));
        assert_eq!(sig.latency_ms, None);
    }

    #[test]
    fn reply_length_counts_chars_not_bytes() {
        // 中文 + emoji：chars().count() 应数"字符"，不是字节。
        let sig = build_reply_length("ws", "w", "m", DateTime::from_millis(1), "你好👋ok");
        // 你(1) 好(1) 👋(1) o(1) k(1) = 5 chars；字节数会是 3+3+4+1+1=12。
        assert_eq!(sig.char_len, Some(5));
    }

    #[test]
    fn reply_length_empty_is_zero() {
        let sig = build_reply_length("ws", "w", "m", DateTime::from_millis(1), "");
        assert_eq!(sig.char_len, Some(0));
    }

    #[test]
    fn reactivation_requires_gap_over_threshold() {
        let now = DateTime::from_millis(REACTIVATION_THRESHOLD_MS + 1);
        // 上一条 inbound 在 0，间隔 > 阈值 → reactivation。
        assert!(is_reactivation(Some(0), now, REACTIVATION_THRESHOLD_MS));
        // 间隔恰等于阈值 → 边界算 reactivation（>=）。
        let exact = DateTime::from_millis(REACTIVATION_THRESHOLD_MS);
        assert!(is_reactivation(Some(0), exact, REACTIVATION_THRESHOLD_MS));
        // 间隔不足 → 不算。
        let close = DateTime::from_millis(REACTIVATION_THRESHOLD_MS - 1);
        assert!(!is_reactivation(Some(0), close, REACTIVATION_THRESHOLD_MS));
    }

    #[test]
    fn reactivation_first_inbound_is_not_reactivation() {
        // 首条 inbound（无 last_inbound）属"新建首次触达"，不是 reactivation。
        assert!(!is_reactivation(None, DateTime::from_millis(99_999_999), REACTIVATION_THRESHOLD_MS));
    }

    #[test]
    fn silence_is_always_censored() {
        let sig = build_silence(
            "ws",
            "w",
            DateTime::from_millis(1_000),
            DateTime::from_millis(90_000),
        );
        assert!(sig.censored, "沉默信号必须 censored=true（删失，不是负例）");
        assert_eq!(sig.unanswered, Some(true));
        assert_eq!(sig.silence_since, Some(DateTime::from_millis(1_000)));
        assert_eq!(sig.silence_ms, Some(89_000));
    }

    #[test]
    fn silence_ms_clamped_nonnegative() {
        // observed_at 早于 outbound（乱序）→ silence_ms 夹到 0，不出负值。
        let sig = build_silence(
            "ws",
            "w",
            DateTime::from_millis(5_000),
            DateTime::from_millis(1_000),
        );
        assert_eq!(sig.silence_ms, Some(0));
    }

    #[test]
    fn dedupe_keys_are_typed_and_stable() {
        assert_eq!(reply_latency_dedupe_key("w", "m1"), "reply_latency:w:m1");
        assert_eq!(reply_length_dedupe_key("w", "m1"), "reply_length:w:m1");
        assert_eq!(reactivation_dedupe_key("w", "m1"), "reactivation:w:m1");
        assert_eq!(silence_dedupe_key("w", 1_000), "silence:w:1000");
        // 不同 type 同 wxid 同 msg → key 不撞，三类信号能共存。
        assert_ne!(
            reply_latency_dedupe_key("w", "m1"),
            reply_length_dedupe_key("w", "m1")
        );
    }

    #[test]
    fn silence_dedupe_key_is_per_outbound_not_per_tick() {
        // 同一条 outbound（同毫秒）多 tick 探测 → 同 key（幂等）。
        let k1 = silence_dedupe_key("w", 12_345);
        let k2 = silence_dedupe_key("w", 12_345);
        assert_eq!(k1, k2);
        // 不同 outbound → 不同 key。
        assert_ne!(silence_dedupe_key("w", 12_345), silence_dedupe_key("w", 12_346));
    }
}

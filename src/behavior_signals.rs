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
        ingest_time: Some(DateTime::now()),
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
        ingest_time: Some(DateTime::now()),
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
        ingest_time: Some(DateTime::now()),
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
        ingest_time: Some(DateTime::now()),
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

/// P3 采集健康度：把一次 [`persist_signal`] 的三态结果计数进 `behavior_signal_metrics`。
///
/// best-effort 旁路：自身失败只 warn，**绝不影响主应答链路**，也绝不让健康度计数
/// 反过来拖垮采集。flag `BEHAVIOR_SIGNAL_METRICS_ENABLED` 关闭时（默认）直接返回。
///
/// 幂等：`_id="{workspace_id}:{date}"` + `$inc`，重复 tick 只累加不重置；
/// 成功写入额外 `$set last_success_at`（新鲜度指标）。
pub async fn record_signal_metric(
    state: &AppState,
    workspace_id: &str,
    result: &anyhow::Result<bool>,
) {
    if !state.config.behavior_signal_metrics_enabled {
        return;
    }
    let date = metric_date_string();
    let id = format!("{workspace_id}:{date}");
    let now = mongodb::bson::DateTime::now();
    let mut inc = mongodb::bson::Document::new();
    let mut set = mongodb::bson::doc! { "updated_at": now };
    inc.insert(metric_inc_field(result), 1_i64);
    if matches!(result, Ok(true)) {
        // 仅真正写入才刷新新鲜度时戳；去重/失败不算"成功采集"。
        set.insert("last_success_at", now);
    }
    let update = mongodb::bson::doc! {
        "$inc": inc,
        "$set": set,
        "$setOnInsert": {
            "workspace_id": workspace_id,
            "date": &date,
        },
    };
    let opts = mongodb::options::UpdateOptions::builder()
        .upsert(true)
        .build();
    if let Err(err) = state
        .db
        .behavior_signal_metrics()
        .update_one(mongodb::bson::doc! { "_id": &id }, update, opts)
        .await
    {
        tracing::warn!(?err, workspace_id, "record_signal_metric failed (non-fatal)");
    }
}

/// 当日 `YYYY-MM-DD`（UTC 截断到日）——`behavior_signal_metrics._id` 的日期分量。
/// 与 `tasks.rs::today_date_string` 同口径（epoch 天数粗截断，足够幂等聚合用）。
fn metric_date_string() -> String {
    let now_ms = mongodb::bson::DateTime::now().timestamp_millis();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let days = now_ms / day_ms;
    let secs = days * 24 * 60 * 60;
    let datetime =
        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).unwrap_or_else(chrono::Utc::now);
    datetime.format("%Y-%m-%d").to_string()
}

/// 三态 → `$inc` 字段名（纯函数，可单测）。`Ok(true)`=真正写入→persisted；
/// `Ok(false)`=撞 dedupe 索引幂等跳过→dedupe_skipped；`Err`=真失败→errors。
fn metric_inc_field(result: &anyhow::Result<bool>) -> &'static str {
    match result {
        Ok(true) => "persisted",
        Ok(false) => "dedupe_skipped",
        Err(_) => "errors",
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

    /// 回归门：`BehaviorSignal` 必须以 snake_case 落库。
    ///
    /// 幂等索引 `{workspace_id, dedupe_key}` 的 `partialFilterExpression` 按
    /// snake-case `dedupe_key` 匹配（`db/indexes.rs`）。若 struct 误加
    /// `#[serde(rename_all = "camelCase")]`，字段会序列化成 `dedupeKey`，partial
    /// filter 命中 0 文档 → unique 约束失效 → 重复信号全部落库（曾被
    /// `behavior_signal_smoke` 集成测试逮到）。本测试在 lib 层（无需 Docker）锁死
    /// 字段名，任何重新引入 camelCase 的改动都会在此处编译期之外即时失败。
    #[test]
    fn serializes_to_snake_case_for_index_match() {
        let sig = build_reply_latency(
            "ws",
            "wxid_a",
            "msg1",
            DateTime::from_millis(10_000),
            Some(3_000),
        );
        let doc = mongodb::bson::to_document(&sig).expect("serialize BehaviorSignal");
        // 索引 / 查询依赖的字段名必须是 snake_case。
        assert!(doc.contains_key("dedupe_key"), "幂等键必须落库为 dedupe_key");
        assert!(doc.contains_key("workspace_id"), "必须落库为 workspace_id");
        assert!(doc.contains_key("contact_wxid"), "必须落库为 contact_wxid");
        assert!(doc.contains_key("signal_type"), "必须落库为 signal_type");
        assert!(doc.contains_key("observed_at"), "必须落库为 observed_at");
        assert!(doc.contains_key("latency_ms"), "必须落库为 latency_ms");
        // 绝不能出现任何 camelCase 变体。
        assert!(!doc.contains_key("dedupeKey"), "camelCase dedupeKey 会让 partial 索引失效");
        assert!(!doc.contains_key("contactWxid"));
        assert!(!doc.contains_key("signalType"));
    }

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

    // ---- P2 双时间戳：observed_at(event_time) + ingest_time(write-time) ----

    #[test]
    fn builders_fill_ingest_time() {
        // 四个 builder 都必须填 ingest_time（落库时刻），否则数据工程无法识别采集延迟/回填。
        let latency = build_reply_latency("ws", "w", "m", DateTime::from_millis(10_000), Some(3_000));
        let length = build_reply_length("ws", "w", "m", DateTime::from_millis(10_000), "hi");
        let react = build_reactivation("ws", "w", "m", DateTime::from_millis(10_000));
        let silence = build_silence("ws", "w", DateTime::from_millis(1_000), DateTime::from_millis(90_000));
        for sig in [latency, length, react, silence] {
            assert!(sig.ingest_time.is_some(), "{} 必须填 ingest_time", sig.signal_type);
        }
    }

    #[test]
    fn event_time_and_ingest_time_coexist() {
        // observed_at = event_time（事实发生时刻，由调用方传入），ingest_time = 落库时刻；
        // 两者独立存在，event_time 不被 builder 篡改成 now()。
        let event_ms = 10_000;
        let sig = build_reply_latency("ws", "w", "m", DateTime::from_millis(event_ms), Some(3_000));
        assert_eq!(sig.observed_at.timestamp_millis(), event_ms, "event_time 必须保留调用方传入值");
        assert!(sig.ingest_time.is_some());
    }

    #[test]
    fn ingest_time_defaults_none_for_legacy_docs() {
        // R11：旧文档无 ingest_time 字段，反序列化必须回落 None，不报错。
        let doc = mongodb::bson::doc! {
            "workspace_id": "ws",
            "contact_wxid": "w",
            "signal_type": "reply_length",
            "observed_at": DateTime::from_millis(1),
            "source": SOURCE_SYSTEM_OBSERVED,
            "confidence": 1.0,
            "censored": false,
            "dedupe_key": "reply_length:w:m",
            "char_len": 2_i64,
        };
        let sig: BehaviorSignal =
            mongodb::bson::from_document(doc).expect("legacy doc without ingest_time must deserialize");
        assert_eq!(sig.ingest_time, None);
    }

    // ---- P3 采集健康度：三态 → $inc 字段 ----

    #[test]
    fn metric_inc_field_maps_three_states() {
        assert_eq!(metric_inc_field(&Ok(true)), "persisted");
        assert_eq!(metric_inc_field(&Ok(false)), "dedupe_skipped");
        assert_eq!(
            metric_inc_field(&Err(anyhow::anyhow!("boom"))),
            "errors"
        );
    }

    #[test]
    fn metric_date_string_is_iso_day() {
        let d = metric_date_string();
        // YYYY-MM-DD：10 字符，两个连字符。
        assert_eq!(d.len(), 10, "got {d}");
        assert_eq!(d.matches('-').count(), 2, "got {d}");
    }
}

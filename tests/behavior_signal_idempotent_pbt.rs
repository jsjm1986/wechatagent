//! 自学习采集管道（第一阶段）：`behavior_signals` 幂等性 PBT。
//!
//! 生产侧幂等由 `db/indexes.rs` 的 partial unique 索引
//! `{ workspace_id:1, dedupe_key:1 }` 保证：同一 `dedupe_key` 的并发 / 重放写入
//! 只成功一次（其余撞 11000，由 `behavior_signals::persist_signal` 吞成
//! `Ok(false)`）。本 PBT 在内存里建模"索引 = 按 dedupe_key 去重的集合"，对
//! 任意事件序列重放，断言去重后每个 dedupe_key 恰好落一次——即 Iron Law ⑤
//! "采集必须幂等"的不变量，不依赖 testcontainers / mongodb。
//!
//! 4 个 property（单文件累计 1024 cases ≥ 64 plan 门）：
//! 1. **replay_collapses_to_unique_keys** — 任意（含大量重复）信号序列重放后，
//!    去重写入数 == 不同 dedupe_key 数。
//! 2. **silence_dedupe_per_outbound** — 同一条 outbound（同毫秒）多次探测只产
//!    一个 silence key；不同 outbound 产不同 key。
//! 3. **inbound_signal_types_never_collide** — 同一条 inbound 的 latency /
//!    length / reactivation 三类信号 dedupe_key 两两不同，可共存。
//! 4. **silence_always_censored_others_never** — 沉默信号恒 censored=true，其余
//!    T1 信号恒 censored=false（Iron Law ②：沉默 = 删失，不是负例）。

use std::collections::HashSet;

use mongodb::bson::DateTime;
use proptest::prelude::*;
use wechatagent::behavior_signals as bs;

/// 内存版"partial unique 索引"：按 dedupe_key 去重，返回是否首次写入（== 生产
/// persist_signal 的 Ok(true) / Ok(false) 语义）。
fn index_insert(seen: &mut HashSet<String>, dedupe_key: &str) -> bool {
    seen.insert(dedupe_key.to_string())
}

// ── Property 1：replay_collapses_to_unique_keys ──────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// 任意 (wxid, msg_id) 对序列（刻意允许大量重复）重放后，真正写入次数必然
    /// 等于不同 dedupe_key 的数量——无论重放多少遍，每个 key 只落一次。
    #[test]
    fn replay_collapses_to_unique_keys(
        events in proptest::collection::vec(
            (0u8..4, 0u8..3),  // (wxid 维度小, msg_id 维度小) → 高碰撞，逼出重复
            1..40,
        ),
    ) {
        let mut seen = HashSet::new();
        let mut writes = 0usize;
        for (w, m) in &events {
            let wxid = format!("wxid_{w}");
            let msg = format!("msg_{m}");
            // 每条 inbound 都构造 latency 信号；同 (wxid,msg) 必撞同 key。
            let sig = bs::build_reply_latency(
                "ws",
                &wxid,
                &msg,
                DateTime::from_millis(1_000),
                Some(500),
            );
            if index_insert(&mut seen, &sig.dedupe_key) {
                writes += 1;
            }
        }
        // 真正写入数 == 不同 (wxid,msg) 组合数。
        let distinct: HashSet<_> = events
            .iter()
            .map(|(w, m)| (*w, *m))
            .collect();
        prop_assert_eq!(writes, distinct.len());
        prop_assert_eq!(seen.len(), distinct.len());
    }
}

// ── Property 2：silence_dedupe_per_outbound ──────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// 同一条 outbound（同毫秒时间戳）被多 tick 探测 → 只产一个 silence key。
    /// 不同 outbound 时间戳 → 必产不同 key。
    #[test]
    fn silence_dedupe_per_outbound(
        outbound_ms in 0i64..1_000_000_000,
        repeat in 1usize..20,
        delta in 1i64..100_000,
    ) {
        let mut seen = HashSet::new();
        let mut writes = 0usize;
        // 同一条 outbound 探测 repeat 次。
        for tick in 0..repeat {
            let observed = DateTime::from_millis(outbound_ms + delta + tick as i64);
            let sig = bs::build_silence(
                "ws",
                "wxid_s",
                DateTime::from_millis(outbound_ms),
                observed,
            );
            if index_insert(&mut seen, &sig.dedupe_key) {
                writes += 1;
            }
        }
        prop_assert_eq!(writes, 1, "同一 outbound 多 tick 只能落一条沉默信号");

        // 另一条不同 outbound → 必新增一个 key。
        let other = bs::build_silence(
            "ws",
            "wxid_s",
            DateTime::from_millis(outbound_ms + delta),
            DateTime::from_millis(outbound_ms + delta + 1),
        );
        prop_assert!(index_insert(&mut seen, &other.dedupe_key));
        prop_assert_eq!(seen.len(), 2);
    }
}

// ── Property 3：inbound_signal_types_never_collide ───────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// 同一条 inbound 的 latency / length / reactivation 三类信号 dedupe_key
    /// 两两不同——三类可在同一索引下共存，不会互相覆盖。
    #[test]
    fn inbound_signal_types_never_collide(
        w in 0u32..50,
        m in 0u32..50,
    ) {
        let wxid = format!("wxid_{w}");
        let msg = format!("msg_{m}");
        let inbound_at = DateTime::from_millis(10_000);

        let lat = bs::build_reply_latency("ws", &wxid, &msg, inbound_at, Some(1));
        let len = bs::build_reply_length("ws", &wxid, &msg, inbound_at, "hi");
        let react = bs::build_reactivation("ws", &wxid, &msg, inbound_at);

        let keys: HashSet<_> = [
            lat.dedupe_key.clone(),
            len.dedupe_key.clone(),
            react.dedupe_key.clone(),
        ]
        .into_iter()
        .collect();
        prop_assert_eq!(keys.len(), 3, "三类信号 key 必须两两不同");
    }
}

// ── Property 4：silence_always_censored_others_never ─────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// Iron Law ②：沉默信号恒 censored=true（删失），其余 T1 信号恒 false。
    /// 同时 source 恒 system_observed、confidence 恒 1.0（Iron Law ④）。
    #[test]
    fn silence_always_censored_others_never(
        outbound_ms in 0i64..1_000_000_000,
        latency_base in 0i64..1_000_000,
        content_len in 0usize..200,
    ) {
        let inbound_at = DateTime::from_millis(outbound_ms + latency_base + 1);
        let content: String = "x".repeat(content_len);

        let lat = bs::build_reply_latency("ws", "w", "m", inbound_at, Some(outbound_ms));
        let len = bs::build_reply_length("ws", "w", "m", inbound_at, &content);
        let react = bs::build_reactivation("ws", "w", "m", inbound_at);
        let silence = bs::build_silence(
            "ws",
            "w",
            DateTime::from_millis(outbound_ms),
            inbound_at,
        );

        for sig in [&lat, &len, &react] {
            prop_assert!(!sig.censored, "T1 行为信号不得 censored");
            prop_assert_eq!(&sig.source, bs::SOURCE_SYSTEM_OBSERVED);
            prop_assert_eq!(sig.confidence, 1.0);
        }
        prop_assert!(silence.censored, "沉默信号必须 censored=true");
        prop_assert_eq!(silence.unanswered, Some(true));
        prop_assert_eq!(&silence.source, bs::SOURCE_SYSTEM_OBSERVED);
        prop_assert_eq!(silence.confidence, 1.0);
    }
}

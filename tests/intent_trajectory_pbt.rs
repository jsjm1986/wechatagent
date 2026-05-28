//! Phase D / D1：`intent_trajectory` 滑窗 cap PBT。
//!
//! 镜像生产路径 mongo `$push + $slice: -MAX_ITEMS` 行为，纯函数 PBT 4 个不变量
//! （单文件累计 1024 cases ≥ 64 plan 门）：
//!
//! 1. **length_capped_at_max_items** — 任意 N + 1 次 push 后，长度永远
//!    `min(N + 1, MAX_ITEMS)`，不超过 50。
//! 2. **last_entry_always_preserved** — 最近一次 push 的 entry 永远在尾部。
//! 3. **fifo_drops_oldest_first** — 当 N+1 > MAX_ITEMS 时，被丢弃的恰好是最前
//!    的 `len_after_push - MAX_ITEMS` 项；保留的尾部与原 entry 顺序一致。
//! 4. **idempotent_under_cap** — 当 N + 1 ≤ MAX_ITEMS 时，输出长度 = 输入长度
//!    + 1（无截断）。
//!
//! 不依赖 testcontainers / mongodb / mock LLM —— 纯 in-memory 调用，挂在
//! `cargo test --tests` 默认通道，参与 R11.6 `pbt cumulative ≥ 33` 基线门。

use mongodb::bson::DateTime;
use proptest::prelude::*;
use wechatagent::agent::cap_intent_trajectory;
use wechatagent::models::IntentTrajectoryEntry;

fn mk_entry(turn: i32, intent: &str) -> IntentTrajectoryEntry {
    IntentTrajectoryEntry {
        turn_index: turn,
        intent: intent.to_string(),
        objection_type: None,
        recorded_at: DateTime::from_millis(turn as i64 * 1000),
    }
}

fn build_existing(n: usize) -> Vec<IntentTrajectoryEntry> {
    (0..n).map(|i| mk_entry(i as i32, &format!("intent_{i}"))).collect()
}

// ── Property 1：length_capped_at_max_items ───────────────────────────────

proptest! {
    /// 任意起始长度 N ∈ [0, 200]：push 一条后长度永远 ≤ MAX_ITEMS。
    #[test]
    fn length_capped_at_max_items(n in 0usize..=200) {
        let existing = build_existing(n);
        let after = cap_intent_trajectory(&existing, mk_entry(9999, "new"));
        let cap = IntentTrajectoryEntry::MAX_ITEMS;
        let expected = (n + 1).min(cap);
        prop_assert_eq!(after.len(), expected);
        prop_assert!(after.len() <= cap);
    }
}

// ── Property 2：last_entry_always_preserved ──────────────────────────────

proptest! {
    /// 任意起始长度 N ∈ [0, 200]：push 后尾部一定是新 entry。
    #[test]
    fn last_entry_always_preserved(n in 0usize..=200, turn in -1000i32..=1000) {
        let existing = build_existing(n);
        let new_entry = mk_entry(turn, "marker_unique_xyz");
        let after = cap_intent_trajectory(&existing, new_entry.clone());
        let last = after.last().expect("non-empty after push");
        prop_assert_eq!(last.turn_index, turn);
        prop_assert_eq!(last.intent.as_str(), "marker_unique_xyz");
    }
}

// ── Property 3：fifo_drops_oldest_first ──────────────────────────────────

proptest! {
    /// 起始长度 N > MAX_ITEMS - 1：被丢弃的恰好是最前 (N + 1 - MAX_ITEMS) 项；
    /// 保留的尾部与原顺序一致。
    #[test]
    fn fifo_drops_oldest_first(n in 50usize..=200) {
        let cap = IntentTrajectoryEntry::MAX_ITEMS;
        let existing = build_existing(n);
        let after = cap_intent_trajectory(&existing, mk_entry(9000, "tail"));
        prop_assert_eq!(after.len(), cap);

        // 头部应当是 existing 的第 (n + 1 - cap) 项（被截掉前面那些）。
        let drop_n = n + 1 - cap;
        let head_turn = after[0].turn_index;
        prop_assert_eq!(head_turn, drop_n as i32);

        // 尾部 cap-1 项应当与 existing 后段顺序一致（除了最后那条新 entry）。
        for i in 0..(cap - 1) {
            let kept = &after[i];
            let expected_turn = (drop_n + i) as i32;
            prop_assert_eq!(kept.turn_index, expected_turn);
        }
    }
}

// ── Property 4：idempotent_under_cap ─────────────────────────────────────

proptest! {
    /// 起始长度 N + 1 ≤ MAX_ITEMS：push 后长度 = N + 1，且原 entry 全部保留
    /// （顺序不变），新 entry 在尾部。
    #[test]
    fn idempotent_under_cap(n in 0usize..50) {
        // n in [0, 49] 保证 n + 1 <= 50 = MAX_ITEMS
        let existing = build_existing(n);
        let new_entry = mk_entry(7777, "tail_under_cap");
        let after = cap_intent_trajectory(&existing, new_entry.clone());
        prop_assert_eq!(after.len(), n + 1);
        // 前 n 项与 existing 完全一致
        for i in 0..n {
            prop_assert_eq!(after[i].turn_index, existing[i].turn_index);
            prop_assert_eq!(after[i].intent.clone(), existing[i].intent.clone());
        }
        // 尾部是新 entry
        prop_assert_eq!(after[n].turn_index, 7777);
    }
}

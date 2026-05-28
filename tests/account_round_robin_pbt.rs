//! Phase D / D4：`account_scheduler` 多账号轮询 PBT。
//!
//! 镜像生产路径 [`assign_account`] 的核心决策（`decide_assigned_account`）；纯
//! 函数 PBT 4 个不变量（单文件累计 1024 cases ≥ 64 plan 门）：
//!
//! 1. **all_full_capacity_falls_back_or_none** — 当所有 online 账号都
//!    `used >= capacity` 时，必须退化为 online-only fallback；若同时有 online
//!    账号则永远返回 Some（保送达 > 严格遵守 capacity），否则返回 None。
//! 2. **stable_pick_per_wxid** — 给定同 (accounts, used, cur_hour, wxid) 两次
//!    调用必返回同一 account_id；同 wxid 决策可复现是 D4 的散列锚。
//! 3. **off_hours_excluded_when_alternative_exists** — 命中 off_hours 的账号
//!    在严格池（有其它候选时）被排除；只在"全部命中"时才退化进 fallback。
//! 4. **capacity_zero_unbounded** — capacity=0 的账号无论 used_today 多大都被
//!    视为可参与候选；不被 capacity 闸排除。
//!
//! 不依赖 testcontainers / mongodb / mock LLM —— 纯 in-memory 调用，挂在
//! `cargo test --tests` 默认通道。

use mongodb::bson::DateTime as BsonDate;
use proptest::prelude::*;
use wechatagent::account_scheduler::decide_assigned_account;
use wechatagent::models::{HourRange, WechatAccount};

fn account(id: &str, online: bool, capacity: u32, off: Vec<HourRange>) -> WechatAccount {
    WechatAccount {
        id: None,
        workspace_id: "default".to_string(),
        account_id: id.to_string(),
        alias: id.to_string(),
        display_name: id.to_string(),
        app_id: None,
        wxid: None,
        nick_name: None,
        mcp_base_url: None,
        mcp_api_key: None,
        online,
        last_sync_at: BsonDate::now(),
        capacity,
        persona_tag: Some("sales_assistant".to_string()),
        off_hours: off,
        created_at: BsonDate::now(),
        updated_at: BsonDate::now(),
    }
}

// ── Property 1：all_full_capacity_falls_back_or_none ─────────────────────

proptest! {
    /// 给定 N 个 online 账号、所有 capacity=1 且 used=1（满）→ 严格池为空，
    /// 必须退化进 fallback（任意 online 账号），返回 Some；至少有一个 online
    /// 账号时绝不返回 None。
    #[test]
    fn all_full_capacity_falls_back_or_none(
        n in 1usize..=8,
        wxid_seed in any::<u32>(),
        cur_hour in 0u32..24,
    ) {
        let accounts: Vec<WechatAccount> = (0..n)
            .map(|i| account(&format!("acc_{i}"), true, 1, vec![]))
            .collect();
        let used: Vec<(String, i64)> = (0..n)
            .map(|i| (format!("acc_{i}"), 1))
            .collect();
        let wxid = format!("wxid_{wxid_seed}");

        let pick = decide_assigned_account(&accounts, &used, cur_hour, &wxid);
        prop_assert!(pick.is_some(), "fallback must select some online account");
        let id = &pick.unwrap().account_id;
        prop_assert!(id.starts_with("acc_"));
    }
}

// ── Property 2：stable_pick_per_wxid ─────────────────────────────────────

proptest! {
    /// 同 (accounts, used, cur_hour, wxid) 决策稳定；不会因为多次调用拿到不同
    /// 账号。这是 D4 散列稳定性的 PBT 锚。
    #[test]
    fn stable_pick_per_wxid(
        n in 1usize..=8,
        wxid_seed in any::<u32>(),
        cur_hour in 0u32..24,
        capacity_seed in 1u32..=100,
    ) {
        let accounts: Vec<WechatAccount> = (0..n)
            .map(|i| account(&format!("acc_{i}"), true, capacity_seed, vec![]))
            .collect();
        let used: Vec<(String, i64)> = Vec::new();
        let wxid = format!("wxid_{wxid_seed}");

        let p1 = decide_assigned_account(&accounts, &used, cur_hour, &wxid)
            .expect("non-empty pool")
            .account_id
            .clone();
        let p2 = decide_assigned_account(&accounts, &used, cur_hour, &wxid)
            .expect("non-empty pool")
            .account_id
            .clone();
        prop_assert_eq!(p1, p2);
    }
}

// ── Property 3：off_hours_excluded_when_alternative_exists ───────────────

proptest! {
    /// 在 cur_hour 命中 off_hours 的账号 + 至少一个不在 off_hours 的账号 →
    /// 严格池非空，被选中的账号必然不在 off_hours。
    /// （只有"所有账号都命中"时才退化到 fallback，那条 fallback 路径在 P1 已
    /// 覆盖。）
    #[test]
    fn off_hours_excluded_when_alternative_exists(
        wxid_seed in any::<u32>(),
        cur_hour in 1u32..23,
    ) {
        // off_hours 命中区间 = [cur_hour, cur_hour + 1)
        let off = vec![HourRange {
            start_hour: cur_hour,
            end_hour: cur_hour + 1,
        }];
        let accounts = vec![
            account("acc_off", true, 100, off.clone()),
            account("acc_open", true, 100, vec![]),
        ];
        let used: Vec<(String, i64)> = Vec::new();
        let wxid = format!("wxid_{wxid_seed}");

        let pick = decide_assigned_account(&accounts, &used, cur_hour, &wxid)
            .expect("non-empty pool");
        prop_assert_eq!(pick.account_id.as_str(), "acc_open");
    }
}

// ── Property 4：capacity_zero_unbounded ──────────────────────────────────

proptest! {
    /// capacity=0 的账号无论 used 多大都参与候选；与一个普通 capacity=1 used=1
    /// 满账号同池时，必选 capacity=0 的那个。
    #[test]
    fn capacity_zero_unbounded(
        used_amount in 0i64..=10_000,
        wxid_seed in any::<u32>(),
        cur_hour in 0u32..24,
    ) {
        let accounts = vec![
            account("acc_full", true, 1, vec![]),
            account("acc_unbounded", true, 0, vec![]),
        ];
        let used = vec![
            ("acc_full".to_string(), 1),
            ("acc_unbounded".to_string(), used_amount),
        ];
        let wxid = format!("wxid_{wxid_seed}");

        let pick = decide_assigned_account(&accounts, &used, cur_hour, &wxid)
            .expect("non-empty pool");
        prop_assert_eq!(pick.account_id.as_str(), "acc_unbounded");
    }
}

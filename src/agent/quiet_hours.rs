//! 作息门控（quiet hours，#69）：运营方时区内的"静默时段"判定与"醒来时刻"计算。
//!
//! 产品语义：客户在运营方休息时段（默认 22:00–08:00）发来的消息**不立即回**，
//! 而是把唯一的 `inbound_reply` 被动回复义务排到运营方醒来时段，届时一次性基于累积
//! 消息回复——最像真人（睡觉时不回、醒来看完所有消息再答）。主动发送（planner
//! 催进 / 承诺跟进）若在静默时段到点，则**重排**到醒来时刻而非取消（避免丢承诺）。
//!
//! 唯一豁免（S5-3）：交易域 profile 下的**显式购买/付款承诺**入站即时应答、不等醒来
//! （半夜要下单的客户等 10 小时 = 直接丢单），见
//! [`bypass_deferral_for_explicit_buying_intent`]。
//!
//! 时区：用**运营参数固定偏移** `quiet_hours_tz_offset_hours`（小时，如中国 +8），
//! 不依赖部署宿主时区（`chrono::Local` 取的是进程时区，容器多默认 UTC，会让
//! "22:00 静默"实际在 UTC 22:00 触发、偏 8 小时）。判定全部用 epoch 毫秒 + 偏移的
//! 纯整数运算——既消除宿主依赖，又规避本地时刻歧义（夏令时 / 不存在的本地时刻）。
//!
//! 全部判定逻辑做成**纯函数**（UTC 毫秒 + 偏移 / 小时数），完全本地可测；只有两个
//! 取真实时钟的薄包装（[`is_quiet_now`] / [`next_wake_at`]）用 `Utc::now()`。

use chrono::Utc;

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

/// 从 contact 标识派生确定性 jitter（毫秒），落在 [0, max_seconds*1000]。同一 seed
/// 恒定（可复现、可测），不同 contact 散开，把整点唤醒打散避免齐发。max_seconds=0 → 恒 0。
///
/// 用 FNV-1a 而非 `DefaultHasher`：后者哈希算法跨 Rust 版本不保证稳定，会让 jitter
/// 不可复现；FNV-1a 是固定常量算法，同一 seed 在任何版本恒定。
pub(crate) fn jitter_ms_for_seed(seed: &str, max_seconds: u32) -> i64 {
    if max_seconds == 0 {
        return 0;
    }
    let mut h: u64 = 0xcbf29ce484222325; // FNV-1a offset basis
    for b in seed.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3); // FNV prime
    }
    let max_ms = (max_seconds as u64) * 1000;
    (h % (max_ms + 1)) as i64
}

/// 给定 UTC 毫秒、醒来小时 `end`、时区偏移、jitter 毫秒，返回下一次"运营方本地
/// `end`:00 + jitter"对应的 UTC 毫秒。严格在 `now` 之后（恰好命中 `end`:00 也取次日，
/// 保证 wake 落在未来，与旧 `next_wake_instant` 的"严格大于"语义一致）。jitter 把同一
/// workspace 多客户的整点唤醒散开（per-contact 确定性偏移，见 [`jitter_ms_for_seed`]）。
pub(crate) fn next_wake_utc_ms(
    now_utc_ms: i64,
    end: u32,
    tz_offset_hours: i32,
    jitter_ms: i64,
) -> i64 {
    let off = (tz_offset_hours as i64) * 3_600_000;
    let local_ms = now_utc_ms + off;
    let day = local_ms.div_euclid(86_400_000); // 本地"第几天"
    let end_ms_today = day * 86_400_000 + (end.min(23) as i64) * 3_600_000;
    let local_target = if end_ms_today > local_ms {
        end_ms_today
    } else {
        end_ms_today + 86_400_000
    };
    (local_target - off) + jitter_ms // 回到 UTC 并叠加 per-contact jitter
}

/// 薄包装：当前真实时刻（按运营方偏移换算）是否在静默时段。生产判定入口。
pub(crate) fn is_quiet_now(start: u32, end: u32, tz_offset_hours: i32) -> bool {
    in_quiet_hours(
        hour_in_offset(Utc::now().timestamp_millis(), tz_offset_hours),
        start,
        end,
    )
}

/// 薄包装：从现在算下一次醒来时刻（UTC）+ per-contact jitter，转成 BSON `DateTime`
/// 供 task `run_at` 用。`jitter_seed` 通常传 `contact.wxid`（同 contact 恒定偏移、
/// 不同 contact 散开），`jitter_max_seconds` 来自 `config.wake_jitter_max_seconds`。
pub(crate) fn next_wake_at(
    end: u32,
    tz_offset_hours: i32,
    jitter_seed: &str,
    jitter_max_seconds: u32,
) -> mongodb::bson::DateTime {
    let jitter = jitter_ms_for_seed(jitter_seed, jitter_max_seconds);
    mongodb::bson::DateTime::from_millis(next_wake_utc_ms(
        Utc::now().timestamp_millis(),
        end,
        tz_offset_hours,
        jitter,
    ))
}

/// S5-3：静默时段 defer 的**显式交易意图豁免**判定（确定性、零 LLM 成本）。
///
/// 与 reaction 确定性购买下限（`reaction.rs` 的 `deterministic_buying`）**同一词表、
/// 同一语义门**：交易域 profile（`transaction_facts_enabled=true`）+ 显式购买/付款
/// 承诺短语（[`super::reaction::explicit_buying_intent`]：≤120 字、反例 marker 过滤）。
/// 两个条件缺一不可——非交易域（情感陪伴等）即便命中购买短语也不豁免，杜绝
/// admin 误配下交易语义渗入非交易对话。
///
/// 设计边界：v1 刻意只做交易词表豁免，**不做"高意向阶段"判定**——阶段集合是
/// 行业可配的（`system_taxonomies` / DomainProfile），硬编码销售阶段违反通用化；
/// 后续若需要按阶段豁免，应走 DomainProfile 配置声明，而不是在这里加分支。
///
/// 放在 quiet_hours 模块（而非 webhooks）：`reaction` 是 `agent` 的私有子模块，
/// 词表函数 `pub(crate)` 但模块路径对 crate 根不可见；本模块是 `pub(crate) mod`，
/// 由同 parent 内转发即可让 webhooks 消费，无须放宽 `reaction` 模块本身的可见性。
pub(crate) fn bypass_deferral_for_explicit_buying_intent(
    active_profile: &crate::models::DomainProfile,
    content: &str,
) -> bool {
    active_profile.transaction_facts_enabled && super::reaction::explicit_buying_intent(content)
}

/// 解析某 contact 的**有效作息门控开关**——现行语义是 **workspace 开关唯一权威**：
/// 直接返回 `workspace_enabled`（即 `runtime.quiet_hours_enabled`），contact /
/// profile 两级的 `quiet_hours.enabled_override` 已**不再**参与调度判定。
///
/// 历史脉络：universal-domain-adaptation H19 / G04 曾把本函数接上
/// [`resolve_operation_mode`](crate::planner::resolve_operation_mode) 三级链
/// （contact override → profile.per_relationship → profile 默认范式），后收敛回
/// workspace-only。保留 `_contact` / `_profile` 入参只为调用面签名稳定与滚动升级
/// 兼容（老数据里的 override 字段仍可读，但不改变行为）。纯函数、不查 DB。
pub(crate) fn effective_quiet_hours_enabled(
    _contact: &crate::models::Contact,
    _profile: &crate::models::DomainProfile,
    workspace_enabled: bool,
) -> bool {
    // Workspace policy is authoritative. Contact/profile overrides remain readable only for
    // rolling-upgrade compatibility and no longer alter scheduling behavior.
    workspace_enabled
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
        let wake = next_wake_utc_ms(now, 8, 8, 0);
        assert_eq!(wake, utc_ms("2026-06-09T00:00:00Z"));
    }

    #[test]
    fn wake_next_day_when_end_already_passed() {
        // 北京 23:00（= UTC 15:00），end=8 → 北京次日 08:00（= 次日 UTC 00:00）。
        let now = utc_ms("2026-06-09T15:00:00Z");
        let wake = next_wake_utc_ms(now, 8, 8, 0);
        assert_eq!(wake, utc_ms("2026-06-10T00:00:00Z"));
    }

    #[test]
    fn wake_strictly_after_now_at_exact_hour() {
        // 恰好北京 08:00（= UTC 00:00）命中 end → 不取当天，取次日，保证 wake 严格在未来。
        let now = utc_ms("2026-06-09T00:00:00Z");
        let wake = next_wake_utc_ms(now, 8, 8, 0);
        assert_eq!(wake, utc_ms("2026-06-10T00:00:00Z"));
        assert!(wake > now, "wake 必须严格在 now 之后");
    }

    #[test]
    fn wake_respects_negative_offset() {
        // 西五区 -5：UTC 12:00 = 当地 07:00，end=8 → 当地当天 08:00 = UTC 13:00。
        let now = utc_ms("2026-06-09T12:00:00Z");
        let wake = next_wake_utc_ms(now, 8, -5, 0);
        assert_eq!(wake, utc_ms("2026-06-09T13:00:00Z"));
    }

    #[test]
    fn jitter_is_deterministic_per_seed() {
        let now = 1_700_000_000_000;
        let a = next_wake_utc_ms(now, 8, 8, jitter_ms_for_seed("wxid_alice", 900));
        let b = next_wake_utc_ms(now, 8, 8, jitter_ms_for_seed("wxid_alice", 900));
        assert_eq!(a, b);
    }
    #[test]
    fn jitter_differs_across_seeds() {
        assert_ne!(
            jitter_ms_for_seed("wxid_alice", 900),
            jitter_ms_for_seed("wxid_bob", 900)
        );
    }
    #[test]
    fn jitter_within_bounds() {
        for seed in ["a", "b", "c", "xyz", "wxid_123456"] {
            let j = jitter_ms_for_seed(seed, 900);
            assert!(j >= 0 && j <= 900 * 1000, "jitter 越界: {}", j);
        }
    }
    #[test]
    fn jitter_zero_max_is_noop() {
        assert_eq!(jitter_ms_for_seed("anything", 0), 0);
    }

    /// G04：构造一个 managed、无 operation_mode_override 的最小 Contact，便于断言
    /// `effective_quiet_hours_enabled` 走 resolve_operation_mode 三级链回落 profile 级。
    fn contact_no_override() -> crate::models::Contact {
        crate::models::Contact {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            wxid: "quiet_hours_test".to_string(),
            nickname: None,
            remark: None,
            alias: None,
            avatar_url: None,
            agent_status: crate::models::AgentStatus::Managed,
            sex: None,
            human_profile_note: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            manual_tags: Vec::new(),
            manual_tags_updated_at: None,
            manual_tags_by: None,
            confirmed_tags: Vec::new(),
            bayesian_signals: Vec::new(),
            personality_profile: None,
            tags_version: 0,
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: mongodb::bson::Document::new(),
            profile_attributes: mongodb::bson::Document::new(),
            profile_updated_at: None,
            domain_attributes: None,
            domain_attributes_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            custom_agent_instructions: None,
            operation_mode_override: None,
            last_outbound_style: None,
            intent_trajectory: Vec::new(),
            outcome_events: Vec::new(),
            locale: None,
            created_at: mongodb::bson::DateTime::now(),
            updated_at: mongodb::bson::DateTime::now(),
        }
    }

    /// Workspace 作息开关是唯一权威来源；历史 profile/contact override 仅保留读取兼容，
    /// 不得改变调度结果。
    #[test]
    fn quiet_hours_uses_workspace_policy_only() {
        let contact = contact_no_override();
        let mut profile = crate::agent::domain_profile::default_domain_profile("default");
        let mut mode = crate::models::OperationMode::default();
        mode.quiet_hours.enabled_override = Some(false);
        profile.operation_mode = mode;

        assert!(effective_quiet_hours_enabled(&contact, &profile, true));
        assert!(!effective_quiet_hours_enabled(&contact, &profile, false));
    }

    // ── S5-3：静默 defer 的显式交易意图豁免门 ──────────────────────────

    /// 交易域（DEFAULT 销售 profile，transaction_facts_enabled=true）+ 显式购买/
    /// 付款承诺 → 豁免为真；寒暄、反例 marker（"如果"假设句）不豁免。
    #[test]
    fn buying_intent_bypass_hits_only_explicit_commitment_on_transaction_profile() {
        let tx = crate::agent::domain_profile::default_domain_profile("ws-quiet-tx");
        assert!(bypass_deferral_for_explicit_buying_intent(
            &tx,
            "我要买，现在付款"
        ));
        assert!(!bypass_deferral_for_explicit_buying_intent(
            &tx,
            "今天好累呀，晚点再聊"
        ));
        // 与 reaction 确定性购买下限同一词表同一语义：假设/反例 marker 一样被过滤。
        assert!(!bypass_deferral_for_explicit_buying_intent(
            &tx,
            "如果我要买，现在付款有优惠吗"
        ));
        assert!(!bypass_deferral_for_explicit_buying_intent(&tx, "先不买了"));
    }

    /// 非交易域（情感陪伴 example，transaction_facts_enabled=false）：同一购买短语
    /// 也不豁免——profile 门与词表门缺一不可。
    #[test]
    fn buying_intent_bypass_requires_transaction_profile() {
        let nontx =
            crate::agent::domain_profile::example_emotional_companion_profile("ws-quiet-nontx");
        assert!(!bypass_deferral_for_explicit_buying_intent(
            &nontx,
            "我要买，现在付款"
        ));
    }
}

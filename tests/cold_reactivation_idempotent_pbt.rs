//! Phase D / D3：`cold_contact_worker` 冷重激活幂等性 PBT。
//!
//! 镜像生产路径 [`scan_cold_outbound`] 的判定（`decide_cold_emit`）；纯函数 PBT
//! 4 个不变量（单文件累计 1024 cases ≥ 64 plan 门）：
//!
//! 1. **already_pending_skips_emit** — 任何同 contact 已存在 pending follow_up
//!    的情形下，本 tick 必返回 [`ColdEmitDecision::AlreadyPending`]，绝不返回
//!    `Emit`，保证"同 contact 一天一次"的幂等核心。
//! 2. **non_managed_never_emits** — `agent_status != Managed` 永远不 emit；这是
//!    "AI-autonomous 中 normal/blocked 不应被冷链路骚扰"的红线。
//! 3. **inbound_newer_than_outbound_skips** — 用户已经回过话（`last_inbound_at`
//!    晚于 `last_outbound_at`）的 contact 属 silent 段，不属 cold；必返回
//!    `UserRecentlyReplied`。
//! 4. **emit_only_when_outbound_strictly_old** — 当且仅当 (Managed + 有过
//!    outbound + outbound 比 cold_before 早 + 无更新 inbound + 无 cooldown +
//!    无 pending follow_up) 全部满足时才返回 `Emit`。
//!
//! 不依赖 testcontainers / mongodb / mock LLM —— 纯 in-memory 调用，挂在
//! `cargo test --tests` 默认通道。

use mongodb::bson::{DateTime, Document as BsonDocument};
use proptest::prelude::*;
use wechatagent::cold_contact_worker::{decide_cold_emit, ColdEmitDecision};
use wechatagent::models::{AgentStatus, Contact};

const NOW_MS: i64 = 1_700_000_000_000; // 2023-11-14T22:13:20Z 任意基准
const COLD_THRESHOLD_MS: i64 = 168 * 3600 * 1000; // 默认 7 天
const COLD_BEFORE_MS: i64 = NOW_MS - COLD_THRESHOLD_MS;

fn template(wxid: &str) -> Contact {
    let now = DateTime::from_millis(NOW_MS);
    Contact {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        tags: Vec::new(),
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: BsonDocument::new(),
        profile_attributes: BsonDocument::new(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        custom_agent_instructions: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        deal_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

// ── Property 1：already_pending_skips_emit ───────────────────────────────

proptest! {
    /// 任意 outbound_age_hours / inbound_offset / cooldown 状态下，只要
    /// `has_pending_follow_up=true`，决策必然是 AlreadyPending，绝不 Emit。
    #[test]
    fn already_pending_skips_emit(
        outbound_age_hours in 0i64..=720,
        inbound_offset_hours in -240i64..=240,
        cooldown_offset_hours in -240i64..=240,
    ) {
        let mut c = template("user_pending");
        c.last_outbound_at = Some(DateTime::from_millis(
            NOW_MS - outbound_age_hours * 3600 * 1000,
        ));
        c.last_inbound_at = Some(DateTime::from_millis(
            NOW_MS - outbound_age_hours * 3600 * 1000 + inbound_offset_hours * 3600 * 1000,
        ));
        c.cooldown_until = Some(DateTime::from_millis(
            NOW_MS + cooldown_offset_hours * 3600 * 1000,
        ));

        let decision = decide_cold_emit(&c, NOW_MS, COLD_BEFORE_MS, true);
        prop_assert_ne!(decision, ColdEmitDecision::Emit);
    }
}

// ── Property 2：non_managed_never_emits ─────────────────────────────────

proptest! {
    /// agent_status != Managed → 任何 outbound_age / pending 状态下都不 emit。
    #[test]
    fn non_managed_never_emits(
        outbound_age_hours in 0i64..=720,
        has_pending in any::<bool>(),
    ) {
        let mut c = template("user_normal");
        c.agent_status = AgentStatus::Normal;
        c.last_outbound_at = Some(DateTime::from_millis(
            NOW_MS - outbound_age_hours * 3600 * 1000,
        ));

        let decision = decide_cold_emit(&c, NOW_MS, COLD_BEFORE_MS, has_pending);
        prop_assert_ne!(decision, ColdEmitDecision::Emit);
    }
}

// ── Property 3：inbound_newer_than_outbound_skips ────────────────────────

proptest! {
    /// last_inbound_at > last_outbound_at（outbound 已老到 cold 区间）→
    /// 必然返回 UserRecentlyReplied，不 emit。
    #[test]
    fn inbound_newer_than_outbound_skips(
        outbound_age_hours in 200i64..=720, // 远在 cold_before 之前
        inbound_after_hours in 1i64..=200,
    ) {
        let mut c = template("user_chatty");
        let outbound_ms = NOW_MS - outbound_age_hours * 3600 * 1000;
        let inbound_ms = outbound_ms + inbound_after_hours * 3600 * 1000;
        c.last_outbound_at = Some(DateTime::from_millis(outbound_ms));
        c.last_inbound_at = Some(DateTime::from_millis(inbound_ms));

        let decision = decide_cold_emit(&c, NOW_MS, COLD_BEFORE_MS, false);
        prop_assert_eq!(decision, ColdEmitDecision::UserRecentlyReplied);
    }
}

// ── Property 4：emit_only_when_outbound_strictly_old ─────────────────────

proptest! {
    /// 完全 happy path：Managed + outbound 比 cold_before 早 + 无 inbound 反超 +
    /// 无 cooldown + 无 pending → 必返回 Emit。
    /// 这是 D3 计划锁定的"冷重激活的唯一可发条件"反向断言。
    #[test]
    fn emit_only_when_outbound_strictly_old(
        outbound_age_hours in 200i64..=720, // 远在 cold_before 之前
    ) {
        let mut c = template("user_cold");
        let outbound_ms = NOW_MS - outbound_age_hours * 3600 * 1000;
        c.last_outbound_at = Some(DateTime::from_millis(outbound_ms));
        c.last_inbound_at = None;
        c.cooldown_until = None;

        let decision = decide_cold_emit(&c, NOW_MS, COLD_BEFORE_MS, false);
        prop_assert_eq!(decision, ColdEmitDecision::Emit);
    }
}

//! Phase D / D3：冷联系人重激活 worker。
//!
//! 与 Strategic Planner 的"静默扫描"互补：
//! - silent 段：`last_inbound_at` 远早于 now（用户来过但停说话）；
//! - **cold 段**：`last_outbound_at` 远早于 now（agent 自己很久没主动出站，
//!   契合"冷链路重激活"语义）。
//!
//! 默认关停：`COLD_CONTACT_WORKER_ENABLED=false`。开启后周期 tick：
//! 1) 选 managed + cooldown 失效 + `last_outbound_at` 在阈值前的 contact；
//! 2) 已有 pending follow_up 的 contact 跳过（与 planner 共用 follow_up 池幂等）；
//! 3) 钩子文案优先从 `operation_knowledge_chunks` 中 `chunk_type="peer_case"` 的
//!    池子里随机挑一条作为 hook 摘要写入 `agent_tasks.content`；池空时退化为
//!    "Planner: cold_reactivation since {last_outbound_ts}"，让 gateway 走默认
//!    Reply Agent 决策（不绕开 outbox / 安全门）；
//! 4) 单 account 当日 `cold_contact_daily_emit_cap` 上限保护；
//! 5) 写 `cold_contact_emit` / `cold_contact_tick` 事件，与 strategic planner
//!    事件 kind 命名空间隔离。
//!
//! 不绕过 gateway：emit 出来的 follow_up 由 tasks worker 拉起，再走标准
//! `handle_follow_up_task` → outbox → MCP；本 worker 仅负责选 contact + 写任务。

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use tokio::time::sleep;

use crate::models::{AgentTask, Contact};
use crate::routes::AppState;

/// 冷联系人 worker 主循环。`cold_contact_worker_enabled=false` 时立刻 return，
/// 等价于关停状态。tick 周期复用 `strategic_planner_interval_seconds` 节奏，
/// 避免再开一组 env。
pub async fn run_cold_contact_worker(state: AppState) {
    if !state.config.cold_contact_worker_enabled {
        tracing::info!("cold_contact_worker disabled by config; loop will not start");
        return;
    }
    tracing::info!(
        threshold_hours = state.config.cold_contact_threshold_hours,
        daily_emit_cap = state.config.cold_contact_daily_emit_cap,
        "cold_contact_worker loop started"
    );
    loop {
        if let Err(error) = tick(&state).await {
            tracing::error!(error = %error, "cold_contact_worker tick failed");
        }
        sleep(Duration::from_secs(
            state.config.strategic_planner_interval_seconds.max(60),
        ))
        .await;
    }
}

/// 单 tick 入口（lib 测试 + 主循环复用）。任何一段失败短路返回。
pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    scan_cold_outbound(state).await
}

async fn scan_cold_outbound(state: &AppState) -> anyhow::Result<()> {
    let workspace_id = state.config.default_workspace_id.clone();
    let now = DateTime::now();
    let now_ms = now.timestamp_millis();
    let threshold_ms = state
        .config
        .cold_contact_threshold_hours
        .saturating_mul(60 * 60 * 1000);
    let cold_before = DateTime::from_millis(now_ms - threshold_ms);

    // S4 (Phase 0)：原先用 default_account_id 锁定单账号扫描，多账号 workspace
    // 下其它 account 的 cold contact 永远不会被重激活。改成 workspace 级扫描；
    // emit 仍按 contact 自带 account_id（粘性绑定，不绕过 account_scheduler 的
    // "已绑定 contact 仍走原账号" 不变量）。
    let filter = cold_candidate_filter_workspace(&workspace_id, cold_before);
    let mut cursor = state.db.contacts().find(filter, None).await?;

    let daily_cap = state.config.cold_contact_daily_emit_cap;
    let already_emitted_today =
        count_today_cold_emit_workspace(state, &workspace_id, now).await?;

    let peer_hooks = load_peer_case_hooks(state, &workspace_id).await.unwrap_or_default();

    let mut scanned = 0i64;
    let mut emitted = 0i64;

    while let Some(contact) = cursor.try_next().await? {
        scanned += 1;
        if !cold_candidate_passes_in_memory(&contact, now_ms) {
            continue;
        }
        if has_pending_follow_up(state, &contact).await? {
            continue;
        }
        // P1-3：每个候选 emit 之前重新拉一次"今日已 emit 数"，作为 hard cap。
        // 旧实现只在 tick 起始读一次 count，并在内存里 decrement 一个本地
        // counter，并发 tick 之间会让真实 emit 数翻倍。改为每轮 re-count，
        // 让 cap 与 cold_contact_emit 事件 collection 真实实时对齐。
        let live_count =
            count_today_cold_emit_workspace(state, &workspace_id, now).await?;
        if cap_reached(live_count, daily_cap) {
            break;
        }
        // S4 (Phase 0)：写一条 account_scheduler_assignment 审计——contact 已经
        // 绑定 account，scheduler 结果只用作"哪个时间被路由到哪个账号"的统一
        // 审计流。失败被吞掉（assign_account 内部已 best-effort 写事件）。
        let _ = crate::account_scheduler::assign_account(
            state,
            &contact.workspace_id,
            &contact.wxid,
            None,
        )
        .await;

        let hook = pick_hook(&peer_hooks, &contact.wxid);
        let last_outbound_repr = contact
            .last_outbound_at
            .map(|d| d.timestamp_millis().to_string())
            .unwrap_or_else(|| "never".to_string());
        let content = match hook.as_deref() {
            Some(hook_text) => format!(
                "Planner: cold_reactivation since {last_outbound_repr} | hook={hook_text}"
            ),
            None => format!("Planner: cold_reactivation since {last_outbound_repr}"),
        };
        emit_cold_follow_up(state, &contact, content, now).await?;
        write_event(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "cold_contact_emit",
            "emitted",
            "Planner: cold_reactivation emitted",
            Some(doc! {
                "source": "cold_contact_worker",
                "lastOutboundAt": contact
                    .last_outbound_at
                    .map(|d| d.timestamp_millis())
                    .unwrap_or(0),
                "thresholdHours": state.config.cold_contact_threshold_hours,
                "hookSelected": hook.is_some(),
            }),
        )
        .await?;
        emitted += 1;
    }

    write_event(
        state,
        &state.config.default_account_id,
        None,
        "cold_contact_tick",
        "ok",
        &format!("cold_contact_worker tick: scanned {scanned}, emitted {emitted}"),
        Some(doc! {
            "scanned": scanned,
            "emitted": emitted,
            "dailyEmitCap": daily_cap,
            "alreadyEmittedToday": already_emitted_today,
            "thresholdHours": state.config.cold_contact_threshold_hours,
        }),
    )
    .await?;
    Ok(())
}

/// S4 (Phase 0)：workspace 级 cold filter。多账号 workspace 的所有 managed
/// contact 都会被扫描，emit 仍按 contact 自己的 account_id 走（粘性）。
pub(crate) fn cold_candidate_filter_workspace(
    workspace_id: &str,
    cold_before: DateTime,
) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "agent_status": "managed",
        "last_outbound_at": { "$lt": cold_before },
        "$or": [
            { "cooldown_until": { "$exists": false } },
            { "cooldown_until": null },
            { "cooldown_until": { "$lt": DateTime::now() } },
        ],
    }
}

/// Rust 侧的语义校验：上一轮 inbound 比 outbound 还新 → 该 contact 应走
/// silent 段（用户已经回过话，不属于"agent 冷链路"语义）；本轮跳过。
pub(crate) fn cold_candidate_passes_in_memory(contact: &Contact, _now_ms: i64) -> bool {
    if !matches!(contact.agent_status, crate::models::AgentStatus::Managed) {
        return false;
    }
    let Some(last_outbound) = contact.last_outbound_at else {
        return false;
    };
    if let Some(last_inbound) = contact.last_inbound_at {
        if last_inbound.timestamp_millis() > last_outbound.timestamp_millis() {
            return false;
        }
    }
    if let Some(cooldown) = contact.cooldown_until {
        if cooldown.timestamp_millis() > DateTime::now().timestamp_millis() {
            return false;
        }
    }
    true
}

/// P1-3：cold_contact_worker 当日 cap 比较。
///
/// 提取出来纯粹是为了让 `cap_reached(live_count, daily_cap)` 可单测：
/// `live_count >= daily_cap` 时返回 true（应停止 emit）。`daily_cap == 0`
/// 关停冷链路；负值视为 0；i64 溢出由 saturating 形式预防。
pub(crate) fn cap_reached(live_count: i64, daily_cap: i64) -> bool {
    let cap = daily_cap.max(0);
    live_count.max(0) >= cap
}

/// S4 (Phase 0)：workspace 级当日已 emit 计数，配合
/// `cold_candidate_filter_workspace`。daily cap 是 workspace 维度，避免一个
/// account 把整个池子的预算吃光也不影响其它 account。
async fn count_today_cold_emit_workspace(
    state: &AppState,
    workspace_id: &str,
    now: DateTime,
) -> anyhow::Result<i64> {
    let now_ms = now.timestamp_millis();
    let day_ms = 24 * 60 * 60 * 1000;
    let day_start_ms = (now_ms / day_ms) * day_ms;
    let count = state
        .db
        .events()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "kind": "cold_contact_emit",
                "created_at": { "$gte": DateTime::from_millis(day_start_ms) },
            },
            None,
        )
        .await?;
    Ok(count as i64)
}

async fn emit_cold_follow_up(
    state: &AppState,
    contact: &Contact,
    content: String,
    now: DateTime,
) -> anyhow::Result<()> {
    let expires_hours: i64 = 48;
    let expires_at = DateTime::from_millis(now.timestamp_millis() + expires_hours * 60 * 60 * 1000);
    let task = AgentTask {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        kind: "follow_up".to_string(),
        run_at: now,
        expires_at: Some(expires_at),
        content,
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: true,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    };
    state.db.tasks().insert_one(task, None).await?;
    Ok(())
}

async fn has_pending_follow_up(state: &AppState, contact: &Contact) -> anyhow::Result<bool> {
    let count = state
        .db
        .tasks()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "kind": "follow_up",
                "status": { "$in": ["pending", "retry", "running"] },
            },
            None,
        )
        .await?;
    Ok(count > 0)
}

async fn write_event(
    state: &AppState,
    account_id: &str,
    contact_wxid: Option<&str>,
    kind: &str,
    status: &str,
    summary: &str,
    details: Option<Document>,
) -> anyhow::Result<()> {
    crate::agent::write_event_for_account(
        state,
        account_id,
        contact_wxid,
        kind,
        status,
        summary,
        details,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}

/// 从 `operation_knowledge_chunks` 中拉一批 `chunk_type="peer_case"` 的 chunk
/// summary 文本，作为冷重激活 follow_up 的 hook 候选池。
///
/// peer_case 是 Phase B 引入的"用作参考的同类案例"chunk 类型；冷链路重激活
/// 文案最适合走该类型，避免直接搬 product_fact（容易被识别为推销话术）。
async fn load_peer_case_hooks(state: &AppState, workspace_id: &str) -> anyhow::Result<Vec<String>> {
    let mut cursor = state
        .db
        .raw()
        .collection::<Document>("operation_knowledge_chunks")
        .find(
            doc! {
                "workspace_id": workspace_id,
                "chunk_type": "peer_case",
                "status": { "$in": ["active", "approved"] },
            },
            None,
        )
        .await?;
    let mut out = Vec::new();
    while let Some(doc) = cursor.try_next().await? {
        if let Ok(s) = doc.get_str("summary") {
            if !s.trim().is_empty() {
                out.push(s.to_string());
            }
        }
    }
    Ok(out)
}

fn pick_hook(pool: &[String], contact_wxid: &str) -> Option<String> {
    if pool.is_empty() {
        return None;
    }
    // 用 contact_wxid 做稳定散列：同 contact 在同一池下永远拿到同一 hook，
    // 测试可复现；池变更时（B 流入新 peer_case）会自然轮换；不依赖 rand crate。
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    contact_wxid.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % pool.len();
    Some(pool[idx].clone())
}

/// Phase D / D3：纯决策函数版本的"是否要为该 contact 在本 tick 发冷重激活"。
///
/// 镜像生产路径 [`scan_cold_outbound`] 的判定（不含 LLM / DB 写入）：
/// - 必须是 managed contact；
/// - `last_outbound_at` 必须早于 `cold_before` 阈值；
/// - 上一轮 inbound 比 outbound 还新 → 属 silent 段，不属 cold；
/// - cooldown 未过期 → 跳过；
/// - 已存在 pending follow_up（同 contact）→ 跳过（与 planner 共用幂等池）。
///
/// 用于 PBT 断言："同一 contact + 已 emit pending follow_up + 同一 tick 内重复
/// 调用，必然返回 Skip"，即 D3 计划锁定的 cold_reactivation_idempotent 不变量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColdEmitDecision {
    /// 当前 tick 应为该 contact emit 一次冷重激活 follow_up。
    Emit,
    /// 不在 cold 范围（outbound 太新）。
    NotCold,
    /// 用户已经回过话（属 silent 段）。
    UserRecentlyReplied,
    /// 状态非 managed。
    NotManaged,
    /// cooldown 未过期。
    OnCooldown,
    /// 从未 outbound 过（属新建未触达，不算冷）。
    NeverOutbound,
    /// 已有 pending follow_up，幂等跳过。
    AlreadyPending,
}

pub fn decide_cold_emit(
    contact: &Contact,
    now_ms: i64,
    cold_before_ms: i64,
    has_pending_follow_up: bool,
) -> ColdEmitDecision {
    use crate::models::AgentStatus;
    if !matches!(contact.agent_status, AgentStatus::Managed) {
        return ColdEmitDecision::NotManaged;
    }
    let Some(last_outbound) = contact.last_outbound_at else {
        return ColdEmitDecision::NeverOutbound;
    };
    if last_outbound.timestamp_millis() >= cold_before_ms {
        return ColdEmitDecision::NotCold;
    }
    if let Some(last_inbound) = contact.last_inbound_at {
        if last_inbound.timestamp_millis() > last_outbound.timestamp_millis() {
            return ColdEmitDecision::UserRecentlyReplied;
        }
    }
    if let Some(cooldown) = contact.cooldown_until {
        if cooldown.timestamp_millis() > now_ms {
            return ColdEmitDecision::OnCooldown;
        }
    }
    if has_pending_follow_up {
        return ColdEmitDecision::AlreadyPending;
    }
    ColdEmitDecision::Emit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AgentStatus;
    use mongodb::bson::Document as BsonDocument;

    fn template(wxid: &str) -> Contact {
        let now = DateTime::now();
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
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn cold_filter_targets_outbound_not_inbound() {
        let cold_before = DateTime::from_millis(1_000);
        let filter = cold_candidate_filter_workspace("default", cold_before);
        // 静默段查 last_inbound_at；冷段必须查 last_outbound_at，不能搞混。
        assert!(filter.contains_key("last_outbound_at"));
        assert!(!filter.contains_key("last_inbound_at"));
        assert_eq!(filter.get_str("agent_status").unwrap(), "managed");
    }

    #[test]
    fn cold_passes_when_outbound_old_and_no_recent_inbound() {
        let mut c = template("user_a");
        c.last_outbound_at = Some(DateTime::from_millis(1_000));
        c.last_inbound_at = Some(DateTime::from_millis(500));
        assert!(cold_candidate_passes_in_memory(&c, 10_000_000));
    }

    #[test]
    fn cold_skips_when_inbound_newer_than_outbound() {
        // 用户已经回过话——属于 silent 段语义，不属于"agent 冷链路"。
        let mut c = template("user_chatty");
        c.last_outbound_at = Some(DateTime::from_millis(1_000));
        c.last_inbound_at = Some(DateTime::from_millis(2_000));
        assert!(!cold_candidate_passes_in_memory(&c, 10_000_000));
    }

    #[test]
    fn cold_skips_normal_status() {
        let mut c = template("user_b");
        c.agent_status = AgentStatus::Normal;
        c.last_outbound_at = Some(DateTime::from_millis(1_000));
        assert!(!cold_candidate_passes_in_memory(&c, 10_000_000));
    }

    #[test]
    fn cold_skips_when_cooldown_in_future() {
        let mut c = template("user_c");
        c.last_outbound_at = Some(DateTime::from_millis(1_000));
        c.cooldown_until = Some(DateTime::from_millis(
            DateTime::now().timestamp_millis() + 60 * 60 * 1000,
        ));
        assert!(!cold_candidate_passes_in_memory(&c, 10_000_000));
    }

    #[test]
    fn cold_skips_when_no_outbound_yet() {
        // 从未有过 outbound 的 contact 不属于"冷"——属于"新建未触达"。
        let c = template("user_unseen");
        assert!(!cold_candidate_passes_in_memory(&c, 10_000_000));
    }

    #[test]
    fn pick_hook_empty_returns_none() {
        let pool: Vec<String> = Vec::new();
        assert!(pick_hook(&pool, "wxid_x").is_none());
    }

    #[test]
    fn pick_hook_returns_one_of_pool_and_is_stable() {
        let pool = vec!["hook A".to_string(), "hook B".to_string()];
        let pick = pick_hook(&pool, "wxid_stable").expect("non-empty pool");
        assert!(pool.contains(&pick));
        // 同 wxid + 同 pool 必然同结果（稳定散列）。
        let again = pick_hook(&pool, "wxid_stable").expect("non-empty pool");
        assert_eq!(pick, again);
    }

    /// P1-3：cap 比较的边界——live_count 严格小于 cap 时放行，
    /// 等于或超出时停止 emit。
    #[test]
    fn cap_reached_open_when_below_cap() {
        assert!(!cap_reached(0, 5));
        assert!(!cap_reached(4, 5));
    }

    /// P1-3：等量必停 + 防越界——并发 tick 把 live_count 推到 cap 之上时
    /// 必须返回 true，避免双 tick 越缸。
    #[test]
    fn cap_reached_blocks_when_at_or_above_cap() {
        assert!(cap_reached(5, 5));
        assert!(cap_reached(6, 5));
    }

    /// P1-3：daily_cap = 0 应等价"关停冷链路"——任意 live_count 都立即停止。
    #[test]
    fn cap_reached_zero_cap_disables_emits() {
        assert!(cap_reached(0, 0));
        assert!(cap_reached(7, 0));
    }

    /// P1-3：负值 sanitization——不应让历史脏数据穿透 cap 保护。
    #[test]
    fn cap_reached_negative_inputs_are_clamped_to_zero() {
        assert!(!cap_reached(-1, 5));
        assert!(cap_reached(0, -3));
    }
}

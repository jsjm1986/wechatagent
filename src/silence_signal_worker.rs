//! 自学习采集管道（第一阶段）/ S2 + S6：沉默删失探测 worker。
//!
//! 与 [`crate::cold_contact_worker`] 互补但语义不同：
//! - cold worker：选久未出站的 contact，**发**一条重激活 follow_up；
//! - 本 worker：选"最后一条 outbound 至今无回"的 contact，**只落一条
//!   `censored=true` 的沉默信号**，绝不发任何消息。
//!
//! Iron Law ②：沉默 = 删失（censored），**不是负例**。本 worker 落的信号永远
//! 带 `censored=true`，下游任何学习公式都不得把它当负反馈扣分。本阶段甚至
//! 没有任何下游消费它——只是把"未来可学的删失形状"幂等地铺好。
//!
//! 默认关停：`SILENCE_SIGNAL_WORKER_ENABLED=false`。开启后周期 tick：
//! 1) workspace 级扫描 `last_outbound_at < now - threshold` 的 managed contact；
//! 2) 仅当该 contact 在那条 outbound 之后再无 inbound（[`decide_silence_signal`]
//!    判 true）才落信号；
//! 3) `dedupe_key="silence:{wxid}:{last_outbound_at_ms}"` + partial unique 索引
//!    保证同一条 outbound 只产一次沉默事件（重复 tick 幂等）；
//! 4) 单 workspace 单 tick `silence_signal_daily_cap` 上限，防首跑信号风暴。

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use tokio::time::sleep;

use crate::behavior_signals;
use crate::models::Contact;
use crate::routes::AppState;

/// 沉默探测 worker 主循环。flag=false 或 interval==0 时立刻 return（关停态）。
pub async fn run_silence_signal_worker(state: AppState) {
    if !state.config.silence_signal_worker_enabled
        || state.config.silence_signal_interval_seconds == 0
    {
        tracing::info!("silence_signal_worker disabled by config; loop will not start");
        return;
    }
    tracing::info!(
        threshold_seconds = state.config.silence_threshold_seconds,
        daily_cap = state.config.silence_signal_daily_cap,
        "silence_signal_worker loop started"
    );
    loop {
        if let Err(error) = tick(&state).await {
            tracing::error!(error = %error, "silence_signal_worker tick failed");
        }
        sleep(Duration::from_secs(
            state.config.silence_signal_interval_seconds.max(60),
        ))
        .await;
    }
}

/// 单 tick 入口（lib 测试 + 主循环复用）。
pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    scan_silence(state).await
}

async fn scan_silence(state: &AppState) -> anyhow::Result<()> {
    let workspace_id = state.config.default_workspace_id.clone();
    let now = DateTime::now();
    let now_ms = now.timestamp_millis();
    let threshold_ms = state
        .config
        .silence_threshold_seconds
        .saturating_mul(1000);
    let silent_before = DateTime::from_millis(now_ms - threshold_ms);

    let filter = silence_candidate_filter(&workspace_id, silent_before);
    let mut cursor = state.db.contacts().find(filter, None).await?;

    let daily_cap = state.config.silence_signal_daily_cap;
    let mut scanned = 0i64;
    let mut emitted = 0i64;

    while let Some(contact) = cursor.try_next().await? {
        scanned += 1;
        if !decide_silence_signal(&contact, now_ms, threshold_ms) {
            continue;
        }
        if cap_reached(emitted, daily_cap) {
            break;
        }
        let Some(last_outbound) = contact.last_outbound_at else {
            continue;
        };
        let signal = behavior_signals::build_silence(
            &contact.workspace_id,
            &contact.wxid,
            last_outbound,
            now,
        );
        // 幂等落库：dedupe_key 撞索引 → persist_signal 返回 Ok(false)，不计 emit。
        let workspace_id = contact.workspace_id.clone();
        let result = behavior_signals::persist_signal(state, signal).await;
        behavior_signals::record_signal_metric(state, &workspace_id, &result).await;
        match result {
            Ok(true) => emitted += 1,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    wxid = %contact.wxid,
                    "silence signal persist failed (best-effort, ignored)"
                );
            }
        }
    }

    write_event(
        state,
        &state.config.default_account_id,
        "silence_signal_tick",
        "ok",
        &format!("silence_signal_worker tick: scanned {scanned}, emitted {emitted}"),
        Some(doc! {
            "scanned": scanned,
            "emitted": emitted,
            "dailyCap": daily_cap,
            "thresholdSeconds": state.config.silence_threshold_seconds,
        }),
    )
    .await?;
    Ok(())
}

/// workspace 级沉默候选过滤：managed + `last_outbound_at` 早于阈值。
/// 进一步的"那条 outbound 之后无 inbound"判定在 [`decide_silence_signal`] 里
/// 用内存字段做（query 只做粗筛，避免复杂 `$expr` 跨字段比较）。
pub(crate) fn silence_candidate_filter(workspace_id: &str, silent_before: DateTime) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "agent_status": "managed",
        "last_outbound_at": { "$lt": silent_before },
    }
}

/// 纯决策：本 tick 是否应为该 contact 落一条沉默删失信号。
///
/// 条件（全部满足）：
/// - managed；
/// - 有 `last_outbound_at` 且其距 now 已超过 `threshold_ms`；
/// - 那条 outbound 之后再无 inbound（`last_inbound_at` 缺失，或 ≤ 出站时间）。
///
/// 注意：用户已经回过话（inbound 比 outbound 新）→ 不是沉默，返回 false。
/// 这正是 silence 与 cold 的语义分界。
pub fn decide_silence_signal(contact: &Contact, now_ms: i64, threshold_ms: i64) -> bool {
    use crate::models::AgentStatus;
    if !matches!(contact.agent_status, AgentStatus::Managed) {
        return false;
    }
    let Some(last_outbound) = contact.last_outbound_at else {
        return false;
    };
    let out_ms = last_outbound.timestamp_millis();
    if now_ms - out_ms < threshold_ms {
        return false;
    }
    if let Some(last_inbound) = contact.last_inbound_at {
        if last_inbound.timestamp_millis() > out_ms {
            // 用户在出站后回过话 → 不沉默。
            return false;
        }
    }
    true
}

/// 单 tick emit cap：`emitted >= cap` 时停止。`cap <= 0` 关停（任意 emit 都停）。
pub(crate) fn cap_reached(emitted: i64, daily_cap: i64) -> bool {
    let cap = daily_cap.max(0);
    emitted.max(0) >= cap
}

async fn write_event(
    state: &AppState,
    account_id: &str,
    kind: &str,
    status: &str,
    summary: &str,
    details: Option<Document>,
) -> anyhow::Result<()> {
    crate::agent::write_event_for_account(state, account_id, None, kind, status, summary, details)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))
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
            deal_events: Vec::new(),
            locale: None,
            created_at: now,
            updated_at: now,
        }
    }

    const THRESHOLD: i64 = 24 * 60 * 60 * 1000; // 24h

    #[test]
    fn filter_targets_outbound_and_managed() {
        let f = silence_candidate_filter("default", DateTime::from_millis(1_000));
        assert!(f.contains_key("last_outbound_at"));
        assert_eq!(f.get_str("agent_status").unwrap(), "managed");
        // 沉默查 outbound，不应混进 inbound 粗筛。
        assert!(!f.contains_key("last_inbound_at"));
    }

    #[test]
    fn silence_when_outbound_old_and_no_reply() {
        let mut c = template("u1");
        c.last_outbound_at = Some(DateTime::from_millis(0));
        // now 远超阈值，且无 inbound。
        assert!(decide_silence_signal(&c, THRESHOLD + 1, THRESHOLD));
    }

    #[test]
    fn no_silence_when_user_replied_after_outbound() {
        // 用户在出站后回过话 → 不是沉默（这是 silence 与 cold 的分界）。
        let mut c = template("u2");
        c.last_outbound_at = Some(DateTime::from_millis(0));
        c.last_inbound_at = Some(DateTime::from_millis(10));
        assert!(!decide_silence_signal(&c, THRESHOLD + 1, THRESHOLD));
    }

    #[test]
    fn no_silence_before_threshold() {
        let mut c = template("u3");
        c.last_outbound_at = Some(DateTime::from_millis(0));
        // 还没到阈值。
        assert!(!decide_silence_signal(&c, THRESHOLD - 1, THRESHOLD));
    }

    #[test]
    fn no_silence_when_no_outbound() {
        let c = template("u4");
        assert!(!decide_silence_signal(&c, THRESHOLD + 1, THRESHOLD));
    }

    #[test]
    fn no_silence_when_not_managed() {
        let mut c = template("u5");
        c.agent_status = AgentStatus::Normal;
        c.last_outbound_at = Some(DateTime::from_millis(0));
        assert!(!decide_silence_signal(&c, THRESHOLD + 1, THRESHOLD));
    }

    #[test]
    fn inbound_equal_to_outbound_still_silent() {
        // inbound 不晚于 outbound（同刻或更早）→ 仍属沉默（用户没在出站后回话）。
        let mut c = template("u6");
        c.last_outbound_at = Some(DateTime::from_millis(100));
        c.last_inbound_at = Some(DateTime::from_millis(100));
        assert!(decide_silence_signal(&c, 100 + THRESHOLD + 1, THRESHOLD));
    }

    #[test]
    fn cap_blocks_at_or_above() {
        assert!(!cap_reached(0, 5));
        assert!(!cap_reached(4, 5));
        assert!(cap_reached(5, 5));
        assert!(cap_reached(6, 5));
    }

    #[test]
    fn cap_zero_disables() {
        assert!(cap_reached(0, 0));
        assert!(cap_reached(3, -1));
    }
}

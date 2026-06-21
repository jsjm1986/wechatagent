//! 主动发送台账：转化判定纯函数（responded 窗口 / stage_advanced 推进）、
//! 聚合率计算。写入 / 回扫的 DB 逻辑在 gateway/tasks 调用侧，这里只放可单测的纯逻辑。

use crate::models::AgentSendLedger;
use crate::routes::AppState;
use mongodb::bson::{doc, oid::ObjectId, DateTime};

/// 构造一条待写台账。转化字段一律留空（回扫填）。
pub(crate) fn build_ledger_entry(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    send_kind: &str,
    target_id: &str,
    target_title: &str,
    run_id: &str,
    customer_stage_at_send: Option<String>,
    now: DateTime,
) -> AgentSendLedger {
    AgentSendLedger {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: contact_wxid.to_string(),
        send_kind: send_kind.to_string(),
        target_id: target_id.to_string(),
        target_title: target_title.to_string(),
        run_id: run_id.to_string(),
        trigger_reason: None,
        customer_stage_at_send,
        sent_at: now,
        responded: None,
        response_window_hours: None,
        stage_advanced: None,
        outcome_evaluated_at: None,
    }
}

/// fail-soft 写台账：失败只 log，绝不返 Err（既成事实纪律——发送已成，
/// 台账缺一条不该影响发送结果，更不能让上游误判为失败而重发）。
pub(crate) async fn record_send(state: &AppState, entry: &AgentSendLedger) {
    if let Err(err) = state.db.agent_send_ledger().insert_one(entry, None).await {
        tracing::error!(
            workspace_id = %entry.workspace_id,
            contact_wxid = %entry.contact_wxid,
            send_kind = %entry.send_kind,
            target_id = %entry.target_id,
            error = %err,
            "send succeeded but persisting agent_send_ledger failed; metrics will miss this send",
        );
    }
}

/// 回查发送物标题做冗余快照。查不到/解析失败返空串（不阻断写台账）。
pub(crate) async fn lookup_target_title(
    state: &AppState,
    workspace_id: &str,
    send_kind: &str,
    target_id: &str,
) -> String {
    let Ok(oid) = ObjectId::parse_str(target_id) else {
        return String::new();
    };
    let filter = doc! { "_id": oid, "workspace_id": workspace_id };
    match send_kind {
        "namecard" => state
            .db
            .referral_cards()
            .find_one(filter, None)
            .await
            .ok()
            .flatten()
            .map(|c| c.display_name)
            .unwrap_or_default(),
        _ => state
            .db
            .content_assets()
            .find_one(filter, None)
            .await
            .ok()
            .flatten()
            .map(|a| a.title)
            .unwrap_or_default(),
    }
}

/// 任一入站时间戳落在 (sent_at, sent_at + window_hours] 内 → 已响应。
/// 早于/等于发送时刻的入站（历史消息）不算。
pub(crate) fn responded_within_window(sent_at_ms: i64, window_hours: i32, inbound_ms: &[i64]) -> bool {
    let window_end = sent_at_ms + (window_hours.max(0) as i64) * 3_600_000;
    inbound_ms
        .iter()
        .any(|&ms| ms > sent_at_ms && ms <= window_end)
}

/// 当前阶段在 ordered_stages 里严格靠后于发送时阶段 → 推进。
/// 任一阶段缺失或不在有序表 → 保守判 false（不算推进）。
pub(crate) fn stage_advanced(
    stage_at_send: Option<&str>,
    current_stage: Option<&str>,
    ordered_stages: &[String],
) -> bool {
    let (Some(from), Some(to)) = (stage_at_send, current_stage) else {
        return false;
    };
    let idx = |s: &str| ordered_stages.iter().position(|x| x == s);
    match (idx(from), idx(to)) {
        (Some(i), Some(j)) => j > i,
        _ => false,
    }
}

/// 响应率：total=0 返 0.0，否则 responded/total 保留 4 位小数。
pub(crate) fn response_rate(total: u64, responded: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let raw = responded as f64 / total as f64;
    (raw * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 3_600_000;

    #[test]
    fn responded_true_when_inbound_in_window() {
        let sent = 1_000_000_000_000;
        // 窗口 24h，入站在 sent 后 2h → 命中
        assert!(responded_within_window(sent, 24, &[sent + 2 * HOUR_MS]));
    }

    #[test]
    fn responded_false_when_inbound_after_window() {
        let sent = 1_000_000_000_000;
        // 入站在 sent 后 25h，窗口 24h → 不命中
        assert!(!responded_within_window(sent, 24, &[sent + 25 * HOUR_MS]));
    }

    #[test]
    fn responded_false_when_inbound_before_send() {
        let sent = 1_000_000_000_000;
        // 入站早于发送（历史消息）→ 不算响应
        assert!(!responded_within_window(sent, 24, &[sent - HOUR_MS]));
    }

    #[test]
    fn responded_false_when_no_inbound() {
        assert!(!responded_within_window(1_000_000_000_000, 24, &[]));
    }

    #[test]
    fn stage_advanced_true_when_moves_forward() {
        let order = vec!["new_contact".to_string(), "意向".to_string(), "待成交".to_string()];
        assert!(stage_advanced(Some("意向"), Some("待成交"), &order));
    }

    #[test]
    fn stage_advanced_false_when_same_or_backward() {
        let order = vec!["new_contact".to_string(), "意向".to_string(), "待成交".to_string()];
        assert!(!stage_advanced(Some("意向"), Some("意向"), &order)); // 持平
        assert!(!stage_advanced(Some("待成交"), Some("意向"), &order)); // 回退
    }

    #[test]
    fn stage_advanced_false_when_unknown_or_missing() {
        let order = vec!["new_contact".to_string(), "意向".to_string()];
        // 任一阶段不在有序表 → 保守判 false（不算推进）
        assert!(!stage_advanced(Some("意向"), Some("不存在"), &order));
        assert!(!stage_advanced(None, Some("意向"), &order));
    }

    #[test]
    fn response_rate_zero_total_is_zero() {
        assert_eq!(response_rate(0, 0), 0.0);
    }

    #[test]
    fn response_rate_basic() {
        assert_eq!(response_rate(4, 1), 0.25);
    }

    #[test]
    fn build_ledger_entry_sets_kind_and_leaves_outcome_none() {
        use mongodb::bson::DateTime;
        let row = build_ledger_entry(
            "ws", "acct", "wx", "media", "asset1", "报价单", "run1",
            Some("意向".to_string()), DateTime::now(),
        );
        assert_eq!(row.send_kind, "media");
        assert_eq!(row.target_id, "asset1");
        assert_eq!(row.target_title, "报价单");
        assert_eq!(row.customer_stage_at_send.as_deref(), Some("意向"));
        // 转化字段发送时必须留空（回扫才填）
        assert!(row.responded.is_none());
        assert!(row.stage_advanced.is_none());
        assert!(row.outcome_evaluated_at.is_none());
    }
}

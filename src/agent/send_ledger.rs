//! 主动发送台账：转化判定纯函数（responded 窗口 / stage_advanced 推进）、
//! 聚合率计算。写入 / 回扫的 DB 逻辑在 gateway/tasks 调用侧，这里只放可单测的纯逻辑。

use crate::error::AppResult;
use crate::models::AgentSendLedger;
use crate::routes::AppState;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

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

/// 从状态机 states 数组按出现顺序抽 key（作为粗略"阶段序"，供 stage_advanced 判定）。
pub(crate) fn ordered_stages_from_machine(state_machine: &Document) -> Vec<String> {
    state_machine
        .get_array("states")
        .map(|states| {
            states
                .iter()
                .filter_map(|s| s.as_document())
                .filter_map(|d| d.get_str("key").ok())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 回扫一批 outcome_evaluated_at 缺失且已过响应窗口的台账条目，回填转化字段。
/// 纯读 + 回写自己表，不调 LLM、不发消息（无副作用红线）。返回处理条数。
pub(crate) async fn scan_send_ledger_outcomes(state: &AppState) -> AppResult<usize> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;

    let default_window_hours: i32 = 24;
    let now = DateTime::now();
    let now_ms = now.timestamp_millis();

    // 待评估：outcome_evaluated_at 缺失。窗口是否已过在内存里按每条 sent_at 判断
    // （避免对 response_window_hours 可空字段做复杂 mongo 时间运算）。
    let filter = doc! { "outcome_evaluated_at": { "$exists": false } };
    let mut cursor = state
        .db
        .agent_send_ledger()
        .find(
            filter,
            FindOptions::builder()
                .limit(200) // 一次限量，防积压时单 tick 过重
                .sort(doc! { "sent_at": 1 })
                .build(),
        )
        .await?;

    let mut processed = 0usize;
    while let Some(row) = cursor.try_next().await? {
        let Some(row_id) = row.id else { continue };
        let window_hours = row.response_window_hours.unwrap_or(default_window_hours);
        let sent_ms = row.sent_at.timestamp_millis();
        let window_end_ms = sent_ms + (window_hours.max(0) as i64) * 3_600_000;
        // 窗口未过 → 跳过本轮（下个 tick 再看）。
        if now_ms < window_end_ms {
            continue;
        }

        // responded：查该 contact 在 (sent, sent+窗口] 内的入站消息时间戳。
        let inbound_filter = doc! {
            "workspace_id": &row.workspace_id,
            "contact_wxid": &row.contact_wxid,
            "direction": "inbound",
            "created_at": {
                "$gt": row.sent_at,
                "$lte": DateTime::from_millis(window_end_ms),
            },
        };
        let inbound_count = state
            .db
            .messages()
            .count_documents(inbound_filter, None)
            .await
            .unwrap_or(0);
        let responded = inbound_count > 0;

        // stage_advanced：取当前 contact.customer_stage vs 发送时快照，按状态机序判断。
        let current_stage = state
            .db
            .contacts()
            .find_one(
                doc! { "workspace_id": &row.workspace_id, "wxid": &row.contact_wxid },
                None,
            )
            .await
            .ok()
            .flatten()
            .and_then(|c| {
                c.domain_attributes
                    .as_ref()
                    .and_then(|d| d.get_str("customer_stage").ok().map(ToString::to_string))
            });
        let ordered = load_user_ops_stage_order(state, &row.workspace_id).await;
        let advanced = stage_advanced(
            row.customer_stage_at_send.as_deref(),
            current_stage.as_deref(),
            &ordered,
        );

        let _ = state
            .db
            .agent_send_ledger()
            .update_one(
                doc! { "_id": row_id },
                doc! { "$set": {
                    "responded": responded,
                    "response_window_hours": window_hours,
                    "stage_advanced": advanced,
                    "outcome_evaluated_at": now,
                }},
                None,
            )
            .await;
        processed += 1;
    }
    Ok(processed)
}

/// 取 user_operations 域当前状态机的阶段序。查不到返空（stage_advanced 保守判 false）。
async fn load_user_ops_stage_order(state: &AppState, workspace_id: &str) -> Vec<String> {
    state
        .db
        .operation_domain_configs()
        .find_one(
            doc! { "workspace_id": workspace_id, "domain": "user_operations", "current_version": true },
            None,
        )
        .await
        .ok()
        .flatten()
        .map(|c| ordered_stages_from_machine(&c.state_machine))
        .unwrap_or_default()
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
    fn ordered_stages_extracts_keys_in_order() {
        use mongodb::bson::doc;
        let machine = doc! {
            "states": [
                { "key": "new_contact", "initial": true },
                { "key": "意向" },
                { "key": "待成交" },
            ]
        };
        let order = ordered_stages_from_machine(&machine);
        assert_eq!(order, vec!["new_contact", "意向", "待成交"]);
    }

    #[test]
    fn ordered_stages_empty_when_no_states() {
        use mongodb::bson::doc;
        assert!(ordered_stages_from_machine(&doc! {}).is_empty());
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

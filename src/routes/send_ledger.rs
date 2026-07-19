//! 主动发送台账只读 API：单客户发送历史 / 素材·名片维度聚合 / 总览。
//! 全部带 `(workspace_id, account_id)` scope（防跨租户/跨业务号串账）。
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{shared::validate_account, AppState};
use crate::{auth::AuthenticatedAdmin, error::AppResult};

/// 读聚合 $sum 计数：兼容 mongo 返回 i32 或 i64（$sum:1 的运行时类型随版本/数据量变）。
/// 否则 i64 时 get_i32 返 Err → unwrap_or(0) 会静默把计数清零，整页统计显示 0。
fn agg_count(d: &Document, key: &str) -> u64 {
    d.get_i64(key)
        .map(|v| v.max(0) as u64)
        .or_else(|_| d.get_i32(key).map(|v| v.max(0) as u64))
        .unwrap_or(0)
}

/// 聚合 `$match`：固定 workspace/account，可选 kind。
pub(super) fn build_stats_match(
    workspace_id: &str,
    account_id: &str,
    kind: Option<&str>,
) -> Document {
    let mut m = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
    };
    if let Some(k) = kind {
        m.insert("send_kind", k);
    }
    m
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatsQuery {
    account_id: String,
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LedgerScopeQuery {
    account_id: String,
}

/// 单客户发送历史（按 sent_at 倒序）。
pub(super) async fn contact_send_history(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(wxid): Path<String>,
    Query(query): Query<LedgerScopeQuery>,
) -> AppResult<Json<Value>> {
    use mongodb::options::FindOptions;
    validate_account(&state, &admin.current_workspace, &query.account_id).await?;
    let mut cursor = state
        .db
        .agent_send_ledger()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &query.account_id,
                "contact_wxid": &wxid,
            },
            FindOptions::builder()
                .sort(doc! { "sent_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(r) = cursor.try_next().await? {
        items.push(json!({
            "sendKind": r.send_kind,
            "targetId": r.target_id,
            "targetTitle": r.target_title,
            "sentAt": crate::models::dt_to_string(r.sent_at),
            "triggerReason": r.trigger_reason,
            "responded": r.responded,
            "stageAdvanced": r.stage_advanced,
        }));
    }
    Ok(Json(json!({ "items": items })))
}

/// 素材/名片维度聚合排行榜。
pub(super) async fn send_ledger_stats(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<StatsQuery>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &admin.current_workspace, &q.account_id).await?;
    let match_doc = build_stats_match(&admin.current_workspace, &q.account_id, q.kind.as_deref());
    let pipeline = vec![
        doc! { "$match": match_doc },
        doc! { "$group": {
            "_id": "$target_id",
            "targetTitle": { "$last": "$target_title" },
            "sentCount": { "$sum": 1 },
            "contacts": { "$addToSet": "$contact_wxid" },
            "respondedCount": { "$sum": { "$cond": [ { "$eq": ["$responded", true] }, 1, 0 ] } },
            "stageAdvancedCount": { "$sum": { "$cond": [ { "$eq": ["$stage_advanced", true] }, 1, 0 ] } },
            "evaluatedCount": { "$sum": { "$cond": [ { "$ifNull": ["$outcome_evaluated_at", false] }, 1, 0 ] } },
        }},
        doc! { "$sort": { "sentCount": -1 } },
        doc! { "$limit": 100 },
    ];
    let mut cursor = state
        .db
        .agent_send_ledger()
        .aggregate(pipeline, None)
        .await?;
    let mut items = Vec::new();
    while let Some(d) = cursor.try_next().await? {
        let sent = agg_count(&d, "sentCount");
        let responded = agg_count(&d, "respondedCount");
        let advanced = agg_count(&d, "stageAdvancedCount");
        let evaluated = agg_count(&d, "evaluatedCount");
        let contact_count = d.get_array("contacts").map(|a| a.len()).unwrap_or(0);
        items.push(json!({
            "targetId": d.get_str("_id").unwrap_or_default(),
            "targetTitle": d.get_str("targetTitle").unwrap_or_default(),
            "sentCount": sent,
            "contactCount": contact_count,
            // 率以"已评估条目"为分母（未过窗口的不计入），避免新发未评估拉低率
            "responseRate": crate::agent::send_ledger::response_rate(evaluated, responded),
            "stageAdvanceRate": crate::agent::send_ledger::response_rate(evaluated, advanced),
        }));
    }
    Ok(Json(json!({ "items": items })))
}

/// 总览：总发送数 + 整体响应率/推进率。
pub(super) async fn send_ledger_overview(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<LedgerScopeQuery>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &admin.current_workspace, &query.account_id).await?;
    let pipeline = vec![
        doc! { "$match": {
            "workspace_id": &admin.current_workspace,
            "account_id": &query.account_id,
        } },
        doc! { "$group": {
            "_id": null,
            "total": { "$sum": 1 },
            "respondedCount": { "$sum": { "$cond": [ { "$eq": ["$responded", true] }, 1, 0 ] } },
            "stageAdvancedCount": { "$sum": { "$cond": [ { "$eq": ["$stage_advanced", true] }, 1, 0 ] } },
            "evaluatedCount": { "$sum": { "$cond": [ { "$ifNull": ["$outcome_evaluated_at", false] }, 1, 0 ] } },
        }},
    ];
    let mut cursor = state
        .db
        .agent_send_ledger()
        .aggregate(pipeline, None)
        .await?;
    let (mut total, mut responded, mut advanced, mut evaluated) = (0u64, 0u64, 0u64, 0u64);
    if let Some(d) = cursor.try_next().await? {
        total = agg_count(&d, "total");
        responded = agg_count(&d, "respondedCount");
        advanced = agg_count(&d, "stageAdvancedCount");
        evaluated = agg_count(&d, "evaluatedCount");
    }
    Ok(Json(json!({
        "totalSends": total,
        "responseRate": crate::agent::send_ledger::response_rate(evaluated, responded),
        "stageAdvanceRate": crate::agent::send_ledger::response_rate(evaluated, advanced),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_match_pins_workspace_and_kind() {
        let m = build_stats_match("ws1", "account-a", Some("media"));
        assert_eq!(m.get_str("workspace_id").ok(), Some("ws1"));
        assert_eq!(m.get_str("account_id").ok(), Some("account-a"));
        assert_eq!(m.get_str("send_kind").ok(), Some("media"));
    }

    #[test]
    fn stats_match_without_kind_omits_kind() {
        let m = build_stats_match("ws1", "account-a", None);
        assert_eq!(m.get_str("workspace_id").ok(), Some("ws1"));
        assert_eq!(m.get_str("account_id").ok(), Some("account-a"));
        assert!(!m.contains_key("send_kind"));
    }

    #[test]
    fn agg_count_reads_both_i32_and_i64() {
        let d = doc! {
            "asI32": 7i32,
            "asI64": 42i64,
            "negI64": -3i64,
            "negI32": -5i32,
        };
        // i32 路径
        assert_eq!(agg_count(&d, "asI32"), 7);
        // i64 路径（mongo $sum:1 在大数据量/新版本可能返 i64）
        assert_eq!(agg_count(&d, "asI64"), 42);
        // 负值钳制为 0（保持原 .max(0) 语义）
        assert_eq!(agg_count(&d, "negI64"), 0);
        assert_eq!(agg_count(&d, "negI32"), 0);
        // 缺失字段返 0
        assert_eq!(agg_count(&d, "missing"), 0);
    }
}

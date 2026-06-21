//! 主动发送台账只读 API：单客户发送历史 / 素材·名片维度聚合 / 总览。
//! 全部带 workspace_id scope（防跨租户 IDOR）。
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use serde::Deserialize;
use serde_json::{json, Value};

use super::AppState;
use crate::{auth::AuthenticatedAdmin, error::AppResult};

/// 聚合 $match：固定 workspace，可选 kind。
pub(super) fn build_stats_match(workspace_id: &str, kind: Option<&str>) -> Document {
    let mut m = doc! { "workspace_id": workspace_id };
    if let Some(k) = kind {
        m.insert("send_kind", k);
    }
    m
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatsQuery {
    kind: Option<String>,
}

/// 单客户发送历史（按 sent_at 倒序）。
pub(super) async fn contact_send_history(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(wxid): Path<String>,
) -> AppResult<Json<Value>> {
    use mongodb::options::FindOptions;
    let mut cursor = state
        .db
        .agent_send_ledger()
        .find(
            doc! { "workspace_id": &admin.current_workspace, "contact_wxid": &wxid },
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
    let match_doc = build_stats_match(&admin.current_workspace, q.kind.as_deref());
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
    let mut cursor = state.db.agent_send_ledger().aggregate(pipeline, None).await?;
    let mut items = Vec::new();
    while let Some(d) = cursor.try_next().await? {
        let sent = d.get_i32("sentCount").unwrap_or(0).max(0) as u64;
        let responded = d.get_i32("respondedCount").unwrap_or(0).max(0) as u64;
        let advanced = d.get_i32("stageAdvancedCount").unwrap_or(0).max(0) as u64;
        let evaluated = d.get_i32("evaluatedCount").unwrap_or(0).max(0) as u64;
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
) -> AppResult<Json<Value>> {
    let pipeline = vec![
        doc! { "$match": { "workspace_id": &admin.current_workspace } },
        doc! { "$group": {
            "_id": null,
            "total": { "$sum": 1 },
            "respondedCount": { "$sum": { "$cond": [ { "$eq": ["$responded", true] }, 1, 0 ] } },
            "stageAdvancedCount": { "$sum": { "$cond": [ { "$eq": ["$stage_advanced", true] }, 1, 0 ] } },
            "evaluatedCount": { "$sum": { "$cond": [ { "$ifNull": ["$outcome_evaluated_at", false] }, 1, 0 ] } },
        }},
    ];
    let mut cursor = state.db.agent_send_ledger().aggregate(pipeline, None).await?;
    let (mut total, mut responded, mut advanced, mut evaluated) = (0u64, 0u64, 0u64, 0u64);
    if let Some(d) = cursor.try_next().await? {
        total = d.get_i32("total").unwrap_or(0).max(0) as u64;
        responded = d.get_i32("respondedCount").unwrap_or(0).max(0) as u64;
        advanced = d.get_i32("stageAdvancedCount").unwrap_or(0).max(0) as u64;
        evaluated = d.get_i32("evaluatedCount").unwrap_or(0).max(0) as u64;
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
        let m = build_stats_match("ws1", Some("media"));
        assert_eq!(m.get_str("workspace_id").ok(), Some("ws1"));
        assert_eq!(m.get_str("send_kind").ok(), Some("media"));
    }

    #[test]
    fn stats_match_without_kind_omits_kind() {
        let m = build_stats_match("ws1", None);
        assert_eq!(m.get_str("workspace_id").ok(), Some("ws1"));
        assert!(!m.contains_key("send_kind"));
    }
}

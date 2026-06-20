//! ask-human 只读聚合器：扇出查各待审来源，归一成统一 InboxItem。
//! 每 source 独立查询，失败标 error 不整体崩。零侵入（不动任何写路径）。

use axum::extract::{Query, State};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::AppState;
use crate::auth::AuthenticatedAdmin;
use crate::error::AppResult;
use mongodb::bson::{doc, DateTime, Document};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub source: String,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub severity: String,
    pub created_at: Option<DateTime>,
    pub age_hours: f64,
    pub action_kind: String, // "inline" | "rich"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rich_component: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rich_params: Option<Document>,
}

fn age_hours_of(created: Option<DateTime>, now_ms: i64) -> f64 {
    created
        .map(|c| (now_ms - c.timestamp_millis()) as f64 / (3600.0 * 1000.0))
        .unwrap_or(0.0)
}

/// 请示通道 pending → InboxItem（inline）。
async fn collect_escalations(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    let items =
        crate::agent::escalation::list_escalations_by_workspace(state, ws, "pending").await?;
    Ok(items
        .into_iter()
        .map(|e| InboxItem {
            source: "principal_escalation".into(),
            id: e.short_code.clone(),
            title: format!("请示 #{}", e.short_code),
            summary: e.reason.clone(),
            severity: "high".into(),
            created_at: Some(e.created_at),
            age_hours: age_hours_of(Some(e.created_at), now_ms),
            action_kind: "inline".into(),
            rich_component: None,
            rich_params: None,
        })
        .collect())
}

/// 知识切片 needs_review → InboxItem（rich：在统一频道内挂知识核验组件）。
async fn collect_knowledge_review(
    state: &AppState,
    ws: &str,
    _now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! { "workspace_id": ws, "integrity_status": "needs_review" },
            mongodb::options::FindOptions::builder().limit(100).build(),
        )
        .await?;
    let chunks: Vec<crate::models::OperationKnowledgeChunk> = cursor.try_collect().await?;
    Ok(chunks
        .into_iter()
        .map(|c| {
            let id = c.id.map(|o| o.to_hex()).unwrap_or_default();
            InboxItem {
                source: "knowledge_review".into(),
                id: id.clone(),
                title: c.title.clone(),
                summary: c.body.clone().unwrap_or_default().chars().take(80).collect(),
                severity: "medium".into(),
                created_at: None,
                age_hours: 0.0,
                action_kind: "rich".into(),
                rich_component: Some("knowledgeReview".into()),
                rich_params: Some(doc! { "chunkId": id }),
            }
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct InboxQuery {
    #[serde(default)]
    pub source: Option<String>,
}

/// GET /api/admin/ask-human/inbox?source=<filter>
pub async fn ask_human_inbox(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<InboxQuery>,
) -> AppResult<Json<Value>> {
    let ws = &admin.current_workspace;
    let now_ms = DateTime::now().timestamp_millis();
    let mut items: Vec<InboxItem> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();

    // 每 source 独立降级：Err 不整体崩，记进 errors 数组。
    macro_rules! collect_source {
        ($name:expr, $fut:expr) => {
            if q.source.as_deref().map(|s| s == $name).unwrap_or(true) {
                match $fut.await {
                    Ok(mut v) => items.append(&mut v),
                    Err(e) => errors.push(json!({ "source": $name, "error": e.to_string() })),
                }
            }
        };
    }

    collect_source!("principal_escalation", collect_escalations(&state, ws, now_ms));
    collect_source!("knowledge_review", collect_knowledge_review(&state, ws, now_ms));
    // Task 12 在此追加其余 source。

    Ok(Json(json!({ "items": items, "errors": errors })))
}

/// GET /api/admin/ask-human/summary —— 各 source pending 计数。
pub async fn ask_human_summary(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let ws = &admin.current_workspace;
    let escalations = state
        .db
        .agent_principal_escalations()
        .count_documents(doc! { "workspace_id": ws, "status": "pending" }, None)
        .await
        .unwrap_or(0);
    let knowledge = state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": ws, "integrity_status": "needs_review" }, None)
        .await
        .unwrap_or(0);
    // Task 12 追加其余 source 计数。
    Ok(Json(json!({
        "principalEscalation": escalations,
        "knowledgeReview": knowledge,
    })))
}

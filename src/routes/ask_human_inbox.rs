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

/// 标签候选 pending → inline。隔离键是 scope（account_id 或 "global"），无 workspace_id；
/// 仅暴露 scope="global" 的共享候选，避免泄漏账户私有候选（IDOR 安全）。
async fn collect_taxonomy_candidates(
    state: &AppState,
    _ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .collection_taxonomy_candidates()
        .find(
            doc! { "scope": "global", "status": "pending" },
            mongodb::options::FindOptions::builder().limit(100).build(),
        )
        .await?;
    let rows: Vec<crate::models::TaxonomyCandidate> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|c| {
            let id = c.id.map(|o| o.to_hex()).unwrap_or_default();
            InboxItem {
                source: "taxonomy_candidate".into(),
                id,
                title: format!("标签候选：{}", c.kind),
                summary: c.raw_value.clone(),
                severity: "low".into(),
                created_at: Some(c.last_seen_at),
                age_hours: age_hours_of(Some(c.last_seen_at), now_ms),
                action_kind: "inline".into(),
                rich_component: None,
                rich_params: None,
            }
        })
        .collect())
}

/// 关系类型建议 pending → inline。无 created_at，用 last_seen_at。
async fn collect_relationship_suggestions(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .collection_relationship_type_suggestions()
        .find(
            doc! { "workspace_id": ws, "status": "pending" },
            mongodb::options::FindOptions::builder().limit(100).build(),
        )
        .await?;
    let rows: Vec<crate::models::RelationshipTypeSuggestion> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let id = r.id.map(|o| o.to_hex()).unwrap_or_default();
            InboxItem {
                source: "relationship_suggestion".into(),
                id,
                title: format!("关系类型建议：{}", r.suggested_value),
                summary: r.suggested_value.clone(),
                severity: "low".into(),
                created_at: Some(r.last_seen_at),
                age_hours: age_hours_of(Some(r.last_seen_at), now_ms),
                action_kind: "inline".into(),
                rich_component: None,
                rich_params: None,
            }
        })
        .collect())
}

/// 知识缺口信号 pending → inline。
async fn collect_gap_signals(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .knowledge_gap_signals()
        .find(
            doc! { "workspace_id": ws, "status": "pending" },
            mongodb::options::FindOptions::builder().limit(100).build(),
        )
        .await?;
    let rows: Vec<crate::models::KnowledgeGapSignal> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|g| {
            let id = g.id.map(|o| o.to_hex()).unwrap_or_default();
            InboxItem {
                source: "gap_signal".into(),
                id,
                title: g.title.clone(),
                summary: g.description.clone(),
                severity: "medium".into(),
                created_at: Some(g.created_at),
                age_hours: age_hours_of(Some(g.created_at), now_ms),
                action_kind: "inline".into(),
                rich_component: None,
                rich_params: None,
            }
        })
        .collect())
}

/// profile 待激活草稿(current_version=true && is_active=false) → rich。
async fn collect_profile_drafts(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .domain_profiles()
        .find(
            doc! { "workspace_id": ws, "current_version": true, "is_active": false },
            mongodb::options::FindOptions::builder().limit(50).build(),
        )
        .await?;
    let rows: Vec<crate::models::DomainProfile> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|p| {
            let id = p.id.map(|o| o.to_hex()).unwrap_or_default();
            InboxItem {
                source: "profile_risky".into(),
                id: id.clone(),
                title: format!("待激活画像：{}", p.display_name),
                summary: "AI 生成的运营画像草稿待人审激活".into(),
                severity: "high".into(),
                created_at: Some(p.created_at),
                age_hours: age_hours_of(Some(p.created_at), now_ms),
                action_kind: "rich".into(),
                rich_component: Some("profilePublish".into()),
                rich_params: Some(doc! { "profileId": id }),
            }
        })
        .collect())
}

/// 进化候选 eligible_for_release → rich。
async fn collect_evolution_proposals(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .proposals()
        .find(
            doc! { "workspace_id": ws, "status": "eligible_for_release" },
            mongodb::options::FindOptions::builder().limit(50).build(),
        )
        .await?;
    let rows: Vec<crate::models::Proposal> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|p| {
            let id = p.id.map(|o| o.to_hex()).unwrap_or_default();
            InboxItem {
                source: "evolution_proposal".into(),
                id: id.clone(),
                title: format!("进化候选：{}", p.proposal_kind),
                summary: p.diff_summary.clone().unwrap_or_default(),
                severity: "medium".into(),
                created_at: Some(p.created_at),
                age_hours: age_hours_of(Some(p.created_at), now_ms),
                action_kind: "rich".into(),
                rich_component: Some("evolutionRelease".into()),
                rich_params: Some(doc! { "proposalId": id }),
            }
        })
        .collect())
}

/// lessons_learned pending_review → rich。裸 Document（无 typed accessor）。
async fn collect_lessons_learned(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let coll = state.db.raw().collection::<Document>("lessons_learned");
    let cursor = coll
        .find(
            doc! { "workspace_id": ws, "review_status": "pending_review" },
            mongodb::options::FindOptions::builder().limit(50).build(),
        )
        .await?;
    let rows: Vec<Document> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|d| {
            // lessonId 必须用文档的 lesson_id 字段（{workspace}::{pattern_kind}），
            // 与 list/promote 端点（routes/lessons_learned.rs 按 lesson_id 寻址）一致；
            // 不可用 _id hex，否则深链卡片加载/晋升均 NotFound。
            let id = d.get_str("lesson_id").unwrap_or_default().to_string();
            let kind = d.get_str("pattern_kind").unwrap_or("").to_string();
            let created = d.get_datetime("created_at").ok().copied();
            InboxItem {
                source: "lessons_learned".into(),
                id: id.clone(),
                title: format!("经验晋升：{kind}"),
                summary: "AI 总结的经验待人审晋升为案例".into(),
                severity: "low".into(),
                created_at: created,
                age_hours: age_hours_of(created, now_ms),
                action_kind: "rich".into(),
                rich_component: Some("lessonsPromote".into()),
                rich_params: Some(doc! { "lessonId": id }),
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
    collect_source!("taxonomy_candidate", collect_taxonomy_candidates(&state, ws, now_ms));
    collect_source!("relationship_suggestion", collect_relationship_suggestions(&state, ws, now_ms));
    collect_source!("gap_signal", collect_gap_signals(&state, ws, now_ms));
    collect_source!("profile_risky", collect_profile_drafts(&state, ws, now_ms));
    collect_source!("evolution_proposal", collect_evolution_proposals(&state, ws, now_ms));
    collect_source!("lessons_learned", collect_lessons_learned(&state, ws, now_ms));

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
    let taxonomy_candidate = state
        .db
        .collection_taxonomy_candidates()
        .count_documents(doc! { "scope": "global", "status": "pending" }, None)
        .await
        .unwrap_or(0);
    let relationship_suggestion = state
        .db
        .collection_relationship_type_suggestions()
        .count_documents(doc! { "workspace_id": ws, "status": "pending" }, None)
        .await
        .unwrap_or(0);
    let gap_signal = state
        .db
        .knowledge_gap_signals()
        .count_documents(doc! { "workspace_id": ws, "status": "pending" }, None)
        .await
        .unwrap_or(0);
    let profile_risky = state
        .db
        .domain_profiles()
        .count_documents(
            doc! { "workspace_id": ws, "current_version": true, "is_active": false },
            None,
        )
        .await
        .unwrap_or(0);
    let evolution_proposal = state
        .db
        .proposals()
        .count_documents(doc! { "workspace_id": ws, "status": "eligible_for_release" }, None)
        .await
        .unwrap_or(0);
    let lessons_learned = state
        .db
        .raw()
        .collection::<Document>("lessons_learned")
        .count_documents(
            doc! { "workspace_id": ws, "review_status": "pending_review" },
            None,
        )
        .await
        .unwrap_or(0);
    Ok(Json(json!({
        "principalEscalation": escalations,
        "knowledgeReview": knowledge,
        "taxonomyCandidate": taxonomy_candidate,
        "relationshipSuggestion": relationship_suggestion,
        "gapSignal": gap_signal,
        "profileRisky": profile_risky,
        "evolutionProposal": evolution_proposal,
        "lessonsLearned": lessons_learned,
    })))
}

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
    // 请示卡富字段（仅 principal_escalation 来源填充）：决策人需看到客户是谁、
    // 请示什么问题、属哪类，避免盲裁。其余来源恒 None（skip_serializing 不输出）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_for_principal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_wxid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_wxid: Option<String>,
    // 关系类型建议富字段（仅 relationship_suggestion 来源填充）：决策人需看到
    // AI 判断依据/置信度/出现次数，避免盲批改写 relationship_type。其余来源恒 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrences: Option<i32>,
    // gap_signal 富字段:知识缺口的类型 + 语义严重度(info/warning/error/high,
    // 独立于 severity 排序标度)。仅 gap_signal 来源填充,其余恒 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_severity: Option<String>,
    // KB-08：知识切片核验状态(needs_review / needs_human_audit),供前端区分"待审"vs"AI预审通过·待复核"。
    // 仅 knowledge_review 来源填充,其余来源恒 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity_status: Option<String>,
}

fn age_hours_of(created: Option<DateTime>, now_ms: i64) -> f64 {
    created
        .map(|c| (now_ms - c.timestamp_millis()) as f64 / (3600.0 * 1000.0))
        .unwrap_or(0.0)
}

/// 空串归一成 None（避免把空字段当成有值投影给前端）。
fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 单条请示 → InboxItem（具名以便单测）。保留富字段
/// category/question_for_principal/contact_wxid/principal_wxid，避免决策人盲裁。
fn escalation_to_inbox_item(
    e: &crate::models::AgentPrincipalEscalation,
    now_ms: i64,
) -> InboxItem {
    InboxItem {
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
        category: non_empty(&e.category),
        question_for_principal: non_empty(&e.question_for_principal),
        contact_wxid: non_empty(&e.contact_wxid),
        principal_wxid: non_empty(&e.principal_wxid),
        evidence: None,
        confidence: None,
        occurrences: None,
        kind: None,
        signal_severity: None,
        integrity_status: None,
    }
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
        .map(|e| escalation_to_inbox_item(&e, now_ms))
        .collect())
}

/// KB-08：审核收件箱认可的知识切片 integrity_status 集合。
/// needs_review = AI 起草待审；needs_human_audit = auto_verify 预审通过、待人复核。
/// 二者都须进人审收件箱(否则 needs_human_audit 切片成黑洞)。列表查询与 summary 计数共用本函数,防漂移。
pub(crate) fn knowledge_review_statuses() -> [&'static str; 2] {
    ["needs_review", "needs_human_audit"]
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
            doc! { "workspace_id": ws, "integrity_status": { "$in": knowledge_review_statuses().to_vec() } },
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
                category: None,
                question_for_principal: None,
                contact_wxid: None,
                principal_wxid: None,
                evidence: None,
                confidence: None,
                occurrences: None,
                kind: None,
                signal_severity: None,
                integrity_status: c.integrity_status.clone(),
            }
        })
        .collect())
}

/// 单条标签候选 → InboxItem（具名以便单测）。归类 rich：审核 = 给 AI 新造取值
/// 命名并纳入字典，需命名表单，不是简单二元通过/拒绝。富字段 evidence/confidence/
/// occurrences 与 relationship_suggestion 对称接出；rich_params 带全前端渲染所需数据。
fn taxonomy_candidate_to_inbox_item(
    c: &crate::models::TaxonomyCandidate,
    now_ms: i64,
) -> InboxItem {
    let id = c.id.map(|o| o.to_hex()).unwrap_or_default();
    let mut params = doc! {
        "candidateId": id.clone(),
        "scope": c.scope.clone(),
        "kind": c.kind.clone(),
        "rawValue": c.raw_value.clone(),
        "confidence": c.confidence,
        "occurrences": c.occurrences,
    };
    if let Some(ev) = &c.evidence {
        params.insert("evidence", ev.clone());
    }
    if let Some(name) = &c.suggested_display_name {
        params.insert("suggestedDisplayName", name.clone());
    }
    InboxItem {
        source: "taxonomy_candidate".into(),
        id,
        // 人话标题：以 AI 新识别的取值为主语，不暴露裸维度键（维度中文名前端补）。
        title: format!("AI 新识别标签：{}", c.raw_value),
        // 折叠预览：优先 evidence，无则通用框定。
        summary: c
            .evidence
            .clone()
            .unwrap_or_else(|| "AI 在对话中识别到一个尚未收录的取值，请确认是否纳入标签字典".into()),
        severity: "low".into(),
        created_at: Some(c.last_seen_at),
        age_hours: age_hours_of(Some(c.last_seen_at), now_ms),
        action_kind: "rich".into(),
        rich_component: Some("taxonomyCandidateReview".into()),
        rich_params: Some(params),
        category: None,
        question_for_principal: None,
        contact_wxid: None,
        principal_wxid: None,
        evidence: c.evidence.clone(),
        confidence: Some(c.confidence),
        occurrences: Some(c.occurrences),
        kind: None,
        signal_severity: None,
        integrity_status: None,
    }
}

/// 标签候选 pending → rich。只暴露当前 workspace 的 global 候选；account 私有
/// scope 不进入共享审核收件箱。workspace + scope 双重过滤避免跨租户泄漏。
async fn collect_taxonomy_candidates(
    state: &AppState,
    ws: &str,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .collection_taxonomy_candidates()
        .find(
            pending_global_taxonomy_filter(ws),
            mongodb::options::FindOptions::builder().limit(100).build(),
        )
        .await?;
    let rows: Vec<crate::models::TaxonomyCandidate> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|c| taxonomy_candidate_to_inbox_item(&c, now_ms))
        .collect())
}

fn pending_global_taxonomy_filter(workspace_id: &str) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "scope": "global",
        "status": "pending",
    }
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
                summary: r.evidence.clone().unwrap_or_else(|| r.suggested_value.clone()),
                severity: "low".into(),
                created_at: Some(r.last_seen_at),
                age_hours: age_hours_of(Some(r.last_seen_at), now_ms),
                action_kind: "inline".into(),
                rich_component: None,
                rich_params: None,
                category: Some(r.suggested_value.clone()),
                question_for_principal: None,
                contact_wxid: non_empty(&r.contact_id),
                principal_wxid: None,
                evidence: r.evidence.clone(),
                confidence: Some(r.confidence),
                occurrences: Some(r.occurrences),
                kind: None,
                signal_severity: None,
                integrity_status: None,
            }
        })
        .collect())
}

/// 单条知识缺口信号 → InboxItem(具名以便单测)。kind/signal_severity 富字段
/// 让决策人看到缺口类型与语义严重度;severity 保持 "medium" 排序标度不变。
fn gap_to_inbox_item(g: &crate::models::KnowledgeGapSignal, now_ms: i64) -> InboxItem {
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
        category: None,
        question_for_principal: None,
        contact_wxid: None,
        principal_wxid: None,
        evidence: None,
        confidence: None,
        occurrences: None,
        kind: non_empty(&g.kind),
        signal_severity: non_empty(&g.severity),
        integrity_status: None,
    }
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
        .map(|g| gap_to_inbox_item(&g, now_ms))
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
                category: None,
                question_for_principal: None,
                contact_wxid: None,
                principal_wxid: None,
                evidence: None,
                confidence: None,
                occurrences: None,
                kind: None,
                signal_severity: None,
                integrity_status: None,
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
                category: None,
                question_for_principal: None,
                contact_wxid: None,
                principal_wxid: None,
                evidence: None,
                confidence: None,
                occurrences: None,
                kind: None,
                signal_severity: None,
                integrity_status: None,
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
                category: None,
                question_for_principal: None,
                contact_wxid: None,
                principal_wxid: None,
                evidence: None,
                confidence: None,
                occurrences: None,
                kind: None,
                signal_severity: None,
                integrity_status: None,
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
        .count_documents(doc! { "workspace_id": ws, "integrity_status": { "$in": knowledge_review_statuses().to_vec() } }, None)
        .await
        .unwrap_or(0);
    let taxonomy_candidate = state
        .db
        .collection_taxonomy_candidates()
        .count_documents(pending_global_taxonomy_filter(ws), None)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AgentPrincipalEscalation;
    use mongodb::bson::DateTime;

    #[test]
    fn pending_global_taxonomy_filter_is_workspace_scoped() {
        assert_eq!(
            pending_global_taxonomy_filter("ws-a"),
            doc! {
                "workspace_id": "ws-a",
                "scope": "global",
                "status": "pending",
            }
        );
    }

    fn test_escalation_fixture() -> AgentPrincipalEscalation {
        let now = DateTime::now();
        AgentPrincipalEscalation {
            id: None,
            workspace_id: "ws1".into(),
            account_id: "acc1".into(),
            contact_wxid: "wxid_cust".into(),
            short_code: "E1A2".into(),
            status: "pending".into(),
            category: "discount_request".into(),
            reason: "客户想要折扣，超出 AI 职权".into(),
            question_for_principal: "能否给折扣".into(),
            principal_wxid: "wxid_boss".into(),
            decision: None,
            authorization_expires_at: None,
            is_generalizable: false,
            knowledge_proposal_emitted: false,
            last_holding_reply_ms: None,
            last_pushed_at_ms: None,
            created_at: now,
            updated_at: now,
            resolved_at: None,
            resolved_via: None,
        }
    }

    #[test]
    fn escalation_projection_carries_rich_fields() {
        let esc = test_escalation_fixture();
        let now_ms = DateTime::now().timestamp_millis();
        let item = escalation_to_inbox_item(&esc, now_ms);
        assert_eq!(item.category.as_deref(), Some("discount_request"));
        assert_eq!(item.question_for_principal.as_deref(), Some("能否给折扣"));
        assert_eq!(item.contact_wxid.as_deref(), Some("wxid_cust"));
        assert_eq!(item.principal_wxid.as_deref(), Some("wxid_boss"));
    }

    #[test]
    fn escalation_projection_empty_strings_become_none() {
        let mut esc = test_escalation_fixture();
        esc.category = String::new();
        esc.question_for_principal = String::new();
        esc.contact_wxid = String::new();
        esc.principal_wxid = String::new();
        let now_ms = DateTime::now().timestamp_millis();
        let item = escalation_to_inbox_item(&esc, now_ms);
        assert_eq!(item.category, None);
        assert_eq!(item.question_for_principal, None);
        assert_eq!(item.contact_wxid, None);
        assert_eq!(item.principal_wxid, None);
    }

    #[test]
    fn escalation_rich_fields_serialize_camel_case() {
        let esc = test_escalation_fixture();
        let item = escalation_to_inbox_item(&esc, 0);
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["questionForPrincipal"], "能否给折扣");
        assert_eq!(v["contactWxid"], "wxid_cust");
        assert_eq!(v["principalWxid"], "wxid_boss");
        assert_eq!(v["category"], "discount_request");
    }

    fn test_gap_fixture() -> crate::models::KnowledgeGapSignal {
        crate::models::KnowledgeGapSignal {
            id: None,
            signal_id: "g1".into(),
            workspace_id: "ws1".into(),
            dedup_key: None,
            kind: "orphan".into(),
            title: "孤立切片：定价政策".into(),
            description: "该切片无任何入向引用".into(),
            affected_chunk_ids: vec![],
            search_queries: vec![],
            severity: "warning".into(),
            source: "rule".into(),
            status: "pending".into(),
            resolution_note: None,
            created_at: DateTime::now(),
            resolved_at: None,
        }
    }

    #[test]
    fn gap_signal_projection_carries_kind_and_severity() {
        let gap = test_gap_fixture();
        let now_ms = DateTime::now().timestamp_millis();
        let item = gap_to_inbox_item(&gap, now_ms);
        assert_eq!(item.kind.as_deref(), Some("orphan"));
        assert_eq!(item.signal_severity.as_deref(), Some("warning"));
        // severity 排序标度必须保持 "medium",绝不被 gap 语义严重度污染。
        assert_eq!(item.severity, "medium");
    }

    #[test]
    fn gap_signal_rich_fields_serialize_camel_case() {
        let gap = test_gap_fixture();
        let item = gap_to_inbox_item(&gap, 0);
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["kind"], "orphan");
        assert_eq!(v["signalSeverity"], "warning");
        assert_eq!(v["severity"], "medium");
    }

    fn test_candidate_fixture() -> crate::models::TaxonomyCandidate {
        let now = DateTime::now();
        crate::models::TaxonomyCandidate {
            id: None,
            workspace_id: "default".into(),
            scope: "global".into(),
            kind: "emotional_state".into(),
            raw_value: "anxious".into(),
            evidence: Some("客户连续两条消息表达担心".into()),
            confidence: 7,
            first_seen_at: now,
            last_seen_at: now,
            occurrences: 3,
            status: "pending".into(),
            reviewed_at: None,
            reviewed_by: None,
            suggested_display_name: Some("焦虑".into()),
        }
    }

    #[test]
    fn taxonomy_candidate_projected_as_rich() {
        let c = test_candidate_fixture();
        // id=None 时 hex 落空串；本测聚焦 rich 分类与富字段，用 None 固定路径。
        let item = taxonomy_candidate_to_inbox_item(&c, 0);
        assert_eq!(item.action_kind, "rich");
        assert_eq!(item.rich_component.as_deref(), Some("taxonomyCandidateReview"));
        // title 不再以裸维度键作主语（回归防护：不得再出现 "标签候选：emotional_state"）。
        assert!(!item.title.contains("emotional_state"), "title 不应暴露裸维度键: {}", item.title);
        // 顶层富字段与 relationship_suggestion 对称接出。
        assert_eq!(item.evidence.as_deref(), Some("客户连续两条消息表达担心"));
        assert_eq!(item.confidence, Some(7));
        assert_eq!(item.occurrences, Some(3));
    }

    #[test]
    fn taxonomy_candidate_rich_params_carry_all_fields() {
        let c = test_candidate_fixture();
        let item = taxonomy_candidate_to_inbox_item(&c, 0);
        let params = item.rich_params.expect("rich_params 应存在");
        assert_eq!(params.get_str("scope").unwrap(), "global");
        assert_eq!(params.get_str("kind").unwrap(), "emotional_state");
        assert_eq!(params.get_str("rawValue").unwrap(), "anxious");
        assert_eq!(params.get_str("evidence").unwrap(), "客户连续两条消息表达担心");
        assert_eq!(params.get_i32("confidence").unwrap(), 7);
        assert_eq!(params.get_i32("occurrences").unwrap(), 3);
        assert_eq!(params.get_str("suggestedDisplayName").unwrap(), "焦虑");
    }

    #[test]
    fn taxonomy_candidate_optional_fields_omitted_when_absent() {
        let mut c = test_candidate_fixture();
        c.evidence = None;
        c.suggested_display_name = None;
        let item = taxonomy_candidate_to_inbox_item(&c, 0);
        let params = item.rich_params.expect("rich_params 应存在");
        // evidence / suggestedDisplayName 缺省时不写键（不产生 null）。
        assert!(params.get("evidence").is_none());
        assert!(params.get("suggestedDisplayName").is_none());
        assert_eq!(item.evidence, None);
        // confidence / occurrences 是非 Option i32，恒写入。
        assert!(params.get("confidence").is_some());
        assert!(params.get("occurrences").is_some());
    }

    #[test]
    fn taxonomy_candidate_serializes_camel_case() {
        let c = test_candidate_fixture();
        let item = taxonomy_candidate_to_inbox_item(&c, 0);
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["actionKind"], "rich");
        assert_eq!(v["richComponent"], "taxonomyCandidateReview");
        assert_eq!(v["confidence"], 7);
        assert_eq!(v["occurrences"], 3);
    }

    #[test]
    fn knowledge_review_statuses_includes_needs_human_audit() {
        // KB-08 病根锚死：审核收件箱必须同时认 needs_human_audit,
        // 否则 auto_verify 分诊出的切片从收件箱消失(黑洞)。防回退成只查 needs_review。
        let s = knowledge_review_statuses();
        assert!(s.contains(&"needs_review"), "必须含 needs_review");
        assert!(s.contains(&"needs_human_audit"), "必须含 needs_human_audit(KB-08 黑洞根因)");
    }
}

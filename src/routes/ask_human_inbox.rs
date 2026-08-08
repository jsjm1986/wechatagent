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
    /// Owning business account. None means workspace-global governance work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
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
fn escalation_to_inbox_item(e: &crate::models::AgentPrincipalEscalation, now_ms: i64) -> InboxItem {
    InboxItem {
        source: "principal_escalation".into(),
        id: e.short_code.clone(),
        account_id: non_empty(&e.account_id),
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
    account_id: Option<&str>,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let mut filter = doc! { "workspace_id": ws, "status": "pending" };
    if let Some(account_id) = account_id {
        filter.insert("account_id", account_id);
    }
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            filter,
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .build(),
        )
        .await?;
    let rows: Vec<crate::models::AgentPrincipalEscalation> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|entry| escalation_to_inbox_item(&entry, now_ms))
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
    account_id: Option<&str>,
    _now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let filter = account_scoped_filter(
        doc! { "workspace_id": ws, "integrity_status": { "$in": knowledge_review_statuses().to_vec() } },
        account_id,
        true,
    );
    let cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            filter,
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
                account_id: c.account_id.clone().filter(|value| !value.is_empty()),
                title: c.title.clone(),
                summary: c
                    .body
                    .clone()
                    .unwrap_or_default()
                    .chars()
                    .take(80)
                    .collect(),
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
        account_id: None,
        // 人话标题：以 AI 新识别的取值为主语，不暴露裸维度键（维度中文名前端补）。
        title: format!("AI 新识别标签：{}", c.raw_value),
        // 折叠预览：优先 evidence，无则通用框定。
        summary: c.evidence.clone().unwrap_or_else(|| {
            "AI 在对话中识别到一个尚未收录的取值，请确认是否纳入标签字典".into()
        }),
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
    account_id: Option<&str>,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let filter = account_scoped_filter(
        doc! { "workspace_id": ws, "status": "pending" },
        account_id,
        false,
    );
    let cursor = state
        .db
        .collection_relationship_type_suggestions()
        .find(
            filter,
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
                account_id: non_empty(&r.account_id),
                title: format!("关系类型建议：{}", r.suggested_value),
                summary: r
                    .evidence
                    .clone()
                    .unwrap_or_else(|| r.suggested_value.clone()),
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

/// 疑似成交 pending → rich。审批需要可选金额/币种和必填驳回原因，不能复用
/// 空 body 的通用通过/拒绝按钮，因此由专用卡片处置。
fn suspected_deal_to_inbox_item(
    signal: &crate::models::SuspectedDealSignal,
    now_ms: i64,
) -> InboxItem {
    let id = signal.id.map(|value| value.to_hex()).unwrap_or_default();
    let mut params = doc! {
        "signalId": id.clone(),
        "accountId": signal.account_id.clone(),
        "contactId": signal.contact_id.clone(),
        "value": signal.value.clone(),
        "confidence": signal.confidence,
        "occurrences": signal.occurrences,
    };
    if let Some(evidence) = &signal.evidence {
        params.insert("evidence", evidence.clone());
    }
    InboxItem {
        source: "suspected_deal".into(),
        id,
        account_id: non_empty(&signal.account_id),
        title: signal.value.clone(),
        summary: signal
            .evidence
            .clone()
            .unwrap_or_else(|| "AI 识别到疑似成交信号，请运营核实后再登记成交".into()),
        severity: "high".into(),
        created_at: Some(signal.first_seen_at),
        age_hours: age_hours_of(Some(signal.first_seen_at), now_ms),
        action_kind: "rich".into(),
        rich_component: Some("suspectedDealReview".into()),
        rich_params: Some(params),
        category: None,
        question_for_principal: None,
        contact_wxid: non_empty(&signal.contact_id),
        principal_wxid: None,
        evidence: signal.evidence.clone(),
        confidence: Some(signal.confidence),
        occurrences: Some(signal.occurrences),
        kind: None,
        signal_severity: None,
        integrity_status: None,
    }
}

async fn collect_suspected_deals(
    state: &AppState,
    ws: &str,
    account_id: Option<&str>,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let filter = account_scoped_filter(
        doc! { "workspace_id": ws, "status": "pending" },
        account_id,
        false,
    );
    let cursor = state
        .db
        .collection_suspected_deal_signals()
        .find(
            filter,
            mongodb::options::FindOptions::builder()
                .sort(doc! { "last_seen_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
    let rows: Vec<crate::models::SuspectedDealSignal> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|signal| suspected_deal_to_inbox_item(&signal, now_ms))
        .collect())
}

/// 单条知识缺口信号 → InboxItem(具名以便单测)。kind/signal_severity 富字段
/// 让决策人看到缺口类型与语义严重度;severity 保持 "medium" 排序标度不变。
fn gap_to_inbox_item(g: &crate::models::KnowledgeGapSignal, now_ms: i64) -> InboxItem {
    let id = g.id.map(|o| o.to_hex()).unwrap_or_default();
    InboxItem {
        source: "gap_signal".into(),
        id,
        account_id: None,
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
async fn collect_gap_signals(state: &AppState, ws: &str, now_ms: i64) -> AppResult<Vec<InboxItem>> {
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

fn reviewable_profile_filter(workspace_id: &str) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "is_active": false,
        "$or": [
            { "release_status": "draft" },
            { "release_status": "published", "current_version": true },
        ],
    }
}

/// Unpublished drafts and published-current rows waiting for activation both
/// belong to the existing rich review card. Historical published rows remain
/// excluded.
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
            reviewable_profile_filter(ws),
            mongodb::options::FindOptions::builder().limit(50).build(),
        )
        .await?;
    let rows: Vec<crate::models::DomainProfile> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|p| {
            let id = p.id.map(|o| o.to_hex()).unwrap_or_default();
            let is_draft = p.release_status == "draft";
            InboxItem {
                source: "profile_risky".into(),
                id: id.clone(),
                account_id: None,
                title: format!(
                    "{}画像：{}",
                    if is_draft { "待发布" } else { "待激活" },
                    p.display_name
                ),
                summary: if is_draft {
                    "行业画像草稿待人审发布".into()
                } else {
                    "已发布行业画像待人审激活".into()
                },
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
    account_id: Option<&str>,
    now_ms: i64,
) -> AppResult<Vec<InboxItem>> {
    use futures::TryStreamExt;
    let filter = account_scoped_filter(
        doc! { "workspace_id": ws, "status": "eligible_for_release" },
        account_id,
        false,
    );
    let cursor = state
        .db
        .proposals()
        .find(
            filter,
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
                account_id: non_empty(&p.account_id),
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
                account_id: None,
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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxQuery {
    #[serde(default)]
    pub source: Option<String>,
    /// Optional account scope. Workspace-global governance items remain visible.
    #[serde(default)]
    pub account_id: Option<String>,
}

fn requested_account(query: &InboxQuery) -> Option<&str> {
    query
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn account_scoped_filter(
    mut filter: Document,
    account_id: Option<&str>,
    include_global: bool,
) -> Document {
    if let Some(account_id) = account_id {
        if include_global {
            filter.insert(
                "$or",
                vec![
                    doc! { "account_id": account_id },
                    doc! { "account_id": null },
                    doc! { "account_id": { "$exists": false } },
                ],
            );
        } else {
            filter.insert("account_id", account_id);
        }
    }
    filter
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
    let account_id = requested_account(&q);

    // 每 source 独立降级：Err 不整体崩，记进 errors 数组。
    macro_rules! collect_source {
        ($name:expr, $fut:expr) => {
            if q.source.as_deref().map(|s| s == $name).unwrap_or(true) {
                match $fut.await {
                    Ok(mut values) => items.append(&mut values),
                    Err(error) => errors.push(json!({ "source": $name, "error": error.to_string() })),
                }
            }
        };
    }

    collect_source!(
        "principal_escalation",
        collect_escalations(&state, ws, account_id, now_ms)
    );
    collect_source!(
        "knowledge_review",
        collect_knowledge_review(&state, ws, account_id, now_ms)
    );
    collect_source!(
        "taxonomy_candidate",
        collect_taxonomy_candidates(&state, ws, now_ms)
    );
    collect_source!(
        "relationship_suggestion",
        collect_relationship_suggestions(&state, ws, account_id, now_ms)
    );
    collect_source!(
        "suspected_deal",
        collect_suspected_deals(&state, ws, account_id, now_ms)
    );
    collect_source!("gap_signal", collect_gap_signals(&state, ws, now_ms));
    collect_source!("profile_risky", collect_profile_drafts(&state, ws, now_ms));
    collect_source!(
        "evolution_proposal",
        collect_evolution_proposals(&state, ws, account_id, now_ms)
    );
    collect_source!(
        "lessons_learned",
        collect_lessons_learned(&state, ws, now_ms)
    );

    Ok(Json(json!({ "items": items, "errors": errors })))
}

/// GET /api/admin/ask-human/summary —— 各 source pending 计数。
pub async fn ask_human_summary(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<InboxQuery>,
) -> AppResult<Json<Value>> {
    let ws = &admin.current_workspace;
    let account_id = requested_account(&q);
    let escalations_collection = state.db.agent_principal_escalations();
    let knowledge_collection = state.db.operation_knowledge_chunks();
    let taxonomy_collection = state.db.collection_taxonomy_candidates();
    let relationship_collection = state.db.collection_relationship_type_suggestions();
    let suspected_deal_collection = state.db.collection_suspected_deal_signals();
    let gap_collection = state.db.knowledge_gap_signals();
    let profile_collection = state.db.domain_profiles();
    let proposal_collection = state.db.proposals();
    let lessons_collection = state.db.raw().collection::<Document>("lessons_learned");
    let (
        escalations,
        knowledge,
        taxonomy_candidate,
        relationship_suggestion,
        suspected_deal,
        gap_signal,
        profile_risky,
        evolution_proposal,
        lessons_learned,
    ) = tokio::join!(
        escalations_collection
            .count_documents(account_scoped_filter(doc! { "workspace_id": ws, "status": "pending" }, account_id, false), None),
        knowledge_collection.count_documents(
            account_scoped_filter(doc! { "workspace_id": ws, "integrity_status": { "$in": knowledge_review_statuses().to_vec() } }, account_id, true),
            None,
        ),
        taxonomy_collection
            .count_documents(pending_global_taxonomy_filter(ws), None),
        relationship_collection
            .count_documents(account_scoped_filter(doc! { "workspace_id": ws, "status": "pending" }, account_id, false), None),
        suspected_deal_collection
            .count_documents(account_scoped_filter(doc! { "workspace_id": ws, "status": "pending" }, account_id, false), None),
        gap_collection
            .count_documents(doc! { "workspace_id": ws, "status": "pending" }, None),
        profile_collection.count_documents(reviewable_profile_filter(ws), None),
        proposal_collection.count_documents(
            account_scoped_filter(doc! { "workspace_id": ws, "status": "eligible_for_release" }, account_id, false),
            None,
        ),
        lessons_collection.count_documents(
            doc! { "workspace_id": ws, "review_status": "pending_review" },
            None,
        ),
    );

    let mut counts = Vec::with_capacity(9);
    macro_rules! record_count {
        ($key:literal, $source:literal, $result:expr) => {
            counts.push((
                $key,
                $source,
                $result.map_err(|error| {
                    tracing::warn!(
                        source = $source,
                        error = %error,
                        "ask-human summary count unavailable"
                    );
                    "count unavailable".to_string()
                }),
            ));
        };
    }
    record_count!("principalEscalation", "principal_escalation", escalations);
    record_count!("knowledgeReview", "knowledge_review", knowledge);
    record_count!(
        "taxonomyCandidate",
        "taxonomy_candidate",
        taxonomy_candidate
    );
    record_count!(
        "relationshipSuggestion",
        "relationship_suggestion",
        relationship_suggestion
    );
    record_count!("suspectedDeal", "suspected_deal", suspected_deal);
    record_count!("gapSignal", "gap_signal", gap_signal);
    record_count!("profileRisky", "profile_risky", profile_risky);
    record_count!(
        "evolutionProposal",
        "evolution_proposal",
        evolution_proposal
    );
    record_count!("lessonsLearned", "lessons_learned", lessons_learned);

    Ok(Json(build_summary_response(counts, DateTime::now())))
}

fn build_summary_response(
    results: Vec<(&'static str, &'static str, Result<u64, String>)>,
    as_of: DateTime,
) -> Value {
    let source_count = results.len();
    let mut counts = serde_json::Map::new();
    let mut legacy = serde_json::Map::new();
    let mut errors = Vec::new();
    let mut total = 0u64;

    for (key, source, result) in results {
        let value = match result {
            Ok(count) => {
                total = total.saturating_add(count);
                json!(count)
            }
            Err(error) => {
                errors.push(json!({ "source": source, "error": error }));
                Value::Null
            }
        };
        counts.insert(key.to_string(), value.clone());
        legacy.insert(key.to_string(), value);
    }

    let status = match errors.len() {
        0 => "complete",
        failed if failed == source_count => "error",
        _ => "partial",
    };
    let mut response = serde_json::Map::new();
    response.insert("status".into(), json!(status));
    response.insert(
        "asOf".into(),
        crate::models::dt_to_string(as_of)
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    response.insert("counts".into(), Value::Object(counts));
    response.insert("errors".into(), Value::Array(errors));
    response.insert(
        "total".into(),
        if status == "complete" {
            json!(total)
        } else {
            Value::Null
        },
    );
    // Compatibility for clients that still read the original top-level keys.
    response.extend(legacy);
    Value::Object(response)
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

    #[test]
    fn account_filter_scopes_account_rows_before_limit() {
        assert_eq!(
            account_scoped_filter(
                doc! { "workspace_id": "ws-a", "status": "pending" },
                Some("acc-2"),
                false
            ),
            doc! { "workspace_id": "ws-a", "status": "pending", "account_id": "acc-2" },
        );
    }

    #[test]
    fn account_filter_keeps_workspace_global_rows() {
        assert_eq!(
            account_scoped_filter(doc! { "workspace_id": "ws-a" }, Some("acc-2"), true),
            doc! {
                "workspace_id": "ws-a",
                "$or": [
                    { "account_id": "acc-2" },
                    { "account_id": null },
                    { "account_id": { "$exists": false } },
                ],
            },
        );
    }

    #[test]
    fn reviewable_profile_filter_includes_drafts_and_pending_activation_only() {
        assert_eq!(
            reviewable_profile_filter("ws-a"),
            doc! {
                "workspace_id": "ws-a",
                "is_active": false,
                "$or": [
                    { "release_status": "draft" },
                    { "release_status": "published", "current_version": true },
                ],
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
            protocol: None,
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
            relay_state: None,
            relay_task_id: None,
            relay_enqueued_at: None,
            relay_terminal_at: None,
            relay_terminal_reason: None,
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
        assert_eq!(v["accountId"], "acc1");
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
        assert_eq!(
            item.rich_component.as_deref(),
            Some("taxonomyCandidateReview")
        );
        // title 不再以裸维度键作主语（回归防护：不得再出现 "标签候选：emotional_state"）。
        assert!(
            !item.title.contains("emotional_state"),
            "title 不应暴露裸维度键: {}",
            item.title
        );
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
        assert_eq!(
            params.get_str("evidence").unwrap(),
            "客户连续两条消息表达担心"
        );
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
    fn suspected_deal_projects_to_dedicated_rich_review() {
        let now = DateTime::from_millis(1_700_000_000_000);
        let signal = crate::models::SuspectedDealSignal {
            id: Some(mongodb::bson::oid::ObjectId::new()),
            workspace_id: "ws1".into(),
            account_id: "acc1".into(),
            contact_id: "contact1".into(),
            value: "疑似成交·待核实".into(),
            evidence: Some("客户明确表示准备付款".into()),
            confidence: 86,
            status: "pending".into(),
            occurrences: 2,
            first_seen_at: now,
            last_seen_at: now,
            reviewed_at: None,
            reviewed_by: None,
        };

        let item = suspected_deal_to_inbox_item(&signal, now.timestamp_millis());
        assert_eq!(item.source, "suspected_deal");
        assert_eq!(item.action_kind, "rich");
        assert_eq!(item.rich_component.as_deref(), Some("suspectedDealReview"));
        assert_eq!(item.contact_wxid.as_deref(), Some("contact1"));
        assert_eq!(item.evidence.as_deref(), Some("客户明确表示准备付款"));
        assert_eq!(item.confidence, Some(86));
        assert_eq!(item.occurrences, Some(2));
        let params = item.rich_params.expect("dedicated review params");
        assert_eq!(params.get_str("accountId").unwrap(), "acc1");
        assert_eq!(params.get_str("contactId").unwrap(), "contact1");
        assert_eq!(params.get_i32("confidence").unwrap(), 86);
    }

    #[test]
    fn knowledge_review_statuses_includes_needs_human_audit() {
        // KB-08 病根锚死：审核收件箱必须同时认 needs_human_audit,
        // 否则 auto_verify 分诊出的切片从收件箱消失(黑洞)。防回退成只查 needs_review。
        let s = knowledge_review_statuses();
        assert!(s.contains(&"needs_review"), "必须含 needs_review");
        assert!(
            s.contains(&"needs_human_audit"),
            "必须含 needs_human_audit(KB-08 黑洞根因)"
        );
    }

    #[test]
    fn summary_partial_failure_is_null_not_zero() {
        let value = build_summary_response(
            vec![
                ("principalEscalation", "principal_escalation", Ok(2)),
                (
                    "knowledgeReview",
                    "knowledge_review",
                    Err("count unavailable".to_string()),
                ),
            ],
            DateTime::from_millis(1_700_000_000_000),
        );
        assert_eq!(value["status"], "partial");
        assert_eq!(value["counts"]["principalEscalation"], 2);
        assert!(value["counts"]["knowledgeReview"].is_null());
        assert!(value["knowledgeReview"].is_null());
        assert!(value["total"].is_null());
        assert_eq!(value["errors"][0]["source"], "knowledge_review");
    }
}

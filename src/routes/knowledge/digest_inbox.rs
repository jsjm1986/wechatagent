//! 运营知识库 AI 日报 digest + 待办 inbox 聚合。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{bson::doc, bson::oid::ObjectId, options::FindOptions};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};

use super::super::AppState;
use super::*;

// ── knowledge-digest-workstation Phase 1：日报路由（最小骨架） ──────────────
//
// `GET /api/knowledge/digest/today`：查询当日 `knowledge_daily_reports`，命中
// 即返回；未命中**直接 404**，**不**触发同步合成（Phase 2 才接 generate）。
// 设计见 `.kiro/specs/knowledge-digest-workstation/design.md` §6 Routes 与
// `docs/data-and-api.md` 知识库日报工作站章节。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct DigestTodayQuery {
    pub account_id: Option<String>,
    /// `YYYY-MM-DD`；缺省时用运营时区今天。
    pub report_date: Option<String>,
}

pub(in crate::routes) async fn digest_today(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<DigestTodayQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let report_date = query
        .report_date
        .clone()
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());

    let found = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?;

    let report = match found {
        Some(r) => r,
        None => {
            // Phase 2：未命中时**同步合成**今日日报；失败则按 503 / 404 上抛。
            // 避免运营反复刷新 → 命中 worker 还没醒的窗口期。
            crate::knowledge_digest::generate_today_digest(&state).await?
        }
    };

    Ok(serialize_digest_report(&report))
}

/// `POST /api/knowledge/digest/regenerate`：强制重算今日日报。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct DigestRegenerateRequest {
    pub account_id: Option<String>,
    #[serde(default)]
    pub force: bool,
}

pub(in crate::routes) async fn digest_regenerate(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<DigestRegenerateRequest>,
) -> AppResult<Json<Value>> {
    let account_id = body
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();

    if !body.force {
        // 非强制路径：若今日日报已存在，直接返回，不重复调 LLM。
        if let Some(existing) = state
            .db
            .knowledge_daily_reports()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &account_id,
                    "report_date": &report_date,
                },
                None,
            )
            .await?
        {
            return Ok(serialize_digest_report(&existing));
        }
    }
    let report = crate::knowledge_digest::generate_today_digest(&state).await?;
    Ok(serialize_digest_report(&report))
}

/// `POST /api/knowledge/digest/cards/:id/dismiss`：把卡片标记为已忽略，画布灰显。
pub(in crate::routes) async fn digest_dismiss_card(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(card_id_hex): Path<String>,
) -> AppResult<Json<Value>> {
    let card_id = ObjectId::parse_str(&card_id_hex)
        .map_err(|_| AppError::BadRequest(format!("invalid card_id: {card_id_hex}")))?;
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let result = state
        .db
        .knowledge_daily_reports()
        .update_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "report_date": &report_date,
                "cards.cardId": &card_id,
            },
            doc! {
                "$addToSet": { "dismissed_card_ids": &card_id }
            },
            None,
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(format!(
            "未找到包含 cardId={} 的今日日报",
            card_id_hex
        )));
    }
    Ok(Json(json!({
        "ok": true,
        "cardId": card_id_hex,
        "reportDate": report_date,
    })))
}

fn serialize_digest_report(report: &crate::models::KnowledgeDailyReport) -> Json<Value> {
    Json(json!({
        "reportId": report.id.map(|id| id.to_hex()),
        "workspaceId": report.workspace_id,
        "accountId": report.account_id,
        "reportDate": report.report_date,
        "generatedAt": report.generated_at.to_string(),
        "generatedBy": report.generated_by,
        "status": report.status,
        "errorKind": report.error_kind,
        "budgetSnapshot": serde_json::to_value(&report.budget_snapshot).unwrap_or(json!({})),
        "cards": serde_json::to_value(&report.cards).unwrap_or(json!([])),
        "dismissedCardIds": report
            .dismissed_card_ids
            .iter()
            .map(|id| id.to_hex())
            .collect::<Vec<_>>(),
        "promptVersions": serde_json::to_value(&report.prompt_versions).unwrap_or(json!({})),
    }))
}

// ── AI Inbox 聚合（GET /operation-knowledge/inbox） ────────────────────────
//
// 知识库 AI 协作工作站顶层的待办流。把四类只读信号聚合成统一形态：
//   1. digest_card    —— 当日 KnowledgeDailyReport.cards（未 dismiss）
//   2. quote_missing  —— operation_knowledge_chunks 缺 source_quote
//   3. anchors_missing —— operation_knowledge_chunks 缺 source_anchors
//   4. pending_review —— integrity_status == "needs_review"
//
// 全部 read-only，**不写库**、不动 schema、不新增 collection。

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct InboxQuery {
    pub account_id: Option<String>,
    pub priority: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct InboxCardView {
    pub id: String,
    pub priority: String,
    pub kind: String,
    pub title: String,
    pub context_summary: String,
    pub target_chunk_id: Option<String>,
    pub target_pack_id: Option<String>,
    pub suggested_actions: Vec<String>,
    pub origin: String,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct InboxStats {
    pub total: usize,
    pub high: usize,
    pub mid: usize,
    pub low: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct InboxResponse {
    pub items: Vec<InboxCardView>,
    pub stats: InboxStats,
}

/// digest 卡片 severity → inbox priority。
fn severity_to_priority(severity: &str) -> &'static str {
    match severity {
        "critical" => "high",
        "warn" => "mid",
        _ => "low",
    }
}

/// digest 卡片 suggested_action → inbox suggested actions。
fn digest_action_to_actions(action: &str) -> Vec<String> {
    match action {
        "fix_chunk" | "add_chunk" | "retag" => {
            vec!["open_chat".into(), "dismiss".into()]
        }
        "review_evolution" => vec!["open_chat".into(), "dismiss".into()],
        "dismiss" => vec!["dismiss".into()],
        _ => vec!["open_chat".into(), "dismiss".into()],
    }
}

/// digest 卡片 kind → inbox kind。
fn digest_kind_to_inbox_kind(kind: &str) -> &'static str {
    match kind {
        "chunk_missing_field" => "fill_field",
        "chunk_low_hit_rate" => "repair_chunk",
        "chunk_caused_block" => "repair_chunk",
        "pack_outdated" => "repair_chunk",
        "evolution_pending" => "repair_chunk",
        "evolution_released" => "repair_chunk",
        _ => "repair_chunk",
    }
}

/// 比较两条 inbox 条目，priority 高的在前。
fn priority_rank(p: &str) -> u8 {
    match p {
        "high" => 3,
        "mid" => 2,
        "low" => 1,
        _ => 0,
    }
}

/// pending_review chunk 在 inbox 里的优先级。
///
/// `chunk_type=negative_example` 是 reviewer 误判反馈链路（reaction 写入 outbox
/// 失败文本 → enqueue_negative_example_chunk）的 admin 二次确认入口，必须高优；
/// 其它类型 (peer_case / product_fact / style_template) 维持 mid，避免淹没。
fn inbox_pending_review_priority(chunk_type: &str) -> &'static str {
    if chunk_type == "negative_example" {
        "high"
    } else {
        "mid"
    }
}

pub(in crate::routes) async fn knowledge_inbox(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<InboxQuery>,
) -> AppResult<Json<InboxResponse>> {
    let account_id = query
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let limit_cap = query.limit.unwrap_or(24).clamp(1, 100) as usize;
    let priority_filter = query.priority.as_deref();

    let mut items: Vec<InboxCardView> = Vec::new();

    // 1) digest_card: 当日 KnowledgeDailyReport.cards 未 dismiss。
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    if let Some(report) = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?
    {
        let dismissed: std::collections::HashSet<String> = report
            .dismissed_card_ids
            .iter()
            .map(|oid| oid.to_hex())
            .collect();
        for card in &report.cards {
            let card_id_hex = card.card_id.to_hex();
            if dismissed.contains(&card_id_hex) {
                continue;
            }
            // 提取 target chunk / pack id（如果 target_refs 里有）。
            let mut target_chunk: Option<String> = None;
            let mut target_pack: Option<String> = None;
            for r in &card.target_refs {
                let kind = r.get_str("kind").unwrap_or("");
                let id = r.get_str("id").unwrap_or("");
                if id.is_empty() {
                    continue;
                }
                match kind {
                    "chunk" => {
                        if target_chunk.is_none() {
                            target_chunk = Some(id.to_string());
                        }
                    }
                    "pack" | "item" => {
                        if target_pack.is_none() {
                            target_pack = Some(id.to_string());
                        }
                    }
                    _ => {}
                }
            }
            items.push(InboxCardView {
                id: format!("digest:{}", card_id_hex),
                priority: severity_to_priority(&card.severity).to_string(),
                kind: digest_kind_to_inbox_kind(&card.kind).to_string(),
                title: card.title.clone(),
                context_summary: card.summary.clone(),
                target_chunk_id: target_chunk,
                target_pack_id: target_pack,
                suggested_actions: digest_action_to_actions(&card.suggested_action),
                origin: "digest_card".into(),
                created_at: crate::models::dt_to_string(report.generated_at).unwrap_or_default(),
            });
        }
    }

    // 2/3/4) 三类来源都从 operation_knowledge_chunks 拉。统一拉一次，逐条分类。
    let chunks_filter = doc! {
        "workspace_id": &admin.current_workspace,
        "$or": [
            { "account_id": null },
            { "account_id": { "$exists": false } },
            { "account_id": &account_id },
        ],
        "status": { "$in": ["active", "draft"] },
    };
    let chunks_cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            chunks_filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(200_i64)
                .build(),
        )
        .await?;
    let chunks: Vec<OperationKnowledgeChunk> = chunks_cursor.try_collect().await?;

    let cutoff_ms = (chrono::Utc::now()
        - chrono::Duration::days(7))
    .timestamp_millis();

    for c in &chunks {
        let chunk_id_hex = match &c.id {
            Some(oid) => oid.to_hex(),
            None => continue,
        };
        let title = if c.title.trim().is_empty() {
            chunk_id_hex.clone()
        } else {
            c.title.clone()
        };
        let quote = c.source_quote.clone().unwrap_or_default();
        let has_quote = !quote.trim().is_empty();
        let has_anchor = !c.source_anchors.is_empty();
        let integrity = c.integrity_status.clone().unwrap_or_default();
        let updated_ms = c.updated_at.timestamp_millis();

        // 4) pending_review：integrity_status = needs_review 且 7d 内更新。
        // chunk_type=negative_example 升 priority=high 并标 origin=negative_example_review，
        // 因为这是 reviewer 误判反馈链路（reaction → enqueue_negative_example_chunk）的
        // admin 必须二次确认入口；其它类型 (peer_case / product_fact / style_template)
        // 维持 mid + pending_review。
        if integrity == "needs_review" && updated_ms >= cutoff_ms {
            let is_negative_example = c.chunk_type == "negative_example";
            items.push(InboxCardView {
                id: format!("chunk:{}:review", chunk_id_hex),
                priority: inbox_pending_review_priority(&c.chunk_type).into(),
                kind: "repair_chunk".into(),
                title: if is_negative_example {
                    format!("待审反例：{}", title)
                } else {
                    format!("待审切片：{}", title)
                },
                context_summary: c
                    .summary
                    .clone()
                    .unwrap_or_else(|| {
                        if is_negative_example {
                            "AI 从 reviewer 误判信号入队，等运营 admin 二次确认。".into()
                        } else {
                            "AI 起草，等运营确认。".into()
                        }
                    }),
                target_chunk_id: Some(chunk_id_hex.clone()),
                target_pack_id: None,
                suggested_actions: vec!["open_chat".into(), "open_repair".into(), "dismiss".into()],
                origin: if is_negative_example {
                    "negative_example_review".into()
                } else {
                    "pending_review".into()
                },
                created_at: crate::models::dt_to_string(c.updated_at).unwrap_or_default(),
            });
        }

        // 2) quote_missing：active 且无 source_quote。
        if c.status == "active" && !has_quote {
            items.push(InboxCardView {
                id: format!("chunk:{}:quote", chunk_id_hex),
                priority: "high".into(),
                kind: "fill_field".into(),
                title: format!("补原文出处：{}", title),
                context_summary: "AI 检测到该切片缺 sourceQuote，无法通过验证。".into(),
                target_chunk_id: Some(chunk_id_hex.clone()),
                target_pack_id: None,
                suggested_actions: vec!["open_chat".into(), "open_repair".into()],
                origin: "quote_missing".into(),
                created_at: crate::models::dt_to_string(c.updated_at).unwrap_or_default(),
            });
        }

        // 3) anchors_missing：active 且无 source_anchors（即便有 quote 也算）。
        if c.status == "active" && !has_anchor {
            items.push(InboxCardView {
                id: format!("chunk:{}:anchor", chunk_id_hex),
                priority: "high".into(),
                kind: "repair_chunk".into(),
                title: format!("修复原文锚点：{}", title),
                context_summary: "AI 检测到该切片 sourceAnchors 为空，需要重新锚定。".into(),
                target_chunk_id: Some(chunk_id_hex.clone()),
                target_pack_id: None,
                suggested_actions: vec!["open_chat".into(), "open_repair".into()],
                origin: "anchors_missing".into(),
                created_at: crate::models::dt_to_string(c.updated_at).unwrap_or_default(),
            });
        }
    }

    // 优先级过滤。
    if let Some(p) = priority_filter {
        items.retain(|it| it.priority == p);
    }

    // 排序：priority 降序，再按 origin 顺序保留稳定。
    items.sort_by(|a, b| priority_rank(&b.priority).cmp(&priority_rank(&a.priority)));

    // 截断到 limit。
    if items.len() > limit_cap {
        items.truncate(limit_cap);
    }

    let high = items.iter().filter(|c| c.priority == "high").count();
    let mid = items.iter().filter(|c| c.priority == "mid").count();
    let low = items.iter().filter(|c| c.priority == "low").count();
    let stats = InboxStats {
        total: items.len(),
        high,
        mid,
        low,
    };

    Ok(Json(InboxResponse { items, stats }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AI Inbox 聚合纯函数测试 ────────────────────────────────────────

    /// 不变量：digest 卡片 severity → inbox priority 三档映射稳定。
    /// critical → high；warn → mid；info / 其它 → low。
    #[test]
    fn inbox_severity_to_priority_three_buckets() {
        assert_eq!(severity_to_priority("critical"), "high");
        assert_eq!(severity_to_priority("warn"), "mid");
        assert_eq!(severity_to_priority("info"), "low");
        assert_eq!(severity_to_priority(""), "low");
        assert_eq!(severity_to_priority("garbage"), "low");
    }

    #[test]
    fn inbox_pending_review_priority_lifts_negative_example() {
        // negative_example 是 reviewer 误判反馈链路 admin 二次确认入口，必须高优。
        assert_eq!(inbox_pending_review_priority("negative_example"), "high");
    }

    #[test]
    fn inbox_pending_review_priority_keeps_other_chunk_types_mid() {
        // 其它 chunk_type 维持 mid，避免淹没真正高优的反例审核。
        assert_eq!(inbox_pending_review_priority("product_fact"), "mid");
        assert_eq!(inbox_pending_review_priority("style_template"), "mid");
        assert_eq!(inbox_pending_review_priority("peer_case"), "mid");
        assert_eq!(inbox_pending_review_priority(""), "mid");
        assert_eq!(inbox_pending_review_priority("unknown_future_kind"), "mid");
    }

    /// 不变量：digest 卡 kind → inbox kind 不漏映射任何已声明形态。
    /// 这把封闭枚举绑定在测试上，新加 kind 必须显式更新。
    #[test]
    fn inbox_digest_kind_mapping_is_total_for_known_kinds() {
        assert_eq!(digest_kind_to_inbox_kind("chunk_missing_field"), "fill_field");
        assert_eq!(digest_kind_to_inbox_kind("chunk_low_hit_rate"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("chunk_caused_block"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("pack_outdated"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("evolution_pending"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("evolution_released"), "repair_chunk");
        assert_eq!(digest_kind_to_inbox_kind("freeform"), "repair_chunk");
        // 未知 kind 走 fallback。
        assert_eq!(digest_kind_to_inbox_kind("__unknown__"), "repair_chunk");
    }

    /// 不变量：digest suggested_action → inbox suggestedActions 永远非空，
    /// 且 dismiss 必须存在（运营总能 ✕ 不采纳）。
    #[test]
    fn inbox_action_mapping_always_offers_dismiss() {
        for act in &[
            "fix_chunk",
            "add_chunk",
            "retag",
            "review_evolution",
            "dismiss",
            "freeform",
            "__unknown__",
        ] {
            let acts = digest_action_to_actions(act);
            assert!(!acts.is_empty(), "action '{act}' produced empty list");
            assert!(
                acts.iter().any(|a| a == "dismiss"),
                "action '{act}' must allow dismiss, got {:?}",
                acts
            );
        }
    }

    /// 不变量：priority_rank 单调降序 high > mid > low > 其它。
    /// 这是 inbox 排序 contract 的核心。
    #[test]
    fn inbox_priority_rank_orders_high_first() {
        assert!(priority_rank("high") > priority_rank("mid"));
        assert!(priority_rank("mid") > priority_rank("low"));
        assert!(priority_rank("low") > priority_rank("__unknown__"));
    }

    /// 不变量：sort_by(priority_rank) 把 high 排到最前，mid 居中，low 在尾。
    /// 在没有 mongo 的情况下用纯 Vec 验证 inbox 排序行为。
    #[test]
    fn inbox_sort_places_high_priority_first() {
        let mut items: Vec<(&str, &str)> = vec![
            ("c", "low"),
            ("a", "high"),
            ("b", "mid"),
            ("d", "high"),
        ];
        items.sort_by(|x, y| priority_rank(y.1).cmp(&priority_rank(x.1)));
        let priorities: Vec<&str> = items.iter().map(|(_, p)| *p).collect();
        assert_eq!(priorities, vec!["high", "high", "mid", "low"]);
    }

    /// 文案防御：inbox 路径输出文案不应携带禁词。
    /// 当前涉及到的硬编码标题前缀与 contextSummary 模板都在这里集中校验。
    #[test]
    fn inbox_static_strings_have_no_forbidden_words() {
        let cn1: String = ['人', '工', '接', '管'].iter().collect();
        let cn2: String = ['人', '工', '介', '入'].iter().collect();
        let en1: String = ['t', 'a', 'k', 'e', 'o', 'v', 'e', 'r'].iter().collect();
        let en2: String = ['h', 'a', 'n', 'd', '-', 'o', 'f', 'f'].iter().collect();
        let forbidden = [cn1, cn2, en1, en2];
        let candidates = [
            "待审切片：",
            "AI 起草，等运营确认。",
            "补原文出处：",
            "AI 检测到该切片缺 sourceQuote，无法通过验证。",
            "修复原文锚点：",
            "AI 检测到该切片 sourceAnchors 为空，需要重新锚定。",
        ];
        for s in &candidates {
            for w in &forbidden {
                assert!(
                    !s.contains(w.as_str()),
                    "inbox copy '{s}' contains forbidden '{w}'"
                );
            }
        }
    }
}

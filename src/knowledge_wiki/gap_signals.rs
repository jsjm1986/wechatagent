//! `gap_signals` —— 知识库待办信号生成与两阶段消解。
//!
//! 借鉴 LLW `lint.ts` 与 `sweep-reviews.ts` 的论点：
//! 把"知识库的健康度问题"显式落库为 `knowledge_gap_signals`，让运营像处理工单
//! 一样推进；structural 信号靠纯查询发现，semantic 信号交给 LLM batch 判定，
//! 过期的信号靠两阶段 sweep 自动消解。
//!
//! 本模块只**生成 + 消解**信号，不改 chunk；所有 chunk 编辑仍走
//! [`crate::knowledge_wiki::chunk_revisions::apply_chunk_revision`]。
//!
//! 8 类 signal kind:
//! - `orphan` — chunk 既无入链也无 30d 命中，疑似死页
//! - `broken_link` — chunk.related_chunks 指向已 archived/不存在的 chunk
//! - `no_outlinks` — synthesis/comparison/methodology 类 chunk 的 related_chunks 为空
//! - `low_confidence` — `dynamic_confidence < 0.3` 且 30d hit > 0（命中却低分）
//! - `stale` — `valid_to < now`，时效已过
//! - `contradiction` — semantic LLM 标记的双 chunk 矛盾
//! - `missing_chunk` — semantic LLM 推测应存在但缺失的页
//! - `suggestion` — semantic LLM 给出的改进建议
//!
//! 两阶段 sweep（[`sweep_stale_signals`]）：
//! - **Stage 1（规则）**：broken_link 的 target 已恢复 → auto_resolved；
//!   missing_chunk 的标题已存在 chunk → auto_resolved；
//!   stale 的 valid_to 已被推到未来 → auto_resolved。
//! - **Stage 2（LLM batch）**：剩余 pending 切批 40，最多 5 批询问 LLM
//!   "是否仍适用"，明确 resolved 的标 llm_resolved。**本轮 stage 2 暂只
//!   预留接口**，feedback worker 调用入口走 stage 1，保留后续在不破坏 baseline 的
//!   情况下接 LLM。

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use uuid::Uuid;

use crate::db::Database;
use crate::error::AppError;
use crate::models::{KnowledgeGapSignal, OperationKnowledgeChunk};

/// chunk 在 `wiki_type` 是这三类时，related_chunks 为空 → 标 no_outlinks。
const OUTLINK_REQUIRED_TYPES: &[&str] = &["synthesis", "comparison", "methodology"];

/// dynamic_confidence 低于此值且 30d hit > 0 时标 low_confidence。
const LOW_CONFIDENCE_THRESHOLD: f64 = 0.3;

/// Structural lint 单轮报告：返回新增 / 已存在 pending / 自动消解条数。
#[derive(Debug, Default, Clone)]
pub struct LintReport {
    pub new_signals: i64,
    pub existing_pending: i64,
    pub stage1_auto_resolved: i64,
}

/// 取 workspace 下所有 active（非 archived）chunk —— structural lint 与 sweep 共用。
pub async fn load_active_chunks(
    db: &Database,
    workspace_id: &str,
) -> Result<Vec<OperationKnowledgeChunk>, AppError> {
    let cursor = db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "status": { "$ne": "archived" }
            },
            None,
        )
        .await
        .map_err(AppError::from)?;
    cursor.try_collect().await.map_err(AppError::from)
}

/// 执行一次 structural lint：纯规则、不调 LLM。
///
/// 流程：
/// 1. 拉所有 active chunk 一次；
/// 2. 内存里建 `chunk_id -> chunk` 索引、`chunk_id -> incoming_count` 索引；
/// 3. 按 4 类规则生成候选 signal；
/// 4. 按 `(workspace_id, kind, normalized_title)` 去重写 `knowledge_gap_signals`。
pub async fn run_structural_lint(
    db: &Database,
    workspace_id: &str,
) -> Result<LintReport, AppError> {
    let chunks = load_active_chunks(db, workspace_id).await?;
    let candidates = compute_structural_candidates(&chunks, DateTime::now());
    persist_signals(db, workspace_id, candidates).await
}

/// 纯函数：从一组 active chunk 生成 structural lint 的候选 signal。
///
/// 拆出来便于 PBT / 单测 —— 不需要数据库。
pub fn compute_structural_candidates(
    chunks: &[OperationKnowledgeChunk],
    now: DateTime,
) -> Vec<GapSignalCandidate> {
    let known_ids: HashSet<String> = chunks
        .iter()
        .filter_map(|c| c.id.as_ref().map(|o| o.to_hex()))
        .collect();

    let mut incoming: HashMap<String, i64> = HashMap::new();
    for c in chunks {
        let Some(refs) = c.related_chunks.as_ref() else {
            continue;
        };
        for r in refs {
            *incoming.entry(r.chunk_id.clone()).or_default() += 1;
        }
    }

    let mut out: Vec<GapSignalCandidate> = Vec::new();

    for c in chunks {
        let chunk_id = id_str(c);
        let title = c.title.clone();

        // 1. orphan: 既没有入链，30d 也没命中
        let inbound = incoming.get(&chunk_id).copied().unwrap_or(0);
        let hits = c
            .usage_stats
            .as_ref()
            .map(|s| s.hit_count_30d as i64)
            .unwrap_or(0);
        if inbound == 0 && hits == 0 {
            out.push(GapSignalCandidate::new(
                "orphan",
                format!("孤立 chunk：{}", title),
                "info",
                vec![chunk_id.clone()],
                Some("无入链且 30 天内无命中，可考虑补出处或归档"),
            ));
        }

        // 2. broken_link
        if let Some(refs) = c.related_chunks.as_ref() {
            for r in refs {
                if !known_ids.contains(&r.chunk_id) {
                    out.push(GapSignalCandidate::new(
                        "broken_link",
                        format!("断链：{} → {}", title, r.chunk_id),
                        "warning",
                        vec![chunk_id.clone(), r.chunk_id.clone()],
                        Some("目标 chunk 已不存在或已 archived"),
                    ));
                }
            }
        }

        // 3. no_outlinks
        let need_outlinks = c
            .wiki_type
            .as_deref()
            .map(|t| OUTLINK_REQUIRED_TYPES.contains(&t))
            .unwrap_or(false);
        let has_outlinks = c
            .related_chunks
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if need_outlinks && !has_outlinks {
            out.push(GapSignalCandidate::new(
                "no_outlinks",
                format!("综合类页缺出链：{}", title),
                "info",
                vec![chunk_id.clone()],
                Some("综合/对比/方法类页应交叉引用相关 chunk"),
            ));
        }

        // 4. low_confidence
        if let Some(score) = c.dynamic_confidence {
            if score < LOW_CONFIDENCE_THRESHOLD && hits > 0 {
                out.push(GapSignalCandidate::new(
                    "low_confidence",
                    format!("低分 chunk：{}", title),
                    "warning",
                    vec![chunk_id.clone()],
                    Some(format!("dynamic_confidence={:.2} 但 30d 仍被命中", score)),
                ));
            }
        }

        // 5. stale: valid_to 已过
        if let Some(valid_to) = c.valid_to {
            if valid_to.timestamp_millis() < now.timestamp_millis() {
                out.push(GapSignalCandidate::new(
                    "stale",
                    format!("时效已过：{}", title),
                    "warning",
                    vec![chunk_id.clone()],
                    Some("valid_to 已过期，需要确认是否更新或归档"),
                ));
            }
        }
    }

    out
}

/// 单条候选信号。`kind/title/severity` 都已根据规则确定，写库时仅做去重。
#[derive(Debug, Clone)]
pub struct GapSignalCandidate {
    pub kind: String,
    pub title: String,
    pub severity: String,
    pub affected_chunk_ids: Vec<String>,
    pub description: String,
}

impl GapSignalCandidate {
    fn new(
        kind: &str,
        title: String,
        severity: &str,
        affected: Vec<String>,
        desc: Option<impl Into<String>>,
    ) -> Self {
        Self {
            kind: kind.into(),
            title,
            severity: severity.into(),
            affected_chunk_ids: affected,
            description: desc.map(Into::into).unwrap_or_default(),
        }
    }

    /// `(workspace_id, kind, normalized_title)` 是去重键。
    pub fn dedup_key(&self) -> String {
        format!("{}::{}", self.kind, normalize_title(&self.title))
    }
}

/// 把候选信号写库（去重 + stage 1 sweep + 落 pending）。
///
/// 流程：
/// 1. 加载所有当前 pending 的 signal；
/// 2. 用 dedup_key 比对：候选已存在 → 跳过 / merge affected_chunk_ids；
/// 3. 当前 pending 但候选里没有 → stage 1 auto_resolved；
/// 4. 候选里有但 pending 没有 → 新建。
pub async fn persist_signals(
    db: &Database,
    workspace_id: &str,
    candidates: Vec<GapSignalCandidate>,
) -> Result<LintReport, AppError> {
    let pending_cursor = db
        .knowledge_gap_signals()
        .find(
            doc! { "workspace_id": workspace_id, "status": "pending" },
            None,
        )
        .await
        .map_err(AppError::from)?;
    let pending: Vec<KnowledgeGapSignal> = pending_cursor.try_collect().await.map_err(AppError::from)?;

    let mut pending_by_key: HashMap<String, KnowledgeGapSignal> = HashMap::new();
    for s in pending {
        let key = format!("{}::{}", s.kind, normalize_title(&s.title));
        pending_by_key.insert(key, s);
    }

    let mut report = LintReport::default();
    let mut seen_keys: HashSet<String> = HashSet::new();

    for cand in candidates {
        let key = cand.dedup_key();
        seen_keys.insert(key.clone());
        if let Some(existing) = pending_by_key.get(&key) {
            // 合并 affected_chunk_ids（如果有新增）
            let mut merged_ids: HashSet<String> =
                existing.affected_chunk_ids.iter().cloned().collect();
            let before = merged_ids.len();
            for id in &cand.affected_chunk_ids {
                merged_ids.insert(id.clone());
            }
            if merged_ids.len() > before {
                let new_vec: Vec<String> = merged_ids.into_iter().collect();
                db.knowledge_gap_signals()
                    .update_one(
                        doc! { "signal_id": &existing.signal_id },
                        doc! { "$set": { "affected_chunk_ids": &new_vec } },
                        None,
                    )
                    .await
                    .map_err(AppError::from)?;
            }
            report.existing_pending += 1;
        } else {
            let signal = KnowledgeGapSignal {
                id: None,
                signal_id: format!("sig_{}", Uuid::new_v4().simple()),
                workspace_id: workspace_id.to_string(),
                kind: cand.kind,
                title: cand.title,
                description: cand.description,
                affected_chunk_ids: cand.affected_chunk_ids,
                search_queries: Vec::new(),
                severity: cand.severity,
                source: "rule".into(),
                status: "pending".into(),
                resolution_note: None,
                created_at: DateTime::now(),
                resolved_at: None,
            };
            db.knowledge_gap_signals()
                .insert_one(&signal, None)
                .await
                .map_err(AppError::from)?;
            report.new_signals += 1;
        }
    }

    // Stage 1 auto-resolve：当前 pending 但候选里已不再生成的 → auto_resolved。
    // 仅 source=rule，避免 LLM 信号被规则误消解。
    for (key, sig) in pending_by_key.iter() {
        if seen_keys.contains(key) {
            continue;
        }
        if sig.source != "rule" {
            continue;
        }
        db.knowledge_gap_signals()
            .update_one(
                doc! { "signal_id": &sig.signal_id, "status": "pending" },
                doc! {
                    "$set": {
                        "status": "auto_resolved",
                        "resolution_note": "rule:no_longer_matches",
                        "resolved_at": DateTime::now(),
                    }
                },
                None,
            )
            .await
            .map_err(AppError::from)?;
        report.stage1_auto_resolved += 1;
    }

    Ok(report)
}

/// 30 天滑窗 hit/blocked 统计回写 `usage_stats`，并按朴素公式写 `dynamic_confidence`。
///
/// 公式（见 design.md §6.2）：
/// ```text
/// base = integrity_score ?? 0.5
/// hit_rate = hit_count_30d / max(1, hit_count_30d + blocked_count_30d)
/// stale_penalty = if valid_to < now { 0.3 } else { 0.0 }
/// dynamic_confidence = clamp(base * 0.6 + hit_rate * 0.4 - stale_penalty, 0.0, 1.0)
/// ```
///
/// 单次扫 workspace：bulk update —— 当前用单条 update 的 N 次循环（chunk 总量级假设 < 5000）。
pub async fn refresh_usage_stats_and_confidence(
    db: &Database,
    workspace_id: &str,
) -> Result<UsageStatsReport, AppError> {
    use mongodb::bson::Bson;
    let now = DateTime::now();
    let window_start_ms = now.timestamp_millis() - 30 * 24 * 60 * 60 * 1000;
    let window_start = DateTime::from_millis(window_start_ms);

    let cursor = db
        .knowledge_usage_logs()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .map_err(AppError::from)?;
    let logs: Vec<crate::models::KnowledgeUsageLog> =
        cursor.try_collect().await.map_err(AppError::from)?;

    let mut hit: HashMap<String, u32> = HashMap::new();
    let mut blocked: HashMap<String, u32> = HashMap::new();
    let mut last_used: HashMap<String, DateTime> = HashMap::new();
    let mut last_block_reason: HashMap<String, String> = HashMap::new();

    for log in logs {
        for oid in &log.knowledge_ids {
            let key = oid.to_hex();
            if log.review_approved {
                *hit.entry(key.clone()).or_default() += 1;
            } else {
                *blocked.entry(key.clone()).or_default() += 1;
                if let Some(reason) = log.blocked_reason.clone() {
                    last_block_reason.insert(key.clone(), reason);
                }
            }
            let entry = last_used.entry(key).or_insert(log.created_at);
            if log.created_at.timestamp_millis() > entry.timestamp_millis() {
                *entry = log.created_at;
            }
        }
    }

    let chunks = load_active_chunks(db, workspace_id).await?;
    let mut report = UsageStatsReport::default();

    for c in &chunks {
        let Some(oid) = c.id.as_ref() else { continue };
        let key = oid.to_hex();
        let h = *hit.get(&key).unwrap_or(&0);
        let b = *blocked.get(&key).unwrap_or(&0);
        let last_used_at = last_used.get(&key).copied();
        let last_blocked_reason = last_block_reason.get(&key).cloned();

        let base = c.integrity_score.unwrap_or(0.5);
        let hit_rate = if h + b == 0 {
            0.0
        } else {
            h as f64 / (h as f64 + b as f64)
        };
        let stale_penalty = match c.valid_to {
            Some(vt) if vt.timestamp_millis() < now.timestamp_millis() => 0.3,
            _ => 0.0,
        };
        let dyn_conf = (base * 0.6 + hit_rate * 0.4 - stale_penalty).clamp(0.0, 1.0);

        let mut set: Document = doc! {
            "usage_stats": {
                "hit_count_30d": h as i64,
                "blocked_count_30d": b as i64,
                "last_used_at": last_used_at.map(Bson::DateTime).unwrap_or(Bson::Null),
                "last_blocked_reason": last_blocked_reason.map(Bson::String).unwrap_or(Bson::Null),
            },
            "dynamic_confidence": dyn_conf,
            "updated_at": now,
        };
        // 显式去掉 None 字段以保持 doc 干净（mongo 写 null 也合法，这里二选一）
        if last_used_at.is_none() {
            set.insert("usage_stats.last_used_at", Bson::Null);
        }
        db.operation_knowledge_chunks()
            .update_one(
                doc! { "_id": oid },
                doc! { "$set": &set },
                None,
            )
            .await
            .map_err(AppError::from)?;
        report.updated += 1;
    }

    Ok(report)
}

/// `refresh_usage_stats_and_confidence` 的输出统计。
#[derive(Debug, Default, Clone)]
pub struct UsageStatsReport {
    pub updated: i64,
}

/// chunk 命中实时回写 hook —— 由 `agent::knowledge_router::tool_loop` 调用。
///
/// 关键约束：
/// - **fire-and-forget**：调用方只 `let _ = ...`，不阻塞 tool-loop；
/// - **隔离红线**：本函数只读 db，不引用 gateway/outbox/mcp；
/// - 仅做 `$inc` + `$set last_used_at`，不算 dynamic_confidence（让 worker 周期算，
///   避免热路径上做浮点运算）。
pub async fn record_chunk_hit(
    db: &Database,
    chunk_id_hex: &str,
    blocked: bool,
    reason: Option<&str>,
) -> Result<(), AppError> {
    let oid = match mongodb::bson::oid::ObjectId::parse_str(chunk_id_hex) {
        Ok(o) => o,
        Err(_) => return Ok(()),
    };
    let now = DateTime::now();
    let inc_field = if blocked {
        "usage_stats.blocked_count_30d"
    } else {
        "usage_stats.hit_count_30d"
    };
    let mut set_doc = doc! { "usage_stats.last_used_at": now };
    if blocked {
        if let Some(r) = reason {
            set_doc.insert("usage_stats.last_blocked_reason", r);
        }
    }
    db.operation_knowledge_chunks()
        .update_one(
            doc! { "_id": oid },
            doc! { "$inc": { inc_field: 1i64 }, "$set": set_doc },
            None,
        )
        .await
        .map_err(AppError::from)?;
    Ok(())
}

/// Stage 1 sweep（规则消解）：扫 pending 信号，按 kind 验证条件是否仍成立。
///
/// 与 `persist_signals` 的差异：
/// - `persist_signals` 在 lint 一轮内完成"新规则未生成 → auto_resolved"；
/// - `sweep_stale_signals` 是反向兜底——即使 lint 没跑（如 LLM 信号），
///   也能基于当前数据消解明显已修复的 broken_link / missing_chunk / stale。
pub async fn sweep_stale_signals(
    db: &Database,
    workspace_id: &str,
) -> Result<SweepReport, AppError> {
    let chunks = load_active_chunks(db, workspace_id).await?;
    let known_ids: HashSet<String> = chunks
        .iter()
        .filter_map(|c| c.id.as_ref().map(|o| o.to_hex()))
        .collect();
    let known_titles: HashSet<String> = chunks
        .iter()
        .map(|c| normalize_title(&c.title))
        .collect();

    let now = DateTime::now();
    let pending_cursor = db
        .knowledge_gap_signals()
        .find(
            doc! { "workspace_id": workspace_id, "status": "pending" },
            None,
        )
        .await
        .map_err(AppError::from)?;
    let pending: Vec<KnowledgeGapSignal> =
        pending_cursor.try_collect().await.map_err(AppError::from)?;

    let mut report = SweepReport::default();
    for sig in pending {
        let resolved = match sig.kind.as_str() {
            "broken_link" => {
                // 末位 affected_chunk_ids 是 target id；目标恢复即视为修复
                sig.affected_chunk_ids
                    .last()
                    .map(|t| known_ids.contains(t))
                    .unwrap_or(false)
            }
            "missing_chunk" => known_titles.contains(&normalize_title(&sig.title)),
            "stale" => {
                // 找 affected chunk 看 valid_to 是否被推到未来
                if let Some(target) = sig.affected_chunk_ids.first() {
                    chunks
                        .iter()
                        .find(|c| c.id.as_ref().map(|o| o.to_hex()) == Some(target.clone()))
                        .and_then(|c| c.valid_to)
                        .map(|vt| vt.timestamp_millis() >= now.timestamp_millis())
                        .unwrap_or(false)
                } else {
                    false
                }
            }
            _ => false,
        };
        if !resolved {
            continue;
        }
        db.knowledge_gap_signals()
            .update_one(
                doc! { "signal_id": &sig.signal_id, "status": "pending" },
                doc! {
                    "$set": {
                        "status": "auto_resolved",
                        "resolution_note": "rule:condition_recovered",
                        "resolved_at": DateTime::now(),
                    }
                },
                None,
            )
            .await
            .map_err(AppError::from)?;
        report.stage1_auto_resolved += 1;
    }
    Ok(report)
}

/// Sweep 报告。stage 2（LLM）字段预留，不在本轮启用。
#[derive(Debug, Default, Clone)]
pub struct SweepReport {
    pub stage1_auto_resolved: i64,
    pub stage2_llm_resolved: i64,
}

/// 与 LLW `wiki-cleanup.ts` `normalizeKey` 对齐：strip 空白/连字符/下划线 + 小写。
/// 用作 dedup key 与 missing_chunk title 比对。
pub fn normalize_title(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect()
}

fn id_str(c: &OperationKnowledgeChunk) -> String {
    c.id.as_ref().map(|o| o.to_hex()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;

    fn chunk(title: &str, wiki: Option<&str>, related: Vec<(&str, &str)>) -> OperationKnowledgeChunk {
        OperationKnowledgeChunk {
            id: Some(ObjectId::new()),
            workspace_id: "w".into(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: "user_ops".into(),
            knowledge_type: None,
            business_context: None,
            title: title.into(),
            summary: None,
            body: None,
            applicable_scenes: vec![],
            not_applicable_scenes: vec![],
            product_tags: vec![],
            business_topics: vec![],
            source_quote: None,
            source_anchors: vec![],
            integrity_status: None,
            confidence_score: None,
            status: "active".into(),
            priority: 0,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
            wiki_type: wiki.map(String::from),
            domain_attributes: None,
            provenance: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            previous_version_id: None,
            related_chunks: if related.is_empty() {
                None
            } else {
                Some(
                    related
                        .into_iter()
                        .map(|(id, kind)| crate::models::RelatedRef {
                            chunk_id: id.into(),
                            kind: kind.into(),
                            note: None,
                        })
                        .collect(),
                )
            },
            usage_stats: None,
            dynamic_confidence: None,
            integrity_score: None,
            locked_fields: None,
            chunk_type: "product_fact".to_string(),
        }
    }

    #[test]
    fn no_outlinks_only_for_outlink_required_types() {
        let cs = vec![
            chunk("entity 页", Some("entity"), vec![]),
            chunk("方法论页", Some("methodology"), vec![]),
        ];
        let cands = compute_structural_candidates(&cs, DateTime::now());
        let kinds: Vec<&str> = cands.iter().map(|c| c.kind.as_str()).collect();
        assert!(kinds.contains(&"no_outlinks"));
        // entity 页只该出 orphan，不该出 no_outlinks
        let orphan_count = cands.iter().filter(|c| c.kind == "orphan").count();
        let no_out_count = cands.iter().filter(|c| c.kind == "no_outlinks").count();
        assert_eq!(orphan_count, 2, "两条都没有入链应都是 orphan");
        assert_eq!(no_out_count, 1, "只有 methodology 该有 no_outlinks");
    }

    #[test]
    fn broken_link_detected_when_target_missing() {
        let mut cs = vec![chunk("源页", Some("entity"), vec![("missing_id", "references")])];
        cs[0].usage_stats = Some(crate::models::UsageStats {
            hit_count_30d: 5,
            blocked_count_30d: 0,
            last_used_at: None,
            last_blocked_reason: None,
        });
        let cands = compute_structural_candidates(&cs, DateTime::now());
        assert!(cands.iter().any(|c| c.kind == "broken_link"));
    }

    #[test]
    fn stale_when_valid_to_expired() {
        let mut c = chunk("过期页", Some("entity"), vec![]);
        c.valid_to = Some(DateTime::from_millis(0));
        let cands = compute_structural_candidates(&[c], DateTime::now());
        assert!(cands.iter().any(|s| s.kind == "stale"));
    }

    #[test]
    fn low_confidence_only_when_hit_positive() {
        let mut c = chunk("低分但被命中", Some("entity"), vec![]);
        c.dynamic_confidence = Some(0.1);
        c.usage_stats = Some(crate::models::UsageStats {
            hit_count_30d: 2,
            blocked_count_30d: 0,
            last_used_at: None,
            last_blocked_reason: None,
        });
        let cands = compute_structural_candidates(&[c], DateTime::now());
        assert!(cands.iter().any(|s| s.kind == "low_confidence"));
    }

    #[test]
    fn normalize_title_case_and_punct_invariant() {
        assert_eq!(normalize_title("OpenAI vs Claude"), "openaivsclaude");
        assert_eq!(normalize_title("hello-world_foo bar"), "helloworldfoobar");
        assert_eq!(normalize_title("  Spaces  "), "spaces");
    }

    #[test]
    fn dedup_key_groups_same_kind_and_title() {
        let a = GapSignalCandidate::new(
            "orphan",
            "孤立页 X".into(),
            "info",
            vec!["a".into()],
            None::<&str>,
        );
        let b = GapSignalCandidate::new(
            "orphan",
            "孤立页 X".into(),
            "info",
            vec!["b".into()],
            None::<&str>,
        );
        assert_eq!(a.dedup_key(), b.dedup_key());
    }
}

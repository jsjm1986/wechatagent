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
//! - `broken_link` — chunk.related_chunks 指向不存在的 chunk_id
//! - `no_outlinks` — synthesis/comparison/methodology 类 chunk 的 related_chunks 为空
//! - `low_confidence` — `dynamic_confidence < 0.3` 且 30d hit > 0（命中却低分）
//! - `stale` — `valid_to < now`，时效已过
//! - `contradiction` — 同 workspace 同 normalize_title 多 chunk + body 首段 sha256 不一致
//! - `missing_chunk` — chunk.related_chunks 指向已 archived 的 chunk（依赖被回收）
//! - `suggestion` — `usage_stats.blocked_count_30d > 3 && integrity_status != "verified"`
//!
//! 三个新 kind（`contradiction` / `missing_chunk` / `suggestion`）全部走纯规则路径，
//! 不调用 LLM；source 字段维持 `"rule"`，与 stage 1 sweep 一致。
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

/// 悬空 anchor 的 dynamic_confidence 罚分（方法论点 1 软降格半）：被判悬空的
/// chunk 在离线 dynamic_confidence 重算时额外扣此分，与 stale_penalty 叠加。
/// rank_key 读 dynamic_confidence（confidence_micros），故悬空 chunk 自然排低
/// —— 软降格：只降置信、不剔除、不硬阻断、不动 verified-only。
const DANGLING_ANCHOR_PENALTY: f64 = 0.3;

/// 离线降格半的纯函数：仅当文档原文存在（`raw_content` 传入即代表存在）且
/// `source_quote` 被判悬空时返回罚分，否则 0。强制「查无原文不罚」的软语义由
/// 形参签名兜住（调用方只在 `raw_by_doc` 命中时才传 raw 进来）。
fn dangling_anchor_penalty(quote: Option<&str>, raw_content: &str) -> f64 {
    if anchor_is_dangling(quote, Some(raw_content)) {
        DANGLING_ANCHOR_PENALTY
    } else {
        0.0
    }
}

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

/// 取 workspace 下所有 archived chunk 的 hex id —— missing_chunk 区分用。
///
/// `broken_link` vs `missing_chunk` 的差异：目标 id 在 archived 集合里 → missing_chunk
/// （依赖被运营回收，severity=error）；既不在 active 也不在 archived → broken_link
/// （引用拼错或目标从未存在，severity=warning）。
pub async fn load_archived_chunk_ids(
    db: &Database,
    workspace_id: &str,
) -> Result<HashSet<String>, AppError> {
    let cursor = db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "status": "archived"
            },
            None,
        )
        .await
        .map_err(AppError::from)?;
    let archived: Vec<OperationKnowledgeChunk> = cursor.try_collect().await.map_err(AppError::from)?;
    Ok(archived
        .iter()
        .filter_map(|c| c.id.as_ref().map(|o| o.to_hex()))
        .collect())
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
    let archived_ids = load_archived_chunk_ids(db, workspace_id).await?;
    let raw_by_doc = load_raw_content_by_doc(db, workspace_id).await?;
    let candidates =
        compute_structural_candidates(&chunks, &archived_ids, &raw_by_doc, DateTime::now());
    persist_signals(db, workspace_id, candidates).await
}

/// 取 workspace 下所有文档的 `document_id_hex -> raw_content`，供 `dangling_anchor`
/// 检测把 chunk.source_quote 回原文校验（方法论点 1：anchor 永不悬空）。无 raw_content
/// 的文档不入表 —— 检测端「查无原文则跳过」，保持软语义、不误报。
async fn load_raw_content_by_doc(
    db: &Database,
    workspace_id: &str,
) -> Result<HashMap<String, String>, AppError> {
    let cursor = db
        .operation_knowledge_documents()
        .find(doc! { "workspace_id": workspace_id }, None)
        .await
        .map_err(AppError::from)?;
    let docs: Vec<crate::models::OperationKnowledgeDocument> =
        cursor.try_collect().await.map_err(AppError::from)?;
    let mut out: HashMap<String, String> = HashMap::new();
    for d in docs {
        if let (Some(id), Some(raw)) = (d.id.as_ref(), d.raw_content.as_ref()) {
            if !raw.trim().is_empty() {
                out.insert(id.to_hex(), raw.clone());
            }
        }
    }
    Ok(out)
}

/// 软 anchor 校验（方法论点 1）：判断 chunk 的 `source_quote` 相对其文档 `raw_content`
/// 是否**悬空**（即引用文本已无法在原文中定位）。
///
/// 契约（纯函数、无 IO、PBT 锁）：
/// - `quote` 为 None / 空 / 全空白 → **不算悬空**（没有引用可校验，不打扰）；
/// - 去除两侧所有 Unicode 空白后，`quote` 是 `raw_content` 的子串 → **不悬空**
///   （容忍 PDF 抽取换行/空格差异，降低误报）；
/// - 其余（含 `raw_content` 为 None 而 quote 非空）→ **悬空**。
///
/// 注意：检测端 [`compute_structural_candidates`] 只在 raw_content **存在**时才调用本
/// 函数，故「查无原文」不会被误判为悬空——软语义只在能确证「引用不在原文」时才出信号。
pub fn anchor_is_dangling(quote: Option<&str>, raw_content: Option<&str>) -> bool {
    let q = match quote {
        Some(s) if !s.trim().is_empty() => s,
        _ => return false,
    };
    let raw = match raw_content {
        Some(s) => s,
        None => return true,
    };
    let strip_ws = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    let q_norm = strip_ws(q);
    if q_norm.is_empty() {
        return false;
    }
    !strip_ws(raw).contains(&q_norm)
}

/// 纯函数：从一组 active chunk 生成 structural lint 的候选 signal。
///
/// 拆出来便于 PBT / 单测 —— 不需要数据库。`archived_ids` 用来区分
/// `broken_link`（引用从未存在）与 `missing_chunk`（引用了已 archived 的 chunk）。
/// `raw_by_doc`（document_id_hex -> raw_content）供第 9 类 `dangling_anchor` 把
/// chunk.source_quote 回原文软校验；查无 raw_content 的文档其 chunk 跳过校验。
pub fn compute_structural_candidates(
    chunks: &[OperationKnowledgeChunk],
    archived_ids: &HashSet<String>,
    raw_by_doc: &HashMap<String, String>,
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

        // 2. broken_link / missing_chunk
        //    - 目标既不在 active 也不在 archived → broken_link（warning）
        //    - 目标在 archived → missing_chunk（error，依赖被回收）
        if let Some(refs) = c.related_chunks.as_ref() {
            for r in refs {
                if known_ids.contains(&r.chunk_id) {
                    continue;
                }
                if archived_ids.contains(&r.chunk_id) {
                    out.push(GapSignalCandidate::new(
                        "missing_chunk",
                        format!("依赖已归档：{} → {}", title, r.chunk_id),
                        "error",
                        vec![chunk_id.clone(), r.chunk_id.clone()],
                        Some("引用了已 archived 的 chunk，需要补回或换引用"),
                    ));
                } else {
                    out.push(GapSignalCandidate::new(
                        "broken_link",
                        format!("断链：{} → {}", title, r.chunk_id),
                        "warning",
                        vec![chunk_id.clone(), r.chunk_id.clone()],
                        Some("目标 chunk 不存在；可能是 id 拼错或从未导入"),
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

        // 6. suggestion: 未 verified 且 30d 被 grounding 闸 blocked > 3 次
        let blocked = c
            .usage_stats
            .as_ref()
            .map(|s| s.blocked_count_30d as i64)
            .unwrap_or(0);
        let verified = c.integrity_status.as_deref() == Some("verified");
        if !verified && blocked > 3 {
            out.push(GapSignalCandidate::new(
                "suggestion",
                format!("建议补完后 verify：{}", title),
                "info",
                vec![chunk_id.clone()],
                Some(format!(
                    "30 天累计被 grounding 闸拦截 {blocked} 次，建议补 source_quote 后 verify",
                )),
            ));
        }

        // 9. dangling_anchor: source_quote 已无法在文档 raw_content 中定位
        //    （方法论点 1：anchor 永不悬空）。软语义——只在能拿到原文且确证
        //    引用不在原文时才出信号；查无原文（raw_by_doc 缺该文档）直接跳过，
        //    不误报。rank_key 侧据此降格（不硬阻断 open / cite）。
        if let Some(doc_id) = c.document_id.as_ref().map(|o| o.to_hex()) {
            if let Some(raw) = raw_by_doc.get(&doc_id) {
                if anchor_is_dangling(c.source_quote.as_deref(), Some(raw)) {
                    out.push(GapSignalCandidate::new(
                        "dangling_anchor",
                        format!("引用悬空：{}", title),
                        "warning",
                        vec![chunk_id.clone()],
                        Some("source_quote 已无法在文档原文中定位，需重锚或更新引用"),
                    ));
                }
            }
        }
    }

    // 7. contradiction: 同 normalize_title 的多 chunk + body 首段 sha256 不一致
    //    遍历完单条规则后再跨 chunk 聚合，避免每个 chunk 内重复扫整表。
    let mut by_title: HashMap<String, Vec<(&OperationKnowledgeChunk, String)>> = HashMap::new();
    for c in chunks {
        if let Some(body) = c.body.as_deref() {
            let key = normalize_title(&c.title);
            if key.is_empty() {
                continue;
            }
            let hash = sha256_hex(first_paragraph(body));
            by_title.entry(key).or_default().push((c, hash));
        }
    }
    for (norm_title, members) in by_title {
        if members.len() < 2 {
            continue;
        }
        let unique_hashes: HashSet<&str> = members.iter().map(|(_, h)| h.as_str()).collect();
        if unique_hashes.len() < 2 {
            continue; // 同题但首段一致 → 视为重复，由其它流程处理
        }
        let display_title = members
            .first()
            .map(|(c, _)| c.title.clone())
            .unwrap_or(norm_title);
        let affected: Vec<String> = members
            .iter()
            .filter_map(|(c, _)| c.id.as_ref().map(|o| o.to_hex()))
            .collect();
        out.push(GapSignalCandidate::new(
            "contradiction",
            format!("同题异说：{}", display_title),
            "error",
            affected,
            Some("同题多 chunk 首段不一致，需要确认权威说法或合并"),
        ));
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
    /// 召回缺失场景下的「待人类补全」线索：原始 query + 可选的一句 LLM 生成追问。
    /// 运营据此用对话形式补充知识库，闭环实际业务知识仓库。默认空（结构/链接类
    /// 信号不携带 query）；仅 recall_miss（诚实弃答/查无内容）会写入。
    pub search_queries: Vec<String>,
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
            search_queries: Vec::new(),
            description: desc.map(Into::into).unwrap_or_default(),
        }
    }

    /// 去重键：默认 `(kind, normalized_title)`；但 broken_link / missing_chunk
    /// 是同一对 (from_chunk_id, target_chunk_id) 在 archive↔active 切换下的孪
    /// 生信号，必须把 dedup key 绑到具体的引用对上（用 affected_chunk_ids 前两
    /// 项），并对这两类共享同一前缀 `link::from::to`，确保 archive 切换时不会
    /// 产生重复信号；sweep 阶段也据此双向 resolve。
    pub fn dedup_key(&self) -> String {
        signal_dedup_key(&self.kind, &self.title, &self.affected_chunk_ids)
    }
}

/// 与 [`GapSignalCandidate::dedup_key`] 同语义，可直接作用在已落库的
/// [`KnowledgeGapSignal`] 上 —— `persist_signals` 加载现有 pending 信号时用它
/// 还原 dedup key，与候选信号 1:1 对账。
pub(crate) fn signal_dedup_key(kind: &str, title: &str, affected: &[String]) -> String {
    if matches!(kind, "broken_link" | "missing_chunk") && affected.len() >= 2 {
        return format!("link::{}::{}", affected[0], affected[1]);
    }
    format!("{}::{}", kind, normalize_title(title))
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
        let key = signal_dedup_key(&s.kind, &s.title, &s.affected_chunk_ids);
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

/// 在线召回-trace 闭环（方法论点 5）专用的**只增不消解**落库口。
///
/// 与 [`persist_signals`] 的关键差异：`persist_signals` 跑完一轮 lint 后会把
/// 「本轮规则未再生成」的 pending 信号 stage-1 auto_resolve —— 那是**离线全量**
/// 语义。在线召回钩子每次只携带「本次这一问」的单条候选，**绝不能**触发那套
/// sweep，否则会把离线 lint 攒下的 pending 信号全冲掉。所以这里只做
/// 「dedup 命中则合并 affected / 未命中则 insert」，永不 resolve 任何信号。
///
/// `source="recall_trace"`，与离线 `"rule"` / `"llm"` 区分，便于运营按来源筛、
/// 也避免被离线 sweep 的 `source=="rule"` 过滤误消解。fire-and-forget 调用，
/// 失败只丢一条日志、不影响召回热路径。
pub async fn persist_recall_signal(
    db: &Database,
    workspace_id: &str,
    candidate: GapSignalCandidate,
) -> Result<(), AppError> {
    let key = candidate.dedup_key();
    let existing = db
        .knowledge_gap_signals()
        .find_one(
            doc! { "workspace_id": workspace_id, "status": "pending", "kind": &candidate.kind },
            None,
        )
        .await
        .map_err(AppError::from)?
        .filter(|s| signal_dedup_key(&s.kind, &s.title, &s.affected_chunk_ids) == key);

    if let Some(existing) = existing {
        let mut merged: HashSet<String> = existing.affected_chunk_ids.iter().cloned().collect();
        let before = merged.len();
        for id in &candidate.affected_chunk_ids {
            merged.insert(id.clone());
        }
        // search_queries 并集：同一缺失主题反复弃答时累积所有 query 变体（去重、丢空），
        // 给人类更全的对话补全线索。原 affected_chunk_ids 并集逻辑保持不变。
        let mut queries: Vec<String> = existing.search_queries.clone();
        let q_before = queries.len();
        for q in &candidate.search_queries {
            if !q.trim().is_empty() && !queries.iter().any(|e| e == q) {
                queries.push(q.clone());
            }
        }
        let affected_grew = merged.len() > before;
        let queries_grew = queries.len() > q_before;
        if affected_grew || queries_grew {
            let mut set = doc! {};
            if affected_grew {
                let new_vec: Vec<String> = merged.into_iter().collect();
                set.insert("affected_chunk_ids", new_vec);
            }
            if queries_grew {
                set.insert("search_queries", &queries);
            }
            db.knowledge_gap_signals()
                .update_one(
                    doc! { "signal_id": &existing.signal_id },
                    doc! { "$set": set },
                    None,
                )
                .await
                .map_err(AppError::from)?;
        }
        return Ok(());
    }

    let signal = KnowledgeGapSignal {
        id: None,
        signal_id: format!("sig_{}", Uuid::new_v4().simple()),
        workspace_id: workspace_id.to_string(),
        kind: candidate.kind,
        title: candidate.title,
        description: candidate.description,
        affected_chunk_ids: candidate.affected_chunk_ids,
        search_queries: candidate.search_queries,
        severity: candidate.severity,
        source: "recall_trace".into(),
        status: "pending".into(),
        resolution_note: None,
        created_at: DateTime::now(),
        resolved_at: None,
    };
    db.knowledge_gap_signals()
        .insert_one(&signal, None)
        .await
        .map_err(AppError::from)?;
    Ok(())
}
///
/// 病根（镜厅效应）是 hit 信号取 reviewer 自评（`review_approved`），系统学的是
/// "reviewer 喜欢哪些 chunk"而非"哪些 chunk 让用户正反应"。换血即把信号源从
/// reviewer 自评换成按 `run_id` join 出来的 `AgentDecisionReview.outcome_status`。
///
/// 三态语义（Iron Law ②：沉默 = 删失，绝不当负例）：
/// - `Hit`：用户确有正向反应（购买信号）→ 计入 hit 分子；
/// - `Block`：用户确有负向反应（异议/止/退订/投诉/负面）→ 计入 block；
/// - `Censored`：沉默 / 无反应 / `pending` / 空 / 含义不明 → **删失**，既不进
///   hit 也不进 block —— 分母只含"用户确有明确反应"的样本。
///
/// 负向集合复用 [`crate::agent::is_negative_outcome`]，保持单一真相源。
pub fn classify_outcome_label(outcome_status: Option<&str>) -> OutcomeLabel {
    match outcome_status {
        Some("user_replied_buying_signal") => OutcomeLabel::Hit,
        Some(s) if crate::agent::is_negative_outcome(s) => OutcomeLabel::Block,
        // None / "pending" / "" / user_replied_unclassified / 其它 → 删失（不臆测）。
        _ => OutcomeLabel::Censored,
    }
}

/// [`classify_outcome_label`] 的三态结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeLabel {
    /// 用户确有正向反应 → hit 分子。
    Hit,
    /// 用户确有负向反应 → block。
    Block,
    /// 删失：沉默 / 无反应 / pending / 含义不明 → 不进任何分母。
    Censored,
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
///
/// S7 止血：`min_samples` 控制信 hit_rate 所需的最小样本数（hits+blocks）。
/// 低于此值时 dynamic_confidence 只用 base（详见 [`compute_dynamic_confidence`]）。
///
/// **换血（P1）**：`real_outcome_enabled=true`（默认）时，hit/block 不再取
/// reviewer 自评 `review_approved`，而是按 `run_id` join `AgentDecisionReview`
/// 的真实用户反应 `outcome_status`，经 [`classify_outcome_label`] 三态判定：
/// 正向→hit、负向→block、沉默/pending/无反应→删失排除（不进任何分母）。
/// `false`（回滚）时逐字节退回旧 `review_approved` 逻辑。公式本体不变。
pub async fn refresh_usage_stats_and_confidence(
    db: &Database,
    workspace_id: &str,
    min_samples: u64,
    real_outcome_enabled: bool,
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

    // 换血：按本批 logs 的 run_id 批量拉 decision_reviews，建 run_id → outcome_status map。
    // 关闭（回滚）时跳过这次 join，省一次查询。
    let outcome_by_run: HashMap<String, Option<String>> = if real_outcome_enabled {
        let run_ids: Vec<String> = logs
            .iter()
            .map(|l| l.run_id.clone())
            .filter(|r| !r.is_empty())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if run_ids.is_empty() {
            HashMap::new()
        } else {
            let review_cursor = db
                .decision_reviews()
                .find(doc! { "run_id": { "$in": &run_ids } }, None)
                .await
                .map_err(AppError::from)?;
            let reviews: Vec<crate::models::AgentDecisionReview> =
                review_cursor.try_collect().await.map_err(AppError::from)?;
            let mut map: HashMap<String, Option<String>> = HashMap::new();
            for review in reviews {
                if let Some(rid) = review.run_id {
                    map.insert(rid, review.outcome_status);
                }
            }
            map
        }
    } else {
        HashMap::new()
    };

    let mut hit: HashMap<String, u32> = HashMap::new();
    let mut blocked: HashMap<String, u32> = HashMap::new();
    let mut last_used: HashMap<String, DateTime> = HashMap::new();
    let mut last_block_reason: HashMap<String, String> = HashMap::new();

    for log in logs {
        // 换血：真实用户反应三态；删失（Censored）整条 log 跳过 hit/block 统计，
        // 但仍参与 last_used 记账（chunk 确实被召回过）。回滚时退回 reviewer 自评。
        let label = if real_outcome_enabled {
            let outcome = outcome_by_run
                .get(&log.run_id)
                .and_then(|o| o.as_deref());
            Some(classify_outcome_label(outcome))
        } else {
            None
        };
        for oid in &log.knowledge_ids {
            let key = oid.to_hex();
            match label {
                Some(OutcomeLabel::Hit) => {
                    *hit.entry(key.clone()).or_default() += 1;
                }
                Some(OutcomeLabel::Block) => {
                    *blocked.entry(key.clone()).or_default() += 1;
                    if let Some(reason) = log.blocked_reason.clone() {
                        last_block_reason.insert(key.clone(), reason);
                    }
                }
                Some(OutcomeLabel::Censored) => {
                    // 删失：不计 hit 也不计 block，只走下面的 last_used 记账。
                }
                None => {
                    // 回滚路径：逐字节退回旧 reviewer 自评逻辑。
                    if log.review_approved {
                        *hit.entry(key.clone()).or_default() += 1;
                    } else {
                        *blocked.entry(key.clone()).or_default() += 1;
                        if let Some(reason) = log.blocked_reason.clone() {
                            last_block_reason.insert(key.clone(), reason);
                        }
                    }
                }
            }
            let entry = last_used.entry(key).or_insert(log.created_at);
            if log.created_at.timestamp_millis() > entry.timestamp_millis() {
                *entry = log.created_at;
            }
        }
    }

    let chunks = load_active_chunks(db, workspace_id).await?;
    let raw_by_doc = load_raw_content_by_doc(db, workspace_id).await?;
    let mut report = UsageStatsReport::default();

    for c in &chunks {
        let Some(oid) = c.id.as_ref() else { continue };
        let key = oid.to_hex();
        let h = *hit.get(&key).unwrap_or(&0);
        let b = *blocked.get(&key).unwrap_or(&0);
        let last_used_at = last_used.get(&key).copied();
        let last_blocked_reason = last_block_reason.get(&key).cloned();

        let base = c.integrity_score.unwrap_or(0.5);
        let stale_penalty = match c.valid_to {
            Some(vt) if vt.timestamp_millis() < now.timestamp_millis() => 0.3,
            _ => 0.0,
        };
        // Gap3 软降格半（方法论点 1）：source_quote 在文档原文中悬空 → 叠加罚分。
        // 仅文档有 raw_content 时才校验（查无原文不罚），与离线检测同源。
        let dangling_penalty = c
            .document_id
            .as_ref()
            .map(|o| o.to_hex())
            .and_then(|doc_id| raw_by_doc.get(&doc_id))
            .map(|raw| dangling_anchor_penalty(c.source_quote.as_deref(), raw))
            .unwrap_or(0.0);
        let dyn_conf = compute_dynamic_confidence(
            base,
            h as u64,
            b as u64,
            stale_penalty + dangling_penalty,
            min_samples,
        );

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

/// S7 止血：dynamic_confidence 计算的纯函数（无 IO，可单测）。
///
/// 公式（保持与 design.md §6.2 一致）：
/// ```text
/// hit_rate = h / (h + b)
/// dynamic_confidence = clamp(base * 0.6 + hit_rate * 0.4 - stale_penalty, 0, 1)
/// ```
///
/// **最小样本门**：当 `h + b < min_samples` 时，hit_rate 由太少的样本估计，
/// 噪声极大（1-2 个 reviewer 自评就能把置信度甩到极端）。此时**不信
/// hit_rate**，只用 `clamp(base - stale_penalty, 0, 1)`。
///
/// 明确**不做**贝叶斯收缩（Wilson/Beta）——那属于"换血 hit 信号"阶段；当前
/// hit 仍是 reviewer 自评（镜厅效应），给它上贝叶斯只会让镜厅更精致。最小
/// 样本门只是防止极少样本就把置信度甩飞，不改变信号本身的来源。
pub fn compute_dynamic_confidence(
    base: f64,
    h: u64,
    b: u64,
    stale_penalty: f64,
    min_samples: u64,
) -> f64 {
    let total = h + b;
    if total < min_samples {
        return (base - stale_penalty).clamp(0.0, 1.0);
    }
    // total==0 仅在 min_samples==0（止血门关闭）时能走到这；无样本时 hit_rate
    // 取 0（与旧实现一致），避免 0/0 = NaN。
    let hit_rate = if total == 0 {
        0.0
    } else {
        h as f64 / total as f64
    };
    (base * 0.6 + hit_rate * 0.4 - stale_penalty).clamp(0.0, 1.0)
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
///   也能基于当前数据消解明显已修复的 broken_link / missing_chunk / stale /
///   suggestion / contradiction。每类用独立 `resolution_note`，方便审计追溯
///   信号是被哪条规则消解的。
pub async fn sweep_stale_signals(
    db: &Database,
    workspace_id: &str,
) -> Result<SweepReport, AppError> {
    let chunks = load_active_chunks(db, workspace_id).await?;
    let known_ids: HashSet<String> = chunks
        .iter()
        .filter_map(|c| c.id.as_ref().map(|o| o.to_hex()))
        .collect();

    // 同 normalize_title 在 active 集合中的 (chunk_id_hex, body_first_paragraph_sha256) 视图，
    // 用于 contradiction 自愈检查（首段哈希再次一致或同题只剩一条）。
    let mut title_groups: HashMap<String, Vec<String>> = HashMap::new();
    for c in &chunks {
        if let Some(body) = c.body.as_deref() {
            let key = normalize_title(&c.title);
            if key.is_empty() {
                continue;
            }
            title_groups.entry(key).or_default().push(sha256_hex(first_paragraph(body)));
        }
    }

    // chunk_id_hex -> integrity_status，用于 suggestion 自愈检查（被 verify 即消解）。
    let mut integrity_by_id: HashMap<String, String> = HashMap::new();
    for c in &chunks {
        if let (Some(id), Some(st)) = (c.id.as_ref(), c.integrity_status.as_deref()) {
            integrity_by_id.insert(id.to_hex(), st.to_string());
        }
    }

    // incoming：chunk_id_hex -> 入链次数（来自所有 active chunk 的 related_chunks）。
    // 给 orphan arm 用：当前 30d 命中或恢复入链，即视为不再孤立。
    let mut incoming: HashMap<String, i64> = HashMap::new();
    for c in &chunks {
        if let Some(refs) = c.related_chunks.as_ref() {
            for r in refs {
                *incoming.entry(r.chunk_id.clone()).or_default() += 1;
            }
        }
    }

    // chunk_view：chunk_id_hex -> (wiki_type, has_outlinks, dynamic_confidence, hit_30d)。
    // 给 no_outlinks / low_confidence arm 用，避免每次扫描全 chunk。
    let mut chunk_view: HashMap<String, (Option<String>, bool, Option<f64>, i64)> = HashMap::new();
    for c in &chunks {
        let Some(id) = c.id.as_ref() else { continue };
        let has_outlinks = c
            .related_chunks
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let hits = c
            .usage_stats
            .as_ref()
            .map(|s| s.hit_count_30d as i64)
            .unwrap_or(0);
        chunk_view.insert(
            id.to_hex(),
            (
                c.wiki_type.clone(),
                has_outlinks,
                c.dynamic_confidence,
                hits,
            ),
        );
    }

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
        // (resolved, resolution_note)：resolution_note 落库时区分各 kind 自愈原因。
        let outcome: Option<&'static str> = match sig.kind.as_str() {
            "broken_link" => {
                // affected_chunk_ids = [from_id, target_id]：
                // - target 恢复 active → 链已修复
                // - 引用源 from 自己 archived → 信号无意义（与 missing_chunk arm 对称）
                let from_archived = sig
                    .affected_chunk_ids
                    .first()
                    .map(|src| !known_ids.contains(src))
                    .unwrap_or(false);
                let recovered = sig
                    .affected_chunk_ids
                    .last()
                    .map(|t| known_ids.contains(t))
                    .unwrap_or(false);
                if recovered {
                    Some("rule:link_recovered")
                } else if from_archived {
                    Some("rule:source_archived")
                } else {
                    None
                }
            }
            "missing_chunk" => {
                // 末位是已 archived 的 target id；当它重新进 active → 视为依赖恢复。
                let dep_back = sig
                    .affected_chunk_ids
                    .last()
                    .map(|t| known_ids.contains(t))
                    .unwrap_or(false);
                if dep_back {
                    Some("rule:dep_restored")
                } else if sig.affected_chunk_ids.first().map(|src| !known_ids.contains(src)).unwrap_or(false) {
                    // 引用源 chunk 自己也 archived 了 → 信号无意义。
                    Some("rule:dep_unrelated")
                } else {
                    None
                }
            }
            "stale" => {
                // 找 affected chunk 看 valid_to 是否被推到未来
                let extended = if let Some(target) = sig.affected_chunk_ids.first() {
                    chunks
                        .iter()
                        .find(|c| c.id.as_ref().map(|o| o.to_hex()) == Some(target.clone()))
                        .and_then(|c| c.valid_to)
                        .map(|vt| vt.timestamp_millis() >= now.timestamp_millis())
                        .unwrap_or(false)
                } else {
                    false
                };
                if extended { Some("rule:valid_to_extended") } else { None }
            }
            "suggestion" => {
                // chunk 被 verify → 信号失效。
                let verified = sig
                    .affected_chunk_ids
                    .first()
                    .and_then(|id| integrity_by_id.get(id))
                    .map(|st| st == "verified")
                    .unwrap_or(false);
                if verified { Some("rule:chunk_verified") } else { None }
            }
            "contradiction" => {
                // 同题只剩 1 条，或同题首段哈希已收敛到一致 → 视为冲突已解决。
                let key = normalize_title(&sig.title.replace("同题异说：", ""));
                let resolved = match title_groups.get(&key) {
                    None => true,
                    Some(hashes) if hashes.len() < 2 => true,
                    Some(hashes) => {
                        let unique: HashSet<&str> = hashes.iter().map(|s| s.as_str()).collect();
                        unique.len() < 2
                    }
                };
                if resolved { Some("rule:contradiction_resolved") } else { None }
            }
            "orphan" => {
                // chunk 拿到入链或 30d 命中，或自身已 archived → 信号失效。
                if let Some(target) = sig.affected_chunk_ids.first() {
                    if !known_ids.contains(target) {
                        Some("rule:chunk_archived")
                    } else if incoming.get(target).copied().unwrap_or(0) > 0 {
                        Some("rule:incoming_restored")
                    } else if chunk_view.get(target).map(|v| v.3 > 0).unwrap_or(false) {
                        Some("rule:hits_restored")
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            "no_outlinks" => {
                // chunk 已补出链 / 改了 wiki_type 不再要求出链 / 自身 archived → 失效。
                if let Some(target) = sig.affected_chunk_ids.first() {
                    if !known_ids.contains(target) {
                        Some("rule:chunk_archived")
                    } else if let Some((wt, has_outlinks, _, _)) = chunk_view.get(target) {
                        let still_required = wt
                            .as_deref()
                            .map(|t| OUTLINK_REQUIRED_TYPES.contains(&t))
                            .unwrap_or(false);
                        if !still_required {
                            Some("rule:type_changed")
                        } else if *has_outlinks {
                            Some("rule:outlinks_added")
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            "low_confidence" => {
                // dynamic_confidence 已回到阈值之上 / 30d 命中归零 / 自身 archived → 失效。
                if let Some(target) = sig.affected_chunk_ids.first() {
                    if !known_ids.contains(target) {
                        Some("rule:chunk_archived")
                    } else if let Some((_, _, conf, hits)) = chunk_view.get(target) {
                        let lifted = conf
                            .map(|s| s >= LOW_CONFIDENCE_THRESHOLD)
                            .unwrap_or(false);
                        if lifted {
                            Some("rule:confidence_lifted")
                        } else if *hits == 0 {
                            Some("rule:no_recent_hits")
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(note) = outcome else { continue };
        db.knowledge_gap_signals()
            .update_one(
                doc! { "signal_id": &sig.signal_id, "status": "pending" },
                doc! {
                    "$set": {
                        "status": "auto_resolved",
                        "resolution_note": note,
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

/// 取 body 的"第一段"——以双换行为分隔。空 body / 全空白 body 返回空串。
/// 用作 contradiction 检测：同题多 chunk 首段哈希不同 → 视为冲突。
pub fn first_paragraph(body: &str) -> &str {
    let trimmed = body.trim_start();
    let end = trimmed.find("\n\n").unwrap_or(trimmed.len());
    trimmed[..end].trim_end()
}

/// SHA-256 hex digest，仅用于 contradiction 等同性比较（非密码学用途）。
pub fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(s.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn id_str(c: &OperationKnowledgeChunk) -> String {
    c.id.as_ref().map(|o| o.to_hex()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;

    /// S7：样本不足（h+b < min_samples）→ 只用 base，不被少量 hit 甩飞。
    #[test]
    fn dyn_conf_below_min_samples_uses_base_only() {
        // base=0.5，全是 hit（h=2,b=0），若信 hit_rate 会被推到 0.7；
        // 但样本不足 5 → 必须只用 base=0.5。
        let v = compute_dynamic_confidence(0.5, 2, 0, 0.0, 5);
        assert!((v - 0.5).abs() < 1e-9, "got {v}");
    }

    /// S7：样本充足 → 走加权公式 base*0.6 + hit_rate*0.4 - stale。
    #[test]
    fn dyn_conf_at_or_above_min_samples_uses_weighted() {
        // h=4,b=1 → hit_rate=0.8；base=0.5 → 0.5*0.6 + 0.8*0.4 = 0.62。
        let v = compute_dynamic_confidence(0.5, 4, 1, 0.0, 5);
        assert!((v - 0.62).abs() < 1e-9, "got {v}");
    }

    /// S7：边界——恰好等于 min_samples 即视为充足（>=）。
    #[test]
    fn dyn_conf_exactly_min_samples_is_sufficient() {
        // h+b == 5 == min_samples → 走加权分支。
        let weighted = compute_dynamic_confidence(0.5, 5, 0, 0.0, 5);
        // 全 hit → hit_rate=1.0 → 0.5*0.6 + 1.0*0.4 = 0.7。
        assert!((weighted - 0.7).abs() < 1e-9, "got {weighted}");
    }

    /// S7：样本不足时 stale_penalty 仍生效（从 base 扣）。
    #[test]
    fn dyn_conf_below_min_samples_still_applies_stale_penalty() {
        let v = compute_dynamic_confidence(0.5, 1, 0, 0.3, 5);
        assert!((v - 0.2).abs() < 1e-9, "got {v}");
    }

    /// S7：clamp 下界——base 极低 + stale 扣到负 → 夹到 0。
    #[test]
    fn dyn_conf_clamps_to_zero() {
        let v = compute_dynamic_confidence(0.1, 0, 0, 0.3, 5);
        assert_eq!(v, 0.0);
    }

    /// S7：clamp 上界——加权结果超过 1 → 夹到 1。
    #[test]
    fn dyn_conf_clamps_to_one() {
        // base=1.0 全 hit → 1.0*0.6 + 1.0*0.4 = 1.0；再给个负 stale 试图超界。
        let v = compute_dynamic_confidence(1.0, 10, 0, -0.5, 5);
        assert_eq!(v, 1.0);
    }

    /// S7：min_samples=0 → 永远走加权（等价关闭止血门），保留旧行为可回退。
    #[test]
    fn dyn_conf_min_samples_zero_always_weighted() {
        // h=0,b=0,total=0 >= 0 → 走加权，hit_rate=0 → base*0.6。
        let v = compute_dynamic_confidence(0.5, 0, 0, 0.0, 0);
        assert!((v - 0.3).abs() < 1e-9, "got {v}");
    }

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
        let cands = compute_structural_candidates(&cs, &HashSet::new(), &HashMap::new(), DateTime::now());
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
        let cands = compute_structural_candidates(&cs, &HashSet::new(), &HashMap::new(), DateTime::now());
        assert!(cands.iter().any(|c| c.kind == "broken_link"));
    }

    #[test]
    fn stale_when_valid_to_expired() {
        let mut c = chunk("过期页", Some("entity"), vec![]);
        c.valid_to = Some(DateTime::from_millis(0));
        let cands = compute_structural_candidates(&[c], &HashSet::new(), &HashMap::new(), DateTime::now());
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
        let cands = compute_structural_candidates(&[c], &HashSet::new(), &HashMap::new(), DateTime::now());
        assert!(cands.iter().any(|s| s.kind == "low_confidence"));
    }

    #[test]
    fn missing_chunk_when_target_archived() {
        // 引用 target 在 archived 集合里 → 应出 missing_chunk 而不是 broken_link
        let cs = vec![chunk(
            "源页",
            Some("entity"),
            vec![("archived_target_id", "references")],
        )];
        let mut archived = HashSet::new();
        archived.insert("archived_target_id".to_string());
        let cands = compute_structural_candidates(&cs, &archived, &HashMap::new(), DateTime::now());
        assert!(cands.iter().any(|c| c.kind == "missing_chunk"));
        assert!(
            !cands.iter().any(|c| c.kind == "broken_link"),
            "目标在 archived 时不该再出 broken_link"
        );
    }

    #[test]
    fn suggestion_when_unverified_and_blocked_repeatedly() {
        let mut c = chunk("常被拦的草稿", Some("entity"), vec![]);
        c.integrity_status = Some("needs_review".to_string());
        c.usage_stats = Some(crate::models::UsageStats {
            hit_count_30d: 1,
            blocked_count_30d: 5,
            last_used_at: None,
            last_blocked_reason: None,
        });
        let cands = compute_structural_candidates(&[c], &HashSet::new(), &HashMap::new(), DateTime::now());
        assert!(cands.iter().any(|s| s.kind == "suggestion"));

        // 一旦 verified，suggestion 不应再出现
        let mut c2 = chunk("常被拦的草稿", Some("entity"), vec![]);
        c2.integrity_status = Some("verified".to_string());
        c2.usage_stats = Some(crate::models::UsageStats {
            hit_count_30d: 1,
            blocked_count_30d: 5,
            last_used_at: None,
            last_blocked_reason: None,
        });
        let cands2 = compute_structural_candidates(&[c2], &HashSet::new(), &HashMap::new(), DateTime::now());
        assert!(!cands2.iter().any(|s| s.kind == "suggestion"));
    }

    #[test]
    fn contradiction_when_same_title_different_first_paragraph() {
        // 同 normalize_title 多 chunk，body 首段哈希不一致 → contradiction
        let mut a = chunk("产品价格策略", Some("methodology"), vec![]);
        a.body = Some("策略一：阶梯价。\n\n详细说明……".to_string());
        let mut b = chunk("产品价格策略", Some("methodology"), vec![]);
        b.body = Some("策略二：固定价。\n\n详细说明……".to_string());
        let cands = compute_structural_candidates(&[a, b], &HashSet::new(), &HashMap::new(), DateTime::now());
        assert!(cands.iter().any(|s| s.kind == "contradiction"));

        // 同 normalize_title 多 chunk 但首段一致 → 不出 contradiction
        let mut x = chunk("产品价格策略", Some("methodology"), vec![]);
        x.body = Some("策略一：阶梯价。\n\n详细说明……".to_string());
        let mut y = chunk("产品价格策略", Some("methodology"), vec![]);
        y.body = Some("策略一：阶梯价。\n\n另一段补充。".to_string());
        let cands2 = compute_structural_candidates(&[x, y], &HashSet::new(), &HashMap::new(), DateTime::now());
        assert!(!cands2.iter().any(|s| s.kind == "contradiction"));
    }

    #[test]
    fn anchor_is_dangling_contract() {
        // 无引用 → 不悬空（不打扰）
        assert!(!anchor_is_dangling(None, Some("任意原文")));
        assert!(!anchor_is_dangling(Some(""), Some("任意原文")));
        assert!(!anchor_is_dangling(Some("   "), Some("任意原文")));
        // 引用是原文子串 → 不悬空
        assert!(!anchor_is_dangling(Some("阶梯价"), Some("我们采用阶梯价策略")));
        // 容忍空白差异：原文有换行/空格，引用无
        assert!(!anchor_is_dangling(
            Some("阶梯价策略"),
            Some("我们采用阶梯\n价 策略")
        ));
        // 引用不在原文 → 悬空
        assert!(anchor_is_dangling(Some("固定价"), Some("我们采用阶梯价策略")));
        // 有引用但无原文 → 悬空
        assert!(anchor_is_dangling(Some("固定价"), None));
    }

    #[test]
    fn dangling_anchor_flagged_when_quote_not_in_raw() {
        let doc_id = ObjectId::new();
        let mut c = chunk("引用悬空页", Some("entity"), vec![]);
        c.document_id = Some(doc_id);
        c.source_quote = Some("某段不存在于原文的引用".to_string());
        let mut raw_by_doc = HashMap::new();
        raw_by_doc.insert(doc_id.to_hex(), "这是完全不同的文档原文内容".to_string());
        let cands = compute_structural_candidates(
            &[c],
            &HashSet::new(),
            &raw_by_doc,
            DateTime::now(),
        );
        assert!(cands.iter().any(|s| s.kind == "dangling_anchor"));
    }

    #[test]
    fn dangling_anchor_not_flagged_when_quote_present_or_no_raw() {
        let doc_id = ObjectId::new();
        // 引用命中原文 → 不出 dangling_anchor
        let mut hit = chunk("引用命中页", Some("entity"), vec![]);
        hit.document_id = Some(doc_id);
        hit.source_quote = Some("阶梯价".to_string());
        let mut raw_by_doc = HashMap::new();
        raw_by_doc.insert(doc_id.to_hex(), "我们采用阶梯价策略".to_string());
        let cands = compute_structural_candidates(
            &[hit],
            &HashSet::new(),
            &raw_by_doc,
            DateTime::now(),
        );
        assert!(!cands.iter().any(|s| s.kind == "dangling_anchor"));

        // 查无原文（raw_by_doc 缺该文档）→ 软语义跳过，不误报
        let mut no_raw = chunk("查无原文页", Some("entity"), vec![]);
        no_raw.document_id = Some(ObjectId::new());
        no_raw.source_quote = Some("任意引用".to_string());
        let cands2 = compute_structural_candidates(
            &[no_raw],
            &HashSet::new(),
            &HashMap::new(),
            DateTime::now(),
        );
        assert!(!cands2.iter().any(|s| s.kind == "dangling_anchor"));
    }

    /// Gap3 软降格半：悬空 anchor → dynamic_confidence 多扣 DANGLING_ANCHOR_PENALTY；
    /// 命中 / 无引用 → 不扣。罚分进 compute_dynamic_confidence 的 stale 位，
    /// rank_key 读 dynamic_confidence 即自然降格（软：只降不剔除）。
    #[test]
    fn dangling_anchor_penalty_demotes_only_dangling() {
        // 悬空：quote 不在原文 → 罚分 = DANGLING_ANCHOR_PENALTY
        let p_dangling = dangling_anchor_penalty(Some("固定价"), "我们采用阶梯价策略");
        assert!((p_dangling - DANGLING_ANCHOR_PENALTY).abs() < 1e-9);
        // 命中：quote 是原文子串 → 不罚
        assert_eq!(dangling_anchor_penalty(Some("阶梯价"), "我们采用阶梯价策略"), 0.0);
        // 无引用 → 不罚
        assert_eq!(dangling_anchor_penalty(None, "任意原文"), 0.0);

        // 端到端：同 base/样本，悬空者 dynamic_confidence 严格低于命中者。
        let base = 0.8;
        let live = compute_dynamic_confidence(base, 5, 0, 0.0, 5);
        let demoted = compute_dynamic_confidence(
            base,
            5,
            0,
            dangling_anchor_penalty(Some("固定价"), "我们采用阶梯价策略"),
            5,
        );
        assert!(demoted < live, "dangling demoted={demoted} must be < live={live}");
    }

    #[test]
    fn first_paragraph_splits_on_double_newline_and_trims() {
        assert_eq!(first_paragraph("hello\n\nworld"), "hello");
        assert_eq!(first_paragraph("only one"), "only one");
        assert_eq!(first_paragraph("  leading\n\nrest"), "leading");
        assert_eq!(first_paragraph(""), "");
    }

    #[test]
    fn sha256_hex_is_64_chars_lowercase() {
        let h = sha256_hex("hello");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // 同输入哈希一致
        assert_eq!(sha256_hex("hello"), sha256_hex("hello"));
        // 不同输入哈希不同
        assert_ne!(sha256_hex("hello"), sha256_hex("hello world"));
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

    // ---- P1 换血：classify_outcome_label 三态判定 ----

    #[test]
    fn classify_buying_signal_is_hit() {
        assert_eq!(
            classify_outcome_label(Some("user_replied_buying_signal")),
            OutcomeLabel::Hit
        );
    }

    #[test]
    fn classify_negative_outcomes_are_block() {
        // 复用 reaction.rs 的负向集合：异议 / 止 / 退订 / 负面 / 投诉。
        for s in [
            "user_replied_objection",
            "user_replied_stop_requested",
            "user_replied_unsubscribed",
            "user_replied_negative",
            "user_replied_complaint",
        ] {
            assert_eq!(
                classify_outcome_label(Some(s)),
                OutcomeLabel::Block,
                "{s} 应判为 Block"
            );
        }
    }

    #[test]
    fn classify_silence_and_pending_are_censored() {
        // Iron Law ②：沉默 / 无反应 / pending / 空 / 含义不明 → 删失（不进任何分母）。
        assert_eq!(classify_outcome_label(None), OutcomeLabel::Censored);
        assert_eq!(classify_outcome_label(Some("")), OutcomeLabel::Censored);
        assert_eq!(classify_outcome_label(Some("pending")), OutcomeLabel::Censored);
        assert_eq!(
            classify_outcome_label(Some("user_replied_unclassified")),
            OutcomeLabel::Censored
        );
        // 未知/未来枚举值同样删失，不臆测成 hit 或 block。
        assert_eq!(
            classify_outcome_label(Some("some_future_status")),
            OutcomeLabel::Censored
        );
    }

    #[test]
    fn censored_never_counts_as_block() {
        // 回归门：删失绝不能滑成负例（这正是镜厅/把沉默当负例的病根）。
        assert_ne!(classify_outcome_label(None), OutcomeLabel::Block);
        assert_ne!(
            classify_outcome_label(Some("pending")),
            OutcomeLabel::Block
        );
    }
}

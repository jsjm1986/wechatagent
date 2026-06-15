//! `apply_chunk_revision` 状态机 —— 写入路径的"七个动作"统一入口。
//!
//! 设计契约（沿用 `nashsu/llm_wiki` 的 page-merge 三层保护）：
//!
//! 1. **锁定字段守门**：patch 携带 `chunk_id / wiki_type / created_at /
//!    source_anchor / verified_at / verified_by / approved_at` 任一 → 4xx 拒收。
//!    LLM 永远没机会改这些字段。
//! 2. **数组字段 union**：`tags / search_terms / applicable_scenes / ...`
//!    永远 existing ∪ patch，应用层完成，LLM 输出空数组 ≠ 清空。
//! 3. **70% body 长度阈值**：patch 后 body/answer/summary 短于既有 70% → 拒收。
//!
//! 7 个动作：create / patch / split / merge / archive / restore / rollback。
//! 每次写入双写：`operation_knowledge_chunks` + `chunk_revisions`，先写 revisions
//! 后写 chunks（前者失败 → 直接 abort；后者失败 → revisions 留下"试图未成功"
//! 痕迹，便于人工查 last_revision != current_state）。
//!
//! AI source（`ProvenanceSource::Ai`）的写入强制 `status="draft"` +
//! `integrity_status="needs_review"`，对齐 CLAUDE.md "AI 永不自动 verify" 硬约束。

use std::str::FromStr;

use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::knowledge_wiki::page_merge::{
    apply_field_patch, compute_chunk_hash, enforce_locked_fields, is_body_truncated,
    union_array_fields, RevisionError, BODY_TRUNCATION_THRESHOLD, DEFAULT_LOCKED_FIELDS,
    DEFAULT_UNION_ARRAY_KEYS,
};
use crate::models::{CatalogRebuildJob, ChunkRevision};

// ── 操作语义封闭枚举 ───────────────────────────────────────────────────

/// `chunk_revisions.op` 合法值（design.md §9 / CLAUDE.md）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionOp {
    Create,
    Patch,
    Split,
    Merge,
    Rollback,
    Archive,
    Restore,
    Verify,
    Unverify,
}

impl RevisionOp {
    pub fn as_str(self) -> &'static str {
        match self {
            RevisionOp::Create => "create",
            RevisionOp::Patch => "patch",
            RevisionOp::Split => "split",
            RevisionOp::Merge => "merge",
            RevisionOp::Rollback => "rollback",
            RevisionOp::Archive => "archive",
            RevisionOp::Restore => "restore",
            RevisionOp::Verify => "verify",
            RevisionOp::Unverify => "unverify",
        }
    }
}

/// `chunk_revisions.source` 合法值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceSource {
    /// LLM 调用方写入；强制 `status=draft + integrity_status=needs_review`。
    Ai,
    /// 运营 / admin 直接编辑（包含 verify 通过后的人工签字路径）。
    Human,
    /// feedback worker / sweep / cleanup 触发的规则化写入。
    Rule,
    /// import_apply 流式块导入。
    Imported,
}

impl ProvenanceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ProvenanceSource::Ai => "ai",
            ProvenanceSource::Human => "human",
            ProvenanceSource::Rule => "rule",
            ProvenanceSource::Imported => "imported",
        }
    }
}

impl FromStr for ProvenanceSource {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ai" => Ok(ProvenanceSource::Ai),
            "human" => Ok(ProvenanceSource::Human),
            "rule" => Ok(ProvenanceSource::Rule),
            "imported" => Ok(ProvenanceSource::Imported),
            other => Err(AppError::BadRequest(format!(
                "invalid revision source '{other}'; expected one of ai|human|rule|imported"
            ))),
        }
    }
}

// ── 写入结果 ──────────────────────────────────────────────────────────

/// `apply_chunk_revision` 成功返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionApplied {
    pub revision_id: String,
    pub chunk_id: String,
    pub op: String,
    pub before_hash: String,
    pub after_hash: String,
    /// 若内容 hash 未变（patch 全部命中既有值），返回 `unchanged=true`，
    /// 调用方可据此跳过 catalog rebuild enqueue。
    pub unchanged: bool,
}

// ── 入参 ──────────────────────────────────────────────────────────────

/// `apply_chunk_revision` 的入参。
///
/// `patch` 是 BSON `Document`，**仅含要变更的字段**。
/// `actor` / `reason` 写到 `chunk_revisions.created_by / reason` 用于审计追溯。
pub struct RevisionRequest {
    pub op: RevisionOp,
    pub source: ProvenanceSource,
    pub patch: Document,
    pub reason: Option<String>,
    pub actor: Option<String>,
}

// ── 主入口 ────────────────────────────────────────────────────────────

/// 三层保护下的 chunk 写入入口。
///
/// 步骤：
/// 1. `find_one` 既有 chunk（`workspace_id` 守门，跨 workspace 写入 → NotFound）；
/// 2. `apply_field_patch` 对 patch 的顶层 key 做锁定字段守门（含 patch 拒收）；
/// 3. `union_array_fields` 对默认数组字段做 existing ∪ patch；
/// 4. body/answer/summary 长度 < existing × 70% → 拒收；
/// 5. AI source 强制 draft + needs_review；
/// 6. `enforce_locked_fields` 末次防线；
/// 7. 写 `chunk_revisions`（先）+ `operation_knowledge_chunks` 替换写（后）；
/// 8. enqueue `catalog_rebuild_jobs`（best-effort，失败不影响主写入）。
pub async fn apply_chunk_revision(
    db: &Database,
    workspace_id: &str,
    chunk_object_id: ObjectId,
    req: RevisionRequest,
) -> AppResult<RevisionApplied> {
    let coll = db.operation_knowledge_chunks();
    let existing_doc = coll
        .find_one(
            doc! {
                "_id": chunk_object_id,
                "workspace_id": workspace_id,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
    let existing_bson = mongodb::bson::to_document(&existing_doc).map_err(|e| {
        AppError::External(format!("serialize existing chunk to bson failed: {e}"))
    })?;
    let chunk_id_hex = chunk_object_id.to_hex();
    let before_hash = compute_chunk_hash(&existing_bson);

    // 1) 锁定字段守门 + field-level patch 应用（标量字段层）
    let after_patch = apply_field_patch(&existing_bson, &req.patch, DEFAULT_LOCKED_FIELDS).map_err(
        |e| match e {
            RevisionError::LockedFieldInPatch { field } => AppError::BadRequest(format!(
                "字段 {field} 受锁定保护，不允许通过 patch 修改"
            )),
            RevisionError::BodyTruncated {
                old_len,
                new_len,
                threshold,
            } => AppError::BadRequest(format!(
                "新 body 长度 {new_len} 低于既有 {old_len} 的 {:.0}% 阈值；疑似 LLM 截断/偷懒，已拒收",
                threshold * 100.0
            )),
        },
    )?;

    // 2) 数组字段 union（永远 existing ∪ patch）
    let merged = union_array_fields(&after_patch, &req.patch, DEFAULT_UNION_ARRAY_KEYS);

    // 3) body / summary / answer 70% 长度阈值
    let touched_text_field = req.patch.contains_key("body")
        || req.patch.contains_key("summary")
        || req.patch.contains_key("answer");
    if touched_text_field {
        let old_len = text_payload_len(&existing_bson);
        let new_len = text_payload_len(&merged);
        let incoming_len = text_payload_len(&req.patch);
        if is_body_truncated(old_len, incoming_len, new_len, BODY_TRUNCATION_THRESHOLD) {
            return Err(AppError::BadRequest(format!(
                "新 body 长度 {new_len} 低于既有 {old_len} 的 70% 阈值；疑似 LLM 截断/偷懒，已拒收。如确需缩短请人工调整后再写入",
            )));
        }
    }

    // 4) AI source 强制 draft + needs_review
    let mut merged = merged;
    if matches!(req.source, ProvenanceSource::Ai) {
        merged.insert("status", "draft");
        merged.insert("integrity_status", "needs_review");
    }
    // archive / restore 直接覆盖 status，方便上层不必逐个 patch
    match req.op {
        RevisionOp::Archive => {
            merged.insert("status", "archived");
        }
        RevisionOp::Restore => {
            merged.insert("status", "active");
        }
        _ => {}
    }
    merged.insert("updated_at", DateTime::now());

    // 5) provenance 标注（每次写入都覆盖 edited_at / source / edited_by）
    let provenance = doc! {
        "source": req.source.as_str(),
        "edited_at": DateTime::now(),
        "edited_by": req.actor.clone().unwrap_or_default(),
    };
    merged.insert("provenance", Bson::Document(provenance));

    // 6) 末次防线：锁定字段强制覆盖回 existing
    let mut merged = enforce_locked_fields(&merged, &existing_bson, DEFAULT_LOCKED_FIELDS);

    // 6.5) universal-domain-adaptation D1-b：active DomainSchema 校验 / 重写
    // domain_attributes。仅当本 workspace 有 active schema 时生效；无 active schema
    // （DEFAULT / 未配置行业 schema）→ 完全 no-op 直通，零行为变化。命中 required 缺失
    // / enum 越界 → BadRequest 拒收；alias 命中 → 透明改写成 canonical 后落库（计入下方
    // after_hash，alias 改写也算一次实质变更）。
    if let Some(schema) =
        crate::routes::domain_schemas::load_active_domain_schema(db, workspace_id).await?
    {
        if let Ok(attrs) = merged.get_document("domain_attributes") {
            let enforced = crate::routes::domain_schemas::enforce_domain_attributes(
                &schema,
                &attrs.clone(),
            )?;
            merged.insert("domain_attributes", Bson::Document(enforced));
        }
    }

    let after_hash = compute_chunk_hash(&merged);
    let unchanged = before_hash == after_hash;

    // 7) 双写：先 chunk_revisions（不可变历史），后 chunks（可变最新）
    let revision_id = format!("rev_{}_{}", chunk_id_hex, uuid::Uuid::new_v4().simple());
    let revision = ChunkRevision {
        id: None,
        chunk_id: chunk_id_hex.clone(),
        revision_id: revision_id.clone(),
        op: req.op.as_str().to_string(),
        patch: req.patch.clone(),
        before_hash: before_hash.clone(),
        after_hash: after_hash.clone(),
        source: req.source.as_str().to_string(),
        reason: req.reason,
        created_at: DateTime::now(),
        created_by: req.actor,
    };
    db.chunk_revisions().insert_one(revision, None).await?;

    if !unchanged {
        let merged_typed: crate::models::OperationKnowledgeChunk =
            mongodb::bson::from_document(merged.clone())
                .map_err(|e| AppError::External(format!("deserialize merged chunk failed: {e}")))?;
        coll.replace_one(
            doc! {
                "_id": chunk_object_id,
                "workspace_id": workspace_id,
            },
            merged_typed,
            None,
        )
        .await?;

        // 8) enqueue catalog rebuild（best-effort）
        if let Some(doc_id) = existing_doc.document_id {
            let job = CatalogRebuildJob {
                id: None,
                job_id: format!("crj_{}_{}", doc_id.to_hex(), uuid::Uuid::new_v4().simple()),
                workspace_id: workspace_id.to_string(),
                document_id: doc_id,
                queued_at: DateTime::now(),
                status: "queued".to_string(),
                attempts: 0,
                last_error: None,
                started_at: None,
                finished_at: None,
            };
            // 失败仅记录日志，不影响主写入语义
            if let Err(err) = db.catalog_rebuild_jobs().insert_one(job, None).await {
                tracing::warn!(
                    chunk_id = %chunk_id_hex,
                    error = %err,
                    "enqueue catalog rebuild job failed (non-fatal)"
                );
            }
        }
    }

    Ok(RevisionApplied {
        revision_id,
        chunk_id: chunk_id_hex,
        op: req.op.as_str().to_string(),
        before_hash,
        after_hash,
        unchanged,
    })
}

// ── 帮手：text payload 长度（取 body / summary 中较长者）─────────────

/// 取 chunk 中"主体文本"长度。优先 body，其次 summary。
fn text_payload_len(d: &Document) -> usize {
    let body_len = d
        .get_str("body")
        .ok()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let summary_len = d
        .get_str("summary")
        .ok()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    body_len.max(summary_len)
}

// ── 删除级联：normalize_ref_key / cleanup_dangling_refs ───────────────

/// 把 chunk 引用 key 规范化（防 substring 误伤："openai" 不应匹配 "ai"）。
///
/// 借鉴 LLW `wiki-cleanup.ts:49-130`：
/// - 去 `.md` 扩展名；
/// - 取末段（按 `/` 分割），避免 path 前缀干扰；
/// - 全部小写；
/// - 去除 ASCII 空格 / 短横 / 下划线。
pub fn normalize_ref_key(s: &str) -> String {
    let leaf = s.trim_end_matches(".md").rsplit('/').next().unwrap_or(s);
    leaf.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect()
}

/// chunk 删除（archive）后清理其它 chunk 中指向它的 `related_chunks` 条目。
///
/// 实现：
/// 1. 查同 workspace 所有 chunks，遍历其 `related_chunks: Vec<RelatedRef>`；
/// 2. 命中 `chunk_id == archived_id`（或 normalize_ref_key 等价）→ 移除；
/// 3. 每条受影响的 chunk 自己也走 `apply_chunk_revision(op=Patch, source=Rule,
///    reason="cleanup_dangling_refs")`，留追溯。
///
/// 失败不冒泡 —— archive 主动作已成，cleanup 仅 best-effort。
pub async fn cleanup_dangling_refs(
    db: &Database,
    workspace_id: &str,
    archived_chunk_id_hex: &str,
) -> AppResult<usize> {
    use futures::TryStreamExt;
    let normalized_target = normalize_ref_key(archived_chunk_id_hex);
    let mut cursor = db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "related_chunks": { "$exists": true, "$ne": [] }
            },
            None,
        )
        .await?;
    let mut affected = 0usize;
    while let Some(chunk) = cursor.try_next().await? {
        let related = match chunk.related_chunks.clone() {
            Some(r) => r,
            None => continue,
        };
        let kept: Vec<_> = related
            .into_iter()
            .filter(|r| {
                let by_id = r.chunk_id == archived_chunk_id_hex;
                let by_norm = normalize_ref_key(&r.chunk_id) == normalized_target;
                !(by_id || by_norm)
            })
            .collect();
        if kept.len() == chunk.related_chunks.as_ref().map(|v| v.len()).unwrap_or(0) {
            continue;
        }
        // 写一条 patch revision 留痕迹
        let chunk_oid = match chunk.id {
            Some(o) => o,
            None => continue,
        };
        let patch = doc! {
            "related_chunks": mongodb::bson::to_bson(&kept).unwrap_or(Bson::Null),
        };
        // 注意：related_chunks 不在 DEFAULT_UNION_ARRAY_KEYS（结构数组按 chunk_id
        // 去重才正确，简单 string union 不适用），所以这里直接 patch 整数组。
        let req = RevisionRequest {
            op: RevisionOp::Patch,
            source: ProvenanceSource::Rule,
            patch,
            reason: Some(format!(
                "cleanup_dangling_refs: archived chunk {archived_chunk_id_hex}"
            )),
            actor: Some("system:cleanup_worker".to_string()),
        };
        match apply_chunk_revision(db, workspace_id, chunk_oid, req).await {
            Ok(_) => affected += 1,
            Err(err) => {
                tracing::warn!(
                    chunk_id = %chunk_oid.to_hex(),
                    error = %err,
                    "cleanup_dangling_refs apply_chunk_revision failed (non-fatal)"
                );
            }
        }
    }
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_ext_path_and_punctuation() {
        assert_eq!(normalize_ref_key("OpenAI"), "openai");
        assert_eq!(normalize_ref_key("docs/ai_lab.md"), "ailab");
        assert_eq!(normalize_ref_key("a-b_c"), "abc");
    }

    #[test]
    fn normalize_does_not_substring_match_openai_to_ai() {
        // 关键安全保证：normalize 后 "openai" != "ai"
        assert_ne!(normalize_ref_key("openai"), normalize_ref_key("ai"));
    }

    #[test]
    fn revision_op_round_trip() {
        for (op, s) in [
            (RevisionOp::Create, "create"),
            (RevisionOp::Patch, "patch"),
            (RevisionOp::Split, "split"),
            (RevisionOp::Merge, "merge"),
            (RevisionOp::Rollback, "rollback"),
            (RevisionOp::Archive, "archive"),
            (RevisionOp::Restore, "restore"),
            (RevisionOp::Verify, "verify"),
            (RevisionOp::Unverify, "unverify"),
        ] {
            assert_eq!(op.as_str(), s);
        }
    }

    #[test]
    fn provenance_source_round_trip() {
        assert_eq!(ProvenanceSource::Ai.as_str(), "ai");
        assert_eq!(
            ProvenanceSource::from_str("imported").unwrap().as_str(),
            "imported"
        );
        assert!(ProvenanceSource::from_str("evil").is_err());
    }
}

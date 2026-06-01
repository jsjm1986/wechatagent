//! 结构化写 **意图提案**（方法论点 6：写分 LEAF vs STRUCTURAL）。
//!
//! leaf 写（加事实/补引用/编辑正文）走既有 `chunk_revisions` 的 draft+needs_review
//! + auto-verify 4 因子门。**结构化写**（split / merge / reclassify / mark_superseded /
//! rewrite_directory_intent）影响面大、不可逆，本轮**只产 intent proposal**：
//!
//! - `status` 恒 `pending_review`，**绝不 auto-commit、绝不物理删除**；
//! - 中性 candidate/proposal 语义（对齐 AI 自主定位，绝不引入运营外的处置角色）；
//! - 系统侧原子化应用属于**下一轮**（本轮 out-of-scope：无 apply worker /
//!   无版本一致性机器）。
//!
//! 与 [`crate::knowledge_wiki::chunk_revisions`] 的 `RevisionOp` 一样，本模块自带
//! 操作枚举 `StructuralKind`，不碰 `models.rs`（其它 agent 的文件）。集合走
//! [`crate::db::Database::raw`] 逃生口按需懒创建，不动 `db/mod.rs`。

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::Collection;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::{AppError, AppResult};

/// 结构化写操作的封闭枚举（方法论点 6 列举的 5 类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralKind {
    /// 一个原子粒度过粗 → 建议拆成多个细视图。
    Split,
    /// 多个重复/碎片原子 → 建议合并去重。
    Merge,
    /// wiki_type / chunk_type 错放 → 建议重新归类。
    Reclassify,
    /// 旧版本被新版本取代 → 建议标 superseded（绝不物理删除）。
    MarkSuperseded,
    /// 导航卡/目录意图需重写（导航卡恒重算，这里只记意图）。
    RewriteDirectoryIntent,
}

impl StructuralKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StructuralKind::Split => "split",
            StructuralKind::Merge => "merge",
            StructuralKind::Reclassify => "reclassify",
            StructuralKind::MarkSuperseded => "mark_superseded",
            StructuralKind::RewriteDirectoryIntent => "rewrite_directory_intent",
        }
    }
}

/// proposal 生命周期状态。本轮**只有一个合法值**——所有结构化写恒落
/// `pending_review`，等待知识运营质检。apply/reject 流转属于下一轮。
pub const STATUS_PENDING_REVIEW: &str = "pending_review";

/// 结构化写意图提案。落 `structural_proposals` 集合（懒创建）。
///
/// **安全语义红线**（序列化层即锁死）：
/// - 无 `apply` / `commit` / `delete` 字段——本结构体物理上无法表达「已应用」；
/// - `status` 恒 [`STATUS_PENDING_REVIEW`]（[`StructuralProposal::new`] 唯一构造口）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralProposal {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub proposal_id: String,
    pub workspace_id: String,
    /// `split | merge | reclassify | mark_superseded | rewrite_directory_intent`。
    pub kind: String,
    /// 受影响的 chunk_id（hex）；intent 只记指向，不动这些 chunk 本体。
    pub target_chunk_ids: Vec<String>,
    /// 结构化写的具体载荷（如 split 的切点、merge 的目标）。自由 BSON，
    /// 由下一轮 apply worker 解释；本轮只存不解释。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Document>,
    /// 恒 `pending_review`。
    pub status: String,
    /// 为何提此 intent（如 recall_low_yield 触发的粒度问题）。
    pub rationale: String,
    /// 来源：`recall_trace`（Gap1 闭环 emit）/ `rule`（离线 lint）/ `human`。
    pub source: String,
    pub created_at: DateTime,
}

impl StructuralProposal {
    /// 唯一构造口：`status` 强制 [`STATUS_PENDING_REVIEW`]，调用方无法传入其它
    /// 状态——从类型层面杜绝 auto-commit。
    pub fn new(
        workspace_id: impl Into<String>,
        kind: StructuralKind,
        target_chunk_ids: Vec<String>,
        rationale: impl Into<String>,
        source: impl Into<String>,
        payload: Option<Document>,
    ) -> Self {
        Self {
            id: None,
            proposal_id: ObjectId::new().to_hex(),
            workspace_id: workspace_id.into(),
            kind: kind.as_str().to_string(),
            target_chunk_ids,
            payload,
            status: STATUS_PENDING_REVIEW.to_string(),
            rationale: rationale.into(),
            source: source.into(),
            created_at: DateTime::now(),
        }
    }
}

/// `structural_proposals` 集合 accessor —— 走 [`Database::raw`] 逃生口懒创建，
/// 不碰 `db/mod.rs`（隔离红线）。
pub fn structural_proposals(db: &Database) -> Collection<StructuralProposal> {
    db.raw().collection("structural_proposals")
}

/// 落一条结构化写 intent。**只产 intent、绝不应用**：插入 `pending_review`，
/// 不改任何 chunk、不删任何数据、不重算 catalog。返回 `proposal_id`。
///
/// 隔离红线：本函数只 insert，不引用 gateway/outbox/mcp/catalog_rebuild。
pub async fn propose_structural_change(
    db: &Database,
    workspace_id: &str,
    kind: StructuralKind,
    target_chunk_ids: Vec<String>,
    rationale: &str,
    source: &str,
    payload: Option<Document>,
) -> AppResult<String> {
    let proposal = StructuralProposal::new(
        workspace_id,
        kind,
        target_chunk_ids,
        rationale,
        source,
        payload,
    );
    let proposal_id = proposal.proposal_id.clone();
    structural_proposals(db)
        .insert_one(&proposal, None)
        .await
        .map_err(AppError::from)?;
    Ok(proposal_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 安全语义红线：任意 kind 构造后 status 恒 pending_review。
    #[test]
    fn new_always_pending_review() {
        for kind in [
            StructuralKind::Split,
            StructuralKind::Merge,
            StructuralKind::Reclassify,
            StructuralKind::MarkSuperseded,
            StructuralKind::RewriteDirectoryIntent,
        ] {
            let p = StructuralProposal::new(
                "w1",
                kind,
                vec!["c1".to_string()],
                "test rationale",
                "rule",
                None,
            );
            assert_eq!(p.status, STATUS_PENDING_REVIEW);
            assert_eq!(p.kind, kind.as_str());
            assert!(!p.proposal_id.is_empty());
        }
    }

    /// 序列化层无 apply/commit/delete 字段——物理上无法表达「已应用」。
    #[test]
    fn serialized_has_no_apply_or_delete_fields() {
        let p = StructuralProposal::new(
            "w1",
            StructuralKind::MarkSuperseded,
            vec!["old".to_string()],
            "superseded by newer",
            "recall_trace",
            None,
        );
        let bson = mongodb::bson::to_document(&p).expect("serialize");
        for forbidden in ["apply", "applied", "commit", "committed", "delete", "deleted"] {
            assert!(
                !bson.contains_key(forbidden),
                "proposal must not carry field '{forbidden}'",
            );
        }
        assert_eq!(bson.get_str("status").unwrap(), STATUS_PENDING_REVIEW);
    }

    /// kind 枚举字符串与方法论点 6 列举集合一致。
    #[test]
    fn kind_strings_match_methodology() {
        assert_eq!(StructuralKind::Split.as_str(), "split");
        assert_eq!(StructuralKind::Merge.as_str(), "merge");
        assert_eq!(StructuralKind::Reclassify.as_str(), "reclassify");
        assert_eq!(StructuralKind::MarkSuperseded.as_str(), "mark_superseded");
        assert_eq!(
            StructuralKind::RewriteDirectoryIntent.as_str(),
            "rewrite_directory_intent"
        );
    }
}

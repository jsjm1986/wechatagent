//! knowledge-wiki 子系统：把"销售话术 RAG"升级为"运营知识 Wiki + 检索面"。
//!
//! 本模块只负责**写入路径的强约束 + 反馈闭环 + 编辑历史**这四件事:
//! - **质量** — schema 校验 + 锁定字段 + provenance 标注
//! - **可检索** — wiki 形态分层 + frontmatter + wikilinks + 落库 catalog
//! - **可修改** — 字段级 patch + 编辑历史不可变 + 删除级联
//! - **可优化** — usage / hit / blocked 回写 chunk + 两阶段 sweep
//!
//! 召回算法（`agent::knowledge_router` 的 catalog → list_chunks → open_slice）
//! 本轮**零改动**；模块隔离红线：本模块**禁止**引用 `crate::agent::gateway/outbox`、
//! `crate::mcp::*`、`agent_send_outbox`、`run_user_operation_gateway`。
//!
//! 子模块分工：
//! - [`page_merge`]：100% 纯函数 — array union / 锁定字段 / 70% body 阈值 /
//!   field patch / chunk hash。所有写入都要走这里的纯函数预校验。
//! - [`chunk_revisions`]：`apply_chunk_revision` 状态机 + cleanup_dangling_refs。
//!   `routes::knowledge` 的 7 个编辑路由（patch/split/merge/archive/restore/
//!   rollback + revisions list）全部走这里。

pub mod block_parser;
pub mod catalog_rebuild;
pub mod chunk_revisions;
pub mod feedback_worker;
pub mod gap_signals;
pub mod ingest_worker;
pub mod lessons_learned;
pub mod page_merge;
pub mod reviewer_stats;
pub mod structural_proposals;

//! `catalog_rebuild` —— catalog 落库 + 异步重写 worker。
//!
//! 现状：catalog 是每次请求实时聚合（`routes::knowledge::build_operation_knowledge_catalog`），
//! N 个 chunk → O(N) 拼装。
//!
//! 改造：写入路径 `apply_chunk_revision` enqueue 一条 `catalog_rebuild_jobs`；
//! 本 worker 每 N 秒取一批 `status="queued"` 的 job：
//!
//! 1. 按 `document_id` group 聚合该 document 下所有 active chunk；
//! 2. 渲染 `catalog_summary_persisted`（markdown 摘要）；
//! 3. 自增 `catalog_version`；
//! 4. job 标 `done` / `failed`（带 last_error）。
//!
//! 设计要点：
//! - **后向兼容**：`catalog_summary_persisted` 是 `Option<String>`，旧 doc 读出 None
//!   不影响现有路由；
//! - **零阻塞**：写入路径 enqueue 是 best-effort，worker 异步处理；
//! - **空闲休眠**：取不到 job 时 sleep `interval_secs`，CPU 不空转；
//! - **失败容错**：单 job 报错只标 failed + last_error，不 panic worker；
//! - **零新依赖**：用 `tracing` + 既有 mongo accessor。

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use mongodb::options::FindOneAndUpdateOptions;
use tokio::time::sleep;

use crate::db::Database;
use crate::error::AppError;

/// 单次循环最多领取的 job 数。避免 worker 在大批量 enqueue 时长占数据库连接。
const BATCH_SIZE: usize = 16;

/// catalog rebuild worker 主循环。
///
/// 由 `main.rs` 在启动时 `tokio::spawn`：
/// ```text
/// tokio::spawn(catalog_rebuild_worker_loop(db.clone(), 3));
/// ```
///
/// `interval_secs == 0` → 关停（不进循环），便于测试 / 灰度。
pub async fn catalog_rebuild_worker_loop(db: Database, interval_secs: u64) {
    if interval_secs == 0 {
        tracing::info!("catalog_rebuild_worker disabled (interval_secs=0)");
        return;
    }
    let interval = Duration::from_secs(interval_secs);
    tracing::info!(
        interval_secs,
        "catalog_rebuild_worker started"
    );
    loop {
        match drain_pending_jobs(&db).await {
            Ok(n) if n > 0 => {
                tracing::debug!(processed = n, "catalog_rebuild_worker drained jobs");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "catalog_rebuild_worker drain error");
            }
        }
        sleep(interval).await;
    }
}

/// 取一批 queued job 处理，返回处理条数。
///
/// 每条 job 走 `claim_one_job` 原子领取（`status: queued -> processing`），
/// 然后 `rebuild_one_document` 渲染落库，最后 `mark_job_done` / `mark_job_failed`。
async fn drain_pending_jobs(db: &Database) -> Result<usize, AppError> {
    let mut processed = 0usize;
    while processed < BATCH_SIZE {
        let claimed = claim_one_job(db).await?;
        let job = match claimed {
            Some(j) => j,
            None => break,
        };
        let job_id = job.job_id.clone();
        let workspace_id = job.workspace_id.clone();
        let document_id = job.document_id;
        match rebuild_one_document(db, &workspace_id, document_id).await {
            Ok(_) => {
                mark_job_done(db, &job_id).await?;
            }
            Err(e) => {
                let msg = format!("{e}");
                tracing::warn!(job_id = %job_id, error = %msg, "catalog rebuild job failed");
                mark_job_failed(db, &job_id, &msg).await?;
            }
        }
        processed += 1;
    }
    Ok(processed)
}

/// 原子领取一条 queued job：`findOneAndUpdate(status="queued") => status="processing"`。
async fn claim_one_job(db: &Database) -> Result<Option<crate::models::CatalogRebuildJob>, AppError> {
    let now = DateTime::now();
    let filter = doc! { "status": "queued" };
    let update = doc! {
        "$set": {
            "status": "processing",
            "started_at": now,
        },
        "$inc": { "attempts": 1i32 },
    };
    let opts = FindOneAndUpdateOptions::builder()
        .return_document(mongodb::options::ReturnDocument::After)
        .sort(doc! { "queued_at": 1 })
        .build();
    let claimed = db
        .catalog_rebuild_jobs()
        .find_one_and_update(filter, update, opts)
        .await?;
    Ok(claimed)
}

/// 渲染单个 document 的 catalog 落库。
///
/// 步骤：
/// 1. 拉该 document 下所有 `status="active"` 的 chunk（不含 archived）；
/// 2. 按 `priority` desc 排序，渲染 markdown；
/// 3. `findOneAndUpdate documents._id == doc_id` `$set catalog_summary_persisted` `$inc catalog_version`。
async fn rebuild_one_document(
    db: &Database,
    workspace_id: &str,
    document_id: ObjectId,
) -> Result<(), AppError> {
    let mut chunk_cursor = db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "document_id": document_id,
                "status": "active",
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "priority": -1, "updated_at": -1 })
                .limit(500)
                .build(),
        )
        .await?;

    let mut chunks: Vec<crate::models::OperationKnowledgeChunk> = Vec::new();
    while let Some(c) = chunk_cursor.try_next().await? {
        chunks.push(c);
    }

    let rendered = render_persisted_catalog(&chunks);

    db.operation_knowledge_documents()
        .update_one(
            doc! {
                "_id": document_id,
                "workspace_id": workspace_id,
            },
            doc! {
                "$set": {
                    "catalog_summary_persisted": &rendered,
                    "updated_at": DateTime::now(),
                },
                "$inc": { "catalog_version": 1i64 },
            },
            None,
        )
        .await?;

    Ok(())
}

/// 把 chunk 列表渲染为 markdown 形式的 persisted catalog。
///
/// 格式（每 chunk 一段）：
/// ```text
/// ### {title}
/// - id: {chunk_id}
/// - 类型: {wiki_type ?? knowledge_type ?? "未分类"}
/// - 路由: {routing_card ?? "—"}
/// - integrity: {integrity_status ?? "—"} | confidence: {confidence_score ?? "—"}
/// - dynamic: {dynamic_confidence ?? "—"} | hits/30d: {hit_count_30d ?? 0}
/// > {summary or body excerpt ≤ 240 chars}
/// ```
pub fn render_persisted_catalog(chunks: &[crate::models::OperationKnowledgeChunk]) -> String {
    if chunks.is_empty() {
        return String::from("（该文档暂无 active chunk）");
    }
    let mut buf = String::with_capacity(chunks.len() * 256);
    for c in chunks {
        let id = c
            .id
            .map(|o| o.to_hex())
            .unwrap_or_else(|| String::from("?"));
        let wiki_type = c
            .wiki_type
            .as_deref()
            .or(c.knowledge_type.as_deref())
            .unwrap_or("未分类");
        let routing = "—";
        let integrity = c.integrity_status.as_deref().unwrap_or("—");
        let confidence = c
            .confidence_score
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".to_string());
        let dynamic = c
            .dynamic_confidence
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "—".to_string());
        let hits = c
            .usage_stats
            .as_ref()
            .map(|u| u.hit_count_30d)
            .unwrap_or(0);
        let excerpt = c
            .summary
            .as_deref()
            .or(c.body.as_deref())
            .unwrap_or("")
            .chars()
            .take(240)
            .collect::<String>();
        buf.push_str(&format!("### {}\n", c.title));
        buf.push_str(&format!("- id: {id}\n"));
        buf.push_str(&format!("- 类型: {wiki_type}\n"));
        buf.push_str(&format!("- 路由: {routing}\n"));
        buf.push_str(&format!(
            "- integrity: {integrity} | confidence: {confidence}\n"
        ));
        buf.push_str(&format!("- dynamic: {dynamic} | hits/30d: {hits}\n"));
        if !excerpt.is_empty() {
            buf.push_str(&format!("> {}\n", excerpt.replace('\n', " ")));
        }
        buf.push('\n');
    }
    buf
}

async fn mark_job_done(db: &Database, job_id: &str) -> Result<(), AppError> {
    db.catalog_rebuild_jobs()
        .update_one(
            doc! { "job_id": job_id },
            doc! {
                "$set": {
                    "status": "done",
                    "finished_at": DateTime::now(),
                    "last_error": null,
                },
            },
            None,
        )
        .await?;
    Ok(())
}

async fn mark_job_failed(db: &Database, job_id: &str, err: &str) -> Result<(), AppError> {
    db.catalog_rebuild_jobs()
        .update_one(
            doc! { "job_id": job_id },
            doc! {
                "$set": {
                    "status": "failed",
                    "finished_at": DateTime::now(),
                    "last_error": err,
                },
            },
            None,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{OperationKnowledgeChunk, UsageStats};
    use mongodb::bson::DateTime;

    fn empty_chunk(title: &str) -> OperationKnowledgeChunk {
        OperationKnowledgeChunk {
            id: None,
            workspace_id: "ws_default".to_string(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: "user_operations".to_string(),
            knowledge_type: None,
            business_context: None,
            title: title.to_string(),
            summary: Some(format!("摘要 of {title}")),
            body: None,
            applicable_scenes: vec![],
            not_applicable_scenes: vec![],
            product_tags: vec![],
            business_topics: vec![],
            source_quote: None,
            source_anchors: vec![],
            integrity_status: Some("verified".to_string()),
            confidence_score: Some(85),
            status: "active".to_string(),
            priority: 0,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
            wiki_type: Some("methodology".to_string()),
            domain_attributes: None,
            provenance: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            previous_version_id: None,
            related_chunks: None,
            usage_stats: Some(UsageStats {
                hit_count_30d: 7,
                blocked_count_30d: 0,
                last_used_at: None,
                last_blocked_reason: None,
            }),
            dynamic_confidence: Some(0.83),
            integrity_score: None,
            locked_fields: None,
            chunk_type: "product_fact".to_string(),
        }
    }

    #[test]
    fn render_empty_returns_placeholder() {
        let s = render_persisted_catalog(&[]);
        assert!(s.contains("暂无 active chunk"));
    }

    #[test]
    fn render_includes_title_routing_dynamic_and_hits() {
        let chunks = vec![empty_chunk("测试标题")];
        let s = render_persisted_catalog(&chunks);
        assert!(s.contains("### 测试标题"));
        assert!(s.contains("methodology"));
        assert!(s.contains("路由: —"));
        assert!(s.contains("dynamic: 0.83"));
        assert!(s.contains("hits/30d: 7"));
        assert!(s.contains("摘要 of 测试标题"));
    }

    #[test]
    fn render_falls_back_when_optional_fields_missing() {
        let mut c = empty_chunk("退化");
        c.wiki_type = None;
        c.knowledge_type = None;
        c.dynamic_confidence = None;
        c.usage_stats = None;
        c.summary = None;
        c.body = None;
        let s = render_persisted_catalog(&[c]);
        assert!(s.contains("未分类"));
        assert!(s.contains("路由: —"));
        assert!(s.contains("dynamic: —"));
        assert!(s.contains("hits/30d: 0"));
    }
}

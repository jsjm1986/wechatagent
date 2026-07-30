//! `catalog_rebuild` —— catalog 落库 + 异步重写 worker。
//!
//! 现状：catalog 是每次请求实时聚合（`routes::knowledge::build_operation_knowledge_catalog`），
//! N 个 chunk → O(N) 拼装。
//!
//! Every chunk mutation advances a document generation and enqueues one durable
//! intent in the same transaction. The worker leases queued or expired work:
//!
//! 1. 按 `document_id` group 聚合该 document 下所有 active chunk；
//! 2. 渲染 `catalog_summary_persisted`（markdown 摘要）；
//! 3. atomically persist only the latest desired generation;
//! 4. finalize by owner/token/generation CAS, with bounded retry.
//!
//! 设计要点：
//! - **后向兼容**：`catalog_summary_persisted` 是 `Option<String>`，旧 doc 读出 None
//!   不影响现有路由；
//! - **durable intent**: chunk/revision/document generation/job commit together;
//! - **空闲休眠**：取不到 job 时 sleep `interval_secs`，CPU 不空转；
//! - **crash recovery**: expired leases are reclaimed and stale owners are fenced;
//! - **零新依赖**：用 `tracing` + 既有 mongo accessor。

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::{FindOneAndUpdateOptions, ReturnDocument},
};
use tokio::time::sleep;

use crate::db::Database;
use crate::error::AppError;
use crate::knowledge_wiki::chunk_revisions::commit_chunk_transaction;
use crate::models::CatalogRebuildJob;

/// 单次循环最多领取的 job 数。避免 worker 在大批量 enqueue 时长占数据库连接。
const BATCH_SIZE: usize = 16;
const LEASE_SECONDS: i64 = 60;
const MAX_ATTEMPTS: i32 = 5;
const MAX_RETRY_DELAY_SECONDS: i64 = 300;

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
    tracing::info!(interval_secs, "catalog_rebuild_worker started");
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

/// Process at most one bounded batch. Queued/retry work and expired processing
/// leases are all claimable; every side effect is fenced by the returned token.
async fn drain_pending_jobs(db: &Database) -> Result<usize, AppError> {
    fail_exhausted_recoverable_jobs(db).await?;
    let worker = worker_id();
    let mut processed = 0usize;
    while processed < BATCH_SIZE {
        let claimed = claim_one_job(db, &worker).await?;
        let mut job = match claimed {
            Some(j) => j,
            None => break,
        };
        if job.target_generation <= 0 {
            match upgrade_legacy_claim(db, &job).await {
                Ok(upgraded) => job = upgraded,
                Err(error) => {
                    let message = error.to_string();
                    tracing::warn!(
                        job_id = %job.job_id,
                        error = %message,
                        "catalog legacy claim upgrade failed"
                    );
                    let _ = requeue_or_fail_owned_job(db, &job, &message).await;
                    processed += 1;
                    continue;
                }
            }
        }
        let heartbeat = spawn_claim_heartbeat(db.clone(), job.clone());
        let rendered = render_one_document(db, &job.workspace_id, job.document_id).await;
        match rendered {
            Ok(rendered) => {
                if let Err(error) = finalize_rendered_catalog(db, &job, &rendered).await {
                    let message = error.to_string();
                    tracing::warn!(
                        job_id = %job.job_id,
                        claim_generation = job.claim_generation,
                        error = %message,
                        "catalog rebuild finalize failed"
                    );
                    let _ = requeue_or_fail_owned_job(db, &job, &message).await;
                }
            }
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(job_id = %job.job_id, error = %message, "catalog rebuild job failed");
                let _ = requeue_or_fail_owned_job(db, &job, &message).await;
            }
        }
        heartbeat.abort();
        let _ = heartbeat.await;
        processed += 1;
    }
    Ok(processed)
}

/// Public single-batch entrypoint for recovery tests and operational repair.
/// Production normally reaches this through `catalog_rebuild_worker_loop`.
pub async fn run_catalog_rebuild_batch(db: &Database) -> Result<usize, AppError> {
    drain_pending_jobs(db).await
}

fn worker_id() -> String {
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{host}:{}:{}", std::process::id(), uuid::Uuid::new_v4())
}

fn lease_until(now: DateTime) -> DateTime {
    DateTime::from_millis(now.timestamp_millis() + LEASE_SECONDS * 1000)
}

fn retry_delay_seconds(attempts: i32) -> i64 {
    let exponent = attempts.saturating_sub(1).clamp(0, 8) as u32;
    (1_i64 << exponent).min(MAX_RETRY_DELAY_SECONDS)
}

fn claim_identity_filter(job: &CatalogRebuildJob) -> Option<Document> {
    let mut filter = doc! {
        "job_id": &job.job_id,
        "workspace_id": &job.workspace_id,
        "document_id": job.document_id,
        "status": "processing",
        "worker_id": job.worker_id.as_deref()?,
        "claim_token": job.claim_token.as_deref()?,
        "claim_generation": job.claim_generation,
    };
    if job.target_generation <= 0 {
        filter.insert(
            "$or",
            vec![
                doc! { "target_generation": { "$exists": false } },
                doc! { "target_generation": null },
                doc! { "target_generation": { "$lte": 0i64 } },
            ],
        );
    } else {
        filter.insert("target_generation", job.target_generation);
    }
    Some(filter)
}

async fn upgrade_legacy_claim(
    db: &Database,
    job: &CatalogRebuildJob,
) -> Result<CatalogRebuildJob, AppError> {
    const MAX_UPGRADE_ATTEMPTS: usize = 3;
    for attempt in 0..MAX_UPGRADE_ATTEMPTS {
        match upgrade_legacy_claim_once(db, job).await {
            Ok(upgraded) => return Ok(upgraded),
            Err(error)
                if attempt + 1 < MAX_UPGRADE_ATTEMPTS && retryable_finalize_error(&error) =>
            {
                tokio::task::yield_now().await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded catalog legacy-upgrade loop always returns")
}

async fn upgrade_legacy_claim_once(
    db: &Database,
    job: &CatalogRebuildJob,
) -> Result<CatalogRebuildJob, AppError> {
    let now = DateTime::now();
    let mut claim_filter = claim_identity_filter(job)
        .ok_or_else(|| AppError::Conflict("catalog_claim_missing_identity".to_string()))?;
    claim_filter.insert("locked_until", doc! { "$gte": now });
    let mut session = db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result: Result<Option<i64>, AppError> = async {
        if db
            .catalog_rebuild_jobs()
            .find_one_with_session(claim_filter.clone(), None, &mut session)
            .await?
            .is_none()
        {
            return Err(AppError::Conflict("catalog_claim_lost".to_string()));
        }
        let Some(parent) = db
            .operation_knowledge_documents()
            .find_one_with_session(
                doc! {
                    "_id": job.document_id,
                    "workspace_id": &job.workspace_id,
                },
                None,
                &mut session,
            )
            .await?
        else {
            return Ok(None);
        };
        let target = parent
            .catalog_desired_generation
            .max(parent.catalog_applied_generation)
            .checked_add(1)
            .ok_or_else(|| AppError::Conflict("catalog_generation_exhausted".to_string()))?;
        let desired_filter = if parent.catalog_desired_generation == 0 {
            doc! {
                "$or": [
                    { "catalog_desired_generation": 0i64 },
                    { "catalog_desired_generation": { "$exists": false } },
                    { "catalog_desired_generation": null },
                ]
            }
        } else {
            doc! { "catalog_desired_generation": parent.catalog_desired_generation }
        };
        let mut parent_filter = doc! {
            "_id": job.document_id,
            "workspace_id": &job.workspace_id,
        };
        parent_filter.extend(desired_filter);
        let advanced = db
            .operation_knowledge_documents()
            .update_one_with_session(
                parent_filter,
                doc! { "$set": { "catalog_desired_generation": target } },
                None,
                &mut session,
            )
            .await?;
        if advanced.matched_count != 1 {
            return Err(AppError::Conflict(
                "catalog_generation_conflict".to_string(),
            ));
        }
        let upgraded = db
            .catalog_rebuild_jobs()
            .update_one_with_session(
                claim_filter,
                doc! { "$set": { "target_generation": target } },
                None,
                &mut session,
            )
            .await?;
        if upgraded.matched_count != 1 {
            return Err(AppError::Conflict("catalog_claim_lost".to_string()));
        }
        Ok(Some(target))
    }
    .await;
    match result {
        Ok(target) => {
            commit_chunk_transaction(&mut session).await?;
            let mut upgraded = job.clone();
            if let Some(target) = target {
                upgraded.target_generation = target;
            }
            Ok(upgraded)
        }
        Err(error) => {
            let _ = session.abort_transaction().await;
            Err(error)
        }
    }
}

fn spawn_claim_heartbeat(db: Database, job: CatalogRebuildJob) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(Duration::from_secs((LEASE_SECONDS / 3).max(1) as u64));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = DateTime::now();
            let Some(mut filter) = claim_identity_filter(&job) else {
                return;
            };
            filter.insert("locked_until", doc! { "$gte": now });
            match db
                .catalog_rebuild_jobs()
                .update_one(
                    filter,
                    doc! { "$set": { "locked_until": lease_until(now) } },
                    None,
                )
                .await
            {
                Ok(result) if result.matched_count == 1 => {}
                Ok(_) => return,
                Err(error) => {
                    tracing::warn!(
                        job_id = %job.job_id,
                        error = %error,
                        "catalog rebuild heartbeat failed"
                    );
                }
            }
        }
    })
}

async fn fail_exhausted_recoverable_jobs(db: &Database) -> Result<(), AppError> {
    let now = DateTime::now();
    db.catalog_rebuild_jobs()
        .update_many(
            doc! {
                "attempts": { "$gte": MAX_ATTEMPTS },
                "$or": [
                    { "status": "queued" },
                    {
                        "status": "processing",
                        "$or": [
                            { "locked_until": { "$lt": now } },
                            { "locked_until": null },
                            { "locked_until": { "$exists": false } },
                        ],
                    },
                ],
            },
            doc! {
                "$set": {
                    "status": "failed",
                    "finished_at": now,
                    "last_error": "catalog rebuild attempts exhausted",
                },
                "$unset": {
                    "worker_id": "",
                    "claim_token": "",
                    "locked_until": "",
                    "next_retry_at": "",
                },
            },
            None,
        )
        .await?;
    Ok(())
}

/// Atomically claim new work or reclaim an expired worker. `next_retry_at`
/// gates ordinary retries; an expired processing lease is immediately eligible.
async fn claim_one_job(
    db: &Database,
    worker: &str,
) -> Result<Option<crate::models::CatalogRebuildJob>, AppError> {
    let now = DateTime::now();
    let token = uuid::Uuid::new_v4().to_string();
    let filter = doc! {
        "attempts": { "$lt": MAX_ATTEMPTS },
        "$or": [
            {
                "status": "queued",
                "$or": [
                    { "next_retry_at": { "$exists": false } },
                    { "next_retry_at": null },
                    { "next_retry_at": { "$lte": now } },
                ],
            },
            {
                "status": "processing",
                "$or": [
                    { "locked_until": { "$lt": now } },
                    { "locked_until": null },
                    { "locked_until": { "$exists": false } },
                ],
            },
            {
                "status": "failed",
                "$or": [
                    { "target_generation": { "$exists": false } },
                    { "target_generation": null },
                    { "target_generation": { "$lte": 0i64 } },
                ],
            },
        ],
    };
    let update = doc! {
        "$set": {
            "status": "processing",
            "worker_id": worker,
            "claim_token": &token,
            "locked_until": lease_until(now),
            "started_at": now,
        },
        "$inc": {
            "attempts": 1i32,
            "claim_generation": 1i64,
        },
        "$unset": {
            "finished_at": "",
            "next_retry_at": "",
        },
    };
    let opts = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .sort(doc! { "target_generation": 1, "queued_at": 1, "_id": 1 })
        .build();
    let claimed = db
        .catalog_rebuild_jobs()
        .find_one_and_update(filter, update, opts)
        .await?;
    Ok(claimed)
}

/// Render only. The persistent projection is written later in the same
/// transaction that consumes the active claim.
///
/// 步骤：
/// 1. 拉该 document 下所有 `status="active"` 的 chunk（不含 archived）；
/// 2. 按 `priority` desc 排序，渲染 markdown；
/// 3. `findOneAndUpdate documents._id == doc_id` `$set catalog_summary_persisted` `$inc catalog_version`。
async fn render_one_document(
    db: &Database,
    workspace_id: &str,
    document_id: ObjectId,
) -> Result<String, AppError> {
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

    Ok(render_persisted_catalog(&chunks))
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
        let id =
            c.id.map(|o| o.to_hex())
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
        let hits = c.usage_stats.as_ref().map(|u| u.hit_count_30d).unwrap_or(0);
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

fn active_claim_filter(job: &CatalogRebuildJob, now: DateTime) -> Result<Document, AppError> {
    let worker_id = job
        .worker_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("catalog_claim_missing_owner".to_string()))?;
    let claim_token = job
        .claim_token
        .as_deref()
        .ok_or_else(|| AppError::Conflict("catalog_claim_missing_token".to_string()))?;
    let mut filter = doc! {
        "job_id": &job.job_id,
        "workspace_id": &job.workspace_id,
        "document_id": job.document_id,
        "status": "processing",
        "worker_id": worker_id,
        "claim_token": claim_token,
        "claim_generation": job.claim_generation,
        "locked_until": { "$gte": now },
    };
    if job.target_generation <= 0 {
        filter.insert(
            "$or",
            vec![
                doc! { "target_generation": { "$exists": false } },
                doc! { "target_generation": null },
                doc! { "target_generation": { "$lte": 0i64 } },
            ],
        );
    } else {
        filter.insert("target_generation", job.target_generation);
    }
    Ok(filter)
}

async fn finalize_rendered_catalog(
    db: &Database,
    job: &CatalogRebuildJob,
    rendered: &str,
) -> Result<(), AppError> {
    const MAX_FINALIZE_ATTEMPTS: usize = 3;
    for attempt in 0..MAX_FINALIZE_ATTEMPTS {
        match finalize_rendered_catalog_once(db, job, rendered).await {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < MAX_FINALIZE_ATTEMPTS && retryable_finalize_error(&error) =>
            {
                tokio::task::yield_now().await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded catalog finalize loop always returns")
}

fn retryable_finalize_error(error: &AppError) -> bool {
    match error {
        AppError::Db(error) => error.contains_label("TransientTransactionError"),
        AppError::Conflict(code) => code == "catalog_generation_conflict",
        _ => false,
    }
}

async fn finalize_rendered_catalog_once(
    db: &Database,
    job: &CatalogRebuildJob,
    rendered: &str,
) -> Result<(), AppError> {
    let now = DateTime::now();
    let claim_filter = active_claim_filter(job, now)?;
    let mut session = db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result: Result<(), AppError> = async {
        let owned = db
            .catalog_rebuild_jobs()
            .find_one_with_session(claim_filter.clone(), None, &mut session)
            .await?;
        if owned.is_none() {
            return Err(AppError::Conflict("catalog_claim_lost".to_string()));
        }
        let Some(parent) = db
            .operation_knowledge_documents()
            .find_one_with_session(
                doc! {
                    "_id": job.document_id,
                    "workspace_id": &job.workspace_id,
                },
                None,
                &mut session,
            )
            .await?
        else {
            let discarded = db
                .catalog_rebuild_jobs()
                .update_one_with_session(
                    claim_filter,
                    doc! {
                        "$set": {
                            "status": "discarded",
                            "finished_at": now,
                            "last_error": "parent document no longer exists",
                        },
                        "$unset": {
                            "worker_id": "",
                            "claim_token": "",
                            "locked_until": "",
                            "next_retry_at": "",
                        },
                    },
                    None,
                    &mut session,
                )
                .await?;
            if discarded.matched_count != 1 {
                return Err(AppError::Conflict("catalog_claim_lost".to_string()));
            }
            return Ok(());
        };

        if parent.catalog_applied_generation >= job.target_generation
            || parent.catalog_desired_generation > job.target_generation
        {
            let superseded = db
                .catalog_rebuild_jobs()
                .update_one_with_session(
                    claim_filter,
                    doc! {
                        "$set": {
                            "status": "superseded",
                            "finished_at": now,
                            "last_error": null,
                        },
                        "$unset": {
                            "worker_id": "",
                            "claim_token": "",
                            "locked_until": "",
                            "next_retry_at": "",
                        },
                    },
                    None,
                    &mut session,
                )
                .await?;
            if superseded.matched_count != 1 {
                return Err(AppError::Conflict("catalog_claim_lost".to_string()));
            }
            return Ok(());
        }

        let updated = db
            .operation_knowledge_documents()
            .update_one_with_session(
                doc! {
                    "_id": job.document_id,
                    "workspace_id": &job.workspace_id,
                    "catalog_desired_generation": job.target_generation,
                    "catalog_applied_generation": parent.catalog_applied_generation,
                },
                doc! {
                    "$set": {
                        "catalog_summary_persisted": rendered,
                        "catalog_applied_generation": job.target_generation,
                    },
                    "$inc": { "catalog_version": 1i64 },
                },
                None,
                &mut session,
            )
            .await?;
        if updated.matched_count != 1 {
            return Err(AppError::Conflict(
                "catalog_generation_conflict".to_string(),
            ));
        }
        let finished = db
            .catalog_rebuild_jobs()
            .update_one_with_session(
                claim_filter,
                doc! {
                    "$set": {
                        "status": "done",
                        "finished_at": now,
                        "last_error": null,
                    },
                    "$unset": {
                        "worker_id": "",
                        "claim_token": "",
                        "locked_until": "",
                        "next_retry_at": "",
                    },
                },
                None,
                &mut session,
            )
            .await?;
        if finished.matched_count != 1 {
            return Err(AppError::Conflict("catalog_claim_lost".to_string()));
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => commit_chunk_transaction(&mut session).await,
        Err(error) => {
            let _ = session.abort_transaction().await;
            Err(error)
        }
    }
}

async fn requeue_or_fail_owned_job(
    db: &Database,
    job: &CatalogRebuildJob,
    error: &str,
) -> Result<(), AppError> {
    let now = DateTime::now();
    let filter = active_claim_filter(job, now)?;
    let (status, next_retry_at) = if job.attempts >= MAX_ATTEMPTS {
        ("failed", None)
    } else {
        (
            "queued",
            Some(DateTime::from_millis(
                now.timestamp_millis()
                    .saturating_add(retry_delay_seconds(job.attempts) * 1000),
            )),
        )
    };
    let mut set = doc! {
        "status": status,
        "last_error": error,
    };
    if let Some(next_retry_at) = next_retry_at {
        set.insert("next_retry_at", next_retry_at);
    } else {
        set.insert("finished_at", now);
    }
    let result = db
        .catalog_rebuild_jobs()
        .update_one(
            filter,
            doc! {
                "$set": set,
                "$unset": {
                    "worker_id": "",
                    "claim_token": "",
                    "locked_until": "",
                },
            },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict("catalog_claim_lost".to_string()));
    }
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

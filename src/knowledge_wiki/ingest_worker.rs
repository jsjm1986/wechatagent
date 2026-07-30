//! `ingest_worker` —— Phase G P1-6 外部源自动 ingest 主循环。
//!
//! 一轮职责：
//! 1. 跨 workspace 扫所有 `status="active"` 的 [`crate::models::IngestSource`]；
//! 2. 距上次拉取 ≥ `schedule_minutes` 的 source 才发起 GET（自身节流叠加 worker tick）；
//! 3. 带 `If-None-Match: <last_etag>` 条件 GET；304 → 仅刷 last_fetched_at；
//! 4. 200 → 按 kind 走 `feed-rs`（rss）/ `scraper` + 启发式正文抽取（html）→ markdown；
//! 5. 调 [`crate::routes::knowledge::ingest_chunked_text`]，所有 chunk 默认 draft +
//!    integrity_status="needs_review"（红线"AI 永不自动 verify"）；
//! 6. 失败 →failure_streak += 1；连续 3 次 failure_streak ≥ 3 → status="failing"；
//!    7 天不可达（last_fetched_at 距 now > 168h）→ status="disabled"。
//!
//! 关停态：`INGEST_WORKER_INTERVAL_SECONDS=0` 或 `INGEST_WORKER_ENABLED=false`
//! → main.rs 不 spawn / loop 直接 return。

use std::time::Duration;

use chrono::Utc;
use mongodb::{
    bson::{doc, DateTime as BsonDateTime, Document},
    options::{FindOneAndUpdateOptions, ReturnDocument},
};
use sha2::{Digest, Sha256};
use tokio::time::sleep;

use crate::error::{AppError, AppResult};
use crate::knowledge_wiki::chunk_revisions::commit_chunk_transaction;
use crate::models::IngestSource;
use crate::routes::AppState;

const FAILURE_STREAK_TO_FAILING: i32 = 3;
const UNREACHABLE_DISABLE_HOURS: i64 = 24 * 7;
const LEASE_SECONDS: i64 = 120;

/// auto-ingest worker 主循环。`interval_secs == 0` 直接 return（与 feedback_worker 同形）。
pub async fn ingest_worker_loop(state: AppState, interval_secs: u64) {
    if interval_secs == 0 {
        tracing::info!("ingest_worker disabled (interval=0)");
        return;
    }
    tracing::info!("ingest_worker started (interval={}s)", interval_secs);
    loop {
        if let Err(err) = run_one_round(&state).await {
            tracing::warn!(?err, "ingest_worker round failed");
        }
        sleep(Duration::from_secs(interval_secs)).await;
    }
}

/// 跑一轮 ingest（扫所有 workspace → 拉取 → 解析 → 落库）。
/// `pub` 仅为集成测试（`tests/ingest_worker_smoke.rs`）能用 wiremock 驱动单轮；
/// 生产路径只经 [`ingest_worker_loop`]。
pub async fn run_one_round(state: &AppState) -> anyhow::Result<()> {
    let workspaces = list_workspaces(state).await?;
    let worker_id = worker_id();
    for ws in workspaces {
        let sources = list_active_sources(state, &ws).await?;
        for candidate in sources {
            // not-due:本轮没到点、未发请求，也不领取 lease。
            if !is_due(&candidate) {
                continue;
            }
            let Some(src) = claim_source(state, &candidate, &worker_id).await? else {
                continue;
            };
            let heartbeat = spawn_claim_heartbeat(state.clone(), src.clone());
            let fetched = fetch_source(&src).await;
            heartbeat.abort();
            let _ = heartbeat.await;
            // Keep heartbeat writes outside the source-finalization
            // transaction. A final owner-CAS renewal both gives the database
            // transaction a fresh lease window and proves this worker still
            // owns the fetched result before it starts writing knowledge.
            if let Err(error) = renew_claim(state, &src).await {
                tracing::warn!(
                    workspace_id = %src.workspace_id,
                    source_id = %src.source_id,
                    claim_generation = src.claim_generation,
                    error = %error,
                    "ingest source claim lost after fetch"
                );
                continue;
            }
            let fetched_failed = fetched.is_err();
            let result = match fetched {
                Ok(SourceOutcome::NotModified) => {
                    finalize_without_content(state, &src, None, None).await
                }
                Ok(SourceOutcome::Fetched {
                    etag,
                    content_hash,
                    markdown,
                }) => {
                    if should_ingest_content(src.last_content_hash.as_deref(), &content_hash) {
                        finalize_ingested_content(state, &src, &markdown, etag, &content_hash).await
                    } else {
                        finalize_without_content(state, &src, etag, Some(&content_hash)).await
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        workspace_id = %src.workspace_id,
                        source_id = %src.source_id,
                        ?err,
                        "ingest_source failed"
                    );
                    mark_failure(state, &src, &err.to_string()).await
                }
            };
            if let Err(error) = result {
                tracing::warn!(
                    workspace_id = %src.workspace_id,
                    source_id = %src.source_id,
                    claim_generation = src.claim_generation,
                    error = %error,
                    "ingest source claim did not finalize"
                );
                // A fetch/parse failure has already gone through the only
                // claim-owned failure finalizer. Finalize/transaction errors
                // may still release the live claim as a failed attempt.
                if !fetched_failed {
                    let _ = mark_failure(state, &src, &error.to_string()).await;
                }
            }
        }
    }
    Ok(())
}

enum SourceOutcome {
    NotModified,
    Fetched {
        etag: Option<String>,
        content_hash: String,
        markdown: String,
    },
}

async fn fetch_source(src: &IngestSource) -> anyhow::Result<SourceOutcome> {
    let fetched =
        crate::outbound_fetch::fetch_ingest_url(&src.url, src.last_etag.as_deref(), &src.kind)
            .await?;
    if fetched.status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(SourceOutcome::NotModified);
    }
    if !fetched.status.is_success() {
        anyhow::bail!("http {} from {}", fetched.status, src.url);
    }
    let etag = fetched.etag;
    let body_bytes = fetched.body;
    let markdown = match src.kind.as_str() {
        "rss" => render_rss_to_markdown(&body_bytes)?,
        "html" => render_html_to_markdown(&body_bytes)?,
        other => anyhow::bail!("unknown ingest source kind: {other}"),
    };
    if markdown.trim().is_empty() {
        anyhow::bail!("empty parsed body");
    }
    let content_hash = content_sha256(&markdown);
    Ok(SourceOutcome::Fetched {
        etag,
        content_hash,
        markdown,
    })
}

fn worker_id() -> String {
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{host}:{}:{}", std::process::id(), uuid::Uuid::new_v4())
}

fn lease_until(now: BsonDateTime) -> BsonDateTime {
    BsonDateTime::from_millis(now.timestamp_millis() + LEASE_SECONDS * 1000)
}

fn source_generation_filter(generation: i64) -> Document {
    if generation == 0 {
        doc! { "$or": [
            { "source_generation": 0i64 },
            { "source_generation": null },
            { "source_generation": { "$exists": false } },
        ] }
    } else {
        doc! { "source_generation": generation }
    }
}

async fn claim_source(
    state: &AppState,
    candidate: &IngestSource,
    worker_id: &str,
) -> anyhow::Result<Option<IngestSource>> {
    let now = BsonDateTime::now();
    let mut filter = doc! {
        "source_id": &candidate.source_id,
        "workspace_id": &candidate.workspace_id,
        "updated_at": candidate.updated_at,
        "status": { "$in": ["active", "failing"] },
        "$and": [{ "$or": [
            { "locked_until": { "$lt": now } },
            { "locked_until": null },
            { "locked_until": { "$exists": false } },
        ] }],
    };
    filter.extend(source_generation_filter(candidate.source_generation));
    let claimed = state
        .db
        .ingest_sources()
        .find_one_and_update(
            filter,
            doc! {
                "$set": {
                    // Materialize the serde-default generation for a source
                    // created by an old process after m053 already ran.
                    "source_generation": candidate.source_generation,
                    "worker_id": worker_id,
                    "claim_token": uuid::Uuid::new_v4().to_string(),
                    "locked_until": lease_until(now),
                },
                "$inc": { "claim_generation": 1i64 },
            },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    Ok(claimed)
}

fn claim_identity_filter(src: &IngestSource, require_live_lease: bool) -> AppResult<Document> {
    let mut filter = doc! {
        "source_id": &src.source_id,
        "workspace_id": &src.workspace_id,
        "source_generation": src.source_generation,
        // Both new and rolling-deploy legacy CRUD paths update this field.
        // Binding it fences a late fetch even if an old process did not know
        // how to increment source_generation or revoke a claim. Bind the
        // fetched configuration itself as well, so even a same-millisecond
        // legacy update cannot authorize a result from the old URL/config.
        "updated_at": src.updated_at,
        "url": &src.url,
        "kind": &src.kind,
        "schedule_minutes": src.schedule_minutes,
        "label": src.label.as_deref(),
        "status": &src.status,
        "claim_generation": src.claim_generation,
        "worker_id": src.worker_id.as_deref().ok_or_else(|| {
            AppError::Conflict("ingest_claim_missing_worker".to_string())
        })?,
        "claim_token": src.claim_token.as_deref().ok_or_else(|| {
            AppError::Conflict("ingest_claim_missing_token".to_string())
        })?,
    };
    if require_live_lease {
        filter.insert("locked_until", doc! { "$gte": BsonDateTime::now() });
    }
    Ok(filter)
}

fn spawn_claim_heartbeat(state: AppState, src: IngestSource) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs((LEASE_SECONDS / 3) as u64));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let Ok(mut filter) = claim_identity_filter(&src, false) else {
                return;
            };
            let now = BsonDateTime::now();
            filter.insert("locked_until", doc! { "$gte": now });
            match state
                .db
                .ingest_sources()
                .update_one(
                    filter,
                    doc! { "$set": { "locked_until": lease_until(now) } },
                    None,
                )
                .await
            {
                Ok(result) if result.matched_count == 1 => {}
                Ok(_) => return,
                Err(error) => tracing::warn!(
                    source_id = %src.source_id,
                    error = %error,
                    "ingest source heartbeat failed"
                ),
            }
        }
    })
}

async fn renew_claim(state: &AppState, src: &IngestSource) -> AppResult<()> {
    let now = BsonDateTime::now();
    let mut filter = claim_identity_filter(src, false)?;
    filter.insert("locked_until", doc! { "$gte": now });
    let renewed = state
        .db
        .ingest_sources()
        .update_one(
            filter,
            doc! { "$set": { "locked_until": lease_until(now) } },
            None,
        )
        .await?;
    if renewed.matched_count != 1 {
        return Err(AppError::Conflict("ingest_claim_lost".to_string()));
    }
    Ok(())
}

fn clear_claim_update() -> Document {
    doc! {
        "worker_id": "",
        "claim_token": "",
        "locked_until": "",
    }
}

async fn finalize_without_content(
    state: &AppState,
    src: &IngestSource,
    etag: Option<String>,
    content_hash: Option<&str>,
) -> AppResult<()> {
    let now = BsonDateTime::now();
    let mut set = doc! {
        "last_fetched_at": now,
        "last_error": null,
        "failure_streak": 0,
        "status": "active",
        "updated_at": now,
    };
    if let Some(etag) = etag {
        set.insert("last_etag", etag);
    }
    if let Some(content_hash) = content_hash {
        set.insert("last_content_hash", content_hash);
    }
    let result = state
        .db
        .ingest_sources()
        .update_one(
            claim_identity_filter(src, true)?,
            doc! {
                "$set": set,
                "$unset": clear_claim_update(),
            },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict("ingest_claim_lost".to_string()));
    }
    Ok(())
}

async fn finalize_ingested_content(
    state: &AppState,
    src: &IngestSource,
    markdown: &str,
    etag: Option<String>,
    content_hash: &str,
) -> AppResult<()> {
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 0..MAX_ATTEMPTS {
        match finalize_ingested_content_once(state, src, markdown, etag.clone(), content_hash).await
        {
            Ok(()) => return Ok(()),
            Err(error) if attempt + 1 < MAX_ATTEMPTS && retryable_transaction_error(&error) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded ingest finalize loop always returns")
}

fn retryable_transaction_error(error: &AppError) -> bool {
    matches!(error, AppError::Db(db_error) if
        db_error.contains_label("TransientTransactionError")
            || is_duplicate_key_error(db_error))
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};

    match &*error.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            matches!(write_error.code, 11000 | 11001)
        }
        ErrorKind::BulkWrite(bulk) => bulk.write_errors.as_ref().is_some_and(|errors| {
            errors
                .iter()
                .any(|write_error| matches!(write_error.code, 11000 | 11001))
        }),
        _ => false,
    }
}

async fn finalize_ingested_content_once(
    state: &AppState,
    src: &IngestSource,
    markdown: &str,
    etag: Option<String>,
    content_hash: &str,
) -> AppResult<()> {
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result: AppResult<()> = async {
        let claim_filter = claim_identity_filter(src, true)?;
        if state
            .db
            .ingest_sources()
            .find_one_with_session(claim_filter.clone(), None, &mut session)
            .await?
            .is_none()
        {
            return Err(AppError::Conflict("ingest_claim_lost".to_string()));
        }
        let source_name = src
            .label
            .clone()
            .unwrap_or_else(|| format!("{} · {}", src.kind, src.url));
        let outcome = crate::routes::knowledge::ingest_chunked_text_with_session(
            state,
            &src.workspace_id,
            None,
            &source_name,
            markdown,
            &mut session,
        )
        .await?;
        let now = BsonDateTime::now();
        let mut set = doc! {
            "last_fetched_at": now,
            "last_error": null,
            "failure_streak": 0,
            "status": "active",
            "updated_at": now,
            "last_content_hash": content_hash,
        };
        if let Some(etag) = etag {
            set.insert("last_etag", etag);
        }
        let finalized = state
            .db
            .ingest_sources()
            .update_one_with_session(
                claim_filter,
                doc! {
                    "$set": set,
                    "$inc": { "ingest_count": outcome.chunk_ids.len() as i64 },
                    "$unset": clear_claim_update(),
                },
                None,
                &mut session,
            )
            .await?;
        if finalized.matched_count != 1 {
            return Err(AppError::Conflict("ingest_claim_lost".to_string()));
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

fn is_due(src: &IngestSource) -> bool {
    let Some(last) = src.last_fetched_at else {
        return true;
    };
    let now_ms = Utc::now().timestamp_millis();
    let last_ms = last.timestamp_millis();
    let elapsed_min = (now_ms - last_ms) / 60_000;
    elapsed_min >= src.schedule_minutes.max(1)
}

fn content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn should_ingest_content(last_content_hash: Option<&str>, content_hash: &str) -> bool {
    last_content_hash != Some(content_hash)
}

fn render_rss_to_markdown(body: &[u8]) -> anyhow::Result<String> {
    let feed = feed_rs::parser::parse(body)?;
    let mut out = String::new();
    for (idx, entry) in feed.entries.iter().take(50).enumerate() {
        let title = entry
            .title
            .as_ref()
            .map(|t| t.content.trim())
            .unwrap_or("(no title)");
        let link = entry.links.first().map(|l| l.href.as_str()).unwrap_or("");
        let summary = entry
            .summary
            .as_ref()
            .map(|s| s.content.trim().to_string())
            .or_else(|| entry.content.as_ref().and_then(|c| c.body.clone()))
            .unwrap_or_default();
        // 跳过既无标题又无正文的空条目（block_parser 会因 body/summary/answer 全空丢弃）。
        if title.trim().is_empty() && summary.trim().is_empty() {
            continue;
        }
        // block_parser 要求：fence id 安全（entry.id 常是 URL，含 `:` `/` 不安全）→
        // 用稳定 idx 派生安全 id；body 必须是 JSON object 且 body/summary/answer 至少一个非空；
        // fence 终止符必须是 `---END CHUNK---`。
        let block_body = if summary.is_empty() {
            // 无正文时把标题塞进 body，保证非空（否则被 block_parser 当空块丢弃）。
            title.to_string()
        } else {
            summary.clone()
        };
        let payload = serde_json::json!({
            "title": title,
            "summary": if summary.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(summary.clone()) },
            "body": block_body,
            "businessContext": if link.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(format!("source: {link}")) },
        });
        out.push_str(&format!("---CHUNK: rss-{idx}---\n"));
        out.push_str(&serde_json::to_string(&payload)?);
        out.push('\n');
        out.push_str("---END CHUNK---\n\n");
    }
    Ok(out)
}

fn render_html_to_markdown(body: &[u8]) -> anyhow::Result<String> {
    let html = std::str::from_utf8(body).map_err(|e| anyhow::anyhow!("html not utf8: {e}"))?;
    let doc = scraper::Html::parse_document(html);
    let title_sel = scraper::Selector::parse("title").unwrap();
    let body_sel = scraper::Selector::parse("article, main, [role=main], .content, body").unwrap();
    let title = doc
        .select(&title_sel)
        .next()
        .map(|n| n.text().collect::<String>().trim().to_string())
        .unwrap_or_else(|| "imported page".to_string());
    let body_text = doc
        .select(&body_sel)
        .next()
        .map(|n| {
            n.text()
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    if body_text.is_empty() {
        anyhow::bail!("html body empty after extraction");
    }
    // 整页归一为单个 fence block（block_parser 要求 JSON object body + `---END CHUNK---`
    // 终止符 + 安全 id）。落库后由 ingest_chunked_text 强制 draft + needs_review。
    let payload = serde_json::json!({
        "title": title,
        "body": body_text,
    });
    let mut out = String::new();
    out.push_str("---CHUNK: html-page---\n");
    out.push_str(&serde_json::to_string(&payload)?);
    out.push('\n');
    out.push_str("---END CHUNK---\n\n");
    Ok(out)
}

async fn list_workspaces(state: &AppState) -> anyhow::Result<Vec<String>> {
    let cursor = state
        .db
        .ingest_sources()
        .distinct("workspace_id", None, None)
        .await?;
    let workspaces: Vec<String> = cursor
        .into_iter()
        .filter_map(|b| b.as_str().map(String::from))
        .collect();
    Ok(workspaces)
}

async fn list_active_sources(state: &AppState, ws: &str) -> anyhow::Result<Vec<IngestSource>> {
    // active + failing 都纳入扫描：failing 源继续重试 → 成功则 mark_success 复位 active，
    // 持续不可达则 mark_failure 推进到 disabled。disabled 才真正停扫（需 admin 手动复活）。
    let mut cursor = state
        .db
        .ingest_sources()
        .find(
            doc! { "workspace_id": ws, "status": { "$in": ["active", "failing"] } },
            None,
        )
        .await?;
    let mut out = Vec::new();
    use futures::TryStreamExt;
    while let Some(src) = cursor.try_next().await? {
        out.push(src);
    }
    Ok(out)
}

async fn mark_failure(state: &AppState, src: &IngestSource, err: &str) -> AppResult<()> {
    let new_streak = src.failure_streak + 1;
    let mut new_status = src.status.clone();
    if new_streak >= FAILURE_STREAK_TO_FAILING && new_status == "active" {
        new_status = "failing".to_string();
    }
    if let Some(last) = src.last_fetched_at {
        let now_ms = Utc::now().timestamp_millis();
        let last_ms = last.timestamp_millis();
        if (now_ms - last_ms) / 3_600_000 > UNREACHABLE_DISABLE_HOURS {
            new_status = "disabled".to_string();
        }
    }
    let result = state
        .db
        .ingest_sources()
        .update_one(
            claim_identity_filter(src, true)?,
            doc! {
                "$set": {
                    "last_error": err.chars().take(500).collect::<String>(),
                    "failure_streak": new_streak,
                    "status": new_status,
                    "updated_at": BsonDateTime::now(),
                },
                "$unset": clear_claim_update(),
            },
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict("ingest_claim_lost".to_string()));
    }
    Ok(())
}

/// Narrow production-protocol harness used by replica-set redlines. It loads
/// the current source, applies the same due check as the worker, and invokes
/// the real atomic claim path; no network or test-only database behavior is
/// introduced.
#[doc(hidden)]
pub async fn claim_due_source_for_redline(
    state: &AppState,
    source_id: &str,
    worker_id: &str,
) -> AppResult<Option<IngestSource>> {
    let Some(candidate) = state
        .db
        .ingest_sources()
        .find_one(doc! { "source_id": source_id }, None)
        .await?
    else {
        return Ok(None);
    };
    if !is_due(&candidate) {
        return Ok(None);
    }
    claim_source(state, &candidate, worker_id)
        .await
        .map_err(|error| AppError::External(error.to_string()))
}

/// Commit already-fetched parsed content through the same fenced transaction
/// used by the production worker. The content hash is derived server-side so
/// tests cannot bypass checkpoint identity.
#[doc(hidden)]
pub async fn finalize_claimed_content_for_redline(
    state: &AppState,
    claim: &IngestSource,
    markdown: &str,
) -> AppResult<()> {
    renew_claim(state, claim).await?;
    finalize_ingested_content(state, claim, markdown, None, &content_sha256(markdown)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_source(last_fetched: Option<i64>, schedule_min: i64) -> IngestSource {
        IngestSource {
            id: None,
            source_id: "ing_t".into(),
            workspace_id: "ws_a".into(),
            source_generation: 1,
            claim_generation: 0,
            worker_id: None,
            claim_token: None,
            locked_until: None,
            kind: "rss".into(),
            url: "https://example.com/feed.xml".into(),
            label: None,
            schedule_minutes: schedule_min,
            last_fetched_at: last_fetched.map(BsonDateTime::from_millis),
            last_etag: None,
            last_content_hash: None,
            status: "active".into(),
            failure_streak: 0,
            last_error: None,
            ingest_count: 0,
            created_at: BsonDateTime::now(),
            updated_at: BsonDateTime::now(),
        }
    }

    #[test]
    fn is_due_when_never_fetched() {
        let src = sample_source(None, 60);
        assert!(is_due(&src));
    }

    #[test]
    fn is_due_respects_schedule_minutes() {
        let just_now = Utc::now().timestamp_millis();
        let two_hours_ago = just_now - 2 * 3_600_000;
        // 60 分钟节流：刚拉过 → 不应再拉
        let recent = sample_source(Some(just_now - 30 * 60_000), 60);
        assert!(!is_due(&recent));
        // 60 分钟节流：2 小时前 → 应拉
        let stale = sample_source(Some(two_hours_ago), 60);
        assert!(is_due(&stale));
    }

    #[test]
    fn is_due_zero_or_negative_schedule_clamps_to_one_minute() {
        let just_now = Utc::now().timestamp_millis();
        // schedule_minutes=0 走 .max(1) 兜底；30s 前 → 仍 < 1min → not due
        let fresh = sample_source(Some(just_now - 30_000), 0);
        assert!(!is_due(&fresh));
        // 2 分钟前 → due
        let stale = sample_source(Some(just_now - 2 * 60_000), 0);
        assert!(is_due(&stale));
    }

    #[test]
    fn unchanged_content_hash_is_not_ingested_again() {
        let hash = content_sha256("same parsed content");
        assert!(!should_ingest_content(Some(&hash), &hash));
    }

    #[test]
    fn missing_or_changed_content_hash_is_ingested() {
        let first = content_sha256("first parsed content");
        let changed = content_sha256("changed parsed content");

        assert!(should_ingest_content(None, &first));
        assert!(should_ingest_content(Some(&first), &changed));
    }

    #[test]
    fn render_rss_extracts_title_and_entries() {
        let rss = br#"<?xml version="1.0"?>
<rss version="2.0">
  <channel>
    <title>Example Feed</title>
    <item>
      <guid>item-1</guid>
      <title>First post</title>
      <link>https://example.com/1</link>
      <description>Hello world</description>
    </item>
    <item>
      <guid>item-2</guid>
      <title>Second post</title>
      <link>https://example.com/2</link>
      <description>More text</description>
    </item>
  </channel>
</rss>"#;
        let md = render_rss_to_markdown(rss).expect("parse rss");
        // 新 fence 形态：每条目一个 `---CHUNK: rss-<idx>---` + JSON body + `---END CHUNK---`。
        assert!(
            md.contains("---CHUNK: rss-0---"),
            "chunk fence missing: {md}"
        );
        assert!(
            md.contains("---END CHUNK---"),
            "END CHUNK terminator missing: {md}"
        );
        assert!(md.contains("First post"));
        assert!(md.contains("Second post"));
        // 红线回归：渲染产物必须能被 block_parser 解析成离散 chunk（旧 `---END---` bug 会退化为 0 块）。
        let (blocks, warnings) = crate::knowledge_wiki::block_parser::parse_chunk_blocks(&md);
        assert_eq!(
            blocks.len(),
            2,
            "expected 2 discrete chunks, got {}: {md}",
            blocks.len()
        );
        assert!(
            warnings.items.is_empty(),
            "unexpected parse warnings: {:?}",
            warnings.items
        );
    }

    #[test]
    fn render_rss_rejects_garbage() {
        let bad = b"not actually a feed";
        assert!(render_rss_to_markdown(bad).is_err());
    }

    #[test]
    fn render_html_extracts_article_text() {
        let html = br#"<!doctype html>
<html><head><title>Page Title</title></head>
<body>
  <nav>nav noise</nav>
  <article>
    <h1>Heading</h1>
    <p>First paragraph body.</p>
    <p>Second paragraph body.</p>
  </article>
</body></html>"#;
        let md = render_html_to_markdown(html).expect("parse html");
        assert!(
            md.contains("---CHUNK: html-page---"),
            "chunk fence missing: {md}"
        );
        assert!(
            md.contains("---END CHUNK---"),
            "END CHUNK terminator missing: {md}"
        );
        assert!(md.contains("Page Title"));
        assert!(md.contains("First paragraph body."));
        assert!(md.contains("Second paragraph body."));
        // 红线回归：单页归一为 1 个离散 chunk 且无 warning。
        let (blocks, warnings) = crate::knowledge_wiki::block_parser::parse_chunk_blocks(&md);
        assert_eq!(
            blocks.len(),
            1,
            "expected 1 chunk, got {}: {md}",
            blocks.len()
        );
        assert!(
            warnings.items.is_empty(),
            "unexpected parse warnings: {:?}",
            warnings.items
        );
    }

    #[test]
    fn render_html_rejects_empty_body() {
        let html = br#"<!doctype html><html><head><title>x</title></head><body></body></html>"#;
        assert!(render_html_to_markdown(html).is_err());
    }
}

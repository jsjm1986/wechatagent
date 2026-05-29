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
use mongodb::bson::{doc, DateTime as BsonDateTime};
use tokio::time::sleep;

use crate::models::IngestSource;
use crate::routes::AppState;

const FAILURE_STREAK_TO_FAILING: i32 = 3;
const UNREACHABLE_DISABLE_HOURS: i64 = 24 * 7;

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
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("wechatagent-ingest/1.0")
        .build()?;
    for ws in workspaces {
        let sources = list_active_sources(state, &ws).await?;
        for src in sources {
            match process_source(state, &client, &src).await {
                Ok(SourceOutcome::NotModified) => {
                    let _ = mark_success(state, &src, 0).await;
                }
                Ok(SourceOutcome::Ingested {
                    chunk_count,
                    etag,
                }) => {
                    let _ = mark_success_with_etag(state, &src, chunk_count, etag).await;
                }
                Err(err) => {
                    tracing::warn!(
                        workspace_id = %src.workspace_id,
                        source_id = %src.source_id,
                        ?err,
                        "ingest_source failed"
                    );
                    let _ = mark_failure(state, &src, &err.to_string()).await;
                }
            }
        }
    }
    Ok(())
}

enum SourceOutcome {
    NotModified,
    Ingested {
        chunk_count: usize,
        etag: Option<String>,
    },
}

async fn process_source(
    state: &AppState,
    client: &reqwest::Client,
    src: &IngestSource,
) -> anyhow::Result<SourceOutcome> {
    if !is_due(src) {
        return Ok(SourceOutcome::NotModified);
    }
    let mut req = client.get(&src.url);
    if let Some(etag) = &src.last_etag {
        req = req.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let resp = req.send().await?;
    if resp.status().as_u16() == 304 {
        return Ok(SourceOutcome::NotModified);
    }
    if !resp.status().is_success() {
        anyhow::bail!("http {} from {}", resp.status(), src.url);
    }
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let body_bytes = resp.bytes().await?;
    let markdown = match src.kind.as_str() {
        "rss" => render_rss_to_markdown(&body_bytes)?,
        "html" => render_html_to_markdown(&body_bytes)?,
        other => anyhow::bail!("unknown ingest source kind: {other}"),
    };
    if markdown.trim().is_empty() {
        anyhow::bail!("empty parsed body");
    }
    let source_name = src
        .label
        .clone()
        .unwrap_or_else(|| format!("{} · {}", src.kind, src.url));
    let outcome = crate::routes::knowledge::ingest_chunked_text(
        state,
        &src.workspace_id,
        None,
        &source_name,
        &markdown,
    )
    .await
    .map_err(|e| anyhow::anyhow!("ingest_chunked_text failed: {e}"))?;
    Ok(SourceOutcome::Ingested {
        chunk_count: outcome.chunk_ids.len(),
        etag,
    })
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

fn render_rss_to_markdown(body: &[u8]) -> anyhow::Result<String> {
    let feed = feed_rs::parser::parse(body)?;
    let mut out = String::new();
    for (idx, entry) in feed.entries.iter().take(50).enumerate() {
        let title = entry
            .title
            .as_ref()
            .map(|t| t.content.trim())
            .unwrap_or("(no title)");
        let link = entry
            .links
            .first()
            .map(|l| l.href.as_str())
            .unwrap_or("");
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
    let html = std::str::from_utf8(body)
        .map_err(|e| anyhow::anyhow!("html not utf8: {e}"))?;
    let doc = scraper::Html::parse_document(html);
    let title_sel = scraper::Selector::parse("title").unwrap();
    let body_sel =
        scraper::Selector::parse("article, main, [role=main], .content, body").unwrap();
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

async fn list_active_sources(
    state: &AppState,
    ws: &str,
) -> anyhow::Result<Vec<IngestSource>> {
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

async fn mark_success(state: &AppState, src: &IngestSource, chunk_count: usize) -> anyhow::Result<()> {
    state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": &src.source_id },
            doc! {
                "$set": {
                    "last_fetched_at": BsonDateTime::now(),
                    "last_error": null,
                    "failure_streak": 0,
                    "status": "active",
                    "updated_at": BsonDateTime::now(),
                },
                "$inc": { "ingest_count": chunk_count as i64 },
            },
            None,
        )
        .await?;
    Ok(())
}

async fn mark_success_with_etag(
    state: &AppState,
    src: &IngestSource,
    chunk_count: usize,
    etag: Option<String>,
) -> anyhow::Result<()> {
    let mut set_doc = doc! {
        "last_fetched_at": BsonDateTime::now(),
        "last_error": null,
        "failure_streak": 0,
        "status": "active",
        "updated_at": BsonDateTime::now(),
    };
    if let Some(e) = etag {
        set_doc.insert("last_etag", e);
    }
    state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": &src.source_id },
            doc! {
                "$set": set_doc,
                "$inc": { "ingest_count": chunk_count as i64 },
            },
            None,
        )
        .await?;
    Ok(())
}

async fn mark_failure(
    state: &AppState,
    src: &IngestSource,
    err: &str,
) -> anyhow::Result<()> {
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
    state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": &src.source_id },
            doc! {
                "$set": {
                    "last_error": err.chars().take(500).collect::<String>(),
                    "failure_streak": new_streak,
                    "status": new_status,
                    "updated_at": BsonDateTime::now(),
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

    fn sample_source(last_fetched: Option<i64>, schedule_min: i64) -> IngestSource {
        IngestSource {
            id: None,
            source_id: "ing_t".into(),
            workspace_id: "ws_a".into(),
            kind: "rss".into(),
            url: "https://example.com/feed.xml".into(),
            label: None,
            schedule_minutes: schedule_min,
            last_fetched_at: last_fetched.map(BsonDateTime::from_millis),
            last_etag: None,
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
        assert!(md.contains("---CHUNK: rss-0---"), "chunk fence missing: {md}");
        assert!(md.contains("---END CHUNK---"), "END CHUNK terminator missing: {md}");
        assert!(md.contains("First post"));
        assert!(md.contains("Second post"));
        // 红线回归：渲染产物必须能被 block_parser 解析成离散 chunk（旧 `---END---` bug 会退化为 0 块）。
        let (blocks, warnings) = crate::knowledge_wiki::block_parser::parse_chunk_blocks(&md);
        assert_eq!(blocks.len(), 2, "expected 2 discrete chunks, got {}: {md}", blocks.len());
        assert!(warnings.items.is_empty(), "unexpected parse warnings: {:?}", warnings.items);
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
        assert!(md.contains("---CHUNK: html-page---"), "chunk fence missing: {md}");
        assert!(md.contains("---END CHUNK---"), "END CHUNK terminator missing: {md}");
        assert!(md.contains("Page Title"));
        assert!(md.contains("First paragraph body."));
        assert!(md.contains("Second paragraph body."));
        // 红线回归：单页归一为 1 个离散 chunk 且无 warning。
        let (blocks, warnings) = crate::knowledge_wiki::block_parser::parse_chunk_blocks(&md);
        assert_eq!(blocks.len(), 1, "expected 1 chunk, got {}: {md}", blocks.len());
        assert!(warnings.items.is_empty(), "unexpected parse warnings: {:?}", warnings.items);
    }

    #[test]
    fn render_html_rejects_empty_body() {
        let html = br#"<!doctype html><html><head><title>x</title></head><body></body></html>"#;
        assert!(render_html_to_markdown(html).is_err());
    }
}

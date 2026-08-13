//! `ingest_worker_smoke` —— P1-6 auto-ingest worker 单轮冒烟。
//!
//! 覆盖形态（如实说明）：`outbound_fetch` 的公网-only SSRF 门（SR-109，
//! fail-closed、无测试逃生门）使 wiremock 等 loopback mock server **无法**
//! 走通 `run_one_round` 的 fetch 段——因此本文件分两层守护：
//!   1. `run_one_round` 层：拒绝/跳过路径（loopback / 云 metadata / not-due /
//!      due-private），断言不发请求、不产 chunk、failure_streak 语义正确；
//!   2. 成功收尾链（[`finalize_claimed_content_for_redline`]，即 fetch 成功后的
//!      claim-owned 落库段）：经真实 claim + finalize 协议断言 ≥1 chunk 全部
//!      `draft` + `needs_review`（红线"AI 永不自动 verify"）、
//!      `last_fetched_at` 推进、`failure_streak` 归零、`ingest_count` 累加、
//!      claim 释放。
//!      fetch → markdown 的解析段由 `ingest_worker.rs` 内嵌单测覆盖
//!      （`render_rss_to_markdown` / `render_html_to_markdown`）。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB（+ wiremock 走本地回环；
//! 成功收尾链需 replica-set 事务），CI 用
//! `cargo test --test ingest_worker_smoke -- --ignored`（需 Docker）。

mod common;

use mongodb::bson::{doc, DateTime as BsonDateTime};
use wechatagent::knowledge_wiki::ingest_worker::{
    claim_due_source_for_redline, finalize_claimed_content_for_redline, run_one_round,
};
use wechatagent::models::IngestSource;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::TestApp;

const RSS_BODY: &str = r#"<?xml version="1.0"?>
<rss version="2.0">
  <channel>
    <title>Smoke Feed</title>
    <item>
      <guid>smoke-item-1</guid>
      <title>第一条公告</title>
      <link>https://example.com/1</link>
      <description>这是第一条 ingest 测试正文。</description>
    </item>
    <item>
      <guid>smoke-item-2</guid>
      <title>第二条公告</title>
      <link>https://example.com/2</link>
      <description>这是第二条 ingest 测试正文。</description>
    </item>
  </channel>
</rss>"#;

fn ingest_source(workspace_id: &str, source_id: &str, kind: &str, url: String) -> IngestSource {
    IngestSource {
        id: None,
        source_id: source_id.to_string(),
        workspace_id: workspace_id.to_string(),
        source_generation: 1,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        kind: kind.to_string(),
        url,
        label: Some(format!("smoke {kind}")),
        schedule_minutes: 60,
        // None → is_due() 恒 true，本轮立即拉取。
        last_fetched_at: None,
        last_etag: None,
        last_content_hash: None,
        last_error: None,
        status: "active".to_string(),
        failure_streak: 0,
        ingest_count: 0,
        created_at: BsonDateTime::now(),
        updated_at: BsonDateTime::now(),
    }
}

async fn insert_source(app: &TestApp, src: &IngestSource) {
    app.state
        .db
        .ingest_sources()
        .insert_one(src, None)
        .await
        .expect("insert ingest source");
}

async fn reload_source(app: &TestApp, source_id: &str) -> IngestSource {
    app.state
        .db
        .ingest_sources()
        .find_one(doc! { "source_id": source_id }, None)
        .await
        .expect("query ingest source")
        .expect("source should exist")
}

/// SR-109: even an otherwise valid loopback response must be rejected before
/// any request is sent or knowledge is persisted.
#[tokio::test]
#[ignore]
async fn run_one_round_rejects_loopback_source_before_request() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"smoke-etag-1\"")
                .set_body_string(RSS_BODY),
        )
        .mount(&server)
        .await;

    let url = format!("{}/feed.xml", server.uri());
    let src = ingest_source(&ws, "ing_smoke_rss", "rss", url);
    insert_source(&app, &src).await;

    run_one_round(&app.state).await.expect("run_one_round ok");

    let reloaded = reload_source(&app, "ing_smoke_rss").await;
    assert_eq!(reloaded.failure_streak, 1);
    assert!(reloaded
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("non-public network address")));
    assert_eq!(reloaded.ingest_count, 0);
    assert!(reloaded.last_fetched_at.is_none());
    assert!(reloaded.last_etag.is_none());
    assert_eq!(reloaded.status, "active");
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(doc! { "workspace_id": &ws }, None)
            .await
            .expect("count chunks"),
        0
    );
}

/// SR-109: cloud metadata and link-local targets are rejected without I/O.
#[tokio::test]
#[ignore]
async fn run_one_round_rejects_metadata_source() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let src = ingest_source(
        &ws,
        "ing_smoke_fail",
        "html",
        "http://169.254.169.254/latest/meta-data".to_string(),
    );
    insert_source(&app, &src).await;

    run_one_round(&app.state).await.expect("run_one_round ok");

    let reloaded = reload_source(&app, "ing_smoke_fail").await;
    assert_eq!(reloaded.failure_streak, 1, "失败一次 failure_streak=1");
    assert!(reloaded.last_error.is_some(), "应记录 last_error");

    let chunk_count = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks");
    assert_eq!(chunk_count, 0, "失败源不应产任何 chunk");
}

/// 场景 3(H1 回归):source 未到 schedule_minutes(not-due)时,run_one_round 必须
/// 跳过、**不刷 last_fetched_at**。旧 bug 下 not-due 与真 304 共用 NotModified→
/// mark_success 无条件把 last_fetched_at 刷成 now→worker interval<schedule 时源
/// 首拉后永不更新。修复后 not-due 返 SourceOutcome::Skipped,不写任何 DB。
#[tokio::test]
#[ignore]
async fn run_one_round_skips_not_due_source_without_touching_last_fetched_at() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // 不挂任何 wiremock:not-due 本就不该发请求;若代码错误地发了请求会因无 mock
    // 连接失败,但那条路径也不会走到(is_due=false 早退在发请求之前)。
    let src = ingest_source(
        &ws,
        "ing_not_due",
        "rss",
        "http://127.0.0.1:1/never".to_string(),
    );
    insert_source(&app, &src).await;

    // 把 last_fetched_at 设成 10 分钟前(schedule_minutes=60 → 未到点 not-due),
    // 用固定毫秒时刻以便精确断言未被改动。
    let ten_min_ago_ms = mongodb::bson::DateTime::now().timestamp_millis() - 10 * 60 * 1000;
    let pinned = mongodb::bson::DateTime::from_millis(ten_min_ago_ms);
    app.state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": "ing_not_due" },
            doc! { "$set": { "last_fetched_at": pinned } },
            None,
        )
        .await
        .expect("pin last_fetched_at to 10min ago");

    run_one_round(&app.state).await.expect("run_one_round ok");

    let reloaded = reload_source(&app, "ing_not_due").await;
    // 核心红线:not-due 源的 last_fetched_at 必须原封不动(旧 bug 下会被刷成 now)。
    assert_eq!(
        reloaded.last_fetched_at.map(|d| d.timestamp_millis()),
        Some(ten_min_ago_ms),
        "not-due 源的 last_fetched_at 不得被 run_one_round 刷新(旧 bug 会刷成 now)"
    );
    assert_eq!(
        reloaded.ingest_count, 0,
        "not-due 源不应产 chunk / 累加 ingest_count"
    );
    assert_eq!(reloaded.status, "active", "not-due 源状态不变");

    // 且没落任何 chunk。
    let chunk_count = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks");
    assert_eq!(chunk_count, 0, "not-due 源不应产任何 chunk");
}

/// DIV-31：ingest 正向成功链——fetch 成功后的收尾落库段。
///
/// SSRF 公网门使 loopback 无法走通 fetch 段（见文件头）；本测试经生产协议
/// `claim_due_source_for_redline` → [`finalize_claimed_content_for_redline`] 驱动
/// fetch 后的成功链，输入与 `render_rss_to_markdown` 产出同形的 fence markdown，
/// 断言：
///   1. 落 ≥1 chunk 且全部 `status="draft"` + `integrity_status="needs_review"`
///      （红线"AI 永不自动 verify"）；
///   2. source 的 `last_fetched_at` 从 None 推进为 Some、`failure_streak` 从 2
///      归零、`ingest_count` 累加 chunk 数、content_hash 记录；
///   3. claim 三字段（worker_id / claim_token / locked_until）被释放。
#[tokio::test]
#[ignore]
async fn claimed_content_finalize_persists_draft_chunks_and_advances_source() {
    // ingest 收尾链在事务内落库 → 需要 replica-set MongoDB。
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    let mut src = ingest_source(
        &ws,
        "ing_success",
        "rss",
        "https://feeds.example.com/feed.xml".to_string(),
    );
    src.failure_streak = 2;
    insert_source(&app, &src).await;

    let claim = claim_due_source_for_redline(&app.state, "ing_success", "test-worker:success")
        .await
        .expect("claim source")
        .expect("due source must be claimed");

    // 与 render_rss_to_markdown 产出同形的 fence markdown（两条 RSS 条目）。
    let markdown = concat!(
        "---CHUNK: rss-0---\n",
        r#"{"title":"第一条公告","summary":"这是第一条 ingest 测试正文。","body":"这是第一条 ingest 测试正文。","businessContext":"source: https://example.com/1"}"#,
        "\n---END CHUNK---\n\n",
        "---CHUNK: rss-1---\n",
        r#"{"title":"第二条公告","summary":"这是第二条 ingest 测试正文。","body":"这是第二条 ingest 测试正文。","businessContext":"source: https://example.com/2"}"#,
        "\n---END CHUNK---\n\n",
    );

    finalize_claimed_content_for_redline(&app.state, &claim, markdown)
        .await
        .expect("finalize claimed content");

    // 1. chunk 全部 draft + needs_review（红线）。
    let chunks: Vec<mongodb::bson::Document> = {
        use futures::TryStreamExt;
        app.state
            .db
            .operation_knowledge_chunks()
            .clone_with_type::<mongodb::bson::Document>()
            .find(doc! { "workspace_id": &ws }, None)
            .await
            .expect("query chunks")
            .try_collect()
            .await
            .expect("collect chunks")
    };
    assert!(!chunks.is_empty(), "success ingest must persist >=1 chunk");
    for chunk in &chunks {
        assert_eq!(chunk.get_str("status").unwrap_or_default(), "draft");
        assert_eq!(
            chunk.get_str("integrity_status").unwrap_or_default(),
            "needs_review"
        );
    }

    // 2. source 前进：last_fetched_at 推进、streak 归零、count 累加、指纹记录。
    let reloaded = reload_source(&app, "ing_success").await;
    assert!(reloaded.last_fetched_at.is_some(), "last_fetched_at 推进");
    assert_eq!(reloaded.failure_streak, 0, "failure_streak 归零");
    assert_eq!(
        reloaded.ingest_count,
        chunks.len() as i64,
        "ingest_count 累加"
    );
    assert!(reloaded.last_content_hash.is_some(), "content_hash 记录");
    assert_eq!(reloaded.status, "active");
    assert!(reloaded.last_error.is_none());

    // 3. claim 释放。
    assert!(reloaded.worker_id.is_none(), "worker_id 释放");
    assert!(reloaded.claim_token.is_none(), "claim_token 释放");
    assert!(reloaded.locked_until.is_none(), "locked_until 释放");
}

/// A due private source must be evaluated and rejected, while the not-due test
/// above proves scheduling still short-circuits before policy resolution.
#[tokio::test]
#[ignore]
async fn run_one_round_rejects_due_private_source() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"due-etag\"")
                .set_body_string(RSS_BODY),
        )
        .mount(&server)
        .await;

    let url = format!("{}/feed.xml", server.uri());
    let src = ingest_source(&ws, "ing_due", "rss", url);
    insert_source(&app, &src).await;

    // 设成 120 分钟前(schedule_minutes=60 → 已过点 due)。
    let two_hours_ago_ms = mongodb::bson::DateTime::now().timestamp_millis() - 120 * 60 * 1000;
    app.state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": "ing_due" },
            doc! { "$set": { "last_fetched_at": mongodb::bson::DateTime::from_millis(two_hours_ago_ms) } },
            None,
        )
        .await
        .expect("pin last_fetched_at to 120min ago");

    run_one_round(&app.state).await.expect("run_one_round ok");

    let reloaded = reload_source(&app, "ing_due").await;
    assert_eq!(
        reloaded.last_fetched_at.map(|d| d.timestamp_millis()),
        Some(two_hours_ago_ms)
    );
    assert_eq!(reloaded.failure_streak, 1);
    assert_eq!(reloaded.ingest_count, 0);
    assert!(server.received_requests().await.unwrap().is_empty());
}

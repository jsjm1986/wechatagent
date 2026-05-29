//! `ingest_worker_smoke` —— P1-6 auto-ingest worker 单轮端到端冒烟。
//!
//! 用 wiremock 顶替外部 RSS / HTML 源，驱动 [`ingest_worker::run_one_round`] 跑一轮，
//! 断言：
//!   1. RSS 源 → feed-rs 解析 → 落 ≥1 chunk，全部 `draft` + `needs_review`
//!      （红线"AI 永不自动 verify"）；
//!   2. 拉取成功后 source 的 `last_fetched_at` 被刷新、`failure_streak` 归零、
//!      `ingest_count` 累加；
//!   3. 不可达源（wiremock 500）→ `failure_streak` +1，不产 chunk。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB（+ wiremock 走本地回环），
//! CI 用 `cargo test --test ingest_worker_smoke -- --ignored`（需 Docker）。

mod common;

use mongodb::bson::{doc, DateTime as BsonDateTime};
use wechatagent::knowledge_wiki::ingest_worker::run_one_round;
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
        kind: kind.to_string(),
        url,
        label: Some(format!("smoke {kind}")),
        schedule_minutes: 60,
        // None → is_due() 恒 true，本轮立即拉取。
        last_fetched_at: None,
        last_etag: None,
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

/// 场景 1：RSS 源拉取成功 → 落 chunk（draft + needs_review）+ source 状态刷新。
#[tokio::test]
#[ignore]
async fn run_one_round_ingests_rss_into_review_chunks() {
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

    // chunk 落库且全部 draft + needs_review。
    let mut cursor = app
        .state
        .db
        .operation_knowledge_chunks()
        .find(doc! { "workspace_id": &ws }, None)
        .await
        .expect("query chunks");
    use futures::TryStreamExt;
    let mut count = 0usize;
    while let Some(chunk) = cursor.try_next().await.expect("iter chunks") {
        count += 1;
        assert_eq!(
            chunk.status, "draft",
            "ingest chunk 必须 draft（AI 永不自动 verify）",
        );
        assert_eq!(
            chunk.integrity_status.as_deref(),
            Some("needs_review"),
            "ingest chunk 必须 needs_review",
        );
    }
    assert!(count >= 1, "RSS 至少应产 1 chunk，实际 {count}");

    // source 状态：last_fetched_at 已刷新、failure_streak 归零、ingest_count 累加、
    // etag 记录回来。
    let reloaded = reload_source(&app, "ing_smoke_rss").await;
    assert!(reloaded.last_fetched_at.is_some(), "应记录 last_fetched_at");
    assert_eq!(reloaded.failure_streak, 0, "成功后 failure_streak 归零");
    assert!(reloaded.ingest_count >= 1, "ingest_count 应累加");
    assert_eq!(reloaded.last_etag.as_deref(), Some("\"smoke-etag-1\""));
    assert_eq!(reloaded.status, "active");
}

/// 场景 2：源不可达（500）→ failure_streak +1，不产 chunk。
#[tokio::test]
#[ignore]
async fn run_one_round_marks_failure_on_unreachable_source() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let url = format!("{}/feed.xml", server.uri());
    let src = ingest_source(&ws, "ing_smoke_fail", "rss", url);
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

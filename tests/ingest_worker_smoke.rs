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

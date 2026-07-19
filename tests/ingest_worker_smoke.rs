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

/// 场景 4(对照):source 已过 schedule_minutes(due)时,run_one_round 仍正常拉取。
/// 确认拆 Skipped 变体没误伤"到点该拉"的正常路径。
#[tokio::test]
#[ignore]
async fn run_one_round_still_ingests_due_source() {
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
    // due 源被真实拉取:last_fetched_at 前移(> 120min 前的旧值)、产 chunk。
    assert!(
        reloaded
            .last_fetched_at
            .map(|d| d.timestamp_millis())
            .unwrap_or(0)
            > two_hours_ago_ms,
        "due 源应被拉取,last_fetched_at 前移"
    );
    assert!(
        reloaded.ingest_count >= 1,
        "due 源应产 chunk 并累加 ingest_count"
    );
}

/// 场景 5：源不提供 ETag 时，连续两次返回相同内容只导入一次。
#[tokio::test]
#[ignore]
async fn run_one_round_dedupes_unchanged_content_without_etag() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed-no-etag.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RSS_BODY))
        .mount(&server)
        .await;

    let url = format!("{}/feed-no-etag.xml", server.uri());
    let src = ingest_source(&ws, "ing_no_etag_dedupe", "rss", url);
    insert_source(&app, &src).await;

    run_one_round(&app.state).await.expect("first round ok");
    let first_source = reload_source(&app, "ing_no_etag_dedupe").await;
    assert!(
        first_source.last_etag.is_none(),
        "fixture must not return ETag"
    );
    assert!(
        first_source.last_content_hash.is_some(),
        "first successful ingest should persist the content checkpoint"
    );

    let first_documents = app
        .state
        .db
        .operation_knowledge_documents()
        .count_documents(
            doc! { "workspace_id": &ws, "source_name": "smoke rss" },
            None,
        )
        .await
        .expect("count first documents");
    let first_chunks = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count first chunks");
    assert_eq!(first_documents, 1, "first round should create one document");
    assert!(first_chunks > 0, "first round should create chunks");

    // Make the source due again without changing the fetched body.
    let two_hours_ago =
        BsonDateTime::from_millis(BsonDateTime::now().timestamp_millis() - 2 * 60 * 60 * 1000);
    app.state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": "ing_no_etag_dedupe" },
            doc! { "$set": { "last_fetched_at": two_hours_ago } },
            None,
        )
        .await
        .expect("make source due again");

    run_one_round(&app.state).await.expect("second round ok");
    let second_source = reload_source(&app, "ing_no_etag_dedupe").await;
    let second_documents = app
        .state
        .db
        .operation_knowledge_documents()
        .count_documents(
            doc! { "workspace_id": &ws, "source_name": "smoke rss" },
            None,
        )
        .await
        .expect("count second documents");
    let second_chunks = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count second chunks");

    assert_eq!(second_documents, first_documents);
    assert_eq!(second_chunks, first_chunks);
    assert_eq!(second_source.ingest_count, first_source.ingest_count);
    assert_eq!(
        second_source.last_content_hash,
        first_source.last_content_hash,
    );
}

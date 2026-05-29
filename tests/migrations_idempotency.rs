//! 验证迁移框架的幂等性：`TestApp::start()` 已在启动链路跑过一轮
//! `migrations::run`，每条迁移在 `migrations` 集合留一行账。再调一次
//! `migrations::run` 必须按 `_id` 跳过全部已应用项——账册条数不变。
//!
//! 默认 `#[ignore]`，需要 Docker；CI 用 `cargo test -- --ignored` 触发。

mod common;

use mongodb::bson::doc;

#[tokio::test]
#[ignore]
async fn run_is_idempotent_across_reruns() {
    let app = common::TestApp::start().await;

    // 启动链路已执行全部迁移，每条留一行账。
    let count = app
        .state
        .db
        .migrations()
        .count_documents(doc! {}, None)
        .await
        .expect("count migrations");
    assert_eq!(
        count as usize,
        wechatagent::db::migrations::MIGRATIONS.len(),
        "启动后每条 migration 应各留一行账"
    );

    wechatagent::db::migrations::run(&app.state.db)
        .await
        .expect("rerun migrations");

    let count_after = app
        .state
        .db
        .migrations()
        .count_documents(doc! {}, None)
        .await
        .expect("count migrations after rerun");
    assert_eq!(
        count_after, count,
        "二次 run 必须按 _id 跳过已应用项，账册条数不变"
    );
}

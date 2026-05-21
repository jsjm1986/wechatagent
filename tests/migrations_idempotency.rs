//! 验证迁移框架的幂等性：连续两次调用 `migrations::run` 不会重复执行。
//!
//! 默认 `#[ignore]`，需要 Docker；CI 用 `cargo test -- --ignored` 触发。

mod common;

use mongodb::bson::doc;

#[tokio::test]
#[ignore]
async fn run_is_idempotent_with_empty_migrations() {
    let app = common::TestApp::start().await;

    let count = app
        .state
        .db
        .migrations()
        .count_documents(doc! {}, None)
        .await
        .expect("count migrations");
    assert_eq!(count, 0, "no migrations defined yet");

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
    assert_eq!(count_after, 0);
}

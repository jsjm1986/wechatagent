//! 测试基础设施冒烟测试。
//!
//! 默认 `#[ignore]`，需要 Docker；CI 用 `cargo test -- --ignored` 触发。

mod common;

#[test]
fn test_account_fixture_round_trips_through_typed_model() {
    let account: wechatagent::models::WechatAccount = mongodb::bson::from_document(
        common::test_account_document("fixture-workspace", "fixture-account"),
    )
    .expect("test account must deserialize through the production model");
    assert_eq!(account.workspace_id, "fixture-workspace");
    assert_eq!(account.account_id, "fixture-account");
    assert_eq!(account.status.as_deref(), Some("active"));
    assert!(account.online);
}

#[tokio::test]
#[ignore]
async fn test_app_starts_with_default_prompt_pack() {
    let app = common::TestApp::start().await;
    assert_eq!(app.state.config.default_workspace_id, "default");
    assert_eq!(app.state.config.default_account_id, "default");
    assert_eq!(app.llm.calls(), 0);
}

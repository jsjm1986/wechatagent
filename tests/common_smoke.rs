//! 测试基础设施冒烟测试。
//!
//! 默认 `#[ignore]`，需要 Docker；CI 用 `cargo test -- --ignored` 触发。

mod common;

#[tokio::test]
#[ignore]
async fn test_app_starts_with_default_prompt_pack() {
    let app = common::TestApp::start().await;
    assert_eq!(app.state.config.default_workspace_id, "default");
    assert_eq!(app.state.config.default_account_id, "default");
    assert_eq!(app.llm.calls(), 0);
}

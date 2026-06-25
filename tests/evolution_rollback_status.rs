//! 回归：rollback_prompt 把 previous_version 行置 current 时，必须一并恢复 status=active。
//! 缺陷场景：若该行此前被归档(status=archived)，回滚后仅翻 current_version 会导致
//! load_prompt(只取 active) 取不到 → 静默回落 default_prompt_content。
//! 默认 #[ignore]，需 Docker。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::models::PromptTemplate;

#[tokio::test]
#[ignore]
async fn rollback_restores_archived_previous_to_active() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let key = "user.rollback.test_key";

    // 旧版本行：被归档（模拟启动对齐归档过它），current_version=false。
    let now = DateTime::now();
    let old = PromptTemplate {
        id: Some(ObjectId::new()),
        workspace_id: workspace.clone(),
        prompt_key: key.to_string(),
        agent_kind: "user".to_string(),
        layer: "soul".to_string(),
        title: "old".to_string(),
        description: None,
        content: "OLD_CONTENT".to_string(),
        status: "archived".to_string(),
        version: 1,
        prompt_pack_version: "test".to_string(),
        created_by: "system".to_string(),
        created_at: now,
        updated_at: now,
        current_version: false,
        previous_version: None,
        seeded_by: Some("system".to_string()),
        locale: None,
    };
    // 新版本行：evolution release 出来的 current。
    let new = PromptTemplate {
        id: Some(ObjectId::new()),
        prompt_key: key.to_string(),
        content: "NEW_CONTENT".to_string(),
        status: "active".to_string(),
        version: 2,
        current_version: true,
        previous_version: Some(1),
        seeded_by: Some("evolution_release".to_string()),
        ..old.clone()
    };
    app.state.db.prompt_templates().insert_many(vec![&old, &new], None).await.unwrap();

    // 构造一个 released 的 prompt proposal，previous_prompt_version="1"。
    let proposal_id = common::insert_released_prompt_proposal(&app.state, &workspace, key, "1").await;

    // 执行回滚。
    wechatagent::evolution::release::rollback_prompt(&app.state, proposal_id, "tester")
        .await
        .expect("rollback ok");

    // version=1 行应被恢复为 current_version=true AND status=active。
    let restored = app
        .state
        .db
        .prompt_templates()
        .find_one(doc! { "workspace_id": &workspace, "prompt_key": key, "version": 1 }, None)
        .await
        .unwrap()
        .expect("v1 row");
    assert_eq!(restored.current_version, true, "v1 应被置回 current");
    assert_eq!(restored.status, "active", "v1 的 status 必须恢复为 active（治本点）");
}

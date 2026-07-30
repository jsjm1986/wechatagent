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
    let app = common::TestApp::start_repl_set().await;
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
        source_proposal_id: None,
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
    app.state
        .db
        .prompt_templates()
        .insert_many(vec![&old, &new], None)
        .await
        .unwrap();

    // 构造一个 released 的 prompt proposal，previous_prompt_version="1"。
    let proposal_id =
        common::insert_released_prompt_proposal(&app.state, &workspace, key, "1").await;

    // 执行回滚。
    wechatagent::evolution::release::rollback_prompt(
        &app.state,
        proposal_id,
        &workspace,
        &app.state.config.default_account_id,
        "tester",
    )
    .await
    .expect("rollback ok");

    // version=1 行应被恢复为 current_version=true AND status=active。
    let restored = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": key, "version": 1 },
            None,
        )
        .await
        .unwrap()
        .expect("v1 row");
    assert_eq!(restored.current_version, true, "v1 应被置回 current");
    assert_eq!(
        restored.status, "active",
        "v1 的 status 必须恢复为 active（治本点）"
    );
}

/// 回归（Stage4 孤儿#3-B）：rollback 的 previous_version 历史行不存在时（如被手动
/// publish 物删）必须中止事务返 Err，而非静默把当前 current 翻掉后留下「无 current
/// 可用」的假成功。
#[tokio::test]
#[ignore]
async fn rollback_aborts_when_previous_version_missing() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let key = "user.rollback.missing_prev";

    // 只有 current（version 2），没有 version 1 那条历史行（模拟被物删）。
    let now = DateTime::now();
    let current = PromptTemplate {
        id: Some(ObjectId::new()),
        workspace_id: workspace.clone(),
        prompt_key: key.to_string(),
        agent_kind: "user".to_string(),
        layer: "soul".to_string(),
        title: "cur".to_string(),
        description: None,
        content: "CUR_CONTENT".to_string(),
        status: "active".to_string(),
        version: 2,
        prompt_pack_version: "test".to_string(),
        created_by: "system".to_string(),
        created_at: now,
        updated_at: now,
        current_version: true,
        previous_version: Some(1),
        seeded_by: Some("evolution_release".to_string()),
        locale: None,
        source_proposal_id: None,
    };
    app.state
        .db
        .prompt_templates()
        .insert_one(&current, None)
        .await
        .unwrap();

    // released proposal，previous_prompt_version="1"（但 version 1 行不存在）。
    let proposal_id =
        common::insert_released_prompt_proposal(&app.state, &workspace, key, "1").await;

    // 执行回滚：应返 Err（找不到 version 1 行）。
    let result = wechatagent::evolution::release::rollback_prompt(
        &app.state,
        proposal_id,
        &workspace,
        &app.state.config.default_account_id,
        "tester",
    )
    .await;
    assert!(
        result.is_err(),
        "previous_version 行缺失时 rollback 必须返 Err，而非假成功"
    );

    // proposal 必须仍是 released（事务中止，未推到 rolled_back）。
    let proposal = app
        .state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .unwrap()
        .expect("proposal exists");
    assert_eq!(
        proposal.status, "released",
        "rollback 中止后 proposal 应仍为 released，不能被推到 rolled_back"
    );

    // 当前 current（version 2）不能被翻成 false（事务整体回滚）。
    let cur = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": key, "version": 2 },
            None,
        )
        .await
        .unwrap()
        .expect("v2 row");
    assert_eq!(
        cur.current_version, true,
        "事务中止后 version 2 应仍是 current（step 1 的置 false 被回滚）"
    );
}

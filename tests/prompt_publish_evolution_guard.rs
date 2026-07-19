//! 回归（Stage4 孤儿#3-A）：手动 publish_prompt_template 的 delete_many 必须放过
//! seeded_by="evolution_release" 的历史行——否则会物删 evolution 灰度链留下的历史
//! 版本，摧毁 rollback（rollback 靠 version=previous_version 找回历史行）。
//! 默认 #[ignore]，需 Docker。

mod common;

use axum::extract::{Extension, Json, Path, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::PromptTemplate;

fn test_admin(ws: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "test_admin".to_string(),
        username: "test_admin".to_string(),
        current_workspace: ws.to_string(),
    }
}

fn make_row(ws: &str, key: &str, version: i32, status: &str, seeded_by: &str) -> PromptTemplate {
    let now = DateTime::now();
    PromptTemplate {
        id: Some(ObjectId::new()),
        workspace_id: ws.to_string(),
        prompt_key: key.to_string(),
        agent_kind: "user".to_string(),
        layer: "soul".to_string(),
        title: format!("v{version}"),
        description: None,
        // FreelyEditable key（非 forbidden/非 4 个 constrained key）只过禁词闸，
        // 干净中文内容通过 validate_prompt_edit。
        content: format!("这是第 {version} 版的运营话术内容，用于测试发布守卫。"),
        status: status.to_string(),
        version,
        prompt_pack_version: "test".to_string(),
        created_by: "manual".to_string(),
        created_at: now,
        updated_at: now,
        current_version: status == "active",
        previous_version: None,
        seeded_by: Some(seeded_by.to_string()),
        locale: None,
    }
}

#[tokio::test]
#[ignore]
async fn publish_preserves_evolution_history_rows() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let key = "user.test.publish_guard";

    // v1 system（应被删）、v2 evolution_release 历史行（必须存活）、v3 manual draft（被 publish）。
    let sys = make_row(&ws, key, 1, "archived", "system");
    let evo = make_row(&ws, key, 2, "active", "evolution_release");
    let draft = make_row(&ws, key, 3, "draft", "manual");
    let draft_id = draft.id.unwrap().to_hex();
    app.state
        .db
        .prompt_templates()
        .insert_many(vec![&sys, &evo, &draft], None)
        .await
        .unwrap();

    // 直调真 handler，force=true 跳 LLM 闸（字面双闸仍跑，FreelyEditable 只查禁词）。
    let resp = wechatagent::routes::prompt_templates::publish_prompt_template(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(draft_id.clone()),
        Some(Json(
            serde_json::from_value(json!({ "force": true })).unwrap(),
        )),
    )
    .await
    .expect("publish ok");
    assert_eq!(resp.0["ok"], true);

    // 断言 1：evolution_release 历史行（v2）必须存活（修复前会被 delete_many 物删）。
    let evo_after = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &ws, "prompt_key": key, "version": 2 },
            None,
        )
        .await
        .unwrap();
    assert!(
        evo_after.is_some(),
        "seeded_by=evolution_release 的历史行必须在手动 publish 后存活（保住 rollback 链）"
    );

    // 断言 2：非 evolution 的 system 历史行（v1）被删（单版本清理语义保留）。
    let sys_after = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &ws, "prompt_key": key, "version": 1 },
            None,
        )
        .await
        .unwrap();
    assert!(sys_after.is_none(), "非 evolution 历史行应被 publish 清理");

    // 断言 3：被 publish 的 draft（v3）转为 active。
    let draft_after = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &ws, "prompt_key": key, "version": 3 },
            None,
        )
        .await
        .unwrap()
        .expect("v3 row");
    assert_eq!(
        draft_after.status, "active",
        "被 publish 的 draft 应转 active"
    );

    // 断言 4：检测到 evolution 行时写了观测事件（边缘副作用可见）。
    let ev = app
        .state
        .db
        .events()
        .find_one(
            doc! { "workspace_id": &ws, "kind": "prompt_publish_kept_evolution_rows" },
            None,
        )
        .await
        .unwrap();
    assert!(
        ev.is_some(),
        "检测到 evolution 行时应写 prompt_publish_kept_evolution_rows 观测事件"
    );
}

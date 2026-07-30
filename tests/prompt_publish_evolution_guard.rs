//! SR-055: manual Prompt publication is an append-only, single-current switch.
//! Historical system/evolution rows are retained and runtime never performs an
//! implicit contact bucket over multiple `status=active` rows.

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

fn make_row(
    ws: &str,
    key: &str,
    version: i32,
    status: &str,
    current: bool,
    seeded_by: &str,
) -> PromptTemplate {
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
        current_version: current,
        previous_version: None,
        seeded_by: Some(seeded_by.to_string()),
        locale: None,
        source_proposal_id: None,
    }
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn publish_preserves_all_history_and_switches_one_current() {
    let app = common::TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();
    let key = "user.test.publish_guard";

    let sys = make_row(&ws, key, 1, "archived", false, "system");
    let evo = make_row(&ws, key, 2, "active", true, "evolution_release");
    let draft = make_row(&ws, key, 3, "draft", false, "manual");
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

    let rows = app
        .state
        .db
        .prompt_templates()
        .find(
            doc! { "workspace_id": &ws, "prompt_key": key },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "version": 1 })
                .build(),
        )
        .await
        .unwrap();
    let rows: Vec<PromptTemplate> = futures::TryStreamExt::try_collect(rows).await.unwrap();
    assert_eq!(
        rows.len(),
        3,
        "publication must retain every historical row"
    );
    assert_eq!(
        rows.iter().filter(|row| row.current_version).count(),
        1,
        "exactly one canonical current pointer must remain"
    );
    let old_system = rows.iter().find(|row| row.version == 1).unwrap();
    assert_eq!(old_system.status, "archived");
    assert!(!old_system.current_version);
    let old_evolution = rows.iter().find(|row| row.version == 2).unwrap();
    assert_eq!(old_evolution.status, "archived");
    assert!(!old_evolution.current_version);
    let published = rows.iter().find(|row| row.version == 3).unwrap();
    assert_eq!(published.status, "active");
    assert!(published.current_version);

    let first = wechatagent::prompts::load_prompt_for_contact(
        &app.state.db,
        &ws,
        key,
        "contact-a",
        Some("zh-CN"),
    )
    .await
    .unwrap();
    let second = wechatagent::prompts::load_prompt_for_contact(
        &app.state.db,
        &ws,
        key,
        "contact-b",
        Some("en-US"),
    )
    .await
    .unwrap();
    assert_eq!(
        first, second,
        "contact identity must not select another Prompt"
    );
    assert_eq!(first.1, Some(3));

    app.cleanup().await;
}

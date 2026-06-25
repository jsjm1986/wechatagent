//! 波 D2：prompt pack seeding 安全性回归。
//!
//! 性质：
//! - `ensure_prompt_pack_v2` 在已经种过 v2 的 workspace 上**不会**删除运营人员
//!   手工创建的 active / draft 模板（哪怕 prompt_key 不在 spec 中）。
//! - 同一 workspace 上若 spec 里新增了 key（比如波 D 之前缺失的
//!   `user.review.product_claim_markers` / `knowledge.auto_verify`），
//!   `align_prompt_specs` 会在该 key 缺失时（`None => true` 分支）把它们补齐，
//!   而不会因为"版本号已匹配"整体跳过。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::models::PromptTemplate;
use wechatagent::prompts;

fn make_user_template(workspace: &str, key: &str, status: &str) -> PromptTemplate {
    let now = DateTime::now();
    PromptTemplate {
        id: Some(ObjectId::new()),
        workspace_id: workspace.to_string(),
        prompt_key: key.to_string(),
        agent_kind: "user".to_string(),
        layer: "custom".to_string(),
        title: "运营手写".to_string(),
        description: Some("不应被 reseed 删除".to_string()),
        content: "custom content".to_string(),
        status: status.to_string(),
        version: 1,
        prompt_pack_version: prompts::PROMPT_PACK_VERSION.to_string(),
        created_by: "operator".to_string(),
        created_at: now,
        updated_at: now,
        current_version: false,
        previous_version: None,
        seeded_by: Some("manual".to_string()),
        locale: Some(prompts::DEFAULT_LOCALE.to_string()),
    }
}

#[tokio::test]
#[ignore]
async fn ensure_prompt_pack_does_not_delete_user_active_templates() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    // 运营自定义模板（active 与 draft 各一条），key 不在 spec 中。
    let active = make_user_template(&workspace, "user.custom.active_only", "active");
    let draft = make_user_template(&workspace, "user.custom.draft_only", "draft");
    app.state
        .db
        .prompt_templates()
        .insert_many(vec![&active, &draft], None)
        .await
        .unwrap();

    // 重新跑 ensure_prompt_pack_v2（TestApp::start 已经跑过一次）。
    prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("rerun ensure_prompt_pack_v2");

    // active / draft 两条自定义模板都还在。
    let active_after = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": &workspace,
                "prompt_key": "user.custom.active_only"
            },
            None,
        )
        .await
        .unwrap();
    assert!(
        active_after.is_some(),
        "运营自定义 active 模板必须保留，不应被 reseed 删掉"
    );
    let draft_after = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": &workspace,
                "prompt_key": "user.custom.draft_only"
            },
            None,
        )
        .await
        .unwrap();
    assert!(
        draft_after.is_some(),
        "运营自定义 draft 模板必须保留，不应被 reseed 删掉"
    );
}

#[tokio::test]
#[ignore]
async fn ensure_prompt_pack_seeds_all_spec_keys() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();

    // 跑了一遍 prompt pack v2 后，spec 里的两个新 key 都应已落地。
    for key in [
        "user.review.product_claim_markers",
        "knowledge.auto_verify",
    ] {
        let template = app
            .state
            .db
            .prompt_templates()
            .find_one(
                doc! {
                    "workspace_id": &workspace,
                    "prompt_key": key,
                    "status": { "$in": ["active", "draft"] }
                },
                None,
            )
            .await
            .unwrap();
        assert!(
            template.is_some(),
            "ensure_prompt_pack_v2 必须 seed key={key}"
        );
    }
}

/// 系统种子脉络：spec 内容「漂移」后，重跑 ensure 应归档旧 system 行 + 种新行（active）。
/// 用一个真实 spec key（user.reply.soul 等价的系统 key）模拟：先把它的 current system 行
/// content 改脏，重跑 ensure，应被对齐回 spec 内容。
#[tokio::test]
#[ignore]
async fn align_refreshes_drifted_system_row_and_archives_old() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    // 取一个真实存在的系统 prompt key（TestApp::start 已种入）。
    let specs = wechatagent::prompts::prompt_specs_for_test();
    let key = specs.first().expect("at least one spec").0.clone();

    // 制造 Ok(None)（模拟版本 bump 前的旧库）：把所有行的 pack version 改旧，
    // 否则 lookup 命中当前版本走 Ok(Some) 不触发 align。
    app.state
        .db
        .prompt_templates()
        .update_many(
            doc! { "workspace_id": &workspace },
            doc! { "$set": { "prompt_pack_version": "pre_align_old_pack" } },
            None,
        )
        .await
        .unwrap();

    // 把该 key 的 current system 行 content 改脏（模拟旧版本库内容与新 spec 不一致）。
    app.state
        .db
        .prompt_templates()
        .update_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "current_version": true },
            doc! { "$set": { "content": "STALE_DRIFTED_CONTENT" } },
            None,
        )
        .await
        .unwrap();

    // 重跑 ensure_prompt_pack_v2。
    wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("rerun ensure");

    // current active 行的 content 应不再是脏值（被 spec 覆盖）。
    let current = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "current_version": true, "status": "active" },
            None,
        )
        .await
        .unwrap()
        .expect("current row exists");
    assert_ne!(current.content, "STALE_DRIFTED_CONTENT", "脏内容应被 spec 对齐覆盖");

    // 旧脏行应被归档（archived），而非物理删除。
    let archived = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "content": "STALE_DRIFTED_CONTENT" },
            None,
        )
        .await
        .unwrap();
    assert!(archived.is_some(), "旧行应被归档保留，可回溯");
    assert_eq!(archived.unwrap().status, "archived");
}

/// evolution 边界：某 key 存在 seeded_by=evolution_release 的行时，对齐跳过该 key，
/// evolution 行原样保留、未被归档。
#[tokio::test]
#[ignore]
async fn align_skips_keys_with_evolution_release_chain() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    let specs = wechatagent::prompts::prompt_specs_for_test();
    let key = specs.first().expect("spec").0.clone();

    // 模拟 evolution release：插入一条 seeded_by=evolution_release 的 current active 行。
    let mut evo = make_user_template(&workspace, &key, "active");
    evo.seeded_by = Some("evolution_release".to_string());
    evo.current_version = true;
    evo.content = "EVOLUTION_TUNED_CONTENT".to_string();
    app.state.db.prompt_templates().insert_one(&evo, None).await.unwrap();

    // 制造 Ok(None)（模拟版本 bump 前的旧库）：把所有行的 pack version 改旧。
    // evo 行也被改旧没关系——align 的 evolution 守卫按 seeded_by 识别，不看 version。
    app.state
        .db
        .prompt_templates()
        .update_many(
            doc! { "workspace_id": &workspace },
            doc! { "$set": { "prompt_pack_version": "pre_align_old_pack" } },
            None,
        )
        .await
        .unwrap();

    wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("rerun ensure");

    // evolution 行原样保留（content 未变、未被归档）。
    let evo_after = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "seeded_by": "evolution_release" },
            None,
        )
        .await
        .unwrap()
        .expect("evolution row preserved");
    assert_eq!(evo_after.content, "EVOLUTION_TUNED_CONTENT");
    assert_eq!(evo_after.status, "active", "evolution 行不被归档");
}

/// 幂等：spec 没变时重跑不产生新行（不翻版本）。
#[tokio::test]
#[ignore]
async fn align_is_idempotent_when_spec_unchanged() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    // 制造 Ok(None)（模拟版本 bump 前的旧库）：把所有行的 pack version 改旧，
    // 不改任何 content。这样重跑走 Ok(None)→align→每 key normalize 后内容与
    // spec 一致→needs_align=false→不归档不新增→count 不变。
    app.state
        .db
        .prompt_templates()
        .update_many(
            doc! { "workspace_id": &workspace },
            doc! { "$set": { "prompt_pack_version": "pre_align_old_pack" } },
            None,
        )
        .await
        .unwrap();

    let count_before = app
        .state
        .db
        .prompt_templates()
        .count_documents(doc! { "workspace_id": &workspace }, None)
        .await
        .unwrap();

    wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("rerun ensure");

    let count_after = app
        .state
        .db
        .prompt_templates()
        .count_documents(doc! { "workspace_id": &workspace }, None)
        .await
        .unwrap();
    assert_eq!(count_before, count_after, "spec 未变重跑不应新增行");
}

/// 终审 #1 核心回归：版本号匹配（不改 prompt_pack_version）但 system 行内容漂移时，
/// 重跑 ensure_prompt_pack_v2 仍应对齐回 spec。
/// 与 align_refreshes_drifted_system_row_and_archives_old 的区别：那个测试改旧版本号制造
/// Ok(None)；本测试保持当前版本号（旧结构会走 Ok(Some) 不对齐），验证不再版本盲。
#[tokio::test]
#[ignore]
async fn align_refreshes_drift_even_when_pack_version_matches() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    let specs = wechatagent::prompts::prompt_specs_for_test();
    let key = specs.first().expect("at least one spec").0.clone();

    // 关键：不改 prompt_pack_version（保持 TestApp 种入的当前 PROMPT_PACK_VERSION）。
    // 只把该 key 的 current system 行 content 改脏。
    app.state
        .db
        .prompt_templates()
        .update_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "current_version": true },
            doc! { "$set": { "content": "DRIFT_WHILE_VERSION_MATCHES" } },
            None,
        )
        .await
        .unwrap();

    // 重跑 ensure_prompt_pack_v2——新结构走非空库路径必对齐；旧版本盲结构会走 Ok(Some) 不对齐。
    wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("rerun ensure");

    // current active 行 content 不再是脏值（被 spec 覆盖）。
    let current = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "current_version": true, "status": "active" },
            None,
        )
        .await
        .unwrap()
        .expect("current row exists");
    assert_ne!(
        current.content, "DRIFT_WHILE_VERSION_MATCHES",
        "版本号匹配时内容漂移也必须被对齐（不再版本盲）"
    );

    // 脏行被归档而非物删。
    let archived = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "content": "DRIFT_WHILE_VERSION_MATCHES" },
            None,
        )
        .await
        .unwrap();
    assert!(archived.is_some(), "脏行应被归档保留可回溯");
    assert_eq!(archived.unwrap().status, "archived");
}

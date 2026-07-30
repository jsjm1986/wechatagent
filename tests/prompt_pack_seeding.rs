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
        source_proposal_id: None,
    }
}

#[tokio::test]
#[ignore]
async fn ensure_prompt_pack_does_not_delete_user_active_templates() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    // 运营自定义模板（active 与 draft 各一条），key 不在 spec 中。
    let mut active = make_user_template(&workspace, "user.custom.active_only", "active");
    active.current_version = true;
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
    for key in ["user.review.product_claim_markers", "knowledge.auto_verify"] {
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
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn align_refreshes_drifted_system_row_and_archives_old() {
    let app = common::TestApp::start_repl_set().await;
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
    assert_ne!(
        current.content, "STALE_DRIFTED_CONTENT",
        "脏内容应被 spec 对齐覆盖"
    );

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

/// evolution 边界：某 key 的唯一 current 来自 evolution_release 时，对齐跳过该 key，
/// evolution 行原样保留、未被归档。
#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn align_skips_keys_with_evolution_release_chain() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    let specs = wechatagent::prompts::prompt_specs_for_test();
    let key = specs.first().expect("spec").0.clone();

    // 模拟 evolution release：先追加 draft，再走共享事务发布，避免制造双 current。
    let mut evo = make_user_template(&workspace, &key, "draft");
    evo.seeded_by = Some("evolution_release".to_string());
    evo.current_version = false;
    // make_user_template 默认 version=1，与该 key 已 seed 的 system 行 version=1 在唯一索引
    // (workspace_id, prompt_key, version) 上撞 E11000；evolution release 出来的行本就是更高
    // 版本，这里设 2 避让。align 的 evolution 守卫按 seeded_by 识别、不看 version。
    evo.version = 2;
    evo.content = "EVOLUTION_TUNED_CONTENT".to_string();
    app.state
        .db
        .prompt_templates()
        .insert_one(&evo, None)
        .await
        .unwrap();
    wechatagent::prompt_template_versions::publish_version(
        &app.state.db,
        &workspace,
        evo.id.expect("evolution draft id"),
        "evolution-test",
    )
    .await
    .expect("publish evolution current");

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

/// m043 may leave an active built-in spec with draft-only history after clearing an invalid
/// pointer. Startup alignment must preserve that operator draft and append/publish a built-in
/// runtime version; the strict runtime reader remains fail-closed before recovery.
#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn align_recovers_builtin_draft_only_stream_without_publishing_the_draft() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();
    let key = "user.reply.system";

    let original = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": &workspace,
                "prompt_key": key,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("query original current")
        .expect("original current exists");
    let original_id = original.id.expect("original id");
    app.state
        .db
        .prompt_templates()
        .update_one(
            doc! { "_id": original_id },
            doc! { "$set": {
                "status": "draft",
                "current_version": false,
                "seeded_by": "manual",
                "content": "operator draft must remain unpublished",
                "updated_at": DateTime::now(),
            } },
            None,
        )
        .await
        .expect("model post-m043 draft-only stream");

    assert!(wechatagent::prompt_template_versions::load_unique_current(
        &app.state.db,
        &workspace,
        key,
    )
    .await
    .expect_err("runtime reader must reject draft-only stream")
    .to_string()
    .contains("current_prompt_missing"));

    assert!(
        wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account,)
            .await
            .expect("startup alignment recovers draft-only stream")
    );

    let preserved = app
        .state
        .db
        .prompt_templates()
        .find_one(doc! { "_id": original_id }, None)
        .await
        .expect("query preserved draft")
        .expect("draft remains");
    assert_eq!(preserved.status, "draft");
    assert!(!preserved.current_version);
    assert_eq!(preserved.content, "operator draft must remain unpublished");

    let current =
        wechatagent::prompt_template_versions::load_unique_current(&app.state.db, &workspace, key)
            .await
            .expect("strict runtime read after recovery")
            .expect("recovered current exists");
    assert_eq!(current.status, "active");
    assert!(current.current_version);
    assert_eq!(current.seeded_by.as_deref(), Some("system"));
    assert_ne!(current.id, Some(original_id));

    let count_after_first = app
        .state
        .db
        .prompt_templates()
        .count_documents(doc! { "workspace_id": &workspace, "prompt_key": key }, None)
        .await
        .expect("count versions after recovery");
    assert!(
        !wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account,)
            .await
            .expect("idempotent startup alignment")
    );
    let count_after_second = app
        .state
        .db
        .prompt_templates()
        .count_documents(doc! { "workspace_id": &workspace, "prompt_key": key }, None)
        .await
        .expect("count versions after idempotent rerun");
    assert_eq!(count_after_first, count_after_second);

    app.cleanup().await;
}

/// Planning-only specs are intentionally draft. Startup and reruns must never turn them into
/// runtime current/active rows, including after m043 clears the old bootstrap pointer.
#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn planning_prompt_specs_remain_non_current_drafts() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    let mut before = Vec::new();
    for key in ["group.policy", "moment.policy"] {
        let rows = app
            .state
            .db
            .prompt_templates()
            .count_documents(doc! { "workspace_id": &workspace, "prompt_key": key }, None)
            .await
            .expect("count planning prompt versions");
        assert!(rows >= 1, "planning prompt must have a visible draft");
        assert_eq!(
            app.state
                .db
                .prompt_templates()
                .count_documents(
                    doc! {
                        "workspace_id": &workspace,
                        "prompt_key": key,
                        "$or": [
                            { "current_version": true },
                            { "status": "active" },
                        ],
                    },
                    None,
                )
                .await
                .expect("count invalid planning prompt pointers"),
            0,
            "planning prompt {key} must remain unpublished"
        );
        before.push((key, rows));
    }

    assert!(
        !wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
            .await
            .expect("idempotent planning prompt alignment")
    );
    for (key, rows) in before {
        assert_eq!(
            app.state
                .db
                .prompt_templates()
                .count_documents(doc! { "workspace_id": &workspace, "prompt_key": key }, None)
                .await
                .expect("count planning prompt versions after rerun"),
            rows,
            "planning prompt rerun must not allocate a new version"
        );
    }

    app.cleanup().await;
}

/// 终审 #1 核心回归：版本号匹配（不改 prompt_pack_version）但 system 行内容漂移时，
/// 重跑 ensure_prompt_pack_v2 仍应对齐回 spec。
/// 与 align_refreshes_drifted_system_row_and_archives_old 的区别：那个测试改旧版本号制造
/// Ok(None)；本测试保持当前版本号（旧结构会走 Ok(Some) 不对齐），验证不再版本盲。
#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn align_refreshes_drift_even_when_pack_version_matches() {
    let app = common::TestApp::start_repl_set().await;
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

/// SR-055：archived Prompt 是不可变历史，启动对齐不得把它当冗余数据清除。
/// 预置一条孤立 archived 行 → 重跑 ensure → 该行仍保留。
#[tokio::test]
#[ignore]
async fn archived_prompt_history_survives_nonempty_startup() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    // 预置一条孤立的 archived 行（key 不在 spec 中，不参与对齐）。
    let mut archived_row =
        make_user_template(&workspace, "user.custom.archived_orphan", "archived");
    archived_row.current_version = false;
    app.state
        .db
        .prompt_templates()
        .insert_one(&archived_row, None)
        .await
        .unwrap();

    // 确认预置成功。
    let before = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": "user.custom.archived_orphan" },
            None,
        )
        .await
        .unwrap();
    assert!(before.is_some(), "预置 archived 行应存在");

    // 重跑 ensure_prompt_pack_v2（不改版本号→非空库路径→delete_redundant 跑）。
    wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("rerun ensure");

    // archived Prompt 历史必须保留。
    let after = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": "user.custom.archived_orphan" },
            None,
        )
        .await
        .unwrap();
    assert!(after.is_some(), "archived Prompt 历史不得被启动对齐清除");
}

/// 终审 Minor #1 回归：ensure_prompt_pack_v2 返回"是否写入",供运行时调用点据此失效 LRU。
/// spec 漂移→返回 true；spec 一致(幂等)→返回 false。
#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn ensure_returns_true_on_write_false_on_idempotent() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    // 第一次重跑(spec 与 DB 一致)应幂等 → 返回 false(无写入)。
    let wrote_idempotent =
        wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
            .await
            .expect("rerun ensure");
    assert!(!wrote_idempotent, "spec 一致时应幂等无写入→false");

    // 制造漂移:把一个 system 行 content 改脏(不改版本号)。
    let specs = wechatagent::prompts::prompt_specs_for_test();
    let key = specs.first().expect("spec").0.clone();
    app.state
        .db
        .prompt_templates()
        .update_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "current_version": true },
            doc! { "$set": { "content": "DRIFT_FOR_BOOL_RETURN" } },
            None,
        )
        .await
        .unwrap();

    // 再跑应检测到漂移→对齐写入→返回 true。
    let wrote_after_drift =
        wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
            .await
            .expect("rerun ensure");
    assert!(
        wrote_after_drift,
        "spec 漂移时对齐写入→true(供调用点失效LRU)"
    );
}

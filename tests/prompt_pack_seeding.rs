//! 波 D2：prompt pack seeding 安全性回归。
//!
//! 性质：
//! - `ensure_prompt_pack_v2` 在已经种过 v2 的 workspace 上**不会**删除运营人员
//!   手工创建的 active / draft 模板（哪怕 prompt_key 不在 spec 中）。
//! - 同一 workspace 上若 spec 里新增了 key（比如波 D 之前缺失的
//!   `user.review.product_claim_markers` / `knowledge.auto_verify`），
//!   `ensure_missing_prompt_templates` 会按 key upsert 把它们补齐，而不会因为
//!   "版本号已匹配"整体跳过。
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

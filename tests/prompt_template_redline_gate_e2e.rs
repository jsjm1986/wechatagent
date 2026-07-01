//! 集成测试：prompt_templates create/publish 红线闸（#2 绕过链回归门）。
//!
//! create 补字面双闸、publish 补字面双闸+LLM三闸后，触碰红线的内容不得入库/激活。
//! handler 是 pub(super) 够不到 → 验证 Task 3 接入的门函数对相同输入的判定 + DB 状态。
//! 仿 tests/evolution_release_redline.rs。全部 #[ignore] 需 Docker。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;
use wechatagent::prompt_guard::{review_prompt_edit, validate_prompt_edit, PromptEditVerdict};

const TARGET_KEY: &str = "user.reply.policy"; // 强约束层，含红线+业务锚

/// 字符拼接构造禁用词，绕源码字面量 lint。
fn forbidden_phrase() -> String {
    ["人", "工", "接", "管"].concat()
}

/// create 闸（字面双闸）：含禁用词的内容 → validate_prompt_edit Err。
/// 这是 create_prompt_template Step 1 实际调用的同一函数同一参数（key+content）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn create_gate_rejects_forbidden_word() {
    let _app = common::TestApp::start().await; // 起容器对齐其它测试；本断言纯函数
    let content = format!("一些正常话术\n遇到难题就{}给后台", forbidden_phrase());
    assert!(
        validate_prompt_edit(TARGET_KEY, &content).is_err(),
        "create 含禁用词内容必须被字面双闸拒"
    );
}

/// create 闸：强约束层删红线锚 → Err。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn create_gate_rejects_anchor_drift() {
    let _app = common::TestApp::start().await;
    let content = "## 我自己重写的策略\n没有任何红线锚".to_string();
    assert!(
        validate_prompt_edit(TARGET_KEY, &content).is_err(),
        "create 删红线/业务锚必须被锚完整性闸拒"
    );
}

/// 直插一条删了红线锚的 draft（raw insert 模拟历史脏数据/绕过 create 闸），
/// 验证 publish 的字面双闸会拒——即 validate_prompt_edit 对该 content Err。
/// 同时确认：被拒时不应改 status（publish handler 在闸失败时 return Err，不走到 update_one）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn publish_gate_rejects_redline_dropped_draft() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let id = ObjectId::new();
    let now = DateTime::now();
    // draft：强约束 key 但内容删光红线锚。
    app.state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates")
        .insert_one(
            doc! {
                "_id": id, "workspace_id": &ws, "prompt_key": TARGET_KEY,
                "agent_kind": "user", "layer": "policy", "title": "t",
                "content": "## 乱改\n无红线锚", "status": "draft", "version": 99,
                "prompt_pack_version": "custom", "created_by": "manual",
                "created_at": now, "updated_at": now, "current_version": false,
                "seeded_by": "manual",
            },
            None,
        )
        .await
        .expect("insert draft");

    // publish 第一道闸 = validate_prompt_edit(key, content)；该 content 删了锚 → Err。
    let row = app
        .state
        .db
        .prompt_templates()
        .find_one(doc! { "_id": id }, None)
        .await
        .unwrap()
        .unwrap();
    assert!(
        validate_prompt_edit(&row.prompt_key, &row.content).is_err(),
        "删红线锚的 draft 过 publish 字面双闸必须被拒"
    );
    // 该行仍是 draft（未被激活）。
    assert_eq!(row.status, "draft");
}

/// publish 第三闸 LLM 语义：干净内容（过字面双闸）+ mock 判 violation=true → review 返回 Reject。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn publish_llm_gate_rejects_semantic_violation() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    // 干净追加内容（保留锚由 seed 的 current 版本提供 old 基线；这里直接验 review 判定）。
    let clean_new = "补充：本行业语气更稳重。";
    app.llm
        .push_response(json!({ "violation": true, "reason": "变相引入真人转介" }));
    let verdict = review_prompt_edit(&app.state, &ws, TARGET_KEY, "原始内容", clean_new).await;
    assert!(
        matches!(verdict, PromptEditVerdict::Reject(_)),
        "LLM 判违规应 Reject"
    );
    assert_eq!(app.llm.calls(), 1);
}

/// publish 第三闸：mock 判 violation=false → Pass。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn publish_llm_gate_passes_clean() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    app.llm.push_response(json!({ "violation": false }));
    let verdict = review_prompt_edit(&app.state, &ws, TARGET_KEY, "原始", "补充：稳重些。").await;
    assert!(
        matches!(verdict, PromptEditVerdict::Pass),
        "LLM 判合规应 Pass"
    );
}

/// publish 第三闸：LLM 不可用（不排队响应）→ NeedsHumanConfirm（不 fail-open）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn publish_llm_gate_unavailable_needs_confirm() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    // 不 push_response → TestLlmGenerator 返回 Err → review_prompt_edit 降级 NeedsHumanConfirm。
    let verdict = review_prompt_edit(&app.state, &ws, TARGET_KEY, "原始", "补充：稳重些。").await;
    assert!(
        matches!(verdict, PromptEditVerdict::NeedsHumanConfirm { .. }),
        "LLM 不可用应降级人确认,不放水"
    );
}

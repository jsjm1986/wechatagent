//! 集成测试：release_prompt 的三道红线闸（禁词 + 锚点 + LLM 语义）必须真正拦截
//! 触碰红线的候选——返回 `EvolutionError::RedlineGateRejected`，且不写 prompt_templates、
//! proposal 状态不推进到 released。
//!
//! 三道闸（见 src/evolution/release.rs:256 起 / src/prompt_guard.rs）：
//!   闸 1 禁词：`validate_prompt_edit` 命中禁用词表 → 拒（纯函数，不触发 LLM，确定性）
//!   闸 2 锚点：`validate_prompt_edit` 锚段缺失 → 拒（末尾追加保留原文，正例天然过）
//!   闸 3 语义：`review_prompt_edit` 调 LLM 判定 violation=true → 拒
//!
//! setup 照搬 `tests/evolution_rollback_status.rs`：同样走 release.rs 的 mongo 事务路径，
//! 因此用 `TestApp::start_repl_set()`（standalone mongod 无法 commit 多文档事务）。
//! LLM 通过 `TestApp` 内置的 `TestLlmGenerator` mock 排队响应——把闸 3 的判定做成确定性，
//! 整个文件无需真实大模型即可在 CI Docker 环境稳定跑（真模型语义行为另由 nightly
//! real-llm 套件覆盖）。
//!
//! 全部 `#[ignore]`：需 Docker（testcontainers MongoDB），本地不跑、CI `--ignored` 跑。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;
use wechatagent::evolution::error::EvolutionError;

/// 被 release_prompt 改写的强约束层模板 key（已 seed，含红线 + 业务锚）。
const TARGET_KEY: &str = "user.reply.policy";

/// 用字符拼接构造禁用词，绕源码字面量 lint（与 prompt_guard.rs::forbidden_phrase 同模式）。
/// tests/ 目录虽不在 no-human-takeover 扫描区，仍按既有约定稳妥拼接。
fn forbidden_phrase() -> String {
    ["人", "工", "接", "管"].concat()
}

/// 直插一条 `proposal_kind="prompt"` + `status="eligible_for_release"` 的 proposal，
/// 只填 release_prompt 实际读取的字段（kind / status / proposed_template_key /
/// diff_snippet / workspace_id / account_id）。返回其 `ObjectId`。
/// 用 raw collection insert，避开 Proposal 30+ 字段字面量——与 common::insert_released_prompt_proposal 同思路。
async fn insert_eligible_prompt_proposal(
    state: &wechatagent::routes::AppState,
    workspace: &str,
    key: &str,
    diff_snippet: &str,
) -> ObjectId {
    let id = ObjectId::new();
    let now = DateTime::now();
    let current = state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": workspace,
                "prompt_key": key,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("read current prompt template")
        .expect("current prompt template");
    let base_revision = wechatagent::evolution::revision::prompt_revision(
        current.id.expect("current prompt id"),
        current.version,
        &current.content,
    );
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .insert_one(
            doc! {
                "_id": id,
                "experiment_id": "test-exp",
                "workspace_id": workspace,
                "account_id": state.config.default_account_id.clone(),
                "proposal_kind": "prompt",
                "status": "eligible_for_release",
                "proposed_template_key": key,
                "proposed_section": "policy",
                "diff_snippet": diff_snippet,
                "base_revision": base_revision,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("insert eligible prompt proposal");
    id
}

/// 读 (workspace, key, current_version=true) 那条的 (version, content)。
async fn current_version_snapshot(
    state: &wechatagent::routes::AppState,
    workspace: &str,
    key: &str,
) -> (i32, String) {
    let row = state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": workspace, "prompt_key": key, "current_version": true },
            None,
        )
        .await
        .unwrap()
        .expect("seed 应保证有 current_version 行");
    (row.version, row.content)
}

/// 读 proposal 当前 status。
async fn proposal_status(state: &wechatagent::routes::AppState, id: ObjectId) -> String {
    state
        .db
        .proposals()
        .find_one(doc! { "_id": id }, None)
        .await
        .unwrap()
        .expect("proposal exists")
        .status
}

/// 闸 1（禁词，纯函数）：diff_snippet 含拼接出的禁用词 → release 被拒，不写库、proposal 不推进。
/// 禁词在闸 1 即被拦，**不触发** LLM，故无需排队任何 mock 响应，确定性最强。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn release_prompt_rejects_forbidden_word_snippet() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let state = common::evolution_release_state(&app, &workspace).await;

    let (before_version, before_content) =
        current_version_snapshot(&state, &workspace, TARGET_KEY).await;

    // 末尾追加片段含禁用词 → compose 后过闸 1 必拒。
    let snippet = format!("遇到难题就{}给后台", forbidden_phrase());
    let proposal_id =
        insert_eligible_prompt_proposal(&state, &workspace, TARGET_KEY, &snippet).await;

    let result = wechatagent::evolution::release::release_prompt(
        &state,
        proposal_id,
        &workspace,
        &app.state.config.default_account_id,
        "admin",
    )
    .await;

    // 1. 返回 RedlineGateRejected（不是 InvalidStatus / 其它）。
    match result {
        Err(EvolutionError::RedlineGateRejected(_)) => {}
        other => panic!("禁词候选必须被红线闸拒绝，实际：{other:?}"),
    }

    // 2. prompt_templates 没有新版本：current 行版本号 + 内容均不变。
    let (after_version, after_content) =
        current_version_snapshot(&state, &workspace, TARGET_KEY).await;
    assert_eq!(after_version, before_version, "被拒不应写入新版本");
    assert_eq!(after_content, before_content, "被拒不应改动 current 内容");

    // 3. proposal.status 仍是 eligible_for_release（未推进 released）。
    assert_eq!(
        proposal_status(&state, proposal_id).await,
        "eligible_for_release",
        "被拒的 proposal 不应推进到 released"
    );

    // 4. mock LLM 一次都没被调用——闸 1 在 LLM 之前就拦下。
    assert_eq!(app.llm.calls(), 0, "禁词在闸 1 拦截，不应触达 LLM 语义闸");
}

/// 闸 3（LLM 语义）：无禁词、保留锚点的合法追加片段，但 LLM 判定 violation=true →
/// release 仍被拒，不写库、proposal 不推进。用 mock 把 LLM 判定固定为「违规」，确定性。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn release_prompt_rejects_semantic_violation_snippet() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let state = common::evolution_release_state(&app, &workspace).await;

    let (before_version, before_content) =
        current_version_snapshot(&state, &workspace, TARGET_KEY).await;

    // 纯业务措辞（无禁词、原文逐字保留 → 过闸 1+2），靠 mock 让闸 3 判违规。
    let snippet = "补充：遇到复杂情况优先安抚情绪。";
    let proposal_id =
        insert_eligible_prompt_proposal(&state, &workspace, TARGET_KEY, snippet).await;

    // 闸 3 调一次 generate_agent_json（review_prompt_edit）→ 排一条 violation=true。
    app.llm
        .push_response(json!({ "violation": true, "reason": "变相引入真人转介" }));

    let result = wechatagent::evolution::release::release_prompt(
        &state,
        proposal_id,
        &workspace,
        &app.state.config.default_account_id,
        "admin",
    )
    .await;

    match result {
        Err(EvolutionError::RedlineGateRejected(_)) => {}
        other => panic!("LLM 判违规的候选必须被红线闸拒绝，实际：{other:?}"),
    }

    let (after_version, after_content) =
        current_version_snapshot(&state, &workspace, TARGET_KEY).await;
    assert_eq!(after_version, before_version, "语义闸拒绝不应写入新版本");
    assert_eq!(
        after_content, before_content,
        "语义闸拒绝不应改动 current 内容"
    );
    assert_eq!(
        proposal_status(&state, proposal_id).await,
        "eligible_for_release",
        "被语义闸拒绝的 proposal 不应推进到 released"
    );
    assert_eq!(app.llm.calls(), 1, "闸 1+2 已过，应触达闸 3 LLM 恰一次");
}

/// 合法放行对照：无禁词 + 保留锚点 + LLM 判 violation=false → release 成功。
/// 新版本 = 旧版本 + 1，内容以原 prompt 开头、以追加片段结尾，proposal 推进到 released。
/// 闸 3 真调 LLM，本测试用 mock 把判定固定为「合规」使其确定（真模型语义另由 nightly 套件覆盖）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn release_prompt_accepts_clean_append_snippet() {
    let app = common::TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let state = common::evolution_release_state(&app, &workspace).await;

    let (before_version, before_content) =
        current_version_snapshot(&state, &workspace, TARGET_KEY).await;

    let snippet = "补充：本行业语气更稳重。";
    let proposal_id =
        insert_eligible_prompt_proposal(&state, &workspace, TARGET_KEY, snippet).await;

    // 闸 3 判合规。
    app.llm.push_response(json!({ "violation": false }));

    wechatagent::evolution::release::release_prompt(
        &state,
        proposal_id,
        &workspace,
        &app.state.config.default_account_id,
        "admin",
    )
    .await
    .expect("合法追加片段应放行");

    // 新 current 行：version+1，内容 = 原文开头 + 片段结尾（末尾追加语义）。
    let (after_version, after_content) =
        current_version_snapshot(&state, &workspace, TARGET_KEY).await;
    assert_eq!(
        after_version,
        before_version + 1,
        "放行应写入 version+1 新版本"
    );
    assert!(
        after_content.starts_with(before_content.trim_end()),
        "新内容应以原 prompt 正文开头（红线逐字保留）"
    );
    assert!(
        after_content.trim_end().ends_with(snippet),
        "新内容应以追加片段结尾"
    );

    // proposal 推进到 released + 记录被替换的旧版本号。
    let released = app
        .state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .unwrap()
        .expect("proposal exists");
    assert_eq!(
        released.status, "released",
        "放行后 proposal 应推进到 released"
    );
    assert_eq!(
        released.previous_prompt_version.as_deref(),
        Some(before_version.to_string().as_str()),
        "released 行应记录被替换的旧版本号"
    );
}

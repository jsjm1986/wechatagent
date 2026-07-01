# M13 前端 saveOperationProfile 清空 profile_attributes 修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修 `update_operation_profile`(contacts.rs:799)无条件 `$set profile_attributes` 的数据丢失 bug:前端 saveOperationProfile 不发该字段 → payload `#[serde(default)]` 空 Document → 清空 AI 在 gateway 积累的画像。改成镜像 gateway.rs:4034 的「非空才写」守卫。

**Architecture:** admin handler 对 `profile_attributes` 加非空守卫,与 gateway 写回路径完全对齐:空(前端不发/发空)→ 不写 → 保留 AI 积累现值;非空 → 写入(合法覆写)。前端不改(不发该字段是正确的)。只改 src/routes/contacts.rs。

**Tech Stack:** Rust 2021 / Axum handler / MongoDB / testcontainers 集成测试(直调 handler,与 contact_manual_tags_integration.rs 同范式)。

## Global Constraints

- 分支:`fix/m13-profile-attributes-preserve`(从 origin/main b19df42 切,含 H7/H1/H11;spec commit 8487e8d 已在其上)。绝不 push main,只在 worktree `E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/e4-f21-closure` 干活。
- cargo 命令前:`export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"` + `export CARGO_INCREMENTAL=0`。磁盘紧先删 `target/debug/incremental`。
- 基线不回归:`cargo test --lib` ≥ 350 passed / 0 failed。新增回归测试是 `#[ignore]` + Docker 集成测试(不进 lib 计数),本地只编译(`--no-run`),断言留 CI integration job。
- 本地只跑 `cargo test --lib` + 编译集成 binary(`--no-run`);绝不本地全量 `cargo test`(磁盘 os error 112)。
- 过拟合红线:绝不为过测试改业务逻辑。测试锁「空 payload 不清空现值 / 非空正常写」两个真实不变量。
- 禁词 lint:不涉禁词(人工/接管/takeover/hand-off)。
- commit:具名 `git add`,绝不 -A/.;消息 `Co-Authored-By: Claude <noreply@anthropic.com>` 结尾。已授权 commit/push/PR/cron 监控 CI/squash 合并。
- 动手前先 Read `src/routes/contacts.rs` 确认 update_operation_profile 的 set_doc(:795-802)、OperationProfileRequest(:31-43)、handler 可见性(:759 pub(super))与本计划一致(行号可能漂移,以 string anchor 为准)。

---

## 文件结构

- **Modify:** `src/routes/contacts.rs` — (1) set_doc 移除无条件 profile_attributes、改非空才 insert;(2) `update_operation_profile` + `OperationProfileRequest` 改 `pub` 供直调集成测试。
- **Create:** `tests/contact_operation_profile_integration.rs` — `#[ignore]` + Docker 集成回归(前端式请求不清空画像 + 非空正常写 + tags 与画像并存)。

单任务:非空守卫 + 测试可见性 + 回归测试是对同一行为契约(「空 payload 不清空 profile_attributes」)的一次内聚改动。

---

## Task 1: profile_attributes 非空守卫 + 集成回归测试

**Files:** Modify `src/routes/contacts.rs`;Create `tests/contact_operation_profile_integration.rs`

**Interfaces:**
- Consumes: `update_operation_profile`(contacts.rs:759,现 pub(super))、`OperationProfileRequest`(contacts.rs:31,现 pub(super))、`AuthenticatedAdmin`、`Contact`(models.rs)、`TestApp`(tests/common/mod.rs)。
- Produces: `update_operation_profile` 行为修正(空 profile_attributes 不写);handler + 请求体改 pub。签名不变。

- [ ] **Step 1: 动手前先读码验证(不猜)**

Read `src/routes/contacts.rs`:
- `update_operation_profile`(约 :759)当前 `pub(super) async fn`;`OperationProfileRequest`(约 :31)当前 `pub(super) struct`,`profile_attributes` 字段带 `#[serde(default)]`。
- set_doc(约 :795-802)当前含无条件 `"profile_attributes": payload.profile_attributes,`。
- 确认 handler 尾部 `update_one` 用 `doc!{"_id": object_id, "workspace_id": &admin.current_workspace}` 过滤(约 :844)。

Read `tests/contact_manual_tags_integration.rs` 确认测试范式:`test_admin(ws)` 构造 `AuthenticatedAdmin`、`managed_contact(ws,acc,wxid)` 构造 Contact(注意 `profile_attributes: Document::new()` 字段在 :68)、`insert_one` seed、直调 handler、`serde_json::from_value` 构造私有请求体、reload 断言。

Read `tests/common/mod.rs` 确认 `TestApp::start()`、`app.state.config.default_workspace_id` / `default_account_id`。

若与本计划不符,以真实代码为准修正,report 记明分歧。

- [ ] **Step 2: 改可见性(handler + 请求体 pub)**

用 Edit 把 `pub(super) async fn update_operation_profile` 改 `pub async fn update_operation_profile`;`pub(super) struct OperationProfileRequest` 改 `pub struct OperationProfileRequest`。字段保持私有(测试用 serde_json::from_value 构造,同 manual_tags 范式)。

- [ ] **Step 3: 先写集成回归测试(TDD 红,本地只编译)**

Create `tests/contact_operation_profile_integration.rs`:
```rust
//! M13 红线集成测试:update_operation_profile 不得清空 AI 积累的 profile_attributes。
//! 前端 saveOperationProfile 只发 relationshipType/lastCommitment/followUpPolicy,
//! 不带 profileAttributes → payload #[serde(default)] 空 Document。旧 bug 无条件
//! $set → 清空 AI 画像。修复后非空才写(镜像 gateway.rs:4034)。
//! `#[ignore]` 需 Docker;CI:`cargo test --test contact_operation_profile_integration -- --ignored`。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use mongodb::bson::{doc, DateTime, Document};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{AgentStatus, Contact};
use wechatagent::routes::contacts::update_operation_profile;

use crate::common::TestApp;

fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "op_admin".to_string(),
        username: "op_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

fn managed_contact(ws: &str, acc: &str, wxid: &str, profile_attributes: Document) -> Contact {
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes,
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

async fn seed(app: &TestApp, c: Contact) -> String {
    app.state
        .db
        .contacts()
        .insert_one(c, None)
        .await
        .expect("seed contact")
        .inserted_id
        .as_object_id()
        .expect("oid")
        .to_hex()
}

async fn reload(app: &TestApp, wxid: &str) -> Contact {
    app.state
        .db
        .contacts()
        .find_one(doc! { "wxid": wxid }, None)
        .await
        .expect("query contact")
        .expect("contact exists")
}

/// M13 核心红线:前端式请求(不带 profileAttributes)不清空 AI 积累的 profile_attributes。
#[tokio::test]
#[ignore]
async fn front_end_style_request_preserves_profile_attributes() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let ai_attrs = doc! { "budget": "high", "decision_role": "owner" };
    let id = seed(&app, managed_contact(&ws, &acc, "wx_m13_a", ai_attrs.clone())).await;

    update_operation_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "relationshipType": "customer",
                "lastCommitment": "下周回复",
            }))
            .expect("构造前端式请求体(不带 profileAttributes)"),
        ),
    )
    .await
    .expect("update_operation_profile 应成功")
    .0;

    let c = reload(&app, "wx_m13_a").await;
    assert_eq!(
        c.profile_attributes, ai_attrs,
        "前端式请求不带 profileAttributes 时,AI 积累的 profile_attributes 必须原样保留(旧 bug 清空)"
    );
}

/// 对照:带非空 profileAttributes 时正常写入(证明守卫不误伤真实写)。
#[tokio::test]
#[ignore]
async fn non_empty_profile_attributes_is_written() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let id = seed(&app, managed_contact(&ws, &acc, "wx_m13_b", Document::new())).await;

    update_operation_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id),
        axum::Json(
            serde_json::from_value(serde_json::json!({
                "profileAttributes": { "budget": "low" },
            }))
            .expect("构造带 profileAttributes 的请求体"),
        ),
    )
    .await
    .expect("update_operation_profile 应成功")
    .0;

    let c = reload(&app, "wx_m13_b").await;
    assert_eq!(
        c.profile_attributes,
        doc! { "budget": "low" },
        "带非空 profileAttributes 时应正常写入"
    );
}

/// 不回归:更新 tags 的同时保留 profile_attributes(两者并存)。
#[tokio::test]
#[ignore]
async fn updating_tags_preserves_profile_attributes() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let ai_attrs = doc! { "budget": "mid" };
    let id = seed(&app, managed_contact(&ws, &acc, "wx_m13_c", ai_attrs.clone())).await;

    update_operation_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id),
        axum::Json(
            serde_json::from_value(serde_json::json!({ "tags": ["VIP"] }))
                .expect("构造带 tags 的请求体"),
        ),
    )
    .await
    .expect("update_operation_profile 应成功")
    .0;

    let c = reload(&app, "wx_m13_c").await;
    assert_eq!(c.tags, vec!["VIP".to_string()], "tags 应被更新");
    assert_eq!(
        c.profile_attributes, ai_attrs,
        "更新 tags 不应清空 profile_attributes"
    );
}
```

注意:`Contact.tags` 字段名以 Step 1 读到的 models.rs 为准(manual_tags 测试用的是 `manual_tags`;本测试断言的 `tags` 是 `OperationProfileRequest.tags` 写入的字段——Step 1 确认 set_doc 里 `"tags"` 落到 Contact 的哪个字段,若为 `tags` 则断言 `c.tags`,若无独立 `tags` 字段则删掉测试 3 的 tags 断言只保留 profile_attributes 保留断言)。

- [ ] **Step 4: 编译测试 binary(--no-run)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --test contact_operation_profile_integration --no-run 2>&1 | tail -20`
Expected: `Finished` + Executable。若报 `Contact` 字段不匹配 / `tags` 字段不存在,读 models.rs 的 Contact 定义修正(Step 1 应已确认字段全集)。本地无 Docker 不跑断言体。

- [ ] **Step 5: 改 set_doc 非空守卫(修复)**

用 Edit 改 set_doc(约 :795-802):从字面量移除 `"profile_attributes": payload.profile_attributes,`,并在 set_doc 构造后加非空守卫。

old_string:
```rust
    let mut set_doc = doc! {
        "tags": payload.tags,
        "commitments": commitments_bson,
        "follow_up_policy": normalize_optional(payload.follow_up_policy),
        "profile_attributes": payload.profile_attributes,
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
```
new_string:
```rust
    let mut set_doc = doc! {
        "tags": payload.tags,
        "commitments": commitments_bson,
        "follow_up_policy": normalize_optional(payload.follow_up_policy),
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    // 与 gateway.rs 写回一致:profile_attributes 非空才写。前端「运营画像」表单
    // 不管理 profile_attributes(它由 AI 在 gateway 积累),PUT 时不带该字段 →
    // payload 反序列化为空 Document。无条件 $set 会把 AI 积累的画像清空(M13),
    // 故空则跳过、保留现值。
    if !payload.profile_attributes.is_empty() {
        set_doc.insert("profile_attributes", payload.profile_attributes);
    }
```

- [ ] **Step 6: 编译 lib + 集成 binary,跑 lib 基线**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo build --lib 2>&1 | tail -6 && cargo test --test contact_operation_profile_integration --no-run 2>&1 | tail -6 && cargo test --lib 2>&1 | tail -6`
Expected: lib build `Finished` 无 error;集成 binary `Finished`;lib `test result: ok. N passed; 0 failed`,N ≥ 350(集成测试 #[ignore] 不进 lib 计数)。

- [ ] **Step 7: Commit**
```bash
git add src/routes/contacts.rs tests/contact_operation_profile_integration.rs
git commit -m "$(cat <<'EOF'
fix(contacts): profile_attributes 非空才写,不清空 AI 积累画像(M13)

update_operation_profile 无条件 $set profile_attributes。前端 saveOperationProfile
只发 relationshipType/lastCommitment/followUpPolicy,不带 profileAttributes →
OperationProfileRequest.profile_attributes(#[serde(default)])反序列化为空 Document
→ 无条件 $set 把 AI 在 gateway 积累的画像清空。运营点保存即触发,profile_attributes
确喂决策 prompt(decision.rs)。

镜像 gateway.rs:4034 的非空守卫:payload.profile_attributes 非空才写,空则跳过保留
现值。前端不改(不发该字段是正确的,bug 是后端把"没发"当"清空")。handler+请求体
改 pub 供直调集成测试。加 3 个集成回归(前端式请求不清空/非空正常写/tags 并存)。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec coverage:** §4.1 非空守卫→Step5 ✓;§4.2 可见性→Step2 ✓;§6 三测试→Step3 ✓。
**Placeholder scan:** 无 TBD;每步给完整 old/new_string 或完整测试代码;commit 消息完整。Step3 对 `c.tags` 断言留了「以 Step1 读到的 Contact 字段为准」的兜底说明(不是占位,是明确的条件分支指令)。
**Type consistency:** `payload.profile_attributes: Document`,`.is_empty() -> bool`,`set_doc.insert(&str, Document)` ✓;handler 返回 `AppResult<Json<Value>>`,测试 `.await.expect(...).0` 取 Json 内层 ✓;`update_operation_profile(State, Extension, Path, Json)` 四参与直调匹配 ✓。
**注意(TDD):** 本地无 Docker,红态由「旧 bug 下 $set {} 清空→断言失败」逻辑保证,断言真值留 CI;Step 5 修复后 CI 绿。commit 在 lib 基线绿 + 集成 binary 编译过后。

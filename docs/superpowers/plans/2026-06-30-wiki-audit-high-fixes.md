# wiki 审查两个 High 缺陷修复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 wiki 审查发现的两个 High 缺陷——#1 `domain_schemas.rs` serde 字段名错配（动态字段校验静默失效），#2 `prompt_templates.rs` create/publish 绕过 prompt 红线闸。

**Architecture:** #1 把路由层 11 处 camelCase 查询键改回 snake_case，与模型 `insert_one` 实写及全项目约定对齐；新增 testcontainers 集成测试锁 `load_active_domain_schema` 读链路。#2 给 `create` 补字面双闸、给 `publish` 补字面双闸 + LLM 语义三闸（force 跳 LLM 不跳字面），publish 端点改收可选 body；新增集成测试仿 `evolution_release_redline.rs`；前端两个 publish 调用点改为读三态 + 带 force 重提。

**Tech Stack:** Rust 2021 / Axum / MongoDB（mongodb crate）；testcontainers-modules（Mongo）；前端 React 19 + TS + Vitest。

**来源 spec:** `docs/superpowers/specs/2026-06-30-wiki-audit-high-fixes-design.md`

## Global Constraints

- Mongo 字段命名约定：snake_case（`db/indexes.rs` 所有集合一致）；对外 JSON 响应：camelCase（serde rename 或手写 json! 投影）。两者不可混。
- 不为通过测试而改业务逻辑 / 阈值 / 断言（过拟合红线）。真 bug 才修。
- CLAUDE.md「无人工接管」红线：新增**非测试**代码绝不内联禁用词字面量（`src/prompt_guard.rs` 及 `src/` 顶层在 CI lint 扫描区）；测试构造禁用词必须用字符拼接（如 `["人","工","接","管"].concat()`）。`scripts/check-no-human-takeover.sh` 必须 0 violations。
- 红线闸调用路径用既有的 `crate::routes::management_prompt_edit::{validate_prompt_edit, review_prompt_edit, PromptEditVerdict}`（它是 `crate::prompt_guard` 的 re-export 壳，`prompt_templates.rs:150` 已在用此路径，保持一致）。
- 集成测试一律 `#[ignore]`（需 Docker testcontainers）；本地只验编译（`--no-run`）+ `cargo test --lib`，`--ignored` 实跑留 CI。
- CI baseline 门：`cargo test --lib` ≥350/0；`RUSTFLAGS="-D warnings" cargo check --tests` EXIT=0（dead-code 门）。跑 lib 测试前 `touch src/lib.rs` 强制 relink，规避共享 target stale 二进制。
- `AppError` 变体：`BadRequest(String)` / `NotFound(String)` 已存在，直接用。

---

### Task 1: 修复 #1 domain_schemas serde 字段名错配

**Files:**
- Modify: `src/routes/domain_schemas.rs`（11 处 camelCase 查询键改 snake_case，见下）

**Interfaces:**
- Consumes: 无（独立修复）。
- Produces: 修复后 `load_active_domain_schema(db, ws)` 的 filter 为 `{ "workspace_id": ws, "is_active": true }`（snake_case），与 `insert_one` 序列化出的字段名一致。Task 2 的集成测试依赖此。

**改动点清单**（把这三个 camelCase 键的字面量改为 snake_case，仅限出现在 Mongo `doc!` filter / `$set` / sort 里的）：
- `"workspaceId"` → `"workspace_id"`：:176, :252, :277, :286, :308, :324, :345, :358, :371, :406, :522
- `"isActive"` → `"is_active"`：:178, :359, :361, :374
- `"updatedAt"` → `"updated_at"`：:266, :361, :374

**不改**：`DomainSchema` 模型（`models.rs`，保持无 rename_all）；`DomainSchemaView`（对外响应 struct，`rename_all="camelCase"` 正确）；`ListQuery`/`UpsertRequest`/`DomainFieldPayload`（请求体 payload，camelCase 正确）。

- [ ] **Step 1: 改 `list_domain_schemas` 的 filter（:176, :178）**

把 `:176` 的 `let mut filter = doc! { "workspaceId": &workspace_id };` 改为 `"workspace_id"`；把 `:178` 的 `filter.insert("isActive", true);` 改为 `filter.insert("is_active", true);`。

- [ ] **Step 2: 改 `update_domain_schema`（:252, :266, :277, :286）**

- `:252` find_one filter：`"workspaceId"` → `"workspace_id"`（`"schema_id"` 保持不动，本就对）。
- `:266` `$set` update doc：`"updatedAt": now` → `"updated_at": now`。
- `:277` update_one filter：`"workspaceId"` → `"workspace_id"`。
- `:286` refreshed find_one filter：`"workspaceId"` → `"workspace_id"`。

- [ ] **Step 3: 改 `delete_domain_schema`（:308, :324）**

- `:308` find_one filter：`"workspaceId"` → `"workspace_id"`。
- `:324` delete_many filter：`"workspaceId"` → `"workspace_id"`。

- [ ] **Step 4: 改 `activate_domain_schema`（:345, :358, :359, :361, :371, :374）**

- `:345` target find_one filter：`"workspaceId"` → `"workspace_id"`。
- `:358-361` update_many：filter `"workspaceId"` → `"workspace_id"`、`"isActive": true` → `"is_active": true`；`$set` `{ "isActive": false, "updatedAt": now }` → `{ "is_active": false, "updated_at": now }`。
- `:371-374` update_one：filter `"workspaceId"` → `"workspace_id"`；`$set` `{ "isActive": true, "updatedAt": now }` → `{ "is_active": true, "updated_at": now }`。
- 同函数末尾 refreshed find_one filter（约 :382-386，`"workspaceId"`）→ `"workspace_id"`（实施者按实际行核对，确保 activate 内全部 workspaceId 都改）。

- [ ] **Step 5: 改 `next_version_for`（:406）与 `load_active_domain_schema`（:522）**

- `:406` find_one filter：`"workspaceId"` → `"workspace_id"`。
- `:522` `load_active_domain_schema` find_one filter：`doc! { "workspaceId": workspace_id, "isActive": true }` → `doc! { "workspace_id": workspace_id, "is_active": true }`。

- [ ] **Step 6: 全文复查无残留 camelCase 查询键**

Run: `grep -n 'workspaceId\|isActive\|updatedAt' src/routes/domain_schemas.rs`
Expected: 仅剩 `DomainSchemaView`（:123-134 struct 字段名是 Rust 标识符 snake、靠 `rename_all` 转 camel，grep 命中的是 `#[serde(rename_all="camelCase")]` 注解或不命中具体键）、`ListQuery`/`UpsertRequest`/`DomainFieldPayload` 的 serde 字段。**确认 0 处出现在 `doc!` 查询/`$set`/sort 里**。若 grep 命中行号属于上述 payload/view struct 定义则 OK；属于 `doc!{}` 则漏改，补上。

- [ ] **Step 7: 编译验证**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -5`
Expected: `Finished` / EXIT=0（无 warning、无 error）。

- [ ] **Step 8: lib 基线不回归**

Run: `touch src/lib.rs && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350。

- [ ] **Step 9: Commit**

```bash
git add src/routes/domain_schemas.rs
git commit -m "fix(domain-schemas): 查询/\$set/sort字段名改回snake_case对齐insert_one实写

11处workspaceId/isActive/updatedAt错配致load_active_domain_schema恒None,
enforce_domain_attributes(required/enum)从不执行(R2连带)。模型无rename_all序列化
为snake_case,查询却用camelCase。改裸字面量为snake_case,与全项目doc!约定一致。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: #1 集成测试（domain_schema round-trip）

**Files:**
- Create: `tests/domain_schema_persistence_e2e.rs`

**Interfaces:**
- Consumes: `wechatagent::routes::domain_schemas::{load_active_domain_schema, enforce_domain_attributes}`（均 `pub`，签名见下）；`wechatagent::models::{DomainSchema, DomainField}`；`tests/common` 的 `TestApp`。
- `load_active_domain_schema(db: &Database, workspace_id: &str) -> AppResult<Option<DomainSchema>>`
- `enforce_domain_attributes(schema: &DomainSchema, attrs: &Document) -> AppResult<Document>`
- Produces: 无（终端测试）。

**测试覆盖边界（诚实声明）**：5 个 handler 是 `pub(super)`，独立 crate 测试够不到，无法直接断言其 filter。本测试锁 `pub` 的 `load_active_domain_schema` 读链路——证明"写入字段名 ↔ 读取字段名一致"这一根因。handler 内 filter 字段名靠 Task 1 同源同改 + Task 5 终审逐处核保证。

- [ ] **Step 1: 写测试文件骨架 + round-trip 测试**

`tests/domain_schema_persistence_e2e.rs`：

```rust
//! 集成测试：domain_schemas 写入字段名 ↔ 读取字段名一致性（#1 serde 错配回归门）。
//!
//! 修复前路由层查询用 camelCase（workspaceId/isActive），而模型序列化为 snake_case，
//! 导致 load_active_domain_schema 恒 None、enforce_domain_attributes 从不执行。
//! 本测试用与 handler 等价的 DB 写入 + pub 读函数验证 round-trip。
//!
//! 全部 #[ignore]：需 Docker（testcontainers MongoDB），本地不跑、CI --ignored 跑。

mod common;

use mongodb::bson::{doc, DateTime};
use wechatagent::models::{DomainField, DomainSchema};
use wechatagent::routes::domain_schemas::{enforce_domain_attributes, load_active_domain_schema};

/// 构造一条 DomainSchema（与 create_domain_schema handler 构造的等价）。
fn make_schema(workspace: &str, schema_id: &str, is_active: bool, required_field: bool) -> DomainSchema {
    let now = DateTime::now();
    DomainSchema {
        id: None,
        schema_id: schema_id.to_string(),
        workspace_id: workspace.to_string(),
        name: format!("schema-{schema_id}"),
        version: 1,
        fields: vec![DomainField {
            name: "customer_stage".to_string(),
            label: "客户阶段".to_string(),
            kind: "enum".to_string(),
            required: required_field,
            allowed_values: Some(vec!["lead".to_string(), "won".to_string()]),
            alias_of: None,
        }],
        alias_dict: Default::default(),
        guard_dsl: None,
        is_active,
        created_at: now,
        updated_at: now,
    }
}

/// create→load round-trip：写入 active schema，load_active_domain_schema 应返回 Some。
/// 修复前 filter 用 {isActive:true}（camelCase 幽灵键）→ 恒 None → 本测试 fail（红→绿）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn load_active_finds_inserted_active_schema() {
    let app = common::TestApp::start().await;
    let ws = "ws-domain-test";
    let cfg = make_schema(ws, "sales_v1", true, true);
    app.state
        .db
        .domain_schemas()
        .insert_one(&cfg, None)
        .await
        .expect("insert schema");

    let loaded = load_active_domain_schema(&app.state.db, ws)
        .await
        .expect("load ok");
    assert!(loaded.is_some(), "插入 is_active=true 的 schema 后 load 必须返回 Some（修复前恒 None）");
    let loaded = loaded.unwrap();
    assert_eq!(loaded.schema_id, "sales_v1");
    assert!(loaded.is_active);
}
```

- [ ] **Step 2: 验证该测试在修复前会 fail（逻辑确认，不实跑）**

确认断言逻辑：修复前 `load_active_domain_schema` 用 `{ "isActive": true }`，而 `insert_one(&cfg)` 写的是 `is_active`（snake，模型无 rename_all）→ filter miss → `None` → `assert!(loaded.is_some())` fail。Task 1 改 filter 为 `is_active` 后 → Some → pass。这是有效的红→绿回归门。

- [ ] **Step 3: 加 enforce 真生效测试**

追加到同文件：

```rust
/// load 链路打通后，enforce_domain_attributes 能拿到 active schema 并对缺 required 字段 reject。
/// 验证的是「active schema 真能被加载」这一 IO 链路（enforce 纯函数本身已有单测）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn enforce_rejects_missing_required_after_load() {
    let app = common::TestApp::start().await;
    let ws = "ws-enforce-test";
    let cfg = make_schema(ws, "sales_v1", true, true); // customer_stage required
    app.state.db.domain_schemas().insert_one(&cfg, None).await.expect("insert");

    let schema = load_active_domain_schema(&app.state.db, ws)
        .await
        .expect("load ok")
        .expect("active schema present");
    // 缺 required 字段 customer_stage → enforce 应 reject。
    let attrs = doc! { "other": "x" };
    let result = enforce_domain_attributes(&schema, &attrs);
    assert!(result.is_err(), "缺 required 字段必须被 enforce reject");
}
```

- [ ] **Step 4: 加 activate 互斥测试**

追加到同文件：

```rust
/// activate 互斥：两条 schema 都标 active 写入后，模拟 activate B 的 update_many→false + update_one→true，
/// load 应返回 B。验证 activate 那段 $set { is_active } 的字段名命中（修复前写进 camelCase 幽灵键）。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn activate_switches_active_via_snake_case_set() {
    let app = common::TestApp::start().await;
    let ws = "ws-activate-test";
    let col = app.state.db.domain_schemas();
    col.insert_one(&make_schema(ws, "a", true, false), None).await.expect("insert a");
    col.insert_one(&make_schema(ws, "b", false, false), None).await.expect("insert b");

    // 等价 activate B：先把本 ws 全部 is_active 置 false，再把 B 置 true（snake_case）。
    col.update_many(doc! { "workspace_id": ws, "is_active": true }, doc! { "$set": { "is_active": false } }, None).await.expect("deactivate all");
    col.update_one(doc! { "workspace_id": ws, "schema_id": "b" }, doc! { "$set": { "is_active": true } }, None).await.expect("activate b");

    let loaded = load_active_domain_schema(&app.state.db, ws).await.expect("load").expect("some");
    assert_eq!(loaded.schema_id, "b", "activate B 后 load 应返回 B");
}
```

- [ ] **Step 5: 编译验证（不实跑 ignored）**

Run: `cargo test --test domain_schema_persistence_e2e --no-run 2>&1 | tail -5`
Expected: `Finished` / 编译通过（测试因 `#[ignore]` 不执行）。若报 `load_active_domain_schema` / `enforce_domain_attributes` 不可见，确认它们是 `pub`（domain_schemas.rs:515/544）且 `domain_schemas` mod 是 `pub(crate)`——若测试 crate 仍够不到，检查 `src/routes/mod.rs` 是否 `pub use` 或改用 `wechatagent::routes::AppState` 同款可见路径（既有集成测试已能 import `wechatagent::routes::...`，照搬其 import 风格）。

- [ ] **Step 6: Commit**

```bash
git add tests/domain_schema_persistence_e2e.rs
git commit -m "test(domain-schemas): #1 serde错配 round-trip 集成测试(testcontainers)

锁 load_active_domain_schema 读链路:插入is_active=true后load须Some(修复前恒None)、
enforce对缺required字段reject、activate互斥经snake_case \$set生效。
全部#[ignore]需Docker,CI --ignored跑。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 修复 #2 create 补字面双闸 + publish 补字面双闸 + LLM 三闸

**Files:**
- Modify: `src/routes/prompt_templates.rs`

**Interfaces:**
- Consumes: `crate::routes::management_prompt_edit::{validate_prompt_edit, review_prompt_edit, PromptEditVerdict}`（re-export 自 `crate::prompt_guard`）。
  - `validate_prompt_edit(template_key: &str, new_content: &str) -> Result<(), String>`
  - `review_prompt_edit(state: &AppState, workspace_id: &str, template_key: &str, old: &str, new: &str) -> PromptEditVerdict`
  - `PromptEditVerdict` 三态：`Pass` / `Reject(String)` / `NeedsHumanConfirm { diff: String, reason: String }`
- Produces: publish 端点新增可选 body `PublishRequest { force: Option<bool> }`；publish 返回体在 NeedsHumanConfirm 时为 `{ status: "needs_human_confirm", reason, diff }`（与 update 同形）。Task 4 测试、Task 5 前端依赖此契约。

- [ ] **Step 1: create 补字面双闸**

在 `create_prompt_template`（:89）的 `validate_prompt_template_input(&payload)?;`（:94）**之后**、`let latest = ...`（:95）**之前**，插入：

```rust
    // #2 修复：create 与 update 对齐，过字面双闸（禁用词 + 锚完整性）。
    // create 是写入全新整篇内容，对整篇过双闸语义正确；不加 LLM 第三闸
    //（无 old 基线做 diff，且该 draft 最终须经 publish，publish 关口兜 LLM 闸）。
    crate::routes::management_prompt_edit::validate_prompt_edit(&payload.prompt_key, &payload.content)
        .map_err(AppError::BadRequest)?;
```

- [ ] **Step 2: 加 `PublishRequest` payload struct**

在 `PromptTemplateRequest`（:32-47）之后插入：

```rust
/// publish 端点可选 body：force=true 时跳过 LLM 第三闸（管理者已逐字核对），
/// 但仍过字面双闸（禁词/锚完整性是确定性硬闸，force 不可绕）。无 body 时落 default（force=None）。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublishRequest {
    #[serde(default)]
    force: Option<bool>,
}
```

- [ ] **Step 3: publish 端点改签名 + 补两道闸**

把 `publish_prompt_template`（:226-269）整体改为（保留原有删旧版本 + 改 active 逻辑，在其前插入两道闸）：

```rust
pub(super) async fn publish_prompt_template(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    body: Option<Json<PublishRequest>>,
) -> AppResult<Json<Value>> {
    let force = body.and_then(|b| b.0.force).unwrap_or(false);
    let object_id = parse_object_id(&id)?;
    let template = state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("prompt template not found".to_string()))?;

    // #2 修复：publish 是 draft→active 的最终生效点，红线最该把守的关口。
    // 闸 1+2 字面双闸（禁词 + 锚完整性），force 不可绕。
    crate::routes::management_prompt_edit::validate_prompt_edit(
        &template.prompt_key,
        &template.content,
    )
    .map_err(AppError::BadRequest)?;

    // 闸 3 LLM 语义审查（审 diff 增量）。force=true 跳过（管理者已逐字核对）。
    if !force {
        // old 基线 = 当前 current_version=true（回退 status=active）那条的 content；
        // 查不到则空串（全文当增量审，与 update 加载 old 同构）。
        let old_content = state
            .db
            .prompt_templates()
            .find_one(
                doc! {
                    "workspace_id": &template.workspace_id,
                    "prompt_key": &template.prompt_key,
                    "current_version": true
                },
                None,
            )
            .await?
            .map(|t| t.content)
            .unwrap_or_default();
        match crate::routes::management_prompt_edit::review_prompt_edit(
            &state,
            &admin.current_workspace,
            &template.prompt_key,
            &old_content,
            &template.content,
        )
        .await
        {
            crate::routes::management_prompt_edit::PromptEditVerdict::Pass => {}
            crate::routes::management_prompt_edit::PromptEditVerdict::Reject(reason) => {
                return Err(AppError::BadRequest(format!(
                    "红线语义审查拒绝：{reason}（确认无误可带 force 覆盖）"
                )));
            }
            crate::routes::management_prompt_edit::PromptEditVerdict::NeedsHumanConfirm {
                diff,
                reason,
            } => {
                return Ok(Json(json!({
                    "status": "needs_human_confirm",
                    "reason": reason,
                    "diff": diff
                })));
            }
        }
    }

    state
        .db
        .prompt_templates()
        .delete_many(
            doc! {
                "workspace_id": &template.workspace_id,
                "prompt_key": &template.prompt_key,
                "_id": { "$ne": object_id }
            },
            None,
        )
        .await?;
    state
        .db
        .prompt_templates()
        .update_one(
            doc! { "_id": object_id },
            doc! { "$set": { "status": "active", "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    let _ = template;
    Ok(Json(json!({ "ok": true })))
}
```

> 注意：`body: Option<Json<PublishRequest>>` 让无 body 的旧 POST 仍可工作（axum 对缺失/空 body 给 None）。若 axum 版本对 `Option<Json<T>>` 在 content-type 缺失时报错，改用 `Json(body): Json<PublishRequest>` 配合前端始终传 `{}`——但优先 `Option<Json<_>>` 保向后兼容。

- [ ] **Step 4: 编译验证**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -5`
Expected: `Finished` / EXIT=0。

- [ ] **Step 5: lib 基线**

Run: `touch src/lib.rs && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350。

- [ ] **Step 6: 双 lint**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -3`
Expected: 0 violations（本任务新增代码无禁用词字面量）。

- [ ] **Step 7: Commit**

```bash
git add src/routes/prompt_templates.rs
git commit -m "fix(prompt-templates): create/publish补红线闸,堵create-draft→publish绕过链

create补字面双闸(禁词+锚完整性)与update对齐;publish补字面双闸+LLM语义三闸
(force跳LLM不跳字面),draft→active最终生效点把守红线。publish改收可选body携force,
NeedsHumanConfirm返回{status:needs_human_confirm,reason,diff}与update同形。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: #2 集成测试（create/publish 红线闸）

**Files:**
- Create: `tests/prompt_template_redline_gate_e2e.rs`

**Interfaces:**
- Consumes: `tests/common` 的 `TestApp`（含 `TestLlmGenerator` mock）；直插/读 `state.db.prompt_templates()`；红线门函数同 Task 3。create/publish handler 是 `pub(super)` 够不到 → 测试用与 handler 等价的逻辑（调 `validate_prompt_edit` 验 create 闸；调 `validate_prompt_edit` + `review_prompt_edit` 验 publish 闸 + DB 状态）。
- Produces: 无（终端测试）。

**模式参照**：`tests/evolution_release_redline.rs`（同一套门 + mock LLM + 禁词字符拼接）。禁用词用 `["人","工","接","管"].concat()`。

> 说明：handler 是 `pub(super)`，测试无法直调。因此本测试**直接验证 Task 3 接入的门函数 + DB 写入语义**：(a) create 闸 = `validate_prompt_edit(key, content)` 对禁词/锚漂移 Err、对干净 Ok（这是 create 实际调的同一函数同一参数）；(b) publish 闸 = 先 `validate_prompt_edit` 再 `review_prompt_edit`，用 mock LLM 控制三态，并验证「被拒时不改 status / 放行时改 active」。门函数本身在 `prompt_guard.rs` 已有纯函数单测，本测试补的是「create/publish 确实接了这些门」这一集成事实——通过验证门对相同输入的判定 + publish 后 DB 状态。
>
> **覆盖弱点（诚实声明，交终审补足）**：handler `pub(super)` 不可直调，意味着本测试**无法证明 handler 真的调了门函数**——它只证明「门函数对这些输入会拒/会过」+「DB 状态语义正确」。"create/publish 确实接线了门"这一事实，必须由 Task 6 全分支终审**逐行 review `prompt_templates.rs` 代码**确认（验证 Step 1 的 create 闸、Step 3 的 publish 两道闸真的写进了 handler 体、在 DB 写入之前、且 force 只跳 LLM 不跳字面）。这是 `pub(super)` 的客观限制，不是可通过加测试消除的——终审 review 是这条接线的唯一保证。

- [ ] **Step 1: 文件骨架 + create 字面双闸验证**

```rust
//! 集成测试：prompt_templates create/publish 红线闸（#2 绕过链回归门）。
//!
//! create 补字面双闸、publish 补字面双闸+LLM三闸后，触碰红线的内容不得入库/激活。
//! handler 是 pub(super) 够不到 → 验证 Task 3 接入的门函数对相同输入的判定 + DB 状态。
//! 仿 tests/evolution_release_redline.rs。全部 #[ignore] 需 Docker。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;
use wechatagent::routes::management_prompt_edit::{
    review_prompt_edit, validate_prompt_edit, PromptEditVerdict,
};

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
```

- [ ] **Step 2: 编译验证骨架**

Run: `cargo test --test prompt_template_redline_gate_e2e --no-run 2>&1 | tail -5`
Expected: `Finished`。若 `wechatagent::routes::management_prompt_edit` 不可见（mod 是私有 `mod management_prompt_edit;`，见 `routes/mod.rs:62`），改 import 为 `wechatagent::prompt_guard::{review_prompt_edit, validate_prompt_edit, PromptEditVerdict}`（`lib.rs:28` `pub mod prompt_guard` 是公开的）。**优先用 `wechatagent::prompt_guard`**（公开路径）。

- [ ] **Step 3: publish 闸 — 删红线锚的 draft 被拒、status 不变**

追加（验证 publish 第一道字面双闸 + DB 状态语义）：

```rust
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
    let row = app.state.db.prompt_templates().find_one(doc! { "_id": id }, None).await.unwrap().unwrap();
    assert!(
        validate_prompt_edit(&row.prompt_key, &row.content).is_err(),
        "删红线锚的 draft 过 publish 字面双闸必须被拒"
    );
    // 该行仍是 draft（未被激活）。
    assert_eq!(row.status, "draft");
}
```

- [ ] **Step 4: publish 闸 — LLM 三态（mock）**

追加（验证 publish 第三闸三态，用 mock LLM）：

```rust
/// publish 第三闸 LLM 语义：干净内容（过字面双闸）+ mock 判 violation=true → review 返回 Reject。
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn publish_llm_gate_rejects_semantic_violation() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    // 干净追加内容（保留锚由 seed 的 current 版本提供 old 基线；这里直接验 review 判定）。
    let clean_new = "补充：本行业语气更稳重。";
    app.llm.push_response(json!({ "violation": true, "reason": "变相引入真人转介" }));
    let verdict = review_prompt_edit(&app.state, &ws, TARGET_KEY, "原始内容", clean_new).await;
    assert!(matches!(verdict, PromptEditVerdict::Reject(_)), "LLM 判违规应 Reject");
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
    assert!(matches!(verdict, PromptEditVerdict::Pass), "LLM 判合规应 Pass");
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
```

> 注：`review_prompt_edit` 内部调 `load_prompt(key="management.prompt_redline_review.system")`——若 seed 未含该 key，会先走「指令加载失败→NeedsHumanConfirm」分支，导致 violation 测试也返回 NeedsHumanConfirm。实施者验证：`TestApp::start()` 的 `ensure_prompt_pack_v2` 是否 seed 了 `management.prompt_redline_review.system`。若没有，在测试 setup 里直插该 prompt 行（raw insert 到 prompt_templates，key=`management.prompt_redline_review.system`, status=active, current_version=true），或复用 `evolution_release_redline.rs` 已验证可跑的同款 setup（它跑通了 review_prompt_edit 的 mock 路径，照搬其 prompt seed 前提）。

- [ ] **Step 5: 编译验证**

Run: `cargo test --test prompt_template_redline_gate_e2e --no-run 2>&1 | tail -5`
Expected: `Finished`。

- [ ] **Step 6: Commit**

```bash
git add tests/prompt_template_redline_gate_e2e.rs
git commit -m "test(prompt-templates): #2 create/publish红线闸集成测试(testcontainers)

create字面双闸拒禁词/锚漂移;publish删锚draft被拒+status不变;
publish LLM三态(violation=true→Reject/false→Pass/不可用→NeedsHumanConfirm不放水)。
仿evolution_release_redline,mock LLM,禁词字符拼接。全#[ignore]需Docker。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 前端 publish 调用点读三态 + force 重提

**Files:**
- Modify: `frontend/src/stores/strategyStore.ts:236-248`（`publishPromptTemplate`）
- Modify: `frontend/src/features/quality/index.tsx:463`（save 内链式 publish）
- Test: `frontend/src/__tests__/stores/promptSaveThreeState.test.ts`（追加 publish 三态用例）或新增 `frontend/src/__tests__/stores/publishThreeState.test.ts`

**Interfaces:**
- Consumes: Task 3 的 publish 契约——POST `/api/prompt-templates/:id/publish` 带可选 body `{ force?: boolean }`；200 返回 `{ status:"needs_human_confirm", reason, diff }`（需确认）或 `{ ok:true }`；4xx body `{error:"红线语义审查拒绝：…"}`。
- 复用：`SavePromptResult` 类型（strategyStore.ts 已有，含 `needsConfirm`/`rejected`/`ok`/`error`）；`confirm()` 弹框 + `promptDiffBody()`（quality/index.tsx 已用）。

- [ ] **Step 1: 改 `strategyStore.ts` 的 `publishPromptTemplate` 读三态**

把 `publishPromptTemplate`（:236-248）改为接收可选 force、读 needs_human_confirm、catch reject（仿同文件 `savePromptTemplate:202-234` 的三态返回）：

```typescript
  publishPromptTemplate: async (id: string, force?: boolean): Promise<SavePromptResult> => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const resp = await api.post<{ status?: string; reason?: string; diff?: string }>(
        `/api/prompt-templates/${id}/publish`,
        force ? { force: true } : {}
      );
      // needs_human_confirm 是 200，不能当成功 reload。
      if (resp && resp.status === "needs_human_confirm") {
        return { needsConfirm: true, reason: resp.reason ?? "", diff: resp.diff ?? "" };
      }
      await get().loadStrategyData();
      return { ok: true };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("红线语义审查拒绝")) {
        return { rejected: true, reason: message };
      }
      useUiStore.getState().setError(message);
      return { error: true, reason: message };
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },
```

确认 `publishPromptTemplate` 的类型签名（在 store interface 定义处）同步改为 `(id: string, force?: boolean) => Promise<SavePromptResult>`。找到调用 `publishPromptTemplate(` 的组件，让其消费三态（needsConfirm → 弹 confirm → 带 true 重调），照同文件 savePromptTemplate 消费方的现有模式。

- [ ] **Step 2: 改 `quality/index.tsx` 链式 publish 读三态**

`quality/index.tsx:463` 当前是 `await api.post(.../publish);`。改为读返回值、处理 needs_human_confirm + reject（复用本文件已 import 的 `confirm` + `promptDiffBody`）：

```typescript
      const pubResp = await api.post<{ status?: string; reason?: string; diff?: string }>(
        `/api/prompt-templates/${template.id}/publish`,
        publishForce ? { force: true } : {}
      );
      if (pubResp && pubResp.status === "needs_human_confirm") {
        setSaving(false);
        const ok = await confirm({
          title: "发布前需逐字核对后确认",
          body: promptDiffBody(pubResp.reason ?? "", pubResp.diff ?? ""),
          tone: "danger",
          confirmText: "已核对，强制发布",
          requireText: "已核对",
        });
        if (ok) {
          // 带 force 重发 publish（不再重走 update）。
          await api.post(`/api/prompt-templates/${template.id}/publish`, { force: true });
          setStatusMsg("已发布，Rust 端缓存已失效；下一次 review 即生效。");
          await load();
        }
        return;
      }
      setStatusMsg("已发布，Rust 端缓存已失效；下一次 review 即生效。");
      await load();
```

> 实施者注意：`quality/index.tsx` 的 `save()` 已有 `force` 参数（用于 update 的 LLM 闸）。publish 的 force 是独立的——publish 可能在 update 通过后仍被 publish 自己的 LLM 闸拦。用一个局部 `publishForce` 或直接在 confirm 后内联带 `{force:true}` 重发（如上）。catch 块的 reject 处理已存在（:466-479），publish 的 4xx reject 会被同一 catch 捕获，复用即可。

- [ ] **Step 3: 写前端测试（publish 三态）**

新增 `frontend/src/__tests__/stores/publishThreeState.test.ts`，照 `promptSaveThreeState.test.ts` 既有 mock 风格（`vi.mock("../../lib/api")` + `mockResolvedValueOnce`）写完整断言：

```typescript
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStrategyStore } from "../../stores/strategyStore";
import { useUiStore } from "../../stores/uiStore";
import { api } from "../../lib/api";

// #2 修复：后端 publish 补 LLM 红线三闸后，publishPromptTemplate 必须读 200 三态，
// needs_human_confirm 不能静默当成功 reload。
vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

describe("strategyStore.publishPromptTemplate 三态", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({ busy: false, error: "" });
    useStrategyStore.setState({
      loadStrategyData: vi.fn().mockResolvedValue(undefined),
    });
  });

  it("ok(200 {ok:true}) → 返回 {ok:true} 且触发 reload", async () => {
    (api.post as ReturnType<typeof vi.fn>).mockResolvedValueOnce({ ok: true });
    const result = await useStrategyStore.getState().publishPromptTemplate("pt-1");
    expect(result).toEqual({ ok: true });
    expect(useStrategyStore.getState().loadStrategyData).toHaveBeenCalledTimes(1);
  });

  it("NeedsHumanConfirm(200 body) → 返回 {needsConfirm} 且不 reload", async () => {
    (api.post as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      status: "needs_human_confirm",
      reason: "审查服务暂不可用",
      diff: "+变相转介",
    });
    const result = await useStrategyStore.getState().publishPromptTemplate("pt-1");
    expect(result).toEqual({ needsConfirm: true, reason: "审查服务暂不可用", diff: "+变相转介" });
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });

  it("Reject(4xx Error 含『红线语义审查拒绝』) → 返回 {rejected}", async () => {
    (api.post as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error("红线语义审查拒绝：变相引入真人转介（确认无误可带 force 覆盖）")
    );
    const result = await useStrategyStore.getState().publishPromptTemplate("pt-1");
    expect(result).toMatchObject({ rejected: true });
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });

  it("force=true → POST body 带 force:true", async () => {
    (api.post as ReturnType<typeof vi.fn>).mockResolvedValueOnce({ ok: true });
    await useStrategyStore.getState().publishPromptTemplate("pt-1", true);
    expect(api.post).toHaveBeenCalledWith(
      "/api/prompt-templates/pt-1/publish",
      expect.objectContaining({ force: true })
    );
  });
});
```

- [ ] **Step 4: tsc + vitest**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -5`
Expected: 0 error。

Run: `cd frontend && npx vitest run src/__tests__/stores/ src/__tests__/features/quality/ --pool=forks 2>&1 | tail -15`
Expected: 全绿（threads 此 worktree 会超时，必须 `--pool=forks`）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/stores/strategyStore.ts frontend/src/features/quality/index.tsx frontend/src/__tests__/
git commit -m "feat(fe): publish调用点读needs_human_confirm三态+force重提

后端publish补LLM红线三闸后,两个publish调用点(strategyStore.publishPromptTemplate
+quality save内链式publish)改为读200三态:needs_human_confirm弹逐字核对框→勾选带
force:true重提;reject复用既有catch。复用SavePromptResult/confirm/promptDiffBody。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 验证（全任务完成后）

1. `RUSTFLAGS="-D warnings" cargo check --tests`（EXIT=0）。
2. `touch src/lib.rs && cargo test --lib`（≥350/0）。
3. `cargo test --test domain_schema_persistence_e2e --no-run` + `cargo test --test prompt_template_redline_gate_e2e --no-run`（编译过；`--ignored` 实跑留 CI）。
4. `bash scripts/check-no-human-takeover.sh`（0 violations）；`git diff origin/main...HEAD` 新增行 0 命中禁词（测试文件的拼接构造不算字面量）。
5. 前端 `cd frontend && npx tsc --noEmit`（0）+ `npx vitest run --pool=forks` 相关用例全绿。
6. 终审任务须显式逐处核验 Task 1 改的 11 处字段名（handler filter 是 pub(super) 测试盲区）。

## 注意事项

- 当前 git 分支是 `feat/evolution-ui-toggle`（PR #72 已合并）。执行前须基于最新 `origin/main` 开新分支 `fix/wiki-audit-high-fixes`（spec commit acfe0d7 在旧分支上，需 cherry-pick 或在新分支重新提交 spec——执行时处理）。
- 集成测试 `--ignored` 不在本地跑（磁盘/Docker 受限），靠 CI；本地只验 `--no-run` 编译 + lib 基线。

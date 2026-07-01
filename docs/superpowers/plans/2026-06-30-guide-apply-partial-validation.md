# guide apply 部分应用 + prompt 注入合法值 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** guide apply 时,LLM 产出的越界枚举字段(operationState/customerStage/intentLevel)被跳过并记入 skippedFields 回流,合法字段照常落库;同时在 preview prompt 注入状态机/字典合法值,从源头压低 LLM 产越界值的概率。

**Architecture:** 后端在 `apply_contact_changes` 的三个枚举校验点把"越界 → 整请求 400"改为"越界 → 记 skipped + 跳过该字段",函数返回 `Vec<SkippedField>`;`guides.rs` apply handler 把 skipped 回流进响应体与审计事件;`preview` handler 查该 contact 的状态机合法态 + customer_stage/intent_level 字典 canonical 值,传入纯函数 `build_guide_preview_prompt` 注入提示文本。前端给 user-ops 频道挂 ToastProvider,apply 后用动态文案提示哪些字段被跳过。

**Tech Stack:** Rust 2021 + Axum + MongoDB(BSON);React 19 + TypeScript + Zustand + Vite。后端测试用 testcontainers MongoDB(`tests/common::TestApp`,`#[ignore]` 需 Docker)+ lib 内 `#[cfg(test)]` 纯函数单测。

设计依据:`docs/superpowers/specs/2026-06-30-guide-apply-partial-validation-design.md`(已逐节获批 + 自审 + 提交 732786b)。所有代码落点已 read 到行号。

## Global Constraints

- **不过拟合**(项目红线):注入合法值是普适机制(任意行业字典/状态机适用),跳过逻辑对任意越界字段一致;绝不为某条对话/某个越界值点对点打补丁。
- **agent-first**:前端提示文案由 `skippedFields` 动态拼接,不硬编码 "operationState" 等字段名。
- **新增测试只增量 append**,不删改旧维度/旧断言。
- **绝不触碰** `apply_admin_dim_validation` 共用 helper(shared.rs:105,同时服务 contacts.rs 手动表单 + management.rs AI 建档)、`contacts.rs::update_operation_profile` 手动表单的 AdminWrite 硬拒、`admin_relationship_suggestions` 审批硬拒。
- **测试基线不回归**:`cargo test --lib` ≥ 350 passed / 0 failed;4 PBT(state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter)累计 ≥ 33 / 0;`cargo check --tests` 0 error(复刻 CI step2)。
- **canonical 值(逐字,勿臆造)**:customer_stage 含 `new_contact` / `need_discovery` 等 9 项;intent_level 仅 `high` / `medium` / `low` 三档(**注意是 `medium` 不是 `mid`**,m006 seed)。状态机 `cooldown` 态 `allowFromAny:true`(从任意态合法)。
- **回复语言**:面向用户的对话回复用中文;代码标识符/注释/commit 文案遵循文件既有约定。
- 提交前每个 task 末尾 commit;只 `git add` 该 task 具名改动文件,不用 `git add -A`(工作树有并行会话产物)。

## 文件结构(改动落点)

| 文件 | 责任 | 改动类型 |
|---|---|---|
| `src/routes/shared.rs` | apply_contact_changes 越界跳过 + SkippedField 结构 + build_guide_preview_prompt 注入文本 + prompt 纯函数单测 | Modify |
| `src/routes/mod.rs` | `pub use shared::{apply_contact_changes, SkippedField}` 暴露给集成测试 | Modify(1-2 行) |
| `src/routes/guides.rs` | preview handler 查合法值;apply handler 绑定 skipped + 响应/审计回流 | Modify |
| `src/agent/mod.rs` | `pub(crate) use guards::operation_states;` | Modify(1 行) |
| `frontend/src/types/index.ts` | GuideSkippedField / UserOperationGuideApplyResult 类型 | Modify |
| `frontend/src/stores/userOpsStore.ts` | applyGuidePreview 用命名类型 + 返回结果 | Modify |
| `frontend/src/features/user-ops/index.tsx` | 挂 ToastProvider + 回调拼 skipped 提示 | Modify |
| `tests/guide_apply_partial_validation.rs` | apply 部分应用集成测试(4 个,`#[ignore]`) | Create |
| `scripts/biz-test/batch_c_guide.py` | 改断言验 skippedFields 回流 | Modify |

**任务顺序与依赖**:Task 1(后端 skip 核心 + 可见性)→ Task 2(apply 回流)→ Task 3(prompt 注入)→ Task 4(集成测试,依赖 1/3 的对外签名)→ Task 5(prompt 纯函数单测,依赖 3)→ Task 6(前端)→ Task 7(biz-test 断言 + 基线复核)。Task 1-3 是后端编译单元,建议连续完成;每个 task 独立可测、独立 commit。

---

### Task 1: apply_contact_changes 三字段越界跳过 + 记录 + 暴露给测试

**Files:**
- Modify: `src/routes/shared.rs:572-681`(apply_contact_changes 函数体)
- Modify: `src/routes/shared.rs:1`(顶部新增 SkippedField 结构体)
- Modify: `src/routes/mod.rs:141`(pub use 暴露)

**Interfaces:**
- Produces:
  - `pub struct SkippedField { pub field: String, pub reason: String }`(`#[derive(Debug, Clone)]`)
  - `pub(super) async fn apply_contact_changes(state: &AppState, contact: &Contact, changes: &Document) -> AppResult<Vec<SkippedField>>`(签名由 `AppResult<()>` 改为返回 `Vec<SkippedField>`)
  - 在 `src/routes/mod.rs` 经 `pub use shared::{apply_contact_changes, SkippedField};` 暴露(Task 4 集成测试从 `wechatagent::routes::apply_contact_changes` 调用)。
- Consumes:
  - `crate::agent::dimension_registry::{validate_dimension_value, WriteIntent, DimValidation}`(已在 crate 内)
  - `crate::agent::check_state_transition`(已 `pub use` at agent/mod.rs:146)
  - `crate::agent::load_user_operation_domain_config_for_contact`(已 `pub(crate) use` at agent/mod.rs:80)

**背景(为何这么改)**:当前 `apply_contact_changes` 所有字段共用一个 `set_doc`,三个枚举校验点(customerStage :589 / intentLevel :600+:622 / operationState :653)任一越界即 `?` 或 `return Err` 提前返回 400,导致已 insert 的合法字段(humanProfileNote/tags/...)全部陪葬。本 task 把这三处改为"越界 → push 到 skipped + 取 None/跳过写入",合法字段照落。**不改** `apply_admin_dim_validation`(它被 contacts.rs / management.rs 共用),而是在调用点**绕过它直接 match 原始 `DimValidation`** —— 因为 helper 已把 `Reject` 吞成 `Err`,经它就拿不到"跳过"语义。

- [ ] **Step 1: 顶部新增 SkippedField 结构体**

在 `src/routes/shared.rs` 顶部 `use` 块之后(约第 30 行附近,任意 item 定义之前)新增:

```rust
/// guide apply 中被跳过的越界字段(LLM 产出但不在字典/状态机内)。
/// 仅 guide 路径(apply_contact_changes)产出 —— 手动表单(contacts.rs)/审批
/// (admin_relationship_suggestions)路径的 AdminWrite 越界仍硬拒 400,不收集。
#[derive(Debug, Clone)]
pub struct SkippedField {
    /// camelCase 字段名,如 "operationState"(与 suggestedChanges 输入键一致)。
    pub field: String,
    /// 人类可读原因,如 "非法的 operation_state 迁移:...";直接回流给前端 toast。
    pub reason: String,
}
```

- [ ] **Step 2: 改函数签名 + 开头初始化 skipped + 结尾返回**

把 `src/routes/shared.rs:572-576` 的签名与开头:

```rust
pub(super) async fn apply_contact_changes(
    state: &AppState,
    contact: &Contact,
    changes: &Document,
) -> AppResult<()> {
    let mut set_doc = Document::new();
```

改为:

```rust
pub(super) async fn apply_contact_changes(
    state: &AppState,
    contact: &Contact,
    changes: &Document,
) -> AppResult<Vec<SkippedField>> {
    let mut set_doc = Document::new();
    let mut skipped: Vec<SkippedField> = Vec::new();
```

并把函数体内两处 `Ok(())` 都改成 `Ok(skipped)`:
- `:671-673` 的空判 early return:`if set_doc.is_empty() { return Ok(()); }` → `if set_doc.is_empty() { return Ok(skipped); }`
- `:680` 末尾的 `Ok(())` → `Ok(skipped)`

- [ ] **Step 3: customerStage 校验点(:584-620)改就地 match,越界记 skipped**

把 `:589-598` 的 `apply_admin_dim_validation(...)?` 与 `:599-611` 的 intent 同款,改为直接 match。整段(`:584` 的 `if let Some(value) = doc_get_string(changes, "customerStage") {` 块内,到 :611 `};` 为止)替换为:

```rust
    if let Some(value) = doc_get_string(changes, "customerStage") {
        // guide 路径(LLM 产值):越界 → 记 skipped 跳过(不像 contacts.rs 手动表单那样硬拒)。
        // 绕过 apply_admin_dim_validation(它把 Reject 吞成 Err)直接 match 原始 DimValidation。
        use crate::agent::dimension_registry::DimValidation::{Accept, DropSilently, Reject};
        let validated_stage = match agent::dimension_registry::validate_dimension_value(
            &state.db,
            "customer_stage",
            &value,
            &contact.account_id,
            agent::dimension_registry::WriteIntent::AdminWrite,
        )
        .await
        {
            Accept(s) => Some(s),
            DropSilently => None,
            Reject(reason) => {
                skipped.push(SkippedField {
                    field: "customerStage".to_string(),
                    reason,
                });
                None
            }
        };
        let intent = match doc_get_string(changes, "intentLevel") {
            Some(v) => match agent::dimension_registry::validate_dimension_value(
                &state.db,
                "intent_level",
                &v,
                &contact.account_id,
                agent::dimension_registry::WriteIntent::AdminWrite,
            )
            .await
            {
                Accept(s) => Some(s),
                DropSilently => None,
                Reject(reason) => {
                    skipped.push(SkippedField {
                        field: "intentLevel".to_string(),
                        reason,
                    });
                    None
                }
            },
            None => None,
        };
        if let Some(value) = validated_stage {
            // M2:customer_stage 实际变化时同步刷新 customer_stage_updated_at(归一后再比较)。
            let prev = contact_domain_str(contact, "customer_stage");
            let stage_changed = prev.as_deref().map(|s| s != value.as_str()).unwrap_or(true);
            insert_domain_stage_fields(&mut set_doc, Some(&value), intent.as_deref(), stage_changed);
        } else if intent.is_some() {
            // stage 越界/缺席但 intent 通过:仍写 intent(stage_changed=false,不刷 stage 计时)。
            insert_domain_stage_fields(&mut set_doc, None, intent.as_deref(), false);
        }
        // stage 与 intent 都 None(都越界被跳过)→ 不调 insert_domain_stage_fields,
        // 守住 :671 set_doc 空判不变量(否则会凭空写 domain_attributes_updated_at)。
```

(注意:这段保留原 `:612-620` 的三分支门控逻辑不变,只是把 stage/intent 来源从 `?` 改成 match 取 None。块的闭合 `}` 接在原 :620 之后,即下面 `} else if ... intentLevel` 分支。)

- [ ] **Step 4: intentLevel 独立分支(:621-635,stage 缺席的 else-if)改就地 match**

把 `:621-635`(`} else if let Some(value) = doc_get_string(changes, "intentLevel") {` 块)替换为:

```rust
    } else if let Some(value) = doc_get_string(changes, "intentLevel") {
        use crate::agent::dimension_registry::DimValidation::{Accept, DropSilently, Reject};
        let validated = match agent::dimension_registry::validate_dimension_value(
            &state.db,
            "intent_level",
            &value,
            &contact.account_id,
            agent::dimension_registry::WriteIntent::AdminWrite,
        )
        .await
        {
            Accept(s) => Some(s),
            DropSilently => None,
            Reject(reason) => {
                skipped.push(SkippedField {
                    field: "intentLevel".to_string(),
                    reason,
                });
                None
            }
        };
        if let Some(value) = validated {
            insert_domain_stage_fields(&mut set_doc, None, Some(&value), false);
        }
    }
```

- [ ] **Step 5: operationState 校验点(:639-664)改非法迁移记 skipped 跳过**

把 `:653-663`(`if let Some(reason) = agent::check_state_transition(...) { return Err(...); }` 到两行 insert)替换为:

```rust
        if let Some(reason) = agent::check_state_transition(
            domain_config.as_ref(),
            contact.operation_state.as_deref(),
            &value,
        ) {
            // guide 路径:LLM 产的非法迁移 → 记 skipped 跳过该字段(不像 contacts.rs 手动表单
            // 硬拒 400),其余合法字段照落。domain_config=None 时 check 返回 None,照写不变。
            skipped.push(SkippedField {
                field: "operationState".to_string(),
                reason: format!("非法的 operation_state 迁移:{reason}"),
            });
        } else {
            set_doc.insert("operation_state", value);
            set_doc.insert("operation_state_updated_at", DateTime::now());
        }
```

(原 `:665-667` 的 `operationStateReason` 分支保持不变 —— 它是自由文本,不过校验闸。注:若 operationState 被跳过而 operationStateReason 仍写入会留下孤儿 reason,但这是既有自由字段语义,YAGNI 不额外联动,设计 §九已界定。)

- [ ] **Step 6: mod.rs 暴露 apply_contact_changes + SkippedField 给集成测试**

在 `src/routes/mod.rs:141`(`pub use shared::upsert_contact_from_value;` 那行)后新增一行:

```rust
pub use shared::{apply_contact_changes, SkippedField};
```

(`mod shared;` 在 :88 是私有模块,集成测试 crate 只能看到 `pub use` 重导出的项,与 `upsert_contact_from_value` 同模式。)

- [ ] **Step 7: 编译验证**

Run: `cargo check --lib`
Expected: 0 error。可能的 warning:`skipped` 在某些分支未读 —— 不应出现,因为所有分支都 push 或最终 `Ok(skipped)` 读取。若报 `apply_admin_dim_validation` 未使用(本函数不再调它,但 contacts.rs/management.rs 仍调)→ 不会,它在别处有用。

- [ ] **Step 8: 提交**

```bash
git add src/routes/shared.rs src/routes/mod.rs
git commit -m "fix(guide): apply_contact_changes 越界字段跳过+记 skipped 不再连坐合法字段

三个枚举校验点(customerStage/intentLevel/operationState)由越界整请求 400
改为就地 match 原始 DimValidation/check_state_transition:越界记 SkippedField
+跳过该字段,合法字段照落。绕过共用 helper apply_admin_dim_validation(它把
Reject 吞成 Err),不影响 contacts.rs 手动表单/management.rs AI 建档两条路径。
签名返 Vec<SkippedField>,经 routes::mod pub use 暴露给集成测试。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: apply handler 绑定 skipped + 响应/审计回流

**Files:**
- Modify: `src/routes/guides.rs:158`(绑定 skipped)
- Modify: `src/routes/guides.rs:183-190`(审计 details 补 skippedFields)
- Modify: `src/routes/guides.rs:206-212`(响应体补 appliedFields / skippedFields)

**Interfaces:**
- Consumes: `apply_contact_changes(...) -> AppResult<Vec<SkippedField>>`(Task 1)
- Produces: apply 响应 JSON `item` 下新增 `appliedFields: string[]`、`skippedFields: [{field,reason}]` 两键(Task 6 前端消费)。

**背景**:`apply_user_operation_guide`(guides.rs:125-213)目前丢弃 apply_contact_changes 的返回。本 task 接住 skipped,算出 appliedFields(suggestedChanges 顶层键减去 skipped 的字段),回流进响应与审计事件。响应体当前包了一层 `{item: {...}}`(:206-212),新键加在 `item` 内。

- [ ] **Step 1: 绑定 apply_contact_changes 返回的 skipped**

把 `src/routes/guides.rs:158`:

```rust
    apply_contact_changes(&state, &contact, &preview.suggested_changes).await?;
```

改为:

```rust
    let skipped = apply_contact_changes(&state, &contact, &preview.suggested_changes).await?;
    // appliedFields = suggestedChanges 顶层键 - 被跳过的字段(给前端"已应用 N 项"用)。
    let skipped_names: std::collections::HashSet<&str> =
        skipped.iter().map(|s| s.field.as_str()).collect();
    let applied_fields: Vec<String> = preview
        .suggested_changes
        .keys()
        .filter(|k| !skipped_names.contains(k.as_str()))
        .cloned()
        .collect();
    let skipped_json: Vec<Value> = skipped
        .iter()
        .map(|s| json!({ "field": s.field, "reason": s.reason }))
        .collect();
```

(其余三个 apply_*_changes 签名不变,仍 `.await?`。)

- [ ] **Step 2: 审计事件 details 补 skippedFields**

把 `src/routes/guides.rs:183-190` 的 `details: Some(doc! { ... })`,在 `"suggestedChanges": preview.suggested_changes` 后补一行(注意前一行补逗号):

```rust
                details: Some(doc! {
                    "previewId": payload.preview_id,
                    "instruction": preview.instruction,
                    "impactScope": preview.impact_scope,
                    "scopeReason": preview.scope_reason,
                    "readableChanges": preview.readable_changes,
                    "suggestedChanges": preview.suggested_changes,
                    "skippedFields": mongodb::bson::to_bson(&skipped_json).unwrap_or(mongodb::bson::Bson::Array(vec![]))
                }),
```

- [ ] **Step 3: 响应体补 appliedFields / skippedFields**

把 `src/routes/guides.rs:206-212` 的 `Ok(Json(json!({ "item": { ... } })))`:

```rust
    Ok(Json(json!({
        "item": {
            "contact": ApiContact::from(updated_contact),
            "operatingMemory": operating_memory_json(memory),
            "health": health
        }
    })))
```

改为:

```rust
    Ok(Json(json!({
        "item": {
            "contact": ApiContact::from(updated_contact),
            "operatingMemory": operating_memory_json(memory),
            "health": health,
            "appliedFields": applied_fields,
            "skippedFields": skipped_json
        }
    })))
```

- [ ] **Step 4: 编译验证**

Run: `cargo check --lib`
Expected: 0 error。若报 `Value` 未导入 —— guides.rs:6 已 `use serde_json::{json, Value};`,无需补。

- [ ] **Step 5: 提交**

```bash
git add src/routes/guides.rs
git commit -m "feat(guide): apply 响应+审计回流 skippedFields/appliedFields

apply_user_operation_guide 接住 apply_contact_changes 返回的 skipped,
算出 appliedFields(suggestedChanges 键减 skipped),回流进 {item} 响应体
与 user_operation_guide_applied 审计事件 details,供前端提示+事后追溯。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: preview prompt 注入状态机/字典合法值(治源头)

**Files:**
- Modify: `src/agent/mod.rs:86`(新增 `pub(crate) use guards::operation_states;`)
- Modify: `src/routes/guides.rs:48-61`(preview handler 查合法值 + 传参)
- Modify: `src/routes/shared.rs:811-924`(build_guide_preview_prompt 加 3 入参 + 注入文本)

**Interfaces:**
- Consumes:
  - `agent::operation_states(Option<&OperationDomainConfig>) -> Vec<Document>`(guards.rs:105,本 task 新导出)
  - `agent::load_user_operation_domain_config_for_contact(&state, ws, wxid) -> AppResult<Option<OperationDomainConfig>>`(已导出 agent/mod.rs:80)
  - `agent::taxonomy::{global_taxonomy_cache, dimension_values_with_labels}`(均 pub(crate))
  - `TaxonomyCache::find_or_load(&self, &Database)`(async)
- Produces: `build_guide_preview_prompt` 新签名(末尾追加 3 个切片入参),Task 5 纯函数单测调用。

**背景**:`build_guide_preview_prompt`(shared.rs:811,同步纯函数)的 JSON 模板里 customerStage/intentLevel/operationState 只标"可选",没给合法值,LLM 凭空猜必然越界。本 task 让 preview handler 查该 contact 的状态机合法态 key + 字典 canonical 值(照搬 operation_view.rs:66-85 与 domain_profile.rs:1210-1230 范式),传入纯函数注入提示文本。

- [ ] **Step 1: agent/mod.rs 导出 operation_states**

在 `src/agent/mod.rs:86`(`pub(crate) use guards::initial_operation_state_key;` 那行)附近新增一行:

```rust
pub(crate) use guards::operation_states;
```

Run: `cargo check --lib`
Expected: 0 error(可能有 "unused import" warning,Task 3 后续步骤会用到 → 暂忽略,或先做 Step 2-3 再 check)。

- [ ] **Step 2: build_guide_preview_prompt 加 3 入参 + 注入文本**

把 `src/routes/shared.rs:811-819` 的签名:

```rust
pub(super) fn build_guide_preview_prompt(
    instruction: &str,
    mode: &str,
    contact: &Contact,
    memory: &OperatingMemory,
    playbook: Option<&OperationPlaybook>,
    review: Option<&AgentDecisionReview>,
    health: &Value,
) -> String {
```

改为(末尾追加 3 个切片入参):

```rust
pub(super) fn build_guide_preview_prompt(
    instruction: &str,
    mode: &str,
    contact: &Contact,
    memory: &OperatingMemory,
    playbook: Option<&OperationPlaybook>,
    review: Option<&AgentDecisionReview>,
    health: &Value,
    legal_states: &[String],
    stage_values: &[(String, String)],
    intent_values: &[(String, String)],
) -> String {
```

在函数体 `format!` 之前(`:820` 之前)新增合法值文本拼接(照搬 domain_profile.rs:1218 的"暂无受控取值"措辞):

```rust
    let render_states = if legal_states.is_empty() {
        "暂无受控取值,留空此字段(不要臆造)".to_string()
    } else {
        legal_states.join(" / ")
    };
    let render_pairs = |vals: &[(String, String)]| -> String {
        if vals.is_empty() {
            "暂无受控取值,留空此字段(不要臆造)".to_string()
        } else {
            vals.iter()
                .map(|(id, label)| format!("{id}({label})"))
                .collect::<Vec<_>>()
                .join(" / ")
        }
    };
    let render_stages = render_pairs(stage_values);
    let render_intents = render_pairs(intent_values);
```

然后在 `format!` 模板字符串里,把"当前健康度:{}"段(:904 那行 `当前健康度：{}"#,`)之后追加三段合法值提示。即把:

```rust
当前健康度：{}"#,
```

改为:

```rust
当前健康度：{}

可选枚举字段的合法取值(只能从下列里选,留空表示不改;绝不能臆造下列以外的值):
- operationState 合法值：{}
- customerStage 合法值：{}
- intentLevel 合法值：{}"#,
```

并在 `format!` 的参数列表末尾(`:922` 的 `serde_json::to_string(health).unwrap_or_default()` 之后,补逗号)追加三个参数:

```rust
        serde_json::to_string(health).unwrap_or_default(),
        render_states,
        render_stages,
        render_intents
```

- [ ] **Step 3: preview handler 查合法值并传入**

把 `src/routes/guides.rs:48-61`:

```rust
    let memory = ensure_operating_memory(&state, &contact).await?;
    let latest_review = latest_decision_review(&state, &contact).await?;
    let playbook = agent::load_operation_playbook_for_contact(&state, &contact).await?;
    let health = operation_health_json(&contact, &memory, latest_review.as_ref());
    let system = "你是微信私域用户运营产品里的 AI 引导助手。你的职责不是直接写聊天回复，而是根据运营人员的自然语言指令，生成一份可确认的配置修改预览。必须输出严格 JSON。";
    let user = build_guide_preview_prompt(
        &payload.instruction,
        payload.mode.as_deref().unwrap_or("smart"),
        &contact,
        &memory,
        playbook.as_ref(),
        latest_review.as_ref(),
        &health,
    );
```

改为(在 build 调用前查合法值,照搬 operation_view.rs:66-85 范式):

```rust
    let memory = ensure_operating_memory(&state, &contact).await?;
    let latest_review = latest_decision_review(&state, &contact).await?;
    let playbook = agent::load_operation_playbook_for_contact(&state, &contact).await?;
    let health = operation_health_json(&contact, &memory, latest_review.as_ref());
    // 注入合法值(治 LLM 产越界值的源头):状态机合法态 key + customer_stage/intent_level 字典 canonical。
    let domain_config = agent::load_user_operation_domain_config_for_contact(
        &state,
        &contact.workspace_id,
        &contact.wxid,
    )
    .await?;
    let legal_states: Vec<String> = agent::operation_states(domain_config.as_ref())
        .iter()
        .filter_map(|d| d.get_str("key").ok().map(String::from))
        .collect();
    let cache = agent::taxonomy::global_taxonomy_cache();
    cache.find_or_load(&state.db).await; // 冷/过期缓存返回空,先 load(幂等自愈)
    let stage_values = agent::taxonomy::dimension_values_with_labels(
        "customer_stage",
        &admin.current_workspace,
        cache.as_ref(),
    );
    let intent_values = agent::taxonomy::dimension_values_with_labels(
        "intent_level",
        &admin.current_workspace,
        cache.as_ref(),
    );
    let system = "你是微信私域用户运营产品里的 AI 引导助手。你的职责不是直接写聊天回复，而是根据运营人员的自然语言指令，生成一份可确认的配置修改预览。必须输出严格 JSON。";
    let user = build_guide_preview_prompt(
        &payload.instruction,
        payload.mode.as_deref().unwrap_or("smart"),
        &contact,
        &memory,
        playbook.as_ref(),
        latest_review.as_ref(),
        &health,
        &legal_states,
        &stage_values,
        &intent_values,
    );
```

- [ ] **Step 4: 编译验证**

Run: `cargo check --lib`
Expected: 0 error。验证点:`operation_states` 已被使用(Step 1 的 unused warning 消失);`dimension_values_with_labels` 返回 `Vec<(String,String)>` 与入参 `&[(String,String)]` 匹配。

- [ ] **Step 5: 提交**

```bash
git add src/agent/mod.rs src/routes/guides.rs src/routes/shared.rs
git commit -m "feat(guide): preview prompt 注入状态机/字典合法值治越界源头

preview handler 查该 contact 状态机合法态 key + customer_stage/intent_level
字典 canonical 值,传入 build_guide_preview_prompt 注入提示文本(照搬
operation_view/domain_profile 范式,含'暂无受控取值'兜底)。导出
guards::operation_states。LLM 据此优先命中合法值,不再凭空产 'active'。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: apply 部分应用集成测试(4 个,`#[ignore]` 需 Docker)

**Files:**
- Create: `tests/guide_apply_partial_validation.rs`

**Interfaces:**
- Consumes: `wechatagent::routes::{apply_contact_changes, SkippedField}`(Task 1 暴露);`tests/common::TestApp`(已存在)。

**背景**:`TestApp::start()`(tests/common/mod.rs:189-219)会 re-seed m006 taxonomy(customer_stage/intent_level 字典)+ 预热 taxonomy/domain_profile 缓存,并跑全部 migration(含状态机 seed)。所以 `validate_dimension_value("customer_stage","瞎填",AdminWrite)` 会命中字典并 `Reject`,DEFAULT 状态机让 `check_state_transition(...,"active")` 返回 `Some(reason)`。这是验证 skip 行为的前提,无需额外 seed 字典/状态机。

测试**直调 `apply_contact_changes`**(不经 HTTP),seed 一个 contact 到 `app.state.db`,构造 `changes: Document`,断言返回的 `Vec<SkippedField>` 与 contact 落库结果。`operation_state` 初始设 `Some("new_contact")`(DEFAULT 初始态),目标 `"active"` 是状态机里不存在的态 → 非法迁移被跳过。

- [ ] **Step 1: 写 4 个失败测试 + seed helper**

Create `tests/guide_apply_partial_validation.rs`:

```rust
//! guide apply 部分应用红线集成测试:LLM 产的越界枚举字段被跳过+记 skipped,
//! 合法字段照落;手动表单/审批路径的 AdminWrite 硬拒不在本文件范围。
//! 全部 `#[ignore]` 需 Docker。CI:`cargo test --test guide_apply_partial_validation -- --ignored`。
//!
//! ## 红线意义:apply_contact_changes 不能因单个 LLM 越界字段(operationState="active")
//! 整请求 400 把合法字段(humanProfileNote/customerStage/...)全陪葬。
#![cfg(test)]

mod common;

use mongodb::bson::{doc, DateTime, Document};
use wechatagent::models::{AgentStatus, Contact};
use wechatagent::routes::apply_contact_changes;

use crate::common::TestApp;

/// 构造一个 managed contact,operation_state 初始为 DEFAULT 初始态 new_contact。
fn seed_contact(ws: &str, acc: &str, wxid: &str) -> Contact {
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
        operation_state: Some("new_contact".to_string()),
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
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

/// seed + 取回带 _id 的 contact(apply_contact_changes 需要 contact.id)。
async fn insert_and_load(app: &TestApp, c: Contact, wxid: &str) -> Contact {
    app.state.db.contacts().insert_one(c, None).await.expect("seed contact");
    app.state
        .db
        .contacts()
        .find_one(doc! { "wxid": wxid }, None)
        .await
        .expect("query")
        .expect("contact exists")
}

/// 越界字段跳过,合法字段照落。
#[tokio::test]
#[ignore]
async fn apply_skips_invalid_keeps_valid() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = insert_and_load(&app, seed_contact(&ws, &acc, "wx_skip1"), "wx_skip1").await;

    // humanProfileNote 合法;operationState="active" 状态机无此态;customerStage="瞎填" 字典越界。
    let changes = doc! {
        "humanProfileNote": "关注价格",
        "operationState": "active",
        "customerStage": "瞎填一个不存在的阶段",
    };
    let skipped = apply_contact_changes(&app.state, &contact, &changes)
        .await
        .expect("apply 不应整体失败");

    // 合法字段真落库。
    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_skip1" }, None)
        .await
        .expect("query")
        .expect("exists");
    assert_eq!(
        after.human_profile_note.as_deref(),
        Some("关注价格"),
        "合法字段 humanProfileNote 必须落库"
    );
    // operationState 越界被跳过 → 保持初始 new_contact 不变。
    assert_eq!(
        after.operation_state.as_deref(),
        Some("new_contact"),
        "越界 operationState 不应写入,保持原态"
    );
    // 两个越界字段都进 skipped。
    let fields: Vec<&str> = skipped.iter().map(|s| s.field.as_str()).collect();
    assert!(fields.contains(&"operationState"), "operationState 应在 skipped: {fields:?}");
    assert!(fields.contains(&"customerStage"), "customerStage 应在 skipped: {fields:?}");
    assert_eq!(skipped.len(), 2, "恰两个越界字段被跳过");
}

/// 三枚举字段全越界、无其它合法字段 → set_doc 空判生效,contact 完全不变。
#[tokio::test]
#[ignore]
async fn apply_all_invalid_no_empty_write() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = insert_and_load(&app, seed_contact(&ws, &acc, "wx_allbad"), "wx_allbad").await;
    let before_updated = contact.updated_at;

    let changes = doc! {
        "operationState": "active",
        "customerStage": "瞎填阶段",
        "intentLevel": "瞎填意向",
    };
    let skipped = apply_contact_changes(&app.state, &contact, &changes)
        .await
        .expect("apply 不应整体失败");

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_allbad" }, None)
        .await
        .expect("query")
        .expect("exists");
    // set_doc 空判:无合法字段 → 不写库 → updated_at 不变。
    assert_eq!(
        after.updated_at, before_updated,
        "全越界应触发 set_doc 空判,不产生只刷 updated_at 的空写"
    );
    assert_eq!(skipped.len(), 3, "三个越界字段全部记入 skipped");
}

/// customerStage 越界 + intentLevel 合法 → intent 落库,stage 进 skipped。
#[tokio::test]
#[ignore]
async fn apply_intent_valid_stage_skipped() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = insert_and_load(&app, seed_contact(&ws, &acc, "wx_mix"), "wx_mix").await;

    // customerStage 越界(进 skipped),intentLevel="high" 是合法 canonical。
    let changes = doc! {
        "customerStage": "瞎填阶段",
        "intentLevel": "high",
    };
    let skipped = apply_contact_changes(&app.state, &contact, &changes)
        .await
        .expect("apply 不应整体失败");

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_mix" }, None)
        .await
        .expect("query")
        .expect("exists");
    // intent_level 存在 domain_attributes.intent_level。
    let intent = after
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str("intent_level").ok());
    assert_eq!(intent, Some("high"), "合法 intentLevel 必须落库");
    let fields: Vec<&str> = skipped.iter().map(|s| s.field.as_str()).collect();
    assert_eq!(fields, vec!["customerStage"], "仅 customerStage 被跳过");
}

/// 正向回归:三字段全合法 → 全部落库,skipped 为空。
#[tokio::test]
#[ignore]
async fn apply_legal_values_all_persist() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let contact = insert_and_load(&app, seed_contact(&ws, &acc, "wx_ok"), "wx_ok").await;

    // operationState="cooldown" allowFromAny:true → 从任意态合法;stage/intent 用 canonical。
    let changes = doc! {
        "customerStage": "need_discovery",
        "intentLevel": "high",
        "operationState": "cooldown",
    };
    let skipped = apply_contact_changes(&app.state, &contact, &changes)
        .await
        .expect("apply 成功");

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "wxid": "wx_ok" }, None)
        .await
        .expect("query")
        .expect("exists");
    assert_eq!(after.operation_state.as_deref(), Some("cooldown"), "合法迁移落库");
    let stage = after.domain_attributes.as_ref().and_then(|d| d.get_str("customer_stage").ok());
    let intent = after.domain_attributes.as_ref().and_then(|d| d.get_str("intent_level").ok());
    assert_eq!(stage, Some("need_discovery"), "合法 stage 落库");
    assert_eq!(intent, Some("high"), "合法 intent 落库");
    assert!(skipped.is_empty(), "全合法 → skipped 空(证明不影响 happy path)");
}
```

- [ ] **Step 2: 编译验证(不需 Docker)**

Run: `cargo test --test guide_apply_partial_validation --no-run`
Expected: 编译通过,0 error。这一步只编译不跑(`#[ignore]` 测试需 Docker)。
若报 `Contact` 字段不匹配(models.rs 改过字段)→ 以 `tests/contact_manual_tags_integration.rs:34-81` 的 `managed_contact` 为准对齐字段(本计划 seed_contact 即从它复制 + 改 operation_state)。

- [ ] **Step 3: (有 Docker 时)跑测试**

Run: `cargo test --test guide_apply_partial_validation -- --ignored`
Expected: 4 passed。
**本机无 Docker**:本地仅 `--no-run` 编译验证;实跑留 GitHub CI integration job(CLAUDE.md 已述本地磁盘/Docker 限制)。在 PR 描述里标注"集成测试待 CI 验证"。

- [ ] **Step 4: 提交**

```bash
git add tests/guide_apply_partial_validation.rs
git commit -m "test(guide): apply 部分应用集成测试(越界跳过/空写/混合/正向回归)

4 个 #[ignore] 集成测试直调 apply_contact_changes:越界字段跳过+合法落库、
全越界触发 set_doc 空判不空写、stage 越界 intent 合法、三字段全合法正向回归。
TestApp 已 re-seed m006 字典+预热缓存,字典/状态机校验真实生效。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: build_guide_preview_prompt 注入合法值 纯函数单测

**Files:**
- Modify: `src/routes/shared.rs`(`#[cfg(test)] mod tests` 块,:1455 起)

**Interfaces:**
- Consumes: `super::build_guide_preview_prompt`(同模块 `pub(super)`,可直接调);`crate::models::{Contact, OperatingMemory, AgentStatus}`。

**背景**:纯函数单测进 lib(`cargo test --lib`,无需 Docker),验证 Task 3 的注入文本:给合法值切片 → 输出含 canonical key + 中文标签 + "合法值"字样;给空切片 → 含"暂无受控取值"。`Contact` 无 `Default`(只 Debug/Clone/Serde),需全字段构造;`OperatingMemory` 有 `Default`,直接 `::default()`。

- [ ] **Step 1: 在 mod tests 内写测试 + 最小 Contact 构造 helper**

在 `src/routes/shared.rs` 的 `#[cfg(test)] mod tests {`(:1456)块内追加(append,不动现有测试):

```rust
    /// Task 5:build_guide_preview_prompt 注入合法值文本契约。
    #[test]
    fn guide_prompt_injects_legal_values() {
        use super::build_guide_preview_prompt;
        use crate::models::{AgentStatus, Contact, OperatingMemory};
        use mongodb::bson::{DateTime, Document};

        // build_guide_preview_prompt 只读 Contact 的少数字段;Contact 无 Default,全字段构造。
        let contact = Contact {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            wxid: "wx_prompt".to_string(),
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
            operation_state: Some("new_contact".to_string()),
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: Document::new(),
            profile_attributes: Document::new(),
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
        };
        let memory = OperatingMemory::default();
        let health = serde_json::json!({});

        // 有合法值:输出含状态机 key + 字典中文标签 + "合法值"字样。
        let legal_states = vec!["new_contact".to_string(), "need_discovery".to_string()];
        let stage_values = vec![
            ("new_contact".to_string(), "初始了解".to_string()),
            ("need_discovery".to_string(), "需求探索".to_string()),
        ];
        let intent_values = vec![
            ("high".to_string(), "高意向".to_string()),
            ("low".to_string(), "低意向".to_string()),
        ];
        let prompt = build_guide_preview_prompt(
            "标记成高意向",
            "smart",
            &contact,
            &memory,
            None,
            None,
            &health,
            &legal_states,
            &stage_values,
            &intent_values,
        );
        assert!(prompt.contains("合法值"), "应注入'合法值'引导段");
        assert!(prompt.contains("need_discovery"), "应含状态机/字典 canonical key");
        assert!(prompt.contains("高意向"), "应含字典中文标签");

        // 空切片:输出"暂无受控取值"兜底,不 panic。
        let empty: Vec<String> = vec![];
        let empty_pairs: Vec<(String, String)> = vec![];
        let prompt_empty = build_guide_preview_prompt(
            "标记成高意向",
            "smart",
            &contact,
            &memory,
            None,
            None,
            &health,
            &empty,
            &empty_pairs,
            &empty_pairs,
        );
        assert!(
            prompt_empty.contains("暂无受控取值"),
            "空字典应输出'暂无受控取值'兜底"
        );
    }
```

- [ ] **Step 2: 跑测试验证通过**

Run: `cargo test --lib guide_prompt_injects_legal_values`
Expected: 1 passed。若 FAIL 在 `contains("合法值")` → 回到 Task 3 Step 2 确认注入文本里有"合法值"字样;若编译报 Contact 字段不匹配 → 以 models.rs 当前 Contact 定义对齐(本计划字段来自 Task 4 seed_contact,同源)。

- [ ] **Step 3: 提交**

```bash
git add src/routes/shared.rs
git commit -m "test(guide): build_guide_preview_prompt 注入合法值纯函数单测

验证有合法值切片→输出含 canonical key+中文标签+'合法值'引导;空切片→
'暂无受控取值'兜底不 panic。进 lib 测试无需 Docker。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 前端 skipped 回流提示(类型 + store + ToastProvider)

**Files:**
- Modify: `frontend/src/types/index.ts:473`(UserOperationGuidePreview 之后新增两类型)
- Modify: `frontend/src/stores/userOpsStore.ts`(import + interface :122 + action :709-737)
- Modify: `frontend/src/features/user-ops/index.tsx`(挂 ToastProvider + 包 apply 回调拼 toast)

**Interfaces:**
- Consumes: 后端 apply 响应 `item.{contact,operatingMemory,health,appliedFields,skippedFields}`(Task 2);`ToastProvider`/`useToast`(`frontend/src/components/ui/Toast`,已存在,API:`success/error/info: (msg:string)=>void`,**必须在 `<ToastProvider>` 内**否则抛错)。
- Produces: store `applyGuidePreview` 返回 `Promise<UserOperationGuideApplyResult | null>`。

**背景**:apply 响应当前 store 用内联泛型、无命名类型,且 user-ops 频道根组件只挂了 `ConfirmProvider`(index.tsx:53),没挂 ToastProvider → 直接在子树调 `useToast` 会抛错;store 在 React 之外也不能调 `useToast`。本 task:加命名类型 → store 返回结果(不在 store 调 toast)→ index.tsx 挂 ToastProvider 并在 apply 回调里(组件层)拼 toast。

- [ ] **Step 1: 新增前端类型**

在 `frontend/src/types/index.ts:473`(`UserOperationGuidePreview` 类型 `};` 之后)追加:

```ts
export type GuideSkippedField = { field: string; reason: string };

export type UserOperationGuideApplyResult = {
  contact: Contact;
  operatingMemory: OperatingMemory;
  health: OperationHealth;
  appliedFields: string[];
  skippedFields: GuideSkippedField[];
};
```

(`Contact` :85 / `OperatingMemory` :385 / `OperationHealth` :446 均已在同文件定义,直接引用。)

- [ ] **Step 2: store 引入类型 + 改 interface 签名**

在 `frontend/src/stores/userOpsStore.ts:12`(import 块内 `UserOperationGuidePreview,` 那行)后补一行:

```ts
  UserOperationGuideApplyResult,
```

把 interface 里 `:122` 的:

```ts
  applyGuidePreview: () => Promise<void>;
```

改为:

```ts
  applyGuidePreview: () => Promise<UserOperationGuideApplyResult | null>;
```

- [ ] **Step 3: 改 applyGuidePreview action(:709-737)用命名类型 + 返回结果**

把 `frontend/src/stores/userOpsStore.ts:709-737` 整个 action 替换为:

```ts
  applyGuidePreview: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { guidePreview } = get();

    if (!selected || !guidePreview) return null;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: UserOperationGuideApplyResult }>(
        "/api/user-operations/guide/apply",
        { previewId: guidePreview.id }
      );

      set({
        operatingMemory: data.item.operatingMemory,
        guidePreview: null,
        operationHealth: data.item.health
      });

      await refreshContacts(currentAccountId);
      return data.item;
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      return null;
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },
```

(关键改动:内联泛型 → `UserOperationGuideApplyResult`;三处早退/失败 return `null`;成功 `return data.item`。`refreshContacts` 是该文件内既有 helper,沿用。)

- [ ] **Step 4: index.tsx 挂 ToastProvider**

在 `frontend/src/features/user-ops/index.tsx:18`(`import { ConfirmProvider } ...` 那行)后补一行:

```tsx
import { ToastProvider } from "../../components/ui/Toast";
```

把 `:52-56` 的:

```tsx
  return (
    <ConfirmProvider>
      <UserOpsFeatureInner />
    </ConfirmProvider>
  );
```

改为(ToastProvider 包在外,使 Inner 子树可用 useToast):

```tsx
  return (
    <ToastProvider>
      <ConfirmProvider>
        <UserOpsFeatureInner />
      </ConfirmProvider>
    </ToastProvider>
  );
```

- [ ] **Step 5: index.tsx 在 Inner 里包 apply 回调拼 toast**

在 `UserOpsFeatureInner` 顶部(`:186` `const runSavePrompt = usePromptSaveConfirm();` 之后)新增 useToast + 包装回调:

```tsx
  const toast = useToast();
  // apply 成功后:有跳过字段就提示哪些被跳过(文案动态拼接,不硬编码字段名)。
  const onApplyGuide = async () => {
    const res = await applyGuidePreview();
    if (!res) return;
    if (res.skippedFields.length) {
      toast.info(
        `已应用 ${res.appliedFields.length} 项,跳过 ${res.skippedFields
          .map((s) => s.field)
          .join("、")}(取值越界,已忽略)`
      );
    } else {
      toast.success(`已应用 ${res.appliedFields.length} 项配置`);
    }
  };
```

在 `:186` 行附近补 import(顶部 import 块,与 ConfirmProvider 那段并列):已在 Step 4 加 `ToastProvider`;此处还需 `useToast`,把 Step 4 的 import 改为:

```tsx
import { ToastProvider, useToast } from "../../components/ui/Toast";
```

然后把 Cockpit 的 `onApplyGuidePreview={applyGuidePreview}`(:294)改为:

```tsx
            onApplyGuidePreview={onApplyGuide}
```

(Cockpit prop 类型 `onApplyGuidePreview: () => void`(legacy.tsx:256)兼容 `() => Promise<void>` 的 onApplyGuide —— async 函数赋给 `() => void` 在 TS 里合法,返回值被忽略。按钮 `onClick={onApplyGuidePreview}`(legacy.tsx:482)不动。)

- [ ] **Step 6: 前端类型检查 + 构建**

Run: `cd frontend && npm run build`
Expected: tsc 0 error,vite build 成功。重点验证:`UserOperationGuideApplyResult` 类型贯通;`onApplyGuide` 赋给 `() => void` 无类型错;ToastProvider import 路径正确。
若报 `useToast must be inside ToastProvider` —— 这是运行时错误不是编译错;确认 Step 4 的 ToastProvider 包在 Inner 外层即可。

- [ ] **Step 7: (无法本地起浏览器时)说明**

本环境若不能起 dev server 实测交互:在 PR 描述注明"前端改动已过 tsc+build,toast 交互待人工/CI 验证",不谎报已实测 UI。

- [ ] **Step 8: 提交**

```bash
git add frontend/src/types/index.ts frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/index.tsx
git commit -m "feat(guide-fe): apply 后 toast 提示被跳过的越界字段

新增 UserOperationGuideApplyResult/GuideSkippedField 类型;applyGuidePreview
返回结果而非 void;user-ops 频道挂 ToastProvider,组件层包 apply 回调按
skippedFields 动态拼文案(不硬编码字段名)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: biz-test 断言翻转 + 全量基线复核

**Files:**
- Modify: `scripts/biz-test/batch_c_guide.py:121-160`(apply 断言:越界不再 400,改验 skippedFields 回流)

**Interfaces:**
- Consumes: apply 响应新 `item.skippedFields` / `item.appliedFields`(Task 2)。

**背景**:`batch_c_guide.py` 当前把 apply 越界 400 当"红线生效 + medium 发现"(:123-138)。本次修复后,apply 越界**不再 400**,而是 200 + `skippedFields` 回流。Task 7 把这段断言翻转:越界字段进 skippedFields(部分应用),合法字段落库;保留 not-pending 幂等(:13)、preview 不碰业务库(:91)两条铁证不动。

**运行约束(CLAUDE.md / memory)**:biz-test 从本机跑,目标 server 117(须先部署含本次改动的镜像才能实测);LLM 端点 rsxermu claude-opus-4.8 仅 2 并发线程 → 脚本绝不并行;真调 LLM 用 `_lib.api_bg` 后台轮询;撞外部 MCP 502 标 BLOCKED 非项目 bug。**本 task 只改脚本断言,实跑等部署后**(与 design §8.3 一致)。

- [ ] **Step 1: 翻转 apply 越界断言**

把 `scripts/biz-test/batch_c_guide.py:121-138`(`aerr = _lib.is_api_error(apply_resp)` 到 `raise SystemExit(f"apply 失败: {aerr}")` 这段对 `state_machine_rejected` 的处理)替换为:

```python
    aerr = _lib.is_api_error(apply_resp)
    if aerr:
        # 修复后:apply 不应再因 LLM 越界 operationState 整体 400(部分应用)。
        # 若仍 400 且是状态机/字典相关 → 修复回归,记 critical。其余 api_error → BLOCKED(端点/MCP)。
        if "operation_state" in str(apply_resp) or "状态机" in str(apply_resp) or "dimension" in str(apply_resp):
            _lib.record(DOMAIN, "apply 仍因枚举越界整体失败(部分应用修复回归)",
                        f"resp={str(apply_resp)[:200]}", "critical",
                        "修复目标=越界字段跳过+合法字段落库;若 apply 仍 400 说明 shared.rs "
                        "apply_contact_changes 的 skip 改动未生效或被回退")
            raise SystemExit(f"apply 部分应用回归: {aerr}")
        _lib.record(DOMAIN, "apply 端点失败(BLOCKED)", f"resp={str(apply_resp)[:200]}", "high",
                    "非枚举越界类错误,疑端点/MCP,标 BLOCKED 等恢复复跑")
        raise SystemExit(f"apply 失败: {aerr}")

    # apply 成功(200):验 skippedFields 回流 + 合法字段落库。
    item = apply_resp.get("item", {}) if isinstance(apply_resp, dict) else {}
    skipped = item.get("skippedFields", [])
    applied = item.get("appliedFields", [])
    print(f"  appliedFields={applied} skippedFields={skipped}")
    # 若 LLM 这轮产了越界 operationState,它必须出现在 skippedFields(被跳过)而非致全局失败。
    sc_keys = set(sc0.get("keys", []))
    if "operationState" in sc_keys:
        skipped_names = {s.get("field") for s in skipped if isinstance(s, dict)}
        # operationState 要么被 skip(越界),要么被 apply(LLM 这次产了合法态)——两者都不该整体 400。
        _lib.expect("operationState" in skipped_names or "operationState" in applied,
                    DOMAIN, "operationState 要么跳过要么应用,不再连坐合法字段",
                    f"applied={applied} skipped={skipped}", "high",
                    "部分应用红线:单个越界字段不得致全部合法字段丢弃")
```

(下方 `:140` 起的 `apply 成功路径:铁证 3+4`(preview.status=applied + updated_at 刷新)保持不变 —— 它们在 `if aerr: ... raise` 之后,现在 apply 成功路径恒到达。)

- [ ] **Step 2: 校验脚本语法(本地,不实跑)**

Run: `python -c "import ast; ast.parse(open('scripts/biz-test/batch_c_guide.py', encoding='utf-8').read()); print('OK')"`
Expected: `OK`。(本机用 `python` 即 C:\Python314,非 python3。)
实跑(`python scripts/biz-test/batch_c_guide.py`)等 server 117 部署含本次改动的镜像后,串行单跑。

- [ ] **Step 3: 全量基线复核(后端)**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed(新增 guide_prompt_injects_legal_values,只增不减)。

Run: `cargo check --tests`
Expected: 0 error(复刻 CI step2,确保新集成测试 + 全部 tests/ 编译过)。

4 PBT 不在本次改动面,无需单跑;若 CI 红再查。

- [ ] **Step 4: 提交**

```bash
git add scripts/biz-test/batch_c_guide.py
git commit -m "test(biz): guide apply 断言翻转——越界字段验 skippedFields 回流

修复后 apply 越界 operationState 不再整体 400,改验 200+skippedFields 部分应用
(越界进 skipped/合法字段落库);仍 400 且状态机相关→记 critical 回归。
保留 not-pending 幂等+preview 不碰业务库两条铁证。实跑等 117 部署。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review(写完计划后自查)

**1. Spec coverage(逐节对照 design 文档)**:
- §四改动1(apply 三字段跳过)→ Task 1 ✅(含 SkippedField/签名/三校验点/insert_domain_stage_fields 不变量经三分支门控天然守住)
- §五改动2(skipped 回流)→ Task 2 ✅(响应 `{item}` 包裹已修正,审计 details 补 skippedFields)
- §六改动3(prompt 注入)→ Task 3 ✅(operation_states 导出 + handler 查值 + 纯函数加 3 入参 + 注入文本)
- §七改动4(前端)→ Task 6 ✅(类型/store/ToastProvider)
- §八测试 → Task 4(4 集成)+ Task 5(1 prompt 单测)+ Task 7(biz-test 断言)+ 基线 ✅
- §九边界(domain_config=None fail-open / 字典冷启动 / relationship_type 不写)→ Task 1 Step 5 注释 + Task 3 兜底文本覆盖 ✅
- §十非目标(不改 helper / 手动表单 / 审批)→ Global Constraints 显式列明 ✅

**2. Placeholder scan**:无 TBD/TODO;每个改代码的 step 都有完整 before/after 代码块与确切命令 + 预期输出。✅

**3. Type consistency**:
- `SkippedField{field,reason}` 在 Task 1 定义,Task 2/4 一致引用 ✅
- `apply_contact_changes -> AppResult<Vec<SkippedField>>` Task 1 产出,Task 2/4 消费签名一致 ✅
- `build_guide_preview_prompt` 末尾加 `legal_states/&[String]` + `stage_values/intent_values/&[(String,String)]`,Task 3 定义、Task 5 调用参数顺序一致 ✅
- 前端 `UserOperationGuideApplyResult{contact,operatingMemory,health,appliedFields,skippedFields}` Task 6 Step1 定义,store(Step3)+ index.tsx(Step5)消费字段名一致 ✅
- canonical 值:intent_level 用 `high`/`low`(非 "mid"),状态机 `cooldown`(allowFromAny)/`need_discovery` —— 与 m006/prompts.rs 实测一致 ✅

**4. 关键风险已在计划内显式处理**:
- `apply_contact_changes` 原 `pub(super)` 对 tests/ 不可见 → Task 1 Step 6 加 `pub use` 暴露(否则 Task 4 编译失败)
- apply 响应 `{item:{...}}` 包裹 → Task 2 Step 3 新键加在 item 内,Task 6 store 读 `data.item.*`
- `insert_domain_stage_fields` 无条件写时间戳的空写风险 → Task 1 靠"stage/intent 都 None 不调用"门控守住,Task 4 `apply_all_invalid_no_empty_write` 钉死
- 本机无 Docker/不能起浏览器 → Task 4 Step 3 / Task 6 Step 7 明确"留 CI、不谎报实测"

无遗漏,计划与 spec 一致。

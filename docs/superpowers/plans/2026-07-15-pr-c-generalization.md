# PR-C: 通用化底座两处半接线修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 C-01（stagnation 计时写侧写死 customer_stage）与 C-02（初始画像半接线，非销售域维度不采集）两处「通用化只做一半」缺陷。

**Architecture:** C-01 让 stagnation 计时写侧按 active profile 的 `stagnation_dimension` 检测该维度自身变化并写 `{dim}_updated_at`；C-02 让初始画像生成路径比照 live reply 追加两个维度指引。两者互不牵连。

**Tech Stack:** Rust 2021 / Axum / MongoDB。测试用 `cargo test --lib`（纯函数）。

## Global Constraints

- **DEFAULT 销售域字节等价**：`stagnation_dimension` DEFAULT="customer_stage"（domain_profile.rs:791/845）；两 render guidance 函数 DEFAULT 返空串。销售域行为字节不变。
- **反过拟合红线**：不对单条对话点修；判据抽纯函数 + 常量。
- **三线隔离铁律**：不碰 manual_tags / bayesian_signals。
- **lint 门**：新增行不得含 `check-no-human-takeover` / `check-no-model-hint` 禁词。
- **本地验证**（PR#217 教训）：提交前 `cargo test --lib` + `RUSTFLAGS="-D warnings" cargo check --tests`。
- 分支 `fix/audit-medium-batch1`（承接 PR-A 之后，或 PR-A 合并后新起分支——见落地顺序）。

---

### Task 1: C-01 —— stagnation 写侧按维度动态化

**Files:**
- Modify: `src/agent/domain_signals.rs`（`insert_domain_signal_values` :128 加参 + :148-149 按 dim 写；新增纯函数 `dimension_value_changed`）
- Modify: `src/agent/gateway.rs`（:4128-4133 用 stagnation 维度算变化 + 传 dim）
- Modify: `src/routes/shared.rs`（:106 wrapper 传 None）
- Test: `src/agent/domain_signals.rs`（`#[cfg(test)]` 内联）

**Interfaces:**
- Consumes: `contact.domain_attributes: Document`（gateway 可读，:4029-4030 同款 `get_str` 读法）；`active_profile.stagnation_dimension: Option<String>`（models.rs:2048）。
- Produces: `insert_domain_signal_values(set_doc, signals, stage_changed, stagnation_dimension: Option<&str>) -> bool`（加第 4 参）；`dimension_value_changed(prev: Option<&str>, new: Option<&str>) -> bool`。

- [ ] **Step 1: 写失败测试**

在 `src/agent/domain_signals.rs` 的 `#[cfg(test)] mod tests` 内加（若无测试模块则在文件末尾新建 `#[cfg(test)] mod stagnation_dim_tests { use super::*; use mongodb::bson::{doc, Document}; ... }`）：

```rust
#[test]
fn insert_writes_custom_stagnation_dim_timestamp() {
    let mut set_doc = Document::new();
    let mut signals = Document::new();
    signals.insert("relationship_closeness", "亲密");
    // stagnation_dimension=relationship_closeness + 该维度本轮有值 + changed → 写它的 _updated_at
    insert_domain_signal_values(&mut set_doc, &signals, true, Some("relationship_closeness"));
    assert!(set_doc.contains_key("domain_attributes.relationship_closeness_updated_at"));
    assert!(!set_doc.contains_key("domain_attributes.customer_stage_updated_at"));
}

#[test]
fn insert_default_dim_is_customer_stage_byte_equivalent() {
    // DEFAULT 等价守护：None 与 Some("customer_stage") 都写 customer_stage_updated_at。
    for dim in [None, Some("customer_stage")] {
        let mut set_doc = Document::new();
        let mut signals = Document::new();
        signals.insert("customer_stage", "已建联");
        insert_domain_signal_values(&mut set_doc, &signals, true, dim);
        assert!(set_doc.contains_key("domain_attributes.customer_stage_updated_at"));
    }
}

#[test]
fn insert_no_timestamp_when_dim_absent_from_signals() {
    // 纵深守卫：stagnation 维度本轮不在 signals 里 → 不刷时间戳（避免错误重置计时）。
    let mut set_doc = Document::new();
    let mut signals = Document::new();
    signals.insert("customer_stage", "已建联"); // 只有 stage，无 relationship_closeness
    insert_domain_signal_values(&mut set_doc, &signals, true, Some("relationship_closeness"));
    assert!(!set_doc.contains_key("domain_attributes.relationship_closeness_updated_at"));
}

#[test]
fn dimension_value_changed_detects_change() {
    assert!(dimension_value_changed(Some("a"), Some("b")));
    assert!(dimension_value_changed(None, Some("b")));
    assert!(!dimension_value_changed(Some("a"), Some("a")));
    assert!(!dimension_value_changed(Some("a"), None)); // 新值缺失不算变化(不刷)
    assert!(!dimension_value_changed(None, None));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib stagnation_dim dimension_value_changed insert_writes insert_default insert_no_timestamp 2>&1 | tail -20`
Expected: 编译失败（参数个数不符 / `dimension_value_changed` 未定义）。

- [ ] **Step 3: 改内核签名 + 加纯函数**

`src/agent/domain_signals.rs` :128-152，签名加第 4 参，:148-149 按 dim 写：
```rust
pub(crate) fn insert_domain_signal_values(
    set_doc: &mut Document,
    signals: &Document,
    stage_changed: bool,
    stagnation_dimension: Option<&str>,
) -> bool {
    let mut wrote_any = false;
    for (key, value) in signals {
        if let Some(text) = value.as_str() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            set_doc.insert(format!("domain_attributes.{key}"), trimmed);
            wrote_any = true;
        }
    }
    // C-01：停滞计时维度可配。读侧 planner 按 {dim}_updated_at 计时（该维度多久没变）。
    // 纵深守卫：仅当 signals 里确实有该维度键（本轮确有值）且 stage_changed(=该维度变化)
    // 才刷时间戳，否则会写"刚变更"时间戳但维度没写→错误重置下游 stagnation 计时。
    // DEFAULT dim=customer_stage → 与改造前字节等价。
    let dim = stagnation_dimension.unwrap_or("customer_stage");
    if stage_changed && signals.get_str(dim).is_ok() {
        set_doc.insert(format!("domain_attributes.{dim}_updated_at"), DateTime::now());
    }
    wrote_any
}

/// C-01：某维度值本轮是否变化（供 gateway 决定是否刷 stagnation 计时戳）。
/// 新值缺失（本轮未产出该维度）不算变化——不刷时间戳，保持旧计时。纯函数便于单测。
pub(crate) fn dimension_value_changed(prev: Option<&str>, new: Option<&str>) -> bool {
    new.is_some() && prev != new
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib stagnation_dim dimension_value_changed 2>&1 | tail -20`
Expected: 4 个测试 PASS。

- [ ] **Step 5: gateway 调用点按 stagnation 维度算变化**

`src/agent/gateway.rs` :4128-4134 当前：
```rust
        let new_stage = signals_for_attrs.get_str("customer_stage").ok();
        let stage_changed = new_stage.is_some() && prev_stage != new_stage;
        let wrote = crate::agent::domain_signals::insert_domain_signal_values(
            &mut set_doc,
            &signals_for_attrs,
            stage_changed,
        );
```
改为（用 active_profile.stagnation_dimension 算该维度变化；active_profile 已在本函数上文 :3923 载入）：
```rust
        // C-01：按 active profile 的 stagnation_dimension 计算「该维度是否变化」，而非
        // 写死 customer_stage。读侧 planner 按 {dim}_updated_at 计时，写侧须在该维度自身
        // 变化时刷其时间戳。DEFAULT dim=customer_stage → stagnation_changed==原 stage_changed。
        let stagnation_dim = active_profile.stagnation_dimension.as_str();
        let prev_dim = contact.domain_attributes.get_str(stagnation_dim).ok();
        let new_dim = signals_for_attrs.get_str(stagnation_dim).ok();
        let stagnation_changed =
            crate::agent::domain_signals::dimension_value_changed(prev_dim, new_dim);
        let wrote = crate::agent::domain_signals::insert_domain_signal_values(
            &mut set_doc,
            &signals_for_attrs,
            stagnation_changed,
            Some(stagnation_dim),
        );
```
**注意**：`active_profile.stagnation_dimension` 是 `Option<String>`（models.rs:2048），需先取 as_str。核实 :3923 处 `active_profile` 变量名与 `stagnation_dimension` 字段——实现时 grep `active_profile` 在 gateway 该函数内的真实绑定名，若非直接可得则从 profile 取。DEFAULT profile `stagnation_dimension=Some("customer_stage")`；若为 None 则 `.as_deref().unwrap_or("customer_stage")`。

修正上句为稳健写法：
```rust
        let stagnation_dim = active_profile
            .stagnation_dimension
            .as_deref()
            .unwrap_or("customer_stage");
```

- [ ] **Step 6: wrapper 传 None**

`src/routes/shared.rs` :106 当前：
```rust
    crate::agent::domain_signals::insert_domain_signal_values(set_doc, &signals, stage_changed);
```
改为（admin 直写路径不载 active_profile，保持按 customer_stage 语义）：
```rust
    // C-01：admin 直写路径不驱动 stagnation 计时的主逻辑，传 None 保持 customer_stage 语义
    // （字节等价于改造前）。AI 决策路径（gateway）才按 active profile 的 stagnation_dimension。
    crate::agent::domain_signals::insert_domain_signal_values(set_doc, &signals, stage_changed, None);
```

- [ ] **Step 7: 编译 + 全量 lib 回归**

Run: `cargo build --lib 2>&1 | tail -15 && cargo test --lib 2>&1 | tail -6`
Expected: 编译通过；lib 0 failed（新增 4 测试）。若 gateway 有其它 `insert_domain_signal_values` 调用点，编译器会报参数不符——grep `insert_domain_signal_values` 全仓补齐所有调用点（预期只有 gateway:4130 + shared:106 两处，Step 5/6 已覆盖）。

- [ ] **Step 8: 提交**

```bash
git add src/agent/domain_signals.rs src/agent/gateway.rs src/routes/shared.rs
git commit -m "fix(generalization): C-01 stagnation 计时写侧按 stagnation_dimension 动态化

读侧 planner 按 {dim}_updated_at 计时,写侧此前写死 customer_stage_updated_at。
改为 gateway 按 active profile 维度检测该维度自身变化并写对应时间戳;
admin wrapper 传 None 保持现状。DEFAULT customer_stage 字节等价。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: C-02 —— 初始画像接两个维度指引

**Files:**
- Modify: `src/agent/decision.rs`（`build_initial_operation_profile` :48-53 加 account_id 参 + :90 前追加两 guidance 到 task_template）
- Modify: `src/routes/contacts.rs`（4 调用点 :739/:1010/:1173/:1634 传 account_id）
- Modify: `src/routes/management.rs`（1 调用点 :1401 传 account_id）

**Interfaces:**
- Consumes: `render_memory_candidate_types_guidance(&[MemoryDimension]) -> String`（domain_profile.rs:134，DEFAULT 空串）；`render_decision_dimensions_guidance(&[dimensions], &account_id, taxonomy_cache) -> String`（domain_profile.rs:1182）；`global_taxonomy_cache()` + `find_or_load(&db)`（reply 路径范式 decision.rs:676-687）；`active_profile.memory_dimensions` / `.profile_dimensions`。
- Produces: `build_initial_operation_profile(state, workspace_id, account_id: &str, note, playbook)`（加第 3 参 account_id）。

- [ ] **Step 1: 改签名 + 注入两 guidance**

`src/agent/decision.rs` :48-53 签名加 `account_id: &str`（放 workspace_id 后）：
```rust
pub async fn build_initial_operation_profile(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    note: &str,
    playbook: Option<&OperationPlaybook>,
) -> AppResult<GeneratedOperationProfile> {
```

在 task_template 载入后（:89 之后、:90 user format! 之前）追加两 guidance（比照 reply 路径 decision.rs:654-687）：
```rust
    // C-02：比照 live reply 路径（decision.rs 内 decide_reply 的 memory/decision 维度指引），
    // 让初始画像建档时也告知 LLM 本行业记忆槽位 + 参与决策的 typed 维度。DEFAULT 销售域
    // 两函数均返空串 → prompt 字节等价（反过拟合护栏）。
    let taxonomy_cache = crate::agent::taxonomy::global_taxonomy_cache();
    taxonomy_cache.find_or_load(&state.db).await;
    let task_template = format!(
        "{task_template}{}{}",
        super::domain_profile::render_memory_candidate_types_guidance(
            &active_profile.memory_dimensions,
        ),
        super::domain_profile::render_decision_dimensions_guidance(
            &active_profile.profile_dimensions,
            account_id,
            taxonomy_cache.as_ref(),
        )
    );
```
**注意**：`active_profile` 已在 :70-74 载入。核实 `active_profile.memory_dimensions` / `.profile_dimensions` 字段名（DomainProfile，models.rs）——实现时 grep 确认，与 reply 路径用的字段名一致（reply 路径 :655 用 memory_dimensions、:682 用 profile_dimensions）。

- [ ] **Step 2: 编译（预期调用点报错）**

Run: `cargo build --lib 2>&1 | tail -20`
Expected: 编译失败——5 个调用点参数不符（contacts.rs ×4、management.rs ×1）。这是预期的，Step 3 修。

- [ ] **Step 3: 补齐 5 个调用点传 account_id**

逐个改（每处在 workspace_id 后加对应 account_id 实参）：
- `src/routes/contacts.rs:739`：该处在 worker 异步生成路径，account_id 来源需 grep 上下文（通常 `&task.account_id` 或 contact.account_id）。
- `src/routes/contacts.rs:1010` / `:1173` / `:1634`：各自上下文的 account_id。
- `src/routes/management.rs:1401`：`workspace_id` 后传该上下文 account_id。

**实现要求**：每个调用点先 Read 其上下文 ±15 行，确认 account_id 的真实来源变量（不猜——可能是 task.account_id / contact.account_id / query.account_id / default_account_id）。若某调用点确无 account_id 语义，用 `state.config.default_account_id` 兜底（与该处业务语义一致时）。

- [ ] **Step 4: 编译通过 + 全量 lib**

Run: `cargo build --lib 2>&1 | tail -10 && cargo test --lib 2>&1 | tail -6`
Expected: 编译通过；lib 0 failed。

- [ ] **Step 5: 提交**

```bash
git add src/agent/decision.rs src/routes/contacts.rs src/routes/management.rs
git commit -m "fix(generalization): C-02 初始画像接记忆+决策维度指引

build_initial_operation_profile 比照 live reply 追加 render_memory_candidate_types_guidance
+ render_decision_dimensions_guidance(签名加 account_id,taxonomy_cache 走全局单例);
非销售域建档时采集本行业维度。DEFAULT 销售域两函数返空串,prompt 字节等价。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 本地门 + 推送 + PR + 合并

- [ ] **Step 1: `-D warnings` check --tests**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -20`
Expected: 无 error。超时则后台跑等通知。

- [ ] **Step 2: 全量 lib**

Run: `cargo test --lib 2>&1 | tail -6`
Expected: 0 failed。

- [ ] **Step 3: 推送 + 亲验**

```bash
git push origin HEAD:refs/heads/<pr-c-branch>
git ls-remote origin refs/heads/<pr-c-branch>  # == 本地 HEAD
```
（分支名见落地顺序：若 PR-C 与 PR-A 同分支则续推；若 PR-A 已合并则新起 `fix/audit-medium-c` 基于最新 origin/main。）

- [ ] **Step 4: 建 PR + 监控 CI + squash merge（不带 --delete-branch）**

CI 全绿（Baseline+Integration+三 lint）后合，`git fetch && git rev-parse origin/main` 核 mergeCommit 进 main。

## Self-Review

- **Spec 覆盖**：C-01（Task 1）/ C-02（Task 2）全覆盖，与设计文档 PR-C 段（含深度校准）一致。
- **占位符**：Task 3 Step 3 分支名 + Task 2 Step 3 各调用点 account_id 来源标为「实现时 grep 确认」——这是**有意的亲验要求**（红线：不猜 account_id 来源），非占位。其余步骤含真实代码。
- **类型一致**：`insert_domain_signal_values` 第 4 参 `Option<&str>` 在 Task 1 定义并在 gateway/wrapper 消费；`dimension_value_changed` 纯函数签名一致；`build_initial_operation_profile` 加 `account_id: &str` 在 Task 2 定义并在 5 调用点消费。
- **字节等价**：C-01 DEFAULT dim=customer_stage、C-02 两 guidance DEFAULT 空串——均在 Global Constraints 钉住。

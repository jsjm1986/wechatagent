# 通用化后端三残留收口 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把后端通用化的三个残留（H13 状态机本体随 profile / H17 intent_trajectory 轨迹维度容器化 / H18 debounce 随 profile）全部做到位，DEFAULT 销售域行为字节等价。

**Architecture:** 三残留相互独立，按风险递增排序：H18（debounce 下沉 profile 字段）→ H17（轨迹维度容器化，仿 MemoryDimension）→ H13（状态机本体随 profile，引导层联动生成→draft→activate 时 publish 到 operation_domain_configs，消费方零改动、不造双真相源）。

**Tech Stack:** Rust 2021 / Axum / MongoDB(BSON) / serde；测试 = `cargo test --lib`（纯函数/单测）+ testcontainers 集成测试（CI）。

## Global Constraints

- **DEFAULT 销售域字节等价**：三项的 None/空 路径必须与改造前逐字节一致，由测试锁死。
- **serde 向后兼容**：新字段一律 `#[serde(default, skip_serializing_if = "...")]`；H17 保留 `objection_type` 旧字段可读。
- **AI 永不自动 verify**：H13 AI 生成的状态机本体走 draft + 结构校验 + 人审 activate，绝不自动生效。
- **不造双真相源**：H13 状态机本体运行时单一存 `operation_domain_configs`；profile 上仅暂存"待发布草稿料"，publish 后运行时不读它。
- **反过拟合**：H17 reaction prompt 改"维度名随 profile"抽象机制，非针对单条对话调话术。
- **基线门**：`cargo test --lib` ≥350/0；四 PBT（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥33/0；`scripts/check-no-human-takeover.sh` clean；`-D warnings` 净（CI Baseline gate）。
- **磁盘纪律**：编译前 `rm -rf target/debug/incremental` + `CARGO_INCREMENTAL=0`。
- 提交需用户显式批准；commit 精确 `git add` 命名文件，排除并行会话产物（`.kiro/`、`tests/real_llm_*`、`tests/roleplay_*`、`AGENTS.md`、`agent_t*.txt` 等）；子代理 model:opus；回复中文。

---

## 文件结构

| 文件 | 职责 | 残留 |
| --- | --- | --- |
| `src/models.rs` | DomainProfile 加 `debounce_window_ms_override` / `trajectory_dimensions`；IntentTrajectoryEntry 加 `dimensions` 容器；TrajectoryDimension 新结构 | H17/H18 |
| `src/webhooks.rs:584` | debounce 窗口读 active profile override，None 回落 config | H18 |
| `src/agent/reaction.rs:615/715` | push_intent_trajectory_entry 写侧 + format_intent_trajectory_hint 读侧随 profile | H17 |
| `src/agent/domain_profile.rs` | `default_trajectory_dimensions()`（DEFAULT 销售单维 objection_type）+ 渲染 helper | H17 |
| `src/routes/guide_profile.rs:153` | build_profile_generation_prompt 扩展生成状态机本体 + 落 draft 字段 | H13 |
| `src/routes/domain_profiles.rs:462` | activate 时取 profile 暂存的状态机本体 publish 到 operation_domain_configs | H13 |
| `src/routes/domains.rs:239` | 复用 validate_state_machine 校验 AI 生成本体 | H13 |

---

## H18 — debounce 窗口随 profile（最小，先做打通字段范式）

### Task 1: DomainProfile.debounce_window_ms_override + webhook 接入

**Files:**
- Modify: `src/models.rs`（DomainProfile struct，`memory_dimensions` 字段附近 ~:1474）
- Modify: `src/webhooks.rs:584`
- Modify: `src/agent/domain_profile.rs`（`default_domain_profile()` 字面量补 None）
- Test: `src/models.rs` 内联 `#[cfg(test)]` + `src/webhooks.rs` debounce_tests mod

**Interfaces:**
- Produces: `DomainProfile.debounce_window_ms_override: Option<u64>`；webhook 去抖窗口解析 `resolve_debounce_window_ms(profile: &DomainProfile, config_default: u64) -> u64`（纯函数，便于单测）

- [ ] **Step 1: 写失败测试（纯函数回落语义）**

在 `src/agent/domain_profile.rs` 的 `#[cfg(test)] mod tests` 加：
```rust
#[test]
fn debounce_window_none_falls_back_to_config_default() {
    let p = default_domain_profile("ws");
    assert_eq!(resolve_debounce_window_ms(&p, 4000), 4000, "DEFAULT 无 override 回落 env 默认");
}

#[test]
fn debounce_window_some_overrides_config() {
    let mut p = default_domain_profile("ws");
    p.debounce_window_ms_override = Some(8000);
    assert_eq!(resolve_debounce_window_ms(&p, 4000), 8000, "Some 覆盖 env 默认");
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --lib resolve_debounce 2>&1 | tail`
Expected: FAIL（`resolve_debounce_window_ms` / 字段未定义，编译错误）

- [ ] **Step 3: 加字段 + 纯函数**

`src/models.rs` DomainProfile struct 内（`pub memory_dimensions` 字段后）：
```rust
    /// H18：该行业的 webhook 去抖窗口（毫秒）。None 回落全局 config.message_debounce_window_ms。
    /// 陪伴域可设更长窗口（合并多条情绪表达），销售域用默认。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debounce_window_ms_override: Option<u64>,
```
`src/agent/domain_profile.rs` `default_domain_profile()` 字面量补 `debounce_window_ms_override: None,`；在该文件加纯函数：
```rust
/// H18：解析该 profile 的去抖窗口，None 回落 config 全局默认。
pub(crate) fn resolve_debounce_window_ms(profile: &crate::models::DomainProfile, config_default: u64) -> u64 {
    profile.debounce_window_ms_override.unwrap_or(config_default)
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test --lib resolve_debounce 2>&1 | tail`
Expected: PASS（2 passed）

- [ ] **Step 5: webhook 接入**

`src/webhooks.rs:584` 把 `let window_ms = state.config.message_debounce_window_ms;` 改为：
```rust
            let active_profile = crate::agent::domain_profile::load_active_domain_profile(
                &state.db, &workspace_id,
            ).await;
            let window_ms = crate::agent::domain_profile::resolve_debounce_window_ms(
                &active_profile, state.config.message_debounce_window_ms,
            );
```
（确认 `workspace_id` 在该作用域可得；不可得则用 `&contact.workspace_id` 或就近解析。`load_active_domain_profile` 走进程级缓存，无 N+1。）

- [ ] **Step 6: 全量编译 + 基线**

Run: `rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5`
Expected: PASS（≥350，含新增 2 测试，0 failed）

- [ ] **Step 7: Commit**

```bash
git add src/models.rs src/webhooks.rs src/agent/domain_profile.rs
git commit -m "feat(universal/H18): debounce 窗口下沉 DomainProfile.debounce_window_ms_override（None 回落 env）"
```

---

## H17 — intent_trajectory 轨迹维度容器化（中）

### Task 2: TrajectoryDimension 结构 + DomainProfile.trajectory_dimensions + DEFAULT

**Files:**
- Modify: `src/models.rs`（新增 TrajectoryDimension struct；IntentTrajectoryEntry 加 `dimensions` 容器；DomainProfile 加 `trajectory_dimensions`）
- Modify: `src/agent/domain_profile.rs`（`default_trajectory_dimensions()` + `default_domain_profile()` 补字段）
- Test: `src/models.rs` 内联 + `src/agent/domain_profile.rs` tests

**Interfaces:**
- Produces: `TrajectoryDimension { kind: String, display_name: String }`；`DomainProfile.trajectory_dimensions: Vec<TrajectoryDimension>`；`IntentTrajectoryEntry.dimensions: BTreeMap<String, String>`；`default_trajectory_dimensions() -> Vec<TrajectoryDimension>`（DEFAULT 销售单维 objection_type/"异议类型"）

- [ ] **Step 1: 写失败测试（DEFAULT 单维 + 老数据 round-trip）**

`src/agent/domain_profile.rs` tests：
```rust
#[test]
fn default_trajectory_dimensions_is_objection_only() {
    let dims = default_trajectory_dimensions();
    assert_eq!(dims.len(), 1);
    assert_eq!(dims[0].kind, "objection_type");
    assert_eq!(dims[0].display_name, "异议类型");
}
```
`src/models.rs` tests（向后兼容：老 entry 无 dimensions 字段可反序列化）：
```rust
#[test]
fn intent_trajectory_entry_legacy_objection_round_trips() {
    let legacy = mongodb::bson::doc! { "turnIndex": 3, "intent": "advance", "objectionType": "price" };
    let e: IntentTrajectoryEntry = mongodb::bson::from_document(legacy).unwrap();
    assert_eq!(e.objection_type.as_deref(), Some("price"));
    assert!(e.dimensions.is_empty(), "老数据 dimensions 默认空");
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --lib trajectory 2>&1 | tail`
Expected: FAIL（类型/函数未定义）

- [ ] **Step 3: 加结构 + 字段 + DEFAULT**

`src/models.rs`（IntentTrajectoryEntry 定义附近）：
```rust
    /// H17：通用轨迹维度声明（仿 MemoryDimension）。kind 走 system_taxonomies 字典，
    /// display_name 是渲染给 prompt/人看的标签（销售"异议类型"/陪伴"顾虑类型"）。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct TrajectoryDimension {
        pub kind: String,
        pub display_name: String,
    }
```
IntentTrajectoryEntry struct 内 `objection_type` 字段后加：
```rust
        /// H17：通用轨迹维度容器（key=profile 声明的 kind，value=canonical 取值）。
        /// 老数据无此字段→空 map。DEFAULT 销售域只写 objection_type 旧字段、此容器留空（字节等价）。
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        pub dimensions: std::collections::BTreeMap<String, String>,
```
DomainProfile struct 内（`trajectory` 语义靠近 memory_dimensions）：
```rust
    /// H17：该行业的 intent 轨迹维度声明。空 = DEFAULT 销售（仅 objection_type 旧字段）。
    #[serde(default)]
    pub trajectory_dimensions: Vec<TrajectoryDimension>,
```
`src/agent/domain_profile.rs`：
```rust
/// H17 DEFAULT：销售域轨迹维度 = 单维 objection_type，渲染标签"异议类型"。
/// 与改造前 IntentTrajectoryEntry.objection_type 行为等价（写侧仍走旧字段）。
pub(crate) fn default_trajectory_dimensions() -> Vec<crate::models::TrajectoryDimension> {
    vec![crate::models::TrajectoryDimension {
        kind: "objection_type".to_string(),
        display_name: "异议类型".to_string(),
    }]
}
```
`default_domain_profile()` 补 `trajectory_dimensions: default_trajectory_dimensions(),`。

- [ ] **Step 4: 运行验证通过**

Run: `cargo test --lib trajectory 2>&1 | tail`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/models.rs src/agent/domain_profile.rs
git commit -m "feat(universal/H17): TrajectoryDimension + IntentTrajectoryEntry.dimensions 容器 + DEFAULT 销售单维"
```

### Task 3: reaction 写侧/读侧随 profile（DEFAULT 字节等价）

**Files:**
- Modify: `src/agent/reaction.rs:615`（push_intent_trajectory_entry 写侧）
- Modify: `src/agent/reaction.rs:715`（format_intent_trajectory_hint 读侧）
- Test: `src/agent/reaction.rs` tests mod

**Interfaces:**
- Consumes: `default_trajectory_dimensions()`、`DomainProfile.trajectory_dimensions`、`IntentTrajectoryEntry.dimensions`（Task 2）；`load_active_domain_profile`、`validate_dimension_value`、`llm_signal_apply`（现有）
- Produces: 写侧按 active profile 的 trajectory_dimensions 产出（DEFAULT 走旧字段）；读侧 `format_intent_trajectory_hint` DEFAULT 逐字不变、非销售域读 dimensions 容器渲染 display_name

- [ ] **Step 1: 写失败测试（读侧 DEFAULT 字节等价 + 非销售域维度渲染）**

`src/agent/reaction.rs` tests：
```rust
#[test]
fn hint_default_objection_byte_equivalent() {
    let e = crate::models::IntentTrajectoryEntry {
        turn_index: 2, intent: "advance".into(),
        objection_type: Some("price".into()),
        dimensions: std::collections::BTreeMap::new(),
        recorded_at: crate::models::default_epoch_dt(),
    };
    let hint = format_intent_trajectory_hint(&[e]);
    assert!(hint.contains("第2轮 intent=advance objection_type=price"), "DEFAULT 渲染逐字不变");
}

#[test]
fn hint_renders_profile_dimension_from_container() {
    let mut dims = std::collections::BTreeMap::new();
    dims.insert("concern_type".to_string(), "time".to_string());
    let e = crate::models::IntentTrajectoryEntry {
        turn_index: 5, intent: "share".into(), objection_type: None,
        dimensions: dims, recorded_at: crate::models::default_epoch_dt(),
    };
    let hint = format_intent_trajectory_hint(&[e]);
    assert!(hint.contains("concern_type=time"), "dimensions 容器维度被渲染");
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --lib hint_ 2>&1 | tail`
Expected: FAIL（第二个测试，dimensions 未渲染）

- [ ] **Step 3: 读侧改造（保留旧字段渲染 + 追加容器渲染）**

`src/agent/reaction.rs:724-733` 循环体改为：
```rust
    for entry in recent.iter().rev() {
        buf.push_str(&format!("- 第{}轮 intent={}", entry.turn_index, entry.intent));
        // DEFAULT 销售：旧字段 objection_type 逐字渲染（字节等价）。
        if let Some(t) = entry.objection_type.as_deref() {
            buf.push_str(&format!(" objection_type={}", t));
        }
        // 非销售域：dimensions 容器（key 升序，BTreeMap 稳定）。
        for (k, v) in &entry.dimensions {
            buf.push_str(&format!(" {}={}", k, v));
        }
        buf.push('\n');
    }
```

- [ ] **Step 4: 写侧改造（按 active profile 维度产出）**

`src/agent/reaction.rs:638-667`：DEFAULT（trajectory_dimensions 仅 objection_type）保持写 `objectionType` 旧字段路径不变；profile 声明其它维度时，对每个 `dim.kind` 从 `reaction_analysis` 取同名字段、过 `validate_dimension_value(MachineWrite)`、Accept 落 `dimensions` 容器。实现：
```rust
    let profile = crate::agent::domain_profile::load_active_domain_profile(&state.db, &contact.workspace_id).await;
    let traj_dims = if profile.trajectory_dimensions.is_empty() {
        crate::agent::domain_profile::default_trajectory_dimensions()
    } else {
        profile.trajectory_dimensions.clone()
    };
    let mut entry = doc! { "turnIndex": turn_index, "intent": outcome, "recordedAt": DateTime::now() };
    let mut dim_container = doc! {};
    for dim in &traj_dims {
        let raw = doc_string(reaction_analysis, &to_camel(&dim.kind))
            .or_else(|| doc_string(reaction_analysis, &dim.kind))
            .filter(|s| !s.trim().is_empty());
        let Some(raw) = raw else { continue };
        let verdict = crate::agent::dimension_registry::validate_dimension_value(
            &state.db, &dim.kind, &raw, &contact.account_id,
            crate::agent::dimension_registry::WriteIntent::MachineWrite,
        ).await;
        let Some(canonical) = crate::agent::gateway::llm_signal_apply(verdict) else { continue };
        // DEFAULT 销售单维 objection_type → 写旧字段（字节等价）；其它维度 → dimensions 容器。
        if dim.kind == "objection_type" {
            entry.insert("objectionType", canonical);
        } else {
            dim_container.insert(&dim.kind, canonical);
        }
    }
    if !dim_container.is_empty() {
        entry.insert("dimensions", dim_container);
    }
```
（`to_camel` 若无现成 helper，用就近 snake→camel 工具或内联；确认 `doc_string` 签名。保留 `IntentTrajectoryEntry::MAX_ITEMS` $slice 逻辑不动。）

- [ ] **Step 5: 运行验证通过 + 全量基线**

Run: `cargo test --lib hint_ 2>&1 | tail && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5`
Expected: PASS（含新 2 测试，≥350/0）

- [ ] **Step 6: Commit**

```bash
git add src/agent/reaction.rs
git commit -m "feat(universal/H17): reaction 轨迹写/读随 active profile 维度（DEFAULT objection_type 字节等价）"
```

---

## H13 — 状态机本体随 profile（大，拆 3 任务）

### Task 4: DomainProfile 暂存生成的状态机本体 draft 字段

**Files:**
- Modify: `src/models.rs`（DomainProfile 加 `generated_state_machine: Option<Document>`）
- Modify: `src/agent/domain_profile.rs`（`default_domain_profile()` 补 None）
- Test: `src/models.rs` 内联

**Interfaces:**
- Produces: `DomainProfile.generated_state_machine: Option<mongodb::bson::Document>`（draft 暂存料，activate 时取出 publish 到 operation_domain_configs，发布后运行时不读它 —— 不造双真相源）

- [ ] **Step 1: 写失败测试（None 默认 + round-trip）**

```rust
#[test]
fn generated_state_machine_defaults_none_and_round_trips() {
    let p = default_domain_profile("ws");
    assert!(p.generated_state_machine.is_none());
    let d = mongodb::bson::to_document(&p).unwrap();
    let back: DomainProfile = mongodb::bson::from_document(d).unwrap();
    assert!(back.generated_state_machine.is_none());
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --lib generated_state_machine 2>&1 | tail`
Expected: FAIL（字段未定义）

- [ ] **Step 3: 加字段**

`src/models.rs` DomainProfile：
```rust
    /// H13：引导层 AI 生成 profile 时联动产出的状态机本体（draft 暂存料）。
    /// activate 时取出、过 validate_state_machine、publish 一版新 OperationDomainConfig；
    /// **发布后运行时只读 operation_domain_configs，不读本字段**（不造双真相源）。
    /// None = 无生成本体 → activate 不动状态机，运行时回落现有 DEFAULT 销售 9 态。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_state_machine: Option<mongodb::bson::Document>,
```
`default_domain_profile()` 补 `generated_state_machine: None,`。

- [ ] **Step 4: 运行验证通过**

Run: `cargo test --lib generated_state_machine 2>&1 | tail`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/models.rs src/agent/domain_profile.rs
git commit -m "feat(universal/H13): DomainProfile.generated_state_machine draft 暂存字段"
```

### Task 5: 引导层生成 prompt 扩展产出状态机本体 + 落 draft

**Files:**
- Modify: `src/routes/guide_profile.rs:153`（build_profile_generation_prompt 加 stateMachine schema）
- Modify: `src/routes/guide_profile.rs`（候选落库时 snake_case 转换 + 校验 + 存 generated_state_machine）
- Test: `src/routes/guide_profile.rs` tests（prompt 含 stateMachine schema；校验失败回落 None）

**Interfaces:**
- Consumes: `DomainProfile.generated_state_machine`（Task 4）；`validate_state_machine`（domains.rs:239，需提升可见性到 `pub(crate)` 或经路由复用）
- Produces: 生成候选 profile 时 `generated_state_machine` 字段被填充（过 validate）或 None（校验不过 / LLM 未产出）

- [ ] **Step 1: 写失败测试（prompt 含 stateMachine schema 段）**

```rust
#[test]
fn generation_prompt_includes_state_machine_schema() {
    let prompt = build_profile_generation_prompt("卖课的教育机构", &[]);
    assert!(prompt.contains("stateMachine"), "生成 prompt 须含状态机本体 schema");
    assert!(prompt.contains("initial"), "状态机须声明 initial 标志");
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --lib generation_prompt_includes_state_machine 2>&1 | tail`
Expected: FAIL

- [ ] **Step 3: prompt 扩展 + 落库校验**

`build_profile_generation_prompt`（:153）在 profileDimensions schema 后追加 stateMachine schema 段（states 数组，每态 key/name/goal/advanceSignals/riskRules + initial/allowedFrom/forbidsProactive 标志；要求至少一个 initial=true）。候选落库处（~:275）：把 LLM 返回的 stateMachine 经 snake_case 转换为 Document → `validate_state_machine(&doc)`：Ok 则存 `generated_state_machine: Some(doc)`，Err 则 `None` + log warn（不阻断 profile 生成，状态机缺失运行时回落 DEFAULT）。
> 需把 `validate_state_machine`（domains.rs:239 `pub(super)`）提升为 `pub(crate)` 供 guide_profile 复用。

- [ ] **Step 4: 写校验回落测试**

```rust
#[test]
fn invalid_state_machine_falls_back_to_none() {
    // 缺 initial 的状态机 → validate 拒 → 候选 generated_state_machine = None
    let bad = mongodb::bson::doc! { "states": [ { "key": "a", "allowedFrom": ["b"] } ] };
    assert!(crate::routes::domains::validate_state_machine(&bad).is_err());
}
```

- [ ] **Step 5: 运行验证通过 + 基线**

Run: `cargo test --lib state_machine 2>&1 | tail && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5`
Expected: PASS（≥350/0）

- [ ] **Step 6: Commit**

```bash
git add src/routes/guide_profile.rs src/routes/domains.rs
git commit -m "feat(universal/H13): 引导层生成 prompt 联动产出状态机本体 + validate + 落 draft（校验不过回落 None）"
```

### Task 6: activate 联动 publish 状态机本体到 operation_domain_configs

**Files:**
- Modify: `src/routes/domain_profiles.rs:462`（activate_domain_profile 取 generated_state_machine publish）
- Test: `tests/domain_profile_e2e.rs`（集成测试，#[ignore]，testcontainers）

**Interfaces:**
- Consumes: `DomainProfile.generated_state_machine`（Task 4）；`publish_operation_domain_version`（admin_ops_versions.rs:45）或其内部 publish 逻辑；`OperationDomainConfig`（models.rs:764）
- Produces: activate 后 `operation_domain_configs` 在 `(workspace_id, domain="user_operations")` 下 publish 一版新 current，`state_machine` = profile 的 generated_state_machine

- [ ] **Step 1: 写失败集成测试**

`tests/domain_profile_e2e.rs` 加 `#[tokio::test] #[ignore]`：
```rust
// 激活带 generated_state_machine 的 profile → operation_domain_configs 新 current 版本的
// state_machine 等于该本体；激活不带本体的 profile → operation_domain_configs 不变（回落 DEFAULT）。
```
（断言 publish 后 `state.db` 查 `operation_domain_configs` current_version=true 行的 state_machine 含生成的 state keys；无本体时版本号不增。）

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --test domain_profile_e2e activate_publishes_state_machine -- --ignored 2>&1 | tail`（无 Docker 则确认编译失败即可）
Expected: FAIL / 编译错误（联动逻辑未加）

- [ ] **Step 3: activate 联动 publish**

`activate_domain_profile`（:482 切 is_active 后、:497 invalidate 前）插入：取 `target.generated_state_machine`，Some 则调用 publish 逻辑写一版新 `OperationDomainConfig`（复用 `publish_operation_domain_version` 的版本递增 + current 切换；domain="user_operations"，state_machine=本体，seeded_by 标 profile_id 溯源）；None 则不动状态机。事务性：与 is_active 切换同一 handler，失败回滚语义对齐现有 publish。

- [ ] **Step 4: 运行验证通过（CI/有 Docker）+ 本地基线**

Run: `rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5`（集成测试靠 CI）
Expected: lib ≥350/0；集成测试 CI 绿

- [ ] **Step 5: Commit**

```bash
git add src/routes/domain_profiles.rs tests/domain_profile_e2e.rs
git commit -m "feat(universal/H13): activate 联动 publish 状态机本体到 operation_domain_configs（消费方零改动/回落 DEFAULT）"
```

---

## 收尾

### Task 7: 全链验证 + 文档同步

- [ ] **Step 1: 全基线**

Run: `rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5 && cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter 2>&1 | grep "test result" && bash scripts/check-no-human-takeover.sh 2>&1 | tail -2`
Expected: lib ≥350/0；四 PBT ≥33/0；lint 0 violations

- [ ] **Step 2: 文档同步**

更新 `docs/superpowers/specs/2026-06-11-universal-domain-adaptation-design.md` 残留核查节：H13/H17/H18 标已收口。

- [ ] **Step 3: Commit + 等用户授权推送**

```bash
git add docs/superpowers/specs/2026-06-11-universal-domain-adaptation-design.md
git commit -m "docs(universal): 三残留 H13/H17/H18 收口完成，更新残留核查节"
```

---

## Self-Review

**Spec coverage**：H18（Task1）✓ / H17（Task2 结构 + Task3 读写）✓ / H13（Task4 draft 字段 + Task5 生成+校验 + Task6 activate publish）✓ / 收尾验证（Task7）✓。spec 三模块全覆盖。

**Type consistency**：`debounce_window_ms_override: Option<u64>`、`trajectory_dimensions: Vec<TrajectoryDimension>`、`dimensions: BTreeMap<String,String>`、`generated_state_machine: Option<Document>`、`resolve_debounce_window_ms`、`default_trajectory_dimensions`、`validate_state_machine`（pub(crate)）—— 跨任务签名一致。

**红线**：DEFAULT 字节等价（H18 None 回落 / H17 旧字段路径 / H13 无本体回落 DEFAULT 9 态）每任务有测试；AI 永不自动 verify（H13 Task5 落 draft + validate、Task6 人工 activate 才 publish）；serde 向后兼容（H17 Task2 legacy round-trip 测试）；不造双真相源（H13 Task4 字段注释明确 publish 后运行时不读 profile 暂存料）。

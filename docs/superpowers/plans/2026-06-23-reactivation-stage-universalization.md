# reactivation 目标 stage 通用化 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `reactivation_candidate_filter` 硬编码的销售 stage `"dormant_reactivation"` 升级为字典可声明的 `is_reactivation_target` 标记，让非销售域 profile 能声明自己的再激活目标 stage（修复审查 #3：非销售域 reactivation 扫描器 DB 预筛恒空、静默失效）。

**Architecture:** 加一个与现有 `is_terminal` 链路逐处对称的维度标记 `is_reactivation_target`：`TaxonomyValue` 字段 → `taxonomy.rs` cache + `dimension_value_weights` 四元组 → `PlannerStageConfig.reactivation_stages` + `effective_reactivation_stages()` → `reactivation_candidate_filter` 接 config 用 `$in` 预筛 → m006 seed 仅 `dormant_reactivation` 标 true。DEFAULT 销售域**字节等价**（单元素 `$in` ≡ `==`，空字典回落 `["dormant_reactivation"]`）。

**Tech Stack:** Rust 2021 / Axum / MongoDB（bson `doc!`）；`#[serde(default)]` 向后兼容。

## Global Constraints

- 子代理 ALWAYS `model: "opus"`；回复中文。
- `cargo test --lib` ≥350 passed / 0 failed；四 PBT（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥33 / 0 不回归。
- 后端编译验证用 `RUSTFLAGS="-D warnings" cargo check --tests`（磁盘受限：本地 `cargo test --lib` 链接 100+ 集成测试二进制会 `os error 112`；lib 测试断言留 CI 基线门跑）。但**纯函数单测**（不依赖 Docker/Mongo）可用 `cargo test --lib <name>` 单点跑验证（footprint 小）。
- **DEFAULT 销售域字节等价**：单元素 `$in` ≡ `==`；空字典回落 = `["dormant_reactivation"]`。任何任务不得改变销售域 reactivation 行为。
- **向后兼容**：新字段 `#[serde(default)]`，旧 BSON 文档/旧 LLM 输出反序列化不破。
- **agent-first**：不引入关键词匹配，不动 `customer_stage` 的 LLM 语义判定（[[project_agent_first_no_keyword_filters]]）。
- **不过拟合**：标记是可复现抽象（任意域可声明），非对单条对话点对点修补（[[feedback_no_overfitting]]）。
- 精确 `git add` 指定文件，排除并行产物（`.kiro/*` `AGENTS.md` `agent_t*.txt` `t15_single.txt` `dead-code-analysis.md` `docs/superpowers/plans/2026-06-21-sales-media-asset-send.md` 及其它 `??` 计划文件）。
- 提交需用户显式批准。
- `scripts/check-no-human-takeover.sh` / `check-no-model-hint.sh` clean（本改动不涉红线措辞/模型名，顺带确认）。

## File Structure

| 文件 | 职责 | 本计划改动 |
| --- | --- | --- |
| `src/models.rs` | BSON serde 结构 | `TaxonomyValue` 加 `is_reactivation_target` 字段 + 1 处测试 fixture 补字段 |
| `src/agent/taxonomy.rs` | 维度字典缓存 + 派生 | cache struct 加字段 + 4 处构造点补字段 + `dimension_value_weights` 四元组 |
| `src/planner/mod.rs` | 扫描器 + stage 配置 | `PlannerStageConfig` 加 `reactivation_stages` + `effective_reactivation_stages()` + build 填充 + `reactivation_candidate_filter` 接 config + scan_reactivation 调用点 + 1 测试改写 |
| `src/db/migrations/m006_taxonomy_seed.rs` | 字典 seed + 护栏测试 | 8 元组 seed 标记 + H6 护栏测试加对称断言 |

**任务顺序（依赖驱动）**：T1（字段，无依赖）→ T2（taxonomy 派生，依赖 T1 字段）→ T3（PlannerStageConfig + filter，依赖 T2 四元组）→ T4（m006 seed + 护栏，依赖 T1 字段）。T4 也可与 T2/T3 并行（只依赖 T1），但顺序执行最稳。

---

## Task 1：`TaxonomyValue` 加 `is_reactivation_target` 字段

**Files:**
- Modify: `src/models.rs:2571`（`is_terminal` 字段后加新字段）
- Modify: `src/models.rs:4879`（测试 fixture `taxonomy_entry_bson_round_trip` 补字段）

**Interfaces:**
- Consumes: 无
- Produces: `TaxonomyValue.is_reactivation_target: bool`（`#[serde(default)]`，默认 false）。T2/T4 消费此字段。

- [ ] **Step 1: 加字段**

在 `src/models.rs` 的 `TaxonomyValue` struct，`pub is_terminal: bool,`（:2571）之后插入：

```rust
    /// universal-domain-adaptation #3：该取值是否为「再激活目标」stage（profile 可声明）。
    /// 与 is_terminal 正交——`dormant_reactivation` 既是终态又是再激活目标，而
    /// `customer_success` / `cooldown` 是终态但非再激活目标。`#[serde(default)]` 保证旧
    /// BSON 文档 / 未声明此标记的维度向后兼容（缺省 false）。planner 据此构造 reactivation
    /// 目标集合替代写死的 `"dormant_reactivation"` 字面量。
    #[serde(default)]
    pub is_reactivation_target: bool,
```

- [ ] **Step 2: 补测试 fixture**

`src/models.rs` 的 `taxonomy_entry_bson_round_trip` 测试（:4866），`TaxonomyValue` 构造里 `is_terminal: false,`（:4879）之后加一行：

```rust
                is_reactivation_target: false,
```

- [ ] **Step 3: 编译验证**

Run: `RUSTFLAGS="-D warnings" cargo check --lib 2>&1 | tail -20`
Expected: 报错列出 `taxonomy.rs` / `m006_taxonomy_seed.rs` / `planner/mod.rs` 等处 `missing field is_reactivation_target`（这些是 T2/T3/T4 要补的构造点，预期内）。**只要 `models.rs` 本身无错**即 Step 通过——missing-field 错误证明字段已加且强制下游补齐。

> 说明：本任务单独不可能 `cargo check` 全绿（其它构造点未补）。这是预期的——字段是编译期强约束的源头。Step 3 只确认 models.rs 自身语法正确、错误都落在下游构造点。

- [ ] **Step 4: Commit**

```bash
git add src/models.rs
git commit -m "feat(taxonomy): TaxonomyValue 加 is_reactivation_target 字段(serde default,审查#3)"
```

---

## Task 2：taxonomy.rs cache + `dimension_value_weights` 四元组

**Files:**
- Modify: `src/agent/taxonomy.rs:89`（`CachedEntry` struct 加字段）
- Modify: `src/agent/taxonomy.rs:146`（`reload_from_db` 构造点）
- Modify: `src/agent/taxonomy.rs:584`（`build_cache_from_entries` 构造点）
- Modify: `src/agent/taxonomy.rs:613`（测试 helper `make_cache_with_entries` 构造点）
- Modify: `src/agent/taxonomy.rs:642`（测试 helper `make_entry` 构造点）
- Modify: `src/agent/taxonomy.rs:258-276`（`dimension_value_weights` 返回四元组）

**Interfaces:**
- Consumes: `TaxonomyValue.is_reactivation_target`（T1）
- Produces: `dimension_value_weights(kind, scope_account_id, cache) -> Vec<(String, Option<i32>, bool, bool)>`，四元组 = `(canonical_id, priority_weight, is_terminal, is_reactivation_target)`。T3 的 `build_planner_stage_config` 消费第四位。

- [ ] **Step 1: `CachedEntry` 加字段**

`src/agent/taxonomy.rs` 的 `CachedEntry` struct，`is_terminal: bool,`（:89）后加：

```rust
    /// universal-domain-adaptation #3：是否再激活目标 stage（来自 TaxonomyValue）。
    is_reactivation_target: bool,
```

- [ ] **Step 2: 补 4 处构造点**

以下每处 `CachedEntry { ... is_terminal: <X>, }` 后补 `is_reactivation_target`：

`:146`（reload_from_db）— 来自 entry：
```rust
                    is_terminal: entry.value.is_terminal,
                    is_reactivation_target: entry.value.is_reactivation_target,
```

`:584`（build_cache_from_entries）— 来自 entry：
```rust
            is_terminal: entry.value.is_terminal,
            is_reactivation_target: entry.value.is_reactivation_target,
```

`:613`（测试 make_cache_with_entries）— 来自 entry：
```rust
                    is_terminal: entry.value.is_terminal,
                    is_reactivation_target: entry.value.is_reactivation_target,
```

`:642`（测试 make_entry helper）— 固定 false：
```rust
                is_terminal: false,
                is_reactivation_target: false,
```

- [ ] **Step 3: `dimension_value_weights` 改四元组**

`:250` 的 doc 注释把三元组改述四元组（加 `is_reactivation_target`）。函数签名（:262）与内部 Vec 类型（:264）：

```rust
) -> Vec<(String, Option<i32>, bool, bool)> {
    let inner = cache.inner.lock();
    let mut out: Vec<(String, Option<i32>, bool, bool)> = Vec::new();
```

push 处（:271）：

```rust
                    out.push((
                        e.canonical_id.clone(),
                        e.priority_weight,
                        e.is_terminal,
                        e.is_reactivation_target,
                    ));
```

- [ ] **Step 4: 编译验证**

Run: `RUSTFLAGS="-D warnings" cargo check --lib 2>&1 | tail -20`
Expected: 报错只剩 `planner/mod.rs:909` / `:919` 两处 `dimension_value_weights` 解构 pattern 不匹配（三元组 vs 四元组）——T3 要补。taxonomy.rs 自身无错。

> 同 T1：本任务单独不全绿，错误落在 T3 的解构点，预期内。

- [ ] **Step 5: Commit**

```bash
git add src/agent/taxonomy.rs
git commit -m "feat(taxonomy): dimension_value_weights 扩四元组带 is_reactivation_target(审查#3)"
```

---

## Task 3：PlannerStageConfig + reactivation_candidate_filter 通用化

**Files:**
- Modify: `src/planner/mod.rs:823-832`（`PlannerStageConfig` struct 加字段）
- Modify: `src/planner/mod.rs:834-844`（`Default` impl 补字段）
- Modify: `src/planner/mod.rs:875-881`（加 `effective_reactivation_stages()`，紧邻 `effective_terminal_stages`）
- Modify: `src/planner/mod.rs:903-918`（`build_planner_stage_config` 构造 + customer_stage 循环填充 + 四元组解构）
- Modify: `src/planner/mod.rs:919`（intent_level 循环四元组解构）
- Modify: `src/planner/mod.rs:1933-1947`（`reactivation_candidate_filter` 接 config）
- Modify: `src/planner/mod.rs:1974`（`scan_reactivation` 调用点）
- Modify: `src/planner/mod.rs:3410-3421`（测试 `reactivation_candidate_filter_includes_dormant_stage` 改写）

**Interfaces:**
- Consumes: `dimension_value_weights(...) -> Vec<(String, Option<i32>, bool, bool)>`（T2）
- Produces: `reactivation_candidate_filter(workspace_id: &str, account_id: &str, stage_config: &PlannerStageConfig) -> Document`（签名加第三参）；`PlannerStageConfig::effective_reactivation_stages(&self) -> Vec<String>`。

- [ ] **Step 1: 改测试为新签名 + `$in` 断言（先写失败测试）**

`src/planner/mod.rs:3410` 的 `reactivation_candidate_filter_includes_dormant_stage` 整体替换为：

```rust
    /// reactivation_candidate_filter 用 stage_config 的再激活目标集合做 $in 预筛
    /// （DEFAULT 回落 ["dormant_reactivation"]）+ managed + 非冷却。
    #[test]
    fn reactivation_candidate_filter_includes_dormant_stage() {
        let cfg = PlannerStageConfig::default();
        let f = reactivation_candidate_filter("ws1", "acc1", &cfg);
        assert_eq!(f.get_str("workspace_id").unwrap(), "ws1");
        assert_eq!(f.get_str("account_id").unwrap(), "acc1");
        assert_eq!(f.get_str("agent_status").unwrap(), "managed");
        let stage = f
            .get_document("domain_attributes.customer_stage")
            .expect("customer_stage 应为 $in 文档");
        let targets: Vec<&str> = stage
            .get_array("$in")
            .expect("含 $in 数组")
            .iter()
            .map(|b| b.as_str().expect("$in 元素为字符串"))
            .collect();
        assert_eq!(
            targets,
            vec!["dormant_reactivation"],
            "DEFAULT 回落只扫休眠态老客（与原 == 查询字节等价）"
        );
        assert!(f.get_array("$or").is_ok(), "含 cooldown 非冷却 $or 粗筛");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib reactivation_candidate_filter_includes_dormant_stage 2>&1 | tail -15`
Expected: 编译失败（`reactivation_candidate_filter` 仍是两参 + 返回 `==` 字符串；`effective_reactivation_stages` 未定义）。证明测试咬住新契约。

- [ ] **Step 3: `PlannerStageConfig` 加字段**

struct（:828 `terminal_stages` 后）：

```rust
    /// 终态 stage canonical id 集合（is_terminal=true）。
    terminal_stages: std::collections::HashSet<String>,
    /// 再激活目标 stage canonical id 集合（is_reactivation_target=true）。供
    /// reactivation_candidate_filter 的 MongoDB `$in` 预筛。
    reactivation_stages: std::collections::HashSet<String>,
```

`Default` impl（:840 `terminal_stages: ...new(),` 后）：

```rust
            terminal_stages: std::collections::HashSet::new(),
            reactivation_stages: std::collections::HashSet::new(),
```

- [ ] **Step 4: 加 `effective_reactivation_stages()`**

`effective_terminal_stages`（:875-881）方法之后、`impl` 块闭合 `}`（:882）之前插入：

```rust
    /// 有效再激活目标集合（供 MongoDB 端 `$in` 预筛）：字典非空用字典，否则回落写死
    /// `["dormant_reactivation"]`（销售域 DEFAULT）。与 [`reactivation_candidate_filter`]
    /// 同源；单元素回落与原 `== "dormant_reactivation"` 查询字节等价。
    fn effective_reactivation_stages(&self) -> Vec<String> {
        if self.reactivation_stages.is_empty() {
            vec!["dormant_reactivation".to_string()]
        } else {
            self.reactivation_stages.iter().cloned().collect()
        }
    }
```

- [ ] **Step 5: build 构造 + 四元组解构 + 填充**

`build_planner_stage_config` 内 config 构造（:906-907 `terminal_stages: ...new(),` 后）补：

```rust
        terminal_stages: std::collections::HashSet::new(),
        reactivation_stages: std::collections::HashSet::new(),
```

customer_stage 循环（:909-918）整体替换为（解构补第四位 + 填充 reactivation_stages；注意 `id` 被多次用，`terminal_stages.insert` 改 `id.clone()`）：

```rust
    for (id, weight, is_terminal, is_reactivation_target) in
        dimension_value_weights("customer_stage", account_id, &cache)
    {
        if let Some(w) = weight {
            config.stage_weights.insert(id.clone(), w);
        }
        if is_terminal {
            config.terminal_stages.insert(id.clone());
        }
        if is_reactivation_target {
            config.reactivation_stages.insert(id);
        }
    }
```

intent_level 循环（:919）解构补第四位（用 `_` 忽略 is_terminal + is_reactivation_target）：

```rust
    for (id, weight, _is_terminal, _is_reactivation_target) in
        dimension_value_weights("intent_level", account_id, &cache)
    {
```

- [ ] **Step 6: `reactivation_candidate_filter` 接 config**

`:1933-1947` 整体替换：

```rust
/// MongoDB 端粗筛：managed + 非冷却 + customer_stage ∈ 再激活目标集合（stage_config 派生，
/// DEFAULT 回落 ["dormant_reactivation"]）。逐 contact 的 reactivation.enabled 短路 + 休眠
/// 时长 + cadence 节奏在 Rust 侧做。
pub(crate) fn reactivation_candidate_filter(
    workspace_id: &str,
    account_id: &str,
    stage_config: &PlannerStageConfig,
) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "agent_status": "managed",
        "domain_attributes.customer_stage": {
            "$in": stage_config.effective_reactivation_stages()
        },
        "$or": [
            { "cooldown_until": { "$exists": false } },
            { "cooldown_until": null },
            { "cooldown_until": { "$lt": DateTime::now() } },
        ],
    }
}
```

- [ ] **Step 7: `scan_reactivation` 调用点传 config**

`src/planner/mod.rs:1974` 的 `let filter = reactivation_candidate_filter(&workspace_id, &account_id);` 替换为（profile 已在 :1961 加载，先 build stage_config）：

```rust
    let stage_config = build_planner_stage_config(state, &account_id, &profile).await;
    let filter = reactivation_candidate_filter(&workspace_id, &account_id, &stage_config);
```

- [ ] **Step 8: 运行改写的测试 + 编译**

Run: `cargo test --lib reactivation_candidate_filter_includes_dormant_stage 2>&1 | tail -15`
Expected: PASS（`$in: ["dormant_reactivation"]` 断言通过）。
Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -20`
Expected: 0 error / 0 warning（四元组解构全补齐，无 unused）。

- [ ] **Step 9: Commit**

```bash
git add src/planner/mod.rs
git commit -m "feat(planner): reactivation_candidate_filter 接 PlannerStageConfig 用 \$in 预筛(审查#3,DEFAULT字节等价)"
```

---

## Task 4：m006 seed 标记 + H6 对称护栏

**Files:**
- Modify: `src/db/migrations/m006_taxonomy_seed.rs:82-155`（元组类型 6→7 列 + 9 行数据 + 解构 + TaxonomyValue 构造）
- Modify: `src/db/migrations/m006_taxonomy_seed.rs:414-436`（H6 护栏测试加 is_reactivation_target 断言）

**Interfaces:**
- Consumes: `TaxonomyValue.is_reactivation_target`（T1）
- Produces: 字典 seed 中仅 `dormant_reactivation` 的 `is_reactivation_target=true`，其余 customer_stage 取值 =false。

- [ ] **Step 1: 改元组类型 + 9 行数据**

`src/db/migrations/m006_taxonomy_seed.rs:80-82` 的注释与类型声明：

```rust
    // ── customer_stage（9 项，对齐 default_user_operation_state_machine）──
    // 元组末三列 = (priority_weight, is_terminal, is_reactivation_target)，逐字复刻
    // planner::stage_priority_weight 的 match 分支、planner::TERMINAL_STAGES，
    // 与 reactivation_candidate_filter DEFAULT 回落，使配置化后 DEFAULT 行为零变化。
    let customer_stages: &[(&str, &str, &str, &[&str], i32, bool, bool)] = &[
```

9 行各加末列：前 8 行（new_contact / relationship_building / need_discovery / solution_fit / objection_handling / commitment_followup / customer_success / cooldown）末列 = `false`；**仅** `dormant_reactivation`（:148-154）末列 = `true`。例如 customer_success（:131-138）：

```rust
        (
            "customer_success",
            "客户维护",
            "维护成交后关系，发现复购、转介绍和服务风险。",
            &["交付维护", "复购转介绍", "post_sale"],
            10,
            true,
            false,
        ),
```

dormant_reactivation（:147-154）：

```rust
        (
            "dormant_reactivation",
            "沉默唤醒",
            "基于真实价值或明确理由做低频唤醒。",
            &["唤醒", "沉默用户唤醒"],
            10,
            true,
            true,
        ),
```

（其余 7 行同样在原 `is_terminal` 列后补一列 `false`。）

- [ ] **Step 2: 改解构 + TaxonomyValue 构造**

`:156` 的解构与 `:161-169` 的构造：

```rust
    for (id, display, desc, aliases, weight, terminal, reactivation_target) in customer_stages {
        out.push(TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: "customer_stage".to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
                priority_weight: Some(*weight),
                is_terminal: *terminal,
                is_reactivation_target: *reactivation_target,
            },
```

> 注意：此文件还有 intent_level / objection_type 的 seed 块（在 customer_stage 之后），它们各自的 `TaxonomyValue` 构造也需补 `is_reactivation_target: false`（编译会报 missing field 指出确切行）。Step 4 编译时按报错逐处补 false。

- [ ] **Step 3: H6 护栏加对称断言**

`seeded_weights_match_planner_hardcoded_verbatim` 测试（:415），在 customer_stage 循环（:422-436）内、`is_terminal` 断言之后加 `is_reactivation_target` 断言：

```rust
            assert_eq!(
                entry.value.is_terminal,
                terminal.contains(&id),
                "customer_stage \"{}\" 的 is_terminal 必须与 TERMINAL_STAGES 一致",
                id
            );
            assert_eq!(
                entry.value.is_reactivation_target,
                id == "dormant_reactivation",
                "customer_stage \"{}\" 的 is_reactivation_target 仅 dormant_reactivation 应为 true\
                 （与 reactivation_candidate_filter DEFAULT 回落一致，防字典/回落漂移）",
                id
            );
```

- [ ] **Step 4: 编译 + 跑护栏测试**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -20`
Expected: 0 error（若报其它 TaxonomyValue 构造缺 field，逐处补 `is_reactivation_target: false`）/ 0 warning。
Run: `cargo test --lib seeded_weights_match_planner_hardcoded_verbatim 2>&1 | tail -15`
Expected: PASS（仅 dormant_reactivation 标 true 的断言通过）。

- [ ] **Step 5: Commit**

```bash
git add src/db/migrations/m006_taxonomy_seed.rs
git commit -m "feat(m006): customer_stage seed 标记 is_reactivation_target+H6对称护栏(仅dormant_reactivation,审查#3)"
```

---

## Task 5：全量基线 + lint 收口

**Files:** 无（验证任务）

**Interfaces:** Consumes 全部前序任务产物；Produces 无。

- [ ] **Step 1: 全量编译**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -10`
Expected: 0 error / 0 warning。

- [ ] **Step 2: 跑受影响的 lib 单测**

Run: `cargo test --lib taxonomy 2>&1 | tail -15`
Expected: PASS（dimension_value_weights 相关、cache 相关测试不回归）。
Run: `cargo test --lib reactivation 2>&1 | tail -15`
Expected: PASS。
Run: `cargo test --lib seeded_weights 2>&1 | tail -10`
Expected: PASS。

> 全量 `cargo test --lib`（≥350）与四 PBT 留 CI 基线门跑（本地磁盘受限链接会 os error 112）。本地只点跑上述小 footprint 单测。

- [ ] **Step 3: 红线 lint**

Run: `bash scripts/check-no-human-takeover.sh origin/main HEAD 2>&1 | tail -5`
Expected: clean。
Run: `bash scripts/check-no-model-hint.sh origin/main HEAD 2>&1 | tail -5`
Expected: clean。

- [ ] **Step 4: 推送 + 开 PR（用户批准后）**

```bash
git push -u origin fix/reactivation-stage-universalization
gh pr create --title "fix(planner): reactivation 目标 stage 通用化(审查#3 字典 is_reactivation_target)" --body "$(cat <<'EOF'
## Summary
- 审查 #3：`reactivation_candidate_filter` 硬编码销售 stage `"dormant_reactivation"`，非销售域 DB 预筛恒空 → reactivation 扫描器静默失效。
- 加与 `is_terminal` 对称的字典标记 `is_reactivation_target`，`PlannerStageConfig` 派生 `reactivation_stages`，filter 接 config 用 `$in` 预筛。
- DEFAULT 销售域字节等价（单元素 `$in` ≡ `==`，空字典回落 `["dormant_reactivation"]`）。
- 非销售域只需在自己的 customer_stage 字典标记对应"沉默/流失"语义 stage，无需改代码。

## Test plan
- [ ] CI 基线门：`cargo test --lib` ≥350/0 + 四 PBT ≥33/0
- [ ] `reactivation_candidate_filter_includes_dormant_stage`（改写为 `$in` 断言）
- [ ] `seeded_weights_match_planner_hardcoded_verbatim`（加 is_reactivation_target 对称护栏）
- [ ] CI Integration job

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## 自审

**1. Spec 覆盖**：设计 5 处对称改动 → T1（#1 TaxonomyValue 字段）/ T2（#2 taxonomy 四元组）/ T3（#3 PlannerStageConfig + #4 filter）/ T4（#5 m006 seed）。设计「测试影响」4 条 → T3 Step1（planner:3410 改写）/ T4 Step3（m006 对称护栏）/ T3 Step4（effective_reactivation_stages 经 DEFAULT 测试覆盖）/ T2+T3（四元组编译同步）。全覆盖。

**2. Placeholder 扫描**：无 TBD/TODO；每个改代码的 Step 都给了完整代码块；编译验证给了确切命令与预期。T1/T2 的"单独不全绿"已明确说明是预期（编译期强约束源头），非占位。

**3. 类型一致性**：四元组 `(String, Option<i32>, bool, bool)` 在 T2 定义（dimension_value_weights 返回）、T3 消费（build 两处解构）一致。`effective_reactivation_stages(&self) -> Vec<String>` 在 T3 Step4 定义、Step6 filter 调用一致。`reactivation_candidate_filter(.., stage_config: &PlannerStageConfig)` 三参签名在 T3 Step6 定义、Step1 测试 + Step7 scan 调用一致。`is_reactivation_target: bool` 字段名跨 T1/T2/T4 一致。

**4. 风险点**：T3 Step5 的 `id` move——customer_stage 循环里 `id` 被 stage_weights/terminal_stages/reactivation_stages 三处用，已用 `id.clone()` 前两处、最后一处 move，给出完整循环代码避免 borrow-check 错。

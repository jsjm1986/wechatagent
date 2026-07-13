# P3 家族② serde 键容错 / verified 口径对齐 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 两处低风险一致性加固——D-01 给 RawAgentDecision 顶层标签键加 snake_case alias（与初始画像路径双形容错对齐），KB-03 把 is_verified 判定收窄为精确小写匹配（与写入/召回侧口径对齐）。

**Architecture:** 两条独立纯逻辑/serde 加固，互不依赖。D-01 只在 `types.rs` 加两个 `#[serde(alias)]` 属性 + serde 单测；KB-03 只在 `guards.rs` 改一个比较表达式 + 更新注释 + 扩单测。两个 task 各自独立可 review。

**Tech Stack:** Rust 2021，serde derive，纯 lib 单测（本地可跑），无 Docker、无新依赖。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-13-p3-family2-serde-key-tolerance-design.md`（已获批 commit a76f109）。所有行号亲验于分支 fix/p3-family2-serde-key-tolerance（基于 origin/main 4ccaf5c 含 #193）。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆。
- 反过拟合红线：真 bug 才修；改既有测试断言仅限"被本修复有意废除的旧行为 / 签名变更被迫更新"，绝不为过测试改业务逻辑。
- **D-01 只加 alias 不改 rename_all**：`RawAgentDecision` 的 `#[serde(rename_all="camelCase")]`（types.rs:405）不动；alias 是**额外**接受 snake_case，主名仍是 camelCase。只给 `customer_stage`/`intent_level` 两字段加 alias（台账点名的双层标签权威字段），不顺手给其它字段加（YAGNI）。
- **D-01 无误吸风险（已亲验）**：serde 顶层 alias 只作用于**顶层键空间**；`dimension_display_names`（types.rs:132 嵌套 `Document`）内的同名 snake_case 中文显示名键不在顶层反序列化范围，alias 抓不到它。
- **KB-03 是等价收窄**：写入侧 `verify.rs` 恒写小写 `"verified"`，当前不存在 `"Verified"`，改精确匹配对现有数据行为完全等价（纯 latent 口径漂移消除）。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不回退。改动不触 baseline 门 4 PBT（state_transition/memory_card/wiki_chunk_revision/llm_retry_jitter）。
- check-no-human-takeover lint 扫 `src/agent/` 新增行禁词——本 PR 改动为 serde 属性 + 比较符 + 中性注释，不得含禁词。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。绝不动任何 sibling worktree 的 target/。**所有文件路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`；所有 git 命令先 `cd` 到该 worktree 目录再执行**（主仓被并行会话占用，误落主仓/串分支会污染他人工作）。

---

## File Structure

- `src/agent/types.rs`：**Modify** `RawAgentDecision` 的 `customer_stage`（:461）/`intent_level`（:462）各加 `#[serde(alias=...)]` + 在既有 `mod tests`（尾部，紧邻 `raw_decision_without_escalation_still_parses` :2042-2046 之后）新增 1 个 serde 双形单测。D-01 全在此文件。
- `src/agent/guards.rs`：**Modify** `is_verified`（:312-314）比较表达式 `eq_ignore_ascii_case`→`==` + 更新函数 doc（:308-311）+ 在既有 `mod tests`（:316-325）扩断言。KB-03 全在此文件。

两文件互不依赖，两 task 可任意序，建议 D-01 → KB-03。

---

## Task 1: D-01 —— RawAgentDecision 顶层标签键加 snake_case alias（types.rs）

**Files:**
- Modify: `src/agent/types.rs:461-462`（`customer_stage`/`intent_level` 加 alias）
- Test: `src/agent/types.rs`（既有 `mod tests` 内新增 1 serde 双形单测）

**Interfaces:**
- Consumes: 无。
- Produces: 无对外接口变化（`RawAgentDecision` 字段类型不变，仅额外接受 snake_case 键）。

- [ ] **Step 1: 先写单测（先写，验证会失败——当前顶层 snake_case 被 miss 成 None）**

在 `src/agent/types.rs` 的既有 `mod tests` 内、`raw_decision_without_escalation_still_parses`（:2042-2046）之后新增：

```rust
    /// D-01：LLM 若顶层输出 snake_case customer_stage / intent_level（而非 schema
    /// 要求的 camelCase），须经 #[serde(alias)] 正确吸收为 Some，不再静默 miss→None
    /// 致标签丢失。与初始画像路径 decision.rs 的 camel→snake 双形兜底对齐。
    /// 回退（去掉 alias）即变红——rename_all=camelCase 下顶层 snake_case 恒 miss。
    #[test]
    fn raw_decision_accepts_snake_case_stage_and_intent() {
        // 顶层用 snake_case（LLM 偶发形态）。
        let snake = r#"{"customer_stage":"decision","intent_level":"high"}"#;
        let raw: RawAgentDecision = serde_json::from_str(snake).expect("parse snake");
        assert_eq!(raw.customer_stage.as_deref(), Some("decision"),
            "顶层 snake_case customer_stage 须经 alias 吸收");
        assert_eq!(raw.intent_level.as_deref(), Some("high"),
            "顶层 snake_case intent_level 须经 alias 吸收");

        // camelCase 主名仍照常工作（rename_all 主形态不受 alias 影响）。
        let camel = r#"{"customerStage":"evaluation","intentLevel":"medium"}"#;
        let raw2: RawAgentDecision = serde_json::from_str(camel).expect("parse camel");
        assert_eq!(raw2.customer_stage.as_deref(), Some("evaluation"),
            "camelCase 主名仍须正常解析");
        assert_eq!(raw2.intent_level.as_deref(), Some("medium"));
    }
```

- [ ] **Step 2: 运行确认失败**

先 `cd "E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation"`（后续所有命令同一目录）。
Run: `cargo test --lib raw_decision_accepts_snake_case_stage_and_intent 2>&1 | tail -20`
Expected: FAIL —— snake_case 分支断言失败（`raw.customer_stage` 实得 `None`，因 rename_all=camelCase 下顶层 `customer_stage` 键不被 `customerStage` 字段接受）。camelCase 分支此刻已通过。

- [ ] **Step 3: 加 alias**

把 `src/agent/types.rs:461-462`：

```rust
    pub customer_stage: Option<String>,
    pub intent_level: Option<String>,
```

替换为：

```rust
    #[serde(alias = "customer_stage")]
    pub customer_stage: Option<String>,
    #[serde(alias = "intent_level")]
    pub intent_level: Option<String>,
```

（`rename_all="camelCase"` 让主名是 `customerStage`/`intentLevel`；`alias` **额外**接受 snake_case。两键都能命中，与初始画像路径 decision.rs:150-153 双形兜底对齐。）

- [ ] **Step 4: 运行确认单测通过**

Run: `cargo test --lib raw_decision_accepts_snake_case_stage_and_intent 2>&1 | tail -20`
Expected: PASS（snake_case + camelCase 两分支均通过）。

- [ ] **Step 5: 全 lib 测确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 6: Commit**

```bash
git commit -am "fix(agent): RawAgentDecision 顶层 customer_stage/intent_level 加 snake_case alias 容双形 (D-01 P3家族②)"
```

---

## Task 2: KB-03 —— is_verified 收窄为精确小写匹配（guards.rs）

**Files:**
- Modify: `src/agent/guards.rs:308-314`（`is_verified` 函数 doc + 比较表达式）
- Test: `src/agent/guards.rs:316-325`（既有 `mod tests` 内 `is_verified_matches_status` 扩断言）

**Interfaces:**
- Consumes: 无。
- Produces: 无对外接口变化（`is_verified(&str) -> bool` 签名不变，仅收窄判定口径）。

- [ ] **Step 1: 扩既有单测（先写，验证会失败——当前大小写不敏感）**

把 `src/agent/guards.rs:320-324` 的既有测试：

```rust
    #[test]
    fn is_verified_matches_status() {
        assert!(is_verified("verified"));
        assert!(is_verified("draft") == false);
    }
```

替换为：

```rust
    #[test]
    fn is_verified_matches_status() {
        assert!(is_verified("verified"));
        assert!(is_verified("draft") == false);
        // KB-03：口径收窄为精确小写匹配，与写入侧（verify.rs 恒写小写）+ 所有召回
        // 过滤（== "verified"）三方对齐。大写变体不再被硬闸宽松接受（回退到
        // eq_ignore_ascii_case 即变红）。
        assert!(is_verified("Verified") == false, "大写变体须精确不匹配(口径对齐召回侧)");
        assert!(is_verified("VERIFIED") == false);
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib is_verified_matches_status 2>&1 | tail -20`
Expected: FAIL —— `is_verified("Verified")` 当前用 `eq_ignore_ascii_case` 返 `true`，断言期望 `false`。

- [ ] **Step 3: 改比较表达式 + 更新 doc**

把 `src/agent/guards.rs:308-314`：

```rust
/// 命中 verified 语料判定：只要 chunk 的 status（大小写不敏感）等于 "verified"。
///
/// 注：写入侧 `verify.rs` 恒以小写 "verified" 落库，本函数用
/// `eq_ignore_ascii_case` 只是防御性冗余（当前不可能出现大小写变体）。
pub fn is_verified(status: &str) -> bool {
    status.eq_ignore_ascii_case("verified")
}
```

替换为：

```rust
/// 命中 verified 语料判定：chunk 的 status 精确等于小写 "verified"。
///
/// KB-03：口径与写入侧（`verify.rs` 恒写小写 "verified"）+ 所有召回过滤
/// （knowledge_router / knowledge_agent / chat / catalog 均 `== "verified"`
/// 精确匹配）三方统一。此前用 `eq_ignore_ascii_case` 大小写不敏感，与召回侧
/// 精确匹配口径漂移（latent：写入恒小写故当前不可触发）；收窄为精确匹配消除
/// 该漂移，对现有数据行为完全等价。
pub fn is_verified(status: &str) -> bool {
    status == "verified"
}
```

- [ ] **Step 4: 运行确认单测通过**

Run: `cargo test --lib is_verified_matches_status 2>&1 | tail -20`
Expected: PASS。

- [ ] **Step 5: 全 lib 测确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 6: Commit**

```bash
git commit -am "fix(agent): is_verified 收窄为精确小写匹配,消除与召回侧口径漂移 (KB-03 P3家族②)"
```

---

## Self-Review 结论

- **Spec coverage**：D-01（顶层标签键 alias）→ Task 1；KB-03（is_verified 口径收窄）→ Task 2。两条 finding 全覆盖。
- **Placeholder scan**：无 TBD/TODO，每步含完整可编译代码 + 精确命令 + 期望输出。
- **Type consistency**：Task 1 alias 只加属性、字段类型 `Option<String>` 不变；Task 2 `is_verified` 签名 `(&str)->bool` 不变。
- **既有测试冲击**：Task 1 纯新增 serde 单测，不改既有断言；Task 2 扩 `is_verified_matches_status` 既有测（append 两条大写断言，不改原两条）——大写变体行为是 KB-03 有意收窄的口径，反过拟合合规。
- **红线合规**：D-01 不动 rename_all、不顺手给其它字段加 alias（YAGNI）；KB-03 等价收窄不改写入/召回侧；无禁词。

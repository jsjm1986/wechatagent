# ⑨记忆冲突治上游 + ④reviewer 让位下沉 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让记忆 consolidator 真正产出带 id+dimension 的结构化 fact（激活同维冲突裁决主路径+兜底），并让独立 Review Agent 在 assist 模式下放行专属顾问名片引荐。

**Architecture:** ⑨ 走「治上游」——注入给 LLM 的当前卡先 auto_upgrade 带上稳定 id（注入与 prev-merge 同源），consolidator prompt 给 per-item 对象示例 + dimension 改口必填 + fact 原子化 + 跨轮命名复用引导；已有的 `deprecate_same_dimension_conflicts` 兜底在 dimension 真落地后自然 engage。④ 在 `review/mod.rs` 复用 `referral::assist_mode_active` 纯函数注入让位措辞，同解「第三方角色红线」+「factRisk 误判产品承诺」两条 hold 路径，不碰 gates.rs 硬闸阈值。

**Tech Stack:** Rust 2021 (Axum)、MongoDB (bson)、cargo test --lib + proptest PBT。

## Global Constraints

- 过拟合红线：绝不为过测试改业务逻辑/prompt/guards/阈值。沉淀可复现抽象方法论，不点对点修补单条对话/单次 CI 样本。
- agent-first：dimension 是 LLM 语义归类，**非关键词匹配**；裁决纯函数零关键词零 LLM。
- DEFAULT 字节等价：dimension=None 退回按 text 去重旧行为；assist 关账号让位段空串、reviewer system prompt + `prompts.rs:1576` 红线一字不动。
- check-no-human-takeover lint：④ 让位措辞用「专属顾问/增配/我仍在场辅助」，绝不出现「转人工/接管/第三方真人接手/hand-off」。lint 扫 `src/agent/`、`src/routes/`、`src/evolution/`、`frontend/src/` 新增行。
- prompts.rs 改动**必须** bump `PROMPT_PACK_VERSION`（`prompts.rs:15`），否则 DB 启动对齐不重 seed。
- 测试基线不回归：`cargo test --lib` ≥ 350/0；4 PBT（state_transition_pbt/memory_card_invariants/wiki_chunk_revision_pbt/llm_retry_jitter）累计 ≥ 33/0。
- 本地只跑 `cargo test --lib` + 单 PBT；完整集成 + 真模型走 server 117 / CI（磁盘纪律）。
- commit 须用户批准（本计划已整体获批执行）；只 `git add` 具名文件，绝不 `git add -A`。
- 真模型回归多 seed 变体验证泛化，不点对点调单条。

**基线 commit:** eb206d3（v2）+ 8ad6707（本 spec）。spec: `docs/superpowers/specs/2026-06-28-memory-conflict-and-reviewer-yield-design.md`。

---

## File Structure

| 文件 | 责任 | 改动性质 |
|---|---|---|
| `src/models.rs` | `MemoryCardTyped` 新增 `live_dimension_names()` helper（提取非空 dimension 名去重） | 纯新增方法 |
| `src/agent/memory.rs` | 注入前 auto_upgrade（保 id）+ 注入与 prev 同源 + 已有 dimension 名注入；纯函数单测 | 修改 consolidation 路径 + 加测试 |
| `src/prompts.rs` | consolidator task 模板：schema 对象示例 + dimension 改口必填 + fact 原子化；bump 版本；模板断言测试 | 修改 prompt 文本 + 加断言 |
| `src/agent/referral.rs` | 让位措辞共享常量 `REVIEWER_ASSIST_YIELD_NOTE`（reply + review 复用，避免两处漂移） | 纯新增常量 |
| `src/agent/review/mod.rs` | 复用 `assist_mode_active` 注入让位段；纯函数单测 | 修改 reviewer system 组装 + 加测试 |

**任务顺序与依赖**：Task 1（models helper）→ Task 2（memory 保 id 注入，消费 Task 1）→ Task 3（prompts schema/必填）→ Task 4（referral 常量）→ Task 5（review 注入，消费 Task 4）。Task 3/4 互不依赖可并行；Task 2 依赖 Task 1；Task 5 依赖 Task 4。

---

## Task 1: `MemoryCardTyped::live_dimension_names()` 提取已有维度名

**Files:**
- Modify: `src/models.rs`（`MemoryCardTyped` impl 块，紧邻 `auto_upgrade_plain_facts` 之后，约 `:4007`）
- Test: `src/models.rs`（同文件 `#[cfg(test)]` 模块，或新增内联测试）

**Interfaces:**
- Consumes: 现有 `MemoryFactRepr`（`models.rs:3725`）、`MemoryFact.dimension: Option<String>`（`models.rs:3824`）。
- Produces: `pub fn live_dimension_names(&self) -> Vec<String>` —— 返回 `core_facts` + `recent_facts` 中所有 Structured fact 的非空 dimension 名，去重、保持首次出现顺序。供 Task 2 跨轮命名稳定化注入使用。

- [ ] **Step 1: 写失败测试**

在 `src/models.rs` 找到现有 memory 相关测试模块（搜 `mod ` + `MemoryCardTyped`；若无则在文件末尾 `#[cfg(test)] mod memory_card_helpers_tests`）新增：

```rust
#[cfg(test)]
mod live_dimension_names_tests {
    use super::*;

    fn structured(id: &str, text: &str, dim: Option<&str>) -> MemoryFactRepr {
        let mut f = MemoryFact::from_plain_text(text.to_string());
        f.id = id.to_string();
        f.dimension = dim.map(|s| s.to_string());
        MemoryFactRepr::Structured(f)
    }

    #[test]
    fn collects_nonempty_dims_dedup_in_order() {
        let card = MemoryCardTyped {
            core_facts: vec![
                structured("a", "孩子10岁", Some("孩子年龄")),
                structured("b", "预算5000", Some("预算")),
                structured("c", "再提年龄", Some("孩子年龄")), // 重复
                structured("d", "纯字符串无维度", None),
            ],
            recent_facts: vec![structured("e", "决策人是妈妈", Some("决策角色"))],
            ..Default::default()
        };
        assert_eq!(
            card.live_dimension_names(),
            vec!["孩子年龄".to_string(), "预算".to_string(), "决策角色".to_string()]
        );
    }

    #[test]
    fn ignores_plain_and_blank_dims() {
        let mut blank = MemoryFact::from_plain_text("x".to_string());
        blank.dimension = Some("   ".to_string());
        let card = MemoryCardTyped {
            core_facts: vec![
                MemoryFactRepr::Plain("纯字符串".to_string()),
                MemoryFactRepr::Structured(blank),
            ],
            ..Default::default()
        };
        assert!(card.live_dimension_names().is_empty());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib live_dimension_names_tests`
Expected: 编译失败 `no method named live_dimension_names`。

- [ ] **Step 3: 实现 helper**

在 `src/models.rs` 的 `impl MemoryCardTyped` 块内（`auto_upgrade_plain_facts` 之后）新增：

```rust
        /// ⑨跨轮命名稳定化：提取 core_facts + recent_facts 中所有 Structured fact 的
        /// 非空 dimension 名（去重、保首次出现顺序）。供 consolidator prompt 注入
        /// 「已有维度名」引导 LLM 对同一属性沿用同名，缓解跨轮命名漂移。
        /// 仅 Structured 且 dimension 非空白参与；Plain / None / 空白 → 跳过。
        pub fn live_dimension_names(&self) -> Vec<String> {
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for repr in self.core_facts.iter().chain(self.recent_facts.iter()) {
                if let MemoryFactRepr::Structured(f) = repr {
                    if let Some(dim) = f.dimension.as_ref().filter(|d| !d.trim().is_empty()) {
                        if seen.insert(dim.clone()) {
                            out.push(dim.clone());
                        }
                    }
                }
            }
            out
        }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib live_dimension_names_tests`
Expected: 2 passed。

- [ ] **Step 5: Commit**

```bash
git add src/models.rs
git commit -m "feat(memory): MemoryCardTyped::live_dimension_names 提取已有维度名

⑨跨轮命名稳定化基础设施:去重保序提取Structured fact非空dimension名,
供Task2 consolidator prompt注入引导LLM同属性沿用同名。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: ⑨ 保 id 注入 + 注入与 prev 同源 + 已有维度名注入

**Files:**
- Modify: `src/agent/memory.rs`（`consolidate_contact_memory_inner` 内：注入点 `:1253`、user prompt 组装 `:1227-1248`、previous_card `:1312`）
- Test: `src/agent/memory.rs`（`r7_deprecation_tests` 模块加纯函数单测）

**Interfaces:**
- Consumes: Task 1 的 `MemoryCardTyped::live_dimension_names()`；现有 `effective_memory_card`（`memory.rs:105`）、`auto_upgrade_plain_facts`（`models.rs:3992`）。
- Produces: 注入给 LLM 的「当前 memoryCard」JSON 中 coreFacts/recentFacts 为带 id 的对象；user prompt 含「已有维度名」引导行（清单非空时）。`previous_card` 与注入用 card 为**同一升级后实例**。

**说明（为什么这样改）**：当前 `:1253` 注入 `effective_memory_card(&memory).to_document()` 是 DB 里的 Plain 字符串（无 id），`:1312` 又独立调一次 `effective_memory_card`。两处必须改成「升级一次、共用同一份」，否则 LLM 引用的 id 与 prev-merge 的 id 不一致（`from_plain_text` 每次 fresh UUID）。

- [ ] **Step 1: 写失败测试（纯函数：注入卡升级后带 id + 维度名提取）**

在 `src/agent/memory.rs` 的 `r7_deprecation_tests` 模块（`fact` helper 所在，约 `:2040` 后）新增。注意该模块已 `use crate::models::{MemoryCardTyped, MemoryFact, MemoryFactRepr};`：

```rust
    #[test]
    fn injected_card_upgraded_carries_ids_and_dims() {
        // 模拟注入前升级：Plain 字符串 → Structured 带 fresh id。
        let mut card = MemoryCardTyped {
            core_facts: vec![
                MemoryFactRepr::Plain("孩子8岁零基础".to_string()),
                MemoryFactRepr::Plain("预算5000".to_string()),
            ],
            ..Default::default()
        };
        let n = card.auto_upgrade_plain_facts();
        assert_eq!(n, 2, "两条 Plain 应被升级");
        for repr in &card.core_facts {
            match repr {
                MemoryFactRepr::Structured(f) => assert!(!f.id.is_empty(), "升级后必须带 id"),
                MemoryFactRepr::Plain(_) => panic!("不应残留 Plain"),
            }
        }
        // 升级来自 Plain → dimension 仍 None → 维度名清单为空（冷启动语义）。
        assert!(card.live_dimension_names().is_empty());
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib injected_card_upgraded_carries_ids_and_dims`
Expected: 若 Task 1 已合并则此测试**通过**（验证的是既有方法组合行为）。若想看红，先临时把断言 `n` 改 3 跑一次确认测试真在跑，再改回。记录：本测试是「同源升级」语义的回归锚，实现步骤在 Step 3 改生产代码。

- [ ] **Step 3: 改生产代码——注入与 prev 同源升级**

在 `src/agent/memory.rs` `consolidate_contact_memory_inner`：

3a. 在 user prompt 组装（`:1227` 的 `let user = format!(...)`）**之前**，新增升级后的注入卡计算。找到 `:1227` 上方（`tag_observations_json` 定义之后）插入：

```rust
    // ⑨治上游：注入给 LLM 的「当前 memoryCard」必须带稳定 id（让 LLM 有 id 可显式弃用旧 fact），
    // 且与下方 prev-merge 用的 previous_card 同源（同一升级实例），否则 LLM 引用的 id 在合并时
    // 匹配不上（from_plain_text 每次 fresh UUID）。历史 Plain 字符串在此一次性升级为 Structured。
    let mut injected_card = effective_memory_card(&memory);
    injected_card.auto_upgrade_plain_facts();
    // 跨轮命名稳定化：把当前卡里已有的 dimension 名告知 LLM，引导同属性沿用同名。
    // 冷启动（首轮全是 Plain 升级来 → dimension=None）→ 清单空 → 不注入该行（字节等价）。
    let existing_dim_names = injected_card.live_dimension_names();
    let existing_dims_line = if existing_dim_names.is_empty() {
        String::new()
    } else {
        format!(
            "\n已有维度名（同一属性请沿用下列名称，不要新造同义名）：[{}]\n",
            existing_dim_names.join(", ")
        )
    };
```

3b. 把 `:1253` 注入行的 `effective_memory_card(&memory)` 改为 `injected_card`：

```rust
        // task 6.3：prompt wire shape 仍是 Document JSON；典型用 injected_card（已升级带 id）。
        serde_json::to_string(&injected_card.to_document()).unwrap_or_default(),
```

3c. 在 user prompt 模板里追加 `existing_dims_line`。`:1227` 的 `format!` 末尾（`待重判标签观察...{}` 之后、闭合 `"#` 之前）增加一个 `{}` 占位，并在参数列表末尾加 `existing_dims_line`。具体：模板字符串「待重判标签观察（线索，需对话佐证才保留）:\n{}\n」之后加一行 `{}`，参数列表 `tag_observations_json` 之后补 `existing_dims_line`。

3d. 把 `:1312` 的 `let previous_card = effective_memory_card(&memory);` 改为复用同源升级卡：

```rust
    // ⑨治上游：prev-merge 用与注入同一份升级后的卡（id 一致），保证 LLM 引用的 id 命中。
    let previous_card = injected_card.clone();
```

（注：`injected_card` 在 3a 是 `let mut`，3c 注入用 `&injected_card.to_document()` 不消耗所有权，故 3d clone 合法。若借用检查报错，把 3a 的注入 JSON 提前算成 `let injected_card_json = serde_json::to_string(&injected_card.to_document())...;` 再在 format! 用 `injected_card_json`，previous_card 用 `injected_card`。）

- [ ] **Step 4: 运行测试 + 编译**

Run: `cargo test --lib injected_card_upgraded_carries_ids_and_dims` 然后 `cargo build --lib`
Expected: 测试 pass；编译无 error（重点看 3d 借用检查）。

- [ ] **Step 5: 跑记忆相关 PBT 确认不回归**

Run: `cargo test --lib --test memory_card_invariants` 若该 PBT 在 lib 内则 `cargo test --lib pbt_same_dimension`
Expected: 全 pass。

- [ ] **Step 6: Commit**

```bash
git add src/agent/memory.rs
git commit -m "feat(memory): ⑨保id注入+注入与prev同源+已有维度名引导

注入给consolidator的当前卡先auto_upgrade带稳定id(LLM据此显式弃用旧fact),
与prev-merge共用同一升级实例(id一致防匹配不上);非空dimension名清单注入
引导LLM同属性沿用同名(冷启动空清单不注入,字节等价)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: ⑨ consolidator prompt——schema 对象示例 + dimension 改口必填 + fact 原子化

**Files:**
- Modify: `src/prompts.rs`（consolidator task 模板 `:1432` schema、`:1469-1478` 限制段、`:1474` dimension 行；版本号 `:15`）
- Test: `src/prompts.rs`（现有 consolidator 模板断言测试，约 `:2644`）

**Interfaces:**
- Consumes: 无新依赖。
- Produces: DB seed 的 `user.memory_consolidator.task` 模板含 per-item 对象示例（带 id/text/dimension/importance）、fact 原子化要求、dimension「改口必填」措辞。`PROMPT_PACK_VERSION` 递增触发重 seed。

- [ ] **Step 1: 写失败测试（模板断言）**

在 `src/prompts.rs` 找到现有 consolidator 断言测试（搜 `user.memory_consolidator.task` + `discardedTags`，约 `:2644`），在其所在 `#[test]` 函数后新增：

```rust
    #[test]
    fn consolidator_schema_has_structured_fact_shape_and_dimension_required() {
        let specs = prompt_specs();
        let task = specs
            .iter()
            .find(|s| s.key == "user.memory_consolidator.task")
            .expect("user.memory_consolidator.task prompt spec 存在");
        // schema 给出 per-item 对象示例（含 dimension 键），而非空数组。
        assert!(
            task.content.contains("\"dimension\""),
            "coreFacts schema 须给带 dimension 的对象示例,否则 LLM 倾向吐字符串"
        );
        // fact 原子化要求（直接针对累积巨型 summary 根因）。
        assert!(
            task.content.contains("只讲一个事实"),
            "须要求 fact 原子化(一条只讲一个事实)"
        );
        // dimension 改口必填(镜像⑥决策墙手法,不是"可选")。
        assert!(
            task.content.contains("改口") && task.content.contains("必须"),
            "改口/更正场景须把 dimension 升为必填"
        );
    }
```

（注：`default_prompt_pack` 是返回 `Vec<PromptSpec>` 的函数名。若实际函数名不同，先 `grep "fn .*prompt_pack" src/prompts.rs` 确认——现有断言测试已用同一访问方式，照搬它的取法。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib consolidator_schema_has_structured_fact_shape_and_dimension_required`
Expected: FAIL（当前 `:1432` 是 `"coreFacts": []` 空数组，无 dimension 键；无「只讲一个事实」「改口...必须」措辞）。

- [ ] **Step 3a: 改 schema 给对象示例**

`src/prompts.rs:1432`，把：

```rust
    "coreFacts": [],
    "recentFacts": [],
```

改为：

```rust
    "coreFacts": [
      { "id": "沿用「当前 memoryCard」里该条 fact 的 id；新事实留空字符串由系统生成", "text": "一条只讲一个事实的原子陈述（一个属性/一个数值/一个角色）", "dimension": "该事实的语义维度名（如 孩子年龄/预算/决策角色），同一属性跨轮沿用同名", "importance": 8 }
    ],
    "recentFacts": [
      { "id": "", "text": "近期事实，结构同 coreFacts", "dimension": "可留空", "importance": 5 }
    ],
```

- [ ] **Step 3b: 加 fact 原子化要求 + dimension 改口必填**

`src/prompts.rs:1474`，把：

```rust
- 每条 fact 可选带 dimension 字段：对这条事实做语义维度归类（如客户的某个稳定属性维度）。当客户改口 / 更正某维度的旧信息时，给新 fact 标同一 dimension——系统会自动让该维度的旧值退出生效层（你不必手动把旧值列进 discarded）。同一维度同时只应保留一条生效 fact。
```

改为：

```rust
- 每条 fact 必须原子化：只讲一个事实（一个属性 / 一个数值 / 一个角色），不要把多个事实揉进一条 summary 式长句（否则系统无法对单个事实做冲突裁决）。
- dimension 字段：对这条事实做语义维度归类（如 孩子年龄 / 预算 / 决策角色）。当本轮出现对某属性的改口 / 更正（典型：年龄、预算、决策角色变化）时，新 fact 必须带 dimension 字段标注该属性维度，且与被更正的旧 fact 用同一 dimension 名——系统据此自动让该维度旧值退出生效层（你不必手动把旧值列进 discarded）。同一维度同时只应保留一条生效 fact。非改口场景的稳定属性也建议带 dimension。
```

- [ ] **Step 3c: bump 版本号**

`src/prompts.rs:15`，把 `PROMPT_PACK_VERSION` 末尾追加新标记，例如：

```rust
pub const PROMPT_PACK_VERSION: &str = "wechatagent_prompt_pack_v16_2026_06_28_memory_structured_fact_and_dimension_required";
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib consolidator_schema_has_structured_fact_shape_and_dimension_required`
Expected: PASS。

- [ ] **Step 5: 跑既有 consolidator 模板断言 + 全 prompts 测试不回归**

Run: `cargo test --lib prompts`
Expected: 全 pass（含既有 discardedTags 断言）。

- [ ] **Step 6: Commit**

```bash
git add src/prompts.rs
git commit -m "feat(prompts): ⑨consolidator强制结构化fact+dimension改口必填+原子化

coreFacts schema从空数组改带{id,text,dimension,importance}对象示例;fact原子化
(一条只讲一个事实,针对累积巨型summary根因);dimension从'可选'升为改口场景必填
(镜像⑥决策墙手法,A/B已证可选字段被无视);bump PROMPT_PACK_VERSION触发重seed。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: ④ 让位措辞共享常量

**Files:**
- Modify: `src/agent/referral.rs`（文件顶部，紧邻 `assist_mode_active` `:10`）
- Test: `src/agent/referral.rs`（同文件测试模块）

**Interfaces:**
- Consumes: 无。
- Produces: `pub(crate) const REVIEWER_ASSIST_YIELD_NOTE: &str` —— reviewer system prompt 的 assist 让位段文本，供 Task 5 注入。措辞同时覆盖两条 hold 路径，且过 check-no-human-takeover lint。

**说明**：抽成 referral.rs 常量是因为 referral 模块**不在** check-no-human-takeover lint 扫描目录外？——实际 `src/agent/` 在扫描内。故措辞本身必须合规（用「专属顾问/增配/我仍在场辅助」，不出现禁词）。放 referral.rs 是因为该模块已是引荐逻辑的归属地，且 reply 侧（decision.rs）未来可复用同一常量避免漂移。

- [ ] **Step 1: 写失败测试**

在 `src/agent/referral.rs` 测试模块（搜 `#[cfg(test)]`）新增：

```rust
    #[test]
    fn reviewer_assist_yield_note_covers_two_paths_and_passes_lint() {
        let note = super::REVIEWER_ASSIST_YIELD_NOTE;
        // 解路径①：引荐不属于「第三方角色失约」红线。
        assert!(note.contains("专属顾问"));
        assert!(note.contains("让位") || note.contains("不属于"));
        // 解路径②：引荐不是产品能力声明,不计入 hallucination/产品准确度。
        assert!(note.contains("不是产品") || note.contains("不计入"));
        // 过 check-no-human-takeover lint:不含禁词。
        for banned in ["人工接管", "接管", "转人工", "hand-off", "handoff", "人工介入"] {
            assert!(!note.contains(banned), "让位措辞含禁词: {banned}");
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib reviewer_assist_yield_note_covers_two_paths_and_passes_lint`
Expected: 编译失败 `cannot find value REVIEWER_ASSIST_YIELD_NOTE`。

- [ ] **Step 3: 新增常量**

在 `src/agent/referral.rs` `assist_mode_active`（`:10`）**之前**新增：

```rust
/// ④reviewer 让位段（辅助模式下注入 reviewer system prompt）。同时消解两条 hold 路径：
/// ① 引荐专属顾问不属于「除『我』外不得出现人类角色」红线所禁——红线在引荐这一受控动作上让位；
/// ② 引荐不是产品能力声明（不涉产品功能/价格/效果）→ 不计入 hallucination/产品准确度评分。
/// 措辞红线：用「专属顾问/增配/我仍在场辅助」，过 check-no-human-takeover lint。
/// assist 关账号不注入此段（reviewer system prompt 字节等价）。
pub(crate) const REVIEWER_ASSIST_YIELD_NOTE: &str = "\n【受控例外·辅助模式】本账号已开启辅助模式。在辅助模式下，AI 主动为契合条件的客户增配一位「专属顾问」（决策里的 namecardToSend）是本账号的受控业务动作，定位是「我为你增配一位更专属的顾问、我仍在场辅助答疑」。评审时请注意两点：① 这一引荐动作不属于「对话中除『我』之外不得出现任何人类角色」红线所禁止的范畴，该红线在「引荐专属顾问」上让位，不要因此判 needs_revision；② 引荐专属顾问不是对产品能力/价格/效果的声明，不应计入 hallucination/产品准确度评分（不要据此抬高 factRisk）。仅当客户真正契合（明确要签约/到店/深度对接）时才引荐，不为引荐而引荐。\n";
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib reviewer_assist_yield_note_covers_two_paths_and_passes_lint`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src/agent/referral.rs
git commit -m "feat(referral): ④reviewer让位措辞共享常量REVIEWER_ASSIST_YIELD_NOTE

辅助模式reviewer让位段:解两条hold路径(①引荐不属第三方角色红线②引荐非产品声明
不计hallucination);措辞用专属顾问/增配/我仍在场辅助过check-no-human-takeover lint;
抽常量供reply+review复用避免漂移。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: ④ review/mod.rs 注入让位段（assist_on 条件）

**Files:**
- Modify: `src/agent/review/mod.rs`（`review_decision`，system prompt 组装 `:287-305`）
- Test: `src/agent/review/mod.rs`（纯函数单测）

**Interfaces:**
- Consumes: Task 4 的 `referral::REVIEWER_ASSIST_YIELD_NOTE`；现有 `referral::assist_mode_active`（`referral.rs:10`）、`crate::models::ASSIST_MODE_OVERRIDE_ATTR`、`review_decision` 已有入参 `contact`（`:255`）+ `domain_config`（`:259`）。
- Produces: assist_on 时 reviewer system prompt 末尾追加 `REVIEWER_ASSIST_YIELD_NOTE`；assist 关时字节等价。

- [ ] **Step 1: 写失败测试（纯函数：让位段拼接逻辑）**

让位拼接逻辑抽成可测纯函数。在 `src/agent/review/mod.rs` 文件内（`review_decision` 之外）新增纯函数 + 测试：

```rust
/// ④reviewer 让位：assist_on 时在 reviewer system prompt 末尾追加让位段，否则原样返回。
/// 纯函数便于单测;DEFAULT(assist 关)字节等价。
fn append_assist_yield(system: String, assist_on: bool) -> String {
    if assist_on {
        format!("{system}{}", crate::agent::referral::REVIEWER_ASSIST_YIELD_NOTE)
    } else {
        system
    }
}

#[cfg(test)]
mod assist_yield_tests {
    use super::append_assist_yield;

    #[test]
    fn assist_off_is_byte_identical() {
        let base = "原始 reviewer system prompt".to_string();
        assert_eq!(append_assist_yield(base.clone(), false), base);
    }

    #[test]
    fn assist_on_appends_yield_note() {
        let base = "原始 reviewer system prompt".to_string();
        let out = append_assist_yield(base.clone(), true);
        assert!(out.starts_with(&base), "让位段追加在末尾,不改原文");
        assert!(out.contains("专属顾问"));
        assert!(out.len() > base.len());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib assist_yield_tests`
Expected: 编译失败 `cannot find function append_assist_yield`（先放函数再放测试可让 Step 1 自身编译，但生产调用点未接，故 Step 3 才接线）。若 Step 1 函数已写则测试直接 PASS——这是纯函数单测，红在「生产调用未接」由 Step 4 编译验证。

- [ ] **Step 3: 生产接线——review_decision 注入**

`src/agent/review/mod.rs`，在 `:302-305` 的 `apply_review_system_prompt_overrides` 之后接让位。找到：

```rust
    let system = crate::agent::domain_profile::apply_review_system_prompt_overrides(
        &system,
        &active_profile,
    );
```

之后新增：

```rust
    // ④reviewer 让位下沉：辅助模式下,reviewer 须知「引荐专属顾问」是受控业务动作,
    // 解两条 hold 路径(第三方角色红线 + 误判产品承诺抬 factRisk)。assist 关账号字节等价。
    // assist 判定复用 reply 侧同一纯函数(referral::assist_mode_active),客户级 override > 账号级。
    let assist_override = contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str(crate::models::ASSIST_MODE_OVERRIDE_ATTR).ok());
    let assist_on = crate::agent::referral::assist_mode_active(
        domain_config.and_then(|c| c.assist_mode_enabled),
        assist_override,
    );
    let system = append_assist_yield(system, assist_on);
```

- [ ] **Step 4: 运行测试 + 编译**

Run: `cargo test --lib assist_yield_tests && cargo build --lib`
Expected: 测试 pass；编译无 error（重点：`domain_config` 是 `Option<&OperationDomainConfig>`，`.and_then(|c| c.assist_mode_enabled)` 中 `assist_mode_enabled` 字段类型须为 `Option<bool>`——与 decision.rs:375 用法一致，若报类型错对照 decision.rs 同行修正）。

- [ ] **Step 5: check-no-human-takeover lint 自查**

Run（Windows PowerShell）: `scripts/check-no-human-takeover.ps1`
Expected: PASS（新增行措辞合规）。若脚本需 git diff，先确保改动已 `git add`（见 Step 6 先 add 再跑，或脚本对工作树扫描）。

- [ ] **Step 6: Commit**

```bash
git add src/agent/review/mod.rs
git commit -m "feat(review): ④reviewer让位下沉(assist_on注入,解两条hold路径)

review_decision在system prompt组装后复用referral::assist_mode_active判定,
assist_on时append REVIEWER_ASSIST_YIELD_NOTE(解第三方角色红线+引荐误判产品承诺
抬factRisk两路);assist关账号字节等价。append_assist_yield纯函数+单测。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6: 全量基线验证 + 真模型回归（server 117，多 seed）

**Files:**
- 无代码改动（验证任务）。可能 Modify: `scripts/biz-test/batch_a_domain9.py` / `batch_a_domain4.py`（仅当需加 seed 变体，append 不改旧断言）。

**Interfaces:**
- Consumes: Task 1-5 全部改动。
- Produces: 基线门绿 + 真模型证据（⑨ 8岁退出生效层、④ namecard 入 outbox）。

- [ ] **Step 1: 本地基线门**

Run:
```bash
cargo test --lib 2>&1 | tail -5
```
Expected: `≥ 350 passed; 0 failed`。

- [ ] **Step 2: 4 PBT 累计**

Run:
```bash
cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter 2>&1 | grep "test result"
```
Expected: 累计 ≥ 33 passed, 0 failed。（若磁盘紧，逐个跑。）

- [ ] **Step 3: cargo check --tests 复刻 CI step**

Run: `cargo check --tests 2>&1 | tail -3`
Expected: 无 error（防 AppConfig 等构造点漏补——见 memory `config_field_add_test_helpers`；本计划未加 config 字段，预期直接过）。

- [ ] **Step 4: 部署 server 117（git bundle 或 push+fetch，见上轮 Task9 流程）**

部署 commit = Task 5 之后的 HEAD。重编译 `cargo build --release` + 重启 service wechatagent，确认 HTTP 200 + DB prompt 经 align_prompt_specs 重 seed 到 v16（验 prompt 直接 dump 原文目视，**勿用 mongosh --eval 的 indexOf 布尔判断**——跨 SSH 多层引号转义破坏中文）。

- [ ] **Step 5: ⑨ 真模型回归（主 seed + 1 变体）**

```bash
DEPLOY_PASS=... python scripts/biz-test/batch_a_domain9.py
```
Expected: A 阶段 8岁进生效层；B 阶段改口 10岁 → 8岁退出生效层（`not age8_live` PASS）+ 进 deprecatedFacts 或 memory_conflict_resolved 事件。
变体（防过拟合，新建或参数化）：预算 3000→8000 改口，验证同机制泛化，非点对点。判 v 行为看 decision/memoryCard 实体非只看 status；端点污染（llm_tool_use glitch）→ reset + 单发隔离重测。

- [ ] **Step 6: ④ 真模型回归**

```bash
DEPLOY_PASS=... python scripts/biz-test/batch_a_domain4.py
```
Expected: 路径1 assist 关不发卡；路径2 assist 开 + 签约意向 → namecard 入 outbox（`has_card` PASS）。reviewer 不再用第三方角色红线拦截引荐。

- [ ] **Step 7: 记录结果到 memory + 更新 spec 的「Task9 ④⑨ 真测待续」状态**

把真测结论（含任何残留 LLM 不确定性观测）写入 memory `project_structured_field_gap_fix_v2.md` 续记。绝不为过测试改业务逻辑——端点失败标 BLOCKED 不假绿。

---

## Self-Review

**1. Spec coverage:**
- spec §3.1 保 id 注入 → Task 2 ✓
- spec §3.2 schema 对象示例 + dimension 改口必填 + 原子化 → Task 3 ✓
- spec §3.3 跨轮命名稳定化（+冷启动语义）→ Task 1（helper）+ Task 2（注入）✓
- spec §3.4 兜底 engage → 已存在代码，Task 5/6 真测验证 engage ✓（无需新代码，spec 明确「已实现已接入」）
- spec §4.1-4.3 ④ reviewer 让位（注入点/条件/不碰硬闸）→ Task 4 + Task 5 ✓
- spec §5 测试 → 各 Task 的 TDD 步 + Task 6 ✓
- spec §6 变更文件清单 → 与本计划 File Structure 一致 ✓（spec 提「decision.rs 可能抽共享常量」→ 本计划放 referral.rs 更合理，已在 Task 4 说明偏离理由）

**2. Placeholder scan:** 无 TBD/TODO；所有 code step 含完整代码；prompt 措辞逐字给出。Task 3 Step 1 标注了「若函数名不同先 grep」的实操指引（非占位，是防御性核对）。

**3. Type consistency:**
- `live_dimension_names(&self) -> Vec<String>`：Task 1 定义、Task 2 消费 ✓
- `REVIEWER_ASSIST_YIELD_NOTE: &str`：Task 4 定义、Task 5 消费 ✓
- `assist_mode_active(Option<bool>, Option<&str>) -> bool`：referral.rs:10 既有、Task 5 复用（与 decision.rs:374 同签名）✓
- `append_assist_yield(String, bool) -> String`：Task 5 内部定义+消费 ✓

发现并修正：Task 4 说明里原写「referral 不在 lint 扫描目录外」表述拗口——已在常量 doc 注释明确「措辞本身必须合规」，措辞实体不含禁词，测试 Step 1 已断言。

---

## Execution Handoff

计划已存 `docs/superpowers/plans/2026-06-28-memory-conflict-and-reviewer-yield.md`。

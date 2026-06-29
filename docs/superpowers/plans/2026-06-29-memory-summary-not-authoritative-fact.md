# ⑨记忆固化真因修正(memory_summary 不当权威事实) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `memory_summary`（短期滚动上下文）不再被 `memory_card_from_contact` 当权威 core_fact 注入种子卡，改归位到 `extra.recentEpisodeSummary`，根治客户改口后旧值累积成 blob 并存于事实层。

**Architecture:** 单文件单函数极小改动。落点 `src/agent/memory.rs` 的 `memory_card_from_contact`（pub(crate) 纯函数）。删一行 push（memory_summary→core_facts）、改一处 insert（recentEpisodeSummary 注入 memory_summary）。纯字段归位，不动累积逻辑/consolidator prompt/identity 回落/tags 注入。

**Tech Stack:** Rust 2021；cargo test --lib；BSON Document（mongodb crate）。

## Global Constraints

- **agent-first**：纯字段归位（删 1 行 + 改 1 行），零关键词、零数值实体提取、零语义裁决。
- **不改 memory_summary 累积逻辑**（gateway.rs:4044/3581）、不改 consolidator prompt、不碰硬闸阈值。
- **不动**：memory.rs:200 identity 回落、memory.rs:216 human_profile_note 进 core_facts、memory.rs:218-229 manual_tags/confirmed_tags 注入。
- **新增测试只增量 append**，绝不删改现有测试/断言。
- 基线不回归：cargo test --lib ≥ 350/0；4 PBT 累计 ≥ 33/0；check-baseline + check-no-human-takeover + check-evolution-isolation 三 lint 绿；RUSTFLAGS=-D warnings cargo check --tests 通过。
- 本地磁盘紧/编译慢时优先跑指定测试名 + `cargo build --lib`；全量 `--lib` 跑不动则在报告标注，不假绿。集成测试（tests/ 需 Docker）留 CI，不本地跑。

---

### Task 1: memory_card_from_contact 分层修正 + 4 单测

**Files:**
- Modify: `src/agent/memory.rs`（`memory_card_from_contact` 函数体内：删 line 215、改 line 279）
- Test: `src/agent/memory.rs` 的 `#[cfg(test)] mod r7_deprecation_tests`（与既有同位置，尾部 append 4 个单测）

**Interfaces:**
- Consumes（既有，签名已核实）：
  - `pub(crate) fn memory_card_from_contact(contact: &Contact, memory: &OperatingMemory, initial_state: &str) -> MemoryCardTyped`
  - `Contact { memory_summary: Option<String>, human_profile_note: Option<String>, manual_tags: Vec<String>, confirmed_tags: Vec<ConfirmedTag>, .. }`，`#[derive(Default)]`
  - `OperatingMemory`，`#[derive(Default)]`
  - `MemoryFactRepr::as_text(&self) -> &str`（models.rs:3822）
  - `fn doc_string(doc: &Document, key: &str) -> Option<String>`（memory.rs 内既有 helper，memory.rs:169 已用 `doc_string(&card.extra, "recentEpisodeSummary")`）
  - `MemoryCardTyped { core_facts: Vec<MemoryFactRepr>, recent_facts, deprecated_facts, extra: Document }`
- Produces：行为变更——`memory_card_from_contact` 返回的 `core_facts` 不再含 `contact.memory_summary`；`extra.recentEpisodeSummary` 承接 `contact.memory_summary`。

- [ ] **Step 1: 写失败测试（4 个，append 到 r7_deprecation_tests 模块尾部 `}` 之前）**

```rust
    // ⑨真因修正：memory_summary(短期滚动上下文)不再当权威 core_fact,归位 recentEpisodeSummary。
    #[test]
    fn memory_summary_blob_not_in_core_facts_goes_to_recent_episode() {
        use crate::agent::memory::memory_card_from_contact;
        use crate::models::{Contact, OperatingMemory};
        // 改口累积 blob:多行各版本 summary(模拟真测 contact.memory_summary 现场)。
        let blob = "孩子8岁零基础\n更新为10岁\n确认8岁";
        let contact = Contact {
            wxid: "biztest_c9".into(),
            memory_summary: Some(blob.to_string()),
            ..Default::default()
        };
        let memory = OperatingMemory::default();
        let card = memory_card_from_contact(&contact, &memory, "new_contact");
        // 权威事实层不含累积 blob。
        let core_texts: Vec<&str> = card.core_facts.iter().map(|f| f.as_text()).collect();
        assert!(
            !core_texts.iter().any(|t| t.contains("更新为10岁")),
            "memory_summary blob 不应进 core_facts(权威事实层),实际={core_texts:?}"
        );
        // 内容归位到 recentEpisodeSummary。
        let recent = crate::agent::memory::doc_string(&card.extra, "recentEpisodeSummary")
            .unwrap_or_default();
        assert_eq!(recent, blob, "memory_summary 应归位 extra.recentEpisodeSummary");
    }

    #[test]
    fn authoritative_facts_still_in_core_facts() {
        use crate::agent::memory::memory_card_from_contact;
        use crate::models::{Contact, OperatingMemory};
        let contact = Contact {
            wxid: "c".into(),
            human_profile_note: Some("VIP 客户".to_string()),
            manual_tags: vec!["家长".to_string()],
            ..Default::default()
        };
        let card = memory_card_from_contact(&contact, &OperatingMemory::default(), "new_contact");
        let core_texts: Vec<&str> = card.core_facts.iter().map(|f| f.as_text()).collect();
        assert!(core_texts.iter().any(|t| *t == "VIP 客户"), "human_profile_note 应留 core_facts");
        assert!(core_texts.iter().any(|t| *t == "家长"), "manual_tags 应留 core_facts");
    }

    #[test]
    fn identity_fallback_to_memory_summary_unchanged() {
        use crate::agent::memory::memory_card_from_contact;
        use crate::models::{Contact, OperatingMemory};
        // human_profile_note 为空 → identity 回落 memory_summary(单值画像字段,不变)。
        let contact = Contact {
            wxid: "c".into(),
            memory_summary: Some("张三老板".to_string()),
            ..Default::default()
        };
        let card = memory_card_from_contact(&contact, &OperatingMemory::default(), "new_contact");
        let identity = card
            .extra
            .get_document("coreProfile")
            .ok()
            .and_then(|d| d.get_str("identity").ok().map(|s| s.to_string()))
            .unwrap_or_default();
        assert_eq!(identity, "张三老板", "identity 单值回落 memory_summary 应保留");
    }

    #[test]
    fn empty_memory_summary_recent_episode_is_blank() {
        use crate::agent::memory::memory_card_from_contact;
        use crate::models::{Contact, OperatingMemory};
        let contact = Contact { wxid: "c".into(), ..Default::default() };
        let card = memory_card_from_contact(&contact, &OperatingMemory::default(), "new_contact");
        let recent = crate::agent::memory::doc_string(&card.extra, "recentEpisodeSummary")
            .unwrap_or_default();
        assert_eq!(recent, "", "无 memory_summary 时 recentEpisodeSummary 空串(字节等价原行为)");
    }
```

- [ ] **Step 2: 跑测试确认失败（验证当前行为：blob 进了 core_facts、recentEpisodeSummary 是空串）**

Run: `cargo test --lib memory_summary_blob_not_in_core_facts authoritative_facts_still identity_fallback empty_memory_summary`
Expected: `memory_summary_blob_not_in_core_facts_goes_to_recent_episode` FAIL（当前 blob 进了 core_facts、recentEpisodeSummary 是空串）；其余可能已 PASS（identity/authoritative 行为本就对）。关键是第一个测试必须 FAIL，证明它抓到了当前缺陷。

> 注：`cargo test --lib` 只接受一个 filter 子串。逐个跑或用共同子串。若 4 个测试名无共同子串，分多次跑：`cargo test --lib memory_summary_blob`、`cargo test --lib authoritative_facts`、`cargo test --lib identity_fallback`、`cargo test --lib empty_memory_summary`。

- [ ] **Step 3: 实现修正（memory.rs:memory_card_from_contact 函数体内两处）**

改动 1——删除 line 215（memory_summary 不再进 core_facts）：
```rust
    // 删除这一行:
    //   push_unique_text(&mut core_facts, contact.memory_summary.as_deref());
    // 保留下一行(human_profile_note 是运营手写权威画像,留 core_facts):
    push_unique_text(&mut core_facts, contact.human_profile_note.as_deref());
```
即把原来的：
```rust
    let mut core_facts: Vec<String> = Vec::new();
    push_unique_text(&mut core_facts, contact.memory_summary.as_deref());
    push_unique_text(&mut core_facts, contact.human_profile_note.as_deref());
```
改成（删掉 memory_summary 那行）：
```rust
    let mut core_facts: Vec<String> = Vec::new();
    // ⑨真因修正:memory_summary 是短期滚动上下文(gateway append 累积),不当权威 core_fact;
    // 归位到下方 extra.recentEpisodeSummary。human_profile_note(运营手写)仍是权威事实。
    push_unique_text(&mut core_facts, contact.human_profile_note.as_deref());
```

改动 2——line 279 recentEpisodeSummary 承接 memory_summary：
```rust
    // 原:
    //   extra.insert("recentEpisodeSummary", "");
    // 改:
    extra.insert(
        "recentEpisodeSummary",
        contact.memory_summary.clone().unwrap_or_default(),
    );
```

- [ ] **Step 4: 跑测试确认全 PASS**

Run: `cargo test --lib memory_summary_blob`（+ 其余三个子串，同 Step 2 跑法）
Expected: 4 个测试全 PASS。

- [ ] **Step 5: 编译 + 基线不回归**

Run: `cargo build --lib`
Expected: 编译通过（无 warning，注意 `doc_string` 是否需 `pub(crate)` 可见——测试用 `crate::agent::memory::doc_string`，若它当前是私有 `fn` 需在本任务内提升为 `pub(crate) fn`，这是测试可达性要求，属本改动一部分）。

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed（含新增 4 测，预期 1697/0 量级）。本地磁盘紧跑不动全量则跑指定测试名 + build --lib，并在报告标注 BLOCKED 不假绿。

- [ ] **Step 6: 提交**

```bash
git add src/agent/memory.rs
git commit -m "fix(memory): ⑨真因修正——memory_summary不当权威core_fact,归位recentEpisodeSummary

server117全量真测证件一件二未触及真因:memory_summary(短期滚动上下文)被
memory_card_from_contact:215当权威core_fact注入,逐轮累积8段summary成blob致
8岁/10岁并存。删该push,内容归位extra.recentEpisodeSummary。纯字段归位守
agent-first,不动累积逻辑/consolidator prompt/identity回落/tags注入。+4单测。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage**（对照 design §3 实现方案）：
- §3.1 改动1（删 line 215 memory_summary push）→ Task1 Step3 改动1 ✓
- §3.1 改动2（line 279 recentEpisodeSummary 注入 memory_summary）→ Task1 Step3 改动2 ✓
- §3.2 不动点（identity 回落 / human_profile_note / tags）→ Global Constraints + Step3 注释明示保留 ✓
- §5.1 四单测（改口 blob 不进 core_facts+归位 / 权威事实保留 / identity 回落 / 空不炸）→ Task1 Step1 四测 ✓
- §5.2 基线门 → Task1 Step5 + Global Constraints ✓

**2. Placeholder scan**：无 TBD/TODO；每个代码步骤含完整可编译代码；命令含预期输出。

**3. Type consistency**：`memory_card_from_contact` 签名、`MemoryFactRepr::as_text() -> &str`、`Contact`/`OperatingMemory` Default、`doc_string(&Document, &str) -> Option<String>`、`extra.get_document("coreProfile")` 均与 memory.rs:169/3822 + models.rs 既有用法一致。注意 Step5 标注的 `doc_string` 可见性提升（若私有）是测试可达性的必要部分。

**4. 一个潜在编译细节**：测试用 `crate::agent::memory::doc_string`——若 `doc_string` 当前是模块私有 `fn`（非 `pub(crate)`），测试在同 crate 的子模块 `r7_deprecation_tests` 内可用 `super::doc_string` 访问（同模块私有项对子模块可见）。实现者优先用 `super::doc_string` 避免改可见性；仅当 `super::` 不可达时才提升为 `pub(crate)`。Step1 测试代码统一可改用 `super::memory_card_from_contact` / `super::doc_string`（r7_deprecation_tests 是 memory 模块的子模块，`super::` 即 memory 模块），与既有件二测试的引用风格一致。

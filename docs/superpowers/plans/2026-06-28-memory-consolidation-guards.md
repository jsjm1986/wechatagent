# ⑨记忆固化确定性兜底两件套 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 consolidator 偶发降级（blob / 缺 dimension）不再导致客户改口后新旧值并存，用确定性纯函数兜底接住低频偶发降级。

**Architecture:** 两件互补的确定性改动，都在 `src/agent/memory.rs`。件一在 compact 救回循环加 dimension 感知（同 dimension 新值在场不救回旧值，治残余）。件二（方案 X）新增结构性非原子检测纯函数 `fact_is_non_atomic`（换行/句界/长度，零关键词），在 consolidator 调用方检测到 blob 时重试一次拿干净输出、仍失败则丢弃非原子条（治源头）。

**Tech Stack:** Rust 2021、cargo、serde_json（`Value`）、bson（`Document`）。无新依赖。

## Global Constraints

[以下为 spec 的项目级约束，每个任务都隐含包含，值逐字照抄自 spec]

- 不碰硬闸阈值（review/gates 的 factRisk/pressureRisk 等阈值一律不动）。
- 不改 consolidator prompt（`user.memory_consolidator.task` / `.system`，v4 探针证明 prompt 正常无缺陷）。
- 不拆分 consolidator（不新增 LLM 调用的常驻路径；重试仅在偶发降级时触发）。
- agent-first：非原子检测**只用通用结构度量**（换行数 / 句界数 / char 长度），**绝不提取数值实体或关键词**（"找 N岁"是关键词模式，违红线）。
- 新增测试只增量叠加，绝不删改旧维度 / 旧断言。
- 基线门不回归：`cargo test --lib` ≥ 350 passed / 0 failed；4 PBT（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥ 33 / 0。
- `dimension=None` 的 fact 在件一中维持原 text 去重行为（字节等价，不回归）。
- 重试至多 1 次（v4 探针证明 6/6 干净，1 次足够；不无限重试避免 token 失控）。
- 既成事实纪律：DB / 审计写失败只 `tracing::warn!`，不返 Err。
- 提交需用户显式批准；精确 `git add` 命名文件，不 `git add -A`，排除并行会话产物（`_lib.py` / `cleanup.py` 等）。

## File Structure

| 文件 | 责任 | 改动类型 |
|---|---|---|
| `src/agent/memory.rs` | 件一：`compact_memory_card_with_dimensions` 救回循环加 dimension 感知（memory.rs:386-399）；件二：新增纯函数 `fact_is_non_atomic` + `value_has_non_atomic_fact` + `consolidate_contact_memory_inner`（memory.rs:1286 后）检测重试逻辑 | Modify |
| `src/agent/memory.rs` `#[cfg(test)] mod tests` | 件一 4 单测 + 件二 8 单测 + 件二决策逻辑 3 测试（全部 append 到 lib 测试模块，函数保持 `pub(crate)` 不放大可见性） | Modify（append） |

> **决策：件二的检测对象是 serde_json `value` 还是 typed card？** 检测放在 `from_document` **之前**、扫原始 `value` 的 `memoryCard.coreFacts[].text` / `recentFacts[].text`。理由：重试要在最早点决策（避免做完 from_document/auto_upgrade 才发现要重试白做一遍）；且 blob 的特征在原始 text 字段最直接。`value_has_non_atomic_fact(&Value) -> bool` 扫这两个数组。

---

### Task 1: 件一 — compact 救回逻辑加 dimension 感知

**Files:**
- Modify: `src/agent/memory.rs:386-399`（`compact_memory_card_with_dimensions` 的 previous 救回循环）
- Test: `src/agent/memory.rs` `#[cfg(test)] mod tests`（append 4 单测）

**Interfaces:**
- Consumes: `MemoryCardTyped`（字段 `core_facts: Vec<MemoryFactRepr>`）、`MemoryFactRepr::Structured(MemoryFact)`（`MemoryFact.dimension: Option<String>`）、`MemoryFactRepr::as_text() -> &str`。这些都已存在。
- Produces: 无新公开签名（改的是 `compact_memory_card_with_dimensions` 内部循环逻辑，签名不变）。

**背景**：现状救回 previous 未 discarded 的 core_facts 时**仅按 `as_text()` 字符串相等**去重（memory.rs:392-395）。改口场景 "孩子8岁" ≠ "孩子10岁" → 旧值被救回 → 与新值并存。改动：救回前，若 previous 的该 fact 是 Structured 且带非空 dimension，而 incoming（`compact.core_facts`）已存在**同 dimension** 的 Structured fact，则**不救回**。

- [ ] **Step 1: 写失败测试（4 个）**

在 `src/agent/memory.rs` 的 `#[cfg(test)] mod tests` 内 append。先确认测试模块已有的构造 helper：用 `MemoryFactRepr::Structured(MemoryFact { ... })` 直接构造，`MemoryFact` 字段参考 `MemoryFact::from_plain_text` 的全字段（models.rs:3958）。为避免逐字段冗长，加一个测试内 helper：

```rust
    // ⑨件一：dimension 感知救回——同 dimension 新值在场时不救回旧值。
    fn structured_fact(text: &str, dim: Option<&str>) -> crate::models::MemoryFactRepr {
        use crate::models::{MemoryFact, MemoryFactRepr};
        let mut f = MemoryFact::from_plain_text(text.to_string());
        f.dimension = dim.map(|d| d.to_string());
        MemoryFactRepr::Structured(f)
    }

    #[test]
    fn recall_drops_old_value_when_same_dimension_new_value_present() {
        use crate::agent::memory::compact_memory_card_with_dimensions;
        use crate::agent::domain_profile::default_memory_dimensions;
        let mut incoming = default_memory_card();
        incoming.core_facts = vec![structured_fact("孩子10岁", Some("孩子年龄"))];
        let mut previous = default_memory_card();
        previous.core_facts = vec![structured_fact("孩子8岁", Some("孩子年龄"))];
        let out = compact_memory_card_with_dimensions(
            &incoming, Some(&previous), &[], &default_memory_dimensions(),
        );
        let texts: Vec<&str> = out.core_facts.iter().map(|f| f.as_text()).collect();
        assert!(texts.contains(&"孩子10岁"), "新值应在: {texts:?}");
        assert!(!texts.contains(&"孩子8岁"), "同 dimension 旧值不应被救回: {texts:?}");
    }

    #[test]
    fn recall_keeps_old_value_when_no_same_dimension_in_incoming() {
        use crate::agent::memory::compact_memory_card_with_dimensions;
        use crate::agent::domain_profile::default_memory_dimensions;
        let mut incoming = default_memory_card();
        incoming.core_facts = vec![structured_fact("预算5000", Some("预算"))];
        let mut previous = default_memory_card();
        previous.core_facts = vec![structured_fact("孩子8岁", Some("孩子年龄"))];
        let out = compact_memory_card_with_dimensions(
            &incoming, Some(&previous), &[], &default_memory_dimensions(),
        );
        let texts: Vec<&str> = out.core_facts.iter().map(|f| f.as_text()).collect();
        assert!(texts.contains(&"孩子8岁"), "无同 dimension 时旧值应正常救回: {texts:?}");
    }

    #[test]
    fn recall_none_dimension_keeps_text_dedup_behavior() {
        use crate::agent::memory::compact_memory_card_with_dimensions;
        use crate::agent::domain_profile::default_memory_dimensions;
        let mut incoming = default_memory_card();
        incoming.core_facts = vec![structured_fact("孩子10岁", None)];
        let mut previous = default_memory_card();
        previous.core_facts = vec![structured_fact("孩子8岁", None)];
        let out = compact_memory_card_with_dimensions(
            &incoming, Some(&previous), &[], &default_memory_dimensions(),
        );
        let texts: Vec<&str> = out.core_facts.iter().map(|f| f.as_text()).collect();
        // dimension=None → 维持原 text 去重：text 不等 → 两条都在（字节等价回归保护）
        assert!(texts.contains(&"孩子10岁") && texts.contains(&"孩子8岁"),
            "dimension=None 应维持原 text 去重(两条都留): {texts:?}");
    }

    #[test]
    fn recall_keeps_different_dimensions() {
        use crate::agent::memory::compact_memory_card_with_dimensions;
        use crate::agent::domain_profile::default_memory_dimensions;
        let mut incoming = default_memory_card();
        incoming.core_facts = vec![structured_fact("孩子10岁", Some("孩子年龄"))];
        let mut previous = default_memory_card();
        previous.core_facts = vec![structured_fact("预算3万", Some("预算"))];
        let out = compact_memory_card_with_dimensions(
            &incoming, Some(&previous), &[], &default_memory_dimensions(),
        );
        let texts: Vec<&str> = out.core_facts.iter().map(|f| f.as_text()).collect();
        assert!(texts.contains(&"孩子10岁") && texts.contains(&"预算3万"),
            "不同 dimension 不应互相误删: {texts:?}");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib recall_drops_old_value_when_same_dimension`
Expected: FAIL（`recall_drops_old_value...` 断言失败——当前旧值"孩子8岁"被救回，`!texts.contains("孩子8岁")` 不成立）。其余 3 个可能已 PASS（它们验证不回归），但 `recall_drops_old_value` 必须 FAIL，证明缺陷存在。

- [ ] **Step 3: 实现 dimension 感知救回**

修改 `src/agent/memory.rs:386-399` 的救回循环。在 push 前增加同 dimension 判定。替换整个 `if let Some(prev) = previous { ... }` 的 for 循环体（386-399 行）为：

```rust
    if let Some(prev) = previous {
        for fact in &prev.core_facts {
            let fact_text = fact.as_text();
            if discarded.iter().any(|d| d == fact_text) {
                continue;
            }
            // ⑨件一：dimension 感知救回。若该旧 fact 带非空 dimension，且 incoming
            // 已有同 dimension 的 Structured fact（新值已覆盖该维度），则不救回旧值
            // ——防 LLM 漏填 deprecatedFacts/discarded 时改口旧值被 text 不等救回致双值。
            // dimension=None 退回纯 text 去重（字节等价）。纯结构判定,零关键词零 LLM。
            if let MemoryFactRepr::Structured(prev_f) = fact {
                if let Some(prev_dim) = prev_f.dimension.as_ref().filter(|d| !d.trim().is_empty()) {
                    let incoming_has_same_dim = compact.core_facts.iter().any(|item| {
                        matches!(item, MemoryFactRepr::Structured(f)
                            if f.dimension.as_ref().map(|d| d.trim()) == Some(prev_dim.trim()))
                    });
                    if incoming_has_same_dim {
                        continue;
                    }
                }
            }
            if !compact
                .core_facts
                .iter()
                .any(|item| item.as_text() == fact_text)
            {
                compact.core_facts.push(fact.clone());
            }
        }
    }
```

确认文件顶部已 `use` 了 `MemoryFactRepr`（`deprecate_same_dimension_conflicts` 已用，memory.rs:488，故已在作用域；若编译报未导入则在函数内 `use crate::models::MemoryFactRepr;`）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib recall_drops_old_value_when_same_dimension recall_keeps_old_value_when_no_same_dimension recall_none_dimension_keeps_text_dedup recall_keeps_different_dimensions`
Expected: 4 PASS。

- [ ] **Step 5: 跑基线确认不回归**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed（尤其 `memory_card_invariants` 相关测试不回归——件一只在"带 dimension 且同 dimension"时改变行为，None 路径字节等价）。

- [ ] **Step 6: 提交**

```bash
git add src/agent/memory.rs
git commit -m "fix(memory): ⑨件一 compact救回加dimension感知,同dimension新值在场不救回旧值

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 件二A — 纯函数 fact_is_non_atomic + value_has_non_atomic_fact

**Files:**
- Modify: `src/agent/memory.rs`（新增两个纯函数，放在 `deprecate_same_dimension_conflicts` 附近，memory.rs:480 之前或之后的同区块）
- Test: `src/agent/memory.rs` `#[cfg(test)] mod tests`（append 8 单测）

**Interfaces:**
- Produces:
  - `pub(crate) fn fact_is_non_atomic(text: &str) -> bool` — 纯结构度量：text 含 ≥2 换行 `\n`，或 ≥2 句界标点（`。`/`！`/`？`/`;`），或 char 数 > 80。
  - `pub(crate) fn value_has_non_atomic_fact(value: &serde_json::Value) -> bool` — 扫 `value.memoryCard.coreFacts[].text` 与 `recentFacts[].text`（兼容 `memory_card` snake_case key），任一条 `fact_is_non_atomic` 即 true。Task 3 消费。

- [ ] **Step 1: 写失败测试（8 个）**

append 到 `#[cfg(test)] mod tests`：

```rust
    // ⑨件二：结构性非原子检测(零关键词,纯结构度量)。
    #[test]
    fn atomic_fact_normal_short_is_atomic() {
        use crate::agent::memory::fact_is_non_atomic;
        assert!(!fact_is_non_atomic("孩子10岁"));
        assert!(!fact_is_non_atomic("预算5000左右"));
    }

    #[test]
    fn atomic_fact_normal_slightly_long_not_misflagged() {
        use crate::agent::memory::fact_is_non_atomic;
        // 含 1 逗号、~13 字的正常稍长 fact 不应误判
        assert!(!fact_is_non_atomic("孩子10岁，零基础想报编程课"));
    }

    #[test]
    fn non_atomic_multiple_newlines() {
        use crate::agent::memory::fact_is_non_atomic;
        assert!(fact_is_non_atomic("孩子8岁\n更新为10岁\n确认8岁"));
    }

    #[test]
    fn non_atomic_multiple_sentence_breaks() {
        use crate::agent::memory::fact_is_non_atomic;
        assert!(fact_is_non_atomic("孩子8岁。预算5000。男孩。"));
    }

    #[test]
    fn non_atomic_over_length() {
        use crate::agent::memory::fact_is_non_atomic;
        let long = "客户".repeat(45); // 90 字,无换行无句界,仅靠长度命中
        assert!(fact_is_non_atomic(&long));
    }

    #[test]
    fn atomic_single_sentence_break_ok() {
        use crate::agent::memory::fact_is_non_atomic;
        // 单个句号(末尾)不触发(需 ≥2 句界)
        assert!(!fact_is_non_atomic("孩子10岁。"));
    }

    #[test]
    fn value_scan_detects_blob_in_corefacts() {
        use crate::agent::memory::value_has_non_atomic_fact;
        let v = serde_json::json!({
            "memoryCard": {
                "coreFacts": [
                    {"text": "孩子8岁\n更新为10岁\n确认8岁", "dimension": ""},
                    {"text": "预算5000", "dimension": "预算"}
                ],
                "recentFacts": []
            }
        });
        assert!(value_has_non_atomic_fact(&v));
    }

    #[test]
    fn value_scan_clean_corefacts_is_false() {
        use crate::agent::memory::value_has_non_atomic_fact;
        let v = serde_json::json!({
            "memoryCard": {
                "coreFacts": [
                    {"text": "孩子10岁", "dimension": "孩子年龄"},
                    {"text": "预算5000", "dimension": "预算"}
                ],
                "recentFacts": [{"text": "初次接触", "dimension": ""}]
            }
        });
        assert!(!value_has_non_atomic_fact(&v));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib atomic_fact non_atomic value_scan`
Expected: FAIL（`fact_is_non_atomic` / `value_has_non_atomic_fact` 未定义，编译错误 "cannot find function"）。

- [ ] **Step 3: 实现两个纯函数**

在 `src/agent/memory.rs` 的 `deprecate_same_dimension_conflicts`（memory.rs:480）之前插入：

```rust
/// ⑨件二（方案 X）：结构性非原子检测——判断一条 fact 的 text 是否是"非原子 blob"
/// （consolidator 偶发降级把多个事实揉进一条）。**纯结构度量，零关键词、零数值实体
/// 提取、零 LLM**（守 agent-first）：仅看换行数 / 句界标点数 / char 长度这类客观结构特征。
///
/// 三条 OR 判据（互为冗余兜底）：
/// - ≥2 个换行 `\n`（blob 典型形态：多句 summary 用换行拼接）；
/// - ≥2 个句界标点（`。`/`！`/`？`/`;`）（多个完整句 = 多个事实）；
/// - char 数 > 80（正常原子 fact 实测 ≤~20 字，blob 数百字；80 是宽松上界，
///   宁漏判不误伤——漏判的 blob 还有换行/句界判据兜底 + 件一救回 + 重试）。
pub(crate) fn fact_is_non_atomic(text: &str) -> bool {
    let newline_count = text.matches('\n').count();
    if newline_count >= 2 {
        return true;
    }
    let sentence_breaks = text
        .chars()
        .filter(|c| matches!(c, '。' | '！' | '？' | ';' | '；'))
        .count();
    if sentence_breaks >= 2 {
        return true;
    }
    text.chars().count() > 80
}

/// ⑨件二：扫 consolidator 原始输出 `value` 的 `memoryCard.coreFacts[].text` 与
/// `recentFacts[].text`，任一条 `fact_is_non_atomic` 即判定本次输出含非原子 blob。
/// 兼容 `memoryCard`（camelCase）/ `memory_card`（snake_case）两种 key。
pub(crate) fn value_has_non_atomic_fact(value: &serde_json::Value) -> bool {
    let card = value
        .get("memoryCard")
        .or_else(|| value.get("memory_card"))
        .unwrap_or(value);
    for key in ["coreFacts", "recentFacts"] {
        if let Some(arr) = card.get(key).and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    if fact_is_non_atomic(text) {
                        return true;
                    }
                }
            }
        }
    }
    false
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib atomic_fact non_atomic value_scan`
Expected: 8 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/agent/memory.rs
git commit -m "feat(memory): ⑨件二A fact_is_non_atomic + value_has_non_atomic_fact 纯函数(结构度量零关键词)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 件二B — consolidate_contact_memory_inner 检测重试逻辑

**Files:**
- Modify: `src/agent/memory.rs:1286-1295`（`generate_agent_json` 调用后、`from_document` 解析前，插入检测重试）
- Test: `tests/memory_consolidation_guards.rs`（Create，集成验证）+ 复用 Task 2 的纯函数单测

**Interfaces:**
- Consumes: Task 2 的 `value_has_non_atomic_fact(&Value) -> bool`；现有 `generate_agent_json(state, account_id, contact_wxid, run_id, prompt_key, system, user) -> AppResult<Value>`。
- Produces: 重试后的 `value`（供下游 from_document 消费）；warning 字符串 append 到现有 `consolidator_warnings`（Task 当前文件已有该 Vec，memory.rs:1343 附近）。

**背景**：`user.memory_consolidator.task` 不在 `LLM_EXACT_CACHE` 白名单（mod.rs:480-486 仅 4 个 preview/playbook key；测试 mod.rs:1115 印证非白名单返回 None），故重新调 `generate_agent_json` 即全新 LLM 调用，无需绕缓存。

- [ ] **Step 1: 写失败测试（集成，验重试决策纯逻辑）**

由于真 LLM 不可在 CI 跑，集成测试验证的是"检测 + 丢弃"的可观测结果，用纯函数组合 + 构造 value 验证决策逻辑。Create `tests/memory_consolidation_guards.rs`：

```rust
//! ⑨件二集成测试:验证非原子检测 + 丢弃兜底的决策逻辑(不依赖真 LLM)。
use wechatagent::agent::memory::{fact_is_non_atomic, value_has_non_atomic_fact};

#[test]
fn blob_value_is_flagged_for_retry() {
    // 模拟 consolidator 偶发降级输出(coreFacts[0] 是 411 字 blob 形态)
    let blob_value = serde_json::json!({
        "memoryCard": {
            "coreFacts": [
                {"text": "客户孩子8岁零基础预算5000\n更新孩子10岁\n确认8岁零基础", "dimension": ""}
            ],
            "recentFacts": []
        }
    });
    assert!(value_has_non_atomic_fact(&blob_value), "blob 应被判定需重试");
}

#[test]
fn clean_value_not_flagged() {
    let clean = serde_json::json!({
        "memoryCard": {
            "coreFacts": [
                {"text": "孩子10岁", "dimension": "孩子年龄"},
                {"text": "预算5000", "dimension": "预算"}
            ],
            "recentFacts": []
        }
    });
    assert!(!value_has_non_atomic_fact(&clean), "干净输出不应触发重试");
}

#[test]
fn drop_non_atomic_facts_keeps_clean_ones() {
    // 验证"丢弃非原子条、保留正常条"的过滤逻辑(Task 3 实现的丢弃用同一判据)
    let facts = vec![
        ("孩子8岁\n更新10岁\n确认8岁", false), // 非原子,应被丢
        ("孩子10岁", true),                      // 原子,应保留
        ("预算5000", true),                      // 原子,应保留
    ];
    let kept: Vec<&str> = facts.iter()
        .filter(|(t, _)| !fact_is_non_atomic(t))
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(kept, vec!["孩子10岁", "预算5000"], "应丢非原子留原子");
}
```

确认 `fact_is_non_atomic` / `value_has_non_atomic_fact` 在 `pub(crate)` 下能被集成测试访问——若不能（集成测试是外部 crate），需在测试用到的两函数改 `pub`，或把这三个断言改成 lib 内 `#[cfg(test)]` 单测。**优先方案**：保持 `pub(crate)`，把本 Task 的三个测试放进 `src/agent/memory.rs` 的 `#[cfg(test)] mod tests`（与 Task 2 同位置），不创建外部集成文件。删除上面 `tests/memory_consolidation_guards.rs` 的计划，改为 append 到 lib 测试模块。

> **修正决策**：取消创建 `tests/memory_consolidation_guards.rs`。三个测试 append 到 `src/agent/memory.rs` `#[cfg(test)] mod tests`，函数路径用 `use crate::agent::memory::{fact_is_non_atomic, value_has_non_atomic_fact};`，保持 `pub(crate)` 不放大可见性。

- [ ] **Step 2: 跑测试确认失败/通过基线**

Run: `cargo test --lib blob_value_is_flagged clean_value_not_flagged drop_non_atomic_facts`
Expected: 3 个测试本身（只用 Task 2 已实现的纯函数）应 PASS——它们验证纯函数组合行为，不需要 Task 3 的重试代码。这一步确认纯函数行为正确，重试逻辑的"丢弃"用同一判据。

- [ ] **Step 3: 实现检测重试逻辑**

修改 `src/agent/memory.rs:1286-1295`。把现有的：

```rust
    let value = generate_agent_json(
        state,
        Some(&contact.account_id),
        Some(&contact.wxid),
        Some(&run_id),
        "user.memory_consolidator.task",
        &system,
        &user,
    )
    .await?;
```

替换为（加重试 + 丢弃兜底；新增 `mut value` + `mut non_atomic_warnings`）：

```rust
    let mut value = generate_agent_json(
        state,
        Some(&contact.account_id),
        Some(&contact.wxid),
        Some(&run_id),
        "user.memory_consolidator.task",
        &system,
        &user,
    )
    .await?;
    // ⑨件二(方案 X)：结构性非原子检测 + 降级重试。consolidator 偶发降级把多事实揉成
    // blob(违原子化,致同 dimension 裁决空转)。检测到非原子 → 重试一次拿干净输出
    // (consolidator 非缓存白名单,重试即全新 LLM 调用)。复用 detect_tool_use_hijack 的
    // "拒绝降级产物而非将就"姿态。重试至多 1 次(v4 探针证明 6/6 干净,1 次足够)。
    let mut non_atomic_warnings: Vec<String> = Vec::new();
    if value_has_non_atomic_fact(&value) {
        non_atomic_warnings.push("non_atomic_fact_detected".to_string());
        match generate_agent_json(
            state,
            Some(&contact.account_id),
            Some(&contact.wxid),
            Some(&run_id),
            "user.memory_consolidator.task",
            &system,
            &user,
        )
        .await
        {
            Ok(retry_value) => {
                if value_has_non_atomic_fact(&retry_value) {
                    // 重试仍非原子 → 用重试结果(更新),后续 from_document/丢弃在落库前处理。
                    non_atomic_warnings.push("non_atomic_fact_persists_after_retry".to_string());
                }
                value = retry_value;
            }
            Err(err) => {
                // 重试调用失败(端点 glitch 等)→ 保留首次 value,既成事实纪律不阻断固化。
                non_atomic_warnings.push(format!("non_atomic_retry_failed:{err}"));
            }
        }
    }
```

然后在 `from_document` → `auto_upgrade_plain_facts` 之后、`compact_memory_card_with_dimensions` 之前（memory.rs:1316 之后），增加"丢弃仍非原子的 fact"：

```rust
    // ⑨件二：重试后仍非原子的 fact(极低频)→ 丢弃,不落事实层(其余正常 fact 照常)。
    // 丢弃信息会在下一轮固化由 LLM 重新产出(候选记忆仍在)。用同一结构判据。
    if non_atomic_warnings.iter().any(|w| w == "non_atomic_fact_persists_after_retry") {
        let before_core = card_typed.core_facts.len();
        card_typed.core_facts.retain(|f| !fact_is_non_atomic(f.as_text()));
        card_typed.recent_facts.retain(|f| !fact_is_non_atomic(f.as_text()));
        let dropped = before_core - card_typed.core_facts.len();
        if dropped > 0 {
            non_atomic_warnings.push(format!("non_atomic_fact_dropped_after_retry:{dropped}"));
        }
    }
```

最后把 `non_atomic_warnings` 并入现有的 `consolidator_warnings`（memory.rs:1343 附近 `let mut consolidator_warnings = apply_consolidator_deprecations(...)` 之后）：

```rust
    consolidator_warnings.extend(non_atomic_warnings);
```

> 确认 `card_typed` 在该插入点是 `mut` 且 `as_text()` 可用（`MemoryFactRepr::as_text()` 已存在）。`card_typed` 在 memory.rs:1315 已是 `let mut card_typed`。

- [ ] **Step 4: 编译 + 跑相关测试**

Run: `cargo test --lib fact_is_non_atomic value_has_non_atomic blob_value clean_value drop_non_atomic`
Expected: 全 PASS。
Run: `cargo build --lib`
Expected: 编译通过（确认 `value`/`card_typed` 的 `mut`、借用、`generate_agent_json` 签名匹配）。

- [ ] **Step 5: 跑基线确认不回归**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 6: 提交**

```bash
git add src/agent/memory.rs
git commit -m "feat(memory): ⑨件二B consolidator非原子检测+降级重试(重试拿干净输出,仍失败则丢弃非原子条)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 基线门 + lint 终验

**Files:** 无新改动，仅验证。

- [ ] **Step 1: 全 lib 基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 2: 4 PBT 累计**

Run: `cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter`
Expected: 累计 ≥ 33 passed / 0 failed。

- [ ] **Step 3: 三 lint**

Run: `scripts/check-baseline.sh`（或 Windows `scripts/check-baseline.ps1`）
Run: `scripts/check-no-human-takeover.sh`
Run: `scripts/check-evolution-isolation.sh`（若存在）
Expected: 全绿（本改动不涉及发送路径 / 接管词 / evolution，应自然通过）。

- [ ] **Step 4: cargo check --tests（复刻 CI step2，磁盘受限不跑全 test 编译）**

Run: `RUSTFLAGS="-D warnings" cargo check --tests`
Expected: 0 warnings / 0 errors（确认集成测试 crate 也编译过，AppConfig 等构造点无遗漏）。

---

## Self-Review

**1. Spec coverage：**
- spec §3.1 件一 dimension 感知救回 → Task 1 ✓
- spec §3.2 件二 fact_is_non_atomic（换行/句界/长度） → Task 2 ✓
- spec §3.2 件二 value 扫描 + 重试 + 丢弃兜底 → Task 2（扫描）+ Task 3（重试/丢弃）✓
- spec §3.2 "consolidator 非缓存白名单,重试即全新调用" → Task 3 背景已注明 + 代码注释 ✓
- spec §5.1 件一 4 单测 → Task 1 Step 1 ✓；件二纯函数测试 → Task 2 Step 1（8 个，覆盖 spec 列的 6 个 + value 扫描 2 个）✓
- spec §5.4 基线门 → Task 4 ✓
- spec §3.3 两件套互补关系 → 由 Task 1（残余）+ Task 3（源头）共同实现，数据流注释体现 ✓

**2. Placeholder scan：** 无 TBD/TODO；所有代码步骤含完整代码块；命令含预期输出。Task 3 Step 1 原计划创建 `tests/memory_consolidation_guards.rs`,已在同步骤内"修正决策"改为 append 到 lib 测试模块（避免 pub 放大可见性），无残留矛盾。

**3. Type consistency：**
- `fact_is_non_atomic(text: &str) -> bool` — Task 2 定义，Task 3 消费（丢弃 retain）一致。
- `value_has_non_atomic_fact(value: &serde_json::Value) -> bool` — Task 2 定义，Task 3 消费一致。
- `MemoryFactRepr::Structured` / `MemoryFact.dimension: Option<String>` / `as_text()` — Task 1 用，与 memory.rs:488 既有用法一致。
- `consolidator_warnings: Vec<String>` — Task 3 `extend` 与现有 memory.rs:1343 类型一致。
- `card_typed` 是 `mut`（memory.rs:1315 既有）— Task 3 的 retain 可用 ✓。

**修正记录**：Task 3 Step 1 取消创建外部集成测试文件，三测试并入 lib `#[cfg(test)] mod tests`，函数保持 `pub(crate)`。File Structure 表中 `tests/memory_consolidation_guards.rs` 行随之作废（保留说明但实际不创建）。

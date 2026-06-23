# 标签可信度改造 · 子计划 3：压缩重判引擎 + 宽窗口字符预算 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `consolidate_contact_memory` 在归并记忆的同一趟里**整体重判标签**（replace 语义，纠错主力），喂**原始宽窗口对话**（不再只喂候选条目），窗口按**字符预算 + 条数双上限**度量（微信碎消息适配），重判结果带证据写回 `Contact.confirmed_tags`（fail-closed 校验）。

**Architecture:** 复用现有 `consolidate_contact_memory`（OCC 锁 / 去重调度门 / worker / 预算）零改基建，只扩展其 prompt 输入（加宽窗口对话 + 当前 confirmed_tags + tag_observation 候选）与输出解析（加 `reconfirmedTags` 段）。窗口度量新增两个可配 runtime 参数。confirmed_tags 写回与 memory_card 同一次 OCC 写入。

**Tech Stack:** Rust 2021 / Axum / MongoDB / serde / LLM JSON（`generate_agent_json`）。

## Global Constraints

- `cargo test --lib` ≥ 350 / 0；四 PBT 累计 ≥ 33 / 0。
- 本地只 `cargo test --lib` + 单 PBT；集成留 CI。
- **agent-first**：重判由 LLM 看宽窗口语义判，不引入关键词。
- **no-human-takeover**：代码/注释/prompt 避禁用词。
- **过拟合红线**：归并 prompt 只沉淀抽象重判要求，不针对样本调。
- **既成事实纪律**：归并是后台 task，失败走现有 retry（OCC 冲突 → 候选不消费 retry），不破坏。
- **token 预算**：窗口字符上限须与 `run_token_budget`（默认 30000）留账对齐。
- 提交需用户显式批准；精确 `git add`。

## 设计来源

`docs/superpowers/specs/2026-06-23-tag-trust-two-layer-design.md` —— "压缩时整体重判引擎" + "压缩窗口：喂原始宽窗口对话，按字符预算度量"。

## 依赖

- **子计划 1**：`ConfirmedTag` / `Evidence` / `Contact.confirmed_tags` 字段已存在。
- **子计划 2**：`tag_observation` 候选已在写入（`memory_candidates` source="tag_observation"，doc 形态 `{dimension,value,hitCount,evidences}`）；`resolve_evidence` 纯函数可复用。

## 现状核实（已亲读，事实基线）

- `consolidate_contact_memory`（`memory.rs:871`）→ `_inner`（:900）。候选查询 :907-922（`status:"pending"` sort `created_at:1` limit 30）。
- 归并 prompt 装配 :976-1006：`user` 含"当前 memoryCard + 候选记忆 + 昵称/阶段/意向"，**不含原始对话**。
- LLM 调用 :1007-1016 `generate_agent_json(... "user.memory_consolidator.task" ...)`。
- 输出解析 :1021-1085：取 `memoryCard`（:1021）、`discarded`（:1040）、`conflicts`、`compact_memory_card_with_dimensions`（:1053）合并、warnings 落 `agent_run_logs`（:1068-1084）。
- 候选消费 + memory_card 落库 + OCC：`occ_memory_filter`（:634）、写入 :1131-1152、modified_count==0 → retry（:1171）、候选 `status:"consolidated"`（:1178-1188）。
- 宽窗口工具：`load_context_messages`（`gateway.rs:4200-4207`）= `recent_message_limit*6` clamp(24,80)，**按条数**。
- runtime 参数：`UserRuntimeParameters`（`runtime.rs:19`），`from_config`（:113）用 `runtime_parameters_typed()` 解析 + `clamp_i32(val, lo, hi, default)`（范本 :155-159）。归并入口已建 budget（`memory.rs:886`，读 `runtime.run_token_budget`）。
- `RuntimeParametersTyped`：在 `src/models.rs`（grep `struct RuntimeParametersTyped` 确认字段 + 默认值函数位置）。
- ConversationMessage：`models.rs:485-504`，`id: Option<ObjectId>`、`direction`、`content`、`created_at`。

---

## Task 1：窗口字符预算纯函数

**Files:**
- Create: `src/agent/consolidation_window.rs`（纯函数 + 单测）
- Modify: `src/agent/mod.rs`（`mod consolidation_window;`）
- Test: 文件内 `#[cfg(test)]`

**Interfaces:**
- Produces: `pub fn take_window_by_budget(msgs: &[ConversationMessage], char_budget: usize, max_messages: usize) -> Vec<ConversationMessage>` —— 从最近往回取，累积内容字符到 `char_budget` 或条数到 `max_messages`（谁先到为准），返回**按时间正序**（旧→新）的子集供装 prompt。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversationMessage, MessageDirection};
    use bson::oid::ObjectId;

    fn msg(content: &str, ms: i64) -> ConversationMessage {
        ConversationMessage {
            id: Some(ObjectId::new()),
            workspace_id: "w".into(), account_id: "a".into(), contact_wxid: "c".into(),
            message_id: None, dedupe_key: None,
            direction: MessageDirection::Inbound, content: content.into(),
            msg_type: None, media_ref: None, raw: None,
            created_at: bson::DateTime::from_millis(ms),
        }
    }

    #[test]
    fn stops_at_message_count_cap_for_short_spam() {
        // 100 条短消息（"在"=1字），条数上限 60 先到。
        let all: Vec<_> = (0..100).map(|i| msg("在", i)).collect();
        let w = take_window_by_budget(&all, 6000, 60);
        assert_eq!(w.len(), 60);
        // 返回的是最近 60 条，按时间正序（最旧的在前）。
        assert_eq!(w.first().unwrap().created_at.timestamp_millis(), 40);
        assert_eq!(w.last().unwrap().created_at.timestamp_millis(), 99);
    }

    #[test]
    fn stops_at_char_budget_for_long_messages() {
        // 每条 2000 字，char_budget 6000 → 取约 3 条（条数上限 60 不触达）。
        let long = "x".repeat(2000);
        let all: Vec<_> = (0..50).map(|i| msg(&long, i)).collect();
        let w = take_window_by_budget(&all, 6000, 60);
        assert!(w.len() <= 4 && !w.is_empty());
    }

    #[test]
    fn empty_input_yields_empty() {
        assert!(take_window_by_budget(&[], 6000, 60).is_empty());
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib consolidation_window`
Expected: 编译失败。

- [ ] **Step 3: 写实现**

```rust
// src/agent/consolidation_window.rs
use crate::models::ConversationMessage;

/// 从最近消息往回累积，取到 char_budget（按 content 字符数）或 max_messages（条数）
/// 谁先到为准。返回按时间正序（旧→新）的子集，供装 prompt 顺读。
/// 微信碎消息适配：字符预算保信息量下限，条数防垃圾号（全寒暄）空耗回溯。
pub fn take_window_by_budget(
    msgs: &[ConversationMessage],
    char_budget: usize,
    max_messages: usize,
) -> Vec<ConversationMessage> {
    let mut acc_chars = 0usize;
    let mut picked: Vec<ConversationMessage> = Vec::new();
    // 从最新往最旧回溯（假设入参已按 created_at 升序；若不确定，调用方负责排序）。
    for m in msgs.iter().rev() {
        if picked.len() >= max_messages { break; }
        let len = m.content.chars().count();
        if !picked.is_empty() && acc_chars + len > char_budget { break; }
        acc_chars += len;
        picked.push(m.clone());
    }
    picked.reverse(); // 回到时间正序
    picked
}
```

> 注意：入参排序假设——调用方（Task 4）传进来的 `load_recent_messages` 是 `created_at:-1`（倒序）。**统一约定 `take_window_by_budget` 接收升序切片**，调用方负责把倒序结果 reverse 成升序再传，或本函数内先判方向。实现者在 Task 4 接线时确认方向一致（测试里用升序 ms=0..99 即升序）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib consolidation_window`
Expected: 3 passed。

- [ ] **Step 5: 提交**

```bash
git add src/agent/consolidation_window.rs src/agent/mod.rs
git commit -m "feat(tag-trust): char-budget + count cap window selector (子计划3 Task1)"
```

---

## Task 2：新增两个可配窗口 runtime 参数

**Files:**
- Modify: `src/models.rs`（`RuntimeParametersTyped` 加字段 + 默认值函数）
- Modify: `src/agent/runtime.rs:19`（`UserRuntimeParameters` 加字段）、`:113`（`from_config` 填充 + clamp）
- Test: `src/models.rs` 或 `runtime.rs` 默认值 + clamp 单测

**Interfaces:**
- Produces: `UserRuntimeParameters` 新增 `consolidation_window_char_budget: i64`（默认 6000，clamp [1000,16000]）、`consolidation_window_max_messages: i64`（默认 60，clamp [10,200]）。

- [ ] **Step 1: 核实 RuntimeParametersTyped**

grep `struct RuntimeParametersTyped` + 它的默认值（看现有字段如 `run_token_budget` 怎么定义默认值——`#[serde(default="...")]` 还是 `Default` impl）。grep `clamp_i32` 看签名（runtime.rs，范本 `clamp_i32(val, lo, hi, default)`）。注意现有 clamp 用的是 i32，本字段是 i64——确认是否需要 `clamp_i64` 或直接 `.clamp(lo,hi)`。

- [ ] **Step 2: 写失败测试**

```rust
// 在 runtime.rs 测试 mod（仿现有 recent_message_limit 默认值测试，如 models.rs:4303 风格）
#[test]
fn consolidation_window_defaults_and_clamp() {
    // 默认（无 config）
    let p = UserRuntimeParameters::from_config(None, &test_state()); // 复用现有 test_state helper
    assert_eq!(p.consolidation_window_char_budget, 6000);
    assert_eq!(p.consolidation_window_max_messages, 60);
}
```

> 若 `from_config` 需要 `&AppState` 不便单测，则把 clamp 逻辑抽成纯函数单测（如 `clamp_window_char_budget(raw) -> i64`），仿现有 `clamp_i32` 测法。优先复用现有 runtime 测试的构造方式（grep `from_config` 测试）。

- [ ] **Step 3: 写实现**

`RuntimeParametersTyped`（models.rs）加：
```rust
    #[serde(default = "defaults::consolidation_window_char_budget")]
    pub consolidation_window_char_budget: i64,
    #[serde(default = "defaults::consolidation_window_max_messages")]
    pub consolidation_window_max_messages: i64,
```
`defaults` mod 加：
```rust
    pub fn consolidation_window_char_budget() -> i64 { 6000 }
    pub fn consolidation_window_max_messages() -> i64 { 60 }
```
`UserRuntimeParameters`（runtime.rs:19）加同名字段。`from_config`（:149 附近，run_token_budget 旁）加：
```rust
    consolidation_window_char_budget: typed.consolidation_window_char_budget.clamp(1000, 16000),
    consolidation_window_max_messages: typed.consolidation_window_max_messages.clamp(10, 200),
```
其它 `UserRuntimeParameters { ... }` 构造点（runtime.rs:550、mod.rs:519、types.rs:1612、run_envelope.rs:1541 等——grep `recent_message_limit:` 找全）补这两个字段默认值。

- [ ] **Step 4: 测试 + 编译**

Run: `cargo test --lib consolidation_window_defaults`
Expected: passed。
Run: `cargo check --tests` → 0 errors（所有 UserRuntimeParameters 构造点已补）。

- [ ] **Step 5: 提交**

```bash
git add src/models.rs src/agent/runtime.rs src/agent/mod.rs src/agent/run_envelope.rs
git commit -m "feat(tag-trust): configurable consolidation window char budget + msg cap (子计划3 Task2)"
```

---

## Task 3：归并时加载宽窗口对话 + 注入 prompt

**Files:**
- Modify: `src/agent/memory.rs:900-1006`（`consolidate_contact_memory_inner`：加载窗口 + 拼进 user prompt）
- Modify: `src/prompts.rs`（`user.memory_consolidator.task` schema 加标签重判段 + bump 版本）
- Test: prompt 装配的可观测断言

**Interfaces:**
- Consumes: Task 1 `take_window_by_budget`、Task 2 两个 runtime 参数、子计划 2 的 `tag_observation` 候选。
- Produces: 归并 user prompt 含「原始宽窗口对话（带序号）+ 当前 confirmed_tags + tag_observation 候选」。

- [ ] **Step 1: 加载宽窗口**

在 `consolidate_contact_memory_inner`（memory.rs:906 附近，载 memory 后）加：
```rust
// 标签重判需原始宽窗口对话（不只候选条目）。按字符预算 + 条数双上限取。
let recent = load_recent_messages(state, contact, runtime.consolidation_window_max_messages).await?;
// load_recent_messages 返回 created_at:-1（倒序）→ reverse 成升序供窗口函数与序号渲染。
let mut recent_asc = recent; recent_asc.reverse();
let window = crate::agent::consolidation_window::take_window_by_budget(
    &recent_asc,
    runtime.consolidation_window_char_budget as usize,
    runtime.consolidation_window_max_messages as usize,
);
```
> `runtime` 在 `consolidate_contact_memory`（:884）已构造，但 `_inner` 当前签名是否接收 runtime？Read :900 签名——若未接收需透传（把 runtime 传进 `_inner`，或在 `_inner` 内重新 `from_config`）。`load_recent_messages` 在 gateway，确认 `pub(crate)` 可跨模块调用（subagent 报告它在 gateway.rs:4172）。

- [ ] **Step 2: 拼窗口对话 + confirmed_tags + observations 进 user prompt**

在 user prompt 装配（memory.rs:976 的 `format!`）追加三段：
```rust
// 渲染窗口对话，带 0-based 序号（与子计划2 evidence turn 序位一致）
let convo = window.iter().enumerate()
    .map(|(i, m)| format!("[{i}] {}: {}", direction_label(&m.direction), m.content))
    .collect::<Vec<_>>().join("\n");
// 当前 confirmed_tags（带证据）
let current_tags = serde_json::to_string(&contact.confirmed_tags).unwrap_or_default();
// 本窗口的 tag_observation 候选（从 candidates 里筛 source/dimension）
```
把 `convo` / `current_tags` / tag observations 拼进 `format!` 的 user 串（新增「对话原文」「当前确信标签」「待重判标签观察」三个区块）。

- [ ] **Step 3: prompt schema 加重判输出段 + bump 版本**

在 `user.memory_consolidator.task`（prompts.rs，grep 该 key 的 seed 文本）追加输出要求：
```
重判标签：基于上面对话原文，忘掉旧结论重新判定该客户的标签。输出：
"reconfirmedTags": [
  { "value": "标签", "evidenceTurns": [对话序号数组] }   // 每个保留标签必须指认对话依据
],
"discardedTags": [ { "value": "被推翻的旧标签", "reason": "为何推翻" } ]
没有对话依据支撑的标签不要保留（宁可少，不要脑补）。
```
bump `PROMPT_PACK_VERSION`。

- [ ] **Step 4: 测试 + 编译**

加可观测断言（prompt 常量含 `reconfirmedTags`）；或对装配函数若可抽则抽纯函数测 convo 渲染带序号。
Run: `cargo check` → 0 errors。
Run: `cargo test --lib` → ≥ 350 / 0。

- [ ] **Step 5: 提交**

```bash
git add src/agent/memory.rs src/prompts.rs
git commit -m "feat(tag-trust): consolidation feeds raw wide window + tag reconfirm prompt (子计划3 Task3)"
```

---

## Task 4：解析重判输出 + 写回 confirmed_tags（fail-closed）

**Files:**
- Modify: `src/agent/memory.rs:1085-1152`（输出解析段 + OCC 写入段）
- Test: 重判解析 + fail-closed 校验纯函数单测

**Interfaces:**
- Consumes: 子计划 1 `ConfirmedTag`/`Evidence`、Task 1 窗口、`resolve_evidence`（子计划 2）。
- Produces: `pub(crate) fn parse_reconfirmed_tags(value: &serde_json::Value, window: &[ConversationMessage]) -> Vec<ConfirmedTag>` —— 解析 `reconfirmedTags`，每条经 `resolve_evidence` 映射序位→msg_id，**证据为空的标签丢弃**（fail-closed）。写回 `Contact.confirmed_tags`（replace）。

- [ ] **Step 1: 写 parse_reconfirmed_tags 失败测试**

```rust
#[test]
fn parse_reconfirmed_drops_tags_without_resolvable_evidence() {
    let window = vec![/* 2 条消息，见 Task1 测试构造 */];
    let v = serde_json::json!({
        "reconfirmedTags": [
            { "value": "价格敏感", "evidenceTurns": [0] },   // 有效
            { "value": "脑补标签", "evidenceTurns": [99] },  // 越界 → 证据空 → 丢弃
            { "value": "无依据", "evidenceTurns": [] }       // 空 → 丢弃
        ]
    });
    let out = parse_reconfirmed_tags(&v, &window);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].value, "价格敏感");
    assert!(!out[0].evidences.is_empty());
    assert_eq!(out[0].confirmed_by, "consolidation");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib parse_reconfirmed_drops`
Expected: 编译失败。

- [ ] **Step 3: 实现**

```rust
pub(crate) fn parse_reconfirmed_tags(
    value: &serde_json::Value,
    window: &[ConversationMessage],
) -> Vec<ConfirmedTag> {
    let now = DateTime::now();
    value.get("reconfirmedTags").and_then(|v| v.as_array()).map(|arr| {
        arr.iter().filter_map(|item| {
            let val = item.get("value")?.as_str()?.trim().to_string();
            if val.is_empty() { return None; }
            let turns: Vec<i32> = item.get("evidenceTurns")
                .and_then(|t| t.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_i64().map(|n| n as i32)).collect())
                .unwrap_or_default();
            let evidences = crate::agent::tag_evidence::resolve_evidence(window, &turns);
            if evidences.is_empty() { return None; } // fail-closed：无有效证据丢弃
            Some(ConfirmedTag { value: val, evidences, confirmed_at: now, confirmed_by: "consolidation".to_string() })
        }).collect()
    }).unwrap_or_default()
}
```

- [ ] **Step 4: 写回 confirmed_tags 进 OCC 写入**

在 memory.rs OCC 写入段（:1131-1152，`$set` memory_card 处）把 `confirmed_tags` 一并写：
```rust
let reconfirmed = parse_reconfirmed_tags(&value, &window);
// 在现有 occ update 的 $set 里追加：
//   "confirmed_tags": bson::to_bson(&reconfirmed)?
```
replace 语义（整体覆盖 confirmed_tags），与 memory_card 同一次 OCC 写入（共享版本锁，winner 才写）。tag_observation 候选随归并标 `consolidated`（复用现有 :1178-1188 候选消费，确认它把 source=tag_observation 的候选也一并标记——若现有消费按 run/contact 范围标记则自动覆盖，否则补 filter）。

> **隔离铁律**：写回**只动 confirmed_tags**，绝不碰 `manual_tags`（人工层）、不碰 `bayesian_signals`（子计划4 旁路）。

- [ ] **Step 5: 测试 + 编译 + 基线 + 提交**

Run: `cargo test --lib parse_reconfirmed_drops`
Expected: passed。
Run: `cargo check --tests` → 0 errors。
Run: `cargo test --lib` → ≥ 350 / 0。
Run: `cargo test --test memory_card_invariants`（PBT 不回归）
Expected: pass。

```bash
git add src/agent/memory.rs
git commit -m "feat(tag-trust): parse reconfirmed tags fail-closed, replace confirmed_tags in OCC write (子计划3 Task4)"
```

---

## Self-Review（写计划者自检）

**Spec 覆盖：**
- 喂原始宽窗口对话 → Task 3 Step 1-2 ✓
- 字符预算 + 条数双上限 → Task 1（纯函数）+ Task 2（可配）✓
- 整体重判 replace → Task 4 Step 4（confirmed_tags 整体覆盖）✓
- 证据 fail-closed → Task 4（parse_reconfirmed_tags 丢无证据）✓
- 与记忆归并合一搭车 → Task 3/4 都在 consolidate_contact_memory_inner 内 ✓
- 三线隔离（不碰 manual/bayesian）→ Task 4 Step 4 铁律标注 ✓
- OCC 复用 → Task 4 Step 4（同一次 OCC 写入）✓

**占位符扫描：** Task 3 Step 2 的 `direction_label` / observations 筛选用散文描述+部分代码，是因为它依赖未读的 `format!` 上下文与候选筛选惯例——已标注 grep/Read 动作，非占位。Task 4 Step 4 候选消费"是否覆盖 tag_observation"是真实核实点。

**类型一致：** `ConfirmedTag{value,evidences,confirmed_at,confirmed_by}`（子计划1）、`resolve_evidence`（子计划2）、`take_window_by_budget`（Task1）、窗口序位 turn 与子计划2 evidence turn **同一语义**（窗口内 0-based 下标）——跨子计划一致 ✓。

**关键跨子计划一致性：** 子计划 2 的逐轮 evidence turn 是"逐轮窗口"序位，子计划 3 的重判 turn 是"压缩宽窗口"序位——**两者窗口不同**，turn 只在各自窗口内有效，msg_id（ObjectId hex）才是跨窗口稳定锚。重判时 LLM 看的是压缩宽窗口的序号，parse 用压缩窗口映射——已在 Task 4 用 `&window`（压缩窗口）保证一致。

**需实现期核实（已标注）：** `_inner` 是否接收 runtime、`load_recent_messages` 可见性、RuntimeParametersTyped 默认值写法、clamp i64 形式、候选消费是否覆盖 tag_observation、PROMPT_PACK_VERSION 名。

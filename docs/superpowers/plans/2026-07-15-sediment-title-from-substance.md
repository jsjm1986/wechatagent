# 领导授权沉淀知识 title/body 误用 reviewer 质检点评 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让领导授权沉淀的知识 chunk（B 类 verified + 可泛化 draft 提案）的 `title` 来自 `decision.substance`（真正的知识内容），而非 `entry.reason`（卡点原因/reviewer 质检黑话）；`body` 去掉塞质检点评的"卡点"行。同时就地修正生产库已污染的存量 chunk。

**Architecture:** 新增确定性纯函数 `derive_sediment_title_fallback(substance)`（首句+限长，可单测），再叠加一层 LLM 提炼（prompt_key `escalation.sediment.title`，走唯一 JSON 入口 `generate_agent_json`）——提炼失败/空一律回退兜底，沉淀永不失败。`sediment_principal_authorized_knowledge` 与 `emit_knowledge_gap_proposal` 两处改用新 title 来源并去 body 卡点行。存量走一次性 mongosh 脚本（非 migration）。

**Tech Stack:** Rust 2021, cargo, mongodb crate。测试 `cargo test --lib`。存量脚本 mongosh。

## Global Constraints

- **不动 reason 语义**：`entry.reason` 给领导看质检点评是合理设计，问题只在"拿 reason 当知识标题"。不改 `escalate_held_decision` / `trigger_principal_escalation` 的 reason 来源。
- **不动召回权重 / R5.4 / relay / 豁免逻辑**：本次真实测试已验证这些正确。
- **沉淀永不因 title 提炼失败而失败**：LLM 提炼是纯 JSON 生成、无 tool call → `generate_agent_json` 内部 `record_call` 不抛错（只有 tool_call 才抛 `BudgetExceeded`，已亲验 budget.rs），失败/空/解析错一律回退确定性兜底。
- **红线不破**：B 类仍 source=PrincipalAuthorized 两步法落 verified，draft 提案仍 needs_review。title 来源变更不触碰验证语义。
- **no-human-takeover lint**：新增行/prompt 文案/注释禁 `人工/接管/转接/托管/takeover/hand-off`。
- **no-model-hint lint**：新增行禁硬编码模型/品牌名。
- **基线不回归**：`cargo test --lib` 0 failed 且 ≥ 350；PBT 四件 ≥ 33；`scripts/check-baseline.sh` 双门绿。
- **本地磁盘纪律**：只跑 `cargo test --lib` 与 `cargo build --lib`，集成测试交 CI。

---

## File Structure

- `src/agent/escalation/ledger.rs` — 新增 `derive_sediment_title_fallback` 纯函数 + `derive_sediment_title`（LLM+兜底）异步函数；`sediment_principal_authorized_knowledge`（:283）改 title 来源 + 去 body 卡点行；`emit_knowledge_gap_proposal`（:184）同改。新增单元测试。
- `src/prompts.rs` — 新增 prompt spec `escalation.sediment.title`（挂在 `escalation.principal.interpret` 之后，:2238）。
- `scripts/fix_sediment_title_from_substance.js` — 一次性存量修正脚本（备份→修正→回读）。

---

## Task 1: 确定性兜底纯函数 `derive_sediment_title_fallback`

**Files:**
- Modify: `src/agent/escalation/ledger.rs`（新增纯函数 + 测试）

**Interfaces:**
- Produces: `pub(crate) fn derive_sediment_title_fallback(substance: &str) -> String`

- [ ] **Step 1: 先写失败测试（TDD 红）**

在 `ledger.rs` 测试区（文件末尾 `#[cfg(test)] mod tests` 内，若无则新建）加：

```rust
#[test]
fn fallback_takes_first_sentence() {
    // 句号截断：只取首句
    let t = derive_sediment_title_fallback("同意给他八折。本周内付款有效。");
    assert_eq!(t, "同意给他八折");
}

#[test]
fn fallback_no_terminator_takes_whole_when_short() {
    let t = derive_sediment_title_fallback("同意八折");
    assert_eq!(t, "同意八折");
}

#[test]
fn fallback_truncates_long_by_chars_not_bytes() {
    // 41 个中文字符（多字节）应截到 40 + 省略号，且不 panic（按 chars 截断）
    let s = "一".repeat(41);
    let t = derive_sediment_title_fallback(&s);
    assert_eq!(t.chars().count(), 41); // 40 + '…'
    assert!(t.ends_with('…'));
}

#[test]
fn fallback_empty_returns_safe_title() {
    assert_eq!(derive_sediment_title_fallback("   "), "领导授权沉淀");
}

#[test]
fn fallback_newline_is_sentence_terminator() {
    let t = derive_sediment_title_fallback("同意八折\n补充说明若干");
    assert_eq!(t, "同意八折");
}
```

- [ ] **Step 2: 实现（TDD 绿）**

```rust
/// 从领导裁决 substance 提炼一个确定性的知识标题兜底：
/// 取首句（截到第一个句末标点 `。！？!?` 或换行之前），再按 chars 限长 40。
/// 空 substance → 固定安全标题（配合 sediment 空 substance 已提前跳过，实际仅有 substance 时被用到）。
/// LLM 提炼失败时回退到本函数，保证 title 永远可读、沉淀永不失败。
pub(crate) fn derive_sediment_title_fallback(substance: &str) -> String {
    let trimmed = substance.trim();
    if trimmed.is_empty() {
        return "领导授权沉淀".to_string();
    }
    // 首句：截到第一个句末标点 / 换行之前。
    let first = trimmed
        .split(|c| matches!(c, '。' | '！' | '？' | '!' | '?' | '\n'))
        .next()
        .unwrap_or(trimmed)
        .trim();
    let first = if first.is_empty() { trimmed } else { first };
    // 按 chars 限长 40（多字节安全），超长截断加省略号。
    let mut chars: Vec<char> = first.chars().collect();
    if chars.len() > 40 {
        chars.truncate(40);
        let mut out: String = chars.into_iter().collect();
        out.push('…');
        out
    } else {
        chars.into_iter().collect()
    }
}
```

- [ ] **Step 3: 验证**

```sh
cargo test --lib derive_sediment_title_fallback
```

预期：5 个测试全绿。

**Done when:** 5 个 fallback 测试通过，`cargo build --lib` 无警告。

---

## Task 2: prompt spec `escalation.sediment.title`

**Files:**
- Modify: `src/prompts.rs`（`escalation.principal.interpret` 之后，:2238 `},` 与 `]` 之间）

**Interfaces:**
- Produces: prompt_key `escalation.sediment.title`（`ensure_prompt_pack_v2` 启动种入）

- [ ] **Step 1: 加 PromptSpec 条目**

在 `escalation.principal.interpret` 条目的闭合 `},`（:2238）之后插入：

```rust
        PromptSpec {
            key: "escalation.sediment.title",
            agent_kind: "user",
            layer: "escalation",
            title: "领导授权沉淀知识标题提炼器",
            description: "把领导裁决实质（substance）提炼成一句面向全体复用的知识标题。只输出 JSON。",
            status: "active",
            content: r#"你要为一条即将沉淀进知识库、供全体客户复用的运营知识拟一个标题。下面是"领导"授权的一句决策实质，请提炼成一句简洁的知识标题。只输出 JSON，不要解释。

要求：
- 一句话，尽量短（不超过 20 个字），像知识库条目的标题，不是完整句子。
- 概括这条知识"说的是什么"，面向今后检索复用，不要写"领导同意""授权"之类的过程描述。
- 只依据给定的决策实质，不要臆造内容。

输出 JSON：
{
  "title": "一句话知识标题"
}"#,
        },
```

- [ ] **Step 2: 验证内容比对不误触发重种**

```sh
cargo build --lib
```

（提示：prompt 生效按 `normalize_prompt_content` 内容比对决定重种，无需 bump 版本常量——见 memory `project_prompt_pack_version_not_effect_gate`。）

**Done when:** `cargo build --lib` 通过；新 spec 出现在 `prompt_specs()` 返回列表。

---

## Task 3: `derive_sediment_title`（LLM 提炼 + 兜底）异步函数

**Files:**
- Modify: `src/agent/escalation/ledger.rs`（新增异步函数）

**Interfaces:**
- Consumes: `derive_sediment_title_fallback`（Task 1）、prompt `escalation.sediment.title`（Task 2）、`generate_agent_json`（`crate::agent::generate_agent_json`）、`prompts::load_prompt`
- Produces: `async fn derive_sediment_title(state, account_id, contact_wxid, substance) -> String`

- [ ] **Step 1: 实现（LLM 为主，任何失败回退兜底）**

```rust
/// 用 LLM 从 substance 提炼知识标题；任何失败（出错/空/解析失败）回退确定性兜底。
/// 绝不返回 Err——沉淀永不因 title 提炼失败而失败。
async fn derive_sediment_title(
    state: &AppState,
    account_id: &str,
    contact_wxid: &str,
    substance: &str,
) -> String {
    let fallback = derive_sediment_title_fallback(substance);
    if substance.trim().is_empty() {
        return fallback;
    }
    let system = match crate::prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "escalation.sediment.title",
    )
    .await
    {
        Ok(s) => s,
        Err(_) => return fallback,
    };
    let value = match crate::agent::generate_agent_json(
        state,
        Some(account_id),
        Some(contact_wxid),
        None,
        "escalation.sediment.title",
        &system,
        substance,
    )
    .await
    {
        Ok(v) => v,
        Err(_) => return fallback,
    };
    let title = value
        .get("title")
        .and_then(|t| t.as_str())
        .map(|s| s.trim())
        .unwrap_or("");
    if title.is_empty() {
        fallback
    } else {
        // 即便 LLM 返回过长标题也按同口径限长（复用兜底的限长语义）。
        derive_sediment_title_fallback(title)
    }
}
```

（注：`load_prompt` 与 `generate_agent_json` 的确切模块路径在实现时以 `escalation/mod.rs:253-268` `interpret_principal_reply` 的既有用法为准——它已同时用到这两者，照抄其 import/调用范式即可。）

- [ ] **Step 2: 验证**

```sh
cargo build --lib
```

**Done when:** 编译通过；函数签名与 interpret_principal_reply 的 prompt 调用范式一致。

---

## Task 4: 改 `sediment_principal_authorized_knowledge`（B 类）

**Files:**
- Modify: `src/agent/escalation/ledger.rs:283-295`

**Interfaces:**
- Consumes: `derive_sediment_title`（Task 3）

- [ ] **Step 1: 改 title 来源 + 去 body 卡点行**

`ledger.rs:283` 现状 `let title = format!("领导授权沉淀：{}", entry.reason);` 改为：

```rust
    let title = derive_sediment_title(state, &entry.account_id, &entry.contact_wxid, substance).await;
```

`body`（:284-295）去掉 `卡点：{reason}` 行，`reason` 参数一并从 format 移除：

```rust
    let body = format!(
        "源自客户「{}」请示 #{}。\n领导裁决：{}\n约束：{}",
        entry.contact_wxid,
        entry.short_code,
        substance,
        if decision.constraints.is_empty() {
            "无".to_string()
        } else {
            decision.constraints.join("；")
        }
    );
```

- [ ] **Step 2: 验证**

```sh
cargo build --lib
```

确认 `entry.reason` 在本函数内不再被引用（若产生 unused 警告说明已彻底移除）。

**Done when:** 编译通过；grep `entry.reason` 在 `sediment_principal_authorized_knowledge` 函数体内零命中。

---

## Task 5: 改 `emit_knowledge_gap_proposal`（可泛化 draft 提案）

**Files:**
- Modify: `src/agent/escalation/ledger.rs:184-196`

**Interfaces:**
- Consumes: `derive_sediment_title`（Task 3）

- [ ] **Step 1: 改 title 来源 + 去 body 卡点行**

`ledger.rs:184` 现状 `let title = format!("真人决策沉淀（待审核）：{}", escalation.reason);` 改为从 substance 提炼（保留"待审核"语义前缀便于运营识别 draft 性质）：

```rust
    let derived = derive_sediment_title(state, &escalation.account_id, &escalation.contact_wxid, &decision.substance).await;
    let title = format!("待审核：{derived}");
```

`body`（:185-196）去掉 `卡点：{reason}` 行：

```rust
    let body = format!(
        "源自客户「{}」请示 #{}。\n领导裁决：{}\n约束：{}",
        escalation.contact_wxid,
        escalation.short_code,
        decision.substance,
        if decision.constraints.is_empty() {
            "无".to_string()
        } else {
            decision.constraints.join("；")
        }
    );
```

- [ ] **Step 2: 验证**

```sh
cargo build --lib && cargo test --lib
```

**Done when:** 编译通过；`escalation.reason` 在本函数体内零命中；`cargo test --lib` 不回归（≥ 350，0 failed）。

---

## Task 6: 存量修正脚本

**Files:**
- Create: `scripts/fix_sediment_title_from_substance.js`

**Interfaces:**
- 一次性 mongosh 脚本，脚本内不调 LLM，用与 `derive_sediment_title_fallback` 等价的首句+限长 JS 逻辑。

- [ ] **Step 1: 写脚本（备份→修正→回读三段式，仿 `scripts/cleanup_non_human_managed.js`）**

逻辑：
1. `use('wechatagent')`（或 `getSiblingDB('wechatagent')`）。
2. 找 `title` 以 `领导授权沉淀：` 或 `真人决策沉淀（待审核）：` 或（Task 5 改后新增前缀）开头的 chunk。
   - 存量只有旧前缀两条，脚本按旧前缀匹配即可。
3. 对每条：substance 来源优先 `source_quote`，无则从 body 的 `领导裁决：` 段截取。
4. 用等价 JS 首句+限长（`。！？!?\n` 截断、`Array.from(s).slice(0,40)` 按码点截断）算新 title。
   - B 类（`领导授权沉淀：`）→ 新 title 直接是提炼结果；draft（`真人决策沉淀（待审核）：`）→ `待审核：{提炼}`。
5. body 去掉 `卡点：...` 行（按 `\n` split、过滤掉以 `卡点：` 开头的行、再 join）。
6. 备份原 `{_id, title, body}` 打印 → `updateOne $set` → 回读打印新 `{title, body}`。

- [ ] **Step 2: 干跑校验（先只打印不写）**

脚本顶部留 `const DRY_RUN = true;` 开关，先在 117 跑一遍看匹配到的条数（预期 2 条：1 B 类 verified + 1 draft）和拟改的 title/body，确认无误再 `DRY_RUN=false` 落库。

- [ ] **Step 3: 应用 + 回读**

在生产 117 本地跑（mongosh），确认：
- B 类 chunk `6a566a9d6f89ea84b3b24d9d` 的 title 不再是质检黑话、body 无"卡点"行、`status/integrity` 不变（仍 active/verified）。
- draft chunk `6a54f281ce8e1ff82a77cd4a` 的 title 变为 `待审核：...`。

**Done when:** 生产库 2 条污染 chunk 的 title/body 修正完成，其余字段（status/integrity/source/anchors）不变，回读确认。

---

## Verification (whole plan)

1. `cargo build --lib` 无警告。
2. `cargo test --lib`：0 failed 且 ≥ 350（新增 5 个 fallback 测试是净增）。
3. `scripts/check-baseline.sh`（或 `.ps1`）双门绿。
4. lint：`scripts/check-no-human-takeover.sh` + `scripts/check-no-model-hint.sh` 对本次 diff 绿。
5. 存量脚本在 117 应用后回读确认 2 条 chunk 修正、其余字段不变。
6. （可选，CI）集成测试不回归。

## 不做（YAGNI，重申）

- 不改 `entry.reason` 来源/语义。
- 不改召回权重、knowledge_router、knowledge_tools。
- 不改 relay / R5.4 / 豁免写入。
- 不重发 E6PM5 被 MCP 429 卡住的消息（独立问题）。

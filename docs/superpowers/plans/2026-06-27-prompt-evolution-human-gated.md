# Prompt 自优化「AI 提议 + 人工把关发布」实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 prompt 自优化从「发布路径绕过红线闸 + shadow 未实装」的半成品，做成「AI 提议 + 真模型证据 + 人工把关发布」的可用闭环。

**Architecture:** 三阶段。阶段一（安全底线）：把人工编辑 prompt 的三道红线闸从 `routes/management_prompt_edit.rs`（私有模块）下沉到中立顶层模块 `src/prompt_guard.rs`，让 evolution 的 `release_prompt` 也能调用；并把 `release_prompt` 的 snippet 处理从「整篇覆盖」改为「末尾追加」+ 过三闸。阶段二（证据闭环）：在 `src/agent/prompt_shadow.rs` 用真模型跑新旧 prompt 对照，证据存入 proposal，不自动放行。阶段三（前端）：候选详情页展示对照证据 + 人工 release。

**Tech Stack:** Rust 2021 (Axum)、MongoDB (mongodb crate)、React 19 + TypeScript + Vite。后端无 workspace、单 crate。

**关联 spec:** `docs/superpowers/specs/2026-06-27-prompt-evolution-human-gated-design.md`

## Global Constraints

以下为全局约束，每个 task 隐含包含，违反即作废：

- **过拟合红线**：绝不为过测试改业务逻辑/prompt/guards/阈值；只修真 bug，改根因不迎合断言。（CLAUDE.md + memory `feedback_no_overfitting`）
- **测试基线不回归**：`cargo test --lib` ≥ 350 passed / 0 failed；4 个 PBT 文件（`state_transition_pbt` / `memory_card_invariants` / `wiki_chunk_revision_pbt` / `llm_retry_jitter`）累计 ≥ 33 passed / 0 failed。任一失败 → 不可合并。（CLAUDE.md「Test baseline」+ `scripts/check-baseline.sh`）
- **隔离红线**：`scripts/check-evolution-isolation.sh` 全量扫 `src/evolution/**/*.rs`，禁 8 个发送符号字面量（`crate::agent::gateway` / `crate::agent::outbox` / `crate::mcp::` / `agent_send_outbox.insert` / `mcp_client.send` / `run_user_operation_gateway` / `handle_managed_message` / `handle_follow_up_task`）。非注释行命中即 exit 1。
- **无人工接管红线**：`scripts/check-no-human-takeover.sh` 扫 `src/agent/` `src/routes/` `src/evolution/` `frontend/src/` 四目录的 **git diff 新增行**，禁词 `(human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工)`。**注意**：`*/tests/*` 路径跳过，但**同文件 `#[cfg(test)]` mod 不跳过**（脚本只匹配路径）。测试里构造禁用词必须用字符拼接绕字面量。`src/prompt_guard.rs` 是顶层模块**不在这四个扫描目录内**，但仍按既有惯例保留字符拼接 helper（零风险逐字迁移）。
- **本地磁盘纪律**：只跑 `cargo test --lib` 和单个 PBT 文件（`cargo test --test <name>`）；完整 `--ignored` 集成套件留 CI。本地 `cargo check` / `cargo check --tests` 验证编译。
- **commit 须获批**：本计划已获用户批准执行；按 subagent-driven / executing-plans 流程逐 task commit。
- **shell 是 Windows 上的 bash**：用正斜杠、绝对路径；项目根含非 ASCII（`工作项目`），避免 `cd`。
- **语言**：对用户回复用中文；代码/标识符/commit message follow 既有约定。

---

## 文件结构（File Structure）

阶段一新建/改动：

- **Create** `src/prompt_guard.rs` — 中立顶层模块，托管三道红线闸的全部符号（从 `management_prompt_edit.rs` 迁入，可见性升 `pub`）：`PromptEditTier` / `prompt_edit_tier` / `required_anchors` / `validate_prompt_edit` / `PromptEditVerdict` / `extract_diff` / `review_prompt_edit` + 现有 10 个单测。
- **Modify** `src/lib.rs` — 加 `pub mod prompt_guard;` 模块声明。
- **Modify** `src/routes/management_prompt_edit.rs` — 删除迁走的符号，改为 `pub(super) use crate::prompt_guard::{...};` re-export，保持 `prompt_templates.rs:150/169` 调用点不破。
- **Modify** `src/evolution/error.rs` — `EvolutionError` 加 `RedlineGateRejected(String)` 变体。
- **Modify** `src/routes/evolution.rs:379-392` — `evolution_error_to_app_error` 加一条 arm 把 `RedlineGateRejected` 映射为 `AppError::BadRequest`。
- **Modify** `src/evolution/release.rs:181-371` — `release_prompt` 的 snippet 处理改末尾追加 + 调 `validate_prompt_edit` + `review_prompt_edit`；改文档注释（删「整段 diff_snippet 当成新 content」表述）。

阶段二/三文件见下方骨架章节。

---

## 阶段一：安全底线（堵 G2/G3 红线缺口，独立可交付）

> 本阶段单独就堵住「release_prompt 绕过三闸」+「snippet 整篇覆盖删红线」两个缺口，不依赖阶段二是否实装。

### Task 1: 创建 `src/prompt_guard.rs` 中立模块（三闸下沉）

把三道闸的符号从 `routes/management_prompt_edit.rs` 整体迁到顶层 `src/prompt_guard.rs`，可见性 `pub(super)`→`pub`。**纯迁移，零行为变化**——先让新模块带全部 10 个单测独立编译通过。

**Files:**
- Create: `src/prompt_guard.rs`
- Modify: `src/lib.rs`（加模块声明）

**Interfaces:**
- Produces（供 Task 2 的 management_prompt_edit re-export + Task 5 的 release_prompt 调用）：
  - `pub fn validate_prompt_edit(template_key: &str, new_content: &str) -> Result<(), String>`
  - `pub async fn review_prompt_edit(state: &AppState, workspace_id: &str, template_key: &str, old: &str, new: &str) -> PromptEditVerdict`
  - `pub enum PromptEditVerdict { Pass, Reject(String), NeedsHumanConfirm { diff: String, reason: String } }`
  - `pub fn extract_diff(old: &str, new: &str) -> String`
  - `pub fn prompt_edit_tier(template_key: &str) -> PromptEditTier`
  - `pub enum PromptEditTier { FreelyEditable, ConstrainedEditable, Forbidden }`
  - `pub fn required_anchors(template_key: &str) -> Vec<&'static str>`

- [ ] **Step 1: 创建 `src/prompt_guard.rs`，逐字迁入全部符号**

把 `src/routes/management_prompt_edit.rs` 第 1-306 行的全部内容（含文件头注释、import、7 个符号定义、`#[cfg(test)] mod tests`）复制到新文件 `src/prompt_guard.rs`，做以下机械改动：
- 文件头注释保留，补一行说明「本模块从 routes/management_prompt_edit.rs 下沉，供人工编辑路径与 evolution release 路径共用」。
- 所有 `pub(super)` 可见性改成 `pub`（`PromptEditTier` / `prompt_edit_tier` / `validate_prompt_edit` / `PromptEditVerdict` / `extract_diff` / `review_prompt_edit`）。
- `required_anchors` 当前是私有 `fn`（无 `pub(super)`），改 `pub fn`。
- import 保持不变（`crate::evolution::lint::passes_forbidden_words` / `crate::prompts::{...}` / `crate::routes::AppState` / `serde_json::Value`）—— 这些路径从顶层模块仍可达（都是 `pub`）。
- `#[cfg(test)] mod tests` 整段逐字保留（含 `forbidden_phrase()` 字符拼接 helper 和 10 个测试）。

- [ ] **Step 2: `src/lib.rs` 注册模块**

在 `src/lib.rs` 模块声明区（与其它 `pub mod` 并列）加：

```rust
pub mod prompt_guard;
```

放在 `pub mod prompts;` 之后（字母序 + 依赖序：prompt_guard 依赖 prompts 的常量）。

- [ ] **Step 3: 编译验证新模块带测试通过**

Run: `cargo test --lib prompt_guard -- --list`
Expected: 列出 10 个测试名（`prompt_guard::tests::tier_classifies_three_layers` 等），编译无错。

- [ ] **Step 4: 运行新模块的 10 个单测**

Run: `cargo test --lib prompt_guard`
Expected: PASS，10 passed（行为与迁移前逐字相同）。

- [ ] **Step 5: Commit**

```bash
git add src/prompt_guard.rs src/lib.rs
git commit -m "refactor(prompt_guard): 三道红线闸下沉到中立顶层模块

从 routes/management_prompt_edit.rs 逐字迁入 validate_prompt_edit/
review_prompt_edit/extract_diff/PromptEditTier 等,可见性 pub(super)->pub,
供人工编辑路径与 evolution release 路径共用。纯迁移零行为变化,10 单测随迁。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: `management_prompt_edit.rs` 改 re-export（消除重复定义）

迁出后，`management_prompt_edit.rs` 删掉重复定义，改为从 `prompt_guard` re-export，让 `prompt_templates.rs:150/169` 现有调用点零改动继续工作。

**Files:**
- Modify: `src/routes/management_prompt_edit.rs`（删除 1-177 行的符号定义 + 179-306 行的测试，改为 re-export）

**Interfaces:**
- Consumes: Task 1 的 `crate::prompt_guard::{validate_prompt_edit, review_prompt_edit, PromptEditVerdict, ...}`
- Produces: `pub(super) use` 让 `crate::routes::management_prompt_edit::validate_prompt_edit` 等路径仍解析（`prompt_templates.rs` 用的就是这个路径）。

- [ ] **Step 1: 确认现有调用点用的路径（已核实）**

已核实：`src/routes/prompt_templates.rs` 用全限定路径访问，且**只用 3 个符号**：
- `crate::routes::management_prompt_edit::validate_prompt_edit`（:150）
- `crate::routes::management_prompt_edit::review_prompt_edit`（:169）
- `crate::routes::management_prompt_edit::PromptEditVerdict`（:178/179/184，匹配 `Pass`/`Reject`/`NeedsHumanConfirm`）

`mod management_prompt_edit;` 是私有模块（mod.rs:59），其内 `pub(super) use` 项对 `routes` 模块可见，`prompt_templates` 在 `routes` 内 → 全限定路径可达。无需改 `prompt_templates.rs`。执行时再跑一次 grep 复核未漂移：

Run: `grep -n "management_prompt_edit::" src/routes/prompt_templates.rs`
Expected: 只出 `validate_prompt_edit` / `review_prompt_edit` / `PromptEditVerdict` 三个。若多出符号，把它加进下一步 re-export 列表。

- [ ] **Step 2: 把 `management_prompt_edit.rs` 全文替换为 re-export**

将 `src/routes/management_prompt_edit.rs` 整个文件内容替换为（**只 re-export 实际使用的 3 个符号**，避免 `unused_imports` warning）：

```rust
//! 提示词自然语言编辑的三层分级 + 双闸校验（spec §4.4）。
//!
//! 实现已下沉到中立顶层模块 `crate::prompt_guard`（供人工编辑路径与
//! evolution release 路径共用）。本文件只 re-export prompt_templates.rs
//! 实际使用的符号，保持调用路径 `crate::routes::management_prompt_edit::{...}` 不破。

pub(super) use crate::prompt_guard::{review_prompt_edit, validate_prompt_edit, PromptEditVerdict};
```

> 注：`extract_diff`/`prompt_edit_tier`/`required_anchors`/`PromptEditTier` 不被 routes 引用（只在 prompt_guard 内部 + 其测试用），不 re-export。

- [ ] **Step 3: 编译验证调用点不破**

Run: `cargo check`
Expected: 编译通过，无 `unresolved import` / `unused import` 错误。若有 unused，按实际删减 re-export 列表。

- [ ] **Step 4: 跑 prompt_templates 相关测试 + 全 lib 基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed（10 个闸测试现在归属 `prompt_guard::tests`，总数不变）。

- [ ] **Step 5: 跑 no-human-takeover lint 确认不误伤**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: ok（management_prompt_edit.rs 新增行只有 re-export，无禁词；prompt_guard.rs 不在扫描目录）。

- [ ] **Step 6: Commit**

```bash
git add src/routes/management_prompt_edit.rs
git commit -m "refactor(routes): management_prompt_edit 改 re-export prompt_guard

三闸实现已下沉,本文件只保留 re-export 维持 prompt_templates.rs 调用路径。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: `EvolutionError` 加 `RedlineGateRejected` 变体

为 release 路径过闸被拒提供独立错误类型（区别于 `InvalidStatus`，前端要分开展示）。

**Files:**
- Modify: `src/evolution/error.rs`
- Modify: `src/routes/evolution.rs:379-392`（`evolution_error_to_app_error` 加 arm）

**Interfaces:**
- Produces（供 Task 5 release_prompt 返回）：`EvolutionError::RedlineGateRejected(String)`

- [ ] **Step 1: error.rs 加变体**

在 `src/evolution/error.rs` 的 `EvolutionError` enum 内（`Internal(String)` 之前）加：

```rust
    /// release 路径过红线闸（禁词/锚点/LLM 语义）被拒。与 InvalidStatus 区分：
    /// 这不是状态机错误,而是候选内容触碰红线,前端需单独展示「已拒绝发布」。
    #[error("redline gate rejected: {0}")]
    RedlineGateRejected(String),
```

- [ ] **Step 2: evolution.rs 加映射 arm**

在 `src/routes/evolution.rs` 的 `evolution_error_to_app_error`（:379）match 内加：

```rust
        EvolutionError::RedlineGateRejected(msg) => AppError::BadRequest(msg),
```

- [ ] **Step 3: 加映射单测**

在 `src/routes/evolution.rs` 的 `#[cfg(test)] mod tests` 内（紧挨 `evolution_error_to_app_error_maps_invalid_status_to_bad_request` 之后）加：

```rust
    #[test]
    fn evolution_error_redline_rejected_maps_to_bad_request() {
        let err = EvolutionError::RedlineGateRejected("命中禁用词表".to_string());
        match evolution_error_to_app_error(err) {
            AppError::BadRequest(msg) => assert!(msg.contains("禁用词")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }
```

- [ ] **Step 4: 编译 + 跑测试**

Run: `cargo test --lib evolution_error`
Expected: PASS（新测试 + 既有 2 个错误映射测试全过）。`cargo check` 确认 match 穷尽（加变体后若漏 arm 编译器会报 non-exhaustive）。

- [ ] **Step 5: Commit**

```bash
git add src/evolution/error.rs src/routes/evolution.rs
git commit -m "feat(evolution): EvolutionError 加 RedlineGateRejected 变体

release 路径过红线闸被拒的独立错误类型,映射 BadRequest,前端可单独展示。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: prompt 合成纯函数 `compose_appended_content`（末尾追加）

把「原 content + 追加片段」的合成抽成 `prompt_guard.rs` 的纯函数，单测锁定行为（原文逐字保留 + 片段追加），供 Task 5 调用。

**Files:**
- Modify: `src/prompt_guard.rs`（加纯函数 + 单测）

**Interfaces:**
- Consumes: 无
- Produces（供 Task 5）：`pub fn compose_appended_content(current: &str, snippet: &str) -> String`

- [ ] **Step 1: 写失败测试**

在 `src/prompt_guard.rs` 的 `#[cfg(test)] mod tests` 末尾加：

```rust
    #[test]
    fn compose_appends_snippet_preserving_original() {
        let current = "原始 prompt 正文\n红线锚段";
        let snippet = "补充：本行业语气更稳重";
        let composed = compose_appended_content(current, snippet);
        // 原文逐字保留在开头（锚点闸据此天然通过）
        assert!(composed.starts_with(current));
        // 片段出现在末尾
        assert!(composed.ends_with(snippet));
        // 中间有空行分隔
        assert!(composed.contains("红线锚段\n\n补充"));
    }

    #[test]
    fn compose_trims_snippet_edge_whitespace_but_keeps_body() {
        let composed = compose_appended_content("正文", "  \n追加片段\n  ");
        assert!(composed.starts_with("正文\n\n"));
        assert!(composed.contains("追加片段"));
        // 不产生多余尾部空白行
        assert_eq!(composed.trim_end(), composed.trim_end_matches('\n').trim_end());
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib prompt_guard::tests::compose`
Expected: FAIL（`compose_appended_content` not found）。

- [ ] **Step 3: 写最小实现**

在 `src/prompt_guard.rs` 的非测试区（`extract_diff` 附近）加：

```rust
/// 末尾追加合成：原 prompt 正文逐字保留在开头（红线锚点据此天然通过锚闸），
/// critic 片段追加到末尾,空行分隔。critic 只能「加约束」不能改写原红线段。
pub fn compose_appended_content(current: &str, snippet: &str) -> String {
    let snippet = snippet.trim();
    if snippet.is_empty() {
        return current.to_string();
    }
    format!("{}\n\n{}", current.trim_end(), snippet)
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib prompt_guard::tests::compose`
Expected: PASS（2 个新测试）。

- [ ] **Step 5: Commit**

```bash
git add src/prompt_guard.rs
git commit -m "feat(prompt_guard): compose_appended_content 末尾追加合成纯函数

原 prompt 正文逐字保留+critic 片段追加末尾,红线锚点天然过锚闸。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: `release_prompt` 接三闸 + snippet 改末尾追加

核心修复。`release_prompt` 加载 current 后、写库前：合成「原文+追加片段」→ 过禁词/锚点闸（`validate_prompt_edit`）→ 过 LLM 语义闸（`review_prompt_edit`）→ 两闸过才进事务。

**Files:**
- Modify: `src/evolution/release.rs:181-312`（`release_prompt` 函数体 + 文档注释）

**Interfaces:**
- Consumes: `crate::prompt_guard::{compose_appended_content, validate_prompt_edit, review_prompt_edit, PromptEditVerdict}`、Task 3 的 `EvolutionError::RedlineGateRejected`
- Produces: 无（终端写库）

- [ ] **Step 1: 改 release_prompt 文档注释**

把 `src/evolution/release.rs` 第 188-190 行的注释：

```
/// 4. insert 新一条 `version = old.version + 1`、`current_version=true`、
///    `previous_version = Some(old.version)`、`seeded_by="evolution_release"`、
///    `content` = proposal.diff_snippet（W4 简化路径：把整段 diff_snippet 当成新 content）
```

改为：

```
/// 3.5. 合成 new_content = compose_appended_content(current.content, diff_snippet)
///      （末尾追加,原红线正文逐字保留）→ 过 validate_prompt_edit（禁词+锚点闸）
///      + review_prompt_edit（LLM 语义闸）；任一拒则 RedlineGateRejected,不写库
/// 4. insert 新一条 `version = old.version + 1`、`current_version=true`、
///    `previous_version = Some(old.version)`、`seeded_by="evolution_release"`、
///    `content` = new_content（原文 + 追加片段）
```

- [ ] **Step 2: 改 new_content 合成 + 插入三闸校验**

在 `src/evolution/release.rs` 把第 225-229 行的：

```rust
    let new_content = proposal.diff_snippet.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing diff_snippet (W4 release path requires a complete content body): {proposal_id}"
        ))
    })?;
```

改为（保留取 snippet，但语义变为「追加片段」）：

```rust
    let append_snippet = proposal.diff_snippet.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing diff_snippet: {proposal_id}"
        ))
    })?;
```

然后在加载 `current`（第 234-251 行 `let current = ...?;` 块）**之后**、`let old_version = current.version;`（第 252 行）**之前**，插入三闸校验：

```rust
    // ── 红线三闸（与人工编辑路径同源,从 prompt_guard 复用）──
    // 末尾追加:原 prompt 正文逐字保留,critic 片段追加到末尾。
    let new_content = crate::prompt_guard::compose_appended_content(&current.content, &append_snippet);
    // 闸 1+2:禁词 + 锚点完整性（原文保留 → 锚点天然过;不过说明原 prompt 已缺锚,fail-closed 正确）
    crate::prompt_guard::validate_prompt_edit(&prompt_key, &new_content)
        .map_err(EvolutionError::RedlineGateRejected)?;
    // 闸 3:LLM 语义审查追加增量（变相真人转介/削弱 grounding 等语义绕过）
    match crate::prompt_guard::review_prompt_edit(
        state,
        &workspace_id,
        &prompt_key,
        &current.content,
        &new_content,
    )
    .await
    {
        crate::prompt_guard::PromptEditVerdict::Pass => {}
        crate::prompt_guard::PromptEditVerdict::Reject(reason) => {
            return Err(EvolutionError::RedlineGateRejected(format!(
                "LLM 语义闸拒绝:{reason}"
            )));
        }
        crate::prompt_guard::PromptEditVerdict::NeedsHumanConfirm { reason, .. } => {
            // LLM 不可用 → 不 fail-open 放水,不 fail-closed 死路:本次 release 中止,
            // 要求管理员逐字核对后再确认（具体 UI 交互见阶段三）。
            return Err(EvolutionError::RedlineGateRejected(format!(
                "红线语义审查暂不可用,请逐字核对后再发布:{reason}"
            )));
        }
    }
```

> 注：`workspace_id` 已在第 231 行 `let workspace_id = proposal.workspace_id.clone();` 取到，`prompt_key` 在第 220 行取到，`current.content` 字段已核实存在（models.rs:925 `pub content: String`）。第 295 行 `"content": &new_content` 引用的 `new_content` 现在指向合成后的值，无需改。

- [ ] **Step 3: 隔离脚本验证（release.rs 在 evolution/ 扫描区）**

Run: `bash scripts/check-evolution-isolation.sh`
Expected: ok（新增的 `crate::prompt_guard::` 引用不在 8 个禁字面量内；不含 `crate::agent::gateway` 等）。

- [ ] **Step 4: no-human-takeover 验证（release.rs 在扫描区）**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: ok（新增行里「真人转介」「语义审查」等不命中禁词表；禁词表是 `人工接管|接管|人工` 等，注释里的「真人」不在表内——但仍逐字确认 diff 无 `人工`/`接管`）。若命中，把注释措辞改为不含禁词的表述。

- [ ] **Step 5: 编译验证**

Run: `cargo check`
Expected: 编译通过。`release_prompt` 现在是 `async` 调 `review_prompt_edit().await`，函数签名已是 `async`，无需改。

- [ ] **Step 6: 跑全 lib 基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 7: Commit**

```bash
git add src/evolution/release.rs
git commit -m "fix(evolution): release_prompt 接三道红线闸 + snippet 改末尾追加

堵 G2/G3:发布路径不再绕过禁词/锚点/LLM 语义闸,且 diff_snippet 改为
追加到原 prompt 末尾（原红线正文逐字保留,锚点天然过）而非整篇覆盖。
任一闸拒返回 RedlineGateRejected。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: release_prompt 三闸集成测试（红线被拒验证）

构造「diff_snippet 含禁用词」「current 缺锚点」两类候选，验证 release_prompt 被三闸拦下返回 `RedlineGateRejected`，不写 prompt_templates。需 Docker（testcontainers MongoDB），属 `#[ignore]` 集成测试，留 CI 跑。

**Files:**
- Test: `tests/evolution_release_redline.rs`（新建）

**Interfaces:**
- Consumes: `wechatagent::evolution::release::release_prompt`（确认 release_prompt 是 `pub`——release.rs:195 `pub async fn`，✓）；`wechatagent::prompt_guard`；测试辅助 testcontainers 模式参照现有集成测试。

- [ ] **Step 1: 先确认现有集成测试的 Mongo 启动 + seed 模式**

Run: `grep -rln "testcontainers\|GenericImage\|mongo" tests/ | head -5`
然后 Read 其中一个涉及 proposals/prompt_templates 的集成测试，照搬其 `setup`（连库 → `migrations::run` → `ensure_indexes` → seed prompt pack）。**严格按 memory `project_config_seed_in_prompts_not_migrations`：底座行由 `prompts::ensure_prompt_pack_v2` 种,helper 用 `replace_one` upsert 不要 blind `insert_one`（撞 unique 索引 E11000）。**

- [ ] **Step 2: 写测试骨架 + 禁用词候选用例**

```rust
#![cfg(feature = "...")] // 按现有集成测试惯例（若用 cfg）
// 用 #[ignore] 标注,本地不跑,CI --ignored 跑。

#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn release_prompt_rejects_forbidden_word_snippet() {
    // 1. 起 mongo + 连库 + migrations::run + ensure_indexes + ensure_prompt_pack_v2
    //    （照搬现有集成测试 setup）
    // 2. 种一条 proposal: proposal_kind="prompt", status="eligible_for_release",
    //    proposed_template_key="user.reply.policy",
    //    diff_snippet = 字符拼接构造含禁用词的片段（绕字面量 lint）
    //    let forbidden = ["人","工","接","管"].concat();
    //    diff_snippet = Some(format!("遇到难题就{forbidden}给后台"))
    // 3. 调 release_prompt(&state, proposal_id, "admin")
    // 4. assert 返回 Err(EvolutionError::RedlineGateRejected(_))
    // 5. assert prompt_templates 没有新版本写入（version 仍是原值,current 未变）
    // 6. assert proposal.status 仍是 eligible_for_release（未推进 released）
}
```

- [ ] **Step 3: 写「合法追加片段放行」对照用例**

```rust
#[tokio::test]
#[ignore = "requires docker mongodb"]
async fn release_prompt_accepts_clean_append_snippet() {
    // 同 setup。diff_snippet = "补充:本行业语气更稳重。"（无禁词,纯业务追加）
    // 注意:review_prompt_edit 第三闸会真调 LLM——集成测试需配 OPENAI_API_KEY,
    // 或 mock。若 CI 无 key,本用例可拆到 real-llm 套件,或断言到「过了前两闸进入
    // 第三闸」即可（前两闸纯函数确定）。具体留实现时按 CI provider 决定。
    // assert: 若 LLM 可用且判 Pass → prompt_templates 有 version+1 新版本,
    //         current.content 以原文开头、以追加片段结尾。
}
```

> 实现注记：第三闸真调 LLM 使集成测试不确定。优先策略——把「前两闸（禁词+锚点）拒绝」做成确定性集成测试（不触发第三闸，因为禁词在闸 1 就被拦）；「合法放行」用例依赖 LLM，归 real-llm CI 套件或标注允许 LLM 不可用时降级断言。

- [ ] **Step 4: 本地编译验证（不跑 ignored）**

Run: `cargo test --test evolution_release_redline -- --list`
Expected: 列出 2 个测试名，编译通过（不实际跑，本地无 Docker）。

- [ ] **Step 5: Commit**

```bash
git add tests/evolution_release_redline.rs
git commit -m "test(evolution): release_prompt 三闸集成测试（禁词候选被拒+合法放行）

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: 阶段一收尾 — 基线 + 双 lint 全绿确认

**Files:** 无改动，纯验证。

- [ ] **Step 1: 全 lib 基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 2: 四个 PBT 文件**

Run: `cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter`
Expected: 累计 ≥ 33 passed / 0 failed。

- [ ] **Step 3: 隔离 + no-human-takeover 双 lint**

Run: `bash scripts/check-evolution-isolation.sh && bash scripts/check-no-human-takeover.sh`
Expected: 两个都 ok。

- [ ] **Step 4: cargo check --tests（复刻 CI step2，本地 lib 测试不编译集成测试会漏 E0063 等）**

Run: `cargo check --tests`
Expected: 编译通过（按 memory `config_field_add_test_helpers`：本地 `cargo test --lib` 不编译集成测试，必须 `cargo check --tests` 复刻 CI）。

> 阶段一到此完成。G2（绕过三闸）+ G3（snippet 整篇覆盖）红线缺口已堵。可在此切 PR 合并，不依赖阶段二/三。

---

## 阶段二：证据闭环（骨架，待阶段一落地后细化）

> 阶段二涉及改 `decision.rs` / `review/mod.rs` 多处签名加 `prompt_override`，真实代码量大且需逐一核实当前签名（13+ 入参）。本章先锁定目标/文件/接口/验收，**待阶段一合并后**用同一份 plan 文档补全 bite-sized TDD 步骤（届时重新 Read decision.rs/review/mod.rs/simulation.rs 确认签名无变化）。

### 目标
对 cohort.prompt 每条历史失败 run，用「原 prompt + 追加候选片段」跑真模型 Reply+Review，记新旧 `review.scores` 对照存入 proposal，**作为人工 release 的参考证据，不自动放行**。修 G1（replay placeholder）+ G4（假基线）。

### 文件结构
- **Create** `src/agent/prompt_shadow.rs` — 与 `simulation.rs` 同层（必须在 `src/agent/`，因为要引用 `gateway` 的非发送 helper `load_context_messages`/`load_pending_tasks`，evolution/ 引这些会被隔离脚本拦）。暴露 `pub(crate) async fn shadow_replay_prompt_candidate(state, proposal, source_run_id, prompt_override) -> AppResult<PromptShadowOutcome>`。
- **Modify** `src/agent/mod.rs` — 加 `mod prompt_shadow;` + 必要 re-export。
- **Modify** `src/agent/decision.rs` — `decide_reply_with_promote`（:268，14 入参）加可选入参 `prompt_override: Option<&PromptOverride>`，注入到 `assemble_system_prompt`（:704）之前；定义 `PromptOverride { reply_policy_append: Option<String>, ... }` 结构。**现有所有调用点传 `None`（字节等价护栏）。**
- **Modify** `src/agent/review/mod.rs` — `review_decision`（:253，13 入参）同加 `prompt_override` 可选入参。
- **Modify** `src/evolution/replay.rs:222` — prompt 分支从 `ReplayOutcome::failed("prompt_replay_not_implemented_w3")` 改为调 `crate::agent::prompt_shadow::shadow_replay_prompt_candidate(...)`（调 agent 模块入口，不引入 8 禁符号 → 隔离合规）。
- **Modify** `src/evolution/significance.rs:219/424/445` — `grade_prompt` 改存对照证据到 `proposal.eval_metrics`（不自动放行）；修 `original_self_critique_for_metric`/`original_5gate_hit_or_default` 从源 run 真实数据取（G4）；`aggregate_and_grade` 对 prompt 候选 shadow 完成即置 `eligible_for_release`（语义重定为「证据就绪等人工」）。
- **Modify** `src/models.rs:4286` — `Proposal.status` 注释补 `eligible_for_release` 的 prompt 语义说明（不改类型，不加新态）。

### 接口（待阶段一落地后核实签名补全）
- `pub(crate) async fn shadow_replay_prompt_candidate(...) -> AppResult<PromptShadowOutcome>`
- `pub struct PromptShadowOutcome { completed: usize, failed: usize, per_sample: Vec<SampleComparison> }`
- `pub struct PromptOverride { reply_policy_append: Option<String> }`（注入点：load_prompt_for_contact 加载 user.reply.policy 后追加）

### 关键约束（已核实，写步骤时遵守）
- **prompt_override=None 字节等价**：现有所有 `decide_reply_with_promote` / `review_decision` 调用点（gateway/planner/simulation）传 None，行为与改造前逐字相同 → 反过拟合护栏。写完后必须 `cargo test --lib` 基线不动 + 抽查一条现有 decision 测试快照不变。
- **shadow 复用 simulation 加载链**：`load_operation_playbook_for_contact`/`load_or_create_operating_memory`/`load_operation_knowledge`/`load_context_messages`/`route_operation_knowledge`/`select_operation_knowledge_chunks`；源 run inbound 经 `AgentRunLog.context.inboundMessageId`（replay.rs:196 既有读法）反查 `messages()`；inbound 已清理 → 记 failed `source_message_unavailable`。
- **预算**：shadow 真模型计入 `EvolutionBudget`（`budget.record_call`，prompt_critic.rs:137 同款）；耗尽停止后续，候选留 `pending_eval` 下 tick 续跑。
- **隔离脚本**：`prompt_shadow.rs` 放 `src/agent/` 非 evolution/；replay.rs 只调其入口。

### 验收
- `cargo test --lib` 基线不回归；隔离 + no-human-takeover 双 lint 绿。
- prompt 候选能走完 critic → shadow → eligible_for_release（不再卡 G1 placeholder）。
- grade_prompt 的 original 侧基线非恒 None/false（G4 修复，加单测锁定）。
- 一条 prompt 候选 shadow 跑完后 `proposal.eval_metrics` 含 per-sample 新旧五闸对照。

---

## 阶段三：前端证据展示 + 人工 release（骨架，待阶段二落地后细化）

> 前端依赖阶段二产出的 `proposal.eval_metrics` 证据结构。**待阶段二落地后**补全 bite-sized 步骤（届时确认 eval_metrics 的真实 JSON 形状 + 现有 EvolutionCenterTab 组件结构）。前端改动须遵守 `docs/frontend-design-system.md` + memory `frontend_follow_design_system`（真实 token 在 `components/ui/tokens.css`，CSS 用 `.module.css`，蓝仅主操作/紫仅 AI 身份）。

### 目标
候选详情页展示「原 prompt vs 新 prompt 在 N 条历史样本上的五闸/自评对照表」+ critic reasoning + 追加的 diff 片段；管理员看完点 release（复用现有确认串 `RELEASE`，UI 已存在 routes/evolution.rs:149）。

### 文件结构（待核实现有组件后定）
- **Modify** `frontend/src/features/evolution/EvolutionCenterTab.tsx` — 候选列表项加「查看对照证据」入口。
- **Create/Modify** `frontend/src/components/review/ProposalReleaseCard.tsx`（spec 提及，需先确认是否已存在）— 对照表 + release 按钮 + 确认串输入。
- **Modify** `frontend/src/components/review/proposalTypes.ts:7-9` — 若 `eligible_for_release` 语义变化加注释（不改联合类型）。

### 关键约束
- **`proposalTypes.ts` status 联合类型是闭集**（:7-9 显式列 `pending_eval | eligible_for_release | ...`），复用 `eligible_for_release` 不加新态 → 不破类型。
- **RedlineGateRejected 展示**：阶段一加的错误类型，前端 release 失败时显示「该候选触碰红线，已拒绝发布」+ reason。
- **NeedsHumanConfirm（LLM 不可用）交互**：阶段一 release_prompt 此情况返回 RedlineGateRejected（「请逐字核对后再发布」），前端引导管理员重试或人工核对——具体交互此处细化。

### 验收
- 前端能渲染对照表（mock eval_metrics 数据先行）。
- release 成功/被拒/LLM 不可用三态都有明确 UI 反馈。
- `cd frontend && npm run build` 通过；遵守设计系统。

---

## Self-Review（阶段一）

**Spec coverage（阶段一部分）：**
- spec §4.A（三闸下沉）→ Task 1 + Task 2 ✓
- spec §4.B（release 接三闸 + snippet 末尾追加）→ Task 4（合成纯函数）+ Task 5（接闸）✓
- spec §4.B EvolutionError 扩展 → Task 3 ✓
- spec §6（错误处理：三闸拒 / LLM 不可用）→ Task 5 Step 2 三态处理 ✓
- spec §7（测试策略：三闸下沉行为不变 / release 三闸集成 / 基线不回归 / 双 lint）→ Task 1 Step 4 + Task 6 + Task 7 ✓
- spec §4.C/D（shadow + 显著性）→ 阶段二骨架（按用户决定先出骨架）
- spec §4.D 前端 → 阶段三骨架

**Placeholder scan：** 阶段一 Task 1-7 每步都有具体代码/命令/预期；阶段二/三显式标注为「骨架，待落地后细化」（用户已批准此粒度），非占位符遗漏。

**Type consistency：** `validate_prompt_edit(template_key, new_content) -> Result<(), String>` / `review_prompt_edit(state, workspace_id, template_key, old, new) -> PromptEditVerdict` / `compose_appended_content(current, snippet) -> String` / `EvolutionError::RedlineGateRejected(String)` 在 Task 1/3/4/5 间引用一致，均与已核实的真实签名（management_prompt_edit.rs:64/119、release.rs:195、error.rs）对齐。

**注**：Task 6 的「合法放行」集成用例依赖 LLM 不确定性，已在步骤内标注降级策略（前两闸确定性测试为主），非遗漏。

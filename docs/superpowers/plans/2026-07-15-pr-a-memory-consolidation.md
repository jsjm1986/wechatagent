# PR-A: 记忆层 consolidation 三缺陷修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 memory consolidation 落库段的三个已审计确认 Medium 缺陷（A-01 core_facts 缺证据门 / A-02 confirmed_tags replace 丢标签 / A-03 跨集合写非原子重放）。

**Architecture:** 三条同处 `consolidate_contact_memory` 的 OCC winner 落库段（`src/agent/memory.rs`）。核心逻辑抽为纯函数（`merge_confirmed_tags` / `parse_discarded_tags` / A-01 的 importance 天花板判定）用 lib 单测覆盖；A-03 是控制流改动（`?`→fail-soft）。

**Tech Stack:** Rust 2021 / Axum / MongoDB。测试用 `cargo test --lib`（纯函数，无 Docker）。

## Global Constraints

- **DEFAULT 销售域字节等价**：所有改动在 DEFAULT/常规数据下行为不变，仅非默认/异常场景生效。
- **反过拟合红线**：不对单条对话点修；阈值/判据以抽象方法论沉淀（纯函数 + 常量 + 多情形单测）。
- **三线隔离铁律**：consolidation 落库 `$set` 只碰 `confirmed_tags` / `personality_profile`，绝不碰 `manual_tags`（运营权威层）/ `bayesian_signals`。
- **lint 门**：新增行不得含 `check-no-human-takeover` 禁词（human_takeover/接管/人工 等）、`check-no-model-hint` 禁词（anthropic/gpt-N/claude-N/deepseek-X）。
- **本地验证**（PR#217 教训）：提交前必跑 `cargo test --lib` + `RUSTFLAGS="-D warnings" cargo check --tests`（复刻 baseline step2，兜 must_use 等 warning）。
- 分支 `fix/audit-medium-batch1`（基于 origin/main fee3115）。

---

### Task 1: A-02 —— confirmed_tags 合并纯函数 + 消费 discardedTags

**Files:**
- Modify: `src/agent/memory.rs`（新增 `parse_discarded_tags` + `merge_confirmed_tags` 两纯函数；改 consolidation 落库段 :1553/:1621 消费之）
- Test: `src/agent/memory.rs`（`#[cfg(test)]` 内联单测）

**Interfaces:**
- Consumes: `ConfirmedTag`（models.rs:190，字段 `value: String` / `evidences` / `confirmed_at` / `confirmed_by`）；`parse_reconfirmed_tags(value, window) -> Vec<ConfirmedTag>`（memory.rs:1062）；`contact.confirmed_tags: Vec<ConfirmedTag>`。
- Produces: `parse_discarded_tags(value: &serde_json::Value) -> std::collections::HashSet<String>`；`merge_confirmed_tags(old: &[ConfirmedTag], reconfirmed: Vec<ConfirmedTag>, discarded: &HashSet<String>) -> Vec<ConfirmedTag>`。

- [ ] **Step 1: 写失败测试（合并语义）**

在 `src/agent/memory.rs` 的 `#[cfg(test)] mod tests`（文件末尾既有测试模块）内加：

```rust
#[test]
fn merge_confirmed_tags_keeps_old_unless_discarded() {
    use std::collections::HashSet;
    let now = DateTime::now();
    let mk = |v: &str| ConfirmedTag {
        value: v.to_string(),
        evidences: vec![],
        confirmed_at: now,
        confirmed_by: "test".to_string(),
    };
    let old = vec![mk("价格敏感"), mk("预算充足"), mk("已婚")];
    // 本轮只重判出「价格敏感」（带新证据），LLM 显式弃用「已婚」。
    let reconfirmed = vec![mk("价格敏感")];
    let discarded: HashSet<String> = ["已婚".to_string()].into_iter().collect();

    let merged = merge_confirmed_tags(&old, reconfirmed, &discarded);
    let vals: HashSet<String> = merged.iter().map(|t| t.value.clone()).collect();
    // 价格敏感=reconfirmed 保留；预算充足=旧且未弃用→保留；已婚=显式弃用→移除。
    assert!(vals.contains("价格敏感"));
    assert!(vals.contains("预算充足"));
    assert!(!vals.contains("已婚"));
    assert_eq!(merged.len(), 2);
}

#[test]
fn merge_confirmed_tags_reconfirmed_wins_on_duplicate() {
    use std::collections::HashSet;
    let now = DateTime::now();
    let old = vec![ConfirmedTag { value: "价格敏感".into(), evidences: vec![], confirmed_at: now, confirmed_by: "old".into() }];
    let reconfirmed = vec![ConfirmedTag { value: "价格敏感".into(), evidences: vec![], confirmed_at: now, confirmed_by: "consolidation".into() }];
    let discarded: HashSet<String> = HashSet::new();
    let merged = merge_confirmed_tags(&old, reconfirmed, &discarded);
    // 同名以 reconfirmed 为准（不重复），confirmed_by 取新值。
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].confirmed_by, "consolidation");
}

#[test]
fn merge_confirmed_tags_empty_discarded_supersets_reconfirmed() {
    // DEFAULT 等价性守护：旧全部保留 + reconfirmed，无静默丢失。
    use std::collections::HashSet;
    let now = DateTime::now();
    let mk = |v: &str| ConfirmedTag { value: v.into(), evidences: vec![], confirmed_at: now, confirmed_by: "t".into() };
    let old = vec![mk("a"), mk("b")];
    let reconfirmed = vec![mk("c")];
    let merged = merge_confirmed_tags(&old, reconfirmed, &HashSet::new());
    let vals: HashSet<String> = merged.iter().map(|t| t.value.clone()).collect();
    assert_eq!(vals, ["a","b","c"].iter().map(|s| s.to_string()).collect());
}

#[test]
fn parse_discarded_tags_extracts_values() {
    let v = serde_json::json!({ "discardedTags": [ {"value":"已婚","reason":"客户改口"}, {"value":"","reason":"空"}, {"value":"预算低","reason":"推翻"} ] });
    let d = parse_discarded_tags(&v);
    assert!(d.contains("已婚"));
    assert!(d.contains("预算低"));
    assert!(!d.contains("")); // 空 value 不进集合
    assert_eq!(d.len(), 2);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib merge_confirmed_tags parse_discarded_tags 2>&1 | tail -20`
Expected: 编译失败 `cannot find function merge_confirmed_tags` / `parse_discarded_tags`。

- [ ] **Step 3: 写两个纯函数**

在 `src/agent/memory.rs` 的 `parse_reconfirmed_tags`（:1101 结束）之后插入：

```rust
/// A-02：解析 LLM 输出的 `discardedTags:[{value,reason}]`，取被显式推翻的标签 value 集合。
/// 与 `parse_reconfirmed_tags` 平行，但不需要 window/证据锚（弃用是显式动作，不必再佐证）。
pub(crate) fn parse_discarded_tags(value: &serde_json::Value) -> std::collections::HashSet<String> {
    value
        .get("discardedTags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let v = item.get("value")?.as_str()?.trim().to_string();
                    if v.is_empty() { None } else { Some(v) }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A-02：把「本轮重判保留的标签」与「旧确信标签中未被显式弃用的」合并。
/// 对称 core_facts 的「未显式弃用即自动保留」保护（prompts.rs:1461）：旧确信标签
/// 除非被 LLM 列入 discardedTags 显式推翻，否则保留——修掉「证据滚出截断窗口即静默丢
/// 持久标签」。reconfirmed 为本轮权威，同名以其为准。
pub(crate) fn merge_confirmed_tags(
    old: &[ConfirmedTag],
    reconfirmed: Vec<ConfirmedTag>,
    discarded: &std::collections::HashSet<String>,
) -> Vec<ConfirmedTag> {
    let reconfirmed_values: std::collections::HashSet<String> =
        reconfirmed.iter().map(|t| t.value.clone()).collect();
    let mut merged = reconfirmed;
    for tag in old {
        // 旧标签：未在本轮重判覆盖（同名以 reconfirmed 为准）且未被显式弃用 → 保留。
        if !reconfirmed_values.contains(&tag.value) && !discarded.contains(&tag.value) {
            merged.push(tag.clone());
        }
    }
    merged
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib merge_confirmed_tags parse_discarded_tags 2>&1 | tail -20`
Expected: 4 个测试 PASS。

- [ ] **Step 5: 在 consolidation 落库段接入合并**

`src/agent/memory.rs` 当前 :1553：
```rust
    let reconfirmed = parse_reconfirmed_tags(&value, &window);
```
改为（保留原注释，追加合并）：
```rust
    let reconfirmed = parse_reconfirmed_tags(&value, &window);
    // A-02：消费 discardedTags——旧确信标签除非被 LLM 显式弃用否则保留，
    // 对称 core_facts「未显式弃用即保留」，修掉证据滚出截断窗口即静默丢标签。
    let discarded = parse_discarded_tags(&value);
    let reconfirmed = merge_confirmed_tags(&contact.confirmed_tags, reconfirmed, &discarded);
```

当前 :1621（在 A-03 Task 3 会再改此行的错误传播，本步只改值来源，`reconfirmed` 变量名不变故 :1621 `to_bson(&reconfirmed)?` 无需动）。

- [ ] **Step 6: 编译确认接入无误**

Run: `cargo build --lib 2>&1 | tail -15`
Expected: 编译通过（`reconfirmed` shadowing 合法；`contact` 在此作用域可用——已在函数签名 `contact: &Contact`）。

- [ ] **Step 7: 提交**

```bash
git add src/agent/memory.rs
git commit -m "fix(memory): A-02 consolidation 消费 discardedTags 保留旧确信标签

confirmed_tags 从整体 replace 改为「保留 unless 显式弃用」,对称 core_facts。
新增 parse_discarded_tags/merge_confirmed_tags 纯函数 + 4 单测。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: A-01 —— 弱证据候选 importance 天花板

**Files:**
- Modify: `src/agent/memory.rs`（`validated_memory_candidate` :1897-1913 加弱证据判定 + 抽纯函数 `evidence_capped_importance`）
- Test: `src/agent/memory.rs`（`#[cfg(test)]` 内联单测）

**Interfaces:**
- Consumes: `decide_candidate_status(write_score, max_importance)`（memory.rs:1887，`IMPORTANCE_RESCUE_THRESHOLD=8`）；`validated_memory_candidate(Document) -> Option<Document>`（:1897）。
- Produces: `evidence_capped_importance(evidence: &str, importance: i32) -> i32`（弱 evidence 时 clamp 到 < 8）。

- [ ] **Step 1: 写失败测试**

在 `#[cfg(test)] mod tests` 内加：

```rust
#[test]
fn evidence_capped_importance_caps_weak_evidence() {
    // 弱证据（空/极短）+ 高自报 importance → clamp 到 < IMPORTANCE_RESCUE_THRESHOLD(8)，
    // 使其无法凭 max_importance>=8 走 pending 救援通道。
    assert!(evidence_capped_importance("", 10) < 8);
    assert!(evidence_capped_importance("   ", 9) < 8);
    assert!(evidence_capped_importance("嗯", 10) < 8); // 极短
}

#[test]
fn evidence_capped_importance_keeps_substantial() {
    // 充实证据（正常运营记忆都有原话）→ importance 原样不动。
    let ev = "客户明确说预算有500万，要买三套";
    assert_eq!(evidence_capped_importance(ev, 10), 10);
    assert_eq!(evidence_capped_importance(ev, 5), 5);
}

#[test]
fn evidence_capped_importance_does_not_raise() {
    // 只降不升：弱证据低 importance 保持原值（不因 cap 反而抬高）。
    assert_eq!(evidence_capped_importance("", 3), 3);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib evidence_capped_importance 2>&1 | tail -15`
Expected: 编译失败 `cannot find function evidence_capped_importance`。

- [ ] **Step 3: 写纯函数**

在 `validated_memory_candidate`（memory.rs:1897）之前插入：

```rust
/// A-01：弱证据候选的 importance 天花板。core_facts 建立仅凭 LLM 自评（候选 evidence
/// 是自由文本、无 evidenceTurns，无法像 tags/personality 那样 resolve_evidence 锚定）。
/// 折中：evidence 文本过弱（trim 后长度 < 阈值）时，即使 LLM 自报高 importance 也 clamp
/// 到 < IMPORTANCE_RESCUE_THRESHOLD(8)，使其无法凭「max_importance>=8」走 pending 救援
/// 通道（decide_candidate_status），仍可走 write_score>=6 常规通道。只降不升。
const WEAK_EVIDENCE_MIN_LEN: usize = 4;
const IMPORTANCE_CAP_WHEN_WEAK: i32 = 7; // < IMPORTANCE_RESCUE_THRESHOLD(8)

pub(crate) fn evidence_capped_importance(evidence: &str, importance: i32) -> i32 {
    if evidence.trim().chars().count() < WEAK_EVIDENCE_MIN_LEN {
        importance.min(IMPORTANCE_CAP_WHEN_WEAK)
    } else {
        importance
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib evidence_capped_importance 2>&1 | tail -15`
Expected: 3 个测试 PASS。

- [ ] **Step 5: 在 validated_memory_candidate 接入**

`src/agent/memory.rs` 当前 :1901（`validated_memory_candidate` 内）：
```rust
    let importance = doc_i32(Some(&candidate), "importance", 0).clamp(0, 10);
```
改为：
```rust
    let importance = doc_i32(Some(&candidate), "importance", 0).clamp(0, 10);
    // A-01：弱证据候选设 importance 天花板，堵住「空/极短 evidence + 高自报 importance」
    // 凭救援阈值涌入 pending 的噪声通道。
    let importance = evidence_capped_importance(&evidence, importance);
```
（`evidence` 变量已在 :1900 定义 `let evidence = doc_string(&candidate, "evidence")?;`）

- [ ] **Step 6: 编译 + 跑相关既有测试**

Run: `cargo test --lib decide_candidate_status validated 2>&1 | tail -15`
Expected: 编译通过，既有 decide_candidate_status 测试仍 PASS。

- [ ] **Step 7: 提交**

```bash
git add src/agent/memory.rs
git commit -m "fix(memory): A-01 弱证据候选 importance 天花板

evidence 空/极短的候选即使 LLM 自报高 importance 也 clamp 到 <8,
堵住噪声凭救援阈值涌入 pending。新增 evidence_capped_importance 纯函数+3单测。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: A-03 —— confirmed_tags 写改 fail-soft

**Files:**
- Modify: `src/agent/memory.rs`（consolidation 落库段 :1612-1624 confirmed_tags 写从硬 `?` 改 fail-soft warn）

**Interfaces:**
- Consumes: 无新纯函数。控制流改动，对齐同段 personality 写（:1656-1688）的 fail-soft 姿势。

- [ ] **Step 1: 改错误传播为 fail-soft**

`src/agent/memory.rs` 当前 :1612-1624：
```rust
    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "wxid": &contact.wxid,
            },
            doc! { "$set": { "confirmed_tags": to_bson(&reconfirmed)? } },
            None,
        )
        .await?;
```
改为（`to_bson` 与 `.await` 均进 match，失败 warn 不返 Err）：
```rust
    // A-03：confirmed_tags 是 memory_card 之后的 best-effort 搭车写。memory_card OCC
    // 已是权威落库（上面 modified_count==1 才到这），若此处硬 `?` 失败会让整函数返 Err→
    // 候选不被标 consolidated→task retry→候选二次并入已推进的卡（重放）。故改 fail-soft
    // warn（对齐下方 personality 写），失败不触发整轮重放。
    match to_bson(&reconfirmed) {
        Ok(tags_bson) => {
            if let Err(err) = state
                .db
                .contacts()
                .update_one(
                    doc! {
                        "workspace_id": &contact.workspace_id,
                        "account_id": &contact.account_id,
                        "wxid": &contact.wxid,
                    },
                    doc! { "$set": { "confirmed_tags": tags_bson } },
                    None,
                )
                .await
            {
                tracing::warn!(
                    workspace_id = %contact.workspace_id,
                    contact_wxid = %contact.wxid,
                    error = %err,
                    "confirmed_tags write-back failed (fail-soft; memory_card already persisted)"
                );
            }
        }
        Err(err) => {
            tracing::warn!(
                workspace_id = %contact.workspace_id,
                contact_wxid = %contact.wxid,
                error = %err,
                "confirmed_tags to_bson failed (fail-soft)"
            );
        }
    }
```

- [ ] **Step 2: 编译确认**

Run: `cargo build --lib 2>&1 | tail -15`
Expected: 编译通过。

- [ ] **Step 3: 全量 lib 测试回归**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: `test result: ok. NNNN passed; 0 failed`（≥ baseline 350，实际 2000+）。

- [ ] **Step 4: 提交**

```bash
git add src/agent/memory.rs
git commit -m "fix(memory): A-03 confirmed_tags 写改 fail-soft 消除重放

memory_card OCC 写成功后,confirmed_tags 是 best-effort 搭车写;硬 ? 失败会
触发 task retry→候选二次并入卡。改 fail-soft warn(对齐 personality 写)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 本地门 + 推送 + PR + 合并

- [ ] **Step 1: 复刻 baseline step2（-D warnings）**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -20`
Expected: 无 error（PR#217 教训：must_use 等 warning 会在此升 error）。若超时，后台跑等通知。

- [ ] **Step 2: 全量 lib 测试**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: 0 failed，passed 数 ≥ 之前基线（新增 7 个纯函数单测）。

- [ ] **Step 3: 推送（显式 refspec）+ 亲验 tip**

```bash
git push origin HEAD:refs/heads/fix/audit-medium-batch1
git ls-remote origin refs/heads/fix/audit-medium-batch1  # 须 == 本地 HEAD
```

- [ ] **Step 4: 建 PR（显式 --head/--base）**

```bash
gh pr create --head fix/audit-medium-batch1 --base main \
  --title "fix(memory): 审计 Medium A-01/A-02/A-03 consolidation 三缺陷" \
  --body "见 docs/superpowers/specs/2026-07-15-audit-medium-remediation-design.md PR-A 段"
```
建后 `gh pr view --json headRefOid,baseRefName` 核 head==本地 HEAD、base==main。

- [ ] **Step 5: 后台监控 CI，全绿（Baseline+Integration+三 lint）后 squash merge（不带 --delete-branch）**

```bash
gh pr merge <N> --squash
git fetch origin main && git rev-parse origin/main  # 核 mergeCommit 进 main
```

- [ ] **Step 6: 标记 memory**：更新 project_agent_capabilities_audit.md 标 A-01/A-02/A-03 已修（在 Task 241 收官时统一做）。

## Self-Review

- **Spec 覆盖**：A-01（Task 2）/ A-02（Task 1）/ A-03（Task 3）三条全覆盖，与设计文档 PR-A 段一致。
- **占位符**：无 TBD/TODO；每步含真实代码或真实命令。
- **类型一致**：`ConfirmedTag`（value:String）、`parse_reconfirmed_tags -> Vec<ConfirmedTag>`、`merge_confirmed_tags` 签名在 Task 1 定义并在落库段消费；`evidence_capped_importance` 在 Task 2 定义并在 :1901 消费——均亲验自当前代码。
- **落库顺序**：Task 1 改 :1553（值来源）、Task 3 改 :1612-1624（错误传播），两者不冲突（`reconfirmed` 变量名不变）；建议按 Task 1→2→3 顺序（A-02 先，A-03 最后碰同段错误传播）。

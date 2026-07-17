# PR-E: post_release 5 闸映射对调 + pressure 口径修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 evolution post_release 观测面板 [2-01]：`FIVE_GATE_KEYS` 把 fact_risk/pressure_risk 的 status 映射对调（与 threshold/significance 两权威源相反），面板命中率贴错标签；且 pressure_risk 是软闸不产 block 终态，其命中率口径本身无意义。

**Architecture:** post_release 复用 significance 的权威 `safety_block_status_for` 修正 fact_risk 对调；pressure_risk 改用 `revision_failed` 口径（与已有的 human_like/emotional rewrite 两闸同口径，反映软闸真实终态）。纯观测改动，不反哺任何自动决策。

**Tech Stack:** Rust 2021。测试用 `cargo test --lib`。

## Global Constraints

- **纯观测**：post_release delta 只写 agent_events details 供 admin 察觉，不参与 promote/rollback（post_release.rs:65-66 明示）。改映射不影响任何自动放量/回滚决策。
- **权威源单一化**：安全闸映射以 `significance::SAFETY_GATE_BLOCK_STATUS`/`safety_block_status_for`（significance.rs:52/59）为唯一权威——threshold.rs:65-72 `classify_gate_hit` 反向映射与之一致，均已亲验。
- **lint 门**：post_release.rs 属 `src/evolution/`，受 `check-no-human-takeover` + `check-evolution-isolation`（不得引 outbox/mcp）+ `check-no-model-hint` 三门。本改动不新增禁词、不引隔离禁引用。
- **本地验证**：`cargo test --lib` + `RUSTFLAGS="-D warnings" cargo check --tests`。
- 分支：PR-A/PR-C 合并后基于最新 origin/main 新起 `fix/audit-medium-e`（或续用批次分支）。

## 事实基线（主控亲验，2026-07-15 fee3115）

三份映射对照（gate_key → final_review_status）：

| gate_key | post_release.rs:54-60（错） | significance.rs:52-56（对） | threshold.rs:65-72（对，反向） |
| --- | --- | --- | --- |
| fact_risk_block | `blocked_by_safety_guard` ✗ | `held_by_ai_policy` | `held_by_ai_policy` |
| pressure_risk_block | `held_by_ai_policy` ✗ | `blocked_by_safety_guard` | `blocked_by_safety_guard` |
| product_accuracy_score_block | `blocked_unverified_product_claim` ✓ | 同 | 同 |
| human_like_score_rewrite | `revision_failed`（post_release 独有 rewrite 口径） | 不在安全表 | rewrite 类 revision_applied 补判 |
| emotional_value_rewrite | `revision_failed` | 不在安全表 | 同 |

生产真相（gates.rs 亲验）：fact_risk 硬闸→`held_by_ai_policy`；`blocked_by_safety_guard` 来自产品声明 fail-closed + relay。故 significance/threshold 对、post_release 错。pressure_risk 是软闸（gates.rs:160-173 走 revision，不产 block 终态）→ 其命中率应走 `revision_failed` 口径，与 post_release 已有的 rewrite 两闸一致。

---

### Task 1: 修正 FIVE_GATE_KEYS 映射 + 一致性单测

**Files:**
- Modify: `src/evolution/post_release.rs`（FIVE_GATE_KEYS :54-60 三处：fact_risk 改回 held_by_ai_policy、pressure_risk 改 revision_failed；product_accuracy 不动）
- Test: `src/evolution/post_release.rs`（`#[cfg(test)]` 内联一致性单测，钉住与 significance 权威源对齐、防再漂）

**Interfaces:**
- Consumes: `crate::evolution::significance::safety_block_status_for(Option<&str>) -> Option<&'static str>`（significance.rs:59）。
- Produces: 无新公开 API（常量值修正 + 单测）。

- [ ] **Step 1: 写失败测试（一致性钉死）**

在 `src/evolution/post_release.rs` 末尾加（或既有 `#[cfg(test)] mod tests` 内）：

```rust
#[cfg(test)]
mod five_gate_mapping_tests {
    use super::FIVE_GATE_KEYS;
    use crate::evolution::significance::safety_block_status_for;

    /// [2-01]：post_release 的安全闸映射必须与 significance 权威源一致（防三文件再漂）。
    #[test]
    fn safety_gate_mapping_matches_significance_authority() {
        for (gate_key, status) in FIVE_GATE_KEYS {
            if let Some(authoritative) = safety_block_status_for(Some(gate_key)) {
                assert_eq!(
                    *status, authoritative,
                    "post_release gate '{gate_key}' 映射 '{status}' 与 significance 权威 '{authoritative}' 不一致"
                );
            }
        }
    }

    /// fact_risk 硬闸生产落 held_by_ai_policy（非 blocked_by_safety_guard）。
    #[test]
    fn fact_risk_maps_to_held_by_ai_policy() {
        let m: std::collections::HashMap<_, _> = FIVE_GATE_KEYS.iter().cloned().collect();
        assert_eq!(m.get("fact_risk_block"), Some(&"held_by_ai_policy"));
    }

    /// pressure_risk 是软闸(走 revision 不产 block 终态)→命中率走 revision_failed 口径,
    /// 与 human_like/emotional rewrite 两闸一致。
    #[test]
    fn pressure_risk_uses_revision_failed_soft_gate_semantics() {
        let m: std::collections::HashMap<_, _> = FIVE_GATE_KEYS.iter().cloned().collect();
        assert_eq!(m.get("pressure_risk_block"), Some(&"revision_failed"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib five_gate_mapping 2>&1 | tail -20`
Expected: `safety_gate_mapping_matches_significance_authority` FAIL（fact_risk 现值 blocked_by_safety_guard ≠ 权威 held_by_ai_policy）；`fact_risk_maps_to_held_by_ai_policy` FAIL；`pressure_risk_uses_revision_failed` FAIL（现值 held_by_ai_policy）。

- [ ] **Step 3: 修正映射**

`src/evolution/post_release.rs` :54-60 当前：
```rust
const FIVE_GATE_KEYS: &[(&str, &str)] = &[
    ("fact_risk_block", "blocked_by_safety_guard"),
    ("pressure_risk_block", "held_by_ai_policy"),
    ("human_like_score_rewrite", "revision_failed"),
    ("emotional_value_rewrite", "revision_failed"),
    ("product_accuracy_score_block", "blocked_unverified_product_claim"),
];
```
改为（加注释说明对调修正 + pressure 软闸口径）：
```rust
// [2-01] 修正：fact_risk/pressure_risk 此前与 threshold/significance 两权威源对调
// （面板命中率贴错标签）。安全闸映射以 significance::SAFETY_GATE_BLOCK_STATUS 为权威：
// fact_risk 硬闸生产落 held_by_ai_policy；blocked_by_safety_guard 来自产品声明/relay。
// pressure_risk 是软闸（走 revision，生产不产 block 终态），命中率改走 revision_failed
// 口径，与 human_like/emotional 两 rewrite 闸一致。见 five_gate_mapping_tests 钉死一致性。
const FIVE_GATE_KEYS: &[(&str, &str)] = &[
    ("fact_risk_block", "held_by_ai_policy"),
    ("pressure_risk_block", "revision_failed"),
    ("human_like_score_rewrite", "revision_failed"),
    ("emotional_value_rewrite", "revision_failed"),
    ("product_accuracy_score_block", "blocked_unverified_product_claim"),
];
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib five_gate_mapping 2>&1 | tail -15`
Expected: 3 个测试全 PASS。

- [ ] **Step 5: 全量 lib 回归**

Run: `cargo test --lib 2>&1 | tail -6`
Expected: 0 failed（新增 3 测试）。注意：若 post_release 既有测试对 five_gate_hit_rate 具体值有断言，需一并核对——grep `five_gate` / `actual_5gate` 在 post_release 测试里的断言，若断言旧对调值则更新为修正后语义（属修复的一部分，非过拟合）。

- [ ] **Step 6: 提交**

```bash
git add src/evolution/post_release.rs
git commit -m "fix(evolution): 2-01 post_release 5 闸映射对调修正 + pressure revision 口径

fact_risk/pressure_risk 此前与 threshold/significance 两权威源对调,面板命中率
贴错标签(纯观测,不反哺 promote/rollback)。fact_risk 复位 held_by_ai_policy;
pressure_risk 是软闸不产 block 终态,命中率改走 revision_failed(与 rewrite 两闸一致)。
加 five_gate_mapping_tests 钉死与 significance 权威源一致,防三文件再漂。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 本地门 + 推送 + PR + 合并

- [ ] **Step 1: `-D warnings` check --tests**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -15`
Expected: 无 error。超时后台跑等通知。

- [ ] **Step 2: lint 预扫（evolution 三门）**

Run: `git diff origin/main..HEAD -- src/evolution/ | grep '^+' | grep -iE 'human[_ -]?takeover|接管|人工|outbox|mcp::|anthropic|gpt-[0-9]|claude-[0-9]|deepseek-[a-z]'`
Expected: 无输出（无禁词、无隔离禁引用）。

- [ ] **Step 3: 推送 + 亲验 tip**

```bash
git push origin HEAD:refs/heads/<pr-e-branch>
git ls-remote origin refs/heads/<pr-e-branch>  # == 本地 HEAD
```

- [ ] **Step 4: 建 PR + 监控 CI + squash merge（不带 --delete-branch）**

CI 全绿（Baseline+Integration+三 lint）后合，`git fetch && git rev-parse origin/main` 核 mergeCommit 进 main。

## Self-Review

- **Spec 覆盖**：2-01 单条，Task 1 覆盖（映射对调修正 + pressure revision 口径），与设计文档 PR-E 段一致。
- **占位符**：Task 2 分支名标「见落地顺序」；Step 5 的既有测试断言核对是「实现时 grep 确认」——有意的亲验要求，非占位。其余含真实代码。
- **类型一致**：`safety_block_status_for(Option<&str>) -> Option<&'static str>` 签名亲验自 significance.rs:59；`FIVE_GATE_KEYS: &[(&str,&str)]` 亲验自 post_release.rs:54。
- **纯观测不反哺决策**：Global Constraints 钉住——本改动零决策路径影响，仅面板标签正确化。

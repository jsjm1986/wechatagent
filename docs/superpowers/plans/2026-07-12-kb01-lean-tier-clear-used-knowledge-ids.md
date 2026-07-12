# KB-01 修复实施计划：非 Full 档清空 used_knowledge_ids

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。Steps 用 checkbox 跟踪。**红线：改任何代码前必 100% 读懂相关代码，引用必当场 Read/Grep 亲验 file:line，不猜。**

**Goal:** 非 Full 档决策清空 `used_knowledge_ids`，堵住 LLM 自报真实 verified ObjectId 架空 `blocked_unverified_product_claim` 硬闸的缺口（KB-01）。

**Architecture:** `sufficiency.rs` 抽纯函数 `resolve_used_knowledge_ids(forced_full, escalated_to_full, route_ids)`——Full 档返路由 id、非 Full 档返空 Vec；`gateway.rs:1457` 从只有 `if`（Full 档覆盖写、非 Full 档不动透传的自报值）改为**无条件赋值**调该纯函数。抽纯函数使"非 Full 档该不该保留 id"可确定性单测（组合到 `compute_verified_chunks` 验证硬闸仍 block）。

**Tech Stack:** Rust 2021 / Axum。`cargo test --lib` 跑单测（无需 Docker）。

## Global Constraints
- **改前必 100% 读懂 + 引用必亲验 file:line**（CLAUDE.md 最高红线）。行号会漂——每个改码 Task 的 Step 1 必先 Read/Grep 亲验当前真实行号再改。
- **严格限定范围**：只改 `src/agent/sufficiency.rs`（+纯函数+3 测试）、`src/agent/gateway.rs:1457`（改用纯函数）。**不改** `types.rs` carry_through、`gateway.rs:1592`(rewrite)/`:1865`(revision)（均 PromptTier::Full 合法）、`guards.rs` 硬闸、`should_record_used_knowledge_ids`（被复用）。
- **baseline 不回退**：`cargo test --lib` ≥ 350 passed / 0 failed。新增测试只增不减。
- **no-human-takeover lint**：src/ 新增行不得含 `人工接管/takeover/hand-off/人工介入/人工托管/接管/人工`。本修复用「档/切片/硬闸/背书/自报」措辞。
- **设计文档**：`docs/superpowers/specs/2026-07-12-kb01-lean-tier-clear-used-knowledge-ids-design.md`。
- **台账**：`docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KB-01。

## 亲验的现有代码事实（实现者仍须自己 Read 确认当前行号）
- `src/agent/sufficiency.rs`：`should_record_used_knowledge_ids(forced_full: bool, escalated_to_full: bool) -> bool = forced_full || escalated_to_full`（约 :92，`pub(crate)`）；已有 `#[cfg(test)] mod tests`（约 :244 有 `used_knowledge_ids_recorded_only_for_full_tier`）。
- `src/agent/knowledge_router.rs:750`：`pub(crate) fn route_used_knowledge_ids(route: &KnowledgeRouteResult) -> Vec<String>`。
- `src/agent/gateway.rs:1457-1459`：`if crate::agent::sufficiency::should_record_used_knowledge_ids(forced_full, escalated_to_full) { decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route); }`——**无 else**。`forced_full`/`escalated_to_full` 均为 `bool`（:1200 `let escalated_to_full = matches!(...)`，:1222 `let mut forced_full = false`）。
- `src/agent/guards.rs:333`：`pub(crate) fn compute_verified_chunks(used_knowledge_ids: &[String], chunks: &[OperationKnowledgeChunk], now: mongodb::bson::DateTime) -> Vec<&OperationKnowledgeChunk>`——`used ∩ (verified chunks)`；空 used → 返空（:343-345）。用 `chunk.id.map(|id| id.to_hex())` 匹配（:352）。
- `src/agent/guards.rs:309`：`is_verified` 要求 `integrity_status=="verified"`（忽略大小写）且 `valid_to` 未过期。
- 测试构造：`OperationKnowledgeChunk::default()`（guards.rs 测试 `make_test_chunk` 就是它）；`compute_verified_chunks` 匹配需 `chunk.id = Some(ObjectId)`。`use mongodb::bson::{oid::ObjectId, DateTime}`。

---

## Task 1: sufficiency.rs 抽纯函数 resolve_used_knowledge_ids + 3 确定性单测（TDD）

**Files:**
- Modify: `src/agent/sufficiency.rs`（新增 pub(crate) fn，在 `should_record_used_knowledge_ids` 之后；测试加入现有 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `should_record_used_knowledge_ids(bool, bool) -> bool`（已存在，同文件）；测试用 `crate::agent::guards::compute_verified_chunks`、`crate::models::OperationKnowledgeChunk`。
- Produces: `pub(crate) fn resolve_used_knowledge_ids(forced_full: bool, escalated_to_full: bool, route_ids: Vec<String>) -> Vec<String>`（Task 2 的 gateway.rs:1457 调用）。

- [ ] **Step 1: 先读懂（红线）**

Read `src/agent/sufficiency.rs` 全文（`should_record_used_knowledge_ids` 定义 + 注释不变量 + `#[cfg(test)] mod tests` 的 `use` 语句与现有测试范式）。Grep 确认 `compute_verified_chunks` 签名（`grep -n "fn compute_verified_chunks" src/agent/guards.rs`）与 `pub(crate)` 可见性、`OperationKnowledgeChunk` 有 `id: Option<ObjectId>` / `integrity_status: Option<String>` / `valid_to` 字段。确认测试 mod 里能否 `use crate::...`（是否已有 `use super::*`）。**说不清就继续读，不动手。**

- [ ] **Step 2: 写 3 个失败测试**

在 sufficiency.rs 的 `#[cfg(test)] mod tests` 内追加（若已有 `use super::*` 则 resolve_used_knowledge_ids 可见；测试 1 额外 use guards/models/bson）：

```rust
    // ── KB-01：非 Full 档清空 used_knowledge_ids，堵 LLM 自报架空硬闸 ──
    #[test]
    fn kb01_lean_tier_clears_self_reported_ids() {
        // 非 Full 档(false,false)：即便传入(经 carry_through 透传的)自报 id，也一律清空。
        let ids = resolve_used_knowledge_ids(false, false, vec!["a".into(), "b".into()]);
        assert!(ids.is_empty(), "非 Full 档必须清空 used_knowledge_ids(含 LLM 自报)");
    }

    #[test]
    fn kb01_full_tier_keeps_route_ids() {
        // Full 档(forced 或 escalated)：读了切片，保留路由命中 id，不误伤合法背书。
        assert_eq!(
            resolve_used_knowledge_ids(true, false, vec!["id1".into()]),
            vec!["id1".to_string()]
        );
        assert_eq!(
            resolve_used_knowledge_ids(false, true, vec!["id1".into()]),
            vec!["id1".to_string()]
        );
    }

    #[test]
    fn kb01_lean_self_reported_verified_id_cannot_forge_grounding_gate() {
        // 端到端不变量：非 Full 档 + 自报一个真实 verified 语料 id →
        // resolve 清空 → compute_verified_chunks 取 used∩verified 为空 → 硬闸不被架空。
        use crate::models::OperationKnowledgeChunk;
        use mongodb::bson::{oid::ObjectId, DateTime};
        let oid = ObjectId::new();
        let mut chunk = OperationKnowledgeChunk::default();
        chunk.id = Some(oid);
        chunk.integrity_status = Some("verified".into());
        chunk.valid_to = None;
        let self_reported = vec![oid.to_hex()];
        let resolved = resolve_used_knowledge_ids(false, false, self_reported);
        assert!(resolved.is_empty(), "非 Full 档清空自报 id");
        let verified =
            crate::agent::guards::compute_verified_chunks(&resolved, &[chunk], DateTime::now());
        assert!(
            verified.is_empty(),
            "非 Full 档不得有 verified 背书——否则架空 blocked_unverified_product_claim"
        );
    }
```

- [ ] **Step 3: 跑测试确认失败（编译错误）**

Run: `cargo test --lib kb01 2>&1 | tail -20`
Expected: 编译错误 `cannot find function resolve_used_knowledge_ids`（TDD red）。

- [ ] **Step 4: 实现纯函数**

在 `should_record_used_knowledge_ids` 之后插入：

```rust
/// KB-01：本决策最终应记录的 used_knowledge_ids。
/// Full 档(读了切片)记路由命中 id;非 Full 档(没读切片)一律清空——含 LLM 经
/// carry_through 透传的自报值,不给 grounding 硬闸 `compute_verified_chunks` 留架空口。
pub(crate) fn resolve_used_knowledge_ids(
    forced_full: bool,
    escalated_to_full: bool,
    route_ids: Vec<String>,
) -> Vec<String> {
    if should_record_used_knowledge_ids(forced_full, escalated_to_full) {
        route_ids
    } else {
        Vec::new()
    }
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test --lib kb01 2>&1 | tail -20`
Expected: 3 个 kb01_* 测试全 PASS。若测试 1 编译报 use 冲突（如 `super::*` 已带入 DateTime），删冗余 use。

- [ ] **Step 6: Commit**

```bash
git add src/agent/sufficiency.rs
git commit -m "fix(sufficiency): 抽 resolve_used_knowledge_ids 纯函数,非Full档清空(KB-01)

复用 should_record_used_knowledge_ids;非Full档返空Vec,清掉LLM经carry_through透传的自报id。
3确定性lib单测:非Full清空/Full保留/端到端(清空后compute_verified_chunks仍空,硬闸不被架空)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: gateway.rs:1457 改用纯函数（无条件赋值）+ baseline + PR

**Files:**
- Modify: `src/agent/gateway.rs:1457-1459`（`if{...}` → 无条件赋值）

**Interfaces:**
- Consumes: `crate::agent::sufficiency::resolve_used_knowledge_ids(bool, bool, Vec<String>) -> Vec<String>`（Task 1 产出）；`route_used_knowledge_ids(&knowledge_route)`（已存在）。
- Produces: 无（行为层：非 Full 档 decision.used_knowledge_ids 恒清空）。

- [ ] **Step 1: 先读懂（红线）**

Read `src/agent/gateway.rs:1445-1460`（当前 `if should_record_used_knowledge_ids(...) { decision.used_knowledge_ids = route_used_knowledge_ids(...); }` 及上文 forced_full/escalated_to_full 定义）。Grep 亲验当前真实行号（`grep -n "should_record_used_knowledge_ids" src/agent/gateway.rs`）。确认 :1592/:1865 是 rewrite/revision 路径（`grep -n "PromptTier::Full" src/agent/gateway.rs` 核对它们走 Full），**本 Task 不动那两处**。**说不清就继续读。**

- [ ] **Step 2: 改为无条件赋值**

把 gateway.rs:1457-1459 的

```rust
    if crate::agent::sufficiency::should_record_used_knowledge_ids(forced_full, escalated_to_full) {
        decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);
    }
```

替换为（保留上方 ⑤口径修正注释，可更新为"非 Full 档清空"表述）：

```rust
    // ⑤口径修正(KB-01):Full 档记路由命中 id;非 Full 档(Lean/Clarify/Relational 不读切片)
    // 一律清空 used_knowledge_ids——含 LLM 经 carry_through 透传的自报 id,否则自报一个真实
    // verified ObjectId 即可令 grounding 硬闸 used∩verified 非空、架空 blocked_unverified_product_claim。
    decision.used_knowledge_ids = crate::agent::sufficiency::resolve_used_knowledge_ids(
        forced_full,
        escalated_to_full,
        route_used_knowledge_ids(&knowledge_route),
    );
```

- [ ] **Step 3: cargo check 确认编译**

Run: `cargo check --lib 2>&1 | tail -20`
Expected: 0 error。

- [ ] **Step 4: baseline 不回退**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350（比修复前 +3）。

- [ ] **Step 5: no-human-takeover lint 自检**

Run: `git diff origin/main -- src/ | grep -nE "人工接管|takeover|hand.?off|人工介入|人工托管|接管|人工" || echo "lint clean"`
Expected: `lint clean`。

- [ ] **Step 6: Commit**

```bash
git add src/agent/gateway.rs
git commit -m "fix(gateway): used_knowledge_ids 口径点改用 resolve_used_knowledge_ids 无条件赋值(KB-01)

从 if(Full档覆盖写、非Full档不动透传值) 改为无条件赋值:非Full档恒清空。
根治 LLM 自报真实 verified ObjectId 架空 grounding 硬闸。rewrite/revision(:1592/:1865,PromptTier::Full)不动。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

- [ ] **Step 7: push + 开修复 PR**

```bash
git push -u origin fix/kb01-lean-tier-clear-used-knowledge-ids
gh pr create --title "fix: 非 Full 档清空 used_knowledge_ids,堵 LLM 自报架空 grounding 硬闸 (KB-01)" --body "$(cat <<'EOF'
## Summary
修复深度审查批B [KB-01]：非 Full 档决策(Lean/Clarify/Relational,不读切片)若 LLM 自报一个真实存在于 verified 语料的 ObjectId,经 carry_through 透传进 decision.used_knowledge_ids 且不被清空 → grounding 硬闸 used∩verified 非空 → 架空 blocked_unverified_product_claim。

- sufficiency.rs 抽纯函数 resolve_used_knowledge_ids(复用 should_record_used_knowledge_ids,非 Full 档返空 Vec)
- gateway.rs:1457 从 `if{...}`(无 else) 改为无条件赋值:非 Full 档恒清空
- 写端 types.rs carry_through / rewrite(:1592) / revision(:1865,均 PromptTier::Full) 不动

## Test plan
- [x] cargo test --lib(+3 确定性单测:非Full清空/Full保留/端到端 compute_verified_chunks 清空后仍空硬闸不被架空,baseline≥350 不回退)
- [x] no-human-takeover lint clean

设计:docs/superpowers/specs/2026-07-12-kb01-lean-tier-clear-used-knowledge-ids-design.md
台账:docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md KB-01

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review 结论
- **Spec coverage**：设计组件1（纯函数）↔ Task1；组件2（gateway 改用）↔ Task2；测试策略 3 测试 ↔ Task1 Step2；不改清单在 Global Constraints。全覆盖。
- **Placeholder scan**：无 TBD/TODO；每 Step 给完整可编译代码 + 确切命令 + 预期。测试用亲验的 OperationKnowledgeChunk::default()/compute_verified_chunks 构造。
- **Type consistency**：`resolve_used_knowledge_ids(bool, bool, Vec<String>) -> Vec<String>` Task1 定义、Task2 调用一致；`route_used_knowledge_ids(&knowledge_route) -> Vec<String>` 作第三参；`compute_verified_chunks(&[String], &[chunk], DateTime)` 与 guards.rs:333 亲验一致。
- **TDD**：Task1 先写失败测试→确认失败→实现→通过；Task2 check+baseline 不回退。
- **红线**：每个改码 Task Step1 先读懂+亲验行号；明确圈出 :1592/:1865 不动、types.rs carry_through 不动。

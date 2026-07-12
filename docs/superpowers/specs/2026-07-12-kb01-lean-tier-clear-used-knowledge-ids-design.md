# KB-01 修复设计：非 Full 档清空 used_knowledge_ids，堵 LLM 自报架空 grounding 硬闸

- 日期：2026-07-12
- 分支：`fix/kb01-lean-tier-clear-used-knowledge-ids`（基于最新 origin/main b0849d2）
- 来源：深度审查批B [KB-01]（台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`）
- 优先级：P1（红线硬闸可被架空，防御不完整）
- 方案：抽纯函数 `resolve_used_knowledge_ids` + 确定性 lib 单测（用户裁定）

## 问题（KB-01 根因，已主控逐条亲验最新 main 成立）

非 Full 档决策（Lean-Enough / Clarify(Lean) / Escalate(Relational)，三者 `include_business=false`，不注入任何切片正文）若 LLM 自己在输出里吐出一个真实存在于 verified 语料的 24 位 ObjectId hex，该 id 会被记进 `decision.used_knowledge_ids` 并令 grounding 硬闸 `used ∩ verified` 非空 → 放行本应 `blocked_unverified_product_claim` 的产品声明。

**已亲验的事实链**：
- `types.rs:972-974` `carry_through_fields`：`if let Some(v) = raw.used_knowledge_ids { decision.used_knowledge_ids = v; }`——**无条件**把 LLM 原始自报值透传进 decision。
- `gateway.rs:1457-1459`：`if should_record_used_knowledge_ids(forced_full, escalated_to_full) { decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route); }`——只在 Full 档为真时**覆盖写**路由命中 id；**无 else 分支** → 非 Full 档走隐式 else、**不清空**上一步透传的自报值。
- `guards.rs:333-363` `compute_verified_chunks(used_knowledge_ids, chunks, now)`：取 `used ∩ (chunks 中 integrity_status==verified)`，非空即返非空。
- `gates.rs:660` grounding 硬闸调 `compute_verified_chunks`——非空即视为"有 verified 背书"、放行产品声明。
- `sufficiency.rs:85-93` `should_record_used_knowledge_ids(forced_full, escalated_to_full) = forced_full || escalated_to_full`，注释明写不变量"没读切片的决策不得记 id，否则架空硬闸"——该不变量对**路由通道**守住了（非 Full 不覆盖写路由 id），对**LLM 自报通道**没守（透传值不被清空）。
- `selected_chunks`（`gateway.rs:1155` `select_operation_knowledge_chunks`）是 tier-independent 的 verified 全量语料，故非 Full 档的 `used ∩ selected` 完全可能非空。

**验证状态：PLAUSIBLE**（代码缺口 CONFIRMED：自报值确实不被清空；实际可利用性依赖 LLM 恰好吐出真实 verified ObjectId，而 Lean 档 prompt 不注入切片 ID、id 不可猜，真实命中率低——故 Medium 非 High）。红线字面：切片没被自动 verified、精确 `=="verified"` 已核，缺口纯在"自报 id 架空硬闸"这一防御。

## 修复范围（已亲验，确认唯一缺口在 gateway.rs:1457）

- `gateway.rs:1592`(rewrite)、`gateway.rs:1865`(revision)：两处也赋 `used_knowledge_ids = route_used_knowledge_ids(...)`，但其决策生成走 `PromptTier::Full`（:1579 / :1858 亲验），**读了切片**、覆盖写路由 id 合法，**非缺陷、不动**。
- `gateway.rs:250`：`send_contact_message_gateway`（管理端主动发送，reply_text 来自管理请求非 LLM 自报），**非本 finding 向量、不动**。
- **唯一缺口在 `gateway.rs:1457`**：非 Full 档不清空透传的自报值。

## 设计

### 组件 1：纯函数 `resolve_used_knowledge_ids`（新增，无 IO，可单测）

```rust
/// KB-01：本决策最终应记录的 used_knowledge_ids。
/// Full 档(读了切片)记路由命中 id;非 Full 档(没读切片)一律清空——
/// 含 LLM 经 carry_through 透传的自报值,不给 grounding 硬闸留架空口。
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
- 放置：`src/agent/sufficiency.rs`（与 `should_record_used_knowledge_ids` 同文件、同 `pub(crate)`）。
- 复用现有 `should_record_used_knowledge_ids`（不重复判定逻辑）。

### 组件 2：gateway.rs:1457 改为无条件赋值

```rust
decision.used_knowledge_ids = crate::agent::sufficiency::resolve_used_knowledge_ids(
    forced_full,
    escalated_to_full,
    route_used_knowledge_ids(&knowledge_route),
);
```
- 从 `if {...}`（无 else，非 Full 档不动透传值）改为**无条件赋值**：Full 档得路由 id、非 Full 档得空 Vec（清掉透传的自报值）。
- 与 `sufficiency.rs:88-91` 注释意图对齐。

## 不改动的（严格限定范围）

- `types.rs` `carry_through_fields`（无条件透传是给多字段用的通用逻辑，清空责任归 gateway 口径点 :1457；改 carry_through 会牵动其它字段语义，超范围）。
- `gateway.rs:1592`(rewrite) / `:1865`(revision)（PromptTier::Full，合法）。
- `guards.rs` `compute_verified_chunks` 硬闸本身（正确，缺口在喂给它的 used_knowledge_ids）。
- `should_record_used_knowledge_ids`（正确，被新纯函数复用）。
- 前端 / API 契约 / 配置 / 迁移。

## 测试策略（确定性 lib 单测，进 baseline，无需 Docker）

在 `sufficiency.rs` 的 `#[cfg(test)] mod tests` 加：

1. **KB-01 复现+修复（组合到硬闸，锚死不变量）**：非 Full 档（`resolve_used_knowledge_ids(false, false, vec!["<24位hex>"])`）→ 返空 Vec；再喂 `guards::compute_verified_chunks(空, &[该 verified chunk], now)` → 返空 → 证明硬闸不被架空。（这条把纯函数的空返回接到硬闸，验证真实不变量而非孤立函数。）
2. **Full 档不误伤**：`resolve_used_knowledge_ids(true, false, vec!["id1"])` == `["id1"]`；`resolve_used_knowledge_ids(false, true, vec!["id1"])` == `["id1"]`——合法 Full 档背书不被清掉。
3. **非 Full 档清空自报**：`resolve_used_knowledge_ids(false, false, vec!["a","b"])` == `[]`。

**为何抽纯函数而非集成测试**：本地磁盘紧（已清但仍宝贵）、集成需 Docker；架空的核心逻辑（"非 Full 档该不该保留 id"）可确定性单测，gateway :1457 是薄壳（一次赋值）。测试 1 组合 compute_verified_chunks 覆盖"清空→硬闸仍 block"的端到端语义，比 Docker 复现更严谨可控。

## 验证

- `cargo test --lib`（新增 3 测试全绿，baseline lib ≥ 350 / 0 failed 不回退）。
- no-human-takeover lint：新增行用「档/切片/硬闸/背书/自报」措辞，无禁词。
- 不改前端、不改 API 契约、不改配置——无迁移、无 .env 变更。

## 交付

- 单一 src 文件逻辑改动：`sufficiency.rs`（+纯函数+3 测试）、`gateway.rs:1457`（改用纯函数）。
- 独立修复 PR（基于最新 main）。台账 KB-01 标 Closed。

# 升档 run 分档 token 预算 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 progressive-tier 升档（Lean→Full）的 run 获得更高的 token gating 上限，使需知识的首触问题不再被 `blocked_by_budget` 静默拦截、永不回复。

**Architecture:** 给 `RunBudget` 增加一个可授予的 `escalation_bonus`（默认 0，效果 = 抬高 `is_exceeded()` / `record_tool_call()` 判定用的 token 上限，但不改 `tokens_used` 真实累计）。gateway 在两处升档分支、第二程重生成之前调 `grant_escalated_ceiling`，把本 run 的有效上限抬到运营域可配的 `run_token_budget_escalated`（默认 100000）。非升档 run 不授予、行为逐字不变。

**Tech Stack:** Rust 2021 / Axum / MongoDB(BSON serde) / parking_lot Mutex。测试 `cargo test --lib`。

## Global Constraints

- 只放宽**判定用的 gating 上限**，绝不丢弃 / 不篡改 `tokens_used` 真实消费累计（`agent_run_logs.tokens_used` 必须如实反映成本）。
- 未授予时（`escalation_bonus == 0`）`RunBudget` 全部行为与改造前**逐字等价**——既有 `run_budget_*` 单测不得改断言。
- 不改 `max_llm_calls` / `tool_call_budget` 两维（升档健康路径 2 次 reply + review + 1 次 rewrite = 4 ≤ 6，非绑定约束）。
- 不改 R3.7 `blocked_by_budget` 语义本身（预算真超时仍应 fail-closed）。
- 锁顺序纪律：任何同时读 `escalation_bonus` 与 `tokens_used`/`tool_calls_used` 的路径，必须先在独立语句里取 `escalation_bonus` 值并释放，再取其它锁——避免与 `record_tool_call` 的 `tokens→tool_calls` 顺序形成环。
- `cargo test --lib` 基线：≥ 350 passed, 0 failed（现 1814）。

---

### Task 1: RunBudget 增加可授予的升档上限

**Files:**
- Modify: `src/agent/budget.rs`（struct 定义 ~54-68 / `new` ~70-87 / `record_tool_call` ~111-131 / `is_exceeded` ~136-140 / `snapshot` ~146-157 / `RunBudgetSnapshot` ~160-171 / `#[cfg(test)] mod tests` ~185）

**Interfaces:**
- Produces:
  - `RunBudget.escalation_bonus: parking_lot::Mutex<i64>`（字段，init 0）
  - `RunBudget::grant_escalated_ceiling(&self, escalated_total: i64)` — 幂等地把有效 token 上限设为 `max(token_budget, escalated_total)`（实现为 `escalation_bonus = (escalated_total - token_budget).max(0)`）
  - `is_exceeded()` / `record_tool_call()` 的 token 维改为对 `token_budget + escalation_bonus` 比较

- [ ] **Step 1: 写失败测试**（追加到 `src/agent/budget.rs` 的 `#[cfg(test)] mod tests` 末尾，`}` 之前）

```rust
    #[test]
    fn grant_escalated_ceiling_raises_effective_token_budget() {
        let budget = RunBudget::new("run_e", 30_000, 6, 6);
        budget.record_call(40_000);
        assert!(budget.is_exceeded(), "40000 >= 30000 base budget → 超额");
        budget.grant_escalated_ceiling(100_000);
        assert!(
            !budget.is_exceeded(),
            "授予后 40000 < 100000 有效上限 → 不再超额"
        );
    }

    #[test]
    fn grant_escalated_ceiling_is_idempotent() {
        let budget = RunBudget::new("run_e", 30_000, 6, 6);
        budget.grant_escalated_ceiling(100_000);
        budget.grant_escalated_ceiling(100_000);
        budget.record_call(90_000);
        assert!(!budget.is_exceeded(), "90000 < 100000，重复授予同值无副作用");
        budget.record_call(20_000);
        assert!(budget.is_exceeded(), "110000 >= 100000");
    }

    #[test]
    fn grant_escalated_ceiling_below_base_does_not_shrink() {
        let budget = RunBudget::new("run_e", 30_000, 6, 6);
        budget.grant_escalated_ceiling(10_000);
        budget.record_call(20_000);
        assert!(
            !budget.is_exceeded(),
            "escalated_total < token_budget 时 bonus=0，绝不缩小上限"
        );
    }

    #[test]
    fn is_exceeded_without_grant_uses_base_budget() {
        let budget = RunBudget::new("run_e", 100, 6, 6);
        budget.record_call(60);
        assert!(!budget.is_exceeded());
        budget.record_call(50);
        assert!(budget.is_exceeded(), "未授予时 110 >= 100 base，行为逐字不变");
    }

    #[test]
    fn record_tool_call_uses_escalated_ceiling() {
        let budget = RunBudget::new("run_e", 100, 6, 16);
        budget.grant_escalated_ceiling(300);
        budget.record_tool_call(250).expect("250 <= 300 有效上限");
        let err = budget
            .record_tool_call(60)
            .expect_err("250+60 > 300 → TokensExceeded");
        match err {
            BudgetError::TokensExceeded { budget: cap, .. } => {
                assert_eq!(cap, 300, "错误须报有效上限(300)而非 base(100)");
            }
            other => panic!("expected TokensExceeded, got {other:?}"),
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib agent::budget:: 2>&1 | tail -20`
Expected: FAIL —— `no method named grant_escalated_ceiling` / 编译错误（方法未定义）。

- [ ] **Step 3: 加字段 `escalation_bonus`**

在 `pub struct RunBudget { ... }` 里，`pub degraded_reasons: PlMutex<Vec<String>>,` 之前插入：

```rust
    /// B-1 修复：progressive-tier 升档 run 的额外 token gating 余量（默认 0）。
    /// 效果 = 抬高 is_exceeded/record_tool_call 判定用的 token 上限，**不**改
    /// tokens_used 真实累计。只在 gateway 升档分支经 grant_escalated_ceiling 授予。
    pub escalation_bonus: PlMutex<i64>,
```

- [ ] **Step 4: `new()` 初始化 `escalation_bonus: PlMutex::new(0)`**

在 `RunBudget::new` 的 `Self { ... }` 里，`degraded_reasons: PlMutex::new(Vec::new()),` 之前插入：

```rust
            escalation_bonus: PlMutex::new(0),
```

- [ ] **Step 5: 加 `grant_escalated_ceiling` 方法**

在 `impl RunBudget { ... }` 内、`record_call` 之后插入：

```rust
    /// B-1 修复：把本 run 的有效 token gating 上限抬到 `max(token_budget, escalated_total)`。
    /// 幂等——重复以同值调用无副作用；`escalated_total <= token_budget` 时 bonus=0（绝不缩小上限）。
    /// 仅放宽判定上限，不改 tokens_used 真实累计。
    pub fn grant_escalated_ceiling(&self, escalated_total: i64) {
        let bonus = escalated_total.saturating_sub(self.token_budget).max(0);
        *self.escalation_bonus.lock() = bonus;
    }
```

- [ ] **Step 6: `is_exceeded()` 用有效上限**

把现有 `is_exceeded`（约 136-140）整体替换为：

```rust
    pub fn is_exceeded(&self) -> bool {
        // escalation_bonus 在独立语句里取值并立即释放，再取 tokens_used 锁——
        // 避免与 record_tool_call 的 tokens→tool_calls 锁顺序形成环。
        let effective_token_budget = self.token_budget + *self.escalation_bonus.lock();
        *self.tokens_used.lock() >= effective_token_budget
            || *self.llm_calls_used.lock() >= self.max_llm_calls
            || *self.tool_calls_used.lock() >= self.tool_call_budget
    }
```

- [ ] **Step 7: `record_tool_call()` 用有效上限**

把现有 `record_tool_call` 里的 token 检查改为有效上限。将方法体开头的

```rust
        let consumed = tokens_consumed.max(0);
        let mut tokens = self.tokens_used.lock();
        let mut tool_calls = self.tool_calls_used.lock();
```

替换为（先取 escalation_bonus 值并释放，再取其它两锁，保持锁顺序一致）：

```rust
        let consumed = tokens_consumed.max(0);
        let effective_token_budget = self.token_budget + *self.escalation_bonus.lock();
        let mut tokens = self.tokens_used.lock();
        let mut tool_calls = self.tool_calls_used.lock();
```

并把随后的 token 超额判定

```rust
        if (*tokens).saturating_add(consumed) > self.token_budget {
            return Err(BudgetError::TokensExceeded {
                used: *tokens,
                consumed,
                budget: self.token_budget,
            });
        }
```

替换为：

```rust
        if (*tokens).saturating_add(consumed) > effective_token_budget {
            return Err(BudgetError::TokensExceeded {
                used: *tokens,
                consumed,
                budget: effective_token_budget,
            });
        }
```

- [ ] **Step 8: `snapshot()` + `RunBudgetSnapshot` 带上 `escalation_bonus`（可观测）**

在 `RunBudgetSnapshot` 结构体里 `pub tool_calls_used: i32,` 之后加：

```rust
    pub escalation_bonus: i64,
```

在 `snapshot()` 的 `RunBudgetSnapshot { ... }` 里 `tool_calls_used: *self.tool_calls_used.lock(),` 之后加：

```rust
            escalation_bonus: *self.escalation_bonus.lock(),
```

- [ ] **Step 9: 运行测试确认全绿**

Run: `cargo test --lib agent::budget:: 2>&1 | tail -20`
Expected: PASS —— 新 5 个测试 + 既有 `run_budget_*` / `record_tool_call_*` 全绿（既有断言未改）。

- [ ] **Step 10: 提交**

```bash
git add src/agent/budget.rs
git commit -m "feat(budget): RunBudget 增加可授予的升档 token 上限(grant_escalated_ceiling)

B-1 修复地基:escalation_bonus(默认0)抬高 is_exceeded/record_tool_call 判定用的
token 上限,不改 tokens_used 真实累计。未授予时行为逐字等价。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 配置字段 `run_token_budget_escalated` 打通（typed → runtime）

**Files:**
- Modify: `src/models.rs`（`RuntimeParametersTyped` 字段 ~3558 / `defaults` 子模块 ~3711 / `Default` 构造 ~3645 / `#[cfg(test)]` 测试 ~4808）
- Modify: `src/agent/runtime.rs`（`UserRuntimeParameters` 字段 ~35 / `from_config` ~162 / `as_document` ~213 / 硬编码默认构造 ~596）
- Modify: 编译器（E0063）指出的其余 `UserRuntimeParameters` 结构体字面量构造点（已知：`src/agent/mod.rs:~535`、`src/agent/run_envelope.rs:~1553`、`src/agent/types.rs:~1716`）

**Interfaces:**
- Consumes: 无（纯字段新增）
- Produces:
  - `RuntimeParametersTyped.run_token_budget_escalated: i64`（serde 默认 100000，BSON key `runTokenBudgetEscalated`）
  - `UserRuntimeParameters.run_token_budget_escalated: i64`（Task 3 gateway 消费）
  - `defaults::run_token_budget_escalated() -> i64`（= 100000）

- [ ] **Step 1: 写失败测试**（追加到 `src/models.rs` 放 `runtime_parameters_typed_*` 测试的 `#[cfg(test)] mod` 内，与既有 `runtime_parameters_typed_reads_existing_values` 同级）

```rust
    #[test]
    fn runtime_parameters_typed_escalated_budget_default() {
        let p: RuntimeParametersTyped =
            mongodb::bson::from_document(doc! {}).expect("default deserialize");
        assert_eq!(p.run_token_budget_escalated, 100000);
        assert_eq!(typed::defaults::run_token_budget_escalated(), 100000);
    }

    #[test]
    fn runtime_parameters_typed_reads_escalated_budget() {
        let doc = doc! { "runTokenBudgetEscalated": 120000_i64 };
        let p: RuntimeParametersTyped =
            mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(p.run_token_budget_escalated, 120000);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib models:: 2>&1 | tail -20`
Expected: FAIL —— `no field run_token_budget_escalated` / `no function run_token_budget_escalated`（编译错误）。

- [ ] **Step 3: `RuntimeParametersTyped` 加字段**

在 `src/models.rs` `RuntimeParametersTyped` 里 `#[serde(default = "defaults::run_token_budget")] pub run_token_budget: i64,` 之后插入：

```rust
        /// B-1 修复：progressive-tier 升档(Lean→Full)的 run 的 token gating 上限。
        /// 升档触发两程 reply.task,base run_token_budget(30000)容不下,此值放宽升档路径。
        #[serde(default = "defaults::run_token_budget_escalated")]
        pub run_token_budget_escalated: i64,
```

- [ ] **Step 4: `defaults` 子模块加默认函数**

在 `src/models.rs` `defaults` 子模块里 `pub fn run_token_budget() -> i64 { 30000 }` 之后插入：

```rust
        pub fn run_token_budget_escalated() -> i64 {
            100000
        }
```

- [ ] **Step 5: `RuntimeParametersTyped` 的 `Default`/构造补字段**

在 `src/models.rs` 该结构体的默认构造块里 `run_token_budget: defaults::run_token_budget(),` 之后插入：

```rust
                run_token_budget_escalated: defaults::run_token_budget_escalated(),
```

- [ ] **Step 6: `UserRuntimeParameters` 加字段**

在 `src/agent/runtime.rs` 结构体里 `pub run_token_budget: i64,`（~35）之后插入：

```rust
    /// B-1 修复:progressive-tier 升档 run 的 token gating 上限(默认 100000)。
    /// gateway 升档分支经 RunBudget::grant_escalated_ceiling 授予本 run。
    pub run_token_budget_escalated: i64,
```

- [ ] **Step 7: `from_config` 映射**

在 `src/agent/runtime.rs` `from_config` 里 `run_token_budget: typed.run_token_budget,`（~162）之后插入：

```rust
            run_token_budget_escalated: typed.run_token_budget_escalated,
```

- [ ] **Step 8: `as_document` 序列化**

在 `src/agent/runtime.rs` `as_document` 里 `"runTokenBudget": self.run_token_budget,`（~213）之后插入：

```rust
            "runTokenBudgetEscalated": self.run_token_budget_escalated,
```

- [ ] **Step 9: 硬编码默认构造补字段（runtime.rs:~596）**

在 `src/agent/runtime.rs` 该 `UserRuntimeParameters { ... }` 字面量里 `run_token_budget: 30000,`（~596）之后插入：

```rust
            run_token_budget_escalated: 100000,
```

- [ ] **Step 10: 用编译器定位其余字面量构造点并补齐**

Run: `cargo check --tests 2>&1 | grep -A3 "missing.*run_token_budget_escalated" | head -40`
Expected: 列出所有缺该字段的 `UserRuntimeParameters` 结构体字面量（预期 `src/agent/mod.rs:~535`、`src/agent/run_envelope.rs:~1553`、`src/agent/types.rs:~1716`）。
逐个在该文件对应字面量的 `run_token_budget: 30000,` 行之后插入 `run_token_budget_escalated: 100000,`。
（注：`src/agent/memory.rs:~1187` 是 `RunBudget::new(run_id, runtime.run_token_budget, ...)` **函数调用**，不是结构体字面量，不受影响、保持原样。）

- [ ] **Step 11: 运行 check + 测试确认全绿**

Run: `cargo check --tests 2>&1 | tail -5 && cargo test --lib models:: 2>&1 | tail -15`
Expected: check 无 E0063；models 测试全绿（新 2 个 + 既有 `runtime_parameters_typed_*`）。

- [ ] **Step 12: 提交**

```bash
git add src/models.rs src/agent/runtime.rs src/agent/mod.rs src/agent/run_envelope.rs src/agent/types.rs
git commit -m "feat(config): 加 run_token_budget_escalated 配置字段(默认100000)

RuntimeParametersTyped(BSON runTokenBudgetEscalated)→UserRuntimeParameters 打通,
供 gateway 升档分支授予升档 run 更高 token 上限。运营域可配。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: gateway 升档分支授予升档上限

**Files:**
- Modify: `src/agent/gateway.rs`（`ptier_forced_full` 分支 ~1225-1259 / `ptier_escalated` 分支 ~1303-1335，均在 `run_user_operation_gateway_inner` 内）

**Interfaces:**
- Consumes:
  - `RunBudget::grant_escalated_ceiling`（Task 1）
  - `UserRuntimeParameters.run_token_budget_escalated`（Task 2）
  - `current_run_budget()`（已在 gateway.rs 使用，见 ~1445）；`runtime`（`run_user_operation_gateway_inner` 参数，作用域内可见）
- Produces: 无（终端接线）

- [ ] **Step 1: forced_full 分支加授予**

在 `src/agent/gateway.rs` `ptier_forced_full` 分支，写完 `write_event_for_account(... "ptier_forced_full" ...)` 的 `.await.ok();` 之后、`decide_reply_with_promote(` 调用之前，插入：

```rust
                // B-1:升 Full 前放宽本 run 的 token gating 上限,让「Lean 探测 + Full 程
                // + review + 一次 rewrite」不撑爆 base run_token_budget(30000)而被
                // blocked_by_budget 拦回复。tokens_used 仍如实累计,只放宽判定上限。
                if let Some(b) = current_run_budget() {
                    b.grant_escalated_ceiling(runtime.run_token_budget_escalated);
                }
```

- [ ] **Step 2: escalated 分支加授予**

在 `src/agent/gateway.rs` `ptier_escalated` 分支，写完 `write_event_for_account(... "ptier_escalated" ...)` 的 `.await.ok();` 之后、`decide_reply_with_promote(` 调用之前，插入：

```rust
            // B-1:升档(Relational/Full)前放宽本 run 的 token gating 上限——升档触发第二程
            // reply.task,两程叠加超 base run_token_budget(30000)会被 blocked_by_budget 拦
            // 回复。tokens_used 仍如实累计,只放宽判定上限。
            if let Some(b) = current_run_budget() {
                b.grant_escalated_ceiling(runtime.run_token_budget_escalated);
            }
```

- [ ] **Step 3: 编译 + lib 基线**

Run: `cargo test --lib 2>&1 | tail -6`
Expected: `test result: ok.` ≥ 350 passed, 0 failed（现 1814）。

- [ ] **Step 4: 提交**

```bash
git add src/agent/gateway.rs
git commit -m "fix(gateway): 升档 run 授予更高 token 上限,修 B-1 首触被拦(blocked_by_budget)

progressive-tier 升档(Lean→Full)两处分支在第二程重生成前调
grant_escalated_ceiling,把本 run token gating 上限抬到 run_token_budget_escalated
(默认100000)。修:需知识的首触问题不再因两程超30000预算被静默拦回复。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 端到端复现验证（环境就绪时）

**Files:** 无代码改动，仅运行既有复现脚本。

**前置**：本地栈起（后端 `:8080` 用**新编译**二进制 + 本地 Mongo `wechatagent_local_e2e` + LLM 端点健康）。MCP 宕机不影响本验证（decision/review/outbox 判定在发送步之前）。

- [ ] **Step 1: 升档路径修复验证**

Run: `cd scripts/e2e && node fresh_contact_budget.mjs 2>&1 | tail -6`
Expected（修复后）：run `final_review_status` **不再是** `blocked_by_budget`；`tokens_used`（如 ~56770）虽 > 30000 但 < 100000 有效上限 → review 正常跑 → `approved` 或其它正常 finalize 状态，reply 进 outbox（MCP 宕机则 outbox 最终 `failed_terminal`，属 C 类，非本修复回归）。

- [ ] **Step 2: Lean 路径无回归验证**

Run: `cd scripts/e2e && node fresh_greeting.mjs "你好，在吗？" 2>&1 | tail -6`
Expected：仍 `completed` / `approved` / `should_reply=true`，单程 reply.task（~23501 tokens），无 `ptier_escalated`——非升档 run 不授予、行为不变。

- [ ] **Step 3: 记录验证结果**

把两条 run 的 `run_id` / `final_review_status` / `tokens_used` 追加到 `docs/smoke/2026-07-05-newuser-journey-four-way-audit.md` 的 B-1 条目下（标注「修复后复验」），提交。
若 LLM 端点不可用则标 BLOCKED、留待端点恢复复跑，不假绿。

---

## Self-Review

**Spec coverage**：
- spec §4 分档预算机制 → Task 1（budget）+ Task 3（gateway 授予）✓
- spec §5.1 budget.rs 改动 → Task 1 全部步骤 ✓
- spec §5.2 runtime.rs → Task 2 Step 6-9 ✓
- spec §5.3 models.rs → Task 2 Step 3-5 ✓
- spec §5.4 gateway 两处授予 → Task 3 ✓
- spec §5.5 结构体字面量补齐 + memory.rs 不受影响 → Task 2 Step 10 ✓
- spec §6 测试（budget 单测 / 回归 / e2e 复现）→ Task 1 Step 1 / Task 3 Step 3 / Task 4 ✓
- spec §7 非目标（不丢弃 token / 不全局抬 / 不动 llm_calls 维 / 不压 prompt）→ 计划未触碰，符合 ✓

**Placeholder scan**：无 TBD/TODO；所有代码步骤含完整代码块；无"类似上文"引用。✓

**Type consistency**：
- `grant_escalated_ceiling(&self, escalated_total: i64)` 定义（Task 1 Step 5）与调用（Task 3 Step 1/2）签名一致 ✓
- `run_token_budget_escalated: i64` 在 `RuntimeParametersTyped`（Task 2 Step 3）/ `UserRuntimeParameters`（Step 6）/ 默认 100000（Step 4/9）一致 ✓
- BSON key `runTokenBudgetEscalated`（Step 8 序列化 / Step 1 测试读）一致 ✓
- `escalation_bonus: PlMutex<i64>`（字段 Step 3）/ snapshot `escalation_bonus: i64`（Step 8）一致 ✓

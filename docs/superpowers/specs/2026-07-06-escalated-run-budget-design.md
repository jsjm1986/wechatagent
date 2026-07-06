# 升档 run 的分档 token 预算（B-1 修复设计）

日期：2026-07-06
状态：设计待审
关联：`docs/smoke/2026-07-05-newuser-journey-four-way-audit.md` Findings B-1

## 1. 问题

真实新用户首次发**需知识/触发 progressive-tier 升档**的消息（如"你们的课程怎么收费？"）时，
后台流水线跑完后 run `lifecycle=aborted_by_budget` / `final_review_status=blocked_by_budget`
/ `decision.should_reply=false` —— **主回复从不发送**。客户永远收不到答复。

## 2. 根因（实测数据，`run_token_budget`=30000）

progressive-tier 两程叠加撑爆单 run 的 token 预算：

- **第一程 Lean 探测**：prompt 24920 tokens，经 `record_call`（`src/agent/mod.rs:276/340`）记入 `RunBudget.tokens_used`。
- **升档判定**：`decide_tier_escalation`（`src/agent/sufficiency.rs:33`）返回 `Escalate(Full)`（或 `forced_full` 强升）。
- **第二程 Full**：`decide_reply_with_promote` 再跑一次（`src/agent/gateway.rs:1303-1335` / `1215-1259`），prompt 29203 tokens，**同样记入同一 `tokens_used`**。第一程 Lean 决策被**丢弃**，只用 Full 决策。
- 加知识路由 ~2647 → `tokens_used` = **56770 ≥ 30000**。
- 进 review 前的预算检查 `budget_exceeded_for_review`（`src/agent/gateway.rs:1446`）读 `is_exceeded()`（`src/agent/budget.rs:136`，`tokens_used >= token_budget`）为 true → 跳过 LLM review 降级 local → 跳过 rewrite → R3.7（`src/agent/review/gates.rs:620`）`needs_review + 预算超额` → `blocked_by_budget`。

**关键数据（决定修复形态）**：单独一程 Full（29203）+ 路由（~2647）就已 **> 30000**。
即：预算连"单程 Full + review"都容不下，不只是被两程双重计数。对照实验证实——简单问候停 Lean
单程（23501 tokens）能正常过 review 并发送回复；仅升档路径爆预算。

## 3. 为何否决其它选项

- **抬高全局 `run_token_budget`**（用户已否决）：全局拉高每 run 成本上限，且掩盖"单条 prompt 已逼近预算"的事实。
- **"替换而非叠加"（丢弃 Lean 探测的 token 计数）**：实测**不足**。56770 − 24920 = **31850，仍 > 30000**。
  单程 Full 本身就超预算，仅清掉探测程的计数解决不了；且丢弃真实消费的 token 会让 `agent_run_logs.tokens_used` 谎报成本。

## 4. 设计（分档预算：仅对升档 run 放宽 gating 上限）

用户选定方向："改 progressive-tier 升档计费"，具体落到其"升档路径单独放宽预算"子机制。

- **非升档 run（Lean / Relational 停在第一程）**：沿用基础 `run_token_budget`（30000）。紧上限继续守常见路径，防跑飞循环。
- **升档 run**（`forced_full` 强升 Full，或 `Escalate(_)` 升档）：在第二程 `decide_reply_with_promote`
  触发**之前**，把**本 run** 的有效 token gating 上限抬到新配置 `run_token_budget_escalated`
  （默认 100000，按"Lean 探测 + Full 程 + review + 一次 rewrite + 路由 + 余量"估算）。
- **计数保持诚实**：`tokens_used` 仍如实累计每一个真实 token（照常写 `agent_run_logs.tokens_used`）。
  只放宽**判定用的 gating 上限**，不改**消费记录**。明确拒绝丢弃探测程 token（会谎报成本）。

`max_llm_calls` 维度（默认 6）无需改：升档健康路径 = 2 次 reply（Lean+Full）+ review + 一次 rewrite = 4 ≤ 6，
实测 `llm_calls_used=2` 远未触顶，token 才是绑定约束（`is_exceeded()` 是三维 OR，只有 token 维需放宽）。

## 5. 实现面（外科式，最小改动）

1. **`src/agent/budget.rs`**：
   - `RunBudget` 加字段 `escalation_bonus: PlMutex<i64>`（init 0）。
   - 加方法 `grant_escalated_ceiling(&self, escalated_total: i64)`：设 `escalation_bonus = (escalated_total - token_budget).max(0)`（幂等，重复调同值无副作用）。
   - `is_exceeded()` 与 `record_tool_call()` 的 token 比较改为对 `token_budget + *escalation_bonus.lock()`。
   - `token_budget` 字段本身**不动** → churn 最小，既有单测（`run_budget_*`）行为不变（bonus=0 时逐字等价）。
   - `snapshot()` 带上 `escalation_bonus`，供 `agent_run_logs` 可观测（可选，若 snapshot 已落库）。
2. **`src/agent/runtime.rs`**：`UserRuntimeParameters` 加 `run_token_budget_escalated: i64`，`from_config`（:162 邻域）从 `typed.run_token_budget_escalated` 映射，`as_document`（:213 邻域）序列化 `runTokenBudgetEscalated`。默认 100000。
3. **`src/models.rs`**：`RuntimeParametersTyped` 加 `run_token_budget_escalated` 字段（`#[serde(default = "defaults::run_token_budget_escalated")]`，:3558 邻域），`defaults` 加 `run_token_budget_escalated() -> i64 { 100000 }`（:3711 邻域），`Default` 构造补该字段（:3645 邻域）。
4. **`src/agent/gateway.rs`**：在两处升档分支、第二程 `decide_reply_with_promote` 调用**之前**各加一次授权：
   - `ptier_forced_full` 分支（~1225，写完 event 后、`decide_reply_with_promote` 前）。
   - `ptier_escalated` 分支（~1303，写完 event 后、`decide_reply_with_promote` 前）。
   - 形式：`if let Some(b) = current_run_budget() { b.grant_escalated_ceiling(runtime.run_token_budget_escalated); }`（`runtime` 与 `current_run_budget()` 均在 `run_user_operation_gateway_inner` 作用域内）。
5. **其它结构体字面量构造点**：给 `UserRuntimeParameters` / `RuntimeParametersTyped` 加字段后，`cargo check --tests` 会用 E0063 列出每个缺字段的**结构体字面量**构造点（如 `src/agent/mod.rs:535` / `run_envelope.rs:1553` / `types.rs:1716` / `runtime.rs:596` 等默认/测试构造），逐个补 `run_token_budget_escalated: <默认值>` 即可。
   - **注意**：`src/agent/memory.rs:1187` 是 `RunBudget::new(run_id, runtime.run_token_budget, ...)` **函数调用**（位置参数），**不是**结构体字面量，加字段不会波及它；且 memory consolidation 路径不升档，无需授权更高上限，保持原样。

## 6. 测试

- **`budget.rs` 单测**（新增，纯函数）：
  - `grant_escalated_ceiling` 后 `is_exceeded()` 用新上限判定（`token_budget=30000` + grant 100000 → 效上限 100000；`tokens_used=56770` 不再超额）。
  - 未 grant 时行为逐字不变（bonus=0）。
  - `grant` 幂等；`escalated_total < token_budget` 时 bonus=0（不缩小上限）。
- **回归**：`cargo test --lib` 基线 ≥ 350 / 0 failed（现 1814）。
- **端到端复现验证**（本地栈，需 LLM 健康）：`scripts/e2e/fresh_contact_budget.mjs`（升档路径，修后应 `approved` + outbox 入队而非 `blocked_by_budget`）；`scripts/e2e/fresh_greeting.mjs`（Lean 路径，修后仍 `approved`，无回归）。MCP 宕机时发送步仍 C 类 BLOCKED（不影响 decision/review/outbox 判定）。

## 7. 非目标（YAGNI）

- 不做"替换/丢弃探测程 token"组合（上限修对后无必要，且谎报成本）。
- 不全局抬 `run_token_budget`（用户已否决）。
- 不改 `RunBudget` 的 llm_calls / tool_calls 维度（非绑定约束）。
- 不压缩 reply.task 基础 prompt 体积（另一独立方向，用户未选）。
- 不改 R3.7 `blocked_by_budget` 语义本身（预算真超时仍应 fail-closed）。

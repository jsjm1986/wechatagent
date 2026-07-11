# 批A家族① 修复设计：审计/旁路事件 fail-soft 对齐 + reaction 解耦

- 日期：2026-07-12
- 分支：`fix/audit-events-failsoft-alignment`（基于最新 origin/main 054bbc8）
- 来源：深度审查批A 跨环节根因家族①（台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`：B-02 + C-01 + H-01）
- 优先级：P1（改动小、根治"DB 一抖就静默丢客户回复"、价值高）

## 问题（统一根因，已主控亲验最新 main 成立）

**回复入队（`gateway.rs:2586` outbox enqueue）之前的纯审计/旁路写用了 `?`，Mongo 一抖就把本轮客户回复吞掉。**

### C-01 / H-01（`gateway.rs` `apply_agent_updates`）

- 调用点 `gateway.rs:2356`（`.await?` 于 :2365），**早于** 回复 enqueue `:2586`（主控亲验）。
- 函数内 **5 处纯审计 `write_event_for_account` 用 `.await?`**（主控亲验 file:line）：
  - `:4411` `agent.purchase_lifecycle_corrected_by_objective`（g1_correction）
  - `:4480` `agent.profile_churn_observed`
  - `:4503` `agent.operation_state_transition_rejected`
  - `:4525` `agent.operation_state_transitioned`
  - `:4551` `agent.follow_up_run_at_degraded`
- 任一 Mongo insert 失败 → `Err` 沿 `apply_agent_updates`(:2365 `?`) → inner → gateway 冒泡；此时回复**尚未入 outbox** → 本轮回复丢。webhook Inbound Err 只写 agent_error、**不重推** → 该轮回复彻底丢。
- **铁证（同函数孪生用 fail-soft）**：`:3942` `dimension_dropped`、`:3988` `stage_transition_rejected` 都用 `let _ = ...await`，且 :3987/:3966 注释明写"审计写失败不阻断主流程（回复已异步发出），与 dimension_dropped 同风格""与 operation_state_transition_rejected 对称"；`:4260` bayesian 用 `if Err{warn}`。它们声称对称的孪生反而用了 `?`。

### B-02（`webhooks.rs:188-224`）

- 步骤 (e) 网关聚合回复 `handle_managed_message_aggregated` 被嵌在步骤 (d) `record_user_reaction` 的 **`else` 分支**里（主控亲验 `webhooks.rs:199-224`）——reaction 返 `Err` 时只写 `agent_error`、(e) **根本不执行**，本轮回复丢。
- reaction 的 LLM 失败虽被 `unwrap_or_else` 兜底（reaction.rs:162），但其 **DB 脚手架多处 `?` 会真上抛**：`load_domain_config`(reaction.rs:37)、stuck `update_many`(:83)、claim `find_one_and_update`(:110)、写回。任一瞬时 Mongo 错误 → 整轮回复被侧路分析失败连累吞掉。

## 修复设计

### 修复 1：C-01/H-01 —— 5 处纯审计 emit 降级 fail-soft

把 `apply_agent_updates` 内 5 处纯审计事件的 `.await?` 改为 `let _ = ...await;`，与同函数孪生（:3942/:3988 `let _`、:4260 `if Err{warn}`）逐字对齐：`:4411 / :4480 / :4503 / :4525 / :4551`。

**严格限定范围——只改这 5 处纯审计 emit。** 其后 `:4557` follow-up 任务 `insert_one`、contact 画像 `update`、`pending_follow_up_count` 等是**真实业务写**，不在本 finding 范围，**不动**（它们该不该 fail-soft 是另一个产品裁决）。

### 修复 2：B-02 —— (e) 从 (d) 的 else 解耦，无条件执行

把 `webhooks.rs` 步骤 (e) 网关聚合回复从步骤 (d) `record_user_reaction` 的 `else` 分支中移出——(e) **无条件执行**。reaction 只是对*上一轮*结果的旁路分析，与生成*本轮*回复无因果依赖。

- reaction 失败仍只 warn（写 `agent_error`），但**不再阻断** (e)。
- **reaction.rs 内部的 `?` 保持不动**——解耦后它失败不再吞回复，改它属范围外。
- **借用安全性已亲验**：(d) `record_user_reaction(&state, &contact, ...)` 只**借** `&contact`；(e) `handle_managed_message_aggregated(&state, contact, ...)` **移动** `contact`。(e) 移出 else 后借用检查器满足（(d) 借用结束、contact 可移入 (e)，(f)(g) 不再用 contact）。

## 不改动的（严格限定范围）

- 前端 / API 契约 / 配置结构 / .env / 迁移。
- reaction.rs 内部 `?`。
- `gateway.rs` 内真实业务写（follow-up insert / 画像 update / bayesian 写回本身）。
- KD-04 已改的 escalation 路径。

## 测试策略（用户裁定）

### C-01/H-01：真故障注入集成测试（Docker/CI）

以 `tests/c2_operation_state_derivation_e2e.rs` 为模板（mock-LLM + `handle_managed_message` + 非法迁移触发 `operation_state_transition_rejected` 事件路径）：

1. 用 **MongoDB collection validator** 对 `agent_events` 精确拒绝某个 `kind`（如 `agent.operation_state_transition_rejected`）的插入，注入确定性故障。
2. 走非法迁移路径触发该审计写。
3. 断言：即使该审计写失败，`agent_send_outbox` 仍有本轮回复一行（`contact_wxid` 匹配）= **回复未被吞**。

故障注入是 Mongo 标准能力，隔离在 `agent_events`，不碰 `agent_send_outbox`。测试 `#[ignore]` + 需 Docker，本地不跑、CI integration job 覆盖（CLAUDE.md 本地只跑 --lib+PBT）。

**不破坏现有测试已亲验**：`let _` 只丢弃 `Result`、写操作照常执行；`c2_e2e.rs:373`/`real_llm_ops_smoke.rs:2176`/`adversarial.rs:1094` 跑在健康 Mongo（happy path），事件照落、断言照过；无任何现有测试对这 5 处 emit 注入 DB 故障。故 happy path 可观测行为逐字不变。

### B-02：主控亲验，不单写集成测试（用户裁定）

B-02 是 webhooks.rs 三行控制流重构（(e) 移出 else）。要驱动它需去抖 spawn 管道（PENDING map/generation）+ 造 reaction 前置（status=sent 的 decision_review）+ 注入 reaction DB 故障——哈开销大、无现成 harness。正确性靠"控制流显然 + 借用安全性已证"亲验保证。

## 验证

- `cargo check` + `cargo test --lib`（baseline lib ≥ 350 / 0 failed 不回退）。
- `git diff origin/main -- src/agent/ | grep -E "人工接管|takeover|..."` → lint clean（本修复用"审计/旁路/fail-soft/回复已异步发出"既有措辞，无禁词）。
- 集成测试留 CI（本地无 Docker）。

## 交付

- src 改动：`gateway.rs`（5 处 `.await?` → `let _`）、`webhooks.rs`（(e) 解耦）。
- 新增：`tests/audit_event_failsoft_integration.rs`（C-01/H-01 故障注入 e2e，`#[ignore]`）。
- 独立修复 PR（基于最新 main）。台账 findings.md 对应 B-02/C-01/H-01 标记 Closed。

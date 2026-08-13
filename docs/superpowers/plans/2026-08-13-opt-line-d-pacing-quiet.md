# 优化线 D · 发送节奏域 实施计划（S5 批复项 3+4）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 静默时段的显式购买/交易意图消息即时回复（不再最长等 10 小时）；分段发送间隔按文本长度加权（长文不再秒回穿帮）。

**Architecture:** 两个确定性、零 LLM 成本的行为变更，均为用户已拍板的默认行为（不加 feature flag，回滚=git revert）。全部改动限线 D 所有权。

## Global Constraints

- 模型：仅 Fable 5 1M max（不派生 subagent）。
- 红线：动手前全文读懂受影响函数并重验锚点；禁词 lint；磁盘紧张时先清本 worktree `target/debug/incremental`。
- 文件边界：只许修改 `src/webhooks.rs`、`src/agent/reaction.rs`（仅词表函数可见性）、`src/agent/pacing.rs`、`src/agent/outbox_dispatcher.rs`、`src/agent/quiet_hours.rs`（如需）与 `tests/**`。
- 每任务独立 commit；收尾 `cargo test --lib` 全绿（≥2534）+ 四 PBT + `-D warnings` check + 禁词 lint。

---

### Task D1: 静默时段显式交易意图豁免（S5-3，保守收窄版）

**Files:**
- Modify: `src/agent/reaction.rs`（`explicit_buying_intent` 改 `pub(crate)`，函数体一字不动）
- Modify: `src/webhooks.rs`（静默时段 defer 分支）
- Test: `tests/` 新增或扩展 quiet-hours 集成测试（`#[ignore]`）+ webhooks 单测（若判定抽成纯函数）

**行为契约:**
- 现状：`quiet_hours_enabled && is_quiet_now(...)` 时所有 managed 入站一律 defer 到醒来（无例外分支）。
- 目标：defer 前加一个确定性豁免判定——`active_profile.transaction_facts_enabled && reaction::explicit_buying_intent(&content)`（与 reaction 确定性购买下限**同一词表同一语义门**：交易域 profile + 显式购买/付款短语 + 反例 marker 过滤，≤120 字）为真时**不 defer**，走正常 debounce 链路（与非静默时段行为一致）；并写一条 `agent_events`（kind=`quiet_hours_bypassed_buying_intent`，details 含 message_id，best-effort）。
- 设计边界（写进代码注释）：v1 刻意只做交易词表豁免，不做"高意向阶段"判定——阶段集合是行业可配的，硬编码销售阶段违反通用化；后续若需要按阶段豁免，应走 DomainProfile 配置。
- 注意静默分支当前已加载 `active_profile`（resolve_debounce_window 用）——重验后复用，不重复加载。

**Steps:**
- [ ] 读 webhooks 静默分支全文 + `explicit_buying_intent` 全文与其调用方；重验锚点。
- [ ] 失败集成测试：交易 profile + 静默时段 + "我要买现在付款"消息 → 任务 run_at ≈ now（非醒来时刻）+ bypass 事件在场；对照组：寒暄消息仍 defer 到醒来、非交易 profile 的购买短语仍 defer。
- [ ] 实现；测试绿。
- [ ] Commit：`feat(webhooks): bypass quiet-hours deferral for explicit transactional intent on transaction profiles`

### Task D2: 分段发送间隔按长度加权（S5-4）

**Files:**
- Modify: `src/agent/pacing.rs`（函数与单测）
- Modify: `src/agent/outbox_dispatcher.rs`（调用点传入本段字符数——先 grep 确认 `account_send_interval_ms` 全部调用方；若仅 dispatcher 一处则直接改签名不留旧函数）
- Test: pacing 单测扩展

**行为契约:**
- 新签名：`account_send_interval_ms(jitter01: f64, min_ms: i64, max_ms: i64, content_chars: usize) -> i64`。
- 公式：`base = 现线性映射(jitter01, min_ms, max_ms)`；`typing = (content_chars as i64) * PER_CHAR_TYPING_MS`（常量 35ms/字符，注释说明拟人依据：中文输入 25-45 字/分钟量级的保守中值）；`total = base + typing`，封顶 `max_ms + TYPING_CAP_EXTRA_MS`（常量 6000ms）。
- 语义：短消息（"好的"×2 字）行为与现状几乎一致（+70ms）；120 字长段 ≈ base+4.2s，像真人打完一段的节奏。
- 单测：0 字符=现行为逐值等价；长文封顶生效；单调性（字符多间隔不减）。

**Steps:**
- [ ] grep 调用方 → 读 dispatcher 分段发送循环全文 → 失败单测 → 实现 → 测试绿。
- [ ] Commit：`feat(pacing): weight inter-segment send interval by content length (typing-time realism)`

### 收尾

- [ ] `cargo test --lib` ≥2534/0、四 PBT、`-D warnings`、禁词 lint。
- [ ] 交付报告（≤15 行）：锚点重验结论、词表复用与 profile 门的接线说明、调用方清单、测试结果、commit hashes。

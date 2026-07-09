# 被动回复豁免每日触达上限 + 过渡回复改 AI 生成 —— 设计

- 日期：2026-07-10
- 状态：设计（待评审 → writing-plans）
- 关联代码：`src/agent/gateway.rs`、`src/agent/escalation/logic.rs`、`src/agent/escalation/mod.rs`、`src/evolution/lint.rs`
- 关联既有设计：`docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`、`docs/superpowers/specs/2026-06-28-customer-reply-guarantee-design.md`（本设计部分推翻其"占位一律硬编码"的结论）

## 背景与问题

2026-07-10 真实微信全量测试中，测试好友「吴界」连续多轮收到一模一样的机械回复
「这个我帮你确认一下，稍等我给你准信。」。系统审查（全量读码 + file:line 亲验）定位到
**两个相互独立的业务语义缺陷**，都不是实现 bug，而是"有意设计但与正确业务语义不符"。

### 问题一：`daily_limit`（每日触达上限）错误地拦下了客户主动消息的回复

`precheck_send_gateway`（`gateway.rs:3091` 起）的 `daily_limit` 门（`gateway.rs:3129`）位于
`if !is_relay` 块内，**没有任何 trigger 类型区分**，对以下两类一视同仁地生效：

- `AgentTrigger::Inbound`：客户主动发消息 → AI **被动回复**
- `AgentTrigger::FollowUp`：AI **主动触达**（planner / follow-up 跟进任务）

后果：客户主动发消息，AI 却因"过去 24h 已对该客户发够 `max_daily_touches` 条"而不给真答案，
只回一句硬编码占位。吴界当时 `max_daily_touches=2`，24h 内 outbound=6，从第 3 条起全部撞
`daily_limit` → 每次补同一句占位 → 观感就是"反复回同一句"，且画像停在 initial（precheck 在
决策前拦截，Reply Agent 根本没跑）。

**这是错的语义**：`daily_limit` 的初衷是"防主动骚扰 / 防封号"（`daily_touch_count` 注释
`gateway.rs:693` 自称"骚扰门"，账号级软上限 `gateway.rs:3456` 自述"防封号观测"）。客户主动
问问题属于"客户期待内的被动应答"，不该被主动触达上限拦掉。

**佐证**：紧邻的 `quiet_hours` 门（`gateway.rs:3154`）已经用 `matches!(trigger, FollowUp(_))`
只作用于主动发送，注释（`gateway.rs:3148-3153`）明说"入站的静默延迟在 webhook 层已权威"。
`relay` 豁免的理由（`logic.rs:172-173`）也白纸黑字："领导回复是客户期待内的被动应答，不该被
rate_limited/cooldown/daily_limit 拦掉"——同样的逻辑完全适用于普通客户消息。`daily_limit`
缺的正是这层"主动/被动"区分。

### 问题二：给客户的过渡 / 占位回复是硬编码，不是 AI 生成

三个 `&'static str` 死文案（`escalation/logic.rs:85/92/99`）：

- `fallback_holding_reply()`（:85）= 「这个我帮你确认一下，稍等我给你准信。」——闸门拦截后
  所有"客户回应保障占位"的唯一文案来源（`gateway.rs:3418` 构造，`gateway.rs:1043/2189/2276`
  三处挂载）。
- `chain_tail_holding_reply()`（:92）——链尾失联持续安抚（`escalation/mod.rs:390`）。
- `expired_authorization_neutral_reply()`（:99）——relay 授权过期收尾（`escalation/mod.rs:201`）。

对比：正常回复走 decision Agent LLM 生成；领导裁决转述（relay）也走 LLM 重组
（`relay_principal_decision_to_customer` `gateway.rs:755` → 合成消息重入网关 `gateway.rs:768`）。
**唯独过渡 / 占位 / 安抚这一类是死文案**，客户连续触发就反复收到一模一样的机械回复，破坏拟人。

既有设计（`2026-06-28-customer-reply-guarantee-design.md` §2.2 / §7）当初刻意选硬编码，理由是
①避开"埋 LLM 产出字段被无视"的历史教训；②确定性保证不触 no-human-takeover 红线禁词
（当时**没有**运行期出站禁词守卫，只有 CI 静态 lint）；③单条短文案不破坏去抖分段。本设计
在补齐"独立预算旁路 + 运行期禁词守卫 + 硬编码降级兜底"三根支柱后，推翻"一律硬编码"的结论，
改为"AI 生成为主、硬编码为最终兜底"。

## 目标

1. `daily_limit` 只限制 AI 主动触达（`FollowUp`）；客户主动消息的被动回复（`Inbound`）完全豁免。
2. 所有给客户的过渡 / 占位回复改为 AI 生成（含闸门拦截占位、请示安抚、链尾失联、授权过期收尾）；
   硬编码文案降级为"LLM 失败 / 禁词命中 / 预算耗尽"时的最终兜底。
3. 补齐运行期出站禁词守卫，保证 AI 生成的过渡回复不触 no-human-takeover 红线。

## 非目标

- 不改 `max_daily_touches` 的默认值（仍 3，`models.rs:3729`）。仅收窄其语义为"主动触达上限"。
- 不改 relay 转述链路（已是 AI 生成，且已有出站守卫 `gateway.rs:2471-2511`）。
- 不引入新的多租户 / 账号级限流机制。防刷屏仍靠既有 `min_reply_interval` + 账号级软上限。
- 不改 webhook 层的静默时段权威判定。

## 设计

### 修复一：`daily_limit` 豁免被动回复

**改动点**：`gateway.rs:3129-3131`，把 daily_limit 门从"全部非 relay"收窄为"仅 FollowUp"，
参照现成的 `quiet_hours` 门范式（`gateway.rs:3154`）：

```rust
// 现状（对所有非 relay 一视同仁，含 Inbound 被动回复）：
if daily_touch_count(state, contact).await? >= runtime.max_daily_touches {
    return Ok(blocked("daily_limit", "已达到每日触达上限"));
}

// 改为（只作用于 FollowUp 主动触达；此位置已在 if !is_relay 块内，无需再判 relay）：
if matches!(trigger, AgentTrigger::FollowUp(_))
    && daily_touch_count(state, contact).await? >= runtime.max_daily_touches
{
    return Ok(blocked("daily_limit", "已达到每日触达上限"));
}
```

**语义**：
- 客户主动发消息 → AI 被动回复：永不受每日触达上限限制（客户问就答）。
- `max_daily_touches` 只管 AI 主动骚扰客户的频次。默认值不动。

**防刷屏兜底仍在**：
- `rate_limited` / `min_reply_interval`（`gateway.rs:3123`）对 Inbound 仍生效，挡"同一秒疯狂刷屏"。
- 账号级软上限 `account_daily_sent_count`（`gateway.rs:3459`，防封号观测）不变。

**连带更新**：
- 注释 `gateway.rs:3100-3136` 门控顺序说明（标明 daily_limit 仅作用于主动触达）。
- `docs/agent-policy.md:110`（当前明文写"私聊自动回复也受每日触达上限"，改为"仅主动触达受限"）。
- 测试注释 `gateway.rs:5234`（"客户主动问也须 ack"的语义前提失效，改写）。

### 修复二：过渡回复改 AI 生成（三根支柱 + 统一生成器）

**新增统一生成器** `generate_holding_reply`（单一职责，签名如下概念形态）：

```
输入：
  - scene: HoldingReplyScene    // 场景类型（见下表）
  - 客户上下文（最近对话 / 画像，用于场景化）
  - authorized_substance: Option<&str>   // 仅 C 类（授权过期 / 链尾）传领导 substance
输出：String（保证非空、无禁词、无授权外数字）

流程：
  ① 用独立小预算调 LLM 生成场景化安抚话术
     （独立 RunBudget 实例，与主 run 隔离；额度仅够一次短文案）
  ② 出站禁词守卫：evolution::lint::passes_forbidden_words(text)（复用现成词表）
  ③ C 类额外过授权外数字守卫：escalation::relay_introduces_unauthorized_number
  ④ 全部通过 → 用 AI 文案；
     LLM 失败 / 超时 / 独立预算耗尽 / 禁词命中 / 数字越界任一 → 回落对应场景硬编码文案
```

**支柱 1 · 独立预算旁路**：新建 `holding_reply_budget`（独立 `RunBudget` 小额度实例），
专供占位生成，不受主 run 的 `RunBudget` 是否耗尽影响。这样 `blocked_by_budget`（主预算耗尽）
时仍能生成一次占位。超这份独立预算 → 回落硬编码。

> 可行性：`RunBudget::new(run_id, tokens, calls, ...)`（`src/agent/budget.rs:75`）本就支持自由
> 构造独立实例。既有子流程已用同款范式各开一份独立预算调 LLM 并与主 run 隔离——
> `memory.rs:1185`、`reaction.rs:40`、`prompt_shadow.rs:193` 均 `Arc::new(RunBudget::new(...))`。
> `gateway.rs:1463-1466` 还有"task-local budget 为 None 时临时构造 fallback 预算"的先例。
> 本设计的独立预算旁路复用这一既定模式，非新机制。LLM 调用统一走
> `generate_agent_json`（`src/agent/mod.rs:215`，唯一 LLM JSON 入口，负责 budget 记账）。

**支柱 2 · 运行期出站禁词守卫**：**直接复用现成的 `evolution::lint::passes_forbidden_words`**
（`src/evolution/lint.rs:33`），其词表（`lint.rs:13-28`）与 `scripts/check-no-human-takeover`
同款（`人工接管|人工介入|人工托管|接管|人工|takeover|hand-off|...`），已有单测。命中即视为
不合格 → 回落硬编码。复用它同时规避了"在 src/agent/ 新写禁词字面量会被 CI lint 自噬"的风险。

**支柱 3 · 硬编码降级兜底**：三个 `&'static str`（`logic.rs:85/92/99`）**保留不删**，降级为
"最终兜底"。核心不变量守住：LLM 失败 / 禁词命中 / 独立预算耗尽任一 → 硬编码兜底 →
**客户永不被晾死**。

**场景与接入点**：

| 场景 | 触发状态 / 位置 | LLM 生成？ | 硬编码兜底 |
| --- | --- | --- | --- |
| A · 闸门拦截占位（策略 / 安全类） | `held_by_ai_policy` / `blocked_by_safety_guard` / `blocked_unverified_product_claim` / `blocked_by_required_field` 等，`gateway.rs:913` `ensure_customer_acknowledged` | 是（主预算通常在） | `fallback_holding_reply()` |
| A' · 闸门拦截占位（资源耗尽类） | `blocked_by_budget` / `revision_failed`，同上位置 | 是（**用独立预算**，因主预算已耗尽 / LLM 已失灵） | `fallback_holding_reply()` |
| B · 请示领导期间即时安抚 | 方案 B 解耦后由 A 承担（`escalation/mod.rs:36-38`，`escalate_held_decision` 不再直发客户消息） | 是 | `fallback_holding_reply()` |
| C1 · 链尾失联持续安抚 | `scan_escalation_timeouts` 链尾分支 `escalation/mod.rs:390` | 是（后台 tick 也调 LLM，受独立预算约束） | `chain_tail_holding_reply()` |
| C2 · relay 授权过期收尾 | `handle_principal_decision_relay` 早退分支 `escalation/mod.rs:201` | 是 | `expired_authorization_neutral_reply()` |

**关于 A' / C 走独立预算的决定**：用户明确要求"凡给客户的话都是 AI 生成的"，语义纯正优先于
成本。即使 `blocked_by_budget`（主预算耗尽）或后台批量 tick（`scan_escalation_timeouts` 可能
一次扫多条），也为占位生成开一份独立小预算调一次 LLM；耗尽 / 失败再回落硬编码。

### 数据流

```
Inbound（客户主动消息）
  → precheck_send_gateway
      → daily_limit 门：matches!(trigger, FollowUp(_)) → 对 Inbound 恒不触发（修复一）
      → 其它闸门（held / blocked / budget / revision_failed）拦截
  → ensure_customer_acknowledged（gateway.rs:913）
      → generate_holding_reply(scene, ctx)
          → 独立预算调 LLM
          → passes_forbidden_words? + （C 类）数字守卫?
          → 合格用 AI 文案 / 否则硬编码兜底
      → outbox_enqueue（幂等键 #ack-placeholder，与既有一致）
  → dispatcher → MCP message_send_text

FollowUp（AI 主动触达）
  → precheck_send_gateway
      → daily_limit 门：matches!(trigger, FollowUp(_)) → 仍生效（防骚扰 / 防封号）
```

### 错误处理与降级

- LLM 调用失败 / 超时：`generate_holding_reply` 内部捕获，回落硬编码，记 `warn`，不阻断 run。
- 独立预算耗尽：回落硬编码。
- 禁词命中 / 授权外数字：丢弃 LLM 输出，回落硬编码，记 `warn` + 安全事件（参照 relay 守卫
  `gateway.rs:2482-2509` 的 `blocked_by_safety_guard` 事件范式）。
- `ensure_customer_acknowledged` 仍是 fail-soft：入队失败只 warn，不改终态、不阻断 run
  （与现状 `gateway.rs:953-961` 一致）。

## 测试

新增 lib 单测（保持 `cargo test --lib ≥ 350 passed, 0 failed` 基线）：

1. **修复一**：
   - `inbound` 且 `daily_touch_count >= max_daily_touches` → precheck 放行（不 `daily_limit`）。
   - `follow_up` 且超限 → 仍返回 `blocked("daily_limit", ...)`。
2. **修复二 · 降级链**：
   - LLM 成功且文案合格 → 用 AI 文案。
   - LLM 失败 → 回落对应场景硬编码。
   - 独立预算耗尽 → 回落硬编码。
   - AI 文案含禁词（`passes_forbidden_words` 命中）→ 回落硬编码（复用 lint.rs 现成测试范式）。
   - C 类 AI 文案含授权外数字 → 回落硬编码。
3. **不变量**：任一降级路径下，`generate_holding_reply` 返回非空文案（客户永不被晾死）。

集成测试：`ensure_customer_acknowledged` 在闸门拦截下仍入队一条占位（AI 或兜底），幂等键
`#ack-placeholder` 不变。

## 红线与合规

- **无人工接管红线**：AI 生成过渡回复经 `passes_forbidden_words` 运行期守卫 + CI 静态 lint 双保险；
  命中即回落已知无禁词的硬编码文案。红线在"运行期文本"层面首次获得代码级守卫（补齐 spec
  2026-06-28 缺口）。
- **客户永不被晾死**：硬编码兜底保留，是所有降级路径的终点。
- **防封号**：`daily_limit` 仍对主动触达生效；被动回复靠 `min_reply_interval` + 账号级软上限兜底。
- **反过拟合**：禁词守卫 / 数字守卫均为纯函数 + 多形态变体测试，不对单条对话点对点修补。

## 影响面小结

| 文件 | 改动 |
| --- | --- |
| `src/agent/gateway.rs` | daily_limit 门加 `FollowUp` 守卫（:3129）；`ensure_customer_acknowledged`（:913）/ `build_ack_enqueue_request`（:3402）接入 `generate_holding_reply`；注释更新 |
| `src/agent/escalation/logic.rs` | 三个硬编码文案保留（降级兜底）；可能新增 `HoldingReplyScene` 枚举 + 场景→兜底文案映射 |
| `src/agent/escalation/mod.rs` | 链尾（:390）/ 授权过期（:201）接入 `generate_holding_reply` |
| 新增生成器 | `generate_holding_reply`（落 gateway 或 escalation，视依赖而定），独立预算调 LLM + 守卫 + 降级 |
| `src/evolution/lint.rs` | 复用 `passes_forbidden_words`，可能上调可见性（若跨模块调用需 `pub`，现已 `pub`） |
| `docs/agent-policy.md` | :110 每日触达上限语义改写为"仅主动触达" |
| 测试 | 上述新增单测；`gateway.rs:5234` 注释修正 |

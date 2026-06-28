# 全自治"客户必有回应"保障 — 设计

**日期**：2026-06-28
**分支**：将新建（fix/customer-reply-guarantee 或并入既有流程）
**触发**：④reviewer 让位真测（run 3305cba7）发现 held_by_ai_policy 终态下客户零回复，深度探索证实是独立的、更深的全自治红线缺陷——非 ④ 问题。

## 1. 背景与根因（已全代码 + DB + 真测核实）

### 1.1 现象
④真测：assist 开 + 引荐场景，Decision 真 emit namecardToSend（让位生效），但 reply_text 因附带无知识背书的额外承诺（"准备合同/选档位"）+ 角色自相矛盾被 reviewer 判 factRisk=6 → final=held_by_ai_policy → **客户零回复**（conversation_messages outbound 空）。用户核心关注："拦截之后有没有继续回复对方，不要因为拦截导致没有回复对方。"

### 1.2 根因链（每环有 file:line 证据）

**根因一：held_by_ai_policy 是"垃圾桶"终态**（review/gates.rs:840-849）
finalize 末尾 else 分支：所有 `approved=false` 但未归到具体 Blocked* 的件都落 held_by_ai_policy，含硬闸失败（hallucination=6）却躲过 R5.4（verified_chunks 非空，因引用了无关的退费政策 verified chunk，gates.rs:660）的情况。混合两种语义：reviewer 主动 should_hold（gates.rs:773）vs 纯阈值不达标（gates.rs:840）。

**根因二：安抚客户与请示领导被错误耦合**（escalation/mod.rs:65-143）
给客户补发安全占位（mod.rs:137-143 `fallback_holding_reply()`）排在一连串**领导骚扰门之后**：① should_escalate_held(logic.rs:337，held_by_ai_policy 默认 escalate_ai_policy_hold=false)→return ② decider_chain 空→return ③ push_allowed(领导 daily_cap/quiet_hours)→return ④ has_pending_for_contact 去重→return。任一命中，**客户占位一起被跳过**。但"安抚客户"（应无条件）与"请示领导"（该受骚扰门约束）是正交两件事。

**根因三：三类终态默认彻底零回复**
held_by_ai_policy / blocked_by_required_field / blocked_by_budget（+revision_failed 归 held），should_escalate_held 对后两者落 `_=>false`（logic.rs:340），永不补占位。

### 1.3 铁证：系统在别处已贯彻该红线
relay **回程**（领导回复后）即使授权过期也**无条件**给客户发中性话术（mod.rs:182-212，注释明写"否则客户零反馈、被晾死"+fail-soft）。唯独 held **首发**路径漏了——是实现不一致的疏漏，非设计选择。

### 1.4 实测数据（server117，24 run）
final_review_status 分布：approved 16 / held_by_ai_policy 3 / ''（早期空）3 / blocked_by_required_field 2。no_reply=0、should_reply=false 实例=0——**LLM 从未主动判沉默**（A3 主动沉默路径现实未触发，但是无兜底的静默漏洞）。held_by_ai_policy 3 件全 biztest 测试数据。

## 2. 设计目标与红线

### 2.1 核心不变量（用户定：最彻底）
**Inbound 必有回应**：只要 trigger=Inbound（客户真发了消息），本轮无论什么终态，客户都必须收到至少一条 AI 回应（真回复或确定性中性占位），不留任何沉默漏洞。

### 2.2 红线（全程守）
- 不为过测试改业务逻辑/prompt/guards/**硬闸阈值**（过拟合是红线中的红线）。④ 那次 held 是 reviewer 正确拦了话术有瑕疵的回复——hallucination 闸工作正常，**不碰它**。
- agent-first：占位用**确定性文案**（`fallback_holding_reply()`），不靠 LLM 产出字段（避开 ⑨/④ 反复栽的"埋字段 LLM 无视"A/B 教训）。
- **绝不破坏去抖聚合**：现有"客户连发 N 条→4s 窗口去抖→聚合成一轮回复"（webhooks.rs:105-241）必须零影响。绝不能变成"对方发 N 条、我们回 N 条"。
- **绝不破坏拟人分段发送**：现有"一条逻辑回复→按换行/句界拆成最多 4 条短消息拟人分发"（split_reply_into_segments，gateway.rs:3074）必须零影响。占位是**单条短文案**（19 字 < 120 字单段上限），与分段逻辑正交：有真回复才走分段、零回复才补占位，两路径互斥永不同时触发。
- check-no-human-takeover lint：占位文案不含转接类禁词（`fallback_holding_reply()` 已过该 lint + 已有测试 tests/principal_decision_channel.rs:916）。
- DEFAULT 行为变更评估：现状"默认晾死"→改"默认兜底"是行为变更，但符合"客户永不被晾死"红线本意；非 Inbound / 后续接管路径保持字节等价。

## 3. 实现方案（方案 A：per-run 末尾回应保障守卫）

### 3.1 守卫位置与触发
在 `run_user_operation_gateway_inner`（gateway.rs:863-2704）的"客户零回复" return 点补占位。**因去抖已把 N 条客户消息塌成 1 run，per-run 守卫每批正常只触发一次**——不会回 N 条。

**挂载策略（避免多 return 点遗漏）**：守卫逻辑拆两层——
1. **纯函数 `should_send_ack_placeholder(trigger_kind, final_status, should_reply) -> bool`**：无副作用，据终态判定是否该补（5.1 全用例覆盖）。
2. **副作用函数 `ensure_customer_acknowledged(state, contact, run_id, source_event_id, should_abort)`**：内部先 `should_abort_send()` 复查（命中即不补），再 enqueue 占位。

挂载点是 gateway 内**两处**零回复出口（不是所有 return 点——多数早退 return 如 916/934 是 not_managed/精度问题，非"决策完成后零回复"）：
- **拦截分支**（gateway.rs:1886-2010，held/blocked/revision_failed）：在 `return Ok(())`（:2010）前调用。
- **precheck-abort 分支**（gateway.rs:2014-2087）：仅对**非排除清单**的 status（即不在 superseded/quiet_hours_deferred/rate_limited/cooldown/expired 内的零回复 status）。注：A3 主动沉默不走这里，见 3.5。
- **A3 no_reply**：在 Approved 分支 should_reply=false 的 cancel_task("no_reply") 处（gateway.rs:2200 附近）调用。

`should_abort_send` 复查（仿 gateway.rs:2316）放在 `ensure_customer_acknowledged` 内部，三处挂载点共用，确保"客户又发新消息→下轮真回"时不补占位。

### 3.2 补占位的精确条件（白名单 + 排除清单）
仅当**全部**满足才补占位：
1. `trigger.kind() == "inbound"`（客户真发了消息在等；FollowUp 是 AI 主动触达，排除）。
2. 本轮客户**零回复**（既无真回复入 outbox，也未由其它路径发任何客户消息）。因方案 B 把客户占位完全交给本守卫，零回复集 = **所有 Inbound 拦截/沉默终态**：`held_by_ai_policy` / `blocked_by_required_field` / `blocked_by_budget` / `blocked_by_safety_guard` / `blocked_unverified_product_claim` / `revision_failed`，以及 A3 主动沉默（approved + should_reply=false，落 no_reply）。**不挑终态、不依赖 escalate 开关**——只要 Inbound 客户没收到回复就补（呼应核心不变量）。
3. 补占位前**复查 should_abort_send()**（仿 gateway.rs:2316）：命中则不补（说明客户又发了新消息、下一轮会真回，补占位会与下轮回复竞争重复打扰）。
4. （方案 B 下无需"escalation 已发"去重——客户占位唯一来源就是本守卫，escalate_held_decision 不再发任何客户消息。）

**必须排除（这些"后续会被接管"，补占位=回 N 条 或 重复打扰）**：
- `superseded_by_new_inbound`（gateway.rs:2107/2323）：下一轮用更全上下文真回。
- `quiet_hours_deferred`（gateway.rs:2019）：重排到醒来真回。
- `rate_limited`（gateway.rs:2914）：本批刚回过。
- `cooldown`（gateway.rs:2905）：客户主动叫停冷却期，补占位违反客户意愿。
- `expired`（FollowUp 死任务，非 Inbound 当前轮）。
- `not_managed` / `context_changed`（仅 FollowUp 或非 managed，非 Inbound 当前轮）。

### 3.3 占位发送方式
走 **outbox**（非裸 MCP 直发），复用 `fallback_holding_reply()`="这个我帮你确认一下，稍等我给你准信。"。理由：享受 dispatcher 的在线门控（account.online）+幂等键，与现有发送路径一致。幂等键用 run_id 派生（如 `{source_event_id}#ack-placeholder`），防同 run 重复入队。

### 3.4 与 escalation 的解耦（方案 B，根治根因二）
**把 escalation/mod.rs:137-143 的客户占位补发从 escalate_held_decision 中移除**——escalate_held_decision 回归单一职责：只推请示卡给领导 + 落 pending 台账 + 写 awaiting 标记，不再发任何客户消息。客户占位**统一由 gateway 守卫负责**（3.2）。

**解耦的回归保障（关键）**：移除前，safety_guard / unverified_product 的客户占位来自 escalate_held_decision（escalate_safety_guard/escalate_unverified_product 默认 true，logic.rs:333-336）。移除后若守卫不覆盖这两类，会从"半保障"退化成"零回复"。故 3.2 的零回复集**必须包含** blocked_by_safety_guard / blocked_unverified_product_claim——守卫覆盖全部 Inbound 拦截终态，确保解耦零回归。

relay 回程的占位（mod.rs:201 expired 中性话术、mod.rs:396 链尾安抚）**不动**——那是 relay task 独立路径（领导已介入后的客户安抚），非 held 首发，与本守卫正交。

### 3.5 A3 主动沉默的处理
A3（approved + should_reply=false）走 Approved 分支（gateway.rs:2200 cancel_task("no_reply")），不在 1886 拦截分支。守卫需在 Approved 分支的 no_reply 路径也覆盖：Inbound + should_reply=false → 补占位（客户发了消息，AI 却判不回 = 晾死）。实测 should_reply=false 现实零触发，但守卫覆盖它以堵住静默漏洞（核心不变量要求）。

## 4. 组件与数据流

```
Inbound webhook → 去抖聚合(4s窗口,N条→1run) → gateway run
  → decision → review → finalize → finalize_status
  → [各终态分支]
       ├─ Approved + should_reply → outbox 文本回复(分段)         [已有,客户收到]
       ├─ Approved + !should_reply(A3) → no_reply                  [守卫补占位]
       ├─ Held/Blocked* → 拦截分支(1886) → escalate(只推领导)      [守卫补占位]
       ├─ superseded/quiet_deferred/rate_limited/cooldown → return [守卫排除,不补]
       └─ revision_failed → Held(held_by_ai_policy)                [守卫补占位]
  → ensure_customer_acknowledged 守卫:
       trigger=inbound && 终态∈零回复集 && !already_placed && !should_abort
       → outbox enqueue fallback_holding_reply()(幂等键 #ack-placeholder)
```

## 5. 测试

遵循「新增测试只增量叠加」「动态测试反过拟合四铁律」。

### 5.1 纯函数单测（本地 cargo test --lib）
- 守卫判定纯函数 `should_send_ack_placeholder(trigger_kind, final_status, should_reply) -> bool`：
  - inbound + held_by_ai_policy → true
  - inbound + blocked_by_required_field → true
  - inbound + blocked_by_budget → true
  - inbound + blocked_by_safety_guard → true（方案 B：原 escalation 占位移除后由守卫补）
  - inbound + blocked_unverified_product_claim → true（同上）
  - inbound + revision_failed → true
  - inbound + approved + should_reply=false（A3）→ true
  - inbound + superseded_by_new_inbound → **false**（排除：下轮真回）
  - inbound + quiet_hours_deferred → false（排除：醒来真回）
  - inbound + rate_limited → false（排除：本批刚回过）
  - inbound + cooldown → false（排除：客户主动叫停）
  - inbound + expired / not_managed / context_changed → false（非当前轮 Inbound）
  - **follow_up** + 任何零回复终态 → **false**（非 Inbound）
  - inbound + approved + should_reply=true → false（已正常回复）
- 幂等键派生纯函数测试（同 run 同派生键）。

### 5.2 集成测试（CI，Docker）
- Inbound + 构造 held_by_ai_policy → 断言 outbox 有一条 fallback_holding_reply 占位。
- Inbound + 构造 blocked_by_safety_guard / blocked_unverified_product_claim → 断言守卫补占位（验证方案 B 解耦后不回归）。
- Inbound 连发触发 superseded → 断言**无**占位（下一轮真回）。
- escalate_held_decision 跑后 → 断言**不再**直发客户占位（只推领导卡），客户占位来自守卫。
- 占位幂等：同 run 守卫跑两次 → outbox 只一条占位。

### 5.3 真模型回归（server117）
- 复跑 batch_a_domain4.py 路径2（assist 开签约引荐）：reviewer 仍可能 held（话术瑕疵），但**断言客户收到占位**（outbox 有 fallback_holding_reply 或 referral）。
- 换 seed 验证泛化（不同 held 触发场景都补占位）。

### 5.4 基线门（不回归）
- cargo test --lib ≥ 350/0；4 PBT 累计 ≥ 33/0。
- check-baseline + check-no-human-takeover + check-evolution-isolation 三 lint 绿。

## 6. 变更文件清单

| 文件 | 改动 |
|---|---|
| `src/agent/gateway.rs` | 3.1-3.5 守卫 `ensure_customer_acknowledged` + 纯函数 `should_send_ack_placeholder`；各零回复 return 点前调用；A3 no_reply 路径覆盖；5.1 单测 |
| `src/agent/escalation/mod.rs` | （方案 B）移除 mod.rs:137-143 客户占位补发，escalate_held_decision 只推领导（解耦根因二） |
| `src/agent/escalation/logic.rs` | `fallback_holding_reply()` 复用（不改文案） |
| `tests/` | 5.2 集成测试 |

## 7. 非目标（YAGNI）
- 不重构 held_by_ai_policy 垃圾桶终态分类（根因一）——它不影响"客户必有回应"目标，独立专题，本设计只保证落它的件也有占位。
- 不碰 review/gates 硬闸阈值（红线）。
- 不改去抖聚合机制（webhooks.rs）。
- 不给 AgentDecision 加 LLM 产出的占位字段（方案 C，违 A/B 教训）。
- relay 回程占位（mod.rs:201）不动。

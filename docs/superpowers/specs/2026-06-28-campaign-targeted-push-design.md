# 活动定向推送（按购买产品圈人 + 定向推送）设计 spec

> 日期：2026-06-28
> 上游底座：`docs/superpowers/specs/2026-06-15-objective-purchase-facts-design.md`（G2 products / G3 outcome_events / G4 project_entitlements）
> 状态：设计评审稿。本 spec 只定形态与不变量；具体函数签名/prompt 文案落码阶段定。

## 1. 背景与缺口

业务需求：在系统里宣布一个"活动/促销"，把活动信息**主动推送给已经购买过某个特定产品/服务的客户**（如"双11 老客续费 7 折，截止 11.15"推给买过年度会员的人）。

需求拆成三段能力，深度代码核实（见 §10）后确认两头已具备、中间断一环：

| 环节 | 现状 |
| --- | --- |
| ① 指令入口（总控AI / 前端发起） | 已具备（management agent + 前端可扩） |
| ② 按购买产品圈出客户人群 | **缺口**：无"按 product_id 反查 contacts"的查询 + 索引 |
| ③ 给该人群主动推送活动信息 | 已具备发送链路（follow_up→gateway→outbox），无批量扇出入口 |

**底座已就绪（objective-purchase-facts 全落地）**：
- G2 `products` 集合（产品目录，product_id/价格/SKU/status/attributes）；
- G3 `Contact.outcome_events`（成交事件，带 `productRef` 快照 + `verification` 可信度 + `eventKind` deal/reversal）；
- G4 `entitlements.rs` 纯函数：`project_entitlements`（持有投影）、`compute_customer_value_cents` / `classify_value_tier`（LTV 分层）。

**缺口本质**：G4 是 **per-contact 单向投影**（"给定客户→买过什么"），无 **反向 segment 查询**（"给定产品→所有买过的人"）。`OutcomeProductRef` 当初设计就声明"无独立索引、运行时内存 fold"（models.rs:457-458），本 spec 是第一个反向按裸 key 查 Mongo 的场景。

## 2. 设计决策（已与用户敲定）

| 维度 | 决策 |
| --- | --- |
| 发起入口 | **双入口、引擎共用**：核心引擎做扎实，总控AI 工具 + 前端表单都是它的薄封装 |
| 圈人条件 | **多维度 AND 组合**（产品 / 售后状态 / 价值分层 / 客户阶段），每维可空 |
| 活动内容 | **给意图，AI 为每人生成**：活动意图作为 follow_up content，Reply Agent 结合画像/持有/口吻生成个性化话术，走完整 gateway |
| 规模频控 | **复用 planner/gateway 全部现有闸门 + 活动级去重**（同一活动对同一人只推一次） |
| 人工把关 | **圈人后人工确认再扇出**（预览命中人数 + 抽样 → 确认 → 扇出） |

## 3. 总体架构

新建**活动定向推送引擎**（`src/routes/campaigns.rs` + segment 查询逻辑），segment 查询与 campaign 生命周期都在引擎里；总控AI 工具和前端表单都委托引擎 handler（沿用 management 工具委托 REST handler 的现有范式，management.rs:1639/2037 有范例）。

```
入口A：总控AI（management agent）         入口B：前端「活动」表单（本期可选，§9）
  wechatagent.preview_campaign (只读)        POST /api/campaigns + /preview
  wechatagent.dispatch_campaign (确认门)     POST /api/campaigns/:id/dispatch
                  └──────────┬──────────────────┘
                       campaigns 引擎（共用）
                  create → preview(圈人) → confirm_dispatch(扇出)
                              │
                    segment 查询（§5，两阶段）
                              │
                  批量建 follow_up AgentTask（content=活动意图）
                              │
          ┌───────────────────┴── 发送链路一字不改（已验证 §10）──┐
          task worker(CAS认领) → handle_follow_up_task →
          run_user_operation_gateway（precheck 频控 → Reply Agent →
          独立 Review → 产品声明红线 → 改写一次 → outbox → MCP 发送）
```

## 4. 数据模型（2 个新集合）

### 4.1 `campaigns`（活动实体，workspace+account 级）

```rust
pub struct Campaign {
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub title: String,
    /// 活动意图要点（注入 follow_up content，喂 Reply Agent 生成话术）。
    pub intent_text: String,
    pub segment_filter: SegmentFilter,   // §5
    /// draft → previewed → confirmed → dispatching → completed / canceled（闭集）。
    pub status: String,
    pub target_count: Option<i64>,       // preview 算出的命中数
    pub dispatched_count: i64,           // 实际建 task 数
    pub created_by: String,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}
```

status 闭集 + `assert_campaign_status_valid` 写入校验（仿 `ALLOWED_AGENT_TASK_STATUS` models.rs:752 模式）。

### 4.2 `campaign_sends`（每人推送台账）

```rust
pub struct CampaignSend {
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub campaign_id: ObjectId,
    pub contact_wxid: String,
    pub task_id: Option<ObjectId>,       // 关联建出的 follow_up AgentTask
    pub status: String,                  // enqueued / skipped_duplicate / ...
    pub created_at: DateTime,
}
```

**唯一索引 `(campaign_id, contact_wxid)`** = 活动级去重闸：同一活动对同一人只推一次（仿 outbox idempotency_key 幂等模式 indexes.rs:612）。扇出/并发重复时，已存在记录的 contact 跳过。

### 4.3 不碰 AgentTask 闭集模型

活动与任务的关联放 `campaign_sends.task_id`，**不新增 AgentTask 字段**。follow_up 任务的 `content` 承载活动意图（与 management.rs:1461 `create_follow_up_task` 自建 AgentTask 同形态）。

### 4.4 新增索引（contacts）

当前 contacts 只有 `{workspace_id, account_id, wxid}` unique 一条索引（indexes.rs:29-37）。新增：

```
{ "workspace_id":1, "account_id":1, "outcome_events.productRef.productId":1 }
```

**真实 key 路径混合大小写**（核实修正，§10）：`outcome_events`(snake_case，Contact 无 rename_all) + `productRef.productId`(camelCase，OutcomeEvent/OutcomeProductRef 带 `rename_all="camelCase"`）。`outcome_events` 是数组 → multikey 索引；`$elemMatch` 对内嵌对象多条件匹配可命中。

## 5. 人群圈选（segment 查询）

**结构化 filter，非任意表达式引擎**（YAGNI）。各维度 AND，每维 None = 不约束：

```rust
pub struct SegmentFilter {
    pub product_ids: Vec<String>,        // 买过其中任一（$in 取并集）；空 = 不限产品
    pub aftercare: Option<AftercareFilter>,   // InAftercare / Expired / Any；空 = 不限
    pub value_tier: Option<String>,      // high / mid / low；空 = 不限
    pub customer_stage: Option<String>,  // 走 system_taxonomies 字典；空 = 不限
}
```

**两阶段查询**（因购买事实是内嵌 + 派生）：

**阶段1 — MongoDB 粗筛**（命中新索引）：
```
contacts.find({
  workspace_id, account_id,
  agent_status: "managed",              // 只推已纳管客户
  // customer_stage 等裸字段也在此层 filter
  outcome_events: { $elemMatch: {
    productId 路径... ∈ product_ids,     // 真实路径 productRef.productId
    verification: { $in: ["staff_confirmed", "payment_verified"] },  // G4 红线：只认高可信
    eventKind: "deal"                    // 正向成交（旧记录缺省 deal）
  }}
})
```
> `product_ids` 为空时：跳过 outcome_events 条件，退化为"按其他维度圈全体纳管客户"（仍受阶段2 约束）。

**阶段2 — Rust 内存精筛**（复用 G4 纯函数）：
对粗筛结果逐 contact 调：
- `project_entitlements(&contact.outcome_events, &active_products, now, cap)` → 取净持有（退款抵消、净件数>0 才算真持有）；按 `product_ids` 校验确实净持有（粗筛只看有无 deal 事件，reversal 抵消要靠投影）；
- `aftercare` 维度 → 读投影的 `in_aftercare`；
- `value_tier` 维度 → `classify_value_tier(compute_customer_value_cents(&events), mid, high)`。

**为什么两阶段**：产品反查靠索引（性能），但净持有/售后/价值分层是 G4 运行时派生（spec §4.2 既定架构，productRef 无独立索引、内存 fold）。粗筛已把规模缩到"买过相关产品的人"，精筛 N 不大。

**复用而非重写**：`project_entitlements` / `compute_customer_value_cents` / `classify_value_tier` 全是 entitlements.rs 现成纯函数（:64/:493/:511）。**"只认高可信成交"G4 红线天然继承**（`conversation_inferred` 不进投影，entitlements.rs:51）。

**多租户隔离**：查询 filter 必含 `workspace_id`+`account_id`（IDOR 红线，延续 objective-purchase-facts §3.5）。`active_products` 按 `load_active_products(db, workspace_id)`（entitlements.rs:229）只取本 workspace。

**产出**：`(命中 contacts 列表, 命中总数)`，预览与扇出共用。

## 6. 活动生命周期 + 人工确认门

```
1. create_campaign(title, intent_text, segment_filter)
     → 落 campaigns，status=draft
2. preview_campaign(campaign_id)
     → 跑 §5 两阶段查询 → target_count
     → 返回「命中 N 人 + 活动意图 + 抽样 3-5 示例客户(名/持有/阶段)」
     → status=previewed
   ┌─── 人工确认门 ───┐
   │ 总控AI：返回预览，等确认串（§7.2）        │
   │ 前端：弹「命中N人，确认扇出?」            │
   └─────────────────┘
3. confirm_dispatch(campaign_id)
     → status=confirmed→dispatching
     → 重新跑一次圈人（防预览后数据漂移，§8）
     → 对每个命中 contact：
         • campaign_sends 插一条（唯一索引去重，已推过的跳过）
         • 引擎自建 follow_up AgentTask（content=intent_text，
           review_required=true，48h expiry，kind="follow_up"）—— 仿
           management.rs:1461 create_follow_up_task 自建形态，回填 task_id
     → status=completed，dispatched_count
4. task worker → handle_follow_up_task → run_user_operation_gateway
     → 每客户 Reply Agent 结合画像/持有/口吻生成个性化话术
     → 独立 review + 产品声明红线 + 改写一次 + outbox + MCP 发送
```

**自建 follow_up task，不调 planner**（核实修正，§10）：`emit_planner_follow_up`（planner/mod.rs:198）是私有 `async fn`，跨模块不可调。引擎照 management `create_follow_up_task`（management.rs:1461）自建 AgentTask（更内聚、不依赖 planner 内部）。

## 7. 总控AI 工具（入口 A）

### 7.1 两个工具（挂进 management 工具目录）

| 工具 | 风险档 | 职责 |
| --- | --- | --- |
| `wechatagent.preview_campaign` | **Readonly** | 创建+圈人预览（返回命中数+抽样） |
| `wechatagent.dispatch_campaign` | **恒确认**（见 7.2） | 确认后扇出 |

加工具三处改动（management.rs，核实确认）：
1. `merge_product_tools` 工具目录声明（:617-878，description 必须写清参数，否则 LLM 不会调）；
2. `execute_management_tool` match 分支（:1321-2317，委托 `crate::routes::campaigns::*` handler，仿 :1639/:2037）；
3. `tool_effect` 风险分级（:1096-1188，preview **必须显式列 Readonly** :1100-1109，否则落兜底 Low → 被 dry-run 拦截）。

### 7.2 确认门：必须用 `tool_always_requires_confirmation`（核实关键修正，§10）

**坑**：`post_management_message:344` 硬编码 `dangerous_confirm_enabled=false`，所以仅把 dispatch 定为 `Dangerous` 档**不会触发确认门**（测试 `confirmation_gate_off_by_default_phase1` management.rs:2655 证实 Dangerous 不强制确认），dispatch 会直接扇出 → 确认门落空。

**修法**：把 `wechatagent.dispatch_campaign` 加进 `tool_always_requires_confirmation`（management.rs:1263，当前仅 verify/batch_verify）——无视第一期 dangerous 全局开关恒走 `pending_confirmation → confirm_management_command` 链。单点加一个工具名，不动全局开关语义。这样总控AI 路径天然"先 preview(只读) → 暂存待确认 → 确认串 → dispatch 扇出"。

### 7.3 12 工具上限不影响（核实确认）

`execute_plan_tool_calls` 的 `.take(12)`（management.rs:105）限的是"一个 plan 里几个工具调用"，dispatch 工具内部自己循环给 N 个客户建 task 算 **1 个工具调用**，不受 12 限。

## 8. 错误处理与边界

- **预览后数据漂移**：confirm_dispatch 时**重新跑圈人**，不信预览快照人群（防预览→确认间退款/新购）。预览只给规模感。
- **空人群**：命中 0 → 预览如实返回 0；confirm 拒绝扇出（无意义）。
- **扇出中断/重试**：`campaign_sends` 唯一索引保证幂等，已建记录跳过，安全续跑。
- **非交易域**：`transaction_facts_enabled=false`（情感陪伴）产品表空、圈人恒空 → 功能空转，零扰动。
- **gateway 频控削减是预期，不算活动失败**（核实修正，§10）：
  - gateway 有自己的**每日发送上限 `max_daily_touches`**（gateway.rs:2917）+ `min_reply_interval`（:2911）+ cooldown + operation_policy——**与 planner 的 daily_emit_cap 是两回事**。批量圈人推送会被这些频控削掉一部分客户当日触达，这是预期行为。
  - **`context_changed` 自动取消**（gateway.rs:2965）：客户在建任务后又说过话 → 该 follow_up 被取消。对沉默老客户唤醒无碍，活跃客户推送可能被吞——预期。
  - 某客户被 cooldown/红线拦下是正常逐条决策，`campaign_sends` 记该条状态，不影响他人、不回滚活动。
- **task.content 不是发送原文**（核实修正，§10）：经 `follow_up_trigger_message_text`（gateway.rs:4869）变成"触发语境"喂 Reply Agent，客户收到的是 Agent 重新生成、过完 review 的话术（正是"给意图 AI 生成"语义）。
- **不依赖 `review_required` 做必过审开关**（核实修正，§10）：该字段全仓只写不读（routes/tasks.rs:73），独立 Review 本就对所有 trigger 无条件跑（gateway.rs:1310）。
- **命名红线**：campaign/segment 所有新增文案、status 名走 AI 中性词，过 `check-no-human-takeover` lint（新增行被扫）。

## 9. 范围边界（YAGNI）

**本期做**（后端引擎 + 总控AI 工具，完整可独立交付）：
- `campaigns` / `campaign_sends` 集合 + 索引；
- segment 两阶段查询（复用 G4 纯函数）；
- campaign 生命周期 create/preview/confirm_dispatch + REST 路由；
- 总控AI 两工具 + 确认门修正（§7.2）。

**本期不做**：
- ❌ 前端「活动」频道/Tab（第二入口；后端引擎 + 总控AI 路径不依赖前端即可全跑通；前端作为可选后续增量，避免首个 spec 出死页面）；
- ❌ 定时/周期活动（一次性手动触发）；
- ❌ 任意布尔表达式引擎（只固定维度 AND）；
- ❌ 活动效果统计仪表盘（campaign_sends 留了数据底座，分析另立专题）；
- ❌ 支付闭环、AgentTask 闭集模型改动。

## 10. 深度代码核实记录（2026-06-28，零猜测）

逐条对真实代码核实，含 4 处修正：

| 断言 | 结论 | 真实代码证据 |
| --- | --- | --- |
| follow_up task 走完整发送链（gateway+独立review+产品红线+频控），发送链路不改 | CONFIRMED | task worker CAS 认领 tasks.rs:193-211 → handle_follow_up_task gateway.rs:136-159 → run_user_operation_gateway；独立 review 对所有 trigger 无条件跑 gateway.rs:1310；产品红线 review/gates.rs:653-679 与 trigger 类型无关 |
| 按 product 反查的 Mongo 查询 + 索引可行 | CONFIRMED | $elemMatch 命中 multikey 索引；写入侧 add_outcome_event_inner shared.rs:1422 `$push to_bson(&outcome_event)` 真落 camelCase key |
| 加 management 工具三处改法 + 委托范式 + 12 上限不限内部 | CONFIRMED | merge_product_tools mgmt.rs:617 / execute_management_tool match :1321 / tool_effect :1096；委托范例 :1639/:2037；.take(12) :105 限调用数非内部 |
| **修正1**：索引真实 key 是 `outcome_events.productRef.productId`（混合大小写），非 `product_ref.product_id` | 已修 §4.4/§5 | Contact 无 rename_all（snake_case）models.rs:132；OutcomeEvent/OutcomeProductRef 带 `rename_all="camelCase"` models.rs:400/460；product_ref→productRef :435，product_id→productId :463，event_kind→eventKind :444 |
| **修正2**：dispatch 定 Dangerous 不触发确认门，必须用 tool_always_requires_confirmation | 已修 §7.2 | post_management_message:344 硬编码 `dangerous_confirm_enabled=false`；plan_requires_confirmation :1274；测试 confirmation_gate_off_by_default_phase1 :2655 证 Dangerous 不强制确认；恒确认仅 Irreversible + tool_always_requires_confirmation :1263 |
| **修正3**：emit_planner_follow_up 私有不可跨模块调，引擎自建 task | 已修 §6 | planner/mod.rs:198 `async fn`（无 pub）；management.rs:1461 create_follow_up_task 自建 AgentTask 先例 |
| **修正4**：gateway 有独立 max_daily_touches 频控 + context_changed 自动取消 + task.content 非原文 + review_required 只写不读 | 已修 §8 | max_daily_touches gateway.rs:2917、min_reply_interval :2911；context_changed :2965；follow_up_trigger_message_text :4869；review_required 只写 routes/tasks.rs:73 全仓无读取 gate |
| G4 红线"只认高可信成交"天然继承 | CONFIRMED | project_entitlements 内 verification_drives_entitlement 闭集 entitlements.rs:51（conversation_inferred 不进投影） |
| 两阶段查询的 G4 纯函数可复用 | CONFIRMED | project_entitlements entitlements.rs:64 / compute_customer_value_cents :493 / classify_value_tier :511，全 per-contact 纯函数 |
| campaign_sends 唯一索引去重模式有先例 | CONFIRMED | outbox idempotency_key unique 索引 indexes.rs:612 |

**结论**：设计全部技术断言已对真实代码闭环。核实发现并修正 4 处（索引真实 key 混合大小写 / 确认门机制 / planner 函数私有 / gateway 频控与 content 语义）。骨架（双入口引擎共用 / 两阶段圈人 / follow_up 扇出 / 确认门 / 活动去重）不变，假设已全部替换为真相。无残留猜测。

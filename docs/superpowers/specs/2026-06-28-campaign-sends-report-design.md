# 活动推送结果查询（漏推可观测性）设计 spec

> 日期：2026-06-28
> 上游：`docs/superpowers/specs/2026-06-28-campaign-targeted-push-design.md`（活动定向推送，已合并 main d615bdc）
> 分支基线：`feat/campaign-sends-report` ← `origin/main` d615bdc
> 状态：设计评审稿。形态与不变量已定；函数签名/字段名落码阶段以 §7 核实记录为准。

## 1. 背景与缺口

活动定向推送（已上线）dispatch 时给每个圈中客户插一条 `campaign_sends` 台账（`status="enqueued"`）+ 建一个 follow_up `AgentTask`，task 经发送网关（gateway）跑完。但**真实推送结果（发出 / 被频控拦 / 被取消 / 还在队列）目前无法从活动维度查询**：

| 问题 | 现状（已核实，§7） |
| --- | --- |
| `campaign_sends` 是死台账 | 除 dispatch 的 insert + 回填 `taskId` 外，全仓无任何读/更新点；status 永远停在 `enqueued` |
| 真实结果不在 task 上 | follow_up task 经 gateway 通过后停在 `outbox_enqueued`；outbox dispatcher 发送成功**只更新 `agent_run_logs.outbox_status="sent"`（凭 run_id），不回写 AgentTask** |
| 无查询入口 | campaigns 仅 3 个 POST handler（create/preview/dispatch），无 GET；前端零 campaign 引用 |

**结论**：底层没有静默丢失——被频控拦的客户在 `agent_run_logs.status`（如 `daily_limit`）、被取消在 `agent_run_logs.status`（如 `context_changed`）、真送达在 `agent_run_logs.outbox_status="sent"` 都有据可查。**缺的是把这些散落事实按活动维度聚合出来的查询端点。**

## 2. 设计决策（已与用户敲定）

| 维度 | 决策 |
| --- | --- |
| 总体方案 | **按需聚合查询**：新增只读 GET 端点，`campaign_sends` join `agent_run_logs`，实时算真实分布。`campaign_sends`/`campaigns`/gateway/worker **全部零写入改动**（否决了"worker 回写台账"方案——改动面大、碰核心发送循环、且 `outbox_enqueued` 这层 worker 看不到真送达态） |
| 返回粒度 | **汇总 + 明细两层**：顶层 `summary` 聚合桶计数，`items` 每人一行 |
| 分类语义 | **桶 + 原因细分**：每人按 `campaign_sends.status` + 最新 `agent_run_logs.status` + `outbox_status` 组合归桶，`blocked`/`canceled`/`unknown` 保留原始原因 |
| 取数策略 | **批量 `$in` + 内存取最新**：3 次固定查询（campaign_sends → run_logs `$in` → contacts `$in`），无 N+1；retry 同人多条 run log 取 `max(_id)` |

## 3. 总体架构

```
GET /api/campaigns/:id/sends                  （只读，AuthenticatedAdmin）
   │
   ├─ 1) campaigns.find_one({_id, workspaceId}) ── IDOR 校验归属，None→404
   │
   ├─ 2) campaign_sends.find({campaignId, workspaceId})
   │        → Vec<(contactWxid, taskId: Option<ObjectId>, sendStatus)>
   │
   ├─ 3) agent_run_logs.find({                        ← 批量 $in，一次查询
   │        source_event_id: { $in: [taskId.hex...] },
   │        source_kind: SOURCE_KIND_FOLLOW_UP_TASK    ← 引常量，非裸串
   │     })  → 内存 group by source_event_id，同人取 max(_id)
   │
   ├─ 4) contacts.find({ workspace_id, account_id, wxid: {$in:[...]} })  ← 补客户名
   │
   └─ 5) 纯函数分类 + 聚合 → { campaignId, title, status, summary, items }
```

**改动面**：`src/routes/campaigns.rs`（+1 handler +2 纯函数 +单测）、`src/routes/mod.rs`（+1 路由）。两个文件，零写链路、零 gateway/worker/model 改动。

## 4. 端点契约

```
GET /api/campaigns/:id/sends
  鉴权：AuthenticatedAdmin（沿用现有 admin 会话中间件）
  IDOR：campaigns.find_one({_id: oid, workspaceId: admin.current_workspace})
        命中不到 → 404 NotFound（不泄漏跨 workspace 活动存在性）
  :id 非法 ObjectId → 400 BadRequest（沿用 dispatch_campaign 的 ObjectId::parse_str 处理）
```

### Response shape

```json
{
  "campaignId": "...",
  "title": "双11老客续费7折",
  "status": "completed",
  "summary": {
    "targetCount": 500,
    "sent": 470,
    "pending": 8,
    "blocked": { "daily_limit": 25, "cooldown": 5 },
    "escalated": { "blocked_unverified_product_claim": 12, "held_by_ai_policy": 3 },
    "canceled": { "context_changed": 5 },
    "skipped": 3,
    "unknown": 2
  },
  "items": [
    { "contactWxid": "wxid_a", "name": "张三", "status": "sent" },
    { "contactWxid": "wxid_b", "name": "李四", "status": "blocked", "reason": "daily_limit" },
    { "contactWxid": "wxid_c", "name": "王五", "status": "escalated", "reason": "blocked_unverified_product_claim" }
  ]
}
```

- `summary.targetCount` = items 总数 = campaign_sends 行数。
- `summary` 的 `sent/pending/skipped/unknown` 是标量计数；`blocked/canceled/escalated` 是 `{reason: count}` 子 map（reason 二级细分）。
- `items` 每人一行；`sent/pending/skipped` 不带 reason，`blocked/canceled/escalated/unknown` 带 `reason`（原始底层 status 值）。
- `escalated` = 已转交幕后领导请示、待裁决后 AI 会继续触达的件（产品声明待背书 / 安全门 / AI 策略暂缓 / 等更多上下文），区别于 `blocked`（纯频控/硬约束、无后续）。详见 §5.3。
- 字段名 camelCase（前端 JSON 契约一致，沿用 ProductView/preview 返回风格）。

## 5. 聚合算法

### 5.1 取数（3 次固定查询，无 N+1）

1. **台账**：`campaign_sends.find({ campaignId: oid, workspaceId })`（已有唯一索引 `(campaignId, contactWxid)` 加速）→ 收集 `(contactWxid, taskId, sendStatus)`。
2. **run log 批量**：`taskId` 取 `.to_hex()` 集合 → `agent_run_logs.find({ source_event_id: {$in: hexes}, source_kind: SOURCE_KIND_FOLLOW_UP_TASK })` → 内存 `group_by source_event_id`，**同一 task 多条（retry）取 `_id` 最大那条 = 最新一次 run**。
3. **客户名批量**：`contacts.find({ workspace_id, account_id, wxid: {$in: contactWxids} })` → `map<wxid, remark||nickname>`（与 preview_campaign 采样同源）。

### 5.2 混合大小写 key（已核实，照活动定向推送 Task 2 教训）

- `campaign_sends` 查询用 **camelCase**：`campaignId` / `workspaceId`（CampaignSend 带 `rename_all="camelCase"`）。
- `agent_run_logs` 查询用 **snake_case**：`source_event_id` / `source_kind`（AgentRunLog **无** rename_all）。
- `contacts` 查询用 **snake_case**：`workspace_id` / `account_id` / `wxid`（Contact 无 rename_all）。

### 5.3 分类（纯函数，7 桶，优先级自上而下命中即停）

`classify_send_outcome(send_status: &str, run_log: Option<&Document>) -> (bucket, Option<reason>)`：

```
① send_status == "skipped_duplicate"（或无 taskId）
      → ("skipped", None)                     去重命中，当初没建 task
② 有 taskId 但 run_log == None（task 还没被 worker 跑到）
      → ("pending", Some("not_yet_run"))
③ run_log.outbox_status == "sent"
      → ("sent", None)                         唯一"真送达"（最高优先级）
④ run_log.outbox_status ∈ {"failed_terminal","canceled"}
      → ("canceled", Some(outbox_status))      outbox 终态失败/取消
⑤ run_log.outbox_status ∈ {"pending","in_flight"}
      → ("pending", None)                      进了发送队列，未发出/发送中
⑥ run_log.status（在 outbox_status 未命中上面后判定）：
   a. ∈ {allowed, outbox_enqueued, quiet_hours_deferred}
        → ("pending", None)                    放行/已入队/作息重排（会继续）
   b. ∈ {daily_limit, cooldown, rate_limited,
         policy_cooldown, policy_wait_user_reply, policy_consecutive_limit,
         blocked_by_required_field, blocked_by_budget,
         review_blocked, revision_failed, revision_skipped_invalid_direction,
         revision_skipped_budget_exceeded, revision_llm_failure, tool_loop_timeout,
         gateway_blocked}
        → ("blocked", Some(status))            频控/硬约束/改写失败/二次precheck拦截——没发出且无后续
   c. ∈ {blocked_unverified_product_claim, blocked_by_safety_guard,
         held_by_ai_policy, ai_waiting_for_more_context}
        → ("escalated", Some(status))          已转交幕后领导请示，待裁决后 AI 会继续触达
   d. ∈ {context_changed, expired, not_managed,
         no_reply, admin_cancelled, superseded_by_new_inbound}
        → ("canceled", Some(status))           取消（无后续）
   e. 其它（legacy_mode_unchecked / precheck_blocked / 不认识的值）
        → ("unknown", Some(status))            诚实标，绝不强划进 sent
⑦ run_log 存在但 status 字段缺失
      → ("unknown", None)                      诚实标
```

**桶全集**：`sent / pending / blocked / canceled / escalated / skipped / unknown`（7 桶）。

底层 status 字面量全部来自 §7 核实（`GATEWAY_STATUS_VALUES` 闭集 run_envelope.rs:86-135 + `OutboxStatus` 闭集），逐值明确归桶，无值意外落 unknown（仅灰度/口径态与真未知归 unknown）。

**关键设计点**：
- **优先级**：`outbox_status=="sent"`（③）先于一切 `status` 判定，即便 status 还留着 `daily_limit`/`allowed` 也归 sent（真送达优先）。
- **escalated 单列（⑥c）**：`blocked_unverified_product_claim`（产品声明无 verified 背书）/ `blocked_by_safety_guard` / `held_by_ai_policy` / `ai_waiting_for_more_context` 触发后**走请示通道交幕后领导裁决**（should_escalate_held 默认升级，§7），领导补 verified 知识/裁决后 AI 会继续触达——它们**不是失败漏推**，与 `blocked`（纯频控、无后续）语义不同，故单列一桶让运营区分"在请示流程里"vs"彻底没推成"。这符合"无人工接管"红线（客户始终只跟 AI 对话）。
- **红线不拆**：产品红线拦的是 AI 据活动意图临场对客户生成的"未背书具体产品话术"，非运营的活动指令本身（活动意图只是内部触发语境）。本端点只**如实呈现**该状态，不改红线行为。

### 5.4 聚合

`build_sends_summary(items: &[SendItem]) -> Summary`：纯函数，遍历 items，`sent/pending/skipped/unknown` 标量 +1；`blocked/canceled/escalated` 按 reason 二级 map +1（escalated 的 reason 二级 map 尤其重要——区分产品红线 / 安全门 / AI 策略 / 等上下文）。

## 6. 错误处理与边界

- **空台账**（活动建了从没 dispatch）→ campaign_sends 空 → summary 全 0、items `[]`，200 返回（不报错）。
- **taskId 为 None**（dispatch 去重命中那条 `skipped_duplicate`）→ 不进 run log `$in` → 归 skipped。
- **retry 多条 run log** → `max(_id)` 取最新。
- **老 run log 字段缺失**（`source_event_id`/`outbox_status` 是 `#[serde(default)]`，历史 doc 可能空）→ 落 unknown，不崩。
- **IDOR**：campaign find_one + campaign_sends find 的 filter 均含 `workspaceId = admin.current_workspace`（沿用 campaigns.rs 既有红线，写入侧 workspace 由会话注入绝不信前端）。
- **命名红线**：新增行无 `人工/接管/takeover/hand-off` 等词（端点纯技术词 sent/blocked/canceled，天然安全），过 `check-no-human-takeover` lint。

## 7. 范围边界（YAGNI）

**本期做**：
- `GET /api/campaigns/:id/sends` handler + 路由注册；
- `classify_send_outcome` / `build_sends_summary` 两个纯函数 + 单测（逐桶 + 边界）。

**本期不做**：
- ❌ 前端「活动效果」页面/Tab（现状前端零 campaign 引用；本端点只提供数据接口，前端仪表盘后续增量）；
- ❌ 回写 `campaign_sends.status` / 任何写链路改动（已否决 worker 回写方案）；
- ❌ 分页（单活动人群受 managed 客户数限，通常几十到几百，一次性返回可接受；真到万级再另立专题）；
- ❌ CSV 导出 / 时间序列趋势 / 跨活动对比（分析专题）。

## 8. 测试

纯函数为主，全 lib 单测，零 DB、零 LLM：

```
classify_send_outcome 逐桶：
  ① send_status=skipped_duplicate → skipped
  ② 有 taskId、run_log=None → pending/not_yet_run
  ③ outbox_status=sent → sent
  ④ outbox_status=failed_terminal/canceled → canceled + reason
  ⑤ outbox_status=pending/in_flight → pending
  ⑥a status ∈ {allowed,outbox_enqueued,quiet_hours_deferred} → pending
  ⑥b status ∈ {daily_limit,cooldown,rate_limited,policy_*,blocked_by_required_field,
       blocked_by_budget,review_blocked,revision_failed,revision_skipped_*,
       revision_llm_failure,tool_loop_timeout,gateway_blocked} → blocked + reason
  ⑥c status ∈ {blocked_unverified_product_claim,blocked_by_safety_guard,
       held_by_ai_policy,ai_waiting_for_more_context} → escalated + reason
  ⑥d status ∈ {context_changed,expired,not_managed,no_reply,admin_cancelled,
       superseded_by_new_inbound} → canceled + reason
  ⑥e status ∈ {legacy_mode_unchecked,precheck_blocked,未知值} → unknown + reason
  ⑦ run_log 有但 status 字段缺失 → unknown/None
  优先级：outbox_status=sent 时即便 status=daily_limit 也归 sent（命中即停）
  escalated 优先级：status=blocked_unverified_product_claim 且无 outbox=sent → escalated（非 blocked）

build_sends_summary：
  混合桶 items → 标量计数(sent/pending/skipped/unknown) + blocked/canceled/escalated reason 二级 map 正确
  空 items → 全 0（空活动不崩），blocked/canceled/escalated 为 {}
```

分类是纯函数（输入 send_status + run log doc → 输出 bucket+reason），零 LLM、零关键词猜测——符合项目 agent-first 但客观度量用确定性函数的立场。

## 9. 深度代码核实记录（2026-06-28，基于 origin/main d615bdc，零猜测）

逐条对真实代码核实（三路 opus 只读核实 + 主 agent 亲自在 origin/main 复核）：

| 断言 | 结论 | 真实代码证据（origin/main） |
| --- | --- | --- |
| `campaign_sends` 全仓除 dispatch insert + 回填 taskId 外无读/更新点 | CONFIRMED | grep `campaign_sends()`/`CampaignSend` 全引用：models.rs:597 定义 / db/mod.rs:391 accessor / indexes.rs:752 索引 / campaigns.rs:332 insert、:350 回填；无其它 |
| CampaignSend 有 `taskId`、camelCase、status 仅 enqueued/skipped_duplicate | CONFIRMED | models.rs:597-608（`#[serde(rename_all="camelCase")]`，`task_id: Option<ObjectId>`） |
| AgentRunLog 有 source_event_id/source_kind/status，snake_case，`#[serde(default)]` | CONFIRMED | models.rs:2631 struct（无 rename_all）/ :2686 source_event_id / :2690 source_kind |
| 关联键：FollowUp → `task.id.to_hex()` + `SOURCE_KIND_FOLLOW_UP_TASK` | CONFIRMED | gateway.rs:456-461 `trigger_envelope_source` |
| 常量字面量 = `"follow_up_task"`（落码引用常量，非硬编码） | CONFIRMED | run_envelope.rs:43 `pub const SOURCE_KIND_FOLLOW_UP_TASK: &str = "follow_up_task"` |
| precheck blocked 把 `precheck.status` 写进 `agent_run_logs.status` | CONFIRMED | gateway.rs:899-916（`write_agent_run_log(..., &precheck.status, ...)`） |
| precheck block status 全集 | CONFIRMED | gateway.rs precheck_send_gateway + precheck_operation_policy：not_managed/cooldown/rate_limited/daily_limit/expired/quiet_hours_deferred/context_changed/policy_cooldown/policy_wait_user_reply/policy_consecutive_limit；通过=allowed |
| follow_up task 经 gateway 通过后停在 `outbox_enqueued`，dispatcher 不回写 task | CONFIRMED | gateway.rs:2123-2138 写 outbox_enqueued；OutboxEntry 无 task_id 字段（models.rs:2764）；dispatcher 仅 update_run_log_outbox_status（outbox_dispatcher.rs:207 凭 run_id） |
| dispatcher 发送成功写 `agent_run_logs.outbox_status="sent"`（凭 run_id） | CONFIRMED | outbox_dispatcher.rs:712 `update_run_log_outbox_status(state, &entry.run_id, "sent")` |
| OutboxStatus 闭集 | CONFIRMED | outbox.rs:41-65：pending/in_flight/sent/failed_terminal/canceled |
| campaigns.rs 现 3 handler、路由注册 mod.rs:782-784 | CONFIRMED | create(:194)/preview(:227)/dispatch(:280)；mod.rs:37 mod、:257 use、:782-784 route |
| **冲突核对**：main #53 后 12 提交动的文件与本设计依赖零重叠 | CONFIRMED | main 改 decision/media_send/memory/referral/review/prompts/knowledge + models.rs（仅 mod typed 行 3902+）；本设计依赖的 campaigns/mod/gateway/outbox_dispatcher/tasks/db 全部 UNCHANGED |
| **【最终审查补充】follow_up 走完整 gateway，status 不止 precheck 值** | CONFIRMED | finalize 写 run log 的 status = `outbox_enqueued`/`no_reply`（gateway.rs:2225）或 finalize 终态；`GATEWAY_STATUS_VALUES` 闭集（run_envelope.rs:86-135）是 `agent_run_logs.status` 全集——含 blocked_by_*/held_by_ai_policy/ai_waiting_for_more_context/review_blocked/revision_*/tool_loop_timeout/no_reply/admin_cancelled/superseded_by_new_inbound/quiet_hours_deferred。分类必须覆盖全集（§5.3 已扩 7 桶），否则漏计 |
| **产品红线触发条件（三 AND）** | CONFIRMED | gates.rs:653-686：`claim_requires_product_knowledge(claim_analysis)` ∧ `verified_chunks.is_empty()` ∧ `!priced_from_catalog` 三者全真才 `blocked_unverified_product_claim`。拦的是 AI 临场生成的"未背书具体产品话术"，非运营活动指令 |
| **活动意图是内部触发语境，非客户消息** | CONFIRMED | gateway.rs:4873 `follow_up_trigger_message_text` 把 task.content 包成"系统跟进任务到期…任务内容：{intent}"喂 Reply Agent，AI 据此重新生成面向客户的话术、走完整决策+知识路由+review |
| **escalated 类有请示通道出口（非死路、非人工接管）** | CONFIRMED | logic.rs:328-342 `should_escalate_held`：blocked_unverified_product_claim/blocked_by_safety_guard 默认升级、held_by_ai_policy 按开关；held=AI 内部状态，请示交幕后领导补 verified 知识/裁决后 AI 继续，客户始终只跟 AI 对话 |
| **【二审补充】gateway_blocked 归 blocked（非 unknown）** | CONFIRMED | 二道 precheck（gateway.rs:2013，LLM 决策后）命中频控/状态翻转时，顶层 `agent_run_logs.status` 写字面量 `"gateway_blocked"`（:2064），真实原因（final_precheck.status）落 `gateway_result` 子文档（:2070，SendGatewayResult.status types.rs:1500）。语义=被网关拦下没发出 → 归 `blocked`（泛标签 reason）。推翻 plan 原 §5.3e"看不到原因故 unknown"前提（原因可恢复，但泛标签桶已足够诚实，不耦合子文档 shape） |

**结论**：所有技术断言已对 origin/main 最新代码闭环。基线 = origin/main d615bdc（新分支 feat/campaign-sends-report 零落后），与 main 在建的优化零冲突。无残留猜测。

## 10. 设计澄清：产品红线与活动指令不冲突（最终审查留痕）

**质疑**：运营在总控 AI 下的活动推送是人类明确授权的最高指令，为何会被产品红线（`blocked_unverified_product_claim`）拦下？

**核实结论（100% 读全链，红线 HOLDS 不拆）**：
1. 运营说"双11老客7折" → 仅决定 `campaign.intent_text` 并据此建 follow_up task。**这一步红线不参与，畅通无阻**。
2. task 到期 → 活动意图作为**内部触发语境**喂 Reply Agent（gateway.rs:4873），AI **自己生成**面向客户的话术。
3. 红线只在 AI 生成的话术满足三 AND 条件（含未背书具体产品声明）时拦——拦的是"AI 临场对客户编的、无后台数据背书的产品数字/承诺"，**不是运营的活动授权**。人类授权了"搞活动"，但无法预先校验 AI 对每个客户临场编的每个数字；红线正是这层保护，CLAUDE.md 列为 baked-in 红线。
4. 触发后非死路：走请示通道交幕后领导补 verified 知识/裁决 → AI 继续触达（符合"无人工接管"——客户始终只跟 AI 对话）。

**本端点的职责**：只**如实呈现**这一状态（归 `escalated` 桶，与纯频控 `blocked` 区分），不改红线行为、不绕过它。运营看到 `escalated: {blocked_unverified_product_claim: N}` 即知"这 N 人的活动话术涉及待背书产品声明，已转请示，补料后会继续推"。

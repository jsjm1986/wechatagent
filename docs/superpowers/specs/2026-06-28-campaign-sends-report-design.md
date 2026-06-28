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
    "canceled": { "context_changed": 5 },
    "skipped": 3,
    "unknown": 2
  },
  "items": [
    { "contactWxid": "wxid_a", "name": "张三", "status": "sent" },
    { "contactWxid": "wxid_b", "name": "李四", "status": "blocked", "reason": "daily_limit" }
  ]
}
```

- `summary.targetCount` = items 总数 = campaign_sends 行数。
- `summary` 的 `sent/pending/skipped/unknown` 是标量计数；`blocked/canceled` 是 `{reason: count}` 子 map（reason 二级细分）。
- `items` 每人一行；`sent/pending/skipped` 不带 reason，`blocked/canceled/unknown` 带 `reason`（原始底层值）。
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

### 5.3 分类（纯函数，6 桶，优先级自上而下命中即停）

`classify_send_outcome(send_status: &str, run_log: Option<&Document>) -> (bucket, Option<reason>)`：

```
① send_status == "skipped_duplicate"（或无 taskId）
      → ("skipped", None)                     去重命中，当初没建 task
② 有 taskId 但 run_log == None（task 还没被 worker 跑到）
      → ("pending", Some("not_yet_run"))
③ run_log.outbox_status == "sent"
      → ("sent", None)                         唯一"真送达"
④ run_log.outbox_status ∈ {"pending","in_flight"}
   或 outbox_status 缺失但 run_log.status == "allowed"
      → ("pending", None)                      进了发送队列，未发出/发送中
⑤ run_log.status ∈ {daily_limit, cooldown, rate_limited,
                     policy_cooldown, policy_wait_user_reply, policy_consecutive_limit}
      → ("blocked", Some(status))              频控拦截，原因保留
⑥ run_log.status ∈ {context_changed, expired, not_managed}
   或 run_log.outbox_status ∈ {"failed_terminal","canceled"}
      → ("canceled", Some(status))             取消/终态失败，原因保留
⑦ 其它（字段缺失 / 不认识的值）
      → ("unknown", Some(原始值))              诚实标，绝不强划进 sent
```

**桶全集**：`sent / pending / blocked / canceled / skipped / unknown`。

底层 status 字面量全部来自 §7 核实（precheck `blocked(...)` 全集 + `OutboxStatus` 闭集），非猜测。优先级关键点：`outbox_status=="sent"`（③）先于 `status` 判定（⑤⑥），即便 run log 的 status 字段还留着 `allowed` 也归 sent。

### 5.4 聚合

`build_sends_summary(items: &[SendItem]) -> Summary`：纯函数，遍历 items，`sent/pending/skipped/unknown` 标量 +1，`blocked/canceled` 按 reason 二级 map +1。

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
  ④ outbox_status=pending / status=allowed → pending
  ⑤ status ∈ {daily_limit,cooldown,rate_limited,policy_*} → blocked + reason
  ⑥ status ∈ {context_changed,expired,not_managed}、outbox=failed_terminal/canceled → canceled + reason
  ⑦ 空 doc / 不认识的值 → unknown（不进 sent）
  优先级：outbox_status=sent 时即便 status 有值也归 sent（命中即停）

build_sends_summary：
  混合桶 items → 标量计数 + blocked/canceled reason 二级 map 正确
  空 items → 全 0（空活动不崩）
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

**结论**：所有技术断言已对 origin/main 最新代码闭环。基线 = origin/main d615bdc（新分支 feat/campaign-sends-report 零落后），与 main 在建的优化零冲突。无残留猜测。

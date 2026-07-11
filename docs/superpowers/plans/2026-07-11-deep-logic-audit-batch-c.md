# 深度审查批 C（成交活动链）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。**审查工程非写代码**：每 task 派 opus subagent 只读审一个环节、产出带 file:line 的 findings；主控逐条亲验后入台账。只审不修 src。

**Goal:** 从成交活动三频道（campaign/productsDeals/sendAnalytics）入口穿透后端，按业务流四环（圈人→触达→成交登记→成效聚合）逐环深审逻辑正确性与红线，续入统一台账批 C section。

**Architecture:** 4 业务环（Explore 已测绘），3 task 组织：Task1 触达环（最高·孤儿 send/部分失败/规模）、Task2 圈人环（受众一致性/净持有精筛/规模）、Task3 成交登记+成效聚合环（合并·多数已在批B链6审过，此处审上游入口+聚合口径）。复用批 A/B 方法（subagent 只读审 + 主控亲验 + 只入账不修）。

**Tech Stack:** Rust/Axum + MongoDB。审查工具 = Read/Grep（117 真跑同批 A/B 定调：多数触发前提生产无法安全构造，标 PLAUSIBLE，复现留修复阶段写测试）。

## Global Constraints（逐字继承批 A/B）
- **只入账不改 src**；**引用必亲验**（file:line 当场 Read/Grep，不靠 memory/不靠 Explore 测绘锚点免验）；**subagent 结论必主控亲验后入账**，凭猜驳回。
- subagent 一律 opus（harness 拒 model:"opus" 时省略继承主会话）。
- 台账续写 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`（新增"# 批 C"大节 + 各环 section），finding 编号用 `KC-NN`。
- 防假绿：端点/MCP 失败标 BLOCKED；PLAUSIBLE/CONFIRMED 如实标；发现≠修复。
- 权威依据：CLAUDE.md「AI 永不自证成交」「产品声明须 verified 背书」红线、`docs/agent-policy.md`。
- **与批 B 链6 不重复**：add_outcome_event_inner/approve_suspected_deal/entitlements 闭集已在批B亲验红线成立；批 C 审**上游入口衔接 + 触达/圈人 + 成效聚合口径**，链6 已覆盖的落库核心只做交叉引用不重审。

## Explore 测绘的四环锚点（审查起点，实现者仍须自己 Read 亲验）
- 圈人：build_segment_coarse_filter(campaigns.rs:31)/contact_matches_segment(:61)/resolve_segment_contacts(:178)/preview_campaign(:236)。受众不落库，dispatch 时重算。
- 触达：dispatch_campaign(:289)/活动级去重 campaign_sends 唯一索引(:341)/build_campaign_follow_up_task(:127)/tasks().insert_one(:351)。不直连 MCP，扇出 follow_up task 复用 gateway→outbox。
- 成交登记：add_outcome_event_inner(shared.rs:1453 唯一落库)/add_deal_event(contacts.rs:1407 入口1 manual)/approve_suspected_deal_inner(admin_suspected_deals.rs:139 入口2 CAS-first 硬编 staff_confirmed:203)/MCP write_deal_events(management.rs:1917 入口3)。
- 成效聚合：campaign_sends_report(campaigns.rs:492)/classify_send_outcome(:395)/send_ledger_stats(send_ledger.rs:73)/overview(:115)/contact_send_history(:40)/outcome_metrics(:30)。台账写源 agent/send_ledger.rs record_send(:42)/scan_send_ledger_outcomes(:176)。
- ✅ Explore 正面结论（仍须主控抽验）：无直连 MCP 绕 outbox；无鉴权缺失（全 AuthenticatedAdmin + require_session layer）；聚合读均带 workspace 过滤。

---

## Task 1: 触达环 dispatch_campaign 审查（最高优先）+ 批 C section 骨架

**Files:** 审查(只读) `routes/campaigns.rs`（dispatch_campaign:289 / build_campaign_follow_up_task:127 / campaign_sends 去重:341 / classify_send_outcome:395）+ `management.rs`（dispatch_campaign 工具入口:2333 / tool_always_requires_confirmation:1277）。台账新建"# 批 C"大节 + "## 触达环" section。

- [ ] **Step 1: 建批 C section 骨架 + 读透 dispatch 多步非事务写**

台账加"# 批 C（成交活动链）"大节 + 4 环空 section 占位。Read dispatch_campaign(:289-386) 全段：核 Explore 指认的孤儿 send——`campaign_sends` 插入成功（占去重位）后 `tasks().insert_one`(:351) 用 `?` 直接 return error 会留下"已占去重位但无 task"的孤儿（后续 campaign_sends_report 归 pending/not_yet_run 永不推进，且重发被去重索引挡住无法补）。亲验 ordering + `?` 位置，判严重度。

- [ ] **Step 2: 部分失败语义 + 规模行为**

核 dispatch 循环（:341-365）：某人扇出失败是中断整批还是跳过继续？status→dispatching→completed 的推进是否与实际成功数一致？命中规模无上限（resolve cursor 无 limit + 循环逐条 await）在大受众下的行为（超时/内存/task 洪峰）。dispatchedCount 与实际建 task 数是否一致。

- [ ] **Step 3: follow_up task 形态 + gateway 复用正确性**

核 build_campaign_follow_up_task(:127)：review_required/过期/max_attempts 设置；活动消息经 gateway 是否受同一批闸（cooldown/日上限/作息/managed 门）约束——即活动触达不会绕过自动回复的安全闸。classify_send_outcome(:395) 归桶是否覆盖所有 gateway status（不把某 status 误归成功）。

- [ ] **Step 4: 派 subagent 复审 + 主控亲验 + 入账 + Commit**

派 opus subagent 独立复审触达环（指令：先读懂再断言、带 file:line、凭猜驳回）。主控逐条亲验后写台账"触达环"section（含 ✅ 通过点）。`git commit -m "audit(batch-c): 触达环dispatch_campaign(最高优先)"`

---

## Task 2: 圈人环审查（受众一致性 + 净持有精筛 + 规模）

**Files:** 审查(只读) `routes/campaigns.rs`（build_segment_coarse_filter:31 / contact_matches_segment:61 / resolve_segment_contacts:178 / preview_campaign:236）+ `agent/entitlements.rs`（project_entitlements 净持有，批B已审闭集此处审精筛调用）。

- [ ] **Step 1: 两阶段筛选正确性（粗筛 Mongo + 精筛内存）**

核 build_segment_coarse_filter(:31)：workspace_id+account_id+managed 固定过滤；customer_stage 走 domain_attributes；product_ids $elemMatch 反查 outcome_events（verification∈{staff_confirmed,payment_verified}）——核 $elemMatch 条件是否严密（不把 conversation_inferred 或退款后的持有误算成命中）。contact_matches_segment(:61) 精筛三维（product/aftercare/value_tier）与粗筛是否口径一致（不会粗筛漏掉精筛需要的、或精筛放过粗筛该拦的）。

- [ ] **Step 2: preview/dispatch 一致性 + 净持有精筛规模**

核 preview_campaign(:236) 与 dispatch_campaign 各自独立跑 resolve_segment_contacts：targetCount（preview 存）与 dispatchedCount（dispatch 存）语义是否对齐/是否会误导运营；受众漂移（设计如此）是否有边界问题。resolve_segment_contacts(:178) 对每个 contact 跑 project_entitlements 内存精筛，cursor 无 limit/无分页，大 workspace 规模行为（退款抵消计算正确性 + 性能）。

- [ ] **Step 3: 主控亲验 + 入账 + Commit**

派 subagent 复审圈人环，主控亲验。写台账"圈人环"。`git commit -m "audit(batch-c): 圈人环audience筛选一致性"`

---

## Task 3: 成交登记 + 成效聚合环审查（合并）+ 批 C 汇总 + PR

**Files:** 审查(只读) `routes/contacts.rs`（add_deal_event:1407 入口1）+ `management.rs`（write_deal_events:1917 入口3）+ `routes/products.rs`（active products，被 grounding priced_from_catalog 引用）+ `routes/send_ledger.rs`（stats:73/overview:115/history:40）+ `agent/send_ledger.rs`（record_send:42/scan_send_ledger_outcomes:176）。台账批 C 总评。

- [ ] **Step 1: 成交登记三入口衔接（上游，链6 已审落库核心）**

核入口1 add_deal_event(contacts.rs:1407)：source=manual，verification 由前端传（可 staff_confirmed）——admin 手填 staff_confirmed 是否有额外校验/是否 AuthenticatedAdmin 门（人工登记 staff_confirmed 是合法的，核确无 AI 路径能走此入口伪造）。入口3 write_deal_events(management.rs:1917) 转调入口1——核 AI 工具调用最终落库时 verification 能否被 AI 设成 staff_confirmed（应只能 conversation_inferred 或被 validate_deal_verification 拦，批B已验 shared.rs:1410 拒 conversation_inferred，此处核 AI 工具入参链路）。products.rs active products 写入是否 workspace 隔离。

- [ ] **Step 2: 成效聚合口径 + 台账写源正确性**

核 send_ledger stats/overview/history + outcome_metrics 的 workspace/account 过滤一致性（Explore 说均带 workspace，主控抽验 2-3 处）；agg_count(send_ledger.rs:17) i32/i64 兼容读防静默清零；record_send(agent/send_ledger.rs:42) fail-soft 落台账；scan_send_ledger_outcomes(:176) 回扫 responded/stage_advanced 的率计算分子分母是否正确（响应率/推进率不虚高虚低）。campaign_sends_report(:492) join agent_run_logs 归桶正确性。

- [ ] **Step 3: 主控亲验 + 入账 + Commit**

派 subagent 复审成交登记+成效聚合，主控亲验。写台账"成交登记环"+"成效聚合环"。`git commit -m "audit(batch-c): 成交登记+成效聚合环"`

- [ ] **Step 4: 批 C 总评 + 自检防假绿 + PR**

台账加"批 C 总评"：finding 计数按严重度 + 跨环根因家族 + 修复优先级 + 与批 A/B 关联。复核每条 finding file:line 亲验、无夸大、无把设计当 bug。`git commit` + `git push` + 更新 PR#178（批 A+B+C 合并台账）。

---

## Self-Review 结论
- **Spec coverage**：Explore 测绘四环 ↔ Task1(触达)/Task2(圈人)/Task3(成交登记+成效聚合)，全覆盖；优先级（触达>圈人>登记聚合）↔ 独立 task 顺序；只入账不修 ↔ 全 task 无 src 改动 + Global Constraints。
- **Placeholder scan**：无 TBD；每 Step 给具体审查问题 + Explore 锚点（file:line，实现者仍须亲验）。
- **一致性**：台账文件名/finding 编号(KC-NN)/环锚点跨 task 一致；批 B 链6 已审的落库核心（add_outcome_event_inner/approve_suspected_deal/entitlements 闭集）明确标"交叉引用不重审"，批 C 只审上游入口衔接+触达圈人+聚合口径。
- **审查工程适配**：无 TDD；用"读码→对照红线→主控亲验→入账"替代；每 task 独立可提交 deliverable（台账 section）；117 真跑同批 A/B 定调（PLAUSIBLE，留修复阶段）。

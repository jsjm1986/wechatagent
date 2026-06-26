# 前后端业务对齐修复 批次3（其余 MEDIUM + 自治可观测深化）设计

> 本 spec 把批次3 的 19 条从父 spec `2026-06-26-frontend-backend-alignment-fixes-design.md` 的"方案要点"细化为可直接 writing-plans 的详设。父 spec 是全量 76→67 路线图，本文件只覆盖批次3。

- 状态：设计稿，待用户审。
- 基线：批次1（PR#44）+ 批次2（PR#46，merge `ae54a8f`）均已合并 main，CI 双门全绿。批次3 实现须基于此最新 main。
- 条目集（用户拍板）：**19 条** = A2 / A3 / B4 / C6 / C9 / D4 / D5 / E2 / E3 / E5 / E6 / E7 / E8 / E9 / E11 / E12 / E15 / E16 / F23。其中 C6/C9 是批次2 顺延来的（C9 依赖 C6 作宿主），F23 父 spec 归批次3。
- **行号说明**：本 spec 的 file:line 是 brainstorming 阶段（批次2 合并前后）的实证值。批次1/2 改过的文件（types/index.ts、ask_human_inbox.rs、operations/index.tsx、system-strategy/index.tsx、legacy.tsx）行号已变；**writing-plans 阶段须基于最新 main 重新 grep 实证行号**，本 spec 的行号仅供定位参考。

---

## 一、批次3 的性质（与批次2 的区别）

批次2 是单一反模式（前端把行业语义写死成销售域）的 9 个切面，高度同质。**批次3 是跨全部 6 业务域的杂项 MEDIUM 集合**，主题三条线：

1. **补全编辑/操作入口**（"只读不可写"反模式的剩余条目）：A2/A3/D4/D5/E5/E6/E7/E15。
2. **自治可观测深化**：B4（已裁决历史）/C6（run envelope）/C9（tier 遥测）/E2（引荐态可观测）/E12（proposal 详情）。
3. **三条前后端闭环**：E9（治理待办计数，轻后端补字段）/E11（高危确认流，重后端补端点）/F23（疑似成交闭环，重后端全链）。

**核心编排策略：按后端 blast radius 分三组**，这是任务排序与风险隔离的主轴。

---

## 二、条目分组（= 后续 plan 的任务簇）

| 组 | 条目 | 后端改动 | 风险 |
| --- | --- | --- | --- |
| 组一 纯前端 | A2 / A3 / B4 / C6 / C9 / D4 / D5 / E2 / E3 / E5 / E6 / E7 / E8 / E12 / E15 / E16 | 零 | 低（接线已有端点） |
| 组二 轻后端 | E9 | integrity-report 补 D2 降级字段 | 中 |
| 组三 重后端 | E11 / F23 | 新增端点 + 新语义（E11 续跑执行链 / F23 gateway 提取 + 专表 + 三端点） | 高（碰写路径/决策链） |

**执行顺序**：组一（低风险打底，可任意序）→ 组二 → 组三（后端语义重，最后做，独立 task，加固测试）。

**已 grep 实证的关键事实（brainstorming 阶段核实，writing-plans 须复核行号）**：
- C6：后端 `/agent-runs` 端点已存在（`mod.rs:375` → `tasks.rs:85` list_agent_runs），返回 `{items}`，每项经 `shared.rs:1091` agent_run_json **手工拼 camelCase 键**（id/runId/triggerKind/status/planner/context/knowledgeRoute/decision/review/gatewayResult/error/createdAt）。**纯前端可消费**。各阶段是 Document，内部键形态由写入侧决定。
- C9：tier 遥测（tier_used/sufficiency/escalated/forced_full）写入 `AgentRunLog.gateway_result`，随 C6 的 gatewayResult 阶段下发，前端在 C6 视图内读。
- E9：integrity-report（`knowledge/catalog.rs:133` get_operation_knowledge_integrity_report）只返回 `{total, verified, needsReview, rejected, items[]}`，**无 D2 降级字段**。D2 降级数据在 `digest_inbox.rs:454`（active 且无 source_anchors）。`trustTypes.ts:41/109` 的 `gaps[]` 解析后 **0 处消费（死字段）**。
- E11：`management.rs:104` post_management_message 对 dangerous 走 `take(0)` 跳过全部 tool（:192-195），plan 落库（`AgentCommandRun.plan` 是 ManagementPlan 全量序列化，models.rs:2930），run 标 `pending_confirmation`（:283）。`get_management_command`（:353）纯只读，**无任何续跑路径**。management 路由仅 4 个（mod.rs:773-785）。
- F23：suspected_deal 只是 `entitlements.rs:302` render_suspected_deal_guidance 产 prompt 引导，让 LLM 在 `agentGeneratedSignals`（AgentSignal: kind/value/evidence/confidence，types.rs:58）输出弱信号，**沉在 agent_run_logs.decision，无专用 collection、无按 kind 查 pending 通路**。对比 relationship_type 有 `extract_relationship_type_suggestion` + upsert 专表 + list/approve/reject（admin_relationship_suggestions.rs）。
- E16：products-deals 半已接 setError（index.tsx:172-218），**ask-human-config 的 DeciderChainEditor.tsx:27 是裸 `catch {}`（静默吞错）** → 只补 config 半。

---

## 组一：纯前端（16 条，零后端）

### A2. operating-memory PUT 手动编辑零接入 `[前端]`
- **现状**：`userOpsStore.ts:~293` 仅 GET operating-memory；memoryDraft 纯只读（legacy.tsx 渲染进 strong/p），无 input/onChange/提交，`setMemoryDraft` 全仓 0 命中。后端 `OperatingMemoryRequest`(contacts.rs:47) 四字段 user_understanding/relationship_state/product_fit/next_action 无前端写表单。
- **修复**：memoryDraft 区改可编辑表单 + 保存按钮 → PUT `/api/contacts/:id/operating-memory`；保存后回填。复用现有 store action 模式（参考 saveRelationshipType）。
- **测试**：vitest store 测 saveOperatingMemory 发 PUT；组件测编辑→提交。
- **验收**：需浏览器。

### A3. operation-profile 补 last_commitment / follow_up_policy 编辑 `[前端]`
- **现状**：`saveRelationshipType`(userOpsStore.ts:~479) PUT body 仅 relationshipType；后端 OperationProfileRequest(contacts.rs:31-42) 的 last_commitment/follow_up_policy 无人工编辑入口（customer_stage/intent_level **刻意只读** AI 派生，不动）。
- **修复**：补 last_commitment/follow_up_policy 编辑入口 → 并入 operation-profile PUT。customer_stage/intent_level 维持只读。
- **测试**：vitest store 测扩字段提交（断言 PUT body 含两字段，且不含 customer_stage/intent_level）。
- **验收**：需浏览器。

### B4. 请示已裁决记录 status=resolved 历史/裁决/授权过期展示 `[前端]`
- **现状**：聚合收件箱 `ask_human_inbox.rs` 写死 `"pending"`；后端 `list_principal_escalations`(principal_escalations.rs:25-58) 支持 status=resolved 投影 decision/authorizationExpiresAt/resolvedVia，前端无消费（仅 steward.tsx phase-rollup 有 resolved 聚合计数）。
- **修复**：ask-human 频道加"已裁决历史"视图（或筛选器），调 `/admin/principal-escalations?status=resolved` 展示裁决结果/授权到期/裁决渠道。
- **测试**：vitest store/组件测。
- **验收**：需浏览器。

### C6. Agent 运行日志（run envelope）视图 `[前端]`
- **现状**：全前端 grep agent-runs/runEnvelope 0 命中；operationsStore 只拉 events/tasks/decision-reviews/llm-usage，无 /agent-runs。后端 `GET /agent-runs`（mod.rs:375 → tasks.rs:85 list_agent_runs）已存在，返回 `{items}`，每项 agent_run_json 手工拼 camelCase（顶层 planner/context/knowledgeRoute/decision/review/gatewayResult 已就位）。
- **修复**：operations 或 autonomy 加运行日志视图，调 /agent-runs 展示 run envelope——单次运行的决策/复核/送达全链。顶层阶段键直接消费（camelCase 已就位）；各阶段 Document 内部字段按需展开（实现期 grep 写入侧确认内部键，或用通用 key-value 渲染兜底未知字段）。
- **边界**：C9 是本视图的子能力（见下），同 task 内一起做。
- **测试**：vitest store/组件测列表 + 单运行展开。
- **验收**：需浏览器。

### C9. run log tier 遥测展示 `[前端]` — 通用化（渐进式三档）
- **现状**：tier_used/sufficiency/escalated/forced_full 全前端 0 命中；写入 AgentRunLog.gateway_result 的遥测前端零呈现，账号级灰度/A-B 验证无数据面。
- **修复**：在 C6 的 run envelope 视图里，从 gatewayResult 阶段读 tier 遥测字段并展示（tier_used 用了哪档/sufficiency 自评分/escalated 是否升档/forced_full 是否强升）。
- **边界**：依赖 C6 视图落地，**与 C6 同 task 实现**（C6 是宿主，C9 是宿主内的字段展示，拆开无意义）。实现期需 grep gateway_result 写入侧确认 tier 遥测的内部键形态。
- **测试**：随 C6 组件测，给含 tier 遥测的 gatewayResult，断言字段显示。
- **验收**：需浏览器。

### D4. domain-profiles 版本回滚 rollback 无 UI `[前端]`
- **现状**：后端 rollback 端点存在、版本链(previous_version)已展示，但 DomainProfilePanel(system-strategy/index.tsx:~2007) 未挂 ActiveVersionsBar，无回滚按钮。
- **修复**：DomainProfilePanel 挂 ActiveVersionsBar（**复用现有组件**，prompt_templates/playbooks 已用同款），endpointPrefix 指 /api/admin/domain-profiles。
- **测试**：vitest 组件测回滚动作触发 POST。
- **验收**：需浏览器。

### D5. domain-profiles 手动新建空白配置链路死 `[前端]`
- **现状**：`newDomainProfileDraft()`(strategyStore.ts:~332) 置 editingProfile=null，但编辑区 `editing?<Editor>:<placeholder>` 在 null 时只渲染占位，onSave 在 null 时 no-op，永不 POST。手动建配置只能走 AI 生成。
- **修复**：修复新建链路——newDraft 时进入可编辑空白态（draft 初值用一个最小合法 DomainProfileDraft），saveDomainProfile 支持 POST（无 id 时 create，有 id 时既有 PUT）。
- **测试**：vitest store 测新建走 POST、编辑走 PUT。
- **验收**：需浏览器。

### E2. referral「已引荐」态状态可观测 `[前端]`
- **现状**：hydrateSelected(userOpsStore.ts:~265) 只读 assist_mode_override/relationship_type，不读 referred_specialist_at/referred_card_id。联系人详情面板无"已引荐/AI 已退辅助"显式指示（对话流 namecardBubble 有间接观测）。前端 grep referredSpecialistAt 0 命中。
- **修复**：hydrateSelected 读 referred 标记（Contact 类型补字段）；详情面板显式显示"已引荐态 / AI 已退辅助答疑"。与批次1 E1（撤销端点）同源——E1 已落地撤销，E2 补观测。
- **边界**：纯展示。措辞守无人工接管 lint（用"AI 已退辅助答疑"等，不用"人工接管"）。
- **测试**：vitest store/组件测 referred 标记渲染。
- **验收**：需浏览器。

### E3. chunk AI 修复 propose/answer 无入口 `[前端]`
- **现状**：后端 `mod.rs:484 POST /chunks/:id/repair` + `:488 /repair/answer`（repair.rs 注释明示 applyAiRepairPatch 落账闭环）；前端 grep 0 命中，today.tsx:509 "去修复"按钮仅 focus 跳转。有 ReviewChat 会话级 /chat 替代路径（故非彻底不可达）。
- **修复**：chunk 详情加结构化 AI 修复面板（propose patch → 显示 patch → AI 追问 answer → 接受落库）。
- **边界**：落库保持 needs_review 红线（AI 修复产 draft patch，人审接受才落）。
- **测试**：vitest 组件测修复流（propose→answer→accept）。
- **验收**：需浏览器。

### E5. 解除知识关联 unrelate 无 UI `[前端]`
- **现状**：后端 `mod.rs:524 DELETE /chunks/:id/relate/:target_id`；前端建立关联有 UI（shared.tsx:794），related_chunks 反向引用纯只读（shared.tsx:359），无解除按钮。
- **修复**：related_chunks 列表项加"解除关联"按钮 → DELETE relate/:target_id；成功后刷新关联列表。
- **测试**：vitest 组件测解除触发 DELETE。
- **验收**：需浏览器。

### E6. 文档元数据编辑 PUT documents/:id 无入口 `[前端]`
- **现状**：后端 `mod.rs:454 PUT /operation-knowledge/documents/:id`（crud.rs:108 replace_one 整文档替换）；前端 steward.tsx 只增/删/查切片，改文档只能删了重建。
- **修复**：steward.tsx 文档项加编辑表单 → PUT documents/:id。**注意是整文档替换**（replace_one），前端须回填完整文档字段再提交，避免漏字段被清空。
- **测试**：vitest 组件测编辑（断言 PUT body 含完整文档字段）。
- **验收**：需浏览器。

### E7. 手工单条新建切片 POST chunks 无入口 `[前端]`
- **现状**：后端 `mod.rs:463 POST /operation-knowledge/chunks`（crud.rs:192）；前端切片只经 import pipeline 产出，无手工单条新建表单。
- **修复**：steward.tsx 加"手工新建切片"表单 → POST chunks。**红线：新建切片 status=draft + integrity_status=needs_review，AI 永不自动验证**（核实后端 POST 是否已强制此态；若后端未强制，前端提交也须带 draft）。
- **测试**：vitest 组件测新建（断言提交态为 draft/needs_review）。
- **验收**：需浏览器。

### E8. ReviewChat 对话产 patch 后左栏无实时预览 `[前端]`
- **现状**：`ReviewChat.tsx:149` 仅取 turn.patch 为 boolean，patch 内容被弃用；左栏静态 prop chunk 不刷新，需放行后整列表 reload 才见改动。
- **修复**：ReviewChat 收到 turn.patch 后渲染 patch diff 预览 +（可选）实时刷新左栏 chunk。
- **测试**：vitest 组件测 patch 预览渲染。
- **验收**：需浏览器。

### E12. evolution proposal 详情 5 字段未渲染 `[前端]`
- **现状**：riskNote/diffSummary/evalMetrics/cohortRunIds/previousPromptVersion 在 proposalTypes.ts:95-117 有类型、test 有 mock，但 ProposalReleaseCard.tsx 零渲染。运营看不到风险提示/diff/评测/同批 run/前版本。
- **修复**：ProposalReleaseCard 渲染这 5 字段（结构化展示，evalMetrics 是 doc 用 key-value，cohortRunIds 是数组）。
- **测试**：vitest 组件测 5 字段渲染。
- **验收**：需浏览器。

### E15. 多 workspace 切换入口 UI 不可达 `[前端]`
- **现状**：后端 POST /api/auth/workspace + /auth/me workspaces[] 完整，handler 已在 authStore/main.tsx 接好（onSwitchWorkspace），但无 UI 触发，Shell:187 仅把 currentWorkspace 当纯文本显示。
- **修复**：Shell 侧栏把 workspace 文本改为下拉/切换器（调已接好的 onSwitchWorkspace handler）。**纯接线**（handler 已存在）。
- **测试**：vitest 组件测切换触发 handler。
- **验收**：需浏览器。

### E16. DeciderChainEditor 静默吞错补 setError `[前端]`
- **现状**：products-deals ContactPicker 半已接 setError（index.tsx:172-218，批次1 或并行会话已做）；**ask-human-config 的 `DeciderChainEditor.tsx:27` 仍是裸 `catch {}`**（静默吞错，加载失败显示空可选列表，无法区分真无联系人 vs 失败）。
- **修复**：DeciderChainEditor 的 catch 接 setError + 错误提示（同 C1 错误态模式）。**只补 config 半**（products 半已做）。
- **测试**：vitest 组件测错误态（mock 加载失败，断言显示错误而非空列表）。
- **验收**：需浏览器。

---

## 组二：轻后端（1 条）

### E9. 治理待办三计数错配 `[前后端]`
- **现状**：CockpitView.tsx:78-95 三 MetricCard 实为：待审草稿(`integrity.needsReview`)/需复核(**实取 `integrity.rejected`，标签错配**)/知识总数(`integrity.total`)，与 spec 4.1 的「待审草稿数 / D2 降级数 / 知识缺口数」不符。后端 integrity-report（`knowledge/catalog.rs:133`）**无 D2 降级字段**；D2 降级数据在 `digest_inbox.rs:454`（active 且无 source_anchors）。`gaps[]`(trustTypes.ts:41/109) 解析后被丢弃（死字段）。
- **修复（后端）**：integrity-report 补 D2 降级计数字段（`anchorsMissing` 或 `d2Degraded`）——复用 digest_inbox.rs:454 的查询逻辑（active 且无 source_anchors 的 chunk 计数），加进 integrity-report 返回 JSON。**注意 blast radius**：integrity-report 现有消费方（CockpitView 三卡）不回归——加可选字段不破现有。
- **修复（前端）**：CockpitView 三 MetricCard 改为 spec 4.1 三计数：待审草稿（`integrity.needsReview`）/ D2 降级（后端新字段）/ **知识缺口（`knowledge_gap_signals` pending 计数）**；并渲染此前丢弃的 `gaps[]`（integrity-report 自带的缺口明细，与"知识缺口数"治理计数不同源——前者是 integrity 报告内的缺口列表，后者是 gap_signals pending 数）。
- **边界（已定，消歧义）**：「知识缺口数」权威口径 = `knowledge_gap_signals()` status="pending" 计数（依据既有 ask-human spec `2026-06-21-ask-human-unified-channel-phase1.md:1230`，后端 `list_knowledge_gap_signals` 端点已存在 sources_meta.rs:334）。**不是** `gaps.length`（gaps 是 integrity-report 内的另一字段，仅作明细渲染）。前端从 gap-signals 端点取 pending 计数填该卡。
- **测试**：后端集成测 integrity-report 含 D2 降级字段；前端组件测三计数口径 + gaps 渲染。
- **验收**：需浏览器。

---

## 组三：重后端（2 条，新增端点 + 新语义，blast 最大）

### E11. management 高危指令 requires_confirmation 确认流断流 `[前后端]`
- **现状**：`post_management_message`(management.rs:104) 对 dangerous（plan.requires_confirmation || risk_level=="dangerous"，:190）走 `take(0)`（:192-195）跳过全部 tool，plan 落库（`AgentCommandRun.plan` 全量序列化 ManagementPlan，含 tool_calls: Vec<PlannedToolCall>{tool_name, arguments}），run 标 `pending_confirmation`（:283）。`get_management_command`(:353) 纯只读，**无任何续跑路径**。management 路由仅 4 个（mod.rs:773-785）。
- **修复（后端）**：新增 `POST /management-agent/commands/:id/confirm`：
  1. 沿用 `Extension<AuthenticatedAdmin>` + workspace 隔离（读 run `{_id, workspace_id: &admin.current_workspace}`）。
  2. **CAS 幂等护栏**：先原子把 status 从 `pending_confirmation` 改 `running`（`matched_count==0` 即拒绝 4xx）——防并发双发/重放副作用。
  3. 反序列化 `run.plan` 为 ManagementPlan → 重建 tools（`list_tools_for_account(account_id)` + `merge_product_tools`，:145-147）→ 复用 :192-311 执行循环（**抽成共享 fn**，避免与 post_management_message 重复），改 `take(12)`（真执行）。
  4. 执行后回写终态（succeeded/failed），立即推离 pending。
  5. dry_run 取舍：重查 session.dry_run 沿用原意，不擅自改真跑。
- **修复（前端）**：command-center 对 pending_confirmation 的 command 加"确认执行"按钮 → POST confirm。**二次确认弹窗明确提示"将真实执行 N 个操作（含发送微信/写库）"**（confirm 后续跑会真打生产 MCP + 发送网关，blast 最大）。措辞守无人工接管 lint。
- **边界（安全）**：closed-set status 推进（pending_confirmation→running→succeeded/failed）DB 写点校验；confirm 端点是 batch3 唯一会触发生产副作用的新端点，加固集成测（确认后 tool 真执行 + 幂等拒绝二次 confirm）。
- **测试**：后端集成测（confirm 续跑执行 tool + CAS 拒绝二次 confirm + workspace 隔离）；前端组件测确认按钮 + 二次确认弹窗。
- **验收**：✅需浏览器（高危流，重点验收）。

### F23. 疑似成交待核实闭环 `[前后端]` — 通用化
- **现状**：suspected_deal 只是 `entitlements.rs:302` render_suspected_deal_guidance 产 prompt 引导，让 LLM 在 `agentGeneratedSignals`（AgentSignal: kind="suspected_deal"/value/evidence/confidence）输出弱信号，**沉在 agent_run_logs.decision，无专用 collection、无查询/审核端点**。`VERIFICATION_LABEL.conversation_inferred`（products-deals/index.tsx:61 "疑似成交·待核实"）是档位标签。conversation_inferred 即便落 outcome 也被 G4 投影闭集排除。
- **方案 B（用户拍板，与现有待审信号范式一致）**：
  - **修复（后端）**：
    1. gateway 决策写路径加 `extract_suspected_deal_signal`（仿 `extract_relationship_type_suggestion` gateway.rs:3957）——从 AgentDecision.agent_generated_signals 提取 kind=="suspected_deal" 的信号，upsert 进**新 collection `suspected_deal_signals`**（结构照 RelationshipTypeSuggestion：workspace_id/account_id/contact_id/value/evidence/confidence/occurrences/status/first_seen_at/last_seen_at/reviewed_at/reviewed_by，upsert 锚 (workspace_id, contact_id)）。**fail-soft**：提取失败不阻断主决策（受 RunBudget + 红线约束）。
    2. migration 建 `suspected_deal_signals` collection + 索引（$setOnInsert upsert 幂等，沿用 m024/m028 样板；下一个 migration 编号 writing-plans 阶段核实）。
    3. 新增三端点（仿 admin_relationship_suggestions.rs）：`GET /admin/suspected-deals?status=pending`（workspace 隔离，默认 pending，sort last_seen_at）/ `POST .../:id/approve` / `POST .../:id/reject`。
    4. **approve 落正式成交**：调 `add_outcome_event_inner`（shared.rs:1329）传 `verification="staff_confirmed"` + `source="manual"` + event_kind=deal → $push contact.outcome_events + 审计事件 → mark signal approved。**红线：AI 永不直写 outcome_events，人审 staff_confirmed 才落**。
  - **修复（前端）**：products-deals 成交记录 Tab（或 ask-human 收件箱）加"疑似成交待核实"列表，复用 SimpleApproveReject 模式富展示（依据/置信度/客户/出现次数）→ approve/reject。
- **边界（安全）**：closed-set status（pending/approved/rejected）DB 写点校验；gateway extract 必须 fail-soft；approve 路径与 admin 手动登记成交同一 add_outcome_event_inner（零漂移）。
- **测试**：后端集成测（gateway 提取 upsert 信号 + 三端点 + approve 落 outcome staff_confirmed）；前端 vitest 测待核实列表 + 审核动作。
- **验收**：✅需浏览器（红线相关，重点验收）。

---

## 三、依赖与顺序

- **组一 16 条**互相独立，可任意序。**C6 与 C9 同 task**（C9 是 C6 视图内的字段展示）。A2/A3 同改 userOpsStore + legacy.tsx（建议相邻）。D4/D5 同改 strategyStore + DomainProfilePanel（建议相邻）。E3/E5/E6/E7/E8 同改知识库 steward/shared/ReviewChat（建议相邻）。
- **组二（E9）** 后端补字段 + 前端改口径，独立。
- **组三（E11/F23）** 各自前后端成对，独立，**排最后**（后端语义重，blast 大）。E11 抽共享执行 fn；F23 碰 gateway 决策写路径需 fail-soft。
- **文件重叠提示**：批次1/2 改过的文件（types/index.ts、ask_human_inbox.rs、operations/index.tsx、system-strategy/index.tsx、legacy.tsx、userOpsStore.ts、strategyStore.ts）行号已变，**plan 任务须基于最新 main 重新 grep 实证**。

## 四、不在批次3 范围

- 批次4 的 LOW/INFO 增强项（F1-F22 除 F23）+ 有意缺口（F3/F4/F5/F18 默认不做，实施前用户确认才纳入）。
- D9（domain-schemas 写操作，有意缺口，UI 明示后台维护）、D10（participates_in_decision 复选）、D11（CoverageDimension initial_signal/anchor_hint）——父 spec 归批次4。
- C10（ptier_* 事件 detail 透出，批次4，后端为主）。
- 群运营 / 朋友圈（Phase1 范围外，审查已排除）。

## 五、全局约束（实现期绑定，writing-plans 须逐条带入 plan 的 Global Constraints）

- 子 agent 一律 `model:"opus"`；回复中文。
- 无人工接管 CI lint：`src/agent|routes|evolution` + `frontend/src` 新增行（含注释/JSX 文案）禁 `人工/接管/takeover/hand-off/人工接管/转人工/人工介入/人工托管`（测试目录除外）。**E11 二次确认文案、E2 引荐态文案、F23 待核实文案用业务语义措辞，避开禁词**（用 "AI 已退辅助答疑" / "确认执行 N 个操作" / "疑似成交待核实"）。
- 测试基线不回退：`cargo test --lib ≥350/0`、4 PBT ≥33/0、`RUSTFLAGS=-Dwarnings cargo check --tests` 0/0。本地只跑 `cargo test --lib` + 单 PBT，集成测留 CI。
- 测试只增量叠加，不删改旧维度/旧弧/旧金标。
- **AI 永不自动验证知识红线**：E7 手工新建切片保持 status=draft + integrity_status=needs_review；F23 落正式成交一律人审 + `verification="staff_confirmed"`，AI 永不直写 outcome_events。
- **closed-set 枚举 DB 写点校验**：E11 run status（pending_confirmation→running→succeeded/failed）、F23 signal status（pending/approved/rejected）。
- **migration 用 $setOnInsert upsert 幂等**（F23 建 suspected_deal_signals collection + 索引，沿用 m024/m028 样板，下一个编号 writing-plans 阶段核实）。
- **后端改动 blast radius**：E9 补 integrity-report 字段须保 CockpitView 现有消费不回归；E11 confirm 端点会触发生产 MCP + 发送网关（加固测试 + CAS 幂等）；F23 gateway extract 必须 fail-soft（不阻断主决策）。
- 前端遵守现有设计系统：tokens.css 变量、`.module.css`、4 级层级、蓝=主操作专属、紫=AI 身份专属（见 `docs/frontend-design-system.md`）。**D4 复用 ActiveVersionsBar、F23 复用 SimpleApproveReject、E16 复用 C1 错误态模式**——优先复用现有组件不造新。
- git：仅在用户要求时提交；只 `git add` 具名文件，绝不 `git add -A`；commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`；破坏性 gitops 须显式授权。

## 六、不变量（修复全程守住）

- 后端 integrity-report / 新端点加字段必须向后兼容，不破现有消费方。
- F23 gateway extract fail-soft：提取/upsert 失败不阻断主决策链（受 RunBudget 约束）。
- E11 confirm 幂等：CAS status 转移防并发双发，confirm 后立即推离 pending 防重放。
- 新建知识切片/文档保持 status=draft + needs_review；F23 落 outcome 一律 staff_confirmed（AI 永不自动验证 / 永不直写成交）。
- 所有新增前端文案不含禁词（CI 门）。
- 前端新组件遵守设计系统，优先复用现有组件（ActiveVersionsBar / SimpleApproveReject / 错误态模式）。

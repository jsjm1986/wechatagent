# 前后端业务对齐缺口修复 全量设计（76 条）

- 日期：2026-06-26
- 来源：前后端业务对齐全面审查 workflow `wf_30294a0e-19c`（24 后端域 + 10 业务 spec + 6 横切维度 + 独立交叉验证主动证伪，并发 2）。241 原始信号 → 76 确认缺口，REFUTED=0 / NEEDS_HUMAN=0 / 失败 agent=0。审查脚本 `.kiro/specs/universal-test-coverage/frontend-backend-alignment-audit-workflow.mjs`。
- 关联 memory：`project_frontend_backend_alignment_audit_2026_06_26`、`project_universalization_residuals`（#2 前端 labelFor 命门印证）、`frontend_follow_design_system`（前端必须遵守现有设计系统）。
- 状态：设计稿，待用户审。本 spec 是**全量路线图**——76 条审查结论全部给修复方案，按 5 业务域 × 4 优先级批次组织。审查产物 74 CONFIRMED + 2 PARTIAL = 76 条，其中 4 组为跨维度重复命中（runtime-flag/threshold-audit/reassign/产品编辑），合并后为 **67 个去重的可执行条目**（编号 A/B/C/D/E/F）。批次 1（P0+P1）详设可直接走 plan；后续批次实现前各自再用本 spec 对应条目展开 plan。

## 背景与问题

WechatAgent 后端约 50 个资源域、200+ REST 端点，但前端（16 feature / 15 store / 143 文件）对后端能力的兑现度偏低。审查确认两个系统性反模式：

1. **"只读不可写"**：大量后端写端点（编辑/回滚/创建/改派/灰度/审计）在 UI 上无入口，运营只能看不能改。
2. **"错误态静默吞成空态"**：加载失败显示"暂无数据"，管理员误判成"Agent 无活动"。

外加**通用化承诺在前端断裂**（字典 flag/conversationMode 标签/profile 高级字段写死或无入口，"改字典即通用"在 UI 走不通）。

## 全局约束（每个修复任务都隐含遵守）

- **前端遵守现有设计系统**（`frontend_follow_design_system` memory + `docs/frontend-design-system.md`）：真实 token 在 `components/ui/tokens.css`；CSS 用 `.module.css` + import 绑定（避 tree-shake 坑，见 `frontend_css_module_tree_shake_trap`）；4 级层级 / 蓝仅主操作 / 紫仅 AI 身份 / 字号纪律。**写前端前先读 doc + 参照现有 .module.css**。
- **无人工接管红线**（CLAUDE.md + `scripts/check-no-human-takeover.{sh,ps1}` CI 门）：新增前端/后端代码新行禁含 `人工接管/转人工/takeover/hand-off/接管/人工`，只用 AI 内部状态名。
- **测试基线门**（`scripts/check-baseline.{sh,ps1}`）：`cargo test --lib` ≥350/0；4 PBT 累计 ≥33/0；`RUSTFLAGS=-Dwarnings cargo check --tests` 0 err 0 warn。前端 `npm run build` + vitest 三连。
- **AI 永不自动验证知识**：新增知识 status=draft + integrity_status=needs_review 红线不破。
- **测试策略**：自动化测（前端 vitest store/组件、后端 cargo 集成测）+ **用户人肉验收 UI**。每条标注是否需浏览器验收。
- **提交纪律**：只 `git add` 指定文件，commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- **后端改动 blast radius**：扩 InboxItem 投影、加 $unset 端点等动后端的条目，须保 InboxItem 现有消费方（knowledgeReview/taxonomyCandidate 等）不回归。

## 优先级与批次

| 批次 | 优先级 | 条数 | 内容 | 实施时机 |
|---|---|---|---|---|
| **批次 1** | P0+P1 | 14 | 1 critical + 13 high；功能性中断 + 核心能力 UI 不可达 | 本轮详设 → 直接 plan |
| **批次 2** | P2-通用化 | ~8 | 通用化前端断裂（字典 flag/conversationMode/profile 高级字段/per_relationship/画像维度） | 批次 1 后展开 plan |
| **批次 3** | P2-其余 | ~16 | 其余 MEDIUM（编辑入口/解除关联/proposal 详情/审核盲批等） | 批次 2 后 |
| **批次 4** | P3 | ~28 | LOW/INFO 增强项 + 有意缺口（标注 verifier 取舍判定，实施前再决策） | 末批，按需 |

下面**按业务域分组**列全部 76 条，每条标 `[批次N]` `[前/后/前后]`。批次 1 给详细修复方案（file:line + 改法 + 测试），批次 2-4 给方案要点（实施前展开）。

---

## 域一：用户运营驾驶舱（user-ops）

### A1. loadMessages 双死端点 `[批次1][前端]` — CRITICAL
- **缺口**：`userOpsStore.ts:283-309` 在一个 `Promise.all` 里并发 5 个请求，其中 `:292 /api/contacts/${id}/messages` 与 `:295 /api/contacts/${id}/decision-reviews` 两端点后端不存在。`api.get` 对非 2xx throw，`Promise.all` 全有或全无 → catch(:306-308) 只 setError，五面板（对话/运营记忆/记忆候选/决策复盘/运营健康）的 set 永不执行，全空。
- **后端真实路由**：消息 `routes/mod.rs:371 /conversations/:contact_id/messages`；决策复盘 `mod.rs:690 /decision-reviews`（query 过滤）。
- **修复**：`userOpsStore.ts:292` 改 `/api/conversations/${contact.id}/messages?limit=50`；`:295` 改 `/api/decision-reviews?contactId=${contact.id}&limit=20`（参考 `operationsStore.ts:32` 正确写法）。
- **测试**：vitest store 测——mock 两正确端点返回数据，断言 5 面板 state 全部 set 成功；mock 任一失败，断言降级行为（见下方加固）。
- **加固（顺带，非必须但建议）**：把 `Promise.all` 改 `Promise.allSettled`，单面板失败不拖垮其余四面板（呼应反模式"全有或全无"）。
- **验收**：✅需浏览器——起 dev server，选中联系人，确认五面板加载。

### A2. operating-memory PUT 手动编辑零接入 `[批次3][前端]`
- **缺口**：`userOpsStore.ts:293` 仅 GET operating-memory；memoryDraft 纯只读（legacy.tsx:255/259/263/267/280/288 渲染进 strong/p），无 input/onChange/提交，`setMemoryDraft` 全仓 0 命中。后端 `OperatingMemoryRequest`(contacts.rs:47) 四字段 user_understanding/relationship_state/product_fit/next_action 无前端写表单。
- **方案**：memoryDraft 区改可编辑表单 + 保存按钮 → PUT `/api/contacts/:id/operating-memory`；保存后回填。
- **测试**：vitest store 测 saveOperatingMemory 发 PUT；组件测编辑→提交。验收：需浏览器。

### A3. operation-profile PUT 7 字段仅 relationshipType 接通 `[批次3][前端]`
- **缺口**：`saveRelationshipType`(userOpsStore.ts:479-499) PUT body 仅 relationshipType；后端 OperationProfileRequest(contacts.rs:31-42) 另 6 字段（tags 走独立 manual-tags 路由除外）中 last_commitment/follow_up_policy 无人工编辑入口（customer_stage/intent_level 刻意只读 AI 派生，不动）。
- **方案**：补 last_commitment/follow_up_policy 编辑入口 → 并入 operation-profile PUT。customer_stage/intent_level 维持只读。
- **测试**：vitest store 测扩字段提交。验收：需浏览器。

### A4. 行业化画像维度看板未渲染 `[批次2][前端]` — 通用化
- **缺口**：`profileStore.ts:69` loadActiveView 存 `store.dimensions`（ProfileDimensionView[]）后 0 处消费（死字段）；`legacy.tsx:2018-2085` PlannerViewSection 只 labelFor 显示 customer_stage 单维，intent_level/value_tier/churn_reason/purchase_lifecycle 无渲染。
- **方案**：PlannerViewSection 消费 store.dimensions，按 active profile 声明的维度动态渲染（呼应通用化"维度由 profile 驱动"）。
- **测试**：vitest 组件测多维度渲染。验收：需浏览器。

### A5. conversationMode 标签行业化 `[批次2][前端]` — 通用化
- **缺口**：`legacy.tsx:2167-2180` conversationModeLabel switch 写死 4 销售域 case（casual_relationship/value_exchange/consultative/boundary_protection），default 回落显示原始英文 key。情感/陪伴域声明的 intimate_companion 等无中文标签。
- **方案**：标签从 active profile 的 conversation_modes 声明取（label 字段），而非前端写死 switch。呼应 `project_universalization_residuals` #2 labelFor 命门。
- **测试**：vitest 组件测非销售域模式显示中文。验收：需浏览器。

---

## 域二：请示决策通道（ask-human / principal-channel）

### B1. 请示裁决只暴露 2/5 verdict + 条件授权窗无录入 `[批次1][前端]` — HIGH
- **缺口**：`EscalationInline.tsx:10` resolve 形参硬限 `"approved"|"rejected"`，`:16` constraints 硬编码 `[]`，`:17` authorizationWindowHours 硬编码 `null`，UI 仅批准/驳回两按钮。后端 ALLOWED_PRINCIPAL_VERDICT 5 值闭集（approved/rejected/conditional/deferred/delegated_back）+ conditional+authorizationWindowHours→authorization_expires_at 授权过期语义前端不可达。
- **修复**：EscalationInline 扩 5 种 verdict 选择 + conditional 时显示 constraints（约束条款）录入 + authorizationWindowHours（授权窗小时数）输入。POST body 传真实值而非硬编码。
- **测试**：vitest 组件测——选 conditional 显示授权窗输入；提交 body 含 constraints/authorizationWindowHours。验收：✅需浏览器。

### B2. 请示卡聚合投影压扁 `[批次1][前后端]` — HIGH
- **缺口**：后端 `ask_human_inbox.rs:45-59`（collect_escalations / list_escalations_by_workspace 映射）title 固定 `请示 #{short_code}`、summary 只放 reason，category/question_for_principal/contact_wxid/principal_wxid 全丢；InboxItem 结构本身无这些字段。前端 `inboxApi.ts:3-14` InboxItem 接口无这些字段；EscalationInline 仅渲染 title/summary → 决策人盲裁。
- **修复（后端）**：InboxItem 加可选富字段（或用既有 rich_params doc 承载）category/questionForPrincipal/contactWxid/principalWxid；collect_escalations 填充。**保 InboxItem 其余消费方（knowledgeReview/taxonomyCandidate/relationshipSuggestion）不回归**——加可选字段不破现有。
- **修复（前端）**：inboxApi.ts InboxItem 接口加字段；EscalationInline 富展示（客户是谁/属哪类/具体问什么）。
- **测试**：后端集成测 collect_escalations 投影含新字段；前端 vitest 组件测富展示。验收：✅需浏览器。

### B3. 请示改派 reassign 无前端入口 `[批次1][前端]` — HIGH
- **缺口**：后端 `POST /admin/principal-escalations/:short_code/reassign` 存在；全前端 grep reassign/改派/toWxid 0 命中。EscalationInline 仅 resolve 一个动作。决策人链超时无法转备选。
- **修复**：EscalationInline 加"改派"动作 + 备选决策人 wxid 录入 → POST reassign。
- **测试**：vitest 组件测改派提交。验收：需浏览器。

### B4. 请示已裁决记录 status=resolved 历史/裁决/授权过期无展示 `[批次3][前端]`
- **缺口**：聚合收件箱 `ask_human_inbox.rs:44` 写死 `"pending"`；后端 `list_principal_escalations`(principal_escalations.rs:25-58) 支持 status=resolved 投影 decision/authorizationExpiresAt/resolvedVia，前端无消费。仅 steward.tsx:2006-2014 phase-rollup 有 resolved 聚合计数。
- **方案**：ask-human 频道加"已裁决历史"视图（或筛选器），调 `/admin/principal-escalations?status=resolved` 展示裁决结果/授权到期/裁决渠道。
- **测试**：vitest store/组件测。验收：需浏览器。

---

## 域三：自治可观测性（autonomy / evolution / operations）

### C1. Operations 私聊域 4 端点齐故障静默成空态 `[批次1][前端]` — HIGH
- **缺口**：`operationsStore.ts:42-51` catch 仅 console.error 后置空 events/tasks/decisionReviews/llmUsage，无 setError/ErrorBanner；index.tsx 各 tab 走 EmptyState。Promise.all 任一 reject → 全空 → 管理员误判"Agent 无活动"。
- **修复**：catch 接 `useUiStore.setError` + 全局错误横幅；区分空态（成功但无数据）vs 错误态（加载失败）。考虑 Promise.allSettled 让单端点失败不拖垮其余。
- **测试**：vitest store 测——mock 端点失败，断言 setError 被调用、不静默置空。验收：需浏览器。

### C2. outbox 发件箱逐条记录 + 取消无入口 `[批次1][前端]` — HIGH
- **缺口**：后端 `admin_outbox.rs` GET /admin/outbox + POST /admin/outbox/:id/cancel；前端仅 `autonomy/index.tsx:61-69` 展示 /outcomes/autonomy 聚合比率，无逐条记录、无取消按钮。outbox 是 approved 决策发送链路真相源（CLAUDE.md 硬规则）。
- **修复**：autonomy 频道（或新子视图）加 outbox 逐条列表（GET /admin/outbox）+ 每条取消按钮（POST cancel）。
- **测试**：vitest store/组件测列表+取消。验收：✅需浏览器。

### C3. 演化运行时灰度开关 runtime-flag + rollout 比例无 UI `[批次1][前端]` — HIGH
- **缺口**：后端 `evolution.rs:561-583` GET/PUT /api/evolution/runtime-flag（workspace 级 enabled + rolloutPercent 0-100 upsert）；前端仅 `evolution/index.tsx:13` 经 /api/health 读 env 级布尔。EvolutionCenterTab:144 是静态文案。只能改 env 全开/全关。
- **修复**：演化中心加 runtime-flag 控件——enabled 开关 + rolloutPercent 滑块/输入（0-100），GET 读 + PUT 写。
- **测试**：vitest 组件测读写。验收：✅需浏览器。

### C4. 阈值变更不可变审计 threshold-overrides/audit 无 UI `[批次1][前端]` — HIGH
- **缺口**：后端 `GET /api/evolution/threshold-overrides/audit`（release/rollback/auto-release 历史）；全前端 grep 0 命中。自演化合规追溯缺失。
- **修复**：演化中心加审计日志视图，调该端点展示 release/rollback/auto-release 历史行。
- **测试**：vitest 组件测列表渲染。验收：需浏览器。

### C5. 跟进任务操作（立即复核/取消）纯只读 `[批次1][前端]` — HIGH
- **缺口**：`operations/index.tsx:116-138` 跟进任务 tab 仅三只读列，tbody 无 button/onClick。后端 review-now / cancel 端点存在（`/agent-tasks/:id/review-now`、`/cancel`）。
- **修复**：跟进任务行加"立即复核""取消"操作按钮 → POST 对应端点。
- **测试**：vitest 组件测操作按钮。验收：需浏览器。

### C6. Agent 运行日志（run envelope）无消费 `[批次3][前端]`
- **缺口**：全前端 grep agent-runs/runEnvelope 0 命中；operationsStore.ts:29-34 只拉 events/tasks/decision-reviews/llm-usage，无 /agent-runs。GET /agent-runs 端点无前端入口，单次运行包络（决策/复核/送达全链）不可见。
- **方案**：operations 或 autonomy 加运行日志视图，调 /agent-runs 展示 run envelope。
- **测试**：vitest store/组件测。验收：需浏览器。

### C7. autonomy 逐行 finalReviewStatus/holdCategory 裸英文枚举 `[批次2][前端]`
- **缺口**：`autonomy/index.tsx:359-360` 直接渲染 blocked_unverified_product_claim 等英文闭集值，无中文 label map（HoldBar 聚合层 :194-196 有标签，逐行明细裸露）。
- **方案**：加 FINAL_REVIEW_STATUS / HOLD_CATEGORY 中文 label map（闭集，10 值 + 3 值），逐行用 map。
- **测试**：vitest 组件测中文标签。验收：需浏览器。

### C8. DecisionReview 拦截原因坍缩为二元 `[批次2][前后端]`
- **缺口**：`operations:186`/`legacy:563` 仅 `approved?通过:拦截`；types DecisionReview(types/index.ts:285-299) 缺 finalReviewStatus/holdCategory，无法区分 safety_guard/unverified_product/required_field/budget 四分支。后端 shared.rs decision_review_json 输出这些字段但前端类型没建模。
- **方案（前端为主）**：types DecisionReview 加 finalReviewStatus/holdCategory；展示用 C7 的 label map 区分四种拦截分支。**核实后端 decision_review_json 是否已 emit 这两字段**——若没则补后端投影。
- **测试**：vitest 组件测四分支显示。验收：需浏览器。

### C9. run log tier 遥测无消费点 `[批次2][前端]` — 通用化(渐进式三档)
- **缺口**：tier_used/sufficiency/escalated/forced_full 全前端 0 命中；写入 AgentRunLog.gateway_result 的遥测前端零呈现，账号级灰度/A-B 验证无数据面。
- **方案**：依赖 C6（run envelope 视图）落地后，在其中展示 tier 遥测字段。
- **测试**：随 C6。验收：需浏览器。

### C10. ptier_* 遥测事件结构化 detail `[批次4][后端为主]`
- **缺口**：后端 `events.rs:57-64` list_events 返回 json 不含 detail doc；前端 operations 事件 feed 只渲染 kind/summary，detail 里 run_id/knowledge_coverage 等无处读。
- **方案**：后端 list_events 可选透出 detail（注意 payload 体积）；前端事件详情展开。**根因在后端**，前端无法独立补。
- **测试**：后端集成测 + 前端组件测。验收：需浏览器。

---

## 域四：配置自助化（accounts / evaluation / domain-profiles / schemas / taxonomy）

### D1. 评测场景 CRUD evaluation-scenarios 无入口 `[批次1][前端]` — HIGH
- **缺口**：后端 `mod.rs:703-708` GET/POST/PUT/DELETE evaluation-scenarios；非测试代码 0 命中。`quality:276-280` 文案明示 formula-adherence 依赖 active evaluation_scenarios，却无创建/编辑入口 → 评测能力无法自助运营。
- **修复**：quality 频道（或新子页）加评测场景列表 + 新建/编辑/删除表单（ground_truth/inboundText/expectedBehavior 等字段）。
- **测试**：vitest store/组件测 CRUD。验收：✅需浏览器。

### D2. 账号 MCP 密钥配置无表单 `[批次1][前端]` — HIGH
- **缺口**：后端 `mod.rs:312 PUT /accounts/:id/mcp-key`；前端仅 `types/index.ts:37 mcpKeyConfigured` 只读布尔提示，accountStore 纯本地状态，无写回表单。新账号接入只能改库。
- **修复**：账号管理处加 MCP 密钥配置表单 → PUT /accounts/:id/mcp-key。**密钥是敏感值**：输入框用 password 型，不回显已存值（仅显示"已配置"布尔）。
- **测试**：vitest 组件测提交（不断言明文密钥）。验收：✅需浏览器。

### D3. AI 生成状态机本体 generated_state_machine 无人审展示 `[批次1][前端]` — HIGH
- **缺口**：后端 `guide_profile.rs` 落 draft、`domain_profiles.rs` activate 时 validate_state_machine publish；grep generatedStateMachine 0 命中；ProfilePublishCard 仅渲染 display_name+状态，不展示 states/goal/advanceSignals/riskRules。AI 生成状态机走 draft+人审红线，但激活前无界面审阅。
- **修复**：ProfilePublishCard（或详情）展示 generated_state_machine 的 states/goal/advanceSignals/riskRules，供管理员激活前审阅。
- **测试**：vitest 组件测状态机内容渲染。验收：✅需浏览器（人审红线相关，重点验收）。

### D4. domain-profiles 版本回滚 rollback 无 UI `[批次3][前端]`
- **缺口**：后端 rollback 端点存在、版本链(previous_version)已展示，但 DomainProfilePanel(index.tsx:2007) 未挂 ActiveVersionsBar，无回滚按钮。误发布的行业配置无法一键回退。
- **方案**：DomainProfilePanel 挂 ActiveVersionsBar（复用现有组件），endpointPrefix 指 /api/admin/domain-profiles。
- **测试**：vitest 组件测回滚动作。验收：需浏览器。

### D5. domain-profiles 手动新建空白配置链路死 `[批次3][前端]`
- **缺口**：`newDomainProfileDraft()`(strategyStore.ts:332-352) 置 editingProfile=null，但编辑区 `editing?<Editor>:<placeholder>` 在 null 时只渲染占位，onSave 在 null 时 no-op，永不 POST。手动建配置只能走 AI 生成。
- **方案**：修复新建链路——newDraft 时进入可编辑空白态，saveDomainProfile 支持 POST（无 id 时 create）。
- **测试**：vitest store 测新建走 POST。验收：需浏览器。

### D6. 字典 is_reactivation_target/is_terminal 无配置入口 `[批次2][前后端]` — 通用化
- **缺口**：后端 `admin_taxonomies.rs:152` create handler 硬编码 is_terminal:false / is_reactivation_target:false；patch_taxonomy(:196-214) set_doc 白名单不含两 flag。前端 TaxonomiesAdmin 表单（system-strategy/index.tsx createDraft :609-616 / submitCreate :643-679 / submitEdit :691-695）无这两字段。spec 承诺"改字典即通用"在 UI 走不通，非销售域无法启用再激活。
- **修复（后端）**：create handler 接收两 flag；patch_taxonomy set_doc 白名单加两 flag。
- **修复（前端）**：TaxonomiesAdmin create/edit 表单加 is_reactivation_target / is_terminal 复选。
- **测试**：后端集成测 create/patch 落两 flag；前端 vitest 测表单提交。验收：需浏览器。

### D7. profile 高级字段无编辑 UI `[批次2][前端]` — 通用化
- **缺口**：transaction_facts_enabled/reviewer_orientation/mode_gate_policy_override/trajectory_dimensions/debounce_window_ms_override 五字段在后端 DomainProfile(models.rs)，前端 types(index.ts:589-645)与 ProfileEditor 均未声明/覆盖。交易型域只能靠 seed。
- **方案**：types DomainProfile/Draft 加五字段；ProfileEditor 折叠面板加编辑入口。**reviewer_orientation/mode_gate_policy/transaction_facts 属 publish 危险字段**，编辑面加确认/说明。
- **测试**：vitest 组件测字段编辑。验收：需浏览器。

### D8. per_relationship_operation_mode 无入口 `[批次2][前端]` — 通用化(数字分身)
- **缺口**：后端 `models.rs:1763` per_relationship_operation_mode: Option<BTreeMap<String,OperationMode>>；前端只编辑 profile 级单个 operation_mode（system-strategy/index.tsx:1893-1947），无按 relationship_type 键分别配置的 map 编辑入口。三级回落链 contact.override ?? per_relationship[rt] ?? operation_mode 中间一级无配置面。
- **方案**：ProfileEditor 加 per_relationship_operation_mode map 编辑（按 relationship_type 键配 OperationMode）。
- **测试**：vitest 组件测 map 编辑。验收：需浏览器。

### D9. domain-schemas CRUD 写操作无入口 `[批次4][前端]` — ⚠️有意缺口
- **缺口**：后端 POST/PUT/DELETE domain-schemas 存在；前端仅 load+activate，UI 文案明示"字段表由系统管理员维护…不能直接改内容"。
- **verifier 判定**：**可解释的有意缺口**（UI 明示后台维护）。
- **方案（若做）**：atlas.tsx DomainSchemaTab 加 create/edit/delete 表单。**实施前再决策**——当前是有意设计，做了要改 UI 文案承诺。
- 验收：需浏览器。

### D10. ProfileDimension.participates_in_decision 无 checkbox `[批次4][前端]` — 通用化
- **缺口**：维度编辑器 system-strategy/index.tsx:1339-1397 每行只 kind/display_name/description，+添加硬编码 participates_in_decision:true(:1391)，无法建"只观测不进决策"维度。
- **方案**：维度编辑行加 participates_in_decision 复选。
- **测试**：vitest 组件测。验收：需浏览器。

### D11. CoverageDimension.initial_signal/anchor_hint 无编辑 `[批次4][前端]` — 通用化
- **缺口**：后端 CoverageDimension(models.rs:2230-2252) 含 anchor_hint/initial_signal；前端 types(index.ts:525-530) 缺 initial_signal，编辑器(:1602-1647) 只 key/display_name/required。前端新建 completeness 维度 degraded 审计恒 missing。DEFAULT_PROFILE 后端 seed 不受影响。
- **方案**：types CoverageDimension 加 initial_signal；编辑器加 anchor_hint/initial_signal 输入。
- **测试**：vitest 组件测。验收：需浏览器。

---

## 域五：知识库 + 红线语义（knowledge / referral / management）

### E1. referral「已引荐」态不可撤销 `[批次1][前后端]` — HIGH
- **缺口**：后端 referred_specialist_at/referred_card_id 只在 `referral.rs:79-86` $set 写入（referral.rs:171），escalation/logic.rs:313 只要存在该键即恒注入退辅助指引；全 routes 无 $unset/clear 端点；update_assist_override(contacts.rs:562-582) 不触及这两字段。前端 grep clearReferr/revoke/撤销引荐 0 命中（referral-cards:76 onRevoke 改的是名片库审核态，与 per-contact 标记无关）。设计 §6.3 红线承诺态可撤销，当前一旦引荐永久锁定被动答疑。
- **修复（后端）**：加端点 `POST /contacts/:id/clear-referral`（或并入 assist-override），$unset referred_specialist_at + referred_card_id，让客户回主动运营态。
- **修复（前端）**：联系人详情加"撤销引荐/恢复主动运营"动作 → 调该端点。
- **测试**：后端集成测 $unset 后 escalation 不再注入退辅助指引；前端 vitest 测撤销动作。验收：✅需浏览器（红线相关，重点）。

### E2. referral「已引荐」态状态可观测 `[批次3][前端]`
- **缺口**：hydrateSelected(userOpsStore.ts:265-280) 只读 assist_mode_override/relationship_type，不读 referred_specialist_at/referred_card_id。联系人详情面板无"已引荐/AI 已退辅助"显式指示（对话流 namecardBubble 有间接观测）。
- **方案**：hydrateSelected 读 referred 标记；详情面板显式显示"已引荐态"。与 E1 同源。
- **测试**：vitest store/组件测。验收：需浏览器。

### E3. chunk AI 修复 propose/answer 无入口 `[批次3][前端]`
- **缺口**：后端 `mod.rs:484 POST /chunks/:id/repair` + `:488 /repair/answer`（repair.rs:1-9 注释明示应有 applyAiRepairPatch 落账闭环）；前端 grep 0 命中，today.tsx:509 "去修复"按钮仅 focus 跳转。有 ReviewChat 会话级 /chat 替代路径，故非彻底不可达。
- **方案**：chunk 详情加结构化 AI 修复面板（propose patch → 显示 patch → AI 追问 answer → 接受落库）。
- **测试**：vitest 组件测修复流。验收：需浏览器。

### E4. AI pack 修复 propose→apply→落账上报无入口 `[批次4][前端]`
- **缺口**：后端 `mod.rs:631 items/:id/repair` + `:635 repair/applied`（写 AgentEvent kind=knowledge_repair_applied）；前端无调用（propose 入口本身缺失）。
- **方案**：依赖 E3 思路，pack 级修复同链补 UI。**纯审计旁路，影响低**。
- 验收：需浏览器。

### E5. 解除知识关联 unrelate 无 UI `[批次3][前端]`
- **缺口**：后端 `mod.rs:524-526 DELETE /chunks/:id/relate/:target_id`；前端建立关联有 UI（shared.tsx:794-819），related_chunks 反向引用纯只读（shared.tsx:359-376），无解除按钮。
- **方案**：related_chunks 列表项加"解除关联"按钮 → DELETE relate/:target_id。
- **测试**：vitest 组件测解除。验收：需浏览器。

### E6. 文档元数据编辑 PUT documents/:id 无入口 `[批次3][前端]`
- **缺口**：后端 `mod.rs:454 PUT /operation-knowledge/documents/:id`（crud.rs:108 replace_one 整文档替换）；前端 steward.tsx 只增/删/查切片，改文档只能删了重建。
- **方案**：steward.tsx 文档项加编辑表单 → PUT documents/:id。
- **测试**：vitest 组件测编辑。验收：需浏览器。

### E7. 手工单条新建切片 POST chunks 无入口 `[批次3][前端]`
- **缺口**：后端 `mod.rs:463 POST /operation-knowledge/chunks`（crud.rs:192）；前端切片只经 import pipeline 产出，无手工单条新建表单。
- **方案**：steward.tsx 加"手工新建切片"表单 → POST chunks（注意 status=draft + needs_review 红线）。
- **测试**：vitest 组件测新建（断言 draft 态）。验收：需浏览器。

### E8. ReviewChat 对话产 patch 后左栏无实时预览 `[批次3][前端]`
- **缺口**：`ReviewChat.tsx:149` 仅取 turn.patch 为 boolean，patch 内容被弃用；左栏静态 prop chunk 不刷新，需放行后整列表 reload 才见改动。
- **方案**：ReviewChat 收到 turn.patch 后渲染 patch diff 预览 +（可选）实时刷新左栏。
- **测试**：vitest 组件测 patch 预览。验收：需浏览器。

### E9. 治理待办三计数错配 `[批次3][前后端]`
- **缺口**：CockpitView.tsx:78-95 三 MetricCard 实为待审草稿/需复核/知识总数，与 spec 4.1 的 待审草稿数/D2降级数/知识缺口数 不符；gaps[] 解析后(trustTypes.ts:109)被丢弃；integrity-report 解析层(trustTypes.ts:122-131)无 D2 降级来源字段。
- **方案（前后端）**：核实 integrity-report 是否含 D2 降级计数——无则补后端字段；前端 MetricCard 改为 spec 4.1 三计数，渲染 gaps。
- **测试**：后端集成测 + 前端组件测。验收：需浏览器。

### E10. relationship_type LLM 识别建议审核盲批 `[批次2/3][前后端]` — 通用化(数字分身)
- **缺口**：后端 `ask_human_inbox.rs:153-166 collect_relationship_suggestions` 只塞 suggested_value（title/summary 都是它），rich_component/rich_params=None，evidence/confidence/contact_id/occurrences 全不下发。前端 SimpleApproveReject.tsx 仅渲染 title/summary → 盲批改写 contact.relationship_type。
- **修复（后端）**：collect_relationship_suggestions 投影 evidence/confidence/contact_id/occurrences（同 B2 InboxItem 富字段思路）。
- **修复（前端）**：SimpleApproveReject 或专用组件富展示 AI 判断依据/置信度/客户身份。
- **测试**：后端集成测投影；前端 vitest 测富展示。验收：需浏览器。

### E11. management 高危指令 requires_confirmation 确认流断流 `[批次3][前后端]`
- **缺口**：后端 management.rs:189-195 dangerous 时 take(0) 不执行落 pending_confirmation(:281-283)，但 routes 无 confirm/resume/execute 端点；前端 command-center 无确认按钮。dangerous 指令卡死=fail-safe 方向。
- **方案（前后端）**：后端加 confirm/resume 端点续跑 pending_confirmation 的 run；前端 command-center 加二次确认按钮。**端到端断流，后端也缺端点**。
- **测试**：后端集成测续跑；前端组件测确认。验收：需浏览器。

### E12. evolution proposal 详情 5 字段未渲染 `[批次3][前端]`
- **缺口**：riskNote/diffSummary/evalMetrics/cohortRunIds/previousPromptVersion 在 proposalTypes.ts:95-117 有类型、test 有 mock，但 ProposalReleaseCard.tsx 零渲染。运营看不到风险提示/diff/评测/同批 run/前版本。
- **方案**：ProposalReleaseCard 渲染这 5 字段。
- **测试**：vitest 组件测。验收：需浏览器。

### E13. reviewer 隐私边界维度 boundaryPrivacySafety 无法显形 `[批次2][前端]` — 通用化
- **缺口**：`operations/index.tsx:41-49 formatScores()` 写死 5-key 白名单 + undefined 过滤，新维度 boundaryPrivacySafety（渐进式三档加固加的隐私维度）被静默丢弃。
- **方案**：formatScores 改为动态遍历 scores 所有 key（或扩白名单含 boundaryPrivacySafety），加中文 label。
- **测试**：vitest 组件测新维度显示。验收：需浏览器。

### E14. 知识长任务派工创建 dispatch 无入口 `[批次1][前端]` — HIGH
- **缺口**：后端 `mod.rs:665 POST /knowledge/chat/tasks`（chat_task_create，需 plannedSteps + cardIds，KnowledgeTaskWorker 串行执行）；前端 plannedSteps/cardIds 0 命中，ChatWorkbench 只 POST 单轮 /chat。长任务队列 UI 不可达，跟踪还要手工粘贴 taskId（today.tsx:705-798 TaskRail 靠粘贴任务编号）。
- **修复**：ChatWorkbench（或知识频道）加长任务派工入口——选 cardIds + 编 plannedSteps → POST /knowledge/chat/tasks；创建后自动联动 TaskRail 跟踪（消除手工粘贴 taskId）。
- **测试**：vitest store/组件测派工创建 + 联动跟踪。验收：✅需浏览器。

### E15. 多 workspace 切换入口 UI 不可达 `[批次3][前端]`
- **缺口**：后端 POST /api/auth/workspace + /auth/me workspaces[] 完整，handler 已在 authStore/main.tsx 接好，但无 UI 触发，Shell:187 仅把 currentWorkspace 当纯文本显示。多 workspace 用户无法切换。
- **方案**：Shell 侧栏把 workspace 文本改为下拉/切换器（调已接好的 onSwitchWorkspace handler）。
- **测试**：vitest 组件测切换。验收：需浏览器。

### E16. 联系人选择器 + 决策链编辑器静默吞错 `[批次1/3][前端]`
- **缺口**：products-deals ContactPicker(index.tsx:353-362) 与 ask-human-config DeciderChainEditor(:21-31) 的 catch 只 setContacts([])，无 setError；加载失败显示空可选列表，无法区分真无联系人 vs 失败。影响名片引荐/决策链配置。
- **方案**：catch 接 setError + 错误提示（同 C1 错误态模式）。归批次 1 错误态簇或批次 3。
- **测试**：vitest 组件测错误态。验收：需浏览器。

---

## 域六：LOW/INFO 增强项与有意缺口（批次 4，多数标注取舍）

以下条目 verifier 多判为"有缓解/有意取舍/增强项"，列全保完整决策依据，实施前逐条再决策：

- **F1. memory-card 专用端点未调用** `[前]` — 数据已经 operating-memory 透出，仅状态机 initial 态回落丢失。**低优**。
- **F2. GET documents/:id 单文档详情** `[前]` — 列表+/chunks 已覆盖，**冗余端点**。
- **F3. PUT chunks/:id 整体替换** `[前]` — ⚠️patch 局部改写+revisions/rollback 已覆盖，**有意取舍**，建议不做。
- **F4. DELETE chunks/:id 硬删** `[前]` — ⚠️archive/restore 软删闭环存在，知识审计不暴露硬删更安全，**有意取舍**，建议不做。
- **F5. completeness POST refresh** `[前]` — ⚠️与 GET 逐行等价无副作用，GET 已够，**info 级冗余**，建议不做。
- **F6. gap-digest /usage 原始用量列表** `[前]` — per-chunk usageStats + logs/analyze 已覆盖，**低价值**。
- **F7. operation-state-policies GET :id** `[前]` — 列表 payload 自足，**非必需**。
- **F8. decision-reviews/:id 详情 + 诊断快照字段** `[前]` — 列表已给概览，诊断字段对调参有价值非阻断。
- **F9. management commands/:id 回查 + tool-catalog** `[前]` — POST messages 同步内联已拿 command；工具目录后端 LLM 直接持有，**可发现性增强**。
- **F10. simulation evaluations/run** `[前]` — 与 formula-adherence + simulations/dialogue 重叠，**低价值**。
- **F11. AI 确信层 confirmedBy 语义展示** `[前]` — 字段已 deserialize，TagTrustPanel 未呈现 strong_evidence vs consolidation 来源区分。
- **F12. chunk distortionRisks/provenance** `[前]` — ⚠️PARTIAL-REFUTED：distortionRisks 等实际由 ReviewChat 渲染，唯 provenance 是真冗余。**只 provenance 待定**。
- **F13. command-center gatewayStatus 裸展示** `[前]` — 约 33 值英文串，该页偏调试性质，加 label map 即可。
- **F14. explore 任意租户 id** `[前]` — 后端无条件忽略(fail-safe)，误导性死控件，**移除输入框**即可。
- **F15. Operations 加载中态** `[前]` — store 无 loading 字段，挂载即 EmptyState，纯体验，加 loading 态。
- **F16. SSE 断连无重连** `[前]` — explore/today error handler 主动 es.close() 关闭浏览器原生重连，与 chunk WebSocket 退避重连不对称。
- **F17. explore stale closure** `[前]` — error handler 闭包捕获上轮 result 旧值，罕见时序误抑制错误横幅，改 ref 修复。
- **F18. referral 频道可见性门控** `[前]` — ⚠️恒可见（gapType 标签用反，实为 over-reachable），后端 assist_mode 守住红线+页面有免责文案，**低优**。
- **F19. ProfileDimension.participates_in_decision**（= D10，已在域四列）
- **F20. CoverageDimension.initial_signal/anchor_hint**（= D11，已在域四列）
- **F21. 知识长任务跟踪/取消** `[批次4][前端]` — 后端 KnowledgeTaskWorker 串行执行 + 任务态查询存在；前端无列表/会话联动，需手工粘贴 taskId。**依赖 E14（派工创建）落地后才形成闭环**，故 E14 修复时一并补跟踪面板。severity medium 偏 low。
- **F22. gap-digest repair/applied 落库审计** `[批次4][前端]` — `mod.rs` repair/applied 审计旁路，纯审计不影响主修复流程，前端零调用。**低优**，列全保完整。
- **F23. conversation_inferred 疑似成交线索闭环** `[批次3][前后端]` — ⚠️medium，非纯增强。`VERIFICATION_LABEL.conversation_inferred` 徽标存在，但 entitlements/直登链把 conversation_inferred 排除落库，outcome_events 永不出现该值；suspected_deal 仅进 prompt 引导，无审核/查询端点 → 整条疑似成交待核实队列 UI 缺失。**方案**：后端补 suspected_deal 审核/查询端点（draft 待核实，人审后才落 outcome），前端 products-deals 成交记录 Tab 加"疑似成交待核实"列表。归批次 3。

（F19/F20 与 D10/D11 同条交叉引用；本域去重后批次 4 净增约 18 条。**76 条总账**：审查产物 74 CONFIRMED + 2 PARTIAL = 76，其中 report 已合并 4 组跨维度重复命中（runtime-flag/threshold-audit/reassign/产品编辑各被两维度命中 → 本 spec 对应 C3/C4/B3 与产品编辑条目已合并）。本 spec 独立编号条目 A1-A5 + B1-B4 + C1-C10 + D1-D11 + E1-E16 + F1-F23（F19/F20 为交叉引用不计）= 67 个去重后的可执行条目，完整覆盖 76 条审查结论。）

## 不变量（修复全程守住）

- 后端 InboxItem 加字段必须是可选/向后兼容，不破 knowledgeReview/taxonomyCandidate/relationshipSuggestion 现有消费方。
- referral $unset 端点必须真正让 escalation/logic.rs:313 不再注入退辅助指引（红线：撤销后客户回主动运营）。
- 新建知识切片/文档保持 status=draft + needs_review，AI 永不自动验证红线不破。
- 所有新增前端文案不含禁词（CI 门）。
- 前端新组件遵守设计系统（tokens.css / .module.css / 4 级层级 / 颜色纪律）。

## 测试策略

- **前端**：vitest store 测（API 调用形态、错误态）+ 组件测（渲染、交互）。新增 feature 加 `__tests__/`。
- **后端**：cargo 集成测（新端点、扩字段投影），守 baseline ≥350/0。
- **用户验收**：标 ✅需浏览器 的条目（多为 CRITICAL/HIGH 交互），用户起 dev server 人肉验收。我会在每批交付时列清单。
- **回归**：每批跑 `scripts/check-baseline` + 前端 build + vitest 三连 + 禁词 lint。

## 范围与 YAGNI

- 本 spec 是全量路线图，但**只有批次 1（P0+P1，14 条）本轮直接走 plan**。批次 2-4 实现前各自用本 spec 条目展开 plan。
- 有意缺口（F3/F4/F5/F18 等）标注 verifier 取舍判定，**默认不做**，实施前用户确认才纳入。
- 不做群运营/朋友圈（Phase1 范围外，审查已排除）。
- 不借机重构无关代码；每条修复聚焦缺口本身。

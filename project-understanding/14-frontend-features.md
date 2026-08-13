# 前端 features 深读记录（核证日期 2026-08-13）

> 读法：`frontend/src/features/` 下 18 个 feature 目录全部 .ts/.tsx 文件逐行读完（含 knowledge/steward.tsx 3342 行、system-strategy/index.tsx 2831 行等大文件分段全读），另加 `frontend/src/components/review/` 9 个文件。所有断言附 `文件:行号`（行号相对 `frontend/src/`，即 `features/...` 指 `frontend/src/features/...`）。读不懂/存疑处集中在第 5 节。

## 1. 频道→feature→文件地图

### 1.1 入口链

- `main.tsx` → `App.tsx`（163 行）：App 只做三件事——挂全局 chunk WebSocket（`ws://…/api/ws/chunks`，`App.tsx:70-137`）、启动引导拉 `GET /api/accounts` 填 accountStore + `loadActiveProfile()/loadActiveView()`（`App.tsx:146-155`）、渲染 `<GlobalErrorBanner /> + <Shell />`（`App.tsx:157-162`）。频道视图全部迁出至 features/*（`App.tsx:10` 注释自证）。
- WebSocket 事件分发 `dispatchChunkEvent`（`App.tsx:46-68`）：`locked/unlocked` → window CustomEvent `wikiChunkLocked/wikiChunkUnlocked`；`revised` → `wikiChunkRevised` + `invalidateChunks()`；`lagged` → 仅 `invalidateChunks({reason:"lagged"})`。重连 1s 起指数退避、封顶 30s（`App.tsx:74,119`）。
- `app/Shell.tsx`（334 行）：侧栏（分组折叠导航 + AccountSwitcher + WorkspaceSwitcher + 登出）+ 主区（eyebrow/title/subtitle 头 + `<Suspense fallback="加载中…">` 下渲染当前频道组件，`Shell.tsx:322-331`）。0 账号空态给「同步微信号」按钮直接调 `POST /api/accounts/sync`（`Shell.tsx:14-19,57-72`）。
- `app/channels.ts`（344 行）：**单一事实来源** `CHANNELS: ChannelDef[]`（`channels.ts:116-344`），每项含 `id/group/label/caption/icon/eyebrow/title/subtitle/Component(lazy)/visibleWhen?/comingSoon?`。`visibleWhen` 谓词本期无频道使用，留作扩展点（`channels.ts:105-107`）。
- `stores/navigationStore.ts`（122 行）：`activeChannel`（默认 `"command"`，`navigationStore.ts:111`）+ 分组折叠态（localStorage key `wa.nav.collapsed.v4`，存被收起组名数组，白名单校验，默认收起 知识资产/平台配置/建设规划，`navigationStore.ts:31,67-71`）。

### 1.2 频道全表（20 个频道，18 个真实 feature + 2 个占位）

| # | Channel id | 侧栏分组 | label | feature 目录 | 备注 |
|---|---|---|---|---|---|
| 1 | `command` | 日常处置 | AI 总控 | `features/command-center/` | 写操作执行台（自然语言→微信工具调用）+ McpKeyForm（`channels.ts:57-58` 注释） |
| 2 | `accountManagement` | 平台配置 | 账号管理 | `features/account-management/` | 微信账号 + MCP 凭证 + 登录二维码 |
| 3 | `overview` | 日常处置 | 工作台 | `features/overview/` | 运行态势 |
| 4 | `userOps` | 客户运营 | 用户运营 | `features/user-ops/` | 最大业务频道之一 |
| 5 | `groupOps` | 建设规划 | 微信群运营 | —（`Component: OverviewFeature` 占位） | `comingSoon: true`（`channels.ts:171`），侧栏灰显不可点（`Shell.tsx:269-282`） |
| 6 | `momentOps` | 建设规划 | 朋友圈运营 | —（占位同上） | `comingSoon: true`（`channels.ts:183`） |
| 7 | `content` | 知识资产 | 内容资产 | `features/content-assets/` | 话术/FAQ/口吻/禁用表达/文件素材 |
| 8 | `referralCards` | 知识资产 | 专属顾问 | `features/referral-cards/` | 辅助模式名片库 |
| 9 | `askHuman` | 日常处置 | 统一收件箱 | `features/ask-human/` | 9 源待决事项收口 |
| 10 | `askHumanConfig` | 平台配置 | 请示通道配置 | `features/ask-human-config/` | 决策人链/触发情形/频控 |
| 11 | `campaign` | 客户运营 | 活动 | `features/campaign/` | 圈人预览 + 触达分布，确认推送在 AI 总控完成 |
| 12 | `productsDeals` | 客户运营 | 产品与成交 | `features/products-deals/` | 产品目录/成交登记/持有 |
| 13 | `knowledgeWiki` | 知识资产 | 知识库 Wiki | `features/knowledge/` | 最大 feature（19 文件 10711 行） |
| 14 | `systemStrategy` | 平台配置 | 系统策略 | `features/system-strategy/` | Prompt Pack / Agent 策略 |
| 15 | `llmProviders` | 平台配置 | AI 模型配置 | `features/llm-providers/` | LLM 服务商 CRUD/测试/热切换 |
| 16 | `operations` | 运行监控 | 任务日志 | `features/operations/` | 跟进任务 + 事件审计 |
| 17 | `evolution` | 平台配置 | 演化中心 | `features/evolution/` | 自演化 experiments/发布/回滚 |
| 18 | `quality` | 运行监控 | 运营成效 | `features/quality/` | 长期指标/公式遵守度/标记词 |
| 19 | `sendAnalytics` | 运行监控 | 发送成效 | `features/send-analytics/` | 素材/名片发送成效 |
| 20 | `autonomy` | 运行监控 | 自治回路监控 | `features/autonomy/` | 修订触发率/暂缓细分/outbox |

侧栏分组顺序 `GROUP_ORDER = 日常处置/客户运营/知识资产/运行监控/平台配置/建设规划`（`channels.ts:86-93`）。分组归属修正的实证注释见 `channels.ts:56-67`（command 是写操作台不入监控组、autonomy 归监控、「决策审批」组拆散——askHuman 进日常处置、askHumanConfig 进平台配置）。

### 1.3 feature 目录文件清单（行数见第 6 节覆盖自证）

- `command-center/`: index.tsx, McpKeyForm.tsx
- `overview/`: index.tsx
- `account-management/`: index.tsx, AccountLogin.tsx
- `user-ops/`: index.tsx, legacy.tsx, RosterView.tsx, PersonalityPanel.tsx, QuietHoursSettings.tsx, TagTrustPanel.tsx, BayesianTrendChart.tsx, poolHelpers.ts, cockpit/{CockpitPanel,ConfigureView,ObserveView,JudgmentBar}.tsx, cockpit/drilldowns/{ConversationReviewView,MemoryDetailView,SendHistoryView,TrendsDetailView}.tsx
- `content-assets/`: index.tsx
- `referral-cards/`: index.tsx
- `ask-human/`: index.tsx, ResolvedEscalations.tsx, inline/{EscalationInline,SimpleApproveReject,SuspectedDealReviewCard}.tsx
- `ask-human-config/`: index.tsx, DeciderChainEditor.tsx, policyForm.ts, deciderCandidates.ts
- `campaign/`: index.tsx, CampaignBoard.tsx, CampaignCreate.tsx, CampaignList.tsx, ProductMultiSelect.tsx, StageSelect.tsx, buckets.ts, csv.ts
- `products-deals/`: index.tsx
- `knowledge/`: index.tsx, today.tsx, steward.tsx, explore.tsx, atlas.tsx, shared.tsx, labels.ts, trustTypes.ts, chunkActionContracts.ts, chunkInvalidation.ts, ChunkRepairPanel.tsx, DocumentRepairPanel.tsx, DomainSchemaEditor.tsx, cockpit/{CockpitView,ReviewChat,AutoVerifyPanel,AnsweringModeGauge,CoverageVerdict}.tsx, cockpit/useGoLive.ts
- `system-strategy/`: index.tsx
- `llm-providers/`: index.tsx
- `operations/`: index.tsx
- `evolution/`: index.tsx, EvolutionCenterTab.tsx（另有 `src/EvolutionCenterTab.tsx` 根部 re-export 兼容层，22 行，供旧单测 import 路径使用，`src/EvolutionCenterTab.tsx:1-2`）
- `quality/`: index.tsx, EvaluationScenariosPanel.tsx
- `send-analytics/`: index.tsx
- `autonomy/`: index.tsx, OutboxPanel.tsx
- 复用评审组件 `components/review/`: ReviewQueue.tsx, ChunkReviewCard.tsx, TaxonomyCandidateReviewCard.tsx, ProfilePublishCard.tsx, ProposalReleaseCard.tsx, LessonPromoteCard.tsx, proposalPrimitives.tsx, proposalTypes.ts, evidenceMetrics.ts

## 2. 逐 feature 深读

### 2.0 公共底座（features 都依赖）

- **`lib/api.ts`（146 行）**：`api.get/post/put/patch/delete/postForm/postRaw` 薄封装。错误解析 `parseApiError`（`lib/api.ts:16-50`）：`{error:"llm_unavailable"}` → 抛 `LlmUnavailableError`（含 kind/retryCount/detail/hint）；`{error:string}` → `Error(error)`；HTML/SPA fallback → `HTTP <status>（服务端未返回 JSON…）`，短文本保留前 120 字。`postRaw` 不抛错返回 `{ok,status,data}`，专为 lock 409 带 payload 的分支（`lib/api.ts:96-114`）。`openEventSource`（`lib/api.ts:119-145`）统一 SSE 订阅：error 时回调并自动 close。
- **`app/GlobalErrorBanner.tsx`（16 行）**：读 `uiStore.error` 全局渲一条错误横幅；`stores/uiStore.ts`（15 行）只有 `busy/error` 两字段。store 的异步 action 内 catch 后统一 `useUiStore.getState().setError(...)`——这是「store 负责数据+错误上报到全局横幅、组件本地 state 负责表单内错误」分工的通例。
- **`stores/accountStore.ts`（35 行）**：`accounts/selectedAccountId`（localStorage key `wechatagent.accountId`，`accountStore.ts:4`）+ 派生 `currentAccountId()`（选中的不在列表则回落第一个，`accountStore.ts:24-29`）/`currentAccount()`/`onlineCount()`。
- **`stores/contactStore.ts`（130 行）**：联系人快照带**账号 scope 守卫**——`dataAccountId` 记录数据属于哪个账号、`requestGeneration` 递增丢弃迟到响应（`contactStore.ts:67-125`）；`loadContacts` 调 `GET /api/contacts?accountId=&limit=500[&q=]`，响应里混入他账号联系人则拒绝显示并报错（`contactStore.ts:95-101`）。`managedCount/normalCount` 都按 scope 过滤（`contactStore.ts:127-128`）。
- **`stores/profileStore.ts`（79 行）**：`loadActiveProfile` → `GET /api/admin/domain-profiles/active`；`loadActiveView` → `GET /api/operation/active-view`（返回 dimensions + taxonomies 取值字典）。两者失败都降级不阻塞（`profileStore.ts:54-61,70-77`）。纯函数 `labelFor(taxonomies, kind, value)` 三态：`ok`（命中字典）/`unknown_value`（数据野值）/`no_dict`（配置缺失），原值回显不猜标签（`profileStore.ts:23-29`）。全站显示 canonical id 的中文标签都走它。

### 2.1 command-center（AI 总控）— 2 文件

**结构**：无 tab。三栏布局——左「操作范围」面板（`index.tsx:177-209`）、中「指令面板」（`index.tsx:212-287`）、右「执行计划」（`index.tsx:290-329`）。

- **`command-center/index.tsx`（333 行）**
  - 左栏 StatusLine 组：账号在线数、当前账号（`mcpKeyConfigured` 则 tone=ai 否则 warn，`index.tsx:171`）、运营好友数（contactStore.managedCount）、待执行任务数、内容资产数、AI 人格版本数；下嵌 `McpKeyForm`（key 用 `${id}:${accountId}` 强制账号切换时重挂，`index.tsx:201-208`）。
  - 指令面板：textarea 绑 `commandStore.commandDraft`；示例 chips `EXAMPLES`（`index.tsx:14`）；演练模式 checkbox（`commandDryRun`，默认 true，`stores/commandStore.ts:38`）；执行按钮 disabled 条件 `commandBusy || !commandDraft.trim()`（`index.tsx:237`）。
  - **待确认流**：`commandResult.status === "pending_confirmation"` 且 `accountId === currentAccountId` 且有 `planHash` 才渲「确认执行/否决」条（`index.tsx:264-286`）——三重守卫防跨账号确认。
  - 状态标签闭集映射：tool call 状态 `callStatusLabel`（`index.tsx:35-56`，succeeded/accepted/failed/executed_unverified/execution_unknown/dry_run/executing/prepared；注释点明「executed_unverified=工具 Ok 但业务结果未核实，必须显『待核实』不当成功展示」`index.tsx:33-34`）；命令整体状态 `COMMAND_STATUS_LABELS`（`index.tsx:112-119`）；网关状态复用 `lib/reviewLabels.ts` 的 `GATEWAY_STATUS_LABELS`（`index.tsx:60-62`）。
  - `commandCallDetail`（`index.tsx:65-99`）：dry_run 摊开 `would_execute`（tool+content 前 60 字+error）；真实执行拼「实际发送/网关/复核/消息编号/原因」。
  - `dispatchCampaignId` 导出函数（`index.tsx:103-108`）：仅 `wechatagent.dispatch_campaign` 且 status ∈ {succeeded, executed_unverified} 且 response.campaignId 为 string 才返回 id → 渲「查看推送结果 →」按钮，点击 `useCampaignStore.getState().openReport(campaignId)` 跳活动频道（`index.tsx:309-317`）——跨 feature 跳转机制之一。
  - 账号切换效应：`commandResult.accountId !== currentAccountId` 时清结果 + 重载（`index.tsx:152-158`）；contactStore scope 不一致时静默补拉（`index.tsx:160-163`）。
- **`command-center/McpKeyForm.tsx`（148 行）**：账号 MCP 密钥表单。密钥 password 型不回显、仅显「已配置」布尔（`McpKeyForm.tsx:6-7,100-102`）；提交 `PUT /api/accounts/:accountRecordId/mcp-key`，body `{expectedAccountId, mcpApiKey, mcpBaseUrl?}`（camelCase 对齐后端 serde，`McpKeyForm.tsx:63-67`）；成功后立即清空输入不残留明文（`McpKeyForm.tsx:73-74`）。校验：key 为空 → 「请先填写 MCP 密钥」（`McpKeyForm.tsx:52-55`）。**账号切换竞态防护**：`scopeRef` 带 generation，账号 props 变化时 render 期间递增（`McpKeyForm.tsx:24-33`）+ effect 重置全部 state（`McpKeyForm.tsx:35-43`）；save 的 then/catch/finally 三处都校验 generation+scope 一致才 setState（`McpKeyForm.tsx:68-90`）。
- **调用端点**：`GET /api/content-assets[?accountId=]`、`GET /api/agent-souls`、`GET /api/tasks[?accountId=]`（以上 `stores/commandStore.ts:53-57` loadCommandData）；`POST /api/management-agent/sessions`（body accountId/title=draft 前 40 字/dryRun）→ `POST /api/management-agent/sessions/:id/messages`（`commandStore.ts:80-94`）；`POST /api/management-agent/commands/:id/confirm`、`/reject`（body 都带 `{accountId, planHash}` 冻结校验，`commandStore.ts:134-137,180-183`）；`PUT /api/accounts/:id/mcp-key`。
- **与 store 分工**：commandStore 持有 draft/result/dryRun/busy/souls/assets/pendingTasks；confirm/reject 前后各一层「结果仍属当前账号且 planHash 未变」守卫（`commandStore.ts:121-127,138-145`），响应合并时不属当前账号则丢弃（`commandStore.ts:98-100`）。错误全部上报 uiStore 全局横幅。
- **空态/错误态**：无 toolCalls 时执行计划栏渲三步静态占位（加载工具目录/生成执行计划/调用 MCP 工具，`index.tsx:322-328`）。

### 2.2 overview（工作台）— 1 文件

- **`overview/index.tsx`（123 行）**：无 tab。三张统计卡（点击跳频道：托管联系人→userOps、覆盖率→userOps、在线账号→overview 自身，`index.tsx:66-83`）+「实时运营流」面板（前 5 个 managed 联系人，`index.tsx:56`）。
  - 联系人状态三态映射 `contactStateLabel`（`index.tsx:24-28`）：非 managed→「未托管」(inactive)、有 `cooldownUntil`→「**AI 策略暂缓**」(held)、否则「自主回复」(running)——文案红线（不写"人工接管"）在此落地。
  - scope 守卫：`dataAccountId !== currentAccountId` 时渲空数组并静默补拉（`index.tsx:42-50`）。
  - 运营流行内：`operationState` 经 `labelFor(taxonomies,"customer_stage",…)` 显中文（`index.tsx:106`）；副行 `memorySummary || humanProfileNote || "尚无运营备注"`（`index.tsx:109`）。
  - 空态：`EmptyState title="暂无托管联系人" hint="到「用户运营」导入好友并开启自主运营"`（`index.tsx:95`）。
  - 疑点：顶部 import 了 `RefreshCw, ArrowRight`（`index.tsx:2`）但 JSX 未使用（dead import）；「在线账号」卡的 spark 柱状图是写死的静态高度（`index.tsx:79-82`），非真实数据。

### 2.3 account-management（账号管理）— 2 文件

**结构**：列表页 ⇄ 登录页两态切换（`showLogin` 本地 state，`index.tsx:15,49-59`），无 store（直接 api + accountStore.setAccounts）。

- **`account-management/index.tsx`（159 行）**：头部「同步账号」（`POST /api/accounts/sync` 后回拉 `GET /api/accounts`，`index.tsx:30-41`）与「登录微信账号」按钮；三张统计卡（在线/总数/离线）；账号卡网格显 alias/nickName/在线标/wxid/appId/status/MCP 配置与否（`index.tsx:116-154`）。空态文案：「暂无账号 / 点击『登录微信账号』添加第一个账号，或点击『同步账号』从 MCP Server 拉取已登录的账号。」（`index.tsx:100-113`）。
- **`account-management/AccountLogin.tsx`（225 行）**：扫码登录流。表单：账号别名（提示「Workspace Key 必填（如 t-1）；Account Key 可留空」`AccountLogin.tsx:132-134`）、登录平台 mac/ipad、登录流程 auto/manual。`POST /api/accounts/login/begin`（body `{accountAlias?, loginType, loginFlow}`，响应 snake_case：`session_id/qr_data_url/login_page_url`，`AccountLogin.tsx:52-63`）→ 渲二维码 img（dataURL）+ 可选外链 MCP 登录页；轮询 `GET /api/accounts/login/poll?loginSessionId=&accountAlias=`，`pending` 每 2.5s 再轮（`AccountLogin.tsx:92-93`），`success` → 自动 `POST /api/accounts/sync`（失败不阻断成功提示，`AccountLogin.tsx:86-90`）→ `onLoggedIn` 回调让父组件回拉列表；其他状态（如 expired）→ 错误文案「登录已过期/未完成（status）」（`AccountLogin.tsx:95`）。**竞态防护**：`loginGeneration` ref，unmount/reset/重新发起时 +1，所有异步回调校验 generation（`AccountLogin.tsx:31-38,57,66,81,99`）。

### 2.4 user-ops（用户运营）— 16 文件 + 2 专属 store

**内部导航**：顶层三模式 `UserOpsMode = smart | roster | traditional`（`legacy.tsx:169-182` MODE_COPY；模式切换条 `UserOpsModeHeader`，`legacy.tsx:184-210`）。smart = 运营池 ContactsView + CockpitPanel 驾驶舱；roster = RosterView 通讯录；traditional = 4 个 tab（playbooks 运营方法 / prompts 提示词 / settings 运行策略 / audit 审计复盘，`legacy.tsx:420-454` TraditionalOpsTabs）。audit tab 直接内嵌整个 `OperationsFeature`（`index.tsx:457`）。

- **`user-ops/index.tsx`（462 行）**：编排层。根组件补挂 `ToastProvider + ConfirmProvider`（`index.tsx:54-60`，注释说明 usePromptSaveConfirm 需要 ConfirmProvider 祖先）。关键细节：
  - 订阅派生的原始 accountId 字符串而非 `currentAccountId` 函数引用（恒稳定导致 effect 永不触发，`index.tsx:158-164`）。
  - scope 守卫三层：`contactSnapshotIsCurrent`（`index.tsx:166-170`）、切账号时 `clearContactDetail + setSelected(null)`（`index.tsx:245-253`）、选中变化时 `hydrateSelected + loadMessages`（`index.tsx:268-273`）。
  - 计数用后端真实 `contactCounts` 而非对已加载数组 filter（list_contacts limit=500 截断会导致偏小，`index.tsx:217-221`）。
  - guide apply 成功 toast：文案中性化为「已处理 N 项」而非「应用了 N 个字段」（appliedFields 语义偏大，`index.tsx:200-215`）；跳过字段拼「（取值越界，已忽略）」。
  - `pendingTasks` 写死 0（徽标待接 operationsStore，`index.tsx:229-231`）——疑点见第 5 节。
  - traditional/playbooks tab 有 `playbookScopeAccountId === effectiveAccountId` 渲染守卫（`index.tsx:395`）。
- **`user-ops/legacy.tsx`（2091 行）**：canonical 组件库（注释自述「index.tsx 只编排，这里是 7 个根组件 + helper」`legacy.tsx:1-3`）。主要导出：
  - `MEMORY_DRAFT_FIELD_GROUPS`（`legacy.tsx:86-138`）：23 个扁平记忆字段按后端 4 个 Document 分组（用户理解 7 / 关系状态 6 / 产品契合 2 / 下一步动作 8）；与 store 的 `MEMORY_DRAFT_GROUPS`（`stores/userOpsStore.ts:206-238`）两侧字段名须对齐。
  - `USER_RUNTIME_PARAMETER_FIELDS`（`legacy.tsx:140-167`）：20 个运行参数（含 4 个 quietHours* 字段）与默认值（如 hallucinationBlockAt=6、knowledgeGroundingBlockBelow=7、runTokenBudget=30000）。**注意**：`stores/userOpsDomainHelpers.ts:20-47` 有一份近乎相同的副本（一处 label 差异："状态置信复盘线" vs "状态置信 Review 线"）——见第 5 节偏差。
  - `ContactsView`（`legacy.tsx:498-738`）：运营池列表。三 tab（待启用 normal / Agent managed / 全部）；仅 normal tab 开放勾选批量启用（`legacy.tsx:543`）；行内「启用 Agent」「从池移除」（window.confirm 后 onHideFromPool，`legacy.tsx:714-728`）；`selectionScope` 变化清空勾选防串号（`legacy.tsx:538-540`）；超时未跟进标签用 `overdueHours`（poolHelpers）；空态「还没有人主动来找你 / 去通讯录主动开启 Agent 运营。」（`legacy.tsx:633-637`）。
  - `DomainConfigEditor`（`legacy.tsx:852-1059`）：运行策略编辑（基础策略 name/goal/methodology、执行边界 workflow/toolPolicy/automationPolicy/reviewPolicy/assistModeEnabled 下拉、运行参数 20 项 grid、状态机逐状态卡片编辑）。runtimeParameters 在 UI 层是「key = value」行文本互转（`legacy.tsx:1549-1594`）；stateMachine 是 JSON 文本↔结构化卡片互转（`legacy.tsx:1597-1645`）。顶部 `ActiveVersionsBar`（`legacy.tsx:741-849`）：显 v 号/当前生效/影子版本/seededBy/回滚链，动作 `POST /api/admin/operation-domains/:id/{publish|rollout|rollback}`（window.confirm 二次确认，`legacy.tsx:765-785`）。
  - `UserPlaybookPanel`（`legacy.tsx:1062-1242`）：运营方法列表 + 表单（11 字段）+ AI 生成 bar + AI 优化框 + 设默认。保存禁用条件 `!name.trim() || !methodPrompt.trim()`（`legacy.tsx:1230`）。
  - `DomainPromptPanel`（`legacy.tsx:1245-1453`）：人格设定（souls）与任务提示词（promptTemplates）双列表+双表单，按 agentKinds 过滤 + status 排序（active/published→draft→archived，`legacy.tsx:1660-1672`）；prompt 层级下拉 5 值 system_contract/policy/task_template/review/methodology_generator（`legacy.tsx:1402-1408`）；校验：soul 需 name+content，prompt 需 promptKey+title+content（`legacy.tsx:1368,1446`）。
  - `ConversationStream`（`legacy.tsx:1491-1536`）：左右气泡会话流，>30 分钟插时间分隔（`legacy.tsx:1499`）；msgType=namecard 渲「已为客户引荐专属顾问」气泡、media 渲附件气泡（`legacy.tsx:1516-1525`）。
  - `PlannerViewSection`（`legacy.tsx:1865-1977`）：主动跟进视角只读卡——停滞维度（`stagnationDimension/Value/UpdatedAt`）+ commitments 前 5 条 + 上轮对话模式 + 其余画像维度（从 domainAttributes 遍历 dimensions）；labelFor 三态回落都带 title 提示（`legacy.tsx:1914-1927`）。
  - `SendHistorySection`（`legacy.tsx:1985-2060`）：`GET /api/contacts/:wxid/send-history?accountId=`；**明确区分「拉取失败」与「没发过」**——失败渲「发送记录加载失败…」而非事实性断言「还没发过」（`legacy.tsx:1989-1991,2027-2043`）；`responded` 三态 已响应/未响应/待评估（`legacy.tsx:2065-2078`）。
  - 标签映射函数群：`memoryStatusLabel`（pending 待整理/consolidated 已入库/ignored_low_score 低价值忽略，`legacy.tsx:1718-1723`）、`memoryCandidateTypeLabel`（7 类，`legacy.tsx:1726-1738`）、`memoryCandidateSourceLabel`（9 源，`legacy.tsx:1742-1756`）、`simulationStatusLabel`（would_send/no_reply/review_blocked/gateway_blocked，`legacy.tsx:1775-1783`）、`impactScopeLabel`（4 个非默认 scope，`legacy.tsx:1825-1831`）、`playbookCreatedByLabel`（system*/manual/agent/agent_optimized，`legacy.tsx:1709-1716`）、`agentKindLabel`（含 group/moment 预留，`legacy.tsx:1648-1657`）。
- **cockpit/ 驾驶舱**（Task 2-7 重构产物）：
  - `CockpitPanel.tsx`（189 行）：外壳。段控 observe/configure + 下钻 `Drilldown = memory|conversation|sendHistory|trends`（`CockpitPanel.tsx:33-34`）；未选中时渲三步 onboarding PlanStep（`CockpitPanel.tsx:95-105`）；`JudgmentBar` 常驻（请示灯计数来自 `inboxStore.principalEscalationCount`，点击跳 askHuman 频道，`CockpitPanel.tsx:91-92,131`）。
  - `JudgmentBar.tsx`（148 行）：6 chip——人格态（lastConversationMode 经 conversation_mode 字典）/最近轮（`finalReviewTone` 把 finalReviewStatus 闭集 10 态分三色：approved+revision_applied_approved→sent；held_by_ai_policy+ai_waiting_for_more_context→held；blocked_by_safety_guard/required_field/budget/unverified_product_claim+revision_failed→blocked；其余 other，`JudgmentBar.tsx:13-30`）/下一步/风险灯（health 有 danger 项）/作息灯（`inQuietHours` 断言读取 OperationHealth 未声明的顶层键，文案「客户休息时段留言，将在 HH:mm 后统一回复」，`JudgmentBar.tsx:90-94,125-132`）/请示灯（null 显「请示计数不可用」不能与 0 混淆，`JudgmentBar.tsx:58-61`）。
  - `ObserveView.tsx`（164 行）：只读观测——Agent 行为 4 格（语气/节奏/话题/避免，值三级回落 agentProfile→memoryDraft→默认文案，`ObserveView.tsx:55-72`）、Agent 当前判断 6 格（含入站/出站时间分列，`ObserveView.tsx:93-102`）、TagTrustPanel、PersonalityPanel（+走势下钻）、运营健康度（tone 三色 class）、长期记忆卡（点击下钻）、PlannerViewSection、发送历史下钻入口。
  - `ConfigureView.tsx`（475 行）：配置段 4 次级 tab 画像/指令/记忆/工具（`ConfigureView.tsx:36-41`）。画像 tab：运营风格模板（仅 published playbooks）、运营判断 profileNote、特别指令 customAgentInstructions（maxLength 1000 + 计数器，`ConfigureView.tsx:162-169`）、辅助模式 override 三值 default/force_on/force_off + 已引荐态显示 + 「撤销引荐 / 恢复主动运营」按钮（`ConfigureView.tsx:177-209`）、客户类型（relationship_type 字典，未配字典回落写死三项 customer/peer/friend，`ConfigureView.tsx:216-230`）、最近承诺/跟进策略（profileEditDraft）、启停 Agent（停止有 window.confirm；启用 disabled 条件 `!profileNote.trim()`，`ConfigureView.tsx:272-289`）。指令 tab：guide 指令 textarea + 4 示例 chip + 预览（`impactScope` 非默认时 `requiresStrongConfirmation` → window.confirm 逐字确认再带 confirmGlobalImpact 重提，`ConfigureView.tsx:339-352`）。记忆 tab：4 分组手风琴（默认只开第一组，`ConfigureView.tsx:91-94`）。工具 tab：记忆候选列表 + 影子验证（simulation 输入每行一条消息）。
  - drilldowns：`ConversationReviewView.tsx`（157 行，会话流+复盘列表前 8 条，每条可展开 scores/risks/终审/暂缓类别/AutonomyProtocol「AI 内心独白」9 字段三组——回复决策/理解/运营依据，全空不渲染，`ConversationReviewView.tsx:20-24,49-123`）；`MemoryDetailView.tsx`（158 行，记忆全景：事实三分区带 confidence/importance/易失效/证据/弃用原因 + 6 纯文本分区 + coreFactEvictions 核心事实归档「因核心事实窗口上限归档 · 原排名 N」，`MemoryDetailView.tsx:137-153`）；`SendHistoryView.tsx`（20 行，包一层下钻头复用 SendHistorySection）；`TrendsDetailView.tsx`（32 行，PersonalityPanel + BayesianTrendChart）。
- **独立面板组件**：
  - `TagTrustPanel.tsx`（158 行）：三层标签物理分离——运营录入 manualTags（权威，中性色，编辑用逗号/中文逗号分隔 split，`TagTrustPanel.tsx:41-48`）、AI 确信 confirmedTags（紫色只读 chip 带证据条数，点开显 evidence turn/msgId；confirmedBy 闭集 strong_evidence「强证据」/consolidation「压缩重判」，`TagTrustPanel.tsx:9-12`）、贝叶斯评估层（「持续观测，永不驱动行为」，`TagTrustPanel.tsx:151`）。
  - `PersonalityPanel.tsx`（157 行）：OCEAN 五维横 bar；confidence<0.3 灰化、=0 标「证据不足」（诚实原则——后端无证据强制 conf=0，UI 绝不为其画实色 bar，`PersonalityPanel.tsx:8-10,64-68`）；snapshots≥2 画五线 SVG 折线演化图。
  - `BayesianTrendChart.tsx`（113 行）：手写 SVG 折线；仅 `locked=true` 的维度画线（未占槽=证据不够，`BayesianTrendChart.tsx:10,46`）；图例显维度显示名 + 当前值中文 + 置信度%。空态「暂无评估维度（需多轮强证据才占槽）」。
  - `QuietHoursSettings.tsx`（190 行）：作息弹窗（Overlay）。表单校验：`enabled && startHour === endHour` → 报「休息开始时间不能与醒来时间相同」且保存禁用（`QuietHoursSettings.tsx:64,33`）；保存把 4 个 quietHours* 键写回 runtimeParameters 文本再走 `saveOperationDomain("user_operations")`（`QuietHoursSettings.tsx:69-72`）；持久化缺失时按钮退化为「重新加载作息」。生效说明文案：「保存成功后立即生效，无需重启…已排队的醒来回复保留原执行时间」（`QuietHoursSettings.tsx:173-177`）。
  - `poolHelpers.ts`（39 行）：纯函数 `overdueHours`（入站后无出站且 ≥24h，`poolHelpers.ts:3,10-19`）、`formatRelativeTime`（刚刚/N 分钟前/…/N 个月前）。
- **`RosterView.tsx`（425 行）**：通讯录视图。好友卡网格（每页 60 本地分页 hook，`RosterView.tsx:10-17`）；`isNonHuman` 系统账号折叠区不可勾选（`RosterView.tsx:171-173,353-387`）；已 managed 卡禁用。**三个精心处理的竞态/轮询**：请求序号 `reqSeqRef` 丢弃过时响应（防止账号 A 的好友配 B 的 accountId 提交，`RosterView.tsx:64-67`）；syncing 时每 10s 自动重拉（`RosterView.tsx:111-119`）；点「刷新」记 `serverFetchedAt` 基线后每 3s 用 `loadRoster({revalidate:true})` 静默轮询直到快照变化（60s 超时提示「微信侧仍在同步」；不能走 refresh 否则清空勾选草稿；toast 用 ref 不进依赖否则 interval 被无限重置——注释记录实测坑，`RosterView.tsx:121-161`）。批量提交校验：必须勾人且 sharedNote 非空（`RosterView.tsx:188,417`）。
- **调用端点（user-ops 全量）**：详情五连拉 `GET /api/conversations/:contactId/messages?limit=50`、`GET /api/contacts/:id/operating-memory`、`GET /api/contacts/:id/memory-candidates?limit=30`、`GET /api/decision-reviews?accountId=&contactId=&limit=20`、`GET /api/contacts/:id/operation-health`（Promise.allSettled 部分失败仍渲染成功部分，首个错误上报，`userOpsStore.ts:476-513`）；`GET /api/contacts/counts?accountId=`（失败静默保留旧值，`userOpsStore.ts:571-581`）；`GET /api/contacts/roster?accountId=[&force=true]`；`POST /api/contacts/batch-enable`；`POST /api/contacts/:id/hide-from-pool`；`POST /api/contacts/:id/enable-agent`（body expectedAccountId+humanProfileNote+playbookId）/`disable-agent`/`analyze-profile`/`clear-referral`；`PUT /api/contacts/:id/profile-note`/`custom-agent-instructions`/`assist-override`/`operation-profile`/`operating-memory`/`manual-tags`（PUT 类 body 全带 `expectedAccountId` 乐观校验）；`POST /api/user-operations/guide/preview`、`/apply`（body previewId+expectedAccountId+expectedContactId+candidateHash+confirmGlobalImpact——**hash 冻结防应用陈旧预览**，`userOpsStore.ts:958-967`）；`POST /api/contacts/:id/memory-consolidation/run`；`POST /api/user-operations/simulations/dialogue`（messages 按行拆分）；playbooks：`GET/POST /api/operation-playbooks`、`PUT /api/operation-playbooks/:id`（带 expectedVersion 乐观锁）、`POST …/:id/optimize`（校验返回必须是**新 id + version+1** 的候选，编辑器整体切到候选身份，`userOpsStore.ts:1180-1209`）、`POST …/generate`、`POST …/:id/set-default`；域配置：`GET /api/operation-domains`、`PUT /api/operation-domains/:domain`、`POST /api/operation-domains/:domain/reset`、版本动作 `POST /api/admin/operation-domains/:id/{publish|rollout|rollback}`。
- **与 store 分工**：`userOpsStore`（1369 行）持有全部详情/草稿/剧本/域配置/roster 缓存；`detailActionIsCurrent`（`userOpsStore.ts:313-324`）是所有写操作前的六重身份守卫（账号一致+contactStore scope 一致+选中一致+detail 快照一致）。contactStore 管列表与选中；账号级数据在 accountStore。`hydrateSelected` 从 contact.domainAttributes 回填 assist_mode_override/relationship_type/referred_specialist_at/referred_card_id（dotted-key，`userOpsStore.ts:434-450`）。记忆草稿双向映射 `groupMemoryDraft`/`memoryDraftFromMemory`（`userOpsStore.ts:241-270`）。

### 2.5 content-assets（内容资产）— 1 文件

- **`content-assets/index.tsx`（792 行）**：无 tab，左列表 + 右两表单（新增文本资产 / 上传销售素材）。外壳组件用 `key={currentAccountId}` 强制切账号重挂（`index.tsx:72-75`）。
  - 文本资产 5 类 `KIND_OPTIONS`：text/faq/script/forbidden_expression/brand_voice（`index.tsx:11-17`）；老数据 `moment_media` 回落「朋友圈素材」（`index.tsx:23`）。
  - **注入档三档** lean/relational/full（`tierLabel`，`index.tsx:31-40`）；禁用表达恒注入、UI 隐藏档位选择并把 draft 归位 full（避免残留档位落库，`index.tsx:27-28,272-281`）；行内徽标显「恒注入」（`index.tsx:488`）。
  - 上传表单字段：file（accept 图片/PDF/Office/MP4，`index.tsx:54-55`）、mediaType image/file/video、sendTriggerHint、expressionPref（file_primary 以文件为主 / file_support 文件为辅）、targetStages/tags（逗号分隔）、`requiresPrincipalApproval` checkbox「发送前需领导审批」（`index.tsx:414-421`）。提交校验 `!file || !mediaTitle.trim()` 禁用（`index.tsx:425`）。
  - 列表分区：非 media 的文本资产在前，media 素材在「销售素材文件」区（`index.tsx:159-160,225-227`）；每行显 scope 徽标「账号专属 · id / 全账号共享」（`index.tsx:484,655`）；media 行有 审核（标记为可发送）/启停（sendable 缺省视为 true，`index.tsx:588`）/编辑 meta/换文件/删除（window.confirm）。
  - 空态：「暂无内容资产 / 在右侧新增文本、FAQ、话术或品牌语气，供 Agent 自主运营调用。」（`index.tsx:204`）。
- **调用端点**（经 `stores/contentStore.ts`，296 行）：`GET /api/content-assets?accountId=[&tag=]`（响应混入他账号资产则拒绝显示，`contentStore.ts:134-138`）；`POST /api/content-assets`；`POST /api/content-assets/upload`（multipart）；`POST /api/content-assets/:id/review`（body 带 `expectedScope: account|workspace` + expectedAccountId——**scope 冻结**防止把全账号共享资产当账号资产改，`contentStore.ts:64-68`）；`PUT /api/content-assets/:id`；`POST /api/content-assets/:id/file`（multipart 换文件，form 里 set expectedScope）；`POST /api/content-assets/:id/toggle`；`DELETE /api/content-assets/:id?expectedScope=…`。所有写操作前 `actionIsCurrent` 校验（页面账号=当前账号=快照账号且资产仍在列表，`contentStore.ts:79-88`）。
- **与 store 分工**：store 持有 assets + draft（draft 也带 accountId scope，`assetDraftAccountId !== currentAccountId` 时渲染空 draft，`index.tsx:111-113`）；上传表单的 8 个字段是组件本地 state（一次性提交，`index.tsx:97-106`）。

### 2.6 referral-cards（专属顾问名片库）— 1 文件

- **`referral-cards/index.tsx`（270 行）**：无 tab，左名片列表 + 右新增表单。页首两段说明文案直接写明辅助模式语义（新建默认草稿且停用；账号级开关默认关闭；要真正引荐还需去用户运营域开 assistModeEnabled，`index.tsx:75-83`）。
  - 表单：顾问名称、顾问微信号（从好友选择——复用 `userOpsStore.loadRoster` 的通讯录缓存 + `FriendPickerModal`，支持手输 wxid `allowManualWxid`，`index.tsx:36-46,195-203`）、引荐时机自然语言、目标阶段/标签（逗号分隔，提示「取值需在运营域配置阶段字典」）。提交校验 `!displayName.trim() || !targetWxid.trim()` 禁用（`index.tsx:187`）。选好友时若 displayName 为空自动带入 remark/nickname（`index.tsx:48-57`）。
  - 名片行 `ReferralCardRow`：双徽标（reviewStatus：approved「可引荐」/draft「待审核（草稿）」；enabled：已启用/已停用，`index.tsx:223-235`）；动作 标记为可引荐 ⇄ 撤回为草稿、启/停用、删除（window.confirm「删除后 AI 将不再引荐该顾问」，`index.tsx:67-71`）。
  - 空态：「暂无专属顾问名片 / 在右侧录入一位真人顾问的名片与引荐条件，审核启用后供 AI 在辅助模式下主动引荐。」（`index.tsx:96-99`）。
- **调用端点**（`stores/referralCardStore.ts`，115 行）：`GET /api/referral-cards`；`POST /api/referral-cards`（targetStages/tags 前端拆逗号成数组，`referralCardStore.ts:50-57`）；`POST /api/referral-cards/:id/review`（body `{status: "approved"|"draft", note?}`）；`POST /api/referral-cards/:id/toggle`（body `{enabled}`）；`DELETE /api/referral-cards/:id`。

### 2.7 ask-human（统一收件箱）— 5 文件

**结构**：pending 收件箱 ⇄ resolved 已裁决历史 两视图切换（`showResolved` 本地 state，`index.tsx:286,358`）。pending 视图 = 来源 chip 过滤条 + 账号过滤下拉 + `ReviewQueue<InboxItem>` 列表。根组件包 `ConfirmProvider + ToastProvider`（`index.tsx:511-519`）。

- **`ask-human/index.tsx`（519 行）**
  - **9 源单一事实来源 `SOURCE_META`**（`index.tsx:26-60`）：summaryKey(camelCase)↔source(snake_case)↔中文标签——principal_escalation 请示裁决 / knowledge_review 知识核验 / taxonomy_candidate 标签候选 / relationship_suggestion 关系建议 / suspected_deal 疑似成交 / gap_signal 知识缺口 / profile_risky 画像发布 / evolution_proposal 进化发布 / lessons_learned 经验晋升。徽标 tone 表 `SOURCE_TONE`（`index.tsx:63-73`）。
  - **两级渲染分发**：`item.actionKind === "rich"` → `renderRich` 按 `richComponent` 分派 6 张卡（knowledgeReview→ChunkReviewCard、profilePublish→ProfilePublishCard、evolutionRelease→ProposalReleaseCard、lessonsPromote→LessonPromoteCard、taxonomyCandidateReview→TaxonomyCandidateReviewCard、suspectedDealReview→SuspectedDealReviewCard；未知渲「未知 rich 组件：…」，`index.tsx:76-146`）；inline → `renderInline` 按 source 分派（principal_escalation→EscalationInline；relationship_suggestion/gap_signal→SimpleApproveReject 详情 + 行内常驻 `SimpleActionButtons`；未知渲标题兜底，`index.tsx:176-187`）。
  - 一键处置端点表 `SIMPLE_ENDPOINTS`（`index.tsx:154-165`）：relationship_suggestion → `POST /api/admin/relationship-type-suggestions/:id/{approve|reject}`；gap_signal → `POST /api/knowledge/gap-signals/:id/dismiss`。抽表原因：行内按钮与卡体两个消费者共用 URL（`index.tsx:148-153` 注释）。
  - `InboxRow`（`index.tsx:202-259`）：两行式折叠行（徽章+标题行 / 摘要行）；无 children 不可展开不渲 chevron；actions 渲染在 toggle button 之外（button 套 button 非法 HTML + 冒泡，`index.tsx:199-201` 注释）；展开后隐藏摘要防止与卡体重复（`index.tsx:249-251`）。knowledge_review 且 `integrityStatus === "needs_human_audit"` 加 tag「AI预审通过·待复核」（`index.tsx:479-484`）。
  - **chip 可见性规则**（`index.tsx:307-321`）：只渲染 count>0 的源，但三种必须保留——计数不可用（显「不可用」，「隐藏它等于把『查不到』伪装成『没有』」）、正在筛选的源（否则处理完最后一项无法取消筛选）。「全部」chip 是取消过滤的显式落点（`index.tsx:416-431`）。
  - 刷新机制：一切刷新走 `refreshNonce` → ReviewQueue refetch → `fetchItems` → `inboxStore.load()`（store 是唯一 fetch 来源，消除单次刷新打两次 /inbox；fetchItems 必须 memoize 否则死循环——注释详述，`index.tsx:288-300`）。
  - 错误态三层：fatalError「加载失败（显示上次数据）：…」；`errors` 源级「N 个来源暂时不可用：…」；summary 非 complete「待办计数部分不可用：…」（`index.tsx:362-390`）。`summary.total == null` 不渲「待处理 0 项」（错误信息，`index.tsx:327-333`）。空态：「暂无待处理项 / AI 自主运行中，需要决策或审核的事项会自动出现在这里。」（`index.tsx:497-502`）。
  - 账号过滤下拉：「账号筛选下仍保留全局治理事项」（无 accountId 的条目是 workspace 全局治理，`index.tsx:392-411`；`lib/inboxApi.ts:6-7` 注释）。
- **`ask-human/inline/EscalationInline.tsx`（150 行）**：请示裁决表单。裁决口径闭集 5 值（approved 同意/rejected 拒绝/conditional 有条件同意/deferred 暂缓待定/delegated_back 授权 AI 自行处理，`EscalationInline.tsx:8-14`）；approved/conditional 才显示 转述有效期（number 0.01-8760 小时可空）与豁免范围（none/customer_only/knowledge 三档，`EscalationInline.tsx:15-19,95-127`）；conditional 另显约束条款输入。提交 `POST /api/admin/principal-escalations/:code/resolve`（body verdict/substance/constraints[]/authorizationWindowHours/exemptionType；非授权型 verdict 强制 exemptionType="none"，`EscalationInline.tsx:31-45`）；改派 `POST /api/admin/principal-escalations/:code/reassign`（body toWxid）。id 即 short_code（`EscalationInline.tsx:29`）。
- **`ask-human/inline/SimpleApproveReject.tsx`（91 行）**：卡体只渲染行头容不下的细节（summary/判断依据/置信度/出现次数/客户标识/gap kind+严重度经 GAP_SIGNAL_*_LABELS）；`SimpleActionButtons` 通过/拒绝/忽略三按钮按 endpoints 有无渲染，行内与卡体共用（`SimpleApproveReject.tsx:46-91`）。
- **`ask-human/inline/SuspectedDealReviewCard.tsx`（141 行）**：疑似成交复核。**表单校验**：金额可选但填了必须 ≥0 有效数字（`yuanToCents` 元→分并检查 `Number.isSafeInteger`，`SuspectedDealReviewCard.tsx:13-20,37-42`）；币种必须三位大写代码 `/^[A-Z]{3}$/`（`SuspectedDealReviewCard.tsx:43-47`）；驳回原因必填（`SuspectedDealReviewCard.tsx:63-68`）。端点 `POST /api/admin/suspected-deals/:signalId/{approve|reject}`。
- **`ask-human/ResolvedEscalations.tsx`（124 行）**：已裁决历史只读列表，自取数 `GET /api/admin/principal-escalations?status=resolved`（与 pending inboxStore 正交，`ResolvedEscalations.tsx:5-9` 注释并核实 wire 键：外层 camelCase、decision 内层 snake_case 如 `authorization_window_hours`）。显 shortCode/verdict 标签/substance/约束/「本次转述到期」（空值显「本次转述不设期限」）/长期豁免（knowledge→「该客户 + 通用知识」）/裁决渠道。
- **与 store 分工**：`stores/inboxStore.ts`（90 行）——`load()` 并发拉 `GET /api/admin/ask-human/inbox[?source=&accountId=]` + `refreshSummary()`→`GET /api/admin/ask-human/summary[?accountId=]`（`lib/inboxApi.ts:73-86`）；inbox 失败是 fatal 但**保留旧 items 绝不清空**（`inboxStore.ts:81-88`）；summary 失败保留最后成功快照（降级数据，`inboxStore.ts:55-57`）。排序 `sortItems`：severity 降序、同级 ageHours 降序（`lib/inboxApi.ts:56-63`）。`principalEscalationCount` 导出给 user-ops 判断条请示灯（`inboxStore.ts:27-32`）。

### 2.8 ask-human-config（请示通道配置）— 4 文件

**结构**：单页表单（决策人链 / 触发情形 4 开关 / 超时转备选 / 高级：推送频控 details 折叠），无 store（本地 draft state）。

- **`ask-human-config/index.tsx`（210 行）**：
  - 读 `GET /api/operation-domains/user_operations` 从 `item.askHumanPolicy` 抽策略（`index.tsx:35-36`）；保存 `PUT /api/operation-domains/user_operations/ask-human-policy`（`index.tsx:82`）。**fail-safe**：读取失败时禁止保存（「读取现有配置失败，已禁止保存以避免覆盖线上策略」，保存按钮 disabled 条件含 `!loaded || Boolean(loadError)`，`index.tsx:70-74,111,116`）。保存失败草稿不丢（`index.tsx:85-87`）。
  - 4 个触发情形开关文案严格用 AI 内部语义：「安全门拦截时 / 产品声明未经核验时 / **AI 策略主动暂缓时** / 对话停滞推不动时」（`index.tsx:15-20`）——禁词闸规避范例。
  - 可选数值字段语义：空串→删除键（=不限），有值→number（`setNumField`，`index.tsx:93-105`）；静默时段三格须同时填、全空删除 quietHours（`index.tsx:49-68,196`）；「清空决策人链并保存，即明确关闭请示通道」（`index.tsx:121`）。
- **`ask-human-config/policyForm.ts`（93 行）**：`defaultPolicy()`（空链 + escalateSafetyGuard/UnverifiedProduct/Stuck=true、escalateAiPolicyHold=false，与后端非-all 回落一致，`policyForm.ts:4-12`）；`extractPolicy` 逐字段存在性回落；`validatePolicy`（校验规则：链内 wxid 非空、**决策人必须绑定发送账号**、静默小时 0-23、去重/超时非负、每日上限 ≥1；空链不是错误而是显式关闭态，`policyForm.ts:70-93`）。
- **`ask-human-config/DeciderChainEditor.tsx`（222 行）**：决策人链编辑（上移/下移/删除 + FriendPickerModal 选人）。深坑处理密集：
  - 选中未导入好友时先 `POST /api/contacts/import`（**upsert $setOnInsert agent_status="normal" 不托管**；明确不用 batch-enable——那会把内部决策者当客户交给 AI 运营，`DeciderChainEditor.tsx:118-123` 注释）；接口 200 但 `items` 为空 = 静默失败（可能被识别为非真人账号），必须显式报错（`DeciderChainEditor.tsx:136-144`）。
  - `chainRef` 防 stale 闭包（await import 期间 chain 可能已变，`DeciderChainEditor.tsx:34-39`）+ 加链前幂等去重（`DeciderChainEditor.tsx:157-163`）+ `importing` 重入守卫防连点（`DeciderChainEditor.tsx:103-107`）。
  - 弹窗内失败提示一律 toast（Overlay scrim z-index 1000 会盖住内联错误；toast z-index 1100，`DeciderChainEditor.tsx:110-115`）。
  - roster 拉取带请求序号守卫 + syncing 每 10s 轮询（抄 RosterView 同款，`DeciderChainEditor.tsx:41-81`）。
- **`ask-human-config/deciderCandidates.ts`（22 行）**：`isPickableDecider` 与后端 `webhooks::is_operatable_person` 等价过滤（isNonHuman + `gh_` 前缀 + `@chatroom` + `@openim`）；注释详述为何不复制后端 13 条系统号白名单（两份清单必然漂移，`deciderCandidates.ts:1-14`）。

### 2.9 campaign（活动推送）— 8 文件

**结构**：三视图状态机 `view: list | create | board`（存 campaignStore 而非本地——供跨频道 `openReport` 跳入，`campaign/index.tsx:6-11`、`stores/campaignStore.ts:48,72-76`）。`openReport(id)` 会 `setChannel("campaign")` + 清 report + 置 view=board（`campaignStore.ts:72-76`）——AI 总控「查看推送结果」按钮的落点。

- **`campaign/CampaignCreate.tsx`（177 行）**：新建活动表单（标题、意图 intentText、圈人条件：买过的产品多选/客户阶段/售后状态 in_aftercare|expired/价值分层 high|mid|low）。**规格冻结/CAS 流**：首次预览先 `POST /api/campaigns`（拿 id+specVersion）；字段再改动置 `draftDirty` 并使旧预览失效（`onSpecChange`，`CampaignCreate.tsx:83-88`）；脏了再预览先 `PATCH /api/campaigns/:id`（body 带 `expectedSpecVersion` 乐观锁，校验响应 `campaignId` 一致，`CampaignCreate.tsx:58-70`）；然后 `POST /api/campaigns/:id/preview`，校验响应 `specVersion` 与本地一致否则「活动预览规格已变化」（`CampaignCreate.tsx:71-74`）。预览显示「已冻结确认身份：规格 vN · hash 前 12 位」（`CampaignCreate.tsx:155`）与提示「确认推送请在 AI 总控对话中对该活动下发推送（高风险动作由 AI 恒确认门把关）」（`CampaignCreate.tsx:165`）——**本频道不做真实推送**。校验：标题与意图必填才可预览（`CampaignCreate.tsx:31`）。
- **`campaign/CampaignBoard.tsx`（163 行）**：结果看板。7 桶指标 `BUCKETS = sent/pending/blocked/escalated/canceled/skipped/unknown`（`CampaignBoard.tsx:13`）；blocked/canceled/escalated 三桶按 reason 细分显示（经 SEND_OUTCOME_REASON_LABELS）；逐人明细表（50/页分页）+ 桶过滤 chip + CSV 导出。空态（未选活动）：「暂无活动结果 / 在 AI 总控下发活动推送后，点『查看推送结果』进入这里…」（`CampaignBoard.tsx:49-58`）。加载守卫：`lastAttemptedId` 防失败后无限重拉（`CampaignBoard.tsx:35-40`）。
- **`campaign/CampaignList.tsx`（87 行）**：活动表格；状态闭集 6 值 draft/previewed/confirmed/dispatching/completed/canceled → 中文（`CampaignList.tsx:20-30`）；列头 title 澄清「已下发=跟进任务数非真实送达」「命中数=圈人命中」（`CampaignList.tsx:64-65`）。行点击 openReport。
- **`campaign/ProductMultiSelect.tsx`（41 行）**：`GET /api/products?active_only=true` checkbox 组；失败显「产品选项加载失败」、空显「暂无可选产品」。`StageSelect.tsx`（34 行）：`GET /api/admin/taxonomies?kind=customer_stage` 下拉（字典驱动，wire 形状 `items[].value.{id,label}`）。
- **`campaign/buckets.ts`（32 行）**：桶→tone/中文标签；`bucketCount` 兼容数值与按 reason 细分的对象（求和，`buckets.ts:25-32`）。
- **`campaign/csv.ts`（21 行）**：CSV 导出带**公式注入防护**——首个非空白字符是 `=+-@` 时前缀 `'`，控制字符替换为空格，含逗号/引号/换行才加引号（`csv.ts:5-13`）；BOM 由 CampaignBoard 下载时加（`CampaignBoard.tsx:17`）。
- **调用端点**：`GET /api/campaigns`、`POST /api/campaigns`、`PATCH /api/campaigns/:id`、`POST /api/campaigns/:id/preview`、`GET /api/campaigns/:id/sends`（report）、`GET /api/products?active_only=true`、`GET /api/admin/taxonomies?kind=customer_stage`。真实 dispatch 走 AI 总控的 `wechatagent.dispatch_campaign` 工具（见 2.1）。
- **与 store 分工**：campaignStore（121 行）持有列表/report/视图/分页；loadReport 带 generation+selectedCampaignId+响应 campaignId 三重守卫（`campaignStore.ts:86-94`）。

### 2.10 products-deals（产品与成交）— 1 文件

**结构**：4 个 tab（catalog 产品目录 / deals 成交记录 / holdings 客户持有 / review 疑似成交待核实，`index.tsx:11,137-175`），无专属 store（各 tab 本地 state + 直接 api）。

- **金额约定（贯穿全 tab）**：后端金额是**最小币种单位整数（分）**；展示 `fmtPrice` ÷100（`index.tsx:90-96`）、录入 `yuanToCents` ×100 + `Math.round` 防浮点误差（1.1*100=110.00000000000001，`index.tsx:98-107`）。
- **CatalogTab**（`index.tsx:177-350`）：产品列表 + 录入表单（productId 自定唯一/name/price/currency/sku/summary）；归档⇄恢复 `POST /api/products/:id/{archive|restore}`；校验 productId+name 必填（`index.tsx:199,342`）。空态：「暂无产品 / …AI 报价以此为准。无产品行业可留空。」（`index.tsx:247-250`）。
- **ContactPicker**（导出组件，`index.tsx:352-433`）：好友选择面板（`GET /api/contacts?limit=100&accountId=`），账号切换 render 期间递增 generation + effect 清空重拉（`index.tsx:362-393`）。
- **DealsTab**（`index.tsx:457-814`）：登记成交/退款表单 + 成交事件列表（`GET /api/contacts/:id/outcome-events`）。**表单校验**：reversal 必须选关联产品（呼应后端 400，`index.tsx:568-572`）；金额须有效非负数字（`yuanToCents` 单一入口防「纯空格静默落 0 元」，`index.tsx:573-583`）；quantity 归一 ≥1 整数（`index.tsx:592-593`）。提交 `POST /api/contacts/:id/deal-events`（body expectedAccountId/eventKind deal|reversal/verification staff_confirmed|payment_verified/productId?/quantity?/amount 分/currency?/occurredAtMs?/note?，只传非空字段，`index.tsx:584-601`）。产品下拉：deal 只列 active、reversal 放宽全部 status（`index.tsx:550-553`）；选产品自动带出币种（`index.tsx:674-682`）。verification 徽标文案表 `VERIFICATION_LABEL`（conversation_inferred「疑似成交·待核实」/staff_confirmed「已核实」/payment_verified「支付核实」——注释明言「一律用 AI 中性词，规避 CI 命名红线 lint 的禁词集」，`index.tsx:61-66`）。scope/generation 守卫遍布提交与加载（`index.tsx:467-478,488-512,606-630`）。
- **HoldingsTab**（`index.tsx:816-877`）：`GET /api/contacts/:id/entitlements`；行显 quantity/ownedSince/expiresAt/inAftercare 三态徽标（true「售后/有效期内」/false「有效期已过」/null 不显）。空态强调「已核实成交派生的持有记录（疑似线索不计入）」（`index.tsx:853-856`）。
- **SuspectedDealsTab**（F23 方案B，`index.tsx:897-1100`）：`GET /api/admin/suspected-deals?status=pending`；注释明确红线「AI 判断疑似成交只产弱信号沉到待核实队列，**绝不直接落成交**；通过则后端落 verification=staff_confirmed 正式成交」（`index.tsx:891-896`）。每条独立金额/币种草稿（通过时可选提交）；驳回原因内嵌输入替代 window.prompt（`index.tsx:904-907`）、原因必填（`index.tsx:950-955`）。端点与 ask-human 的 SuspectedDealReviewCard 相同：`POST /api/admin/suspected-deals/:id/{approve|reject}`。

### 2.11 knowledge（知识库 Wiki）— 19 文件（13 个顶层 + cockpit/ 6 个），10711 行

**三级 IA**（`knowledge/index.tsx:41-72`）：一级 = 3 模式（workbench 工作台「今日待办与起草」/ library 知识库「问答、浏览与治理」/ console 控制台「录入、Schema 与系统」）；二级 = 各模式左侧 nav pane；三级 = library/quality 下的子 tab（lint 质量信号 / review 待评审 / autoVerify 批量校验，`index.tsx:69,262-293`）。console 的「高级」分组默认折叠（observability/tryRecall/metrics/memory/graph 五个诊断面板，`index.tsx:317-349`）。根组件包 Confirm/Toast/FormDialog 三 Provider（`index.tsx:389-399`）。

**跨模式交互（顶层状态提升，`index.tsx:48-51,88-125`）**：
- B1 `wikiFocusChunk` window 事件（`shared.tsx:518-521` 的 `focusChunk()` 发布）：全局唯一监听在 index，console 模式收到时先切 library 再下发 focusedId（杜绝"死跳转"）；Inspector 挂 workbench/library 两模式第三栏。
- B2 收件箱「找 AI 协作」→ 切 workbench/chat 预填 attachChunkId（`index.tsx:113-117`）。
- B8 概览 CoverageVerdict 维度下钻 → 切 library/quality/review 带 `initialDimFilter`（`index.tsx:120-125`）。
- 另有 `wikiOpenCockpit`（导入完成页「去治理总览」`steward.tsx:1090`）、`wikiTrackTask`（派工成功广播给 TaskRail 自动跟踪，`today.tsx:360,1270-1284`）、`wikiChunksInvalidated`（`chunkInvalidation.ts:3`，见下）。

**实时失效机制**：`chunkInvalidation.ts`（43 行）—— `invalidateChunks(detail)` 发 `wikiChunksInvalidated`；WS 的 revised/lagged 与本地 mutation（reason:"local"）都走它（`App.tsx:51-65`；`shared.tsx:160,395-408`）。`useCoalescedReload`（`chunkInvalidation.ts:21-43`）：请求进行中收到的失效合并为一次尾随 reload，防爆发式并发列表请求。消费者：ReviewView/KnowledgeTreeView/Inspector 都监听并在 lagged 时显「实时更新有积压，正在重新同步…」（`steward.tsx:1642-1652`、`explore.tsx:437-449`、`shared.tsx:168-179`）。

**workbench 模式**（`today.tsx`，1441 行）：
- `DigestCanvas`（`today.tsx:767-1066`）：今日摘要卡片网格。`GET /api/knowledge/digest/today?accountId=`；`POST /api/knowledge/digest/regenerate`（force:true）；`POST /api/knowledge/digest/cards/:cardId/dismiss?accountId=`。**批量派工快照绑定**：勾选卡片（freeform 卡不可勾）→ 组 `DigestSelectionBinding`（reportId/reportDate/reportGeneration/reportHash/selectedCards[{cardId,cardHash}]）→ `POST /api/knowledge/chat/tasks`；缺 reportHash/cardHash 拒绝派工「当前日报缺少服务端快照绑定」（`today.tsx:795-814`）。sessionId 必须走 `randomUuid()`（裸 crypto.randomUUID 在 HTTP+IP 非安全上下文是 undefined，`today.tsx:818-820`）。`digestGeneratedClock` 只做字符串裁剪不 new Date（后端时间串非法 ISO8601，Safari 会 Invalid Date，`today.tsx:703-715`）；`digestTargetRefLabels` 渲染卡片指向的 chunk 短 id（cardId 由 target_refs 派生，两张同名卡必指向不同切片——不渲染运营无从分辨，`today.tsx:728-765,1021-1024`）；metric 阈值 0 不渲染（语义是「有就算问题」，`today.tsx:1039-1044`）。错误横幅 onRetry 是「重新加载」非「AI 重试」（防误解为重发派工，`today.tsx:970-979`）；`latestAttemptStatus != ok` 时显「最近重算仍在进行/未成功…当前继续展示上次成功结果」（`today.tsx:981-987`）。
- `ChatWorkbench`（`today.tsx:97-506`）：AI 协作起草。session 按账号 localStorage 键控 `knowledgeChat.sessionId.<accountId>`（含旧全局键迁移，`today.tsx:35-37,119-137`）。`POST /api/operation-knowledge/chat`（body content/sessionId?/accountId/attachments[{chunkId}]）；历史 `GET /api/operation-knowledge/chat/:sessionId?accountId=`；SSE `GET /api/knowledge/chat/sessions/:sid/stream`（turn 事件触发重拉历史，经 `lib/useSseReconnect` 的 createSseReconnector）；`POST …/chat/:sid/apply`（应用为草稿，成功 focusChunk 新 chunk）；`POST …/chat/:sid/discard`（confirm 弹窗）。**派工确认流**：响应带 plannedSteps 时须同时有 digestSelection+candidateHash 才渲「待确认派工」（缺快照绑定报错，`today.tsx:245-257`）；确认 `POST /api/knowledge/chat/tasks`（body 含 candidateHash/sourceTurnIndex，`today.tsx:336-369`）。
- `KnowledgeInbox`（`today.tsx:526-662`）：`GET /api/operation-knowledge/inbox?priority=`；卡片按 suggestedActions 渲按钮（open_chat→找 AI 协作 / 查看知识 / open_repair→去修复 / dismiss→本地乐观隐藏 + toast「已暂时忽略（刷新后恢复）」——**后端暂无逐条 dismiss 接口，不发死请求**，`today.tsx:570-574`）。
- `TaskRail`（`today.tsx:1098-1441`）：派工跟踪侧栏。`GET /api/knowledge/chat/tasks?accountId=`（列表）、`GET …/tasks/:taskId`（快照）、`POST …/tasks/:taskId/cancel`；SSE 流断线 → `createSseReconnector` 自动重连（onReconnecting 显示第 N 次），放弃后 → 5s 间隔兜底轮询 12 次封顶（`today.tsx:1094-1096,1204-1233`），超限提示「请点击拉取获取最新状态」。左栏空态刻意不用共享 EmptyState（200px 窄栏会挤成四行，`today.tsx:1431-1437`）。

**library 模式**：
- `AskView`（`explore.tsx:48-361`）：知识问答。默认 SSE 实时模式 `GET /api/knowledge/ask/stream?query=`（trace/token/answer/failed/close 五类事件；token 增量渲流式回答；`resultRef` 同步跟踪防 stale closure 误报错，`explore.tsx:59-64`）；非实时 `POST /api/knowledge/ask`。中断=关 EventSource（后端 SSE body drop → 取消信号，`explore.tsx:197-206`）。结果区：answer + 引用 chunk 卡片（点击 focusChunk + 展开显 source_quote；无引用显「该引用未配原文引用；请在评审视图补齐」）+ 工具调用时间线（实时默认展开）。
- `KnowledgeTreeView`（`explore.tsx:388-718`）：纯客户端 3 级树聚合（wikiType 9 类 → businessTopics[0] → title，`explore.tsx:451-463`）；右侧 ChunkDetail 显 source_quote 黄边块/锚点（点击**复制 anchor JSON**——走 `lib/clipboard` 的 copyText 而非裸 navigator.clipboard，HTTP 环境下后者 undefined，`explore.tsx:507-515`）/关联 chip（dead 目标禁用）/正文折叠。只读——「确认/退回请去待评审」。
- `LintView`（`steward.tsx:1341-1539`）：质量信号。`GET /api/knowledge/gap-signals?status=pending&limit=300`；`POST /api/knowledge/gap-signals/sweep`（结果文案「新增 N 个问题，自动处理 M 个」）；`POST /api/knowledge/gap-signals/:id/{dismiss|apply}`。12 类 kind 过滤树 `GAP_SIGNAL_KIND_ORDER`（`steward.tsx:1331-1335`，注释注明 citation_format_rejected 与 recall_miss 刻意分列——前者修复方向是重锚定、后者是补录）。
- `ReviewView`（`steward.tsx:1580-1873`）：评审队列。`GET /api/operation-knowledge/review-queue[?dimension=]`（服务端投影：items+counts+effectiveFilter；分类是 facet 非互斥，`steward.tsx:1541-1557`）。5 分类：contested 已退回/needs_review 待初审/source_orphan 缺少来源/pending_verification 待确认/dependents_pending 关联不完整。批量：`POST /api/operation-knowledge/chunks/batch-verify`（items 带 **expectedUpdatedAt** 乐观锁 + note）/`batch-archive`（ids+reason，confirm 弹窗）；结果显「成功 N，跳过 M（id:原因…）」（`steward.tsx:1668-1724`）。行内嵌共享 `ChunkReviewCard`（传入 pre-fetch 的整行消除 N+1，`steward.tsx:1838-1841`）+「审核 / 对话」打开 ReviewChat 双栏。
- `ReviewChat`（`cockpit/ReviewChat.tsx`，410 行）：审核+对话双栏。左栏裁决：`canGoLive` 前端镜像 D2 闸（source_quote+source_anchors 双非空才可放行，`trustTypes.ts:175-187`）；「让 AI 可以用这条」走 `useGoLive.runGoLive`（`cockpit/useGoLive.ts:13-55`——有 session 先 `POST /chat/:sid/apply` 拿**新 updatedAt** 再 `POST /chunks/:id/verify`（body expectedUpdatedAt），不能拿 apply 前快照核验）；退回 `POST /chunks/:id/reject`。product_fact 类显「放行后会成为 AI 对客的依据」警示（`ReviewChat.tsx:249-254`）。右栏对话：`POST /api/operation-knowledge/chat` 带 `attachments:[{chunkId, expectedUpdatedAt, operation:"update"}]`；响应校验 `targetChunkId+expectedUpdatedAt` 与快照一致才显示 patch（否则「AI 返回的知识目标或版本已变化」，`ReviewChat.tsx:185-195`）；patch 字段中文化（PATCH_FIELD_LABELS 双形态归一）。「更多信息」折叠区：30 天用量/召回置信度/distortionRisks/lockedFields 🔒/有效期。
- `AutoVerifyPanel`（`cockpit/AutoVerifyPanel.tsx`，184 行）：批量校验。`POST /api/operation-knowledge/auto-verify`（body confidenceThreshold 按松紧 5/7/9、humanAuditSampleRate **勾选 0.3 / 取消仍 0.05 硬下限**——「红线姿态不可被关掉，产品声明类已全量强制人审」`AutoVerifyPanel.tsx:55-58`、limit 50/100/500）。结果三堆：AI 觉得没问题/留给你复查/AI 没把握没动。文案定调「是我让 AI 帮我筛，不是 AI 替我做主」（`AutoVerifyPanel.tsx:83-92`）。
- `ChunkRevisionsDrawer`（`steward.tsx:3253-3342`）：按 ChunkPicker 选 chunk 查 `GET /chunks/:id/revisions?limit=100`，逐条展开 patch/beforeHash/afterHash。

**console 模式**：
- `CockpitView`（`cockpit/CockpitView.tsx`，134 行）：概览。并发拉 completeness/integrity-report/gap-signals(pending)；**gaps 失败 null 与空数组区分**——失败卡片显「—」防把加载失败伪装成零待办（`CockpitView.tsx:39-46`）。三卡治理待办（待审草稿/缺原文出处/知识缺口）点击下钻 review。`AnsweringModeGauge`（47 行）：三档 relationship_only/product_safe/fully_supported 恒定 1/2/3 档深度、标签随 active profile（`AnsweringModeGauge.tsx:13-18`）；有待审草稿时读数「只要还有草稿，就绝不宣称完全支撑」。`CoverageVerdict`（62 行）：维度裁决四态 verified「可放心讲」/draft「待你审」/missing「空白·高风险」/methodology「只能讲思路」；effectClaims 缺失特别文案「AI 一旦对客讲成功率/见效/回款，会被安全闸当场拦下」（`CoverageVerdict.tsx:24-37`）。
- `DocumentsView`（`steward.tsx:78-595`）：文档 CRUD。`GET/POST /api/operation-knowledge/documents`、`GET /documents/:id`（冻结 version 后 `PATCH /documents/:id` 带 version 只提交变更字段，`steward.tsx:201-234`）、`DELETE /documents/:id`（=归档，confirm）、`GET /documents/:id/chunks`（行内懒加载+缓存）。**手工新建切片红线**（`steward.tsx:242-253` 注释）：`POST /api/operation-knowledge/chunks` 必须写死 `status:"draft" + integrityStatus:"needs_review"`（后端 create 缺省落 active 且门闸不管 status）——手工新建也先进待审池。E4 批量修复入口：文档下有 needs_review 切片才显示，打开 `DocumentRepairPanel`。
- `ImportWizard`（`steward.tsx:642-1100`）：三步 粘贴→预览→应用。`POST /api/operation-knowledge/import-preview`（大文档返回 jobId → 每 2s 轮询 `GET /import-preview-job/:jobId`；进向导时 `GET /import-preview-jobs?status=running` 跨会话恢复，`steward.tsx:659-679`）；预览必须带 previewId+previewHash 否则拒绝（`steward.tsx:720-724`）；候选可编辑+勾选+「AI 重新分类」`POST /extract-tags`；应用 `POST /import-apply`（body previewId/previewHash/chunks[{candidateId,patch}]）。PDF `POST /import-apply-pdf`（multipart）、图片 `POST /import-apply-image`（FileReader 异步 base64 防大图卡死，`steward.tsx:968-976`）——两者上传前 confirm 知情弹窗「不经逐条预览，结果均为草稿需在待评审确认」。完成页文案：「这些都是草稿,AI 还**不能**拿去跟客户说」（`steward.tsx:1076-1078`）。
- `IngestSourcesView`（`steward.tsx:2148-2362`）：外部源。`GET/POST /api/knowledge/ingest-sources`、`PATCH /ingest-sources/:id`（重新激活 status:"active"）、`DELETE /ingest-sources/:id`（confirm「已抓取入库的知识不会被回收」）。说明文案：「自动保存为待确认草稿（AI 不会自动确认）。连续 3 次抓取失败标记连续失败，7 天无法访问自动停用」（`steward.tsx:2234-2237`）。
- `DomainSchemaTab`（`atlas.tsx:486-702`）+ `DomainSchemaEditor`（197 行）：行业 Schema。`GET/POST /api/admin/domain-schemas`、`PUT /domain-schemas/:schemaId`（body expectedVersion；编辑=创建新版本未激活）、`POST /domain-schemas/:id/activate?expectedVersion=`（confirm「正在进行的会话会立即生效」）、`DELETE /domain-schemas/:id?expectedVersion=`（使用中禁删）。编辑器：字段行 name/label/kind(string|enum|number|date|reference)/required/allowedValues(enum 时)/aliasOf；同义词 `别名=字段名` 行文本。展示层把 aliasDict 渲成「客户说『X』→ 记到『Y』」（`atlas.tsx:683-694`）。
- `AdminGovernanceView`（`atlas.tsx:1083-1417`）：系统配置 4 子 tab——meta（`MetadataDashboard`：`GET /api/operation-knowledge/metadata`，wikiType 分布/verified 占比/编辑者/7 天活跃条形图）、taxonomies（`GET /api/admin/taxonomies[?includeAllVersions=true]`）、policies（`GET /api/admin/operation-state-policies`）、domains（`GET /api/operation-domains`）。三资源共用 `PublishBar`（`atlas.tsx:986-1070`）：`POST /api/admin/{taxonomies|operation-state-policies|operation-domains}/:id/{publish|rollout|rollback}`；rollout 需**逐字输入「确认发布」**（requireText，`atlas.tsx:1006-1013`）。
- 高级诊断：`ObservabilityDashboard`（`steward.tsx:2364-2706`）：10 端点 Promise.allSettled + 逐端点 `safe()` 隔离（任一失败只缺对应卡片，报「N 项诊断数据加载失败，其余正常显示」，`steward.tsx:2401-2438`）——catalog/persisted、catalog、completeness、integrity-report、logs/analyze、knowledge/metrics、admin/observability/phase-rollup、admin/observability/worker-health、behavior-signal-metrics?limit=14、admin/observability/performance?hours=24。`PhaseRollupPanel`（`steward.tsx:2708-2914`）：运行终态/改写原因/审核员误判/负例候选/审核员双脑表现/请示通道（待领导裁决/超 24h/投递失败）/成交追认命中；每卡带 `MetricScopeTag` 口径标签（flow_window/current_inventory/retained_history/rolling_window_cache/mixed，`steward.tsx:2104-2126`）；「超出合法集」标红。`WorkerHealthPanel`（`steward.tsx:2942-3177`）：chat-tasks 状态/gap-signals 历史/发送后投影（积压/最老积压>5min 标红/P95 三项/旧画像跳过）/经验沉淀 14d。`TestMatchPanel`：`POST /api/operation-knowledge/test-match`。`TryRecallView`（`steward.tsx:1136-1309`）：`POST /tools/search` → `POST /tools/open-slice` 两段诊断，透出 riskLevel/覆盖度/需要类别/缺失知识/选中原因/toolTrace/证据摘录。`MetricsTab`（`atlas.tsx:709-779`）：`GET /api/knowledge/metrics` answer cache 命中率。`MemoryDrawer`（`atlas.tsx:1440-1566`）：`GET /api/knowledge/operator-memory?kind=&limit=100`、`POST /operator-memory/:id/revoke`（body accountId/operatorId/reason，confirm）。`ChunkGraphView`（`atlas.tsx:38-463`）：SVG 关系图谱 0 依赖——polar 确定性布局（FNV-1a 哈希角度 + 入度收缩半径）/force 力导向 200 步退火（k_spring=0.06/rest_len=80/k_repel=1400，`atlas.tsx:182-256`）；边样式按 kind（contradicts 红虚线等）；社区染色=并查集连通分量 HSL 等距（`atlas.tsx:126-156`）。

**Inspector 与 chunk 操作**（`shared.tsx`，1136 行）：
- `ChunkInspectorPane`（`shared.tsx:87-415`）：第三栏详情。拉全量 `GET /api/operation-knowledge/chunks` 建 indexById（模块级 `loadChunkOptions` 缓存 20s 供 ChunkPicker 复用，`shared.tsx:41-57`）。渲染：supersededBy 跳转条 / 状态+编号+类型+主题+上一版本+provenance / source_quote 黄边块（无则「无原文引用 — 该知识片段不可核验」）/ 锚点 / 关联（可解除 `DELETE /chunks/:id/relate/:target_id`，confirm）/ 正文 / needs_review 时内嵌 ChunkRepairPanel / ChunkActionsBar / 被引用 / 原文 / 修订时间轴。
- **P1-4 协作 presence**（`shared.tsx:523-686`）：`useChunkInspectorLock` 状态机 idle/self/other/error；`POST /chunks/:id/lock`（409 带 payload → other；60s 心跳续期；unmount `DELETE /chunks/:id/lock` best-effort）；WS `wikiChunkLocked/Unlocked` 事件刷新。徽标文案强调「仅提示，不阻止提交」「协作提示不可用…不影响提交」——真正并发保护由后端事务+CAS（`shared.tsx:729`）。
- `ChunkActionsBar`（`shared.tsx:692-912`）9 动作：确认放行 `POST /chunks/:id/verify`（body **expectedUpdatedAt**，缺失报「缺少版本信息」）/退回 `reject`（reason 必填 FormDialog）/改摘要 `patch`/归档 `archive`（confirm）/恢复 `restore`（仅 archived）/拆分 `split`（校验正整数切点，`shared.tsx:778-781`）/合并 `merge`（chunkRef 选择器）/关联 `relate`（6 关系类型下拉）。请求体构造集中在 `chunkActionContracts.ts`（27 行）。提示：「AI 起草的知识强制为草稿、待确认；只有管理员能手动确认放行」（`shared.tsx:907-909`）。
- `ChunkReferrersList`：`GET /chunks/referrers?target_id=`（懒加载折叠）。`ChunkSourceSection`：`GET /chunks/:id/source`（父文档 rawContent 截 8KB 防撑爆 DOM，`shared.tsx:450-453`）。`ChunkRevisionsTimeline`：`GET /chunks/:id/revisions` + 回滚 `POST /chunks/:id/rollback/:revisionId`（confirm）。
- `ChunkRepairPanel`（163 行）：AI 修复闭环。`POST /chunks/:id/repair`（propose）→ 勾选字段（默认全勾，排除 extras）→ 可答追问 `POST /chunks/:id/repair/answer`（body sessionId/previousPatch/answers/turn）→ 落库走 `lib/applyAiRepairPatch`（AI 永不自动核验，落 draft+needs_review）。`DocumentRepairPanel`（98 行）：聚合文档下 needs_review 切片逐个复用 ChunkRepairPanel，修完 invalidateChunks + onRepaired 重拉父列表。
- `labels.ts`（463 行）：知识频道统一翻译层——只翻译机器枚举不碰业务句子（`labels.ts:1-5`）。约 25 张字典表全部「未知回落原值」。要点：DIGEST_CARD_KIND_LABELS 注释记录曾与后端**零重叠**的教训（`labels.ts:200-217`）；DIGEST_METRIC_NAME_LABELS 是非闭集尽力翻译、camelCase/snake_case 归一（`labels.ts:237-277`）；DIGEST_TARGET_REF_KIND_LABELS 记录 prompts.rs 与 models.rs 两处枚举口径不一致、取并集（`labels.ts:279-291`）——已知问题见第 5 节。
- `trustTypes.ts`（204 行）：completeness/integrity 响应解析（防御式 parse，`trustTypes.ts:69-134`）；`canGoLive` D2 闸镜像；ChunkType 4 值（product_fact/style_template/negative_example/peer_case——「怎么用」与 wikiType「是什么」正交，`trustTypes.ts:139-146`）；ChunkRepairProposal 类型。

### 2.12 system-strategy（系统策略）— 1 文件（2831 行）+ strategyStore

**结构**：4 个 tab `control 总控与 Prompt / taxonomy 标签与状态 / profile 行业配置 / lessons 经验教训`（`index.tsx:2692-2699`）。根组件包 Confirm/Toast Provider（`index.tsx:2651-2661`）。

- **control tab**：三卡说明 + 「重置系统提示词包 v2」（confirm 要求**逐字输入 `RESET PROMPT PACK`**，`index.tsx:32,2721-2736`；`POST /api/prompt-templates/reset-system-pack` body {confirmation}）+ `DomainPromptPanel`（本文件私有版本，agentKinds=["management","methodology"]，与 user-ops legacy 版结构对齐但 CSS Module 化，`index.tsx:289-586`）。
- **prompt 三态保存/发布流**（本频道核心机制，`stores/strategyStore.ts:6-15,218-285`）：`PUT /api/prompt-templates/:id` 与 `POST /api/prompt-templates/:id/publish` 都可能返回三态——200 ok / 200 `{status:"needs_human_confirm", reason, diff}`（**不能当成功 reload**）/ 4xx message 含「红线语义审查拒绝」（rejected）。store 翻译成结构化 `SavePromptResult` 交组件层，由 `usePromptSaveConfirm/usePromptPublishConfirm` hook 弹「逐字核对 + force 覆盖」二次确认再带 `force:true` 重提（`strategyStore.ts:84-95` promptPayload 注释：force=管理者已逐字核对，覆盖 LLM 红线语义审查但字面双闸仍跑）。**PUT 追加不可变草稿**：保存成功必须把 editingId 切到返回的新 id，否则后续发布会误指旧版本（`strategyStore.ts:175-176,240-242`）。soul 同理（`strategyStore.ts:159-184`）。
- **taxonomy tab** 三面板：
  - `StatePolicyAdmin`（`index.tsx:589-674`）：`GET /api/admin/operation-state-policies?includeAllVersions=`；每状态显 allowed/forbidden 动作（经 contracts 的 OPERATION_STATE_ACTION_LABELS，未知显「未知动作（x）」`index.tsx:45-47`）+ ActiveVersionsBar（`POST /api/admin/operation-state-policies/:id/{publish|rollout|rollback}`，window.confirm）。
  - `TaxonomiesAdmin`（`index.tsx:677-1025`）：字典 CRUD。`GET /api/admin/taxonomies?includeAllVersions=&includeDeprecated=`；新增 `POST /api/admin/taxonomies`（用 `postRaw` 处理 409「该字典条目已存在」，`index.tsx:733-748`）；编辑 `PATCH /api/admin/taxonomies/:id`（**基线 diff：只提交变更字段**，无改动提示「没有改动。」，`index.tsx:763-806`）；废弃 `DELETE /api/admin/taxonomies/:id`、恢复 `PATCH {deprecated:false}`。校验：scope/kind/id/label 必填（`index.tsx:724-727`）；kind=customer_stage 时提示需同步配置状态机 state（`index.tsx:882-886`）。每页 20 分页（`usePagedList`，`index.tsx:155-166`）。
  - `TaxonomyCandidatesAdmin`（`index.tsx:1059-1268`）：新词候选审核。`GET /api/admin/taxonomy-candidates?status=&kind=`；pending 行嵌共享 `TaxonomyCandidateReviewCard`；**批量驳回**：勾选 + 原因必填 + confirm → 逐条 `POST /api/admin/taxonomy-candidates/:id/reject`（循环调用统计成功/失败，`index.tsx:1103-1131`）；翻页清空勾选（`index.tsx:1258-1265`）。
- **profile tab** `DomainProfilePanel`（`index.tsx:2277-2494`）：行业配置管理。子 tab list/generate。generate：`POST /api/admin/domain-profiles/generate`（body businessDescription/profileId/displayName?；成功提示「候选配置已生成…同时生成了取值字典候选…需在新词候选审核逐条采纳」，`index.tsx:2388-2394`）。`ProfileEditor`（`index.tsx:1310-2275`）是**全系统最大表单**：基本信息 + 10 个 details 折叠区——维度配置 / 承诺标记词（product_effect 绝对化效果承诺 vs tone_only 语气夸大）/ 方法论生成器引导语 / **五闸阈值覆盖**（factRiskBlockAt 等 5 项，留空=沿用销售域默认 6/7/6/6/7，「不改闸的语义与结构红线」，`index.tsx:1544-1607`）/ 人格·方法论本体覆盖（「不放宽边界保护红线」）/ 自学习极性（正/负极 outcome 词集；「沉默/未分类一律删失（绝不臆测为负）」`index.tsx:1649-1651`）/ 完整度审计维度（key/display_name/required/review_topic_aliases/anchor_hint/initial_signal）/ 经营公式 / 知识条目用途角色（chunk_roles 替代写死销售四态）/ 记忆维度（memoryCard.extra 槽位，留空回落销售八槽）/ 运营范式（funnel/silence/commitment 三驱动力开关）/ 高级发布危险字段（transaction_facts_enabled/reviewer_orientation/mode_gate_policy_override/debounce_window_ms_override——「改动经发布确认流 riskyFields 二次确认」）/ 按关系类型 per_relationship_operation_mode（customer/peer/friend 各配范式）/ 领域标志位（stagnation_dimension/grounding_gate_bypass_without_claim/distrust_self_reported_low_risk）。阈值输入语义：空串删 key、整对象空则 undefined（DEFAULT 零扰动不发空对象，`index.tsx:1329-1342`）。
  - store 端 create/update 的 wire 差异被注释钉死：create `POST /api/admin/domain-profiles` 必须补顶层 camelCase `profileId`（后端 UpsertRequest rename），update `PUT /:id` 直接发 snake_case draft（`strategyStore.ts:423-455`）；发布 `POST /:id/publish` 只移动 published 指针、返回 riskyFields，**必须再显式 `POST /:id/activate`** 才生效（`strategyStore.ts:457-491`）；发布/激活按钮在共享 `ProfilePublishCard`（见 §3）。生效中 profile 也渲染 ActiveVersionsBar（`POST /api/admin/domain-profiles/:id/{publish|rollout|rollback}`，`index.tsx:2333-2347`）。
- **lessons tab** `LessonsLearnedAdmin`（`index.tsx:2496-2649`）：`GET /api/admin/lessons-learned[?patternKind=]`；三模式 success/reviewer_misjudge_negative/blocked_by_safety_guard；「晋升为同行案例」打开共享 `LessonPromoteCard`（面板说明：抽象为 chunk_type=peer_case 候选 chunk，**仍走知识审核队列二次确认才能 verify**，`index.tsx:2572-2575`）。

### 2.13 llm-providers（AI 模型配置）— 1 文件（811 行）

**结构**：列表卡 + 编辑面板（draft 非空时渲染），无 store（本地 state）。
- 协议**中性化**：`ProtocolFormat = "chat" | "messages"`，wire 值同名，UI 零品牌字面量（「兼容 Chat Completions/Messages 协议形态的服务商或自建网关」，`index.tsx:61-94`）。baseUrl 提示按协议区分：messages 填根域不带 /v1；chat 直接贴服务商「OpenAI 兼容 base_url」原文（`index.tsx:640-652`）。
- **激活配置修改的强制测试门**：编辑 isActive 的配置时保存按钮 disabled 直到测试通过（`index.tsx:768`）；`POST /api/admin/llm-providers/test` 成功返回 `activeUpdateApproval {token, expectedUpdatedAt, expiresAt}`；保存时校验 `draft.updatedAt === approval.expectedUpdatedAt` 否则「必须先对这份草稿完成连通性测试」+ window.confirm「保存后将立即热切换全部生产对话」+ body 带 expectedUpdatedAt/activeUpdateConfirmed/activeUpdateTestToken 三件套（`index.tsx:248-259`）；**任何字段修改（`changeDraft`）都作废已获批的测试**（`index.tsx:192-199`）。
- apiKey 语义：编辑态显 mask「****」，保留 mask 占位=不更新；测试时含 mask 则从 body 删掉 apiKey（`index.tsx:346-350,677-679`）。可选数值三态：空+编辑态→null（清除覆盖回落全局默认）、有值→floor、新建空→不发（`index.tsx:211-228`）。
- 删除守卫：isActive 或 isVisionActive 不可删（`index.tsx:274-282`）；激活 `POST …/:providerId/activate`（confirm「立即对所有生产对话生效」）；**视觉模型指派** `POST …/:providerId/vision`（body {active}；要求 supportsVision=true；「当前视觉指派将被原子替换」；取消勾选 supportsVision 时若仍是视觉模型则阻止，`index.tsx:311-335,730-736`）。超时/重试行显示 effective 值 + 来源（Provider 覆盖/全局默认，`index.tsx:462-469`）。
- 端点：`GET/POST /api/admin/llm-providers`、`PUT/DELETE /api/admin/llm-providers/:providerId`、`POST …/test`、`POST …/:id/activate`、`POST …/:id/vision`。

### 2.14 operations（任务日志）— 1 文件 + operationsStore

**结构**：5 tab `tasks 跟进任务 / events 运营事件 / reviews 复核记录 / runs 运行日志 / llm LLM 成本`（`index.tsx:244-250`）。外壳 `key={currentAccountId}` 切账号重挂（`index.tsx:206-209`）。被 user-ops 传统模式 audit tab 整体内嵌复用。
- **store**（`operationsStore.ts:89-146`）：`loadOperationsData` 五连拉 `GET /api/events?accountId=`、`/api/tasks?accountId=`、`/api/decision-reviews?accountId=`、`/api/llm-usage?accountId=`、`/api/agent-runs?accountId=`（Promise.all——一败全空+报错，与 user-ops 的 allSettled 策略不同）；任务混入他账号则拒绝显示（`operationsStore.ts:116-120`）。任务动作 `POST /api/agent-tasks/:id/{review-now|cancel}`（body expectedAccountId；前置 `taskActionIsCurrent` 四重校验，`operationsStore.ts:47-77`）。
- tasks：状态经 TASK_STATUS_LABELS（12 值含 outbox_enqueued「已入发件箱」，`index.tsx:123-135`）；行内「立即复核 / 取消」。
- events：时间线 + kind/status 经 reviewLabels 共享字典；`event.detail` 有内容渲 details JSON（`index.tsx:339-344`）。
- reviews：结论优先 reviewPhase（14 值 tone 映射，`index.tsx:67-90`）；未通过时 `blockedLabel` 按 finalReviewStatus→holdCategory→「拦截」优先级取标签（注释强调 labelOf 空值返回 "—" 是 truthy 不能 || 串联，`index.tsx:197-204`）；outcome 列 OUTCOME_STATUS_LABELS（9 值客户反应，`index.tsx:138-148`）。
- runs：run envelope 表 + 展开行。**C9 档位遥测**：tier_used/sufficiency 真实数据源是 `run.decision` 文档（camelCase sufficiency/missingTier）而非 gatewayResult/events.detail（`index.tsx:107-115` 注释）；missingTier none/relational/full → 「精简档已足够/需关系档/需完整知识档」，escalated=missingTier 非 none（`index.tsx:165-174`）。展开渲 6 阶段（planner/context/knowledgeRoute/decision/review/gatewayResult）通用 key-value 表（未知字段不写死，`index.tsx:176-195`）。触发来源 TRIGGER_KIND_LABELS 5 值（`index.tsx:151-157`）。
- llm：汇总卡（调用次数/总 token——**有未知 usage 时显「至少 N（M 次未知）」**，`index.tsx:440-445`）+ 明细表（状态 LLM_CALL_STATUS_LABELS 4 值闭集：success/cache_hit/failed/json_error，`index.tsx:42-48`；usageKnown=false 显「未知」）。明细截断提示「（全量 N 次，明细已截断）」（`index.tsx:461-464`）。

### 2.15 evolution（演化中心）— 2 文件 + 根部兼容 re-export

- **`evolution/index.tsx`（25 行）**：薄壳，直接委托 `EvolutionCenterTab`。`src/EvolutionCenterTab.tsx`（22 行）是根部 re-export 兼容层（保持旧单测 import 路径，`src/EvolutionCenterTab.tsx:1-3`）。
- **`EvolutionCenterTab.tsx`（528 行）**：
  - **门控层级**（`EvolutionCenterTab.tsx:238-301`）：`GET /api/evolution/runtime-flag` → `envEvolutionEnabled === false` 运维硬锁（「EVOLUTION_ENABLED=false，请联系运维」）；null 显加载/错误重试；workspace 总开关 `flag.enabled` 关闭时**仍拉历史 experiments**（「关闭态只是不再产生新实验，历史实验应可见，否则管理员误判『演化从未运行』」`EvolutionCenterTab.tsx:239-243`）+ dormant 提示条。开关保存 `PUT /api/evolution/runtime-flag`（body enabled+rolloutPercent；「开=全量」逻辑：enabled 且灰度 0 时按 100 发送，`EvolutionCenterTab.tsx:172-174`）；响应必须从 `.flag` 内层读、缺失报错（`EvolutionCenterTab.tsx:94-102,179`）。
  - 数据：`GET /api/evolution/experiments?limit=20`——**强校验 `aggregate7d.coverage.complete`**，不完整直接抛「服务端未返回完整的近 7 天统计覆盖」（`EvolutionCenterTab.tsx:247-250`）。聚合 5 卡（实验/候选/已发布/已回滚/显著性通过率）+ coverage 说明行。
  - 阈值审计：按钮触发 `GET /api/evolution/threshold-overrides/audit`（挂载期不自动拉，`EvolutionCenterTab.tsx:216-236`）；action 闭集 released/rolled_back/auto_released → 中文（`EvolutionCenterTab.tsx:119-131`）。
  - Proposal 列表（threshold 类显 `gateKey: cur → proposed`，prompt 类显 templateKey/section）→ 点击行打开共享 `ProposalReleaseCard`（发布需逐字输 RELEASE、回滚输 ROLLBACK——实现在共享卡内，见 §3）。文案注释：「CI lint 会在 PR 阻断任何回归到非 AI 自主表达的文案」（`EvolutionCenterTab.tsx:13-14`）。

### 2.16 quality（运营成效）— 2 文件

**结构**：4 tab `outcome 长期指标 / autoVerify 知识自动校验 / formula 公式遵守度 / markers 产品声明标记词`（`index.tsx:578-583`）。
- **OutcomeMetricsTab**：`GET /api/agent-outcome-metrics?accountId=&horizon=7d|30d&limit=60`；表列 回复率/对话深度/**AI暂缓澄清率**/AI 拦截率/当日运行数/当日 token；工具栏提示「显示『—』表示该窗口内无样本；不要把它当 0 解读」（`index.tsx:120-122`）。
- **AutoVerifyTab**：`POST /api/operation-knowledge/auto-verify`（body accountId/confidenceThreshold/humanAuditSampleRate/limit——与 knowledge 频道 AutoVerifyPanel 同端点但参数自由输入）。描述文案含红线：「为守住『AI 永不自动放行』红线，预审**绝不**把任何知识直接标记为已确认」（`index.tsx:191-195`）。
- **FormulaAdherenceTab**：`POST /api/user-operations/evaluations/formula-adherence`（body accountId）。摘要显 meanAdherence/degraded 原因/预算使用（tokens/LLM 调用/未评分数/未报告 usage 次数）；行状态五态 error/skipped/unscored/invalid/完成；「缺失或非法管理员金标的存量场景标 unscored，均不按 0 计入平均」（`index.tsx:282-289`）。内嵌 `EvaluationScenariosPanel`（263 行）：评测场景 CRUD——`GET/POST /api/evaluation-scenarios`、`DELETE /api/evaluation-scenarios/:id`；公式列表取自 active domain profile 的 business_formulas（回落销售四公式 DEFAULT_FORMULAS，`EvaluationScenariosPanel.tsx:22-27,47-56`）；**校验**：scenarioId+title 必填、账号必选、输入消息至少一条、每个公式的金标必填且 0-10（「缺失金标不会被解释为 0」，`EvaluationScenariosPanel.tsx:77-103,142-145`）。
- **ProductClaimMarkersTab**（`index.tsx:375-576`）：编辑 promptKey=`user.review.product_claim_markers` 的 active 模板（`GET /api/prompt-templates` 前端过滤）。**JSON 结构校验**：顶层对象、markers 数组（每项 kind/label 字符串）、whitelistPhrases 数组、whitelistWindowChars 数字（`index.tsx:417-433`）。保存走 `PUT /api/prompt-templates/:id`（layer=review_guard）+ **update 与 publish 两道独立三态闸**：各自可能 needs_human_confirm（弹「已核对」requireText 框）或红线 reject（弹强制保存/发布框）；publish 的 force 不继承 update 的 force、内联重发避免 double-PUT（`index.tsx:440-541` 注释详尽）。页面自述「预留配置，当前未启用——当前线上把关由『已验证知识背书』机制负责」（`index.tsx:545-549`）。

### 2.17 send-analytics（发送成效）— 1 文件 + store

- **`send-analytics/index.tsx`（124 行）**：总览 3 指标（总发送数/响应率/阶段推进率）+ 排行 2 tab（media 素材 / namecard 名片）。端点（`stores/sendAnalyticsStore.ts:31-59`）：`GET /api/send-ledger/overview?accountId=`、`GET /api/send-ledger/stats?kind=media|namecard&accountId=`。overview 为 null 时指标显「—」。空态：「暂无素材发送数据/暂无名片引荐数据 / AI 在私聊运营中主动发送后，这里会按发送次数排序展示各项成效。」（`index.tsx:92-96`）。

### 2.18 autonomy（自治回路监控）— 2 文件

- **`autonomy/index.tsx`（420 行）**：`AutonomyOutcomesTab` + `OutboxPanel` 上下两块。
  - 指标：`GET /api/outcomes/autonomy?accountId=&horizon=24h|7d|30d` + `GET /api/outcomes/autonomy/revisions?…&limit=50`。7 张指标卡（改写触发率/通过率/未验证产品声明拦截率/新词候选触发率/自我批判已回应率/自治模式全自动/辅助·拦截计数），每卡带 raw 分数 hint；「旧模式行单独计数，不计入任何比率」（`index.tsx:145-147,153-156`）。
  - **AI 暂缓三类细分条**：AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文（heldByAiPolicy/blockedBySafetyGuard/aiWaitingForMoreContext，`index.tsx:195-202`）——禁词红线的监控端呈现。
  - 发送链路表：入队/已送达/已取消/终态失败/**送达待核验**（deliveryUnknown）+ 四率。Planner 三段（M3/Task 77）：沉默跟进/承诺到期/阶段停滞的 tick/scanned/emitted/capped/backoff（「回退表示 AI 因拦截率过高自主收敛」，`index.tsx:288-322`）。改写记录表可展开修订前后完整摘要+自我批判。
- **`autonomy/OutboxPanel.tsx`（302 行）**：发件箱逐条只读+取消。`GET /api/admin/outbox?accountId=`；payload 四型 text/media/referralCard/**invalid**（「异常：同时绑定素材与名片」，`OutboxPanel.tsx:14-18,55-66`）。仅 pending/in_flight 可取消（`CANCELABLE_STATUSES` 与后端 outbox_status_is_user_cancelable 一致；其它状态直接藏按钮防 409，`OutboxPanel.tsx:35-37`）。取消 `POST /api/admin/outbox/:id/cancel`（body expectedAccountId + **cancelReason 后端强制非空**，写死 "admin_outbox_panel_cancel"，`OutboxPanel.tsx:10-11,203-206`）。confirm 弹窗按状态分级提示取消风险：已越过最后可取消点/曾从发送中恢复 N 次「远端可能已经收到；取消不能撤回已送达内容」/发送中/未越过边界（`cancellationRisk`，`OutboxPanel.tsx:81-92`）。in_flight+cancelRequested 显「取消请求中（等待发送结果）」（`OutboxPanel.tsx:48-53`）。账号 scope 守卫同通例（render 期 generation 递增+三处校验）。

## 3. 跨 feature 机制

### 3.1 复用评审卡组件（`components/review/`，9 文件 1612 行）与使用方

设计原则（各文件头注释反复强调）：**「零跨 feature import」中立化**——卡片只依赖 react/lib/api/ui，feature 反向从中立家 import，杜绝双份定义（Ask-Human Phase 2 Task 6-9 迁移产物）。

| 组件 | 行数 | 职责 / 端点 | 使用方 |
|---|---|---|---|
| `ReviewQueue.tsx` | 114 | 通用待办队列容器：fetchItems→列表；`RowCtx.runAction` 统一「busy→执行→toast→refetch」；**generation+acceptedIds 双守卫**——列表刷新后旧行动作被拒「待办列表已刷新，请在最新条目上重新操作」（`ReviewQueue.tsx:69-91`）；emptyText 支持 ReactNode（组件节点不再套包裹层，`ReviewQueue.tsx:95-101`） | ask-human |
| `ChunkReviewCard.tsx` | 140 | 单 chunk 核验卡。**双形状容忍**：deep-link 走 `GET /api/operation-knowledge/chunks/:id`（raw snake_case）、pre-fetched 走列表整形 camelCase，verify-gate 对两种拼写都读（`ChunkReviewCard.tsx:10-18,104-109`）；gate=hasQuote&&hasAnchor 逐字镜像 steward 不放宽；`POST /chunks/:id/{verify\|reject}`（verify 带 expectedUpdatedAt） | ask-human（knowledgeReview 富卡）、knowledge/ReviewView（pre-fetched 消 N+1） |
| `TaxonomyCandidateReviewCard.tsx` | 165 | 新词候选采纳/驳回。`POST /api/admin/taxonomy-candidates/:id/approve`（body canonicalValue{id,label,aliases,description}；`postRaw` 处理 409「已存在」与 `mergedIntoExisting`，`TaxonomyCandidateReviewCard.tsx:58-78`）、`/reject`（reason 必填）。导出 `TAXONOMY_KIND_LABELS`（6 维度中文，`TaxonomyCandidateReviewCard.tsx:9-16`） | ask-human、system-strategy/TaxonomyCandidatesAdmin |
| `ProfilePublishCard.tsx` | 194 | 行业配置发布/激活卡。`GET/POST /api/admin/domain-profiles/:id` + `/publish` + `/activate`。**发布与激活严格分离**（publish 只动 published 指针）；activate 返回 partial 时提示「核心已激活但附属同步未完成」+「重试附属同步」幂等入口（`ProfilePublishCard.tsx:5-8,93-103,186-190`）；H13 渲染 `generated_state_machine` 供激活前审阅（states/goal/advanceSignals/riskRules，「AI 不自我核验、审阅后才激活」，`ProfilePublishCard.tsx:22-24,134-174`） | ask-human（profilePublish 富卡）、system-strategy/ProfileEditor |
| `ProposalReleaseCard.tsx` | 578 | 演化候选发布/回滚卡。`GET /api/evolution/proposals/:id`；threshold 类渲数值对照+命中率、prompt 类渲双栏 diff+Critic 推理+**五闸涨跌对照表**（Δ 语义色：五闸命中率降为好/升为坏，自评率反向，`ProposalReleaseCard.tsx:396-401`）+逐样本五闸点阵 ●○·（`ProposalReleaseCard.tsx:384-390`）；`ConfirmModal` 必须逐字输入 RELEASE/ROLLBACK（`POST /api/evolution/proposals/:id/{release\|rollback}` body {confirmation}，`ProposalReleaseCard.tsx:503-539`）；发布按钮仅 eligible_for_release、回滚仅 released 可用 | ask-human（evolutionRelease 富卡）、evolution/EvolutionCenterTab |
| `LessonPromoteCard.tsx` | 112 | 教训晋升卡（**非一键**，admin 必填 title≤200/body≤4000）。无单项 GET——拉 `GET /api/admin/lessons-learned` 列表按 lessonId 过滤（`LessonPromoteCard.tsx:29-38`）；`POST /api/admin/lessons-learned/:id/promote-to-peer-case`；成功文案「已晋升为同行案例候选（**仍需在知识审核队列核验**）」 | ask-human（lessonsPromote 富卡）、system-strategy/LessonsLearnedAdmin |
| `proposalPrimitives.tsx` | 62 | 演化状态 6 值字典 + StatusBadge（data-tone 供测试断言）；formatNumber/formatPercent 薄封装 re-export 自 `lib/format`（单一真相源，`proposalPrimitives.tsx:7-13`） | ProposalReleaseCard、EvolutionCenterTab、根部 re-export |
| `proposalTypes.ts` | 140 | 演化域全类型（镜像 `src/routes/evolution.rs` schema） | 同上 |
| `evidenceMetrics.ts` | 107 | evalMetrics（**snake_case**，bson 直转不 camelCase，`evidenceMetrics.ts:1-5`）窄化读取：FIVE_GATE_KEYS/GATE_LABELS/readAggregateEvidence；PROMPT_AGG_METRIC_KEYS 白名单只对 prompt 类过滤（threshold 共享部分 key 绝不能笼统按 key 名过滤，`evidenceMetrics.ts:22-26`） | ProposalReleaseCard |

**其余跨 feature 复用件**：`components/prompt/usePromptSaveConfirm.tsx`（122 行）——prompt 保存/发布三态二次确认 hook（needsConfirm→逐字核对 diff 弹框 requireText「已核对」→force 重提；rejected→「触碰自治边界红线」弹框；`promptDiffBody` 明示核对目的「是否变相引入真人转介」，`usePromptSaveConfirm.tsx:17-43`），消费方 system-strategy、user-ops 传统模式、quality/markers（独立内联版）。`lib/useSseReconnect.ts`（84 行）——SSE 指数退避重连器（**严禁用于一次性 RPC 流如 /knowledge/ask/stream——重连会重发查询重复扣 token**，`useSseReconnect.ts:1-2`；terminal 事件收到即停）。`lib/uuid.ts`（78 行）——randomUuid 三级降级（生产 HTTP+IP 非安全上下文 crypto.randomUUID 为 undefined，`uuid.ts:3-10`）。`lib/applyAiRepairPatch.ts`（53 行）——修复落库 `POST /api/operation-knowledge/repair/applied`，**thenVerify 恒 false**（红线：落库只到 draft+needs_review，`applyAiRepairPatch.ts:3,42`）。`lib/reviewLabels.ts`（370 行）——全站状态字典单一真相源（见 §4.2）。`lib/clipboard.ts`（copyText 非安全上下文退化 execCommand）、`lib/format.ts`（formatRate/formatNumber，null→"—"）。UI 库 `components/ui/`：Avatar/ChunkRef(ChunkPicker)/ConfirmDialog(useConfirm，支持 requireText 逐字确认)/EmptyState/FormDialog/FriendPickerModal/MetricCard/Overlay/PlanStep/StatusBadge/StatusLine/Toast + tokens.css。

### 3.2 知识频道三级 IA 完整交互流

1. **一级：3 模式**（workbench/library/console，`knowledge/index.tsx:52-65`）→ **二级：nav pane**（workbench: digest/chat/inbox + TaskRail 常驻侧栏；library: ask/tree/quality/revisions；console: cockpit + 内容录入组(documents/import/ingest) + 配置组(schema/sysconfig) + 高级折叠组(observability/tryRecall/metrics/memory/graph)）→ **三级：quality 子 tab**（lint/review/autoVerify）。
2. **跨模式跳转事件桥**（全部 window CustomEvent）：
   - `wikiFocusChunk`（B1）：任何位置（问答引用卡/知识树/文档 chunk 行/质量信号/导入完成页/inbox）→ index 唯一监听 → console 态先切 library → Inspector 展开显示该 chunk（`index.tsx:88-110`）。
   - `wikiOpenCockpit`：导入完成页「去治理总览逐条处理」→ 切 console/cockpit（`steward.tsx:1090`）。
   - `wikiTrackTask`（E14）：ChatWorkbench/DigestCanvas 派工成功广播 taskId → TaskRail 自动填入并开始 SSE 跟踪（`today.tsx:360,834,1270-1284`）。
   - `wikiChunksInvalidated`：WS revised/lagged 或本地 mutation → ReviewView/TreeView/Inspector 合并重载（useCoalescedReload 防请求风暴）。
   - 回调传递（非事件）：B2 inbox「找 AI 协作」→ `openChatWith(chunkId)` 切 workbench/chat 预填附件；B8 CockpitView 维度下钻 → `openReviewForDim(dimKey)` 切 library/quality/review 带 `initialDimFilter`（服务端按 review_topic_aliases 精筛，`steward.tsx:1744-1749`）。
3. **典型闭环**（录入→审→放行）：ImportWizard 粘贴/PDF/图片 → 全部落 draft+needs_review → 完成页跳 cockpit 或 focusChunk → ReviewView 按 5 facet 分类处置（单条 ChunkReviewCard verify/reject、批量 batch-verify 带 expectedUpdatedAt、或「审核/对话」进 ReviewChat 让 AI 改完再走 useGoLive 的 apply→verify 链）→ verify 后 chunk 进入 AI 可用集合；期间 gap-signals/digest/inbox 持续产生治理待办回流 workbench。

### 3.3 统一收件箱 9 源卡片渲染分发（ask-human）

分发树（`ask-human/index.tsx:76-187,459-496`）：

```
ReviewQueue<InboxItem>（fetchItems = inboxStore.load → GET /api/admin/ask-human/inbox）
└─ InboxRow（badge=SOURCE_META 中文 + tone=SOURCE_TONE；knowledge_review 附「AI预审通过·待复核」tag）
   ├─ item.actionKind === "rich" → renderRich 按 richComponent 分派（richParams 提供 id）
   │    knowledgeReview        → ChunkReviewCard(chunkId)          [共享]
   │    profilePublish         → ProfilePublishCard(profileId)     [共享]
   │    evolutionRelease       → ProposalReleaseCard(proposalId)   [共享]
   │    lessonsPromote         → LessonPromoteCard(lessonId)       [共享]
   │    taxonomyCandidateReview→ TaxonomyCandidateReviewCard        [共享]
   │    suspectedDealReview    → SuspectedDealReviewCard            [feature 内]
   │    未知 richComponent     → 「未知 rich 组件：<name>」兜底
   └─ inline → renderInline 按 source 分派
        principal_escalation      → EscalationInline（表单：5 verdict/有效期/豁免/约束/改派）
        relationship_suggestion   → SimpleApproveReject 详情 + 行内常驻 SimpleActionButtons（approve/reject）
        gap_signal                → 同上（仅 dismiss）
        未知 source               → 渲染 item.title 兜底
```

9 源与计数键对照（SOURCE_META）：principalEscalation/principal_escalation 请示裁决、knowledgeReview/knowledge_review 知识核验、taxonomyCandidate/taxonomy_candidate 标签候选、relationshipSuggestion/relationship_suggestion 关系建议、suspectedDeal/suspected_deal 疑似成交、gapSignal/gap_signal 知识缺口、profileRisky/profile_risky 画像发布、evolutionProposal/evolution_proposal 进化发布、lessonsLearned/lessons_learned 经验晋升。summary 键 camelCase、inbox ?source= 过滤 snake_case（由后端 ask_human_inbox.rs 定义，`index.tsx:24-25`）。

### 3.4 其它跨 feature 通道

- **跨频道跳转**：`navigationStore.setChannel`——overview 统计卡→userOps；user-ops JudgmentBar 请示灯→askHuman；command-center dispatch_campaign 成功→`campaignStore.openReport(id)`（内部 setChannel("campaign")+置 view=board）。
- **store 复用跨频道**：`userOpsStore.loadRoster` 的通讯录缓存被 referral-cards（选顾问）与 ask-human-config（选决策人）复用；`inboxStore.principalEscalationCount` 被 user-ops 判断条消费；`strategyStore` 同时服务 system-strategy 与 user-ops 传统模式 prompts tab；`OperationsFeature` 整组件被 user-ops audit tab 内嵌。
- **全局 WS**：App 顶层唯一 `ws://…/api/ws/chunks` → wikiChunkLocked/Unlocked（Inspector presence 徽标）+ wikiChunkRevised/lagged → invalidateChunks（知识列表失效重载）。

## 4. 事实卡速查

### 4.1 feature → 后端端点完整映射

| feature | 端点（方法 路径） |
|---|---|
| 启动引导(App/Shell) | GET /api/accounts；POST /api/accounts/sync；GET /api/admin/domain-profiles/active；GET /api/operation/active-view；WS /api/ws/chunks |
| command-center | GET /api/content-assets[?accountId]；GET /api/agent-souls；GET /api/tasks[?accountId]；POST /api/management-agent/sessions；POST /api/management-agent/sessions/:id/messages；POST /api/management-agent/commands/:id/confirm｜/reject；PUT /api/accounts/:id/mcp-key |
| account-management | GET /api/accounts；POST /api/accounts/sync；POST /api/accounts/login/begin；GET /api/accounts/login/poll |
| overview | （仅 contactStore 的 GET /api/contacts） |
| user-ops | GET /api/contacts?accountId&limit=500[&q]；GET /api/contacts/counts；GET /api/contacts/roster[?force]；POST /api/contacts/import（经 ask-human-config 复用）；POST /api/contacts/batch-enable；POST /api/contacts/:id/hide-from-pool｜enable-agent｜disable-agent｜analyze-profile｜clear-referral｜memory-consolidation/run；PUT /api/contacts/:id/profile-note｜custom-agent-instructions｜assist-override｜operation-profile｜operating-memory｜manual-tags；GET /api/conversations/:id/messages?limit=50；GET /api/contacts/:id/operating-memory｜memory-candidates?limit=30｜operation-health；GET /api/contacts/:wxid/send-history?accountId；GET /api/decision-reviews?accountId&contactId&limit=20；POST /api/user-operations/guide/preview｜apply；POST /api/user-operations/simulations/dialogue；GET/POST /api/operation-playbooks；PUT /api/operation-playbooks/:id；POST /api/operation-playbooks/:id/optimize｜set-default；POST /api/operation-playbooks/generate；GET /api/operation-domains；PUT /api/operation-domains/:domain；POST /api/operation-domains/:domain/reset；POST /api/admin/operation-domains/:id/publish｜rollout｜rollback |
| content-assets | GET /api/content-assets?accountId[&tag]；POST /api/content-assets；POST /api/content-assets/upload；POST /api/content-assets/:id/review｜toggle｜file；PUT /api/content-assets/:id；DELETE /api/content-assets/:id?expectedScope… |
| referral-cards | GET/POST /api/referral-cards；POST /api/referral-cards/:id/review｜toggle；DELETE /api/referral-cards/:id |
| ask-human | GET /api/admin/ask-human/inbox[?source&accountId]；GET /api/admin/ask-human/summary[?accountId]；POST /api/admin/principal-escalations/:code/resolve｜reassign；GET /api/admin/principal-escalations?status=resolved；POST /api/admin/relationship-type-suggestions/:id/approve｜reject；POST /api/knowledge/gap-signals/:id/dismiss；POST /api/admin/suspected-deals/:id/approve｜reject；（富卡另见 §3.1 共享组件端点） |
| ask-human-config | GET /api/operation-domains/user_operations；PUT /api/operation-domains/user_operations/ask-human-policy；POST /api/contacts/import；GET /api/contacts/roster（经 userOpsStore） |
| campaign | GET/POST /api/campaigns；PATCH /api/campaigns/:id；POST /api/campaigns/:id/preview；GET /api/campaigns/:id/sends；GET /api/products?active_only=true；GET /api/admin/taxonomies?kind=customer_stage |
| products-deals | GET/POST /api/products；POST /api/products/:id/archive｜restore；GET /api/contacts?limit=100&accountId；GET /api/contacts/:id/outcome-events｜entitlements；POST /api/contacts/:id/deal-events；GET /api/admin/suspected-deals?status=pending；POST /api/admin/suspected-deals/:id/approve｜reject |
| knowledge | （operation-knowledge 族）GET/POST documents；GET/PATCH/DELETE documents/:id；GET documents/:id/chunks；GET chunks；POST chunks；GET chunks/:id；POST chunks/:id/verify｜reject｜patch｜archive｜restore｜split｜merge｜relate；DELETE chunks/:id/relate/:target_id；GET chunks/referrers?target_id；GET chunks/:id/revisions[?limit]；POST chunks/:id/rollback/:revisionId；GET chunks/:id/source；POST chunks/:id/lock｜DELETE lock；POST chunks/batch-verify｜batch-archive；GET review-queue[?dimension]；POST import-preview；GET import-preview-job/:id；GET import-preview-jobs?status=running；POST import-apply｜import-apply-pdf｜import-apply-image；POST extract-tags；POST auto-verify；POST test-match；POST tools/search｜tools/open-slice；GET metadata；GET completeness；GET integrity-report；GET catalog｜catalog/persisted；GET logs/analyze；POST repair（chunks/:id/repair｜repair/answer）；POST repair/applied；GET/POST chat；GET chat/:sid；POST chat/:sid/apply｜discard；GET inbox[?priority]。（knowledge 族）POST /api/knowledge/ask；GET /api/knowledge/ask/stream；GET/POST /api/knowledge/chat/tasks；GET /api/knowledge/chat/tasks/:id；POST /api/knowledge/chat/tasks/:id/cancel；GET /api/knowledge/chat/sessions/:sid/stream(SSE)；GET /api/knowledge/digest/today；POST /api/knowledge/digest/regenerate；POST /api/knowledge/digest/cards/:id/dismiss；GET /api/knowledge/gap-signals[?status&limit]；POST /api/knowledge/gap-signals/sweep；POST /api/knowledge/gap-signals/:id/dismiss｜apply；GET /api/knowledge/metrics；GET /api/knowledge/operator-memory；POST /api/knowledge/operator-memory/:id/revoke；GET/POST /api/knowledge/ingest-sources；PATCH/DELETE /api/knowledge/ingest-sources/:id。（admin 族）GET/POST/PUT/DELETE /api/admin/domain-schemas[/:id]；POST /api/admin/domain-schemas/:id/activate；GET /api/admin/taxonomies；GET /api/admin/operation-state-policies；GET /api/operation-domains；POST /api/admin/{taxonomies｜operation-state-policies｜operation-domains}/:id/{publish｜rollout｜rollback}；GET /api/admin/observability/phase-rollup｜worker-health｜performance?hours=24；GET /api/behavior-signal-metrics?limit=14 |
| system-strategy | GET /api/agent-souls；POST /api/agent-souls；PUT /api/agent-souls/:id；POST /api/agent-souls/:id/publish；GET /api/prompt-templates；POST /api/prompt-templates；PUT /api/prompt-templates/:id；POST /api/prompt-templates/:id/publish；POST /api/prompt-templates/reset-system-pack；GET /api/admin/operation-state-policies?includeAllVersions；GET/POST /api/admin/taxonomies；PATCH/DELETE /api/admin/taxonomies/:id；GET /api/admin/taxonomy-candidates?status&kind；POST /api/admin/taxonomy-candidates/:id/approve｜reject；GET /api/admin/domain-profiles；POST /api/admin/domain-profiles[/generate]；PUT/DELETE /api/admin/domain-profiles/:id；POST /api/admin/domain-profiles/:id/publish｜activate｜rollout｜rollback；GET /api/admin/lessons-learned[?patternKind]；POST /api/admin/lessons-learned/:id/promote-to-peer-case；版本条 POST /api/admin/{taxonomies｜operation-state-policies}/:id/{publish｜rollout｜rollback} |
| llm-providers | GET/POST /api/admin/llm-providers；PUT/DELETE /api/admin/llm-providers/:providerId；POST /api/admin/llm-providers/test；POST /api/admin/llm-providers/:id/activate｜vision |
| operations | GET /api/events?accountId；GET /api/tasks?accountId；GET /api/decision-reviews?accountId；GET /api/llm-usage?accountId；GET /api/agent-runs?accountId；POST /api/agent-tasks/:id/review-now｜cancel |
| evolution | GET /api/evolution/runtime-flag；PUT /api/evolution/runtime-flag；GET /api/evolution/experiments?limit=20；GET /api/evolution/threshold-overrides/audit；GET /api/evolution/proposals/:id；POST /api/evolution/proposals/:id/release｜rollback |
| quality | GET /api/agent-outcome-metrics?accountId&horizon&limit=60；POST /api/operation-knowledge/auto-verify；POST /api/user-operations/evaluations/formula-adherence；GET/POST /api/evaluation-scenarios；DELETE /api/evaluation-scenarios/:id；GET /api/prompt-templates + PUT /api/prompt-templates/:id + POST …/publish（markers） |
| send-analytics | GET /api/send-ledger/overview?accountId；GET /api/send-ledger/stats?kind&accountId |
| autonomy | GET /api/outcomes/autonomy?accountId&horizon；GET /api/outcomes/autonomy/revisions?…&limit=50；GET /api/admin/outbox?accountId；POST /api/admin/outbox/:id/cancel |

### 4.2 空态/错误态文案约定

- **空态组件**：主区统一 `components/ui/EmptyState`（title+hint+可选 action）；窄栏/行内用文字（TaskRail 明确注释为何不用共享 EmptyState，`today.tsx:1431-1437`）；user-ops 内另有 `EmptyInline`/局部 `EmptyState`（legacy.tsx:1469-1488）。空态 hint 惯例是**指路句**：「到『用户运营』导入好友并开启自主运营」「确认推送请在 AI 总控对话中完成」「先到『账号管理』同步通讯录」。
- **「不可用 ≠ 0 / 失败 ≠ 空」诚实原则**（多处显式注释）：ask-human chip 计数不可用显「不可用」不隐藏（`ask-human/index.tsx:311-313`）；summary.total null 不渲「待处理 0 项」；CockpitView gaps 失败显「—」（「避免把加载失败伪装成零待办」）；SendHistorySection 失败渲「加载失败」绝不渲「还没发过」；quality/autonomy 指标「显示『—』表示该窗口内无样本；不要把它当 0 解读」；LLM 成本有未知 usage 显「至少 N」。
- **错误态三层**：全局横幅（uiStore.error←store 异步 action）、面板级 inline error（组件本地 state）、字段级校验文案（表单内）。降级保留旧数据惯例：inboxStore fatal 保留旧 items「加载失败（显示上次数据）」；contactCounts 失败静默保留旧值；digest 显「当前继续展示上次成功结果」。
- **加载态**：普遍「加载中…」；按钮进行时态「保存中…/测试中…/派工中…/筛选中…」。

### 4.3 文案红线词规避方式（自治禁词闸）

CI lint（`scripts/check-no-human-takeover`）扫 frontend/src 新增行，多个文件头注释明示（`JudgmentBar.tsx:3`、`EvolutionCenterTab.tsx:13-14`、`reviewLabels.ts:3-4`、`products-deals/index.tsx:61`）。前端的系统性规避手段：
1. **闭集字典集中翻译**：`lib/reviewLabels.ts` 是措辞单一真相源——`held_by_ai_policy`→「AI 策略主动暂缓」、`blocked_by_safety_guard`→「安全门拦截」、`ai_waiting_for_more_context`→「AI 等待更多上下文」（FINAL_REVIEW_STATUS_LABELS 10 值/HOLD_CATEGORY_LABELS 3 值，`reviewLabels.ts:6-23`）；GATEWAY_STATUS_LABELS 39 值展开复用同措辞消除口径漂移（`reviewLabels.ts:43-78`）；EVENT_KIND_LABELS 130+ 值全部逐写入点亲验。
2. **业务语义替换**：不说"人工接管"而说「AI 策略暂缓」（overview）、「请示决策人定夺/交由决策人裁决」（ask-human-config 开关 hint）、「疑似成交·待核实」（products-deals VERIFICATION_LABEL 注释直言「规避 CI 命名红线 lint 的禁词集」）、「授权 AI 自行处理」（delegated_back）、「留给你复查」（AutoVerifyPanel）、「回退表示 AI 因拦截率过高自主收敛」（autonomy planner）。
3. **红线机制在 UI 的呈现**（不是文案而是结构）：AI 起草/导入/抓取一律 draft+needs_review（「AI 永不自动 verify」在 steward 手工新建、ImportWizard、IngestSources、ChunkRepairPanel/applyAiRepairPatch thenVerify=false、AutoVerify 抽审 5% 硬下限、LessonPromote 仍走审核队列等 ≥6 处独立落实）；高危动作逐字确认（RELEASE/ROLLBACK/RESET PROMPT PACK/「已核对」/「确认发布」rollout）；prompt 语义审查三态（needs_human_confirm/红线拒绝→逐字核对+force）。

## 5. 偏差与疑点

1. **dead code（已核实）**：
   - `overview/index.tsx:2` import 的 `RefreshCw, ArrowRight` 未在 JSX 使用（grep 全文件仅 import 行命中）。
   - `knowledge/labels.ts:62-72` `REVIEW_CATEGORY_LABELS`/`reviewCategoryLabel` 无任何消费者（grep 全 src 仅自身定义命中）；且其措辞与 steward 实际使用的 `REVIEW_CATEGORIES`（`steward.tsx:1551-1557`）**同键不同文**——contested「有争议」vs「已退回」、needs_review「待确认」vs「待初审」、source_orphan「缺来源」vs「缺少来源」、dependents_pending「依赖待定」vs「关联不完整」。若未来有人接线 labels.ts 版本会出现两套分类文案。
   - `user-ops/legacy.tsx:1804-1822` `readableChangeItems` 定义后无调用方（guide 预览已改用 `ChangePreview`+`authoritativeChanges`）。
   - `stores/operationsStore.ts:148-172` `loadAgentRuns` 仅被单测消费（组件走 `loadOperationsData` 一次拉五端点）。
2. **重复定义/平行实现**：
   - `USER_RUNTIME_PARAMETER_FIELDS` 在 `user-ops/legacy.tsx:140-167` 与 `stores/userOpsDomainHelpers.ts:20-47` 各有一份近乎相同的 20 项表（一处 label 差异：「状态置信复盘线」vs「状态置信 Review 线」），runtimeParameters 文本互转函数（parseParameterValue/orderedRuntimeParameters/runtimeParametersText/FromText）与 `jsonFromText` 也双份实现——修改参数集需同步两处，易漂移。
   - `DomainPromptPanel`、`ActiveVersionsBar`、`agentKindLabel`、`statusSortOrder`、`usePagedList` 在 user-ops/legacy.tsx 与 system-strategy/index.tsx 各有一份平行实现（后者 CSS Module 化；行为近同但 window.confirm 文案略异）。`formatTime` 在 legacy.tsx 与 operations/index.tsx 双份。
   - 批量校验入口双份：knowledge/cockpit/AutoVerifyPanel（三档松紧 5/7/9、抽审 0.3/0.05 硬下限）与 quality/AutoVerifyTab（阈值/抽样比例/上限自由输入，抽样可填 0——**后端是否同样强制下限前端未体现**）打同一端点 `POST /api/operation-knowledge/auto-verify`，参数约束口径不一致。
   - `yuanToCents` 在 products-deals/index.tsx:100 与 ask-human/inline/SuspectedDealReviewCard.tsx:13 双份（行为略异：前者 null 宽容、后者含 isSafeInteger 校验）。
3. **未接线/占位 UI**：
   - user-ops 传统模式 audit tab 徽标 `pendingTasks` 写死 0（`user-ops/index.tsx:229-231`，注释自认「徽标后续可订阅 operationsStore.pending 派生」）。
   - overview「在线账号」卡的 spark 柱状图为写死静态高度（`overview/index.tsx:79-82`），非真实数据。
   - `channels.ts:105-107` `visibleWhen` 谓词本期无频道使用（自述留作扩展点）；groupOps/momentOps 两占位频道 Component 指向 OverviewFeature 仅靠 comingSoon 阻止进入。
   - knowledge/KnowledgeInbox 的「暂时忽略」是**本地乐观隐藏**（刷新恢复），注释自认「后端暂无逐条 dismiss 接口时不发死请求」（`today.tsx:570-574`）——与 gap_signal 在 LintView/ask-human 有真 dismiss 端点形成不对称。
   - quality/ProductClaimMarkersTab 编辑的兜底守卫配置「当前未启用」（页面自述，编辑不影响评审行为，`quality/index.tsx:545-549`）。
4. **与后端口径已知不一致（前端注释自曝）**：
   - `DIGEST_TARGET_REF_KIND_LABELS`：prompts.rs 给 LLM 的枚举（chunk|pack|proposal）与 models.rs 字段注释（chunk|pack|item|run|evolution_proposal）两处口径不一致，前端取并集兜底（`labels.ts:279-291`）。
   - `digestMetricNameLabel` 面对的 metric.name 非闭集（LLM 自由填写），字典只是尽力翻译（`labels.ts:237-245`）。
   - ChunkReviewCard 必须同时容忍 snake_case（GET by id 原始序列化）与 camelCase（列表整形）两种响应形状（`ChunkReviewCard.tsx:10-18`）——后端两个端点序列化不统一。
   - ResolvedEscalations 的 decision 内层是 snake_case（models.rs 注释明示不 rename），外层 camelCase（`ResolvedEscalations.tsx:5-9`）。
   - JudgmentBar 的 `inQuietHours/nextWakeAt`、`lastConversationMode` 均为 TS 类型未声明的拓展键，断言读取（`JudgmentBar.tsx:72-74,90-94`）——类型定义落后于后端实际下发。
5. **文案/语义疑点**：
   - knowledge/TryRecallView 的 `accountId` 输入框 placeholder 写「客户 ID（可选，默认 default）」（`steward.tsx:1206`），字段实为账号 ID，与旁边「联系人 ID」并列易误导。
   - `GAP_SIGNAL_SEVERITY_LABELS` 键集 info/warning/error/high（`reviewLabels.ts:109-114`）与 knowledge digest 的 severity 闭集 info/warn/critical（`labels.ts:51-55`）是两套不同的严重度体系，同名概念不同取值（分属 gap_signal 与 digest 域，非 bug 但读码易混）。
   - system-strategy 版本条与 user-ops 版本条对同一动作的确认文案不同（`确认发布 X 新版本（version=N+1）` vs `（vN+1）`），无功能影响。
6. **规模/维护性观察**：knowledge/steward.tsx 3342 行内含 8 个互不相关的大组件（Documents/Import/TryRecall/Lint/Review/Ingest/Observability/Revisions），system-strategy/index.tsx 2831 行内含 6 个面板 + 全系统最大表单 ProfileEditor（约 965 行），均远超单文件可读上限；ProfileEditor 的 10 个折叠区字段直接映射 DomainProfile 全部可配置面，是理解后端 profile 语义的最佳前端参照。

## 6. 覆盖自证

逐文件 100% 读毕（含大文件分段全读：steward.tsx 4 段、system-strategy 4 段、legacy.tsx 2 段、today/atlas/shared 各 2 段）。行数与 `wc -l` 核对一致。

| feature / 目录 | 文件数 | 总行数 |
|---|---|---|
| command-center | 2 | 481 |
| overview | 1 | 123 |
| account-management | 2 | 384 |
| user-ops（含 cockpit/ 4 + drilldowns/ 4） | 16 | 4978 |
| content-assets | 1 | 792 |
| referral-cards | 1 | 270 |
| ask-human（含 inline/ 3） | 5 | 1025 |
| ask-human-config | 4 | 547 |
| campaign | 8 | 566 |
| products-deals | 1 | 1100 |
| knowledge（含 cockpit/ 6） | 19 | 10711 |
| system-strategy | 1 | 2831 |
| llm-providers | 1 | 811 |
| operations | 1 | 570 |
| evolution | 2 | 553 |
| quality | 2 | 886 |
| send-analytics | 1 | 124 |
| autonomy | 2 | 722 |
| **features/ 小计** | **70** | **27474** |
| components/review/ | 9 | 1612 |
| **任务范围合计** | **79** | **29086** |

任务范围外、为核证「与 store 的分工 / 频道地图 / 文案字典」而全文补读的支撑文件（同样逐行读完）：`App.tsx`(163)、`app/channels.ts`(344)、`app/Shell.tsx`(334)、`stores/` 全部 16 个（含 userOpsStore 1369、strategyStore 506、userOpsDomainHelpers 177、navigationStore 122 等，合计 3226）、`lib/api.ts`(146)、`lib/inboxApi.ts`(132)、`lib/reviewLabels.ts`(370)、`lib/useSseReconnect.ts`(84)、`lib/uuid.ts`(78)、`lib/applyAiRepairPatch.ts`(53)、`components/prompt/usePromptSaveConfirm.tsx`(122)、`src/EvolutionCenterTab.tsx`(22)。


---

## 追记：27 号 API 三方对账回写（2026-08-13，主会话执行）

27 号发现本记录（与 13 号）漏检的 **2 个运行时真缺陷**（query 参数命名错配，主会话双侧亲验）：

1. **"被引用"功能恒 400**：`knowledge/shared.tsx:933` 发 `?target_id=`，后端 `ChunkReferrersQuery` 经 `rename_all="camelCase"` 只认 **`targetId`** 且必填无 default（`wiki_edit.rs:840-844`）→ Query 反序列化失败恒 400。后端注释 `:846` 自己也写错参数名。
2. **产品过滤静默失效**：`campaign/ProductMultiSelect.tsx:15` 发 `?active_only=true`，后端 `ListQuery.active_only` wire 名为 **`activeOnly`** 且带 `#[serde(default)]`（`products.rs:40-46`）→ 参数被静默忽略、返回全量，campaign 圈人下拉混入归档产品。

教训：前端 query 参数名与后端 body 的 camelCase 约定不同源——后端 Query 结构体同样走 `rename_all="camelCase"`，前端存在 snake_case 惯性书写；契约测试只对账 response 键集、不覆盖 request query 参数名，此类错配无守护。

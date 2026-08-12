# 前端核心深读记录（核证日期 2026-08-13）

> 深读范围：`frontend/` 根配置 5 文件 + `src/main.tsx`、`App.tsx`、`src/app/` 全部、`src/stores/` 全部 17 个 store 文件、`src/contracts/` 全部 38 个 `*.contract.ts`、`src/lib/` 全部 8 文件、`src/types/index.ts`、`src/components/` 目录结构与 `ui/` 全部共享组件（含 index.ts 桶文件）+ `components/prompt/usePromptSaveConfirm.tsx` + `components/LlmErrorBanner.tsx`。
> 另为核证跨文件机制补读：`src/features/knowledge/chunkInvalidation.ts`（App.tsx 直接依赖）、`src/EvolutionCenterTab.tsx`、`src/vite-env.d.ts`、7 个 `__tests__/contracts/*.contract.test.ts` 的 import 面与两个代表性测试全文、后端 `src/routes/contract_snapshot.rs` 的 bless 机制段。
> 全部断言基于逐行阅读，附 `文件:行号`；读不懂/前端侧无法证实的写入「§5 偏差与疑点」。

---

## 1. 模块地图（目录树 + 每文件一句话）

```
frontend/
├── package.json            # npm 清单：React 19 + Vite 8 + TS 5.9 + Zustand 5 + lucide-react；vitest 4 测试
├── vite.config.ts          # dev server 代理 /api、/webhooks → http://localhost:8080
├── tsconfig.json           # ES2022 / strict:true / noEmit / react-jsx
├── vitest.config.ts        # 独立于 vite.config；jsdom + globals + setup + 仅扫 src/__tests__/**/*.test.{ts,tsx}
├── index.html              # 单入口 <div id="root"> + /src/main.tsx；zh-CN
├── walkthrough.py          # （未在本次深读范围，页面演练脚本）
└── src/
    ├── main.tsx            # 启动：fetch 401 拦截器 monkey-patch、LoginScreen/AuthGate/AuthedApp、注入 authStore handlers
    ├── App.tsx             # 登录后根组件：chunk WebSocket 事件流 + 启动引导（accounts / activeProfile / activeView）
    ├── EvolutionCenterTab.tsx  # 纯 re-export 兼容层（真实实现在 features/evolution/）
    ├── vite-env.d.ts       # vite/client 引用 + *.module.css 类型声明
    ├── styles.css          # 全局样式（68KB，未逐行；频道级样式主体）
    ├── app/
    │   ├── Shell.tsx               # 布局壳：侧栏（分组导航+账号切换+workspace切换+登出）+ 主区 header + Suspense 频道出口
    │   ├── Shell.module.css        # 壳样式（未逐行）
    │   ├── channels.ts             # 20 频道单一事实源（分组/文案/图标/lazy 组件/visibleWhen/comingSoon）
    │   ├── GlobalErrorBanner.tsx   # 消费 uiStore.error 的全局 role=alert 横幅
    │   └── GlobalErrorBanner.module.css
    ├── stores/             # 17 个 Zustand store（zustand create，无中间件，无 devtools）
    │   ├── navigationStore.ts      # 活跃频道 + 分组折叠态（localStorage v4 key + 白名单清洗 + 旧 key 清理）
    │   ├── authStore.ts            # 登录用户快照 + onLogout/onSwitchWorkspace 回调注入槽
    │   ├── accountStore.ts         # 账号列表 + 选中账号（localStorage 持久化）+ currentAccountId 收敛
    │   ├── uiStore.ts              # 全局 busy/error 两字段
    │   ├── profileStore.ts         # active DomainProfile + 维度/取值字典 + labelFor 纯函数
    │   ├── contactStore.ts         # 联系人列表（账号作用域 + requestGeneration 竞态防护 + 响应账号校验）
    │   ├── userOpsStore.ts         # 用户运营频道全量状态（1369 行；详情联动/剧本/roster/域配置/引导/模拟）
    │   ├── userOpsDomainHelpers.ts # 运营域 runtimeParameters 文本↔对象互转 + 作息(quiet hours)提取/回写 + payload
    │   ├── commandStore.ts         # AI 总控：management-agent session/messages + 高危计划 confirm/reject（planHash 冻结）
    │   ├── contentStore.ts         # 内容资产 CRUD + 上传/换文件/审核/启停（账号+workspace 双作用域防护）
    │   ├── referralCardStore.ts    # 专属顾问名片 CRUD + 审核/启停（workspace 级）
    │   ├── campaignStore.ts        # 活动列表 + 触达报告（generation 防护 + openReport 跨频道跳转）
    │   ├── operationsStore.ts      # 任务日志频道 5 端点并发加载 + 任务 review-now/cancel
    │   ├── strategyStore.ts        # 系统策略：souls/prompt-templates 版本流 + DomainProfile 生成/发布/激活（三态保存）
    │   ├── inboxStore.ts           # 统一收件箱 items+summary（失败保留旧快照，fatalError 不清 items）
    │   └── sendAnalyticsStore.ts   # 发送成效 overview + media/namecard stats
    ├── contracts/          # 38 个 .contract.ts（前端声明后端投影键集）+ 42 个 .fixture.json（后端 bless 写出）
    ├── lib/
    │   ├── api.ts                  # fetch 薄封装（get/post/put/patch/delete/postForm/postRaw）+ parseApiError + LlmUnavailableError + openEventSource（无消费方）
    │   ├── inboxApi.ts             # 收件箱两端点封装 + severity 排序 + summary 宽松解析
    │   ├── useSseReconnect.ts      # SSE 指数退避重连器（仅监听流；禁用于一次性 RPC 流）
    │   ├── reviewLabels.ts         # 全站英文闭集→中文标签单一真相源（10+ 张映射表 + labelOf 兜底）
    │   ├── applyAiRepairPatch.ts   # AI 修复 patch 落库（thenVerify 恒 false 红线）
    │   ├── clipboard.ts            # 复制两级降级（Clipboard API → execCommand）
    │   ├── format.ts               # formatRate/formatNumber/formatTimestamp（BSON $date 脱壳第二道防线）
    │   └── uuid.ts                 # randomUuid 三级降级（非安全上下文兜底）
    ├── types/index.ts      # 跨 feature 共享类型单一来源（842 行；Channel/Contact/DomainProfile/AskHumanPolicy 等）
    ├── components/
    │   ├── LlmErrorBanner.tsx(+css)   # LLM 不可用错误横幅（kind 中文映射 + client_error 区分 + AI 重试按钮）
    │   ├── prompt/usePromptSaveConfirm.tsx  # prompt 保存/发布三态二次确认流（needsConfirm/rejected → 逐字核对 + force）
    │   ├── review/                  # 审核卡片族（ChunkReviewCard/ProposalReleaseCard/ReviewQueue 等 12 文件，结构级记录）
    │   └── ui/                      # 共享组件库（12 组件 + tokens.css + reset.css），逐个精读见 §2.6
    ├── features/           # 18 个频道 feature 目录（本篇仅结构 + chunkInvalidation.ts 深读，其余归 14-frontend-features.md）
    └── __tests__/          # vitest：app/stores/lib/components/features/contracts 分目录 + setup.ts
```

`features/` 顶层 18 目录：`account-management, ask-human, ask-human-config, autonomy, campaign, command-center, content-assets, evolution, knowledge, llm-providers, operations, overview, products-deals, quality, referral-cards, send-analytics, system-strategy, user-ops`。

---

## 2. 逐文件深读

### 2.1 根配置

**package.json（38 行）**
- 脚本：`dev`=vite 5173 host 0.0.0.0；`build`=`tsc && vite build`（先全量类型检查）；`test`=`vitest run`（package.json:7-11）。
- 依赖锁定点：`vite 8.2.1`、`zustand 5.0.14`、`@vitejs/plugin-react 6.0.5` 精确版本；react/react-dom `^19.2.0`；`lucide-react ^0.554.0`（13-21）。
- `overrides` 钉死传递依赖 `undici/postcss/nanoid/esbuild`（32-37）——供应链版本收敛。
- 无路由库、无 css 框架、无请求库（原生 fetch）。

**vite.config.ts（13 行）**：仅 react 插件 + dev 代理 `/api`、`/webhooks` → `http://localhost:8080`（vite.config.ts:7-10）。生产由 Rust `ServeDir` 托管 `frontend/dist`（对齐 CLAUDE.md）。

**tsconfig.json（22 行）**：`target ES2022`、`strict: true`、`noEmit: true`、`jsx: react-jsx`、`moduleResolution: Node`、`resolveJsonModule: true`（fixture JSON 直接 import 的前提）（tsconfig.json:3-17）；`include: ["src"]`。

**vitest.config.ts（19 行）**：与 vite.config 刻意分离（注释 vitest.config.ts:1-6）；jsdom + globals + `setupFiles: ./src/__tests__/setup.ts` + 仅收 `src/__tests__/**/*.test.{ts,tsx}`（13-18），避免误吞 App.tsx 内字符串。

**index.html（14 行）**：`lang="zh-CN"`、标题 `WechatAgent Admin`、`/favicon.svg`、模块入口 `/src/main.tsx`（index.html:2-11）。

### 2.2 入口与 app/

**src/main.tsx（200 行）—— 认证入口 + 全局 401 拦截**
- **fetch monkey-patch**（main.tsx:12-24）：模块加载即执行。保留 `originalFetch`（13）；包装后所有 `window.fetch` 响应若 `401` 且 `url.startsWith("/api/")` 且非 `/api/auth/login`，则 `sessionStorage.removeItem("wa.authed")` 并派发 `window` 事件 `wa-auth-expired`（18-21）。判定用 `typeof input === "string" ? input : (input as Request).url`（17）。
- **LoginScreen**（34-115）：受控表单，`POST /api/auth/login`（用 originalFetch 绕过拦截器，45-49），失败读 body.error（`invalid_credentials`→中文，52-53）；成功后再 `GET /api/auth/me`（56）拿 `MeResponse{username,userId,workspaces?,currentWorkspace?}`（26-31），写 sessionStorage 标记（62）后 `onLoggedIn(me)`。页脚提示管理员账号来自 `BOOTSTRAP_ADMIN_USERNAME/PASSWORD` 环境变量（108-111）。
- **AuthGate**（117-169）：挂载即 `GET /api/auth/me` 校验既有会话（125），`cancelled` 闭包防卸载后 setState（122-141）；监听 `wa-auth-expired` → `setMe(null)` 打回登录页（144-150）；`logout()` `POST /api/auth/logout`（失败也清本地态，152-160）。三态渲染：bootstrapping 占位 / LoginScreen / AuthedApp（162-168）。
- **AuthedApp**（171-194）：effect 内把 `me` 写入 `useAuthStore.setUser`，并注入 handlers：`onLogout`；`onSwitchWorkspace` = `POST /api/auth/workspace {workspaceId}` 成功后 `window.location.reload()`（180-185），同 workspace 短路（177-178）。渲染 `<App/>`。
- 根：`createRoot(...).render(<React.StrictMode><AuthGate/></React.StrictMode>)`（196-200）。

**src/App.tsx（163 行）—— chunk WS + 启动引导**
- `ChunkEventEnvelope` 判别联合（App.tsx:21-44）：`hello{workspace}` / `lagged` / `locked{chunk_id,workspace_id,owner_user_id,owner_username,expires_at}` / `unlocked{chunk_id,workspace_id,owner_user_id}` / `revised{chunk_id,workspace_id,revision_kind,actor}`——注意 **wire 键为 snake_case**（与多数 API 的 camelCase 不同）。
- `dispatchChunkEvent`（46-68）：hello 忽略；`lagged`→`invalidateChunks({reason:"lagged"})`；locked/unlocked→`window` CustomEvent `wikiChunkLocked`/`wikiChunkUnlocked`；revised→CustomEvent `wikiChunkRevised` **且** `invalidateChunks({reason:"revised", chunkId, revisionKind})`（59-66）。
- `useChunkEventStream`（70-137）：连 `ws(s)://host/api/ws/chunks`（79-80）；onmessage JSON 解析失败静默丢（90-96）；onclose→`scheduleReconnect`，退避 `backoffMs` 初始 1000、每次 ×2、cap 30000（112-119），onopen 重置 1000（87-89）；onerror 只 close 触发 onclose（103-109）；卸载 cancelled+clearTimeout+close（124-135）。注释声明：WS 只做协作提示，失败不阻塞业务，写一致性靠后端事务/CAS（19-20）。
- `App`（140-163）：顶层挂一次 WS；`accountsBootstrapRef` 防 StrictMode 双跑（145-148）；启动引导 `GET /api/accounts`→`accountStore.setAccounts`，错误→`uiStore.setError`（149-152）；`profileStore.loadActiveProfile()` + `loadActiveView()`（153-154）。渲染 `<GlobalErrorBanner/><Shell/>`。

**src/app/GlobalErrorBanner.tsx（16 行）**：读 `uiStore.error`；非空渲染 `role="alert"` 横幅 + ✕ 清空（GlobalErrorBanner.tsx:5-14）。

**src/app/channels.ts（344 行）—— 频道单一事实源**
- 18 个 `lazy(() => import("../features/*"))`（channels.ts:26-43）。
- `ChannelGroup` 6 组闭集：日常处置/客户运营/知识资产/运行监控/平台配置/建设规划（68-74）；`GROUP_ORDER` 显示顺序单点（86-93，Shell 不再持第二份）。
- `ChannelDef`（95-111）：`id/group/label/caption/icon/eyebrow/title/subtitle/Component` + 可选 `visibleWhen?(profile)`（未定义→默认显示；**本期无频道使用**，106-107）+ `comingSoon?`（占位灰显不可点，109-110）。
- `CHANNELS` 共 **20 项**（116-344），与 `types/index.ts:4-24` 的 `Channel` 联合 20 成员一一对应。分组归属：日常处置=command/overview/askHuman；客户运营=userOps/campaign/productsDeals；知识资产=content/referralCards/knowledgeWiki；运行监控=operations/quality/sendAnalytics/autonomy；平台配置=accountManagement/askHumanConfig/systemStrategy/llmProviders/evolution；建设规划=groupOps/momentOps（均 `comingSoon: true`，Component 暂指 OverviewFeature，163-184）。
- 头注留有三次归组修正的实证依据（56-67）与图标轨/hint 字段删除原因（80-85）——决策日志内联在源码。

**src/app/Shell.tsx（334 行）—— 布局壳与导航模型**
- `syncAccounts()`（Shell.tsx:14-19）：`POST /api/accounts/sync` → 回拉 `GET /api/accounts` 刷 accountStore，返回 synced 数。
- `AccountSwitcher`（24-132）：0 账号空态直接给「同步微信号」按钮（57-72）；否则 trigger（当前账号 + `在线数/总数`）+ listbox 菜单（93-129），菜单尾部复用同步入口（116-126）。当前账号收敛：selectedAccountId 不在列表则回落 `accounts[0]`（74-76）。点击外部 mousedown 关闭（33-40）。
- `WorkspaceSwitcher`（138-195）：仅 `user.workspaces.length > 1` 时由 Shell 渲染（307-311）；选项点击调 `authStore.onSwitchWorkspace`（182）——即 main.tsx 注入的 POST+reload。带 `data-testid`（162,179）。
- `Shell`（197-334）：**导航模型 = navigationStore.activeChannel 单值切换**（`setChannel`，无路由、无 URL 同步）。`def = CHANNELS.find(...) ?? CHANNELS[0]`（205）。`groupItems(group)` = 按组过滤再按 `visibleWhen(activeProfile)` 过滤（210-213）。分组渲染：`GROUP_ORDER.map`，空组不渲染（239-241）；分组标签是 `<button aria-expanded>` 折叠开关（249-265）；收起且组内含活跃频道时打点 `holdsActive`（243-244,261-263）+ 收起时显示组内计数（264）；`comingSoon` 项渲染为 `aria-disabled` div + 「未上线」角标（269-282）；普通项 `aria-current="page"`（283-293）。设计决策注释：独立折叠（非手风琴互斥）、允许滚动（230-237）。footer：AccountSwitcher + 用户条（头像首字母/用户名/workspace 文本或切换器/登出按钮）（300-319）。主区：eyebrow/title/subtitle 来自频道 def + `<Suspense fallback>` 包 lazy `Component`（322-331）。

### 2.3 stores/（17 文件全量）

通用形态：全部 `create<State & Actions>()`，**无 persist/devtools 中间件**；持久化手写 localStorage；跨 store 用 `useXxxStore.getState()` 直调。

**navigationStore.ts（122 行）**
- State：`activeChannel: Channel`（初始 `"command"`，navigationStore.ts:111）、`collapsedGroups: Set<ChannelGroup>`。Actions：`setChannel`（112）、`toggleGroup`（114-121，必须造新 Set 触发引用比较重渲染，注释 115）。
- 持久化：key `wa.nav.collapsed.v4`（31）存**被收起**的组名数组；`LEGACY_KEYS` 6 个旧 key 每次 load 先删（33-40,76）；`VALID_GROUPS` 白名单过滤脏值（44-51,83-87）；`raw===null`→`DEFAULT_COLLAPSED`（知识资产/平台配置/建设规划，67-71），**存过 "[]" 是合法「全部展开」**，与未存过必须区分（78-80）；读写全 try/catch（74-99）。key 轮换纪律与 v2→v4 历史内联成注释（21-30）。
- 端点：无。竞态防护：不需要（纯本地）。

**authStore.ts（24 行）**：`user: AuthUser|null` + `onLogout/onSwitchWorkspace` 回调槽（null 初始）+ `setUser/setHandlers`（authStore.ts:18-24）。回调由 main.tsx:174-190 注入。端点：无（间接经 handlers）。

**accountStore.ts（35 行）**：`accounts: Account[]`、`selectedAccountId`（初始读 localStorage `wechatagent.accountId`，accountStore.ts:4,18）。Actions：`setAccounts`（19）、`selectAccount`（写 localStorage + set，20-23）、派生 getter `currentAccountId()`（选中不在列表回落 `accounts[0]?.accountId ?? ""`，24-29）、`currentAccount()`（30-33）、`onlineCount()`（34）。端点：无（数据来自 App.tsx / Shell.syncAccounts）。**`currentAccountId()` 是全站账号作用域防护的唯一裁判**。

**uiStore.ts（15 行）**：`busy/error` + setter（uiStore.ts:10-15）。error 由 GlobalErrorBanner 全局展示；busy 由多个 feature 消费禁用按钮（核证：features/referral-cards/index.tsx:15、system-strategy/index.tsx:2664、content-assets/index.tsx:78、knowledge/shared.tsx:730）。

**profileStore.ts（79 行）**
- State：`activeProfile: DomainProfile|null`、`dimensions: ProfileDimensionView[]`、`taxonomies: TaxonomyMap`、`loading/error`（profileStore.ts:31-38）。
- `labelFor(taxonomies, kind, value)` 纯函数三态：无字典→`{text:value,status:"no_dict"}`；有字典无命中→`unknown_value`；命中→`ok`（23-29）——「绝不显示错误销售标签」。
- Actions：`loadActiveProfile` `GET /api/admin/domain-profiles/active`（50-52），失败降级 null 照常跑（54-61）；`loadActiveView` `GET /api/operation/active-view` → dimensions+taxonomies（63-77），失败清空降级。
- 竞态防护：无 generation（启动一次性拉取；App.tsx:153-154 的引导中调用）。

**contactStore.ts（130 行）**
- State：`contacts/selected/dataAccountId/requestGeneration/loading/contactTab`（contactStore.ts:8-13）。
- **竞态防护三件套**（本仓库范式的最完整样本）：
  1. `requestGeneration` 单调递增，响应落地前比对（88-93,105-109,116-123）；
  2. `dataAccountId` 数据作用域与请求账号一致才收（89,107,118）；
  3. `currentAccountId()` 全局当前账号仍是请求账号才收（91,108,120）。
- 额外**响应体账号校验**：`items.some(c => c.accountId !== accountId)` → 拒收并 setError「联系人响应账号与当前账号不一致，已拒绝显示。」（95-101）。
- `setSelected` 拒绝跨账号选中（46-55）；`clearForAccount` 清列表+选中并 bump generation（59-65）；`loadContacts(accountId, query, {silent})`：scope 变更时立清 contacts/selected（69-75），账号为空或已切走则直接停（76-79）；参数 `accountId&limit=500[&q=]`（81-83）。
- 派生：`managedCount/normalCount` 基于 `scopedContacts()`（dataAccountId 与当前账号一致才算数，28-31,127-128）。
- 端点：`GET /api/contacts?accountId=…&limit=500[&q=]`（86）。

**commandStore.ts（202 行）**
- State：`commandDraft`（初始为演示文案，commandStore.ts:36）、`commandResult: CommandResult|null`、`commandDryRun`（初始 true）、`commandBusy`、`souls/assets/pendingTasks`（7-15,35-42）。
- `loadCommandData(accountId?)`：并发 `GET /api/content-assets[?accountId]` + `GET /api/agent-souls` + `GET /api/tasks[?accountId]`，pendingTasks=status==="pending" 计数（50-69）。
- `runCommand(accountId)`：先 `POST /api/management-agent/sessions {accountId,title,dryRun}` 建会话（80-84），再 `POST /api/management-agent/sessions/:id/messages {accountId,content,dryRun}`（87-94）；**落结果前校验当前账号未切走**（96-100）；随后重拉 tasks 刷 pendingTasks（102-106）。
- `confirmCommand(id)` / `rejectCommand(id)`（117-160/163-201）：前置校验五条——commandResult 存在、id 匹配、`accountId` 与 `planHash` 存在、accountId===当前账号（121-128/167-174），否则 setError「该执行计划不属于当前账号或缺少冻结标识」；`POST /api/management-agent/commands/:id/confirm|reject {accountId, planHash}`；**set 回执前在 updater 里再整套校验一遍**（138-145/184-191）。`planHash` 是后端渲染给运营的计划的 SHA-256 冻结绑定（types/index.ts:284-287）。注释引后端 `src/routes/management.rs:506-549` 的 ConfirmResponse 形状（27-33，前端注释所述，后端行号未在本次核证）。
- 竞态防护：账号切换防护有；无 generation（单结果面板，后写覆盖可接受）。

**contentStore.ts（296 行）**
- State：`assets/assetsAccountId/assetsRequestGeneration/assetDraft/assetDraftAccountId`（contentStore.ts:27-33）。`AssetDraft{kind,title,body,usageScene,minInjectTier}`（7-13）。
- **作用域模型**：资产可能是账号级或 workspace 级——`assetScope(asset)` 依 `asset.accountId` 产出 `{expectedScope:"account",expectedAccountId}` 或 `{expectedScope:"workspace"}`（64-68），写操作全部回传给后端做 CAS 式校验。
- 防护函数：`actionIsCurrent(asset,pageAccountId)`（79-88）＝页面账号即当前账号 ∧ 列表作用域一致 ∧ 资产属于该页（workspace 级恒真，74-76）∧ 资产仍在当前列表里；`refreshIfCurrent`/`reportIfCurrent`（90-100）。
- Actions 与端点：`loadAssets` `GET /api/content-assets?accountId[&tag]`（114-149，generation+scope+响应账号三重校验 128-138）；`createAsset` `POST /api/content-assets`（151-186，draft 账号一致才发，成功后保留 kind/minInjectTier 清其余）；`uploadMediaAsset` `POST /api/content-assets/upload`（FormData，188-203）；`reviewMediaAsset` `POST /api/content-assets/:id/review {scope,status:"approved"|"draft",note}`（205-221）；`editAssetMeta` `PUT /api/content-assets/:id {fields,scope}`（223-238）；`replaceAssetFile` `POST /api/content-assets/:id/file`（FormData 注入 expectedScope/expectedAccountId，240-262）；`toggleAssetSendable` `POST /api/content-assets/:id/toggle {scope,sendable}`（264-279）；`deleteAsset` `DELETE /api/content-assets/:id?expectedScope=…`（281-294）。全部 busy/error 走 uiStore，finally 仅当前页才復位 busy（如 184）。

**referralCardStore.ts（115 行）**
- State：`cards: ReferralCard[]`、`cardDraft`（displayName/targetWxid/sendTriggerHint/targetStages/tags 逗号串，referralCardStore.ts:6-17）。
- Actions：`loadCards` `GET /api/referral-cards`（34-41，**无 accountId 参数**——名片是 workspace 级资源，type 中 accountId 可空 types/index.ts:186）；`createCard` `POST /api/referral-cards`（targetStages/tags 逗号拆数组，43-75）；`reviewCard` `POST /api/referral-cards/:id/review {status,note}`（77-88）；`toggleCard` `POST /api/referral-cards/:id/toggle {enabled}`（90-101）；`deleteCard` `DELETE /api/referral-cards/:id`（103-114）。
- 竞态防护：无 generation/账号校验（workspace 级列表，低风险；见 §5-9）。

**campaignStore.ts（121 行）**
- State：`selectedCampaignId/report/loading/lastAttemptedId/reportRequestGeneration/view("list"|"create"|"board")/campaigns/listLoading/listLoaded/page`（campaignStore.ts:42-59）。类型内联定义 `CampaignSummary`（blocked/canceled/escalated 是 Record<string,number> 归桶，6-15）、`CampaignSendItem`、`CampaignReport`、`CampaignListItem`。
- `openReport(id)`（72-76）：**跨频道跳转入口**——set 选中+清 report+view=board，`useNavigationStore.setChannel("campaign")`，再 loadReport。
- `loadReport(id)` `GET /api/campaigns/:id/sends`（77-106）：generation + `selectedCampaignId===id` + 响应 `campaignId===id` 三重校验（88-92）；同 id 重载时保留旧 report 避免闪空（83）。
- `loadCampaigns` `GET /api/campaigns`（108-118，无 generation）；`clear` 全量复位 + bump generation（120）。

**operationsStore.ts（179 行）**
- State：`events/tasks/decisionReviews/llmUsage/agentRuns` + `dataAccountId/requestGeneration/agentRunsGeneration/loading/opsTab`（operationsStore.ts:14-24）。
- `loadOperationsData(accountId)`（89-146）：五端点 `Promise.all` —— `GET /api/events?accountId`、`/api/tasks?accountId`、`/api/decision-reviews?accountId`、`/api/llm-usage?accountId`、`/api/agent-runs?accountId`（103-109）；generation+scope+currentAccountId 三重校验（110-114）；**tasks 响应账号校验**不一致全清并报错（115-120）。
- `loadAgentRuns(accountId)`（148-172）：独立 `agentRunsGeneration` 防护。
- `reviewTaskNow/cancelTask` → `runTaskAction`（58-77）：`taskActionIsCurrent`（47-56，页面账号=当前=数据作用域=task.accountId ∧ task 仍在列表）才发 `POST /api/agent-tasks/:id/review-now|cancel {expectedAccountId}`，成功后整页重载。

**strategyStore.ts（506 行）**
- State（24-42）：souls/promptTemplates + soulDraft/editingSoulId + promptDraft/editingPromptId + DomainProfile 段（domainProfiles/editingProfile/isCreatingProfile/profileDraft/profileTab/generating/generateError/generateResult）。
- **三态保存协议 `SavePromptResult`**（11-15）：`{ok}` | `{needsConfirm,reason,diff}`（后端 200 body `status:"needs_human_confirm"`，**不能当成功**，注释 6-10）| `{rejected,reason}`（后端 4xx，message 含「红线语义审查拒绝」）| `{error,reason}`。
- 版本流红线：`saveSoul` PUT `/api/agent-souls/:id` 返回**新版本 id**，必须 `set({editingSoulId: saved.id})` 让后续 publish 指向新草稿（159-184，注释 175）；`savePromptTemplate` 同理（218-258，241-242）；`saveDomainProfile` 同理（423-455，448）。
- Actions 与端点：`loadStrategyData`（GET `/api/agent-souls` + `/api/prompt-templates` 并发，124-137）；`createSoul` POST；`publishSoul` POST `/:id/publish`（186-198）；`createPromptTemplate` POST（200-216）；`savePromptTemplate(force?)` PUT（218-258，force 载荷注释 92-93：字面双闸仍跑、只越过 LLM 语义审查）；`publishPromptTemplate(id,force?)` POST `/:id/publish`（260-285，同三态）；`resetSystemPromptPack(confirmation)` POST `reset-system-pack`（287-303）；`loadDomainProfiles` GET `/api/admin/domain-profiles`（346-353）；`generateDomainProfile` POST `…/generate {businessDescription,profileId[,displayName]}`（357-372）；`editDomainProfile`（374-408，把 profile 25 个字段逐一搬进 snake_case draft）；`newDomainProfileDraft`（410-419，`isCreatingProfile` 让编辑器在「新建态」渲染，注释 34-36）；`saveDomainProfile`（423-455：update=PUT `/:id` 直发 snake_case draft；create=POST 且**必须补顶层 camelCase `profileId`**，注释 436-438——后端 UpsertRequest 顶层读 profileId、内层 profile_id 会 flatten）；`publishDomainProfile` POST `/:id/publish`（457-474，**发布只移 published-current 指针，须再 activate 才生效**，注释 466）；`activateDomainProfile` POST `/:id/activate` 返回 `{ok,status:"completed"|"partial",retryable,errors[]}`（17-22,476-491）；`deleteDomainProfile`（493-505，`window.confirm` 原生确认）。
- 竞态防护：无 generation（策略页低频编辑）；错误统一 uiStore，唯 rejected/needsConfirm 例外走返回值（避免与普通错误混淆，注释 247-249）。

**inboxStore.ts（90 行）**
- State：`items/errors/summary/loading/fatalError/activeSource/activeAccountId/requestGeneration/summaryRequestGeneration`（inboxStore.ts:11-25）。
- `principalEscalationCount(summary)` 导出：`summary.counts.principalEscalation` 数字才返回（27-32，供判断条请示灯）。
- `refreshSummary(accountId?)`（46-58）：独立 generation；**失败静默吞**——保留最后一次成功快照，「null 表示尚不可用」（55-57）。
- `load(source?,accountId?)`（59-89）：`Promise.all([fetchInbox, refreshSummary])`（refreshSummary 永不 reject）；成功 `sortItems` 排序、errors = inbox.errors ∪ summary.errors 按 source 去重（72-80）；**请求级失败保留旧 items 不清空**，只置 fatalError（81-88）。
- 端点（经 lib/inboxApi）：`GET /api/admin/ask-human/inbox[?source&accountId]`、`GET /api/admin/ask-human/summary[?accountId]`。

**sendAnalyticsStore.ts（60 行）**：`overview/mediaStats/namecardStats`；`loadOverview` `GET /api/send-ledger/overview?accountId`（31-44）；`loadStats(kind,accountId)` `GET /api/send-ledger/stats?kind=media|namecard&accountId`（45-59）。accountId 为空时清空对应槽。**无 generation 防护**（见 §5-10）。

**userOpsDomainHelpers.ts（177 行）—— 运营域参数编解码纯函数层**
- `QUIET_HOURS_COMPATIBILITY_DEFAULTS {enabled:true,startHour:22,endHour:8,tzOffsetHours:8}`（12-17，与后端兼容默认一致，保存时四字段全落库）。
- `USER_RUNTIME_PARAMETER_FIELDS` 20 项（20-47）：runtime 参数的 key/中文 label/说明/类型/默认值字典——recentMessageLimit=12、minReplyIntervalSeconds=20、maxDailyTouches=3、maxPendingFollowUps=3、followUpExpiresHours=48、cooldownAfterNoReplyHours=24、hallucinationBlockAt=6、knowledgeGroundingBlockBelow=7、humanLikeRewriteBelow=6、emotionalValueRewriteBelow=6、operationStateConfidenceFullReviewBelow=4、runTokenBudget=30000、runMaxLlmCalls=6、simulationTokenBudget=60000、reactionTokenBudget=8000、reactionMaxLlmCalls=2、quietHoursEnabled=true、quietHoursStart=22、quietHoursEnd=8、quietHoursTzOffsetHours=8。
- 文本协议：`runtimeParametersText`（对象→`key = value` 行文本，已知键按字典序先行，70-83）↔`runtimeParametersFromText`（按首个 `=` 拆、值经 `parseParameterValue` true/false/数字/串，49-54,85-98）。`jsonText/jsonFromText` 处理 stateMachine JSON 文本（56-68，解析失败回 `{}`）。
- `quietHoursSettingsFromDraft`（106-131）：从 draft 文本抽四个作息字段（`integerParameter` 越界回落默认，100-104）；`runtimeParametersWithQuietHours`（133-143）把表单四字段写回文本。
- `domainPayload(draft)`（145-158）：draft→PUT 载荷（runtimeParameters 文本转对象、stateMachine 文本转 JSON、assistModeEnabled 透传）；`domainDraftFromConfig/domainDraftsFromConfigs`（160-177）反向。

**userOpsStore.ts（1369 行）—— 用户运营频道主 store**
- **State**（29-96）：模式 `userOpsMode:"smart"|"traditional"|"roster"`、`traditionalOpsTab`；选中联动数据 `messages/operatingMemory/memoryCandidates/memoryDraft/operationHealth/decisionReviews` + **详情作用域三元组** `detailAccountId/detailContactId/detailGeneration`；表单草稿 `searchQuery/profileNote/customAgentInstructions/assistOverride/relationshipType/referredSpecialistAt/referredCardId/profileEditDraft{lastCommitment,followUpPolicy}/guideInstruction/guidePreview/guidePreviewGeneration/simulationInput/simulationTurns/selectedPlaybookId`；剧本区 `playbooks/playbookDraft/generatePlaybookText/optimizePlaybookText/editingPlaybookId/editingPlaybookAccountId/editingPlaybookVersion/playbookScopeAccountId/playbookRequestGeneration`；域配置 `operationDomains/domainDrafts`；`rosterCache`（按 accountId 键控 `{items,syncing,fetchedAt,serverFetchedAt}`，85-88——`serverFetchedAt` 是**服务端快照生成时刻**，用于判断 force 刷新是否落地，注释 82-88）；`contactCounts{all,managed,normal}`（后端真实计数，不受 limit=500 截断，90-91）；`guideBusy/simulationBusy`。
- **记忆表单双向映射**：`MEMORY_DRAFT_GROUPS`（206-238）把 23 个扁平字段归入后端四个 Document（userUnderstanding 7 / relationshipState 6 / productFit 2 / nextAction 8）；`groupMemoryDraft`（241-250）提交方向、`memoryDraftFromMemory`（254-270）回填方向，同一份映射防漂移（注释 203-205）。
- **详情防护谓词 `detailActionIsCurrent`**（313-324）：当前账号非空 ∧ contact.accountId=当前 ∧ contactStore.dataAccountId=当前 ∧ contactStore.selected 的 id+accountId 匹配 ∧ store 自身 detailAccountId/detailContactId 匹配——七条全过才许写。
- **核心 action（端点全列）**：
  - `hydrateSelected(contact)`（421-461）：清空联动数据 + bump detailGeneration + 从 contact 回填草稿（assistOverride/relationshipType/referredSpecialistAt/referredCardId 从 `domainAttributes` dotted-key 取，434-450；lastCommitment/followUpPolicy 回填 profileEditDraft，452-455）。
  - `loadMessages(contact)`（464-514）：前置七条校验（468-475）；`Promise.allSettled` 并发 5 端点——`GET /api/conversations/:contactId/messages?limit=50`、`GET /api/contacts/:id/operating-memory`、`GET /api/contacts/:id/memory-candidates?limit=30`、`GET /api/decision-reviews?accountId&contactId&limit=20`、`GET /api/contacts/:id/operation-health`（476-482）；落地前**再整套校验 detailGeneration/账号/选中**（483-492）；allSettled 逐槽取值失败回空，首个 rejected 报 uiStore（505-511）；随后 `useInboxStore.refreshSummary()`（512-513，请示灯与收件箱共用 summary）。
  - `loadPlaybooks(accountId)`（517-561）：`GET /api/operation-playbooks?accountId`；换账号即清编辑态（520-535）；generation+scope+currentAccountId 校验（540-547）+ **响应账号校验**（548-550）。
  - `loadContacts`（565-567，转发 contactStore 并透传 searchQuery 作 q）；`loadContactCounts` `GET /api/contacts/counts?accountId`（571-581，失败静默保留旧值）；`loadRoster(accountId,{force,revalidate})` `GET /api/contacts/roster?accountId[&force=true]`（588-631：缓存命中直接回；revalidate=绕缓存重读**不带 force**（避免叠加后台单飞拉取，注释 590-593）；仅 `!syncing` 结果落缓存 616-629）；`batchEnable` `POST /api/contacts/batch-enable`（634-639，返回 `{enabled,queued,rejectedSelf?,rejectedNonHuman?}`）；`hideFromPool` `POST /api/contacts/:id/hide-from-pool`（643-647）；`loadDomains` `GET /api/operation-domains`（650-660，同步生成 domainDrafts）。
  - 详情写操作（全走 detailActionIsCurrent，除注明）：`enableAgent` `POST /api/contacts/:id/enable-agent {expectedAccountId,humanProfileNote?,playbookId?}`（663-686）；`disableAgent` `POST …/disable-agent`（688-704，**仅判 selected 非空**）；`saveProfileNote` `PUT …/profile-note`（706-727）；`saveCustomAgentInstructions` `PUT …/custom-agent-instructions`（729-750）；`saveAssistOverride` `PUT …/assist-override {mode}`（752-773）；`saveOperationProfile` `PUT …/operation-profile {playbookId?,relationshipType?,lastCommitment?,followUpPolicy?}`（775-799）；`saveOperatingMemory` `PUT …/operating-memory {expectedAccountId, …四组}`（801-824）；`clearReferral(contactId)` `POST …/clear-referral`（826-839，仅 busy 防护）；`saveManualTags` `PUT …/manual-tags {expectedAccountId,tags}`（841-861）；`analyzeProfile` `POST …/analyze-profile`（863-879，仅判 selected）。
  - 引导流：`previewGuideInstruction(instruction)` `POST /api/user-operations/guide/preview {accountId,contactId,instruction}`（881-935）：guidePreviewGeneration 防护 + 落地前校验 + **响应身份校验** `data.item.accountId/contactId` 不符抛 `guide_preview_identity_conflict`（908-913）；FE-1 直接消费后端构建好的 `health`（919-921，不再前端重建占位分，注释 915-918）。`applyGuidePreview(confirmGlobalImpact?)` `POST /api/user-operations/guide/apply {previewId,expectedAccountId,expectedContactId,candidateHash,confirmGlobalImpact}`（937-1003）：前置校验含 `candidateHash` 存在（942-952）；成功清 preview + 刷联系人 + 重新 hydrate 选中 + 重载详情（976-988）。
  - `runMemoryConsolidation` `POST /api/contacts/:id/memory-consolidation/run` + 回拉 memory-candidates（1005-1023，仅判 selected）；`runDialogueSimulation` `POST /api/user-operations/simulations/dialogue {accountId,contactId,messages[]}`（1025-1056，simulationInput 按行拆）。
  - 剧本：`createPlaybook` `POST /api/operation-playbooks`（1058-1096，scope+generation 校验后清 draft 并重载）；`savePlaybook` `PUT /api/operation-playbooks/:id {accountId,expectedVersion,…}`（1098-1149：**乐观锁 expectedVersion**；前置六条校验，编辑账号漂移即整体清编辑态 1116-1123；成功 set 新 version）；`optimizePlaybook(id)` `POST …/:id/optimize {accountId,expectedVersion,instruction}`（1151-1215：返回**新的非默认候选行**，校验 `data.item.id !== id` 且 `version === editingPlaybookVersion+1`（1184-1187），编辑器整体切到候选身份 1204-1208）；`generatePlaybook` `POST …/generate {accountId,description}`（1217-1267，同款校验后进入编辑态）；`setDefaultPlaybook` `POST …/:id/set-default {accountId,expectedVersion}`（1269-1296）；`editPlaybook/newPlaybookDraft`（1298-1333，本地）。
  - 域配置：`saveOperationDomain(domain,draftOverride?)` `PUT /api/operation-domains/:domain`（1336-1354，载荷经 domainPayload）；`resetOperationDomain` `POST /api/operation-domains/:domain/reset`（1356-1368）。
- 跨 store 依赖：contactStore（selected/loadContacts）、accountStore（currentAccountId）、uiStore（busy/error）、inboxStore（refreshSummary）。

### 2.4 lib/（8 文件全量）

**api.ts（146 行）**
- `LlmUnavailableError extends Error`：kind/retryCount/detail/hint，message 取 hint||detail（api.ts:1-14）。
- `parseApiError(response)`（16-50）：body 先 JSON——`error==="llm_unavailable"` 构造 LlmUnavailableError（20-30）；`error` 字符串 → `Error(json.error)`（31-33）；**非 JSON 防呆**：content-type text/html → `HTTP <status>（服务端未返回 JSON…）`（39-42）；空体或 `<!doctype/<html` 开头 → `HTTP <status>`（43-46）；短纯文本保留前 120 字（47-49）。动机注释：SPA fallback 曾把整段 HTML 塞进 UI（35-37）。
- `api` 对象（52-115）：get/post/put/patch/delete 全部 `if(!response.ok) throw await parseApiError` 后 `response.json()`；post body 可选（62）；`postForm` 不设 Content-Type 让浏览器带 boundary（90-95）；`postRaw` **不抛错**返回 `{ok,status,data|null}`（96-114，供 lock 409 带 payload 等分支——消费方核证：system-strategy、TaxonomyCandidateReviewCard）。**认证零显式处理**：cookie 会话由浏览器自动携带，401 由 main.tsx 拦截器全局兜。
- `openEventSource(url,{onEvent,onError,events})`（119-145）：统一 SSE 订阅封装，error→onError+close；**全库无消费方**（核证 rg 仅 api.ts:119 定义处；见 §5-2）。

**inboxApi.ts（132 行）**
- 类型：`InboxItem`（source/id/accountId?/title/summary/severity/createdAt/ageHours/actionKind:"inline"|"rich"/richComponent?/richParams? + 各源扩展字段，inboxApi.ts:3-26）；`InboxSummary{status:"complete"|"partial"|"error",asOf,counts:Record<string,number|null>,errors,total}`（35-41）。
- `severityRank` high3/medium2/low1/其他0（43-54）；`sortItems` 严重度降序、同级 ageHours 降序、返回新数组（57-63）。
- `fetchInbox` `GET /api/admin/ask-human/inbox`（73-81，items/errors 缺省补空）；`fetchSummary` `GET /api/admin/ask-human/summary`（83-132）：**宽松双形态解析**——有嵌套 `counts` 对象用之，否则把顶层非 status/asOf/errors/total 的数字键当 counts（87-107）；errors 数组逐项形状校验（108-116）；status 非法时按 errors 推导 partial/complete（117-124）。
- 与后端契约松耦合是有意的（summary 属降级数据）。

**useSseReconnect.ts（84 行）**
- 头注红线：**仅用于长连接监听流；严禁用于一次性 RPC 流（如 /knowledge/ask/stream）——重连会重发查询、重复扣 token**（1-2）。
- `createSseReconnector(url, opts)`（25-84）：退避 `delay=min(capMs, baseDelayMs×2^attempt)`，默认 base 1000/cap 30000/maxRetries 6（26-28）；**open 或任一注册业务事件都重置 attempt**（44-55）；`terminalEvents`（如后端推 "close"）→ stop 停止重连（56-63）；error → 清 es，超限 stop+onGaveUp，否则计划重连（64-77）；`es !== next` 旧连接事件防串扰（45,51,60,65）；返回 `{close}` 由调用方卸载时调（81-83）。
- 消费方核证：`features/knowledge/today.tsx`；一次性 RPC 流（knowledge/explore.tsx）用裸 EventSource，符合红线。

**reviewLabels.ts（370 行）—— 英文闭集→中文标签单一真相源**
- `FINAL_REVIEW_STATUS_LABELS` 10 项（6-17，对齐后端 `run_envelope.rs` FINAL_REVIEW_STATUS_VALUES，头注 2-3）；`HOLD_CATEGORY_LABELS` 3 项（19-23）；`REVIEW_PHASE_LABELS` 15 项（25-41）。
- `GATEWAY_STATUS_LABELS`（46-78）：展开 FINAL_REVIEW 后补 gateway 过程态，**39 值闭集由 `gateway_status_values.fixture.json` 对账**（头注 43-45；测试核证 `__tests__/lib/reviewLabels.test.ts:80-84` 遍历 fixture 要求全覆盖且中文）。
- `SEND_OUTCOME_REASON_LABELS`（83-89）：GATEWAY 超集 + campaign 专有 4 值（not_yet_run/failed_terminal/canceled/policy_consecutive_limit，注释引后端 classify_send_outcome）。
- `GAP_SIGNAL_KIND_LABELS` 12 项（92-107，含 citation_format_rejected 与 recall_miss 的修复方向区分注释 103-105）；`GAP_SIGNAL_SEVERITY_LABELS` 4 项（109-114）。
- 请示通道：`ESCALATION_CATEGORY_LABELS` 3 项（117-121）/`ESCALATION_VERDICT_LABELS` 5 项（123-129）/`ESCALATION_RESOLVED_VIA_LABELS` 2 项（131-134）。
- `VERSION_STATUS_LABELS` 4 项（138-143）；`NEXT_BEST_ACTION_TYPE_LABELS` reply/follow_up（147-150）；`REVIEW_SCORE_LABELS` 8 维（154-163）；`SEEDED_BY_LABELS` 5 项 + `seededByLabel()`（167-177）；`PROMPT_LAYER_LABELS` 5 层 + `promptLayerLabel()`（180-190）。
- `EVENT_STATUS_LABELS`（196-227）：GATEWAY 展开 + 通用日志态，历史同义拼写双收（warn/warning、success/succeeded、observe/observed，注释 194-195）。
- `EVENT_KIND_LABELS` **约 130 键**（232-362）：按写入点分组注释（入站 webhook/发送网关/发件箱/主动运营 planner/后台 worker/决策 finalize/记忆知识/自优化 evolution），头注声明「全量取值经逐写入点亲验枚举、无 format! 动态拼接」（229-231）。
- `labelOf(map,value)`：null/undefined/"" → "—"，未知值回落原值不吞（364-370）。

**applyAiRepairPatch.ts（53 行）**：`applyAiRepairPatch(input)` → `POST /api/operation-knowledge/repair/applied`（27-45）载荷含 targetKind:"chunk"/patch/acceptedFields/skippedFields（patch 有而未勾选，24）/confidenceHint/extras/**`thenVerify: false` 恒定红线**（42，头注 3：AI 永不自动 verify，落库只到 draft+needs_review）。用裸 fetch 而非 api 封装；失败返回 `{ok:false,reason:"apply_failed"|"server_error"}` 不抛。`input.originalChunk` 仅为调用方兼容保留（22 `void`）。

**clipboard.ts（71 行）**：`copyText` 两级降级——`navigator.clipboard.writeText`（安全上下文）→ 隐藏 textarea + `document.execCommand("copy")`（不要求安全上下文，但须在用户手势同步链内，头注 8-14,66）；execCommand 路径处理焦点归还（48-59）；返回 boolean 永不抛。动机：生产是纯 HTTP+IP，异步 Clipboard API 不存在（2-6）。

**format.ts（44 行）**：`formatRate`（0..1→"xx.x%"，NaN/null→"—"，7-10）；`formatNumber`（定点，12-15）；`formatTimestamp`（27-44）：字符串原样；对象按 BSON 扩展 JSON `{"$date":…}` 三形态脱壳（string/number/$numberLong）；数字→ISO；**第二道防线**——后端契约是 RFC3339 字符串，裸 bson::DateTime 泄漏时避免 React child 崩频道白屏（头注 17-26，曾在 domain-profiles tab 实崩）。

**uuid.ts（78 行）**：`randomUuid` 三级降级——`crypto.randomUUID`（安全上下文）→ `crypto.getRandomValues` 手拼 RFC4122 v4（26-33）→ `Math.random`+时间戳混入（36-52）；每级 try/catch（有宿主把 randomUUID 定义成会抛的 stub，注释 56-59）；仅用于前端操作命名（sessionId 类），**禁用于 token/密钥**（17-18）。

### 2.5 types/index.ts（842 行）

跨 feature 共享类型单一来源（头注 1）。要点：
- `Channel` 20 值联合（types/index.ts:4-24）；`AgentStatus "normal"|"managed"`（3）；`ContactTab/TraditionalOpsTab/UserOpsMode("smart"|"traditional"|"roster")/OpsTab`（25-28）。
- `Account`（30-41）：id/accountId/alias/displayName/appId?/wxid?/nickName?/mcpKeyConfigured?/online/status?。
- **标签可信度三层**：`Evidence{turn,msgId}`（51）、`ConfirmedTag`（53，AI 确信层每条带证据）、`BayesianPoint/BayesianSignal`（55-70，append-only 旁路、永不驱动行为）、`PersonalityFacet/Snapshot/Profile`（72-84，大五 OCEAN 慢变量）。
- `Contact`（86-137）：核心运营对象——agentStatus/humanProfileNote/customAgentInstructions/agentProfile/playbookId(+Version)/tags + manualTags/confirmedTags/bayesianSignals/personalityProfile 分层标签 + `domainAttributes: Record<string,unknown>`（110，assist_mode_override/relationship_type/referred_* 都藏在这）+ stagnation 三字段（113-115）+ `commitments/lastCommitment/followUpPolicy`（117-119）+ `operationState(+Reason/Confidence/UpdatedAt)/cooldownUntil/operationPolicy/profileAttributes` + `lastInboundAt/lastOutboundAt/lastMessageAt/lastInboundPreview`（129-135）。
- `RosterEntry`（140-148）：agentStatus 增 `"not_imported"`。`Message`（158-168）：msgType "text"|"media"|"namecard" + mediaRef。`SendHistoryItem`（172-180）。`ReferralCard(+Draft)`（183-205）。`EventItem/TaskItem`（207-231）。`ContentAsset`（233-256，媒体文件字段 + reviewStatus/sendable/expressionPref）。`AgentSoul`（258-271）。`CommandToolCall/CommandResult`（273-291，planHash SHA-256 冻结注释 286）。`LlmUsageItem/LlmUsageResponse`（293-330，含 cache hit/miss 与 usageComplete）。`AutonomyProtocol`（332-342，九个自述字段）。`DecisionReview`（344-362，scores/risks/finalReviewStatus/reviewPhase/holdCategory）。`AgentRunItem`（366-382，**各阶段是后端 Document、前端 key-value 渲染兜底不写死字段名**，注释 364-365）。`PromptTemplate(+Draft)`（384-406）。`OperationPlaybook(+Draft)`（408-442，releaseStatus/version 乐观锁）。`OperatingMemory`（444-457，四 Document + memoryCard/contextPack 版本对）。`MemoryCandidateItem`（459-469）。`OperatingMemoryDraft` 23 扁平字段（471-495）。`OperationHealth(+Item)`（497-508）。`UserOperationGuidePreview`（510-536：candidateHash/authoritativeChanges/requiresStrongConfirmation/playbookAffectedContacts；`health?` 可选因后端兼容期，注释 522-525；healthScores 遗留保留 526-527）。`GuideSkippedField/GuideAuthoritativeChange/UserOperationGuideApplyResult`（538-556）。`SimulationTurn`（558-571）。`DomainKey` 三域（573）。`OperationDomainConfig(+Draft)`（575-607，Draft 中 runtimeParameters/stateMachine 是**文本**）。
- **DomainProfile 段**（609-803）：头注声明后端 `serde_json::to_value` 序列化 → **snake_case JSON**（610-611，与其余 camelCase API 的关键差异）；`ProfileDimension/BusinessFormula(eval_score_key 漂移护栏注释 623-625)/CommitmentMarkers/CoverageDimension/ChunkRole/MemoryDimension/OutcomePolarity/FunnelMode/SilenceMode/CommitmentMode/QuietHoursMode/OperationMode/ReviewerOrientation(690-694，字段是 camelCase!)/TrajectoryDimension/ProfileThresholds(702-710，camelCase，rename_all)/GeneratedState(Machine)`（712-728：**外层 snake_case 但内层 key 保留 camelCase** 的特例，注释 713-716）；`DomainProfile` 全字段（730-773）与 `DomainProfileDraft`（775-803，**无 generated_state_machine**，见 §5-4）；`GenerateProfileRequest/Response`（805-817）。
- 请示通道：`DeciderRef/AskHumanQuietHours/AskHumanPolicy`（819-842，camelCase serde，P3 配置页 + P2 收件箱共用）。

### 2.6 components/

**LlmErrorBanner.tsx（110 行）**：`LLM_KIND_LABELS` 13 kind 中文（11-25）；`normalizeError`（27-55）——LlmUnavailableError 原样、**普通 Error 归为 `client_error`**（请求可能根本没发出去，不冒充上游故障，注释 36-40）；`retryLabel`（60-66）client_error 显示调用方动作名（默认「重试」）而非「AI 重试」；渲染 kind 徽标+自动重试次数+hint+可折叠 detail+重试按钮（82-109）。

**prompt/usePromptSaveConfirm.tsx（122 行）**：`promptDiffBody(reason,diff)` 共享 diff 核对块（17-43，深色 pre）；`usePromptSaveConfirm()`（45-80）返回 runSave：调 strategyStore.savePromptTemplate → needsConfirm/rejected 两分支都弹 `useConfirm` danger 框、**requireText:"已核对"** 打字解锁 → force 重提（55-78）；`usePromptPublishConfirm()`（87-122）publish 同款。须在 ConfirmProvider 内。

**review/（结构级）**：ChunkReviewCard / LessonPromoteCard / ProfilePublishCard / ProposalReleaseCard(+module.css) / ReviewQueue / TaxonomyCandidateReviewCard(+module.css) / evidenceMetrics.ts / proposalPrimitives(.tsx/.module.css) / proposalTypes.ts ——统一收件箱 rich 卡片与演化发布卡片族（详读归 features 篇）。

**ui/ 共享组件（12 组 + tokens/reset）**
- `tokens.css`（96 行）：全站唯一变量源，禁止组件硬编码色值（tokens.css:1）。6 语义色 running #30D158 / scheduled #0A84FF / held #FF9F0A / blocked #FF453A / brand #5E5CE6 / inactive #8E8E93 + 对应半透明 fill（3-17）；ink 四阶（20）；surface-page #eef1f5 / card #fff / hairline（23）；圆角 11/18/24（26）；**4pt 间距刻度 sp-1..sp-8**（33，含「为何不替换旧值」的注释 29-32）；字号五档 micro10/label11/body13/group13.5/lead16（37-41）；行高 tight1.25/body1.45（44）；导航轨 20px+gap12+行高 36px+r-nav 8px+icon 墨色 #9a9aa1+hover/active 淡底（49-66，激活态只用「淡底+字重」两信号的决策注释 62-64）；阴影三档（70-72）；侧栏纯色 #f7f8fa（76）；`--focus-ring` 双层 + `:focus-visible` 全局键盘焦点环（79,87-90）；z-index overlay 1000/toast 1100 + scrim（82-83）；`breathe-running` 呼吸动画仅「进行中」语义（93-96）。
- `reset.css`（3 行）：box-sizing + body margin 0 + 字体平滑（唯一允许裸标签全局选择器的文件之二，注释 1）。
- `Avatar`（6 行）：首字母 + StatusTone 着色 + live 呼吸（Avatar.tsx:4-6）。
- `ChunkRef`（132 行）：`shortId` 全局唯一 ID 短显规则 `xxxxxx…xxxx`（6-9，终结三套并存）；`ChunkRef` 无 onFocus 渲染 span、有则渲染按钮聚焦 Inspector（13-35，onFocus 注入保持 ui 层不依赖 features）；`ChunkPicker`（44-132）：搜索式选择器替代手输 ObjectId，open 时惰性 `loadChunks()` 一次（61-67），title/id 双字段过滤取前 20（78-84），点外关闭（69-76）。
- `ConfirmDialog`（97 行）：Context+Provider 模式，`useConfirm()` 返回 promise 化 confirm（27-33）；`requireText` 危险守护——输入完全一致才解锁（11-13,42,59-72）；danger 时 `closeOnScrim=false`（51）。
- `EmptyState`（18 行）：icon(默认 Inbox)/title/hint/action。
- `FormDialog`（143 行）：promise 化表单弹窗替代 window.prompt（32-33）；FormField 四 kind：text/textarea/select/chunkRef（6-9），chunkRef 内嵌 ChunkPicker、loadChunks 注入（105-110）；required 未填禁 submit（55-59）。
- `FriendPickerModal`（153 行）：好友选择器——remark/nickname/wxid 三字段过滤（48-54），PAGE_SIZE=60 分页（14,56-58），计数反馈（74-78），头像回退首字母（99-103），`allowManualWxid` 手输 wxid 入口（116-140）；maxWidth 720 双列网格（63-65）。
- `MetricCard`（16 行）：label/value/detail 可点卡片。
- `Overlay`（103 行）：所有弹窗底座——portal 到 body、role=dialog+aria-modal、**focus-trap**（Tab 循环 51-66）、Esc 关闭（45-49）、scrim 点击关闭可配（84-86）、进场焦点移入/退场焦点归还（37-43,75）、body overflow 锁定（70-74）、maxWidth 可覆盖默认 480（25-27）。
- `PlanStep`（20 行）：ready/pending 步骤行。
- `StatusBadge`（11 行）：`StatusTone` 五值闭集 running/scheduled/held/blocked/inactive（3）＝tokens 六色去 brand。
- `StatusLine`（16 行）：label/value 行，tone ai/good/neutral/warn。
- `Toast`（77 行）：Context+Provider，success 3s/error 6s/info 4s 自动消失（22），portal + `role="status" aria-live="polite"`（53）；**持久错误不走这里，用 LlmErrorBanner**（注释 24-25）。
- 各 index.ts 桶文件仅 re-export（每个 1-2 行）。

### 2.7 contracts/ 全清单与对账机制

**机制**（双向对账，三个环节）：
1. 后端 `src/routes/contract_snapshot.rs:45-75` `assert_contract_fixture(name, value)`：构造全量 model→调投影→`UPDATE_SNAPSHOTS=1 cargo test --lib` 时**写** `frontend/src/contracts/<name>.fixture.json`（bless），平时只读比对，形状漂移即 Rust 测试红。
2. fixture JSON 是**前后端唯一真相源**（contract_snapshot.rs:5）。
3. 前端 `.contract.ts` 手工声明 `CANONICAL_KEYS`；vitest 契约测试把 fixture 键集与声明**双向**比对（`missingInFrontend`=后端发了没声明 / `deadInFrontend`=声明了后端没发），任何一侧漂移测红（`__tests__/contracts/operationKnowledgeChunk.contract.test.ts:13-20`、`operationsDomain.contract.test.ts:25-39`）。

**38 个 .contract.ts 全清单**（契约 → 后端投影函数[出自各文件头注] → 覆盖测试）：

| # | contract 文件 | 后端投影 / 闭集 | 键数 | 覆盖测试（__tests__/contracts/） |
|---|---|---|---|---|
| 1 | agentRun.contract.ts:2-18 | agent_run_json | 15 | operationsDomain |
| 2 | behaviorSignalMetric.contract.ts:2-11 | behavior_signal_metric_json | 8 | operationsDomain |
| 3 | decisionReview.contract.ts:2-34 | decision_review_json | 30 | operationsDomain |
| 4 | evaluationScenario.contract.ts:3-16 | evaluation_scenario_json | 12 | configPlaybookDomain |
| 5 | experimentEnvelope.contract.ts:2-17 | experiment_envelope_json | 14 | evolutionDomain |
| 6 | experimentSummary.contract.ts:3-7 | experiment_summary_json（聚合） | 3 | evolutionDomain |
| 7 | guideApplyReceipt.contract.ts:2-10 | receipt_json | 7 | operationsDomain |
| 8 | guidePreview.contract.ts:2-24 | guide_preview_json | 21 | operationsDomain |
| 9 | importJobProgress.contract.ts:4-10 | import_job_progress_json | 5 | knowledgeDomain |
| 10 | ingestSource.contract.ts:5-23 | ingest_source_json | 17 | knowledgeDomain |
| 11 | knowledgeUsageLog.contract.ts:4-17 | knowledge_usage_json | 12 | knowledgeDomain |
| 12 | llmCallLog.contract.ts:2-21 | llm_call_log_json | 18 | operationsDomain |
| 13 | memoryCandidate.contract.ts:2-15 | memory_candidate_json | 12 | operationsDomain |
| 14 | operatingMemory.contract.ts:2-15 | operating_memory_json | 12 | operationsDomain |
| 15 | operationDomain.contract.ts:3-24 | operation_domain_json | 20 | taxonomyDomain |
| 16 | operationHealth.contract.ts:4-10 | operation_health 聚合（含作息灯 inQuietHours/nextWakeAt/quietHoursEnabled） | 5 | operationsDomain |
| 17 | operationKnowledgeChunk.contract.ts:13-47 | operation_knowledge_chunk_json | 33 | operationKnowledgeChunk（独立） |
| 18 | operationKnowledgeChunkDetail.contract.ts:6 | 同上（详情包裹） | 1（item） | knowledgeDomain |
| 19 | operationKnowledgeDocument.contract.ts:3-22 | operation_knowledge_document_json | 18 | knowledgeDomain |
| 20 | operationStateAction.contract.ts:2-18 | **值闭集**（非键集）：reply/acknowledgement/silent/follow_up/cooldown + 中文标签 | 5 值 | operationStateAction（独立） |
| 21 | operationStatePolicy.contract.ts:2-16 | operation_state_policy_json | 13 | taxonomyDomain |
| 22 | outboxEntry.contract.ts:2-32 | outbox_entry_json | 28 | configPlaybookDomain |
| 23 | outboxPayload.contract.ts:2-7 | outbox_payload_json（media） | 4 | configPlaybookDomain |
| 24 | outcomeMetric.contract.ts:2-14 | outcome_metric_json | 11 | operationsDomain |
| 25 | playbook.contract.ts:2-22 | playbook_json | 19 | configPlaybookDomain |
| 26 | promptTemplate.contract.ts:2-16 | prompt_template_json | 13 | configPlaybookDomain |
| 27 | proposalDetail.contract.ts:2-32 | proposal_detail_json | 28 | evolutionDomain |
| 28 | proposalSummary.contract.ts:2-17 | proposal_summary_json | 14 | evolutionDomain |
| 29 | relationshipSuggestion.contract.ts:2-16 | relationship_suggestion_json | 13 | taxonomyDomain |
| 30 | revisionApplied.contract.ts:4-12 | revision_applied_to_json | 7 | knowledgeDomain |
| 31 | runtimeFlag.contract.ts:2-10 | runtime_flag_json | 7 | evolutionDomain |
| 32 | shadowReplay.contract.ts:2-18 | shadow_replay_json | 15 | evolutionDomain |
| 33 | suspectedDeal.contract.ts:2-16 | suspected_deal_json | 13 | configPlaybookDomain |
| 34 | taxonomyCandidate.contract.ts:2-17 | taxonomy_candidate_json | 14 | taxonomyDomain |
| 35 | taxonomyEntry.contract.ts:3-14 | taxonomy_entry_json | 10 | taxonomyDomain |
| 36 | thresholdOverride.contract.ts:2-11 | threshold_override_json | 8 | evolutionDomain |
| 37 | thresholdOverrideAudit.contract.ts:2-15 | threshold_override_audit_json | 12 | evolutionDomain |
| 38 | toolCall.contract.ts:2-9 | tool_call_json | 6 | configPlaybookDomain |

覆盖核证：7 个契约测试的 import 面逐一核过（operationsDomain 10 契约、knowledgeDomain 6、configPlaybookDomain 7、evolutionDomain 8、taxonomyDomain 5、operationKnowledgeChunk 1、operationStateAction 1 = 38，无遗漏）。

**无 .contract.ts 的 fixture（4 个）**：
- `gateway_status_values.fixture.json`（39 值数组）：由 `__tests__/lib/reviewLabels.test.ts:2,80-84` 消费——校验 GATEWAY_STATUS_LABELS 全覆盖；后端 bless 点 `src/agent/run_envelope.rs`（gateway_status_values_match_frontend_contract_fixture 测试）。
- `domain_profile.fixture.json` / `knowledge_chat_turn.fixture.json` / `worker_control.fixture.json`：后端 bless（`src/routes/domain_profiles.rs:1632`、`src/routes/knowledge/chat.rs:3716`、`src/routes/worker_controls.rs:131`）但**前端无任何 import**（rg 全库核证零命中）→ 单侧锁形，见 §5-3。

### 2.8 App 依赖的 features 边界文件

**features/knowledge/chunkInvalidation.ts（43 行）**：`CHUNKS_INVALIDATED_EVENT="wikiChunksInvalidated"`（3）；`invalidateChunks(detail{reason:"revised"|"lagged"|"local",chunkId?,revisionKind?})` 派发 window 事件（11-14）；`useCoalescedReload(loadOnce)`（21-43）——**合并突发失效**：运行中再来失效只置 pending，尾随一次 reload 保证事件不丢且并发列表请求有界（while(pending) 循环 31-36，running promise 复用 27-29）。

**src/EvolutionCenterTab.tsx（22 行）**：纯 re-export `features/evolution/EvolutionCenterTab` 的组件/工具/类型（3-22），保既有测试 import 路径不破。

---

## 3. 跨文件机制

### 3.1 认证态生命周期与 401 处理
1. 模块加载即 monkey-patch `window.fetch`（main.tsx:14-24）；`originalFetch` 留给 auth 自身端点用（45,56,125,154,180）。
2. 首屏 AuthGate `GET /api/auth/me`：200→进 AuthedApp 并 setItem `wa.authed`；非 200→LoginScreen（125-134）。
3. 登录：`POST /api/auth/login` → `GET /api/auth/me` → `onLoggedIn`（40-63）。会话载体是后端 `wa_session` cookie（浏览器自动带，前端零 token 管理）。
4. 运行期任一 `/api/*`（除 /api/auth/login）返回 401 → 拦截器清 sessionStorage + 派发 `wa-auth-expired` → AuthGate 监听置 `me=null` → 渲染 LoginScreen（16-21,144-150）。**store 里的旧数据不清**——回登录页后组件树卸载，重新登录后 `window.location` 未刷新（同 SPA 实例），各 store 靠账号作用域防护自愈。
5. 登出：`POST /api/auth/logout`（失败也清本地）→ setMe(null)（152-160）。
6. `wa.authed` sessionStorage 键只写不读（见 §5-1）。

### 3.2 workspace 切换
- `me.workspaces/currentWorkspace` 由 `/api/auth/me` 下发（main.tsx:26-31）→ authStore.user（171-190）。
- Shell 仅当 `workspaces.length > 1` 渲染 WorkspaceSwitcher（Shell.tsx:215-217,307-311），否则纯文本。
- 切换 = `POST /api/auth/workspace {workspaceId}` 成功后 **`window.location.reload()` 全页重载**（main.tsx:180-185）——所有 store/WS/SSE 随页面销毁重建，无需逐一失效（最简一致性策略）。

### 3.3 chunk WebSocket 事件流（知识协作）
- 单连接：App 顶层 `useChunkEventStream` 连 `/api/ws/chunks`（App.tsx:79-80），全进程共享。
- 事件扇出：WS envelope（snake_case）→ `dispatchChunkEvent` → 三个 window CustomEvent（`wikiChunkLocked/Unlocked/Revised`）+ 失效总线 `wikiChunksInvalidated`（revised/lagged 触发，App.tsx:51,61-65 → chunkInvalidation.ts:11-14）。
- 消费侧用 `useCoalescedReload` 合并重载（chunkInvalidation.ts:21-43）；`lagged`（服务端广播积压）触发整体失效兜底。
- 重连：1s 起倍增至 30s cap，onopen 重置（App.tsx:87-89,112-119）；失败不阻塞业务（注释 19-20）。

### 3.4 SSE 策略（与 WS 互补）
- 监听型长流（可幂等重连）：`createSseReconnector`——指数退避 1s→30s、6 次封顶、任一业务事件重置计数、terminalEvents 优雅收尾（useSseReconnect.ts:25-84）；消费方 knowledge/today.tsx。
- 一次性 RPC 流（重连=重发查询、重复扣 token）：**禁用重连器**（useSseReconnect.ts:1-2），knowledge/explore.tsx 用裸 EventSource。
- `api.ts` 的 `openEventSource` 是第三套封装但无人使用（§5-2）。

### 3.5 账号作用域与竞态防护（全站范式）
- 唯一裁判：`accountStore.currentAccountId()`（accountStore.ts:24-29）。
- 四层防护梯度（按数据敏感度递增）：
  1. **无防护**：workspace 级列表（referralCardStore、campaignStore.loadCampaigns、strategyStore、sendAnalyticsStore）。
  2. **generation + scope + currentAccountId 三重校验**：contactStore.loadContacts（88-93）、contentStore.loadAssets（128-132）、operationsStore.loadOperationsData（110-114）、campaignStore.loadReport（88-92）、userOpsStore.loadPlaybooks（540-547）。
  3. **响应体身份校验**（不信后端路由正确性）：contacts（contactStore.ts:95-101）、assets（contentStore.ts:134-138）、tasks（operationsStore.ts:115-120）、playbooks（userOpsStore.ts:548-550）、campaign report `campaignId===id`（campaignStore.ts:91）、guide preview `accountId/contactId`（userOpsStore.ts:908-913）、command 结果账号（commandStore.ts:96-100）。
  4. **写操作前置谓词 + 服务端 CAS 双保险**：`detailActionIsCurrent` 七条（userOpsStore.ts:313-324）/`actionIsCurrent`（contentStore.ts:79-88）/`taskActionIsCurrent`（operationsStore.ts:47-56）＋载荷回传 `expectedAccountId`/`expectedScope`/`expectedVersion`/`planHash`/`candidateHash` 由后端终审。
- 乐观锁样本：playbook `expectedVersion`（userOpsStore.ts:1131-1135）；命令计划 `planHash`（commandStore.ts:134-137）；引导 `candidateHash`（userOpsStore.ts:964）。

### 3.6 store 间依赖图（getState 直调，无订阅环）
```
main.tsx ──▶ authStore（setUser/setHandlers）
App.tsx ──▶ accountStore.setAccounts；profileStore.load*；uiStore.setError
Shell.tsx ─▶ navigationStore / authStore / accountStore / profileStore(activeProfile→visibleWhen)
contactStore ──▶ accountStore(currentAccountId)、uiStore(error)
contentStore ──▶ accountStore、uiStore
operationsStore ─▶ accountStore、uiStore
commandStore ──▶ accountStore、uiStore
campaignStore ──▶ uiStore、navigationStore(setChannel 跨频道跳转)
strategyStore / referralCardStore / sendAnalyticsStore ──▶ uiStore
userOpsStore ──▶ contactStore(selected/loadContacts)、accountStore、uiStore、inboxStore(refreshSummary)
inboxStore ──▶（仅 lib/inboxApi）
profileStore / navigationStore / authStore / accountStore / uiStore ──▶ 无 store 依赖（叶）
```
- 唯一「store 主动改导航」的点：`campaignStore.openReport` → `navigationStore.setChannel("campaign")`（campaignStore.ts:74）。
- 唯一「业务 store 触发另一业务 store 网络请求」的点：userOpsStore.loadMessages 尾部 `inboxStore.refreshSummary()`（userOpsStore.ts:512-513）。

### 3.7 错误处理分层
1. HTTP 层：`parseApiError` 统一脱壳（JSON error 字段 / llm_unavailable 专类 / HTML 防呆）（api.ts:16-50）。
2. 全局层：store catch → `uiStore.setError` → GlobalErrorBanner（role=alert）。
3. 专用层：LLM 故障 → `LlmUnavailableError` → LlmErrorBanner（kind 中文 + AI 重试；client_error 与上游故障区分）。
4. 三态层：prompt/profile 保存的 needs_human_confirm（200 非成功）与红线拒绝（4xx 特定文案）走**返回值**而非 setError（strategyStore.ts:232-254），由 usePromptSaveConfirm 弹 requireText 确认框。
5. 降级层：inbox summary 失败留旧快照（inboxStore.ts:55-57）、contactCounts 失败保旧值（userOpsStore.ts:578-580）、profileStore 失败照常跑（54-61,70-77）、瞬时提示走 Toast 不走横幅（Toast.tsx:24-25）。

---

## 4. 事实卡速查

### 4.1 store → 端点全映射

| store | 方法 | 端点 |
|---|---|---|
| （main.tsx） | 登录/校验/登出/切 workspace | POST /api/auth/login；GET /api/auth/me；POST /api/auth/logout；POST /api/auth/workspace |
| （App.tsx） | 启动引导 | GET /api/accounts；WS /api/ws/chunks |
| （Shell.tsx） | syncAccounts | POST /api/accounts/sync；GET /api/accounts |
| profileStore | loadActiveProfile / loadActiveView | GET /api/admin/domain-profiles/active；GET /api/operation/active-view |
| contactStore | loadContacts | GET /api/contacts?accountId&limit=500[&q] |
| commandStore | loadCommandData / runCommand / confirm / reject | GET /api/content-assets[?accountId]；GET /api/agent-souls；GET /api/tasks[?accountId]；POST /api/management-agent/sessions；POST /api/management-agent/sessions/:id/messages；POST /api/management-agent/commands/:id/confirm｜/reject |
| contentStore | 全 8 action | GET/POST /api/content-assets；POST /api/content-assets/upload；POST /api/content-assets/:id/review｜/file｜/toggle；PUT /api/content-assets/:id；DELETE /api/content-assets/:id?expectedScope… |
| referralCardStore | 全 5 action | GET/POST /api/referral-cards；POST /api/referral-cards/:id/review｜/toggle；DELETE /api/referral-cards/:id |
| campaignStore | loadCampaigns / loadReport | GET /api/campaigns；GET /api/campaigns/:id/sends |
| operationsStore | loadOperationsData / loadAgentRuns / 任务动作 | GET /api/events、/api/tasks、/api/decision-reviews、/api/llm-usage、/api/agent-runs（均 ?accountId）；POST /api/agent-tasks/:id/review-now｜/cancel |
| inboxStore | load / refreshSummary | GET /api/admin/ask-human/inbox[?source&accountId]；GET /api/admin/ask-human/summary[?accountId] |
| sendAnalyticsStore | loadOverview / loadStats | GET /api/send-ledger/overview?accountId；GET /api/send-ledger/stats?kind&accountId |
| strategyStore | souls/prompts/profiles 全流 | GET/POST /api/agent-souls；PUT /api/agent-souls/:id；POST /api/agent-souls/:id/publish；GET/POST /api/prompt-templates；PUT /api/prompt-templates/:id；POST /api/prompt-templates/:id/publish；POST /api/prompt-templates/reset-system-pack；GET/POST /api/admin/domain-profiles；POST …/generate；PUT …/:id；POST …/:id/publish｜/activate；DELETE …/:id |
| userOpsStore | 详情联动 | GET /api/conversations/:id/messages?limit=50；GET /api/contacts/:id/operating-memory｜/memory-candidates?limit=30｜/operation-health；GET /api/decision-reviews?accountId&contactId&limit=20 |
| userOpsStore | 池/通讯录/域 | GET /api/contacts/counts?accountId；GET /api/contacts/roster?accountId[&force]；POST /api/contacts/batch-enable；POST /api/contacts/:id/hide-from-pool；GET /api/operation-domains；PUT /api/operation-domains/:domain；POST /api/operation-domains/:domain/reset |
| userOpsStore | 联系人写操作 | POST /api/contacts/:id/enable-agent｜/disable-agent｜/clear-referral｜/analyze-profile｜/memory-consolidation/run；PUT /api/contacts/:id/profile-note｜/custom-agent-instructions｜/assist-override｜/operation-profile｜/operating-memory｜/manual-tags |
| userOpsStore | 引导/模拟/剧本 | POST /api/user-operations/guide/preview｜/apply；POST /api/user-operations/simulations/dialogue；GET/POST /api/operation-playbooks；POST …/generate；PUT …/:id；POST …/:id/optimize｜/set-default |
| lib/applyAiRepairPatch | applyAiRepairPatch | POST /api/operation-knowledge/repair/applied（thenVerify 恒 false） |

### 4.2 契约 fixture ↔ 后端投影映射
见 §2.7 表（38 契约 + 4 无契约 fixture）。机制一句话：**后端 `UPDATE_SNAPSHOTS=1` bless fixture → 前端 CANONICAL_KEYS 双向对账 → 任一侧漂移 vitest 红**。

### 4.3 本地持久化键
| key | 存储 | 内容 | 位置 |
|---|---|---|---|
| `wechatagent.accountId` | localStorage | 选中账号 id | accountStore.ts:4 |
| `wa.nav.collapsed.v4` | localStorage | 被收起的分组名数组（组名改名必须轮换 key） | navigationStore.ts:31 |
| `wa.authed` | sessionStorage | 登录标记（只写不读，§5-1） | main.tsx:12 |

### 4.4 设计 token 要点
- 六语义色（running/scheduled/held/blocked/brand/inactive）+ 半透明 fill；StatusBadge tone=去 brand 的五值（tokens.css:3-17；StatusBadge.tsx:3）。
- 4pt 间距刻度（sp-1..sp-8）、五档字号（10/11/13/13.5/16）、三档阴影、圆角 11/18/24 + 导航专用 8（tokens.css:26-72）。
- 呼吸动画仅「进行中」语义（93-96）；键盘焦点环全局 :focus-visible（87-90）；overlay z 1000 / toast z 1100（82-83）。
- 组件禁止硬编码色值，唯一变量源 tokens.css（1）。

### 4.5 其他速查
- 频道初始值 `command`（navigationStore.ts:111）；默认收起组：知识资产/平台配置/建设规划（67-71）。
- runtime 参数 20 项默认值表：userOpsDomainHelpers.ts:20-47（§2.3）。
- 记忆表单 23 字段 ↔ 四 Document 分组：userOpsStore.ts:206-238。
- 中文标签表 13 张：reviewLabels.ts（§2.4）+ OPERATION_STATE_ACTION_LABELS（operationStateAction.contract.ts:12-18）+ LLM_KIND_LABELS（LlmErrorBanner.tsx:11-25）。

---

## 5. 偏差与疑点

1. **`wa.authed` 只写不读（dead state）**：main.tsx 六处 setItem/removeItem（12,19,62,129,132,158），全库无 `getItem`（rg 核证）。注释宣称「开关用 sessionStorage（重启 tab 也能复现）」（10-11），但真实登录判定完全依赖 `/api/auth/me`，该键无消费者——注释与实现漂移。
2. **`openEventSource` 是 dead code**：api.ts:119-145 导出，rg 全库仅命中定义处；SSE 实际走 `createSseReconnector`（today.tsx）与裸 EventSource（explore.tsx）。头注「替代散落的裸 EventSource」未兑现。
3. **三个 fixture 无前端对账**：`domain_profile` / `knowledge_chat_turn` / `worker_control` 由后端 bless（domain_profiles.rs:1632、knowledge/chat.rs:3716、worker_controls.rs:131）但前端无 .contract.ts 也无测试 import（rg 零命中）——只锁了后端侧形状，「后端删字段强制前端清理」的那半闭环缺失。
4. **`DomainProfileDraft` 缺 `generated_state_machine`**（types/index.ts:775-803 vs DomainProfile 761）：`editDomainProfile` 不搬该字段、`saveDomainProfile` PUT 不回传（strategyStore.ts:374-455）。~~更新时后端是否保留该字段，前端侧无法证实——疑点~~ → **【主会话已核证不成立 2026-08-13】后端 PUT 是部分更新：`src/routes/domain_profiles.rs:1149-1153` 明确"从 body 剥离后端管理键后只 `$set` 出现的内容键，未编辑字段原值保持"（该形态正是修复 `replace_one` 整行覆盖丢字段的历史问题后的结果），测试 `:1341` 锁定"body 只带一个字段 → set_doc 只含那一个键"。前端不回传 `generated_state_machine` 不会丢 AI 状态机草稿；该字段仅经 generate 写入、activate 消费（`:904-907`），设计上本就不经人工编辑轨道。**
5. **detail 写操作防护强度不一**：`disableAgent`（userOpsStore.ts:691）、`analyzeProfile`（866）、`runMemoryConsolidation`（1007）只判 `selected` 非空，`clearReferral`（826-828）接显式 contactId 无谓词；同类的 enableAgent/saveProfileNote 等走七条 `detailActionIsCurrent`。快速切账号窗口内，前三者理论上可对旧选中发写请求（服务端有无 expectedAccountId 兜底？disableAgent 载荷为空 `{}`（697）——**疑点**，依赖后端按 contactId 自身账号归属校验）。
6. **sendAnalyticsStore / campaignStore.loadCampaigns / strategyStore 无 generation 防护**：快速切换账号/连点时晚到响应可覆盖新数据（sendAnalyticsStore.ts:31-59；campaignStore.ts:108-118）。workspace 级或低频页面，风险低但与 §3.5 范式不一致。
7. **401 拦截器 URL 判定的边界**：`url.startsWith("/api/")`（main.tsx:18）——若某处以绝对 URL（`http://…/api/…`）构造 Request 则不触发 forceLogout。当前 api.ts 全部相对路径字符串，实际无此调用；纯边界事实。
8. **`api.delete`/`post` 等强制 `response.json()`**（api.ts:56,64,73,88）：后端若对写操作返回 204 空体会在成功路径抛 JSON 解析错。现约定所有端点返回 JSON 体（前端侧未逐端点核证后端）——潜在契约脆弱点。
9. **referralCardStore.loadCards 不带 accountId**（referralCardStore.ts:36），而 `createCard` 却可传 accountId（59）：列表是 workspace 全量、创建可账号定向——读写作用域不对称是有意设计（名片 accountId 可空，types/index.ts:186），但列表页按账号过滤只能靠前端（本 store 未做）。
10. **`visibleWhen` 全体未使用**：ChannelDef 提供 profile 谓词（channels.ts:105-107），20 频道无一定义（注释自认「本期无频道使用，留作扩展点」）；Shell 的 groupItems 过滤当前恒真（Shell.tsx:210-213）。留意勿当成已生效的行业化开关。
11. **演示文案硬编码为初始 state**：commandStore.commandDraft（36）、userOpsStore.simulationInput（352）、generatePlaybookText/optimizePlaybookText（358-359）——刻意的引导性默认值，非 bug；但换行业部署时这些中文示例会原样出现。
12. **WS 事件 snake_case vs REST camelCase**：ChunkEventEnvelope 用 snake_case（App.tsx:24-44），contracts 全 camelCase，DomainProfile 域又是 snake_case（types/index.ts:610-611），GeneratedStateMachine 外 snake 内 camel（713-716），ReviewerOrientation/ProfileThresholds 在 snake_case 家族里又是 camelCase（690-694,702-710）。命名制度有四套并存，均有注释背书，但极易写错——依赖契约测试兜底的面只覆盖 camelCase 投影。
13. **`ContactTab` 类型 vs userOps 实际 tab**：contactStore.contactTab: ContactTab（"all"|"managed"|"normal"，types 25）与 userOpsStore.contactCounts 三键同构——一致；但 `TraditionalOpsTab` 含 "prompts"|"settings"|"audit"（types 26）的消费面在 features（本篇未核）。
14. **StrictMode 双跑防护只盖 App 引导**：accountsBootstrapRef（App.tsx:145-148）防的是同一 effect 双执行；`useChunkEventStream` 无此防护但清理函数完备（双连接建立后立刻被清理，可接受）。
15. **`inboxStore.load` 的 errors 合并**依赖 `refreshSummary` 先完成写 state（Promise.all 中并发，71 行读 `get().summary`）——refreshSummary 在 all 内 resolve 后其 set 已落，时序正确；但若 summary 请求晚于 inbox 且 generation 已换，读到的是旧 summary errors——降级数据可容忍（注释 65）。
16. `walkthrough.py`（frontend 根）与 `styles.css`、`Shell.module.css`、各 `*.module.css`、`features/`、`__tests__/` 主体不在本篇逐行范围（features 深读见 `14-frontend-features.md`）。

---

## 6. 覆盖自证（每文件 + 行数）

行数按 `wc -l`（末行无换行的文件按内容行数注记）。**全部逐行读完**，除注明「结构级」者。

**根配置（5/5）**：package.json 38；vite.config.ts 13；tsconfig.json 22；vitest.config.ts 19；index.html 14。

**入口/app（6/6）**：src/main.tsx 200；src/App.tsx 163；src/app/Shell.tsx 334；src/app/channels.ts 344；src/app/GlobalErrorBanner.tsx 16；（CSS：Shell.module.css / GlobalErrorBanner.module.css 未逐行，样式文件）。

**stores（17/17）**：navigationStore.ts 122；authStore.ts 24；accountStore.ts 35；uiStore.ts 15；profileStore.ts 79；contactStore.ts 130；commandStore.ts 202；contentStore.ts 296；referralCardStore.ts 115；campaignStore.ts 121；operationsStore.ts 179；strategyStore.ts 506；inboxStore.ts 90；sendAnalyticsStore.ts 60；userOpsDomainHelpers.ts 177；userOpsStore.ts 1369；—— 合计 3520 行。

**contracts（38/38 个 .contract.ts）**：agentRun 20；behaviorSignalMetric 13；decisionReview 36；evaluationScenario 16；experimentEnvelope 17；experimentSummary 7；guideApplyReceipt 12；guidePreview 26；importJobProgress 12；ingestSource 25；knowledgeUsageLog 19；llmCallLog 23；memoryCandidate 17；operatingMemory 17；operationDomain 26；operationHealth 12；operationKnowledgeChunk 49；operationKnowledgeChunkDetail 8；operationKnowledgeDocument 24；operationStateAction 18；operationStatePolicy 18；outboxEntry 32；outboxPayload 9；outcomeMetric 16；playbook 22；promptTemplate 16；proposalDetail 32；proposalSummary 17；relationshipSuggestion 18；revisionApplied 14；runtimeFlag 10；shadowReplay 18；suspectedDeal 16；taxonomyCandidate 19；taxonomyEntry 16；thresholdOverride 11；thresholdOverrideAudit 15；toolCall 11 —— 合计 728 行（fixture JSON 42 个按需抽查，非逐行对象）。

**lib（8/8）**：api.ts 146；inboxApi.ts 132；useSseReconnect.ts 84；reviewLabels.ts 370；applyAiRepairPatch.ts 53；clipboard.ts 71；format.ts 44；uuid.ts 78 —— 合计 978 行。

**types（1/1）**：types/index.ts 842。

**components**：LlmErrorBanner.tsx 110；prompt/usePromptSaveConfirm.tsx 122；ui/tokens.css 96；ui/reset.css 3；Avatar.tsx 6 + index 1；ChunkRef.tsx 132 + index 2；ConfirmDialog.tsx 97 + index 2；EmptyState.tsx 18 + index 1；FormDialog.tsx 143 + index 2；FriendPickerModal.tsx 153 + index 2；MetricCard.tsx 16 + index 1；Overlay.tsx 103 + index 1；PlanStep.tsx 20 + index 1；StatusBadge.tsx 11 + index 1（无行尾换行，wc 计 0）；StatusLine.tsx 16 + index 1；Toast.tsx 77 + index 1 —— ui/ 全组件逐行；review/ 12 文件仅目录结构级（归 features 篇）。

**跨文件核证补读**：src/features/knowledge/chunkInvalidation.ts 43；src/EvolutionCenterTab.tsx 22；src/vite-env.d.ts 5；__tests__/contracts/operationKnowledgeChunk.contract.test.ts 25 + operationsDomain.contract.test.ts 62（全文）+ 其余 5 个契约测试 import 面；__tests__/lib/reviewLabels.test.ts 关键段；后端 src/routes/contract_snapshot.rs bless 段与 4 处 assert_contract_fixture 调用点（rg 定位）。

**总量**：required 清单内文件 8,319 行（wc 合计）全部逐行读完；另补读约 160 行核证材料。

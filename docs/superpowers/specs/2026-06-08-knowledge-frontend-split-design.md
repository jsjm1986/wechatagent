# knowledge/index.tsx 拆分重构设计

- 日期：2026-06-08
- 范围：`frontend/src/features/knowledge/index.tsx`（6582 行，46 个组件挤在单文件）
- 目标：按业务域把巨型单文件拆成主壳 + 4 个域目录 + 1 个共享层，降低单文件体量、提升可维护性，为后续高频的 kb-frontend 加功能铺路。
- 性质：**纯结构重构，行为零变更**——只搬代码 + 补 import/export，不改任何逻辑 / JSX / state。

## 为什么是这个文件

全项目大文件审查（体量 × churn × 可分性 × 测试安全网四维交叉）结论：整个项目只有 2 个文件值得现在重构，`knowledge/index.tsx` 是第一优先。判断依据：

- **体量最大**：6582 行，46 个组件，其中 13 个是 200~450 行的大 View。
- **churn 正在上升**：近期 8+ 个 `feat(kb-frontend)` commit（治理驾驶舱 CockpitView、auto-verify 批量屏、ReviewChat 双栏、导入向导…），说明这块在高频长新功能，可维护性痛点正在累积——不是已稳定的死代码。
- **可分性高（8/10）**：旁边的 `cockpit/` 子目录已经证明"抽组件到独立文件 + 各自 CSS Module"这条路走得通（已抽出 8 个组件）。
- **风险低**：React 组件按 View 抽文件是纯搬运；前端有 vitest 套件兜底，且 `tsc && vite build` + `vitest run` 本地可完整验证（无 Docker、无外部依赖），不像后端改控制流。

对比明确不动的：`knowledge_agent.rs`（4/10，巨型 LLM 主循环不可拆）、`gateway.rs`（已记录暂缓，无单测）、`models.rs`（CLAUDE.md 约定 struct 单文件）、`prompts.rs`（长字符串常量）、`legacy.tsx`（命名误导，实为稳定主体）。

## 关键事实（决定了拆分是低风险的）

这三点是在勘察阶段确证的，是整个方案安全的前提：

1. **组件间零 prop 耦合**。各 View 之间不靠 prop drilling 或 React Context 通信，而是靠 `window` CustomEvent（`wikiFocusChunk` / `wikiOpenCockpit`）+ 各自 fetch。所以抽文件**不需要**提升 state、不需要建 Context——纯剪切粘贴 + 补 import/export 即可。
2. **测试天然兜底**。集成测试 `frontend/src/__tests__/features/knowledge/knowledge.test.tsx` 只 `import KnowledgeFeature from "../../../features/knowledge"`（默认导出），验证 4-mode 渲染 + active 态切换。`cockpit/` 子组件各有独立单测。只要保持 `index.tsx` 的默认导出和各 mode 行为不变，回归网就在。
3. **对外契约只有一个**。`frontend/src/app/channels.ts:30` 用 `lazy(() => import("../features/knowledge"))` 拿默认导出 `KnowledgeFeature`（index.tsx:6580，是 `KnowledgeWikiView` 的薄包装）。这是唯一对外接口——重构只要保住这个默认导出，外部零感知。

## 目标结构

```
features/knowledge/
├── index.tsx              主壳：KnowledgeWikiView + 4 Mode 分发器 + 默认导出 KnowledgeFeature（目标 ~450 行）
├── shared.tsx             跨组件共享层（详见下）
├── today/                 TodayMode 域：DigestCanvas、ChatWorkbench、KnowledgeInbox、TaskRail
├── explore/               ExploreMode 域：AskView、KnowledgeTreeView、ChunkDetail
├── steward/               StewardMode 域：LintView、ReviewView、DocumentsView、ImportWizard、
│                          IngestSourcesView、ObservabilityDashboard、TryRecallView、
│                          ChunkRevisionsDrawer + Chunk* 系列（Inspector/Source/Actions/Referrers/
│                          Revisions/Lock 等）
├── atlas/                 AtlasMode 域：DomainSchemaTab、MetricsTab、MemoryDrawer、ChunkGraphView、
│                          AdminGovernanceView + 各 Governance 子组件（Taxonomies/StatePolicies/Domain）
├── trustTypes.ts          （已存在，不动）
├── Knowledge.module.css   （已存在，不动；CSS 是 :global 全局壳，组件搬文件不影响 className）
└── cockpit/               （已存在，不动）
```

域目录的内部组织：每个域目录用 `index.tsx`（或 `index.ts` barrel）re-export 该域对外要用的组件给主壳；域内组件可一个文件也可按需再分文件，但不强求"每组件一文件"。

### 按 mode 分组而非"每组件一文件"的理由

- mode 是真实业务边界（用户就是按 今日/探索/治理/全景 切换的），同 mode 的组件常一起改，放一起减少跳文件。
- 6582 → 主壳 ~450 + 4 个域目录（每个域 500~2000 行），比 46 个碎文件更好导航。
- 与 `cockpit/` 已验证的"子目录分组"模式一致，团队已熟悉。

## 共享层 shared.tsx 内容

只放**确实被多个域跨用**的东西（勘察统计的引用次数为证）：

| 项 | 类型 | 跨用情况 |
| --- | --- | --- |
| `LlmErrorBanner` + `llmKindLabel` + `LLM_KIND_LABELS` | 组件 + helper | 多个调 LLM 的面板复用（2+ 处） |
| `focusChunk` | 事件桥 fn | 16 处调用，跨 explore / steward |
| `numberOr` / `stringOr` | helper | 10 / 4 处 |
| `classifyChunk` | helper | review/inspector 共用 |
| 共享类型 `TreeChunkItem` / `ReviewChunkItem` / `ReviewCategory` | type/interface | `TreeChunkItem extends ReviewChunkItem extends TrustChunkFields`（TrustChunkFields 已在 trustTypes.ts）；跨 explore/steward/atlas 引用 |

类型搬运按继承链顺序：`ReviewChunkItem`（extends `TrustChunkFields` from trustTypes）→ `TreeChunkItem`（extends `ReviewChunkItem`）。

域内私有的 type / helper / 子组件**不进** shared，跟着各自的域走，避免 shared 变成第二个垃圾桶。

## 分批策略（每批独立验证，可随时停）

每批 = 抽一个域 → 本地 `cd frontend && npx tsc --noEmit` + `npm run build` + `npm run test` 必须全绿才进下一批。任一批失败即就地修复，不堆叠到下一批。

| 批次 | 内容 | 选它的理由 |
| --- | --- | --- |
| 1 | **shared.tsx** | 先抽共享层，后续批次都依赖它，避免每批回头改 |
| 2 | **atlas/** | 最独立（Governance 子组件自成一体），风险最低，先练手验证流程 |
| 3 | **explore/** | AskView + KnowledgeTreeView + ChunkDetail，中等体量 |
| 4 | **today/** | DigestCanvas + ChatWorkbench + KnowledgeInbox + TaskRail |
| 5 | **steward/** | 最大块（Chunk* 系列 + 8 个 View），放最后，前面批次已磨合好流程 |
| 6 | **收尾** | index.tsx 只剩主壳；最终 `tsc && vite build` + `vitest run` 全绿 + 人工跑 dev server 点一遍 4 mode |

## 验证策略

- **每批**：`npx tsc --noEmit`（类型）+ `npm run build`（`tsc && vite build`，构建）+ `npm run test`（vitest run，含 knowledge 集成测试 + cockpit 单测）三者全绿。
- **收尾人工验证**：`npm run dev` 起 dev server，手动点过 今日/探索/治理/全景 4 个 mode 及各 pane，确认 CustomEvent 桥（点 chunk → Inspector 聚焦 / 点治理总览跳转）仍工作。
- 本地资源说明：前端验证全程纯 JS/TS，无 Docker、无重编译，本地可完整跑，不受后端"重套件走 CI"约束。

## 非目标（YAGNI）

- 不改任何组件逻辑、JSX 结构、state 管理、API 调用、CSS。
- 不把 CustomEvent 通信改成 Context / 状态库（那是另一个独立决策，不在本次范围）。
- 不强求"每组件一文件"的极致拆分。
- 不动 `cockpit/`、`trustTypes.ts`、`Knowledge.module.css`。
- 不碰 `legacy.tsx` 或其他前端文件。

## 风险与回退

- **风险**：组件搬文件时漏带某个域内私有 helper/type，导致 `tsc` 报 unresolved。**缓解**：每批 `tsc --noEmit` 立即暴露，就地补 import。
- **风险**：误把域私有 helper 当共享提到 shared，造成反向依赖。**缓解**：shared 只收勘察确证的跨用项（上表），其余跟域走。
- **回退**：每批独立 commit；任一批验证不过且无法就地修复，`git revert` 该批即可，不影响已合入的前序批次。

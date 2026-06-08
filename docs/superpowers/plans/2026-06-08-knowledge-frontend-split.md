# knowledge/index.tsx 拆分重构 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `frontend/src/features/knowledge/index.tsx`（6582 行、46 组件）按 4-mode 业务域拆成「主壳 + shared + today/explore/steward/atlas 四个域目录」，单文件体量大幅下降，行为零变更。

**Architecture:** 纯结构重构——把组件按业务域剪切到独立文件，补 import/export，不改任何逻辑/JSX/state/CSS。组件间已靠 `window` CustomEvent 通信（零 prop 耦合），所以无需提升 state 或建 Context。唯一对外契约是默认导出 `KnowledgeFeature`（被 `app/channels.ts:30` 的 `lazy(() => import("../features/knowledge"))` 消费），保住它即外部零感知。

**Tech Stack:** React 19 + TypeScript + Vite + vitest。验证全程纯前端，无 Docker、无重编译，本地可完整跑。

**关键术语 — "ChunkInspector 簇"：** 指 index.tsx 第 2894–3850 行这一整块内聚单元，含 `classifyChunk`、`ChunkInspectorPane`、`ChunkSourceSection`、`focusChunk`、类型 `LockHolder`/`ChunkLockState`/`ChunkActionState`/`ReferrerEntry`/`RevisionEntry`、`formatLockExpiry`、`ChunkLockBadge`、`useChunkInspectorLock`、`ChunkActionsBar`、`ChunkReferrersList`、`ChunkRevisionsTimeline`。因为 `ChunkInspectorPane` 被 explore（行 250）和 steward（行 391）**双域**引用，整簇归入 shared。

---

## 文件结构（重构后）

```
features/knowledge/
├── index.tsx        主壳：LLM 之外的顶层——KnowledgeMode 类型、ModeMeta、KNOWLEDGE_MODES、
│                    KnowledgeWikiView、TodayMode、ExploreMode、StewardMode、AtlasMode、
│                    默认导出 KnowledgeFeature（目标 ~450 行）
├── shared.tsx       LlmErrorBanner 簇 + numberOr/stringOr + 共享类型(ReviewChunkItem/TreeChunkItem/
│                    ReviewCategory) + ChunkInspector 簇（含 focusChunk 事件桥）
├── atlas.tsx        ChunkGraphView、DomainSchemaTab、MetricsTab、MemoryDrawer、AdminGovernanceView
│                    + TaxonomiesGovernance/StatePoliciesGovernance/DomainGovernance + MetadataDashboard
│                    + PublishBar + 各自私有 type
├── explore.tsx      AskView、KnowledgeTreeView、ChunkDetail、WIKI_TYPES_ORDER、stripTool + 各自私有 type
├── today.tsx        DigestCanvas、ChatWorkbench、KnowledgeInbox、TaskRail、severityBadgeClass + 各自私有 type
├── steward.tsx      LintView、ReviewView、DocumentsView、ImportWizard、TryRecallView、IngestSourcesView、
│                    ObservabilityDashboard(+PhaseRollupPanel/WorkerHealthPanel/TestMatchPanel)、
│                    ChunkRevisionsDrawer + REVIEW_CATEGORIES + 各自私有 type
├── trustTypes.ts    （不动）
├── Knowledge.module.css （不动）
└── cockpit/         （不动；已确认零反向依赖 index.tsx）
```

**注意：原 index.tsx 第 6306 行 `tryLlmError` 是死代码（`void tryLlmError;` 自我消费，无真实调用）——重构时直接删除，不带进任何文件。**

---

## 组件 → 域 精确映射（搬运依据）

> 行号基于重构前的 index.tsx。搬运以「函数/类型定义整体」为单位，连同其上方的注释块一起搬。

**shared.tsx：**
| 项 | 原行 |
| --- | --- |
| `LLM_KIND_LABELS` / `llmKindLabel` / `LlmErrorBanner` | 52–118 |
| `ReviewCategory` / `ReviewChunkItem` | 2509–2533 |
| ChunkInspector 簇（classifyChunk → ChunkRevisionsTimeline，含 focusChunk） | 2894–3850 |
| `TreeChunkItem` | 3864–3870 |
| `numberOr` / `stringOr` | 6032–6037 |

**atlas.tsx：** ChunkGraphView(471–898) · DomainSchemaField/DomainSchemaItem/DomainSchemaTab(1707–1839) · MetricsTab(2211–2282)+AnswerCacheMetrics(2283–2294) · AdminGovernanceView 簇(5175–5734：含 MetadataResp/MetadataDashboard/PublishBarProps/PublishBar/TaxonomyEntryView/AdminGovernanceView/TaxonomiesGovernance/StatePolicyEntryView/StatePoliciesGovernance/DomainEntryView/DomainGovernance) · MemoryDrawer 簇(6481–end：OperatorMemoryView/OPERATOR_MEMORY_KINDS/MemoryDrawer)

**explore.tsx：** GapSignalItem/GAP_SIGNAL_KINDS(1840–1866) · AskSourceQuote/AskToolTraceStep/AskResult/AskView(1867–2199) · stripTool(2200–2210) · WIKI_TYPES_ORDER(3851–3863) · KnowledgeTreeView(5735–5927) · ChunkDetail(5928–6031)

**today.tsx：** ChatTurnView/ChatTurnResponse/ChatWorkbench(3872–4202) · InboxItemView/InboxResp/KnowledgeInbox(4203–4330) · DigestCardView/DigestReportView/severityBadgeClass/DigestCanvas(6147–6305) · ChatTaskView/TaskRail(6311–6480)

**steward.tsx：** DocumentItem/DocumentChunkRow/DocumentsView(899–1141) · ImportPreviewChunk/ImportPreviewResult/ImportWizard(1142–1499) · TryRecall 类型×3/TryRecallView(1500–1706) · REVIEW_CATEGORIES/ReviewView(2535–2893) · LintView(2295–2508) · IngestSource 类型/IngestSourcesView(4331–4569) · ObservabilityDashboard(4570–4817)+PhaseRollupPanel(4818–4932)+WorkerHealthPanel(4933–5113)+TestMatchPanel(5114–5177) · ChunkRevisionItem/ChunkRevisionsDrawer(6039–6146)

---

## 执行约定（每个 Task 通用）

- **导出方式**：被主壳或其他文件引用的组件/类型用 `export function` / `export interface`；域内私有的不导出。
- **import 方向**：`index.tsx` 从各域文件 import 它在 JSX 里直接渲染的组件；各域文件从 `./shared` import 共享项；各文件保留自己实际用到的 lucide 图标和 `../../lib/api` 等原始 import（不要照抄全量 import，只带用到的）。
- **每个 Task 末尾验证三连**（在 `frontend/` 目录下）：
  - `npx tsc --noEmit` → 期望 0 error
  - `npm run build` → 期望 build 成功
  - `npm run test` → 期望全绿（含 knowledge 集成测试 + cockpit 单测）
- **每个 Task 独立 commit**，commit message 前缀 `refactor(kb-frontend):`。

---

## Task 1: 建 shared.tsx（共享层先行）

**Files:**
- Create: `frontend/src/features/knowledge/shared.tsx`
- Modify: `frontend/src/features/knowledge/index.tsx`（删除已搬出的项，补 `import ... from "./shared"`）

- [ ] **Step 1: 建 shared.tsx，从 index.tsx 搬入共享项**

新建 `frontend/src/features/knowledge/shared.tsx`，把下列项**整体剪切**过来（连同上方注释），全部加 `export`：
- `LLM_KIND_LABELS`、`llmKindLabel`、`LlmErrorBanner`（原 52–118）
- 类型 `ReviewCategory`、`ReviewChunkItem`（原 2509–2533）、`TreeChunkItem`（原 3864–3870）
- ChunkInspector 簇（原 2894–3850 整块）：`classifyChunk`、`ChunkInspectorPane`、`ChunkSourceSection`、`focusChunk`、`LockHolder`、`ChunkLockState`、`formatLockExpiry`、`ChunkLockBadge`、`useChunkInspectorLock`、`ChunkActionState`、`ChunkActionsBar`、`ReferrerEntry`、`ChunkReferrersList`、`RevisionEntry`、`ChunkRevisionsTimeline`
- `numberOr`、`stringOr`（原 6032–6037）

shared.tsx 顶部 import：
```tsx
import { useState, useEffect, useRef, useCallback, Fragment } from "react";
import { AlertTriangle, CheckCircle2, ChevronDown, ChevronRight, Clock3, Eye, History, Link2, Loader2, RefreshCw, ShieldCheck, SquarePen, Trash2, Undo2, X } from "lucide-react";
import { parseApiError, LlmUnavailableError } from "../../lib/api";
import { parseCompleteness, parseIntegrityReport, type TrustChunkFields } from "./trustTypes";
```
> 说明：上面的 lucide 图标清单需对照搬入代码实际使用增删；`tsc` 会报未使用/未定义，按报错调整。`export` 标记加在：`LlmErrorBanner`、`ChunkInspectorPane`、`ChunkRevisionsTimeline`、`focusChunk`、`classifyChunk`、`numberOr`、`stringOr` 以及类型 `ReviewChunkItem`/`TreeChunkItem`/`ReviewCategory`（其余簇内项若仅被簇内引用可不导出）。

- [ ] **Step 2: 在 index.tsx 补 import 并删除已搬出代码**

删除 index.tsx 中已搬走的全部行段，在文件顶部 import 区下方加。**重要：只 import 当前 index.tsx 里仍有代码实际引用的项，否则 tsc 报未用 import。** Task 1 阶段 index.tsx 除主壳外还残留尚未搬出的域组件（DocumentsView/AskView/MetricsTab… 都还在），它们用到 classifyChunk/numberOr/stringOr/各共享类型，所以这些要 import；`ChunkInspectorPane` 被主壳 ExploreMode/StewardMode 的 JSX 直接渲染，也要 import：
```tsx
import {
  LlmErrorBanner,
  ChunkInspectorPane,
  focusChunk,
  classifyChunk,
  numberOr,
  stringOr,
  type ReviewChunkItem,
  type TreeChunkItem,
  type ReviewCategory,
} from "./shared";
```
> 不要 import `ChunkRevisionsTimeline`/`ChunkLockBadge`/`ChunkSourceSection` 等簇内子组件——它们只被簇内的 `ChunkInspectorPane` 引用（已随簇进 shared.tsx），index.tsx 不直接渲染它们。后续 Task 3/4/5 把域组件搬走后，若 index.tsx 不再用某项（如 classifyChunk 随 steward 走后），在该 Task 的 Step 2 顺手从这行 import 删掉——以每步 tsc 报的"未用 import"为准。

- [ ] **Step 3: 验证三连**

Run（在 `frontend/`）：
```bash
npx tsc --noEmit && npm run build && npm run test
```
Expected: tsc 0 error；build 成功；test 全绿。
> 若报 `ChunkInspectorPane` 等的私有依赖（如 `ChunkSourceSection`）未定义，说明簇内有项漏搬——回 Step 1 补全整块 2894–3850。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/features/knowledge/shared.tsx frontend/src/features/knowledge/index.tsx
git commit -m "refactor(kb-frontend): 抽出 shared.tsx(LlmErrorBanner+ChunkInspector簇+共享helper/type)"
```

---

## Task 2: 建 atlas.tsx（最独立，先练手）

**Files:**
- Create: `frontend/src/features/knowledge/atlas.tsx`
- Modify: `frontend/src/features/knowledge/index.tsx`

- [ ] **Step 1: 建 atlas.tsx，搬入 AtlasMode 的 5 个 pane 组件**

新建 `frontend/src/features/knowledge/atlas.tsx`，整体剪切：
- `ChunkGraphView`（原 471–898，连同上方 P1 注释块 453–470）
- `DomainSchemaField`/`DomainSchemaItem`/`DomainSchemaTab`（原 1707–1839）
- `MetricsTab`（原 2211–2282）+ `AnswerCacheMetrics`（原 2283–2294）
- AdminGovernanceView 簇（原 5175–5734）：`MetadataResp`、`MetadataDashboard`、`PublishBarProps`、`PublishBar`、`TaxonomyEntryView`、`AdminGovernanceView`、`TaxonomiesGovernance`、`StatePolicyEntryView`、`StatePoliciesGovernance`、`DomainEntryView`、`DomainGovernance`
- MemoryDrawer 簇（原 6481–文件末）：`OperatorMemoryView`、`OPERATOR_MEMORY_KINDS`、`MemoryDrawer`

加 `export` 的对外项：`ChunkGraphView`、`DomainSchemaTab`、`MetricsTab`、`MemoryDrawer`、`AdminGovernanceView`（这 5 个是 AtlasMode JSX 直接渲染的）。

atlas.tsx 顶部 import（按实际使用增删）：
```tsx
import { useState, useEffect, useRef, useCallback, Fragment } from "react";
import { Activity, BookOpen, BrainCircuit, Network, ShieldCheck /* ...按实际用到的图标补全 */ } from "lucide-react";
import { parseApiError } from "../../lib/api";
import { numberOr, stringOr, focusChunk, type TreeChunkItem } from "./shared";
```
> ChunkGraphView 用到 `TreeChunkItem`、`focusChunk`；Governance 用到 `numberOr`/`stringOr`。具体按 tsc 报错对齐。

- [ ] **Step 2: 在 index.tsx 补 import 并删除已搬出代码**

删除 index.tsx 中上述行段，加：
```tsx
import { ChunkGraphView, DomainSchemaTab, MetricsTab, MemoryDrawer, AdminGovernanceView } from "./atlas";
```

- [ ] **Step 3: 验证三连**

Run（在 `frontend/`）：
```bash
npx tsc --noEmit && npm run build && npm run test
```
Expected: 全绿。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/features/knowledge/atlas.tsx frontend/src/features/knowledge/index.tsx
git commit -m "refactor(kb-frontend): 抽出 atlas.tsx(ChunkGraph/DomainSchema/Metrics/Memory/Governance)"
```

---

## Task 3: 建 explore.tsx

**Files:**
- Create: `frontend/src/features/knowledge/explore.tsx`
- Modify: `frontend/src/features/knowledge/index.tsx`

- [ ] **Step 1: 建 explore.tsx，搬入 ExploreMode 组件**

新建 `frontend/src/features/knowledge/explore.tsx`，整体剪切：
- `GapSignalItem`/`GAP_SIGNAL_KINDS`（原 1840–1866）
- `AskSourceQuote`/`AskToolTraceStep`/`AskResult`/`AskView`（原 1867–2199）
- `stripTool`（原 2200–2210）
- `WIKI_TYPES_ORDER`（原 3851–3863）
- `KnowledgeTreeView`（原 5735–5927）
- `ChunkDetail`（原 5928–6031）

加 `export`：`AskView`、`KnowledgeTreeView`（ExploreMode JSX 直接渲染）。`ChunkDetail` 被 `KnowledgeTreeView` 内部用——若同文件则无需 export。

explore.tsx 顶部 import：
```tsx
import { useState, useEffect, useMemo, useRef, Fragment } from "react";
import { ArrowRight, ChevronDown, ChevronRight, Compass, Link2, Loader2, Search, SendHorizonal /* 按实际增删 */ } from "lucide-react";
import { parseApiError, LlmUnavailableError } from "../../lib/api";
import { LlmErrorBanner, focusChunk, numberOr, stringOr, type TreeChunkItem, type ReviewChunkItem } from "./shared";
```

- [ ] **Step 2: 在 index.tsx 补 import 并删除已搬出代码**

```tsx
import { AskView, KnowledgeTreeView } from "./explore";
```

- [ ] **Step 3: 验证三连**

```bash
npx tsc --noEmit && npm run build && npm run test
```
Expected: 全绿。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/features/knowledge/explore.tsx frontend/src/features/knowledge/index.tsx
git commit -m "refactor(kb-frontend): 抽出 explore.tsx(AskView/KnowledgeTreeView/ChunkDetail)"
```

---

## Task 4: 建 today.tsx

**Files:**
- Create: `frontend/src/features/knowledge/today.tsx`
- Modify: `frontend/src/features/knowledge/index.tsx`

- [ ] **Step 1: 建 today.tsx，搬入 TodayMode 组件**

新建 `frontend/src/features/knowledge/today.tsx`，整体剪切：
- `ChatTurnView`/`ChatTurnResponse`/`ChatWorkbench`（原 3872–4202）
- `InboxItemView`/`InboxResp`/`KnowledgeInbox`（原 4203–4330）
- `DigestCardView`/`DigestReportView`/`severityBadgeClass`/`DigestCanvas`（原 6147–6305）
- `ChatTaskView`/`TaskRail`（原 6311–6480）
- **不要**搬 `tryLlmError`（原 6306–6310，死代码，删除）

加 `export`：`DigestCanvas`、`ChatWorkbench`、`KnowledgeInbox`、`TaskRail`（TodayMode JSX 直接渲染）。

today.tsx 顶部 import：
```tsx
import { useState, useEffect, useRef, Fragment } from "react";
import { Calendar, Inbox, Loader2, MessageSquareText, SendHorizonal, Sparkles /* 按实际增删 */ } from "lucide-react";
import { parseApiError, LlmUnavailableError } from "../../lib/api";
import { LlmErrorBanner, numberOr, stringOr } from "./shared";
```

- [ ] **Step 2: 在 index.tsx 补 import 并删除已搬出代码**

```tsx
import { DigestCanvas, ChatWorkbench, KnowledgeInbox, TaskRail } from "./today";
```

- [ ] **Step 3: 验证三连**

```bash
npx tsc --noEmit && npm run build && npm run test
```
Expected: 全绿。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/features/knowledge/today.tsx frontend/src/features/knowledge/index.tsx
git commit -m "refactor(kb-frontend): 抽出 today.tsx(Digest/Chat/Inbox/TaskRail) + 删死代码 tryLlmError"
```

---

## Task 5: 建 steward.tsx（最大块，放最后）

**Files:**
- Create: `frontend/src/features/knowledge/steward.tsx`
- Modify: `frontend/src/features/knowledge/index.tsx`

- [ ] **Step 1: 建 steward.tsx，搬入 StewardMode 剩余组件**

新建 `frontend/src/features/knowledge/steward.tsx`，整体剪切：
- `DocumentItem`/`DocumentChunkRow`/`DocumentsView`（原 899–1141）
- `ImportPreviewChunk`/`ImportPreviewResult`/`ImportWizard`（原 1142–1499）
- `TryRecallTraceStep`/`TryRecallRouteResult`/`TryRecallSliceItem`/`TryRecallView`（原 1500–1706）
- `LintView`（原 2295–2508）
- `REVIEW_CATEGORIES`/`ReviewView`（原 2535–2893）
- `CatalogPersistedView`/`LogsAnalyzeView`/`IngestSourceItem`/`IngestSourcesView`（原 4331–4569）
- `ObservabilityDashboard`（原 4570–4817）+ `PhaseRollupPanel`（4818–4932）+ `WorkerHealthPanel`（4933–5113）+ `TestMatchPanel`（5114–5177）
- `ChunkRevisionItem`/`ChunkRevisionsDrawer`（原 6039–6146）

加 `export`：`LintView`、`ReviewView`、`DocumentsView`、`ImportWizard`、`IngestSourcesView`、`ObservabilityDashboard`、`TryRecallView`、`ChunkRevisionsDrawer`（StewardMode JSX 直接渲染的 8 个）。

steward.tsx 顶部 import：
```tsx
import { useState, useEffect, useMemo, useRef, Fragment } from "react";
import { Activity, AlertTriangle, Archive, CheckCircle2, ChevronDown, ChevronRight, Clock3, FileText, GitMerge, Loader2, RefreshCw, Rss, Scissors, Search, ShieldCheck, UploadCloud, Workflow /* 按实际增删 */ } from "lucide-react";
import { parseApiError, LlmUnavailableError } from "../../lib/api";
import { parseCompleteness, parseIntegrityReport, type IntegrityReportView } from "./trustTypes";
import { LlmErrorBanner, classifyChunk, focusChunk, numberOr, stringOr, type ReviewChunkItem, type ReviewCategory } from "./shared";
```

- [ ] **Step 2: 在 index.tsx 补 import 并删除已搬出代码**

```tsx
import {
  LintView, ReviewView, DocumentsView, ImportWizard,
  IngestSourcesView, ObservabilityDashboard, TryRecallView, ChunkRevisionsDrawer,
} from "./steward";
```
此时 index.tsx 应只剩：顶部 import + `KnowledgeMode`/`ModeMeta`/`KNOWLEDGE_MODES` + `KnowledgeWikiView` + `TodayMode`/`ExploreMode`/`StewardMode`/`AtlasMode` + 默认导出 `KnowledgeFeature`。精简 index.tsx 顶部不再使用的 lucide 图标 import（保留 mode-bar 用的 Calendar/Compass/Wrench/MapIcon/FileBox 等）。

- [ ] **Step 3: 验证三连**

```bash
npx tsc --noEmit && npm run build && npm run test
```
Expected: 全绿。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/features/knowledge/steward.tsx frontend/src/features/knowledge/index.tsx
git commit -m "refactor(kb-frontend): 抽出 steward.tsx(Lint/Review/Documents/Import/Ingest/Observability/TryRecall/Revisions)"
```

---

## Task 6: 收尾验证

**Files:**
- Verify: `frontend/src/features/knowledge/index.tsx`（确认行数降到 ~450）

- [ ] **Step 1: 确认 index.tsx 体量与残留**

Run:
```bash
wc -l frontend/src/features/knowledge/index.tsx
grep -nE "^(export )?(async )?(function|const|type|interface) [A-Za-z]" frontend/src/features/knowledge/index.tsx
```
Expected: ~450 行；顶层定义只剩 `KnowledgeMode`/`ModeMeta`/`KNOWLEDGE_MODES`/`KnowledgeWikiView`/`TodayMode`/`ExploreMode`/`StewardMode`/`AtlasMode`/`KnowledgeFeature`。

- [ ] **Step 2: 确认无 tryLlmError 残留、无未用 import**

Run:
```bash
grep -rn "tryLlmError" frontend/src/features/knowledge/
npx tsc --noEmit
```
Expected: `tryLlmError` 0 命中；tsc 0 error（`-Dwarnings` 等价的未用 import 在 tsc/vite 下也应为 0）。

- [ ] **Step 3: 全量验证三连**

Run（在 `frontend/`）：
```bash
npx tsc --noEmit && npm run build && npm run test
```
Expected: 全绿。

- [ ] **Step 4: 人工跑 dev server 点验 4 mode**

Run:
```bash
npm run dev
```
手动验证（浏览器开 dev server 地址）：
1. 顶部 mode-bar 4 个按钮（今日/探索/治理/全景）渲染正常，点击切换 active 态正确。
2. **今日**：Digest / AI 协作 / 待办收件箱 三 pane 切换；右侧 TaskRail 渲染。
3. **探索**：左侧 KnowledgeTreeView 树渲染；点某 chunk → 右侧 ChunkInspectorPane 聚焦（验证 `wikiFocusChunk` 事件桥跨文件仍工作）；中间 AskView 可输入。
4. **治理**：治理总览（CockpitView）"打开待评审"按钮 → 切到 review pane（验证 `wikiOpenCockpit` 事件桥）；其余 8 pane 可切。
5. **全景**：Schema/指标/记忆/关系图谱/治理 5 pane 可切；关系图谱 SVG 渲染。

> 这一步是行为等价的最终保证——CustomEvent 桥跨文件通信是本次重构唯一的隐性风险点，必须人工确认。若无法跑浏览器，明确说明「dev server 已起、tsc/build/test 全绿，但未做人工点验」，不假报成功。

- [ ] **Step 5: 收尾 commit（如有 import 精简等微调）**

```bash
git add frontend/src/features/knowledge/index.tsx
git commit -m "refactor(kb-frontend): 收尾——index.tsx 精简至主壳(~450行) + 清未用 import"
```

---

## 验证策略总览

| 层 | 手段 | 何时 |
| --- | --- | --- |
| 类型 | `npx tsc --noEmit` | 每 Task |
| 构建 | `npm run build`（tsc && vite build） | 每 Task |
| 测试 | `npm run test`（vitest：knowledge 集成测试 + cockpit 单测） | 每 Task |
| 行为 | `npm run dev` 人工点验 4 mode + 2 个 CustomEvent 桥 | 收尾 |

## 回退策略

每 Task 独立 commit。任一 Task 验证不过且无法就地修复 → `git revert <该 commit>`，不影响已合入的前序 Task。

## 非目标（YAGNI）

- 不改任何组件逻辑/JSX/state/API/CSS。
- 不把 CustomEvent 改成 Context/状态库。
- 不强求"每组件一文件"（域内可多组件同文件）。
- 不动 cockpit/、trustTypes.ts、Knowledge.module.css、legacy.tsx。

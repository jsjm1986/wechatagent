# Ask-Human 统一收件箱前端（Phase 2）设计

> 状态：设计已逐段获批（2026-06-21 brainstorming）。本 spec 详设 **Phase 2（统一收件箱前端，方案 B 全量）**，可直接转 writing-plans。P3（配置页）是下一独立子项目，不在本轮。

## 背景与目标

ask-human 统一频道分三个子项目顺序交付：**P1 后端地基（已完成）→ P2 收件箱前端（本 spec）→ P3 配置页**。P1 已交付只读聚合端点（`/inbox`+`/summary`）、escalation 三处置端点、配置写入端点。

P2 目标：把系统里 8 类「需要人介入/审核/决策」的待审触点收口成一个 `askHuman` 前端频道，作为所有 ask-human 事项的 **canonical 主场**。频道内既能处置简单项（inline 就地处置），也能挂载从老页中立化出来的真实交互组件（rich 在频道内挂载）。**老页面导航与外观保持不动**，其交互组件抽成中立共享组件，老页改为复用同一组件的薄壳。

**用户拍板：方案 B 一次全量 4 源中立化**（最忠实设计文档「统一频道是 canonical 主场，老页交互组件中立化、改薄壳、不往外跳」的愿景）。这需要跨前后端改动（见「后端补缺」）。

## 消费的 P1 契约（已实证，不可改后端契约形状）

- `GET /api/admin/ask-human/inbox?source=<filter>` → `{ items: InboxItem[], errors: [{source, error}] }`（per-source 降级：单源查询失败记进 `errors[]`，其余源照常返回）
- `GET /api/admin/ask-human/summary` → `{ principalEscalation, knowledgeReview, taxonomyCandidate, relationshipSuggestion, gapSignal, profileRisky, evolutionProposal, lessonsLearned }`（各源 pending 计数；`unwrap_or(0)` 降级，单源计数失败显 0 不阻断）
- `InboxItem`（camelCase serde）: `source, id, title, summary, severity("high"|"medium"|"low"), createdAt(Option), ageHours, actionKind("inline"|"rich"), richComponent?(Option), richParams?(Option<Document>)`
- 8 个 source 字符串：`principal_escalation, knowledge_review, taxonomy_candidate, relationship_suggestion, gap_signal, profile_risky, evolution_proposal, lessons_learned`
- 4 个 richComponent 字符串：`knowledgeReview, profilePublish, evolutionRelease, lessonsPromote`

## 架构与分层

```
新频道 features/ask-human/
  ├── index.tsx          外壳：ConfirmProvider/ToastProvider 包裹 + summary 仪表盘 + source sub-tab（纯 useState，仿 knowledge/index.tsx）
  ├── inboxStore (Zustand) 消费 /inbox + /summary；保成功 items + 暴露 per-source errors
  ├── InboxList          归一卡片列：severity/age/title/summary，按 actionKind 分派
  ├── inline 处置器       4 类：escalation / taxonomy / relationship / gap —— 频道内直接调已有端点
  └── rich 挂载点         按 richComponent 渲染 4 个中立化共享组件（richParams 的 id 驱动）

新增中立组件家 components/review/（老页与频道都 import）
  ├── ReviewQueue.tsx        通用「list + 行内动作 + refetch」原语
  ├── ChunkReviewCard.tsx    ← knowledgeReview（从 steward ReviewView 抽单卡）
  ├── ProfilePublishCard.tsx ← profilePublish（从 system-strategy 抽「发布/激活+状态徽标」，解耦 strategyStore）
  ├── ProposalReleaseCard.tsx← evolutionRelease（复用 evolution ProposalDetailView，已 id 驱动）
  └── LessonPromoteCard.tsx  ← lessonsPromote（从 LessonsLearnedAdmin 抽，含人工填写表单）

后端补缺（全量 B 必需，与前端同一 P2 plan）
  ① P1 ask_human_inbox.rs::collect_lessons_learned 给 richParams 加 lessonId（当前 None，否则 lessons 无法深链）
  ② 补 GET /api/operation-knowledge/chunks/:id 单 chunk 路径（knowledgeReview 深链；当前只有列表端点）
```

### 加频道的机制（实证 frontend/src）

加 `askHuman` 频道只需 3 处编辑，无 switch/map：
1. `types/index.ts`：`Channel` union（`:4-18`）加 `| "askHuman"`。
2. `app/channels.ts`：顶部加 `const AskHumanFeature = lazy(() => import("../features/ask-human"));`，导入一个 lucide 图标（如 `Inbox`），在 `CHANNELS` 数组（`:49`）push 一个 `ChannelDef`。`ChannelDef` 形状（`channels.ts:34-44`）：`{ id: Channel; group: "运营"|"知识"|"系统"; label; caption; icon: LucideIcon; eyebrow; title; subtitle; Component: LazyExoticComponent }`。group 取「运营」。
3. 无第三处接线：`Shell.tsx:137` 用 `CHANNELS.find(c => c.id === activeChannel)` 查表渲染 `<Suspense><Component/></Suspense>`，侧栏导航从 `CHANNELS` 按 group 自动渲染。加数组项即自动接线。

### 共享基础设施（实证，直接复用）

- `lib/api.ts` 导出 `api`（`get<T>(url)`/`post<T>(url,body?)`/`put`/`delete`/`postForm`/`postRaw`）+ `parseApiError`。非 2xx 抛 `parseApiError`。新 store/卡片一律用 `api`，不用裸 fetch（统一错误处理）。
- `useToast()` → `{ success, error, info }(msg: string)`，自动消失。`useConfirm()` → `async (opts) => Promise<boolean>`，`opts: { title, body?, tone?: "default"|"danger", confirmText?, requireText? }`（`requireText` 强制输入匹配串，用于破坏性动作）。Provider 定义在 `components/ui/{Toast,ConfirmDialog,FormDialog}/`（目录 barrel）。频道外壳按 `knowledge/index.tsx:384-394` 模式包裹 `<ConfirmProvider><ToastProvider>`。

## 组件与接口

### 通用原语 `ReviewQueue<T>`（components/review/ReviewQueue.tsx）

统一今天 steward/evolution/system-strategy 各自手搓 6+ 次的 `{items, loading, error, busyId}` 状态机 + `load()`/refetch + 行内动作分派。

```ts
interface RowCtx {
  busy: boolean;
  runAction: (fn: () => Promise<unknown>, successMsg?: string) => Promise<void>;
  // runAction: 置 busyId → await fn → 成功 toast.success + refetch → 失败 toast.error；统一惯例
}
interface ReviewQueueProps<T> {
  fetchItems: () => Promise<T[]>;            // 用 lib/api
  getId: (item: T) => string;
  renderItem: (item: T, ctx: RowCtx) => ReactNode;
  emptyText?: string;
}
```

### 4 个 rich 适配器（各 = 一张卡片；老页和频道复用同一卡片）

| 适配器 | 数据来源 | 列表端点 | 处置端点 | 中立化要点 |
|---|---|---|---|---|
| `ProposalReleaseCard` | evolution `ProposalDetailView`（`EvolutionCenterTab.tsx:422`，已 id 驱动 `{proposalId,onClose,onActionDone}`） | `GET /api/evolution/experiments?limit=20`（客户端 flatten + 按 `status==="eligible_for_release"` 过滤） | `POST /api/evolution/proposals/:id/release {confirmation:"RELEASE"}`；rollback 同 | 低：把 `ProposalDetailView` 物理移到 `components/review/ProposalReleaseCard.tsx`（中立家），evolution 老页改 import 它的薄壳；同时统一其私有 apiGet/apiPost 到 lib/api。**所有 4 张卡片都落 `components/review/`，老页一律 import 中立家，不反向跨 feature import** |
| `ChunkReviewCard` | 从 `steward.tsx:1129 ReviewView` 抽单卡 | **新增 `GET /api/operation-knowledge/chunks/:id`**（深链单项；列表 `GET /chunks` 仍供老页） | `POST /api/operation-knowledge/chunks/:id/{verify,reject}` body `{}` | 中：verify-gate（hasQuote && hasAnchor）随卡片走；`focusChunk` 的 window-event 在频道里 dead 但优雅降级 |
| `LessonPromoteCard` | 从 `system-strategy/index.tsx:2055 LessonsLearnedAdmin` 抽 | `GET /api/admin/lessons-learned?patternKind=`（按 `lessonId` 取单项） | `POST /api/admin/lessons-learned/:lessonId/promote-to-peer-case {title, body, summary?}` | 中：晋升需人工填 title/body → 频道内**展开内联表单**（非一键）；**依赖后端补缺①给 richParams.lessonId** |
| `ProfilePublishCard` | 从 `system-strategy ProfileEditor:1026` 抽精简版 | `GET /api/admin/domain-profiles`（客户端按 is_active/current_version 过滤） | `POST /api/admin/domain-profiles/:id/{publish,activate,rollout}` body `{}`；publish 返 `{ok,pendingActivation?,riskyFields?}` | 高：**解耦 strategyStore**——只搬「发布/激活 + 状态徽标」动作，直调 store 的 `publishDomainProfile(id)`/`activateDomainProfile(id)`，**不搬**生成/编辑全表单（留老页） |

### 4 个 inline 处置器（频道内就地，不跳页）

| 源 | 动作 | 端点 |
|---|---|---|
| `principal_escalation` | resolve（`{verdict, substance, constraints, authorizationWindowHours}`）+ reassign（`{toWxid}`） | P1 `POST /api/admin/principal-escalations/:short_code/{resolve,reassign}`；list `GET ?status=pending\|resolved` |
| `taxonomy_candidate` | approve / reject | `admin_taxonomy_candidates`（approve/reject 路由） |
| `relationship_suggestion` | approve / reject | `admin_relationship_suggestions`（`mod.rs:740-748` 已注册） |
| `gap_signal` | dismiss | `dismiss_knowledge_gap_signal`（`mod.rs:186/510`） |

### 老页薄壳化

steward 的 `ReviewView` 改为渲染 `<ReviewQueue fetchItems={listChunks} renderItem={(c,ctx)=><ChunkReviewCard .../>}/>`；system-strategy 的 profile/lessons、evolution 同理。**导航和页面入口完全不动**——老页和频道渲染同一组件形态，仅数据入口不同。退役老页是将来独立决策，本轮不做。

## 数据流、错误处理与降级

`inboxStore`（Zustand）是唯一数据入口：

```ts
interface InboxState {
  items: InboxItem[];                      // /inbox 成功项
  errors: { source: string; error: string }[];  // per-source 失败（P1 已返回，不吞）
  summary: Record<string, number> | null;  // /summary 各源计数
  loading: boolean;
  fatalError: string | null;               // 请求级失败（网络/401）
  load: (source?: string) => Promise<void>; // 手动触发，无轮询
}
```

**降级语义（P2 硬要求，最易被实现者写错——绝不照抄 operationsStore 的「catch→全置空」）：**
- `load()` 成功 → `items` 填好源、`errors` 显形坏源（频道顶部一条「N 个来源暂时不可用」可展开提示），其余源照常处置。**坏源绝不连累好源。**
- `load()` 整个请求失败（网络/401）→ 保留上次 `items`，置 `fatalError` 显红 banner，**不清空**。
- `summary` 单源计数失败显 0（P1 已是 `unwrap_or(0)` 语义），不阻断仪表盘。

**行内动作**：处置 → `busyId` 锁该行 → `api.post` → 成功 `toast.success` + `load(currentSource)` 重拉 → 失败 `toast.error` 不动列表。破坏性动作（reject/reassign/release 打字确认）走 `useConfirm({ tone:"danger", requireText })`。

**rich 项自治**：rich 卡片由 richParams 的 id 自己 fetch 单项、自己处置、处置完通过回调通知频道 `load()` 刷新计数。老页薄壳与频道复用同一卡片，互不知道对方存在。

**手动刷新，无轮询、无 WebSocket**（与 steward/system-strategy 一致）。

## 测试策略

- **纯函数单测（vitest，最高价值层）**：`actionKind` 路由、severity/age 排序、`errors[]` 合并降级（**坏源不清空好源**这条核心不变量必测）、summary 计数映射。
- **组件冒烟**：`ReviewQueue` 原语的 loading/error/busy 状态机 + refetch 回调；inline 处置器的 confirm→post→toast→refetch 链路（mock `api`）。
- **老页薄壳回归**：steward/system-strategy/evolution 改薄壳后断言仍渲染同一组件、原有处置仍工作（防中立化回归）。
- **后端两处补缺**：`GET /chunks/:id` 单项路径 + lessons `richParams.lessonId` 走 Rust lib 单测 + 并入 P1 `ask_human_phase1_e2e` 同源集成测试（`#[ignore]`，CI 跑）。
- **UI 真验**：起 dev server 在浏览器走 golden path（进频道→看 summary→inline 处置一条 escalation→rich 挂载一个 evolution proposal→刷新计数）+ 降级路径（造一个坏源看其余正常）。无法在浏览器验证的部分明确说明，不假称成功。

## 交付边界

- ✅ 含：新 `askHuman` 频道 + inboxStore + ReviewQueue 原语 + 4 inline 处置器 + 4 rich 中立化卡片 + 4 处老页薄壳化 + 2 处后端补缺。
- ❌ 不含：退役老页（将来独立决策）；P3 配置页（下一子项目）；新增轮询/WebSocket（保持手动刷新）。

## 红线与约束

- **no-takeover lint**（`scripts/check-no-human-takeover.sh` 扫 `frontend/src/` 新增行）：命名一律 `ask-human`/`askHuman`/`principal`/`escalation`/`decider`，**绝不出现** `takeover`/`人工接管`/`转人工`/`人工介入`/`接管`/`人工`。
- **中立化不改变老页处置语义**：纯重构 + 复用，老页行为字节级不变（薄壳回归测试守住）。
- **后端补缺向后兼容**：`richParams.lessonId` 是加字段（`#[serde(default)]` 不影响旧消费）；`GET /chunks/:id` 是新增路径，不动既有列表端点。
- **测试基线不回归**：后端补缺的 lib 单测进 `cargo test --lib`（≥350/0）；四 PBT 累计 ≥33/0。
- **CI paths 过滤**：P2 以前端改动为主 + 后端仅 2 处。后端补缺会触发后端 job（`ci.yml`），需让基线门 + 集成 job 都跑到；纯前端文件靠本地 vitest + npm build。
- **磁盘纪律**：后端编译前 `rm -rf target/debug/incremental` + `CARGO_INCREMENTAL=0`；本地只 `cargo test --lib` + 单 PBT，集成测试 `#[ignore]` 靠 CI。
- **共享工作树**：本分支历史与素材库会话交错；前端 `git add` 精确具名，排除并行会话产物。

## 命令

- 前端：`cd frontend && npm run dev`（vite，代理 /api → :8080）；`npm run build`；`npm run test`（vitest）。
- 后端补缺：`cargo check --lib`；`cargo test --lib`；`cargo test --test ask_human_phase1_e2e --no-run`（编译，集成测试 CI 跑）。

## P3 接口骨架（不在本轮，仅预留）

P3 配置页消费 P1 的 `PUT .../ask-human-policy`：决策人链（复用 `GET /api/contacts` 做联系人选择器选 decider_chain，非手填 wxid）、四 escalate_* 开关、骚扰频率（dedupe_window/daily_cap/quiet_hours）、超时（timeout_hours），每项配引导文案。

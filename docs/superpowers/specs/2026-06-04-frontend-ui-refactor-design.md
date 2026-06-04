# 前端 UI 重构设计（架构 + 视觉）

- **日期**：2026-06-04
- **范围**：纯前端（`frontend/`），不改后端 API 契约
- **目标**：把 13165 行的 `App.tsx` + 7488 行全局 `styles.css` 重构为有边界的模块化结构，并统一视觉语言为"易读优先 + 功能元素呼吸 + 语义颜色 token"。
- **方式**：增量迁移，每步线上可跑、可回滚、过验证门。

---

## 1. 背景与现状债

当前 `frontend/src/` 的真实状态（已核查）：

| 文件 | 行数 | 问题 |
| --- | --- | --- |
| `App.tsx` | 13165 | 塞了 80+ 个组件，从顶层视图到 `MetricCard`/`StatusLine` 原子组件全在一个文件 |
| `styles.css` | 7488 | 全局无作用域，任意规则可影响全站 |
| `EvolutionCenterTab.tsx` | 745 | 唯一已拆出的页面 —— 证明增量拆分可行 |

- **状态管理**：`App()` 顶层 52 个 `useState`，全项目 285 个 `useState`、**0** 个 `useReducer`/`useContext`/`createContext` —— 纯 prop 透传，`managedCount` 这类值要穿四五层组件。
- **导航**：单个 `activeChannel` state + 13 个 `activeChannel === "xxx" && <View/>` 条件渲染（13 个 channel），所有页面同时挂在一棵树上。

**三个病根**：① 巨型文件无模块边界；② 全局样式污染；③ prop 透传 + 全树重渲染。

---

## 2. 视觉设计语言（基调先行）

经多轮高保真 mockup 迭代确定，三根支柱：

### 2.1 基调 = 易读优先

实色白卡 + 深文字。**玻璃质感只做点缀**（高光 / 镜边 / 投影），文字永远压在实色底上，不让背景透过来吃掉对比度。

**理由**（已用三栏对比图验证）：企业运营后台背景是浅灰中性色，Apple "Liquid Glass" 的半透明在没有鲜艳壁纸可折射时只会变浑浊 + 降低文字对比度。通透在本场景是纯亏 —— 不惊艳还牺牲可读性。运营人员长时间盯数据，易读必须压过通透。

### 2.2 呼吸感 = 只放功能元素

- 背景静止，**无扫屏 / 飘动光团等大动画**。
- 呼吸只发生在功能元素：进行中的状态标签（边框辉光）、关键数字（极轻脉动）、主按钮（光晕）、激活频道（侧栏蓝条）、实时活动频道（呼吸圆点）、打字指示点、头像呼吸环。
- 多元素错开慢节奏（2.2s～4.4s，带 delay），同屏多处微动但整体安静。
- 侧栏**不放裸数字**：用呼吸圆点表达"这里有活动"，不学自明。

### 2.3 颜色语义 = 6 token + 2 刻度 + 分类中性

**原则：状态用色，分类不用色；克制，不彩虹化。**

**6 个核心语义色 token**（覆盖的后端枚举来自 `src/agent/run_envelope.rs` 与 `src/db/migrations/m006_taxonomy_seed.rs`）：

| Token | 色值 | 语义 | 覆盖枚举（示例） |
| --- | --- | --- | --- |
| `--color-running` | `#30D158` 绿 | 进行中 / 成功（**唯一呼吸**） | `running` `sent` `approved` `allowed` `completed` `llm:success` |
| `--color-scheduled` | `#0A84FF` 蓝 | 已排程 / 关键数据 / 可点击 | 已排程跟进、`pending`、关键指标、链接/按钮 |
| `--color-held` | `#FF9F0A` 琥珀 | 暂缓 / 待核验 | `held_by_ai_policy` `ai_waiting_for_more_context` `cooldown` `needs_review` |
| `--color-blocked` | `#FF453A` 玫红 | 拦截 / 失败 / 风险 | `blocked_by_safety_guard` `blocked_unverified_product_claim` `revision_failed` `llm:failed` |
| `--color-brand` | `#5E5CE6` 靛 | AI 身份 / 品牌（**不表达状态**） | logo、AI badge、署名、当前 stage |
| `--color-inactive` | `#8E8E93` 灰 | 未托管 / 停用 / 历史 | `not_managed` `expired` `daily_limit` `admin_cancelled` `superseded_by_new_inbound` `llm:cache_hit` |

**2 条复用刻度**（不引入新色相）：

- **intent_level** 高/中/低 → 同一靛色三档明度（非三种颜色）。
- **方法论评分闸**（FactRisk / PressureRisk / HumanLikeScore / EmotionalValue / ProductAccuracyScore）→ 绿/琥珀/红通用三档（pass / rewrite / block）。

**分类维度不用颜色**：

- **objection_type** 7 项（价格/信任/时机/决策权/产品适配/风险/其他）→ 中性描边标签，靠文字/图标区分。
- **customer_stage** 9 阶段 → 步进条，仅当前阶段靛色高亮，其余中性。

> 注：颜色语义须与后端闭合枚举保持一致。新增 UI 状态前先 grep `GATEWAY_STATUS_VALUES` / `FINAL_REVIEW_STATUS_VALUES` 确认枚举未变。红线：不得出现 `human-takeover/人工接管` 等字样（`scripts/check-no-human-takeover` 已对 `frontend/src/` 生效），状态用 AI 自治内部名（自主回复 / 已排程 / AI 策略暂缓）。

---

## 3. 架构方案（方案一：领域切片 + CSS Modules + Zustand）

### 3.1 目标目录结构

```
frontend/src/
├── main.tsx                      # 入口（基本不动）
├── App.tsx                       # 瘦身到 ~80 行：GlobalErrorBanner + Shell
│
├── app/
│   ├── channels.ts               # Channel 类型 + channel→组件注册表（替代 13 个 === &&）
│   ├── Shell.tsx                 # 侧栏 + 顶栏 + 内容槽布局壳
│   └── Shell.module.css
│
├── stores/                       # Zustand 全局 store（跨 feature 共享）
│   ├── navigationStore.ts        # activeChannel + setChannel
│   ├── accountStore.ts           # accounts, selectedAccountId(持久化), 派生 currentAccount
│   ├── contactStore.ts           # contacts, selected, contactTab, 派生 managedCount/normalCount/hasLiveSession
│   └── uiStore.ts                # busy, error, llmUsage
│
├── components/ui/                # 共享原子库（语义 token 落地处）
│   ├── tokens.css                # ★ 颜色 token / 间距 / 圆角 / 字号 / 呼吸节奏 —— 唯一全局变量来源
│   ├── reset.css                 # 极小的全局 reset
│   ├── StatusBadge/              # 状态标签（5 色 + 呼吸）
│   ├── MetricCard/ · Avatar/ · StageStepper/ · IntentMeter/
│   ├── EmptyState/ · StatusLine/ · PlanStep/ · TagChipInput/ · …
│   └── （每个：Xxx.tsx + Xxx.module.css + index.ts）
│
├── features/                     # 按业务领域切（每个自给自足）
│   ├── command-center/ · overview/ · user-ops/ · knowledge/
│   ├── system-strategy/ · operations/ · content-assets/
│   ├── autonomy/ · evolution/ · quality/ · llm-providers/
│   └── （每个：components/ + hooks/ + api.ts + types.ts + *.module.css + index.ts）
│
├── lib/
│   ├── api.ts                    # fetch 封装 + 错误处理
│   └── format.ts                 # 共享格式化
│
└── types/                        # 跨 feature 共享类型（Channel、Contact、Message…）
```

**病根对应**：巨型文件 → 文件按 feature/组件拆；样式污染 → CSS Modules + 单一 tokens.css；prop 透传 → Zustand 选择器订阅；13 个 `&&` → channels.ts 注册表。

### 3.2 Zustand store 划分 + 数据流

依赖审计已通过（见 §6）。划分标准：**跨多 feature 共享 → 全局 store；只服务单 feature → 留 feature 内部**。

**全局 store（4 个）**：

| Store | 持有 | 谁用 |
| --- | --- | --- |
| `navigationStore` | `activeChannel`, `setChannel` | 几乎所有页面 |
| `accountStore` | `accounts`, `selectedAccountId`(localStorage 持久化), 派生 `currentAccount` | 顶栏 + 大量页面 |
| `contactStore` | `contacts`, `selected`, `contactTab`, 派生 `managedCount`/`normalCount`/`hasLiveSession` | 透传最深的值 |
| `uiStore` | `busy`, `error`, `llmUsage` | 全局加载/错误态 |

**feature 级状态（不进全局）**：各 draft / editing*Id / 输入框文本 / 列表数据，留在对应 feature 的 hook（如 `useUserOpsData()`）。如：
- `user-ops`：messages, events, tasks, operatingMemory, memoryCandidates, memoryDraft, simulation*, guide*, profileNote, query
- `system-strategy`：souls, promptTemplates, operationDomains, domainDrafts, playbooks, *Draft, editing*Id
- `command-center`：commandDraft, commandResult, commandDryRun, commandBusy
- `content-assets`：assets, assetDraft

**数据流（以 managedCount 为例）**：

```
现在：  App() useState → CommandCenterView → 下钻 4 层 → 真正使用的组件
重构后：contactStore 内派生 managedCount；任意组件 useContactStore(s => s.managedCount) 直接订阅
```

**关键**：选择器订阅 —— 组件只订自己关心的字段，`contacts` 变而 `activeChannel` 没变时只订 channel 的组件不重渲染。这是选 Zustand 而非 Context 的核心理由（Context 一变全树重渲染）。store 之间不互相 import，跨 store 派生在组件层用多个选择器组合。

### 3.3 样式体系：tokens.css + CSS Modules

**① `components/ui/tokens.css` —— 唯一全局样式，只定义变量**：

```css
:root {
  /* 语义颜色 token（§2.3 的 6 色，唯一来源） */
  --color-running:#30D158; --color-scheduled:#0A84FF; --color-held:#FF9F0A;
  --color-blocked:#FF453A; --color-brand:#5E5CE6;     --color-inactive:#8E8E93;
  /* 文字阶梯（易读基调：深） */
  --ink-1:#1d1d1f; --ink-2:#515156; --ink-3:#76767b; --ink-4:#b0b0b5;
  /* 表面（易读：实色白卡） */
  --surface-page:#eef1f5; --surface-card:#ffffff; --hairline:rgba(0,0,0,.08);
  /* 间距 / 圆角 / 呼吸节奏 */
  --r-sm:11px; --r-md:18px; --r-lg:24px;
  --breathe-slow:4s; --breathe-mid:3.2s;
}
```

**② 每个组件自带 `.module.css`**，类名编译后带哈希（`.badge` → `StatusBadge_badge_x7k2`），全局污染从机制上消除。

**③ 硬规则（写进 spec，靠 lint 守）**：
- 组件样式里**禁止十六进制色值**，必须走 `var(--color-*)` —— 改 token 即全站生效。
- 全局选择器只允许出现在 `tokens.css` 和 `reset.css`。
- 呼吸动效统一用 `--breathe-*` 变量。

**④ 旧 styles.css 处理**：跟随 feature 增量迁移，摘出该 feature 用到的样式改写为组件 module.css（顺便用 token 变量替换写死色值）。全部迁完后旧文件缩到只剩 tokens.css + reset.css。不一次性重写。

### 3.4 导航注册表 + Shell 壳

**① `app/channels.ts`**：channel 的单一事实来源，每个 channel 一条记录，含 `id/group/label/icon/eyebrow/title/subtitle/Component`，`Component` 用 `lazy(() => import('../features/xxx'))` 懒加载。收益：加页面 = 加一行；按 feature 分包（首屏只加载 command）；侧栏导航 + 页头文字从同一份数据生成。

**② `app/Shell.tsx`**：读 `navigationStore.activeChannel` → 从 `CHANNELS` 找定义 → 渲染 `<Sidebar/>` + `<PageHeader/>` + `<Suspense><Component/></Suspense>`。

**③ `App.tsx`** 瘦身到 ~80 行：`<GlobalErrorBanner/>` + `<Shell/>`。Zustand store 是模块级单例，无需 Provider 包裹。启动预加载（accounts 等）放对应 store 初始化或 Shell 的 useEffect。

**④ 侧栏呼吸圆点**：`Sidebar` 遍历 `CHANNELS`，对有实时活动的 channel（由 store 派生值如 `contactStore.hasLiveSession` 决定）渲染绿色呼吸圆点 —— 数据驱动。

---

## 4. 迁移路径

### 4.1 顺序（地基到叶子，先易后难）

- **阶段 0 脚手架**：建目录骨架 + tokens.css（填 6 色 token）+ reset.css + 装 Zustand。旧 App.tsx 照常跑。
- **阶段 1 原子库**：抽 `components/ui/`（StatusBadge、MetricCard、Avatar、StageStepper、IntentMeter、EmptyState…），用 token 变量写样式。新组件暂未被引用，零风险。
- **阶段 2 store + 外壳**：建 4 个全局 store + channels.ts + Shell；先用最简单的 `overview` channel 跑通骨架。
- **阶段 3 按 feature 逐个搬**（每个一个独立提交）：overview → command-center → content-assets → operations → user-ops → system-strategy → knowledge（最大，最后）→ autonomy/evolution/quality/llm-providers。每搬一个：删旧 App.tsx 对应代码、摘 styles.css 对应样式改 module.css、改用新 store/ui 组件。
- **阶段 4 收尾**：App.tsx 瘦身；删空旧 styles.css；补关键 store 和 ui 组件单元测试。

### 4.2 验证门（不过不进下一步）

- `cd frontend && npm run build`（`tsc && vite build`）零错误 —— TS 类型是搬迁正确性第一道保险。
- `npm test`（Vitest）现有测试不回归；新抽的 store / ui 组件补测试。
- **每搬完一个 feature，在浏览器实际点开该 channel 走主路径**，确认渲染/交互/数据加载正常（类型过 ≠ 功能对）。
- 每个 feature 一个独立提交，可精确回滚。

### 4.3 风险与应对

| 风险 | 应对 |
| --- | --- |
| 漏改一处 prop→store，运行时报错 | TS 严格模式 + 每 feature 构建门 |
| 摘全局样式影响还没迁的页面 | 只动该 feature 独有的类；共享类留最后；每步肉眼回归 |
| 懒加载分包后切换白屏 | Suspense 骨架屏 fallback；预加载相邻 channel |
| 周期长、与日常改动冲突 | feature 粒度提交，随时可暂停；旧结构始终可跑 |

---

## 5. 不做的事（守住范围）

- 不上 React Router（保留项目 state 导航的简洁约定）。
- 不引入 UI 组件库（shadcn/Radix 等），自建 ui 原子库。
- Phase 1 只做用户运营域；群 / 朋友圈 channel 占位保留，不在本次扩功能。
- 不改后端 API 契约 —— 纯前端结构调整。

---

## 6. 依赖审计：Zustand

- **版本** 5.0.14，**License** MIT
- **运行时依赖** 0 个（无传递依赖，供应链攻击面极小）
- **周下载** ~3900 万
- **维护方** pmndrs（jotai / react-three-fiber 同组织），仓库 github.com/pmndrs/zustand
- 结论：符合项目"只用审计过的依赖"政策（2026-06-04 核查通过）。

---

## 7. 验收标准

- `App.tsx` 瘦身到 ~80 行（验收上限 ≤150 行）；`styles.css` 仅剩 tokens.css + reset.css（或等价）。
- 每个 feature 独立目录，组件自带 `.module.css`，样式无十六进制硬编码色值。
- 跨页共享状态走 4 个 Zustand store，无深层 prop 透传。
- 导航由 `channels.ts` 注册表驱动；feature 懒加载分包。
- 视觉符合 §2：易读基调、功能元素呼吸、6 色语义 token。
- `npm run build` + `npm test` 全绿；各 channel 主路径浏览器验证通过。
